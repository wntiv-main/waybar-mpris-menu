use std::rc::Rc;

use waybar_cffi::{
	Module,
	gtk::{
		Box as GtkBox, Button, Window,
		gdk::{AnchorHints, EventMask, Rectangle},
		gio::{Cancellable, bus_get_sync},
		glib::{self, clone, Cast},
		traits::{ButtonExt, ContainerExt, GtkWindowExt, WidgetExt}
	},
	serde, waybar_module
};

mod player;
mod player_model;
mod player_manager;

use crate::player_manager::PlayerManager;

struct MprisWidget {
	player_manager: Rc<PlayerManager>,
}

impl Module for MprisWidget {
	type Config = Config;

	fn init(info: &waybar_cffi::InitInfo, _config: Self::Config) -> Self {
		let container = info.get_root_widget();
		let label = Button::with_label("Hello World");
		container.add(&label);
		container.show_all();

		let menu = GtkBox::new(waybar_cffi::gtk::Orientation::Vertical, 0);
		menu.show_all();

		let menu_container = Window::builder()
			.type_(waybar_cffi::gtk::WindowType::Toplevel)
			.child(&menu)
			.decorated(false)
			.type_hint(waybar_cffi::gtk::gdk::WindowTypeHint::PopupMenu)
			.attached_to(&label)
			.modal(false)
			.resizable(false)
			.can_focus(true)
			.events(EventMask::FOCUS_CHANGE_MASK)
			.build();
		menu_container.connect_realize(clone!(@weak label => move |slf| {
			let rect = label.allocation();
			let (x, y) = label.translate_coordinates(&label.toplevel().unwrap(), 0, 0).unwrap();
			let rect = Rectangle::new(x, y, rect.width(), rect.height());
			slf.window().expect("GdkWindow backend not available").move_to_rect(
				&rect,
				gdk::Gravity::SouthWest,
				gdk::Gravity::NorthWest,
				AnchorHints::FLIP_Y | AnchorHints::SLIDE_X,
				0, 0,
			);
		}));
		menu_container.connect_focus_out_event(|slf, _e| {
			println!("focus lost");
			slf.hide();
			waybar_cffi::gtk::glib::Propagation::Proceed
		});
		menu_container.connect_grab_broken_event(|slf, _e| {
			println!("grab dead");
			slf.hide();
			waybar_cffi::gtk::glib::Propagation::Proceed
		});

		label.connect_size_allocate(clone!(@weak menu_container => move |slf, rect| {
			if let Some(win) = menu_container.window() {
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
		}));

		label.connect_clicked(clone!(@weak menu_container => move |_label| {
			if menu_container.is_visible() {
				menu_container.hide();
			} else {
				if let Some(tl) = container.toplevel() {
					if let Some(pwin) = tl.downcast_ref::<Window>() {
						menu_container.set_transient_for(Some(pwin));
					}
				}
				menu_container.present();
			}
		}));

		let dbus = bus_get_sync(waybar_cffi::gtk::gio::BusType::Session,
			None::<&Cancellable>).expect("Could not connect to D-Bus");

		let player_manager = PlayerManager::new(dbus, menu);

		MprisWidget { player_manager }
	}
}

waybar_module!(MprisWidget);

#[derive(serde::Deserialize)]
struct Config {
}
