use std::{cell::RefCell, collections::HashMap, rc::Rc};

use waybar_cffi::{
	Module,
	gtk::{
		Box as GtkBox, Button, Window,
		gdk::{AnchorHints, EventMask, Rectangle},
		gio::{Cancellable, DBusCallFlags, DBusSignalFlags, bus_get_sync},
		glib::{Cast, Variant, VariantDict, VariantTy},
		prelude::WidgetExtManual,
		traits::{ButtonExt, ContainerExt, GtkWindowExt, WidgetExt}
	},
	serde, waybar_module
};

mod player;
mod player_model;
use player::PlayerWidget;

struct MprisWidget;

impl Module for MprisWidget {
	type Config = Config;

	fn init(info: &waybar_cffi::InitInfo, _config: Self::Config) -> Self {
		let container = info.get_root_widget();
		let label = Rc::new(RefCell::new(Button::with_label("Hello World")));
		container.add(&*label.borrow());
		container.show_all();

		let menu_container = Rc::new(RefCell::new(None));
		let _menu = GtkBox::new(waybar_cffi::gtk::Orientation::Vertical, 0);
		_menu.show_all();
		let menu = Rc::new(RefCell::new(_menu));

		label.borrow().connect_size_allocate({
			let menu_container = Rc::clone(&menu_container);
			move |slf, rect| {
				if let Some(win) = menu_container.borrow().as_ref().map_or(None, Window::window) {
					if let Some((x, y)) = slf.translate_coordinates(
							&slf.toplevel().unwrap(), 0, 0) {
						let rect = Rectangle::new(x, y, rect.width(), rect.height());
						win.move_to_rect(&rect,
							gdk::Gravity::SouthWest,
							gdk::Gravity::NorthWest,
							AnchorHints::FLIP_Y | AnchorHints::SLIDE_X,
							0, 0);
					}
				}
		}});

		label.borrow().connect_clicked({
			let menu = Rc::clone(&menu);
			let label = Rc::clone(&label);
			let menu_container = Rc::clone(&menu_container);
			move |_label| {
				if menu_container.borrow().is_none() {
					let _menu = menu.borrow();
					let mut _mcont = Window::builder()
						.type_(waybar_cffi::gtk::WindowType::Toplevel)
						.child(&*_menu)
						.decorated(false)
						.type_hint(waybar_cffi::gtk::gdk::WindowTypeHint::PopupMenu)
						.attached_to(_label)
						.modal(false)
						.resizable(false)
						.can_focus(true)
						.events(EventMask::FOCUS_CHANGE_MASK)
						.build();
					let label = Rc::clone(&label);
					_mcont.connect_realize(move |slf| {
						let _label = label.borrow();
						let rect = _label.allocation();
						let (x, y) = _label.translate_coordinates(
							&_label.toplevel().unwrap(), 0, 0).unwrap();
						let rect = Rectangle::new(x, y, rect.width(), rect.height());
						slf.window().expect("GdkWindow backend not available").move_to_rect(
							&rect,
							gdk::Gravity::SouthWest,
							gdk::Gravity::NorthWest,
							AnchorHints::FLIP_Y | AnchorHints::SLIDE_X,
							0, 0,
						);
					});
					_mcont.connect_focus_out_event(|slf, _e| {
						println!("focus lost");
						slf.hide();
						waybar_cffi::gtk::glib::Propagation::Proceed
					});
					_mcont.connect_grab_broken_event(|slf, _e| {
						println!("grab dead");
						slf.hide();
						waybar_cffi::gtk::glib::Propagation::Proceed
					});

					menu_container.replace(Some(_mcont));
				}
				let winopt = menu_container.borrow();
				let win = winopt.as_ref().unwrap();
				if win.is_visible() {
					win.hide();
				} else {
					if let Some(tl) = container.toplevel() {
						if let Some(pwin) = tl.downcast_ref::<Window>() {
							win.set_transient_for(Some(pwin));
						}
					}
					win.present();
					// win.show_all();
				}
		}});

		let player_by_name: Rc<RefCell<HashMap<String, Rc<PlayerWidget>>>> = Rc::new(RefCell::new(HashMap::new()));

		let dbus = Rc::new(RefCell::new(
			bus_get_sync(waybar_cffi::gtk::gio::BusType::Session,
			None::<&Cancellable>).expect("Could not connect to D-Bus")));

		let playerctld_owner = Rc::new(RefCell::new(dbus.borrow().call_sync(
			Some("org.freedesktop.DBus"),
			"/org/freedesktop/DBus",
			"org.freedesktop.DBus",
			"GetNameOwner",
			Some(&Variant::from(("org.mpris.MediaPlayer2.playerctld",))),
			Some(VariantTy::TUPLE),
			DBusCallFlags::NONE,
			-1, None::<&Cancellable>)
			.map_or(None, |v| { v.get::<(String,)>().map(|t| { t.0 }) })));

		dbus.borrow().signal_subscribe(
			Some("org.freedesktop.DBus"),
			Some("org.freedesktop.DBus"),
			Some("NameOwnerChanged"),
			Some("/org/freedesktop/DBus"),
			Some("org.mpris.MediaPlayer2"),
			DBusSignalFlags::MATCH_ARG0_NAMESPACE,
			{
				let dbus = Rc::clone(&dbus);
				let menu = Rc::clone(&menu);
				let player_by_name = Rc::clone(&player_by_name);
				let playerctld_owner = Rc::clone(&playerctld_owner);
				move |_conn, _sender, _path, _interface, _signal, params| {
					if let Some((name, old_owner, new_owner)) = params.get::<(String, String, String)>() {
						if !name.starts_with("org.mpris.MediaPlayer2") { return; }
						if name == "org.mpris.MediaPlayer2.playerctld" && !new_owner.is_empty() {
							if let Some(player) = player_by_name.borrow().get(&new_owner) {
								menu.borrow().remove(&player.root);
								unsafe { player.root.destroy(); }
								player_by_name.borrow_mut().remove(&new_owner);
							}
							playerctld_owner.replace(Some(new_owner));
							return;
						}
						if new_owner.is_empty() {
							// dead player
							if let Some(player) = player_by_name.borrow().get(&old_owner) {
								println!("killing {}", name);
								menu.borrow().remove(&player.root);
								unsafe { player.root.destroy(); }
							}
							player_by_name.borrow_mut().remove(&old_owner);
						}
						if old_owner.is_empty() {
							if player_by_name.borrow().contains_key(&new_owner) { return; }
							// new player
							println!("new {} at {}", name, new_owner);
							let player = PlayerWidget::new(dbus.clone(), new_owner.clone());
							menu.borrow().add(&player.root);
							player_by_name.borrow_mut().insert(new_owner, player);
						}
					}
				}
		});

		dbus.borrow().signal_subscribe(
			None,
			Some("org.mpris.MediaPlayer2.Player"),
			Some("Seeked"),
			Some("/org/mpris/MediaPlayer2"),
			None,
			DBusSignalFlags::NONE,
			{
				let dbus = Rc::clone(&dbus);
				let menu = Rc::clone(&menu);
				let player_by_name = Rc::clone(&player_by_name);
				let playerctld_owner = Rc::clone(&playerctld_owner);
				move |_conn, sender, _path, _interface, _signal, params| {
					if !player_by_name.borrow().contains_key(sender) {
						if playerctld_owner.borrow().as_ref().is_some_and(|s| { s == sender }) { return; }
						println!("revived {}", sender);
						let player = PlayerWidget::new(dbus.clone(), sender.to_string());
						menu.borrow().add(&player.root);
						player_by_name.borrow_mut().insert(sender.to_string(), player);
					}
					let map = player_by_name.borrow();
					let player = map.get(sender).unwrap();
					let (position,) = params.get::<(i64,)>().expect("Failed to unpack Seeked");
					player.update_seek(position);
				}
		});

		dbus.borrow().signal_subscribe(
			None,
			Some("org.freedesktop.DBus.Properties"),
			Some("PropertiesChanged"),
			Some("/org/mpris/MediaPlayer2"),
			None,
			DBusSignalFlags::NONE,
			{
				let dbus = Rc::clone(&dbus);
				let menu = Rc::clone(&menu);
				let player_by_name = Rc::clone(&player_by_name);
				let playerctld_owner = Rc::clone(&playerctld_owner);
				move |_conn, sender, _path, _interface, _signal, params| {
					if !player_by_name.borrow().contains_key(sender) {
						if playerctld_owner.borrow().as_ref().is_some_and(|s| { s == sender }) { return; }
						println!("revived {}", sender);
						let player = PlayerWidget::new(dbus.clone(), sender.to_string());
						menu.borrow().add(&player.root);
						player_by_name.borrow_mut().insert(sender.to_string(), player);
					}
					let map = player_by_name.borrow();
					let player = map.get(sender).unwrap();
					let (_interface, changed_props, _invalid_props) = params.get::<(String, VariantDict, Vec<String>)>().expect("Failed to unpack PropertiesChanged");
					for el in changed_props.end().iter() {
						let k = el.child_get::<String>(0);
						let v = el.child_get::<Variant>(1);
						player.update_prop(&k, v);
					}
				}
		});

		MprisWidget
	}
}

waybar_module!(MprisWidget);

#[derive(serde::Deserialize)]
struct Config {
}
