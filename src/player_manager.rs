use std::{cell::RefCell, collections::HashMap, rc::Rc};

use waybar_cffi::gtk::{
	Box as GtkBox,
	gdk::gio::{Cancellable, DBusCallFlags, DBusConnection, DBusSignalFlags},
	glib::{self, clone, ToVariant, Variant, VariantDict, VariantTy},
	prelude::WidgetExtManual, traits::ContainerExt
};

use crate::player::PlayerWidget;

pub struct PlayerManager {
	dbus_conn: DBusConnection,
	player_by_name: RefCell<HashMap<String, Rc<PlayerWidget>>>,
	player_list_widget: GtkBox,
	playerctld_owner: RefCell<Option<String>>,
}

impl PlayerManager {
	fn add_player(slf: &Rc<Self>, name: String) -> Result<(), glib::Error> {
		let player = PlayerWidget::new(slf, name.clone());
		match player {
			Ok(player) => {
				slf.player_list_widget.add(player.get_root());
				slf.player_by_name.borrow_mut().insert(name, player);
				Ok(())
			}
			Err(error) => {
				eprintln!("Failed to create player: {}", error);
				Err(error)
			}
		}
	}

	pub fn remove_player(&self, name: &str) -> bool {
		if let Some(player) = self.player_by_name.borrow().get(name) {
			self.player_list_widget.remove(player.get_root());
			unsafe { player.get_root().destroy(); }
		}
		self.player_by_name.borrow_mut().remove(name).is_some()
	}

	pub fn get_player(&self, name: &str) -> Option<Rc<PlayerWidget>> {
		return self.player_by_name.borrow().get(name).cloned();
	}

	pub fn have_player_by_name(&self, name: &str) -> bool {
		return self.player_by_name.borrow().contains_key(name);
	}

	fn dbus_get_name_owner(dbus_conn: &DBusConnection, name: &str) -> Option<String> {
		dbus_conn.call_sync(
			Some("org.freedesktop.DBus"),
			"/org/freedesktop/DBus",
			"org.freedesktop.DBus",
			"GetNameOwner",
			Some(&Variant::from((name,))),
			Some(VariantTy::TUPLE),
			DBusCallFlags::NONE,
			3000, None::<&Cancellable>)
			.map_or(None, |v| { v.get::<(String,)>().map(|t| { t.0 }) })
	}

	pub fn new(dbus_conn: DBusConnection, player_list_widget: GtkBox) -> Rc<Self> {
		let playerctld_owner = RefCell::new(
			Self::dbus_get_name_owner(&dbus_conn, "org.mpris.MediaPlayer2.playerctld"));
		PlayerManager {
			dbus_conn,
			player_by_name: RefCell::new(HashMap::new()),
			player_list_widget,
			playerctld_owner,
		}.attach_listeners()
	}

	fn attach_listeners(self) -> Rc<Self> {
		let slf = Rc::new(self);

		slf.dbus_conn.signal_subscribe(
			Some("org.freedesktop.DBus"),
			Some("org.freedesktop.DBus"),
			Some("NameOwnerChanged"),
			Some("/org/freedesktop/DBus"),
			Some("org.mpris.MediaPlayer2"),
			DBusSignalFlags::MATCH_ARG0_NAMESPACE,
			clone!(@weak slf => move |_conn, _sender, _path, _interface, _signal, params| {
				if let Some((name, old_owner, new_owner)) = params.get::<(String, String, String)>() {
					if !name.starts_with("org.mpris.MediaPlayer2") { return; }
					if name == "org.mpris.MediaPlayer2.playerctld" && !new_owner.is_empty() {
						slf.remove_player(&new_owner);
						slf.playerctld_owner.replace(Some(new_owner));
						return;
					}
					if new_owner.is_empty() {
						// dead player
						if slf.have_player_by_name(&old_owner) {
							println!("killing {}", name);
						}
						slf.remove_player(&old_owner);
						return;
					}
					if old_owner.is_empty() {
						if slf.have_player_by_name(&new_owner) { return; }
						println!("new {} at {}", name, new_owner);
						let _ = Self::add_player(&slf, new_owner);
					}
				}
			})
		);

		slf.dbus_conn.signal_subscribe(
			None,
			Some("org.mpris.MediaPlayer2.Player"),
			Some("Seeked"),
			Some("/org/mpris/MediaPlayer2"),
			None,
			DBusSignalFlags::NONE,
			clone!(@weak slf => move |_conn, sender, _path, _interface, _signal, params| {
				if !slf.have_player_by_name(sender) {
					if slf.playerctld_owner.borrow().as_ref().is_some_and(|s| { s == sender }) { return; }
					println!("revived {}", sender);
					if Self::add_player(&slf, sender.to_string()).is_err() { return; }
				}
				let player = slf.get_player(sender).unwrap();
				if let Some((position,)) = params.get::<(i64,)>() {
					player.update_seek(position);
				} else {
					eprintln!("Unexpected arguments to Seeked signal: {}", params.print(true).as_str());
				}
			})
		);

		slf.dbus_conn.signal_subscribe(
			None,
			Some("org.freedesktop.DBus.Properties"),
			Some("PropertiesChanged"),
			Some("/org/mpris/MediaPlayer2"),
			None,
			DBusSignalFlags::NONE,
			clone!(@weak slf => move |_conn, sender, _path, _interface, _signal, params| {
				if !slf.have_player_by_name(sender) {
					if slf.playerctld_owner.borrow().as_ref().is_some_and(|s| { s == sender }) { return; }
					println!("revived {}", sender);
					if Self::add_player(&slf, sender.to_string()).is_err() { return; }
				}
				let player = slf.get_player(sender).unwrap();
				if let Some((_interface, changed_props, _invalid_props)) = params.get::<(String, VariantDict, Vec<String>)>(){
					for el in changed_props.end().iter() {
						let k = el.child_get::<String>(0);
						let v = el.child_get::<Variant>(1);
						player.update_prop(&k, v);
					}
				} else {
					eprintln!("Unexpected arguments to PropertiesChanged signal: {}", params.print(true).as_str());
				}
			})
		);
		slf
	}

	pub fn probe_initial_players(slf: &Rc<Self>) {
		let names = slf.dbus_conn.call_sync(
			Some("org.freedesktop.DBus"),
			"/org/freedesktop/DBus",
			"org.freedesktop.DBus",
			"ListNames",
			None,
			Some(VariantTy::TUPLE),
			DBusCallFlags::NONE,
			3000,
			None::<&Cancellable>)
			.expect("Could not probe DBus for MPRIS players")
			.child_get::<Vec<String>>(0);
		for name in &names {
			if !name.starts_with("org.mpris.MediaPlayer2")
				|| name == "org.mpris.MediaPlayer2.playerctld" { continue; }
			let bus_name = Self::dbus_get_name_owner(&slf.dbus_conn, name);
			if let Some(bus_name) = bus_name {
				let _ = Self::add_player(slf, bus_name);
			}
		}
	}

	pub fn get_all_player_data(&self, name: &str) -> Result<Variant, glib::Error> {
		self.dbus_conn.call_sync(
			Some(name),
			"/org/mpris/MediaPlayer2",
			"org.freedesktop.DBus.Properties",
			"GetAll",
			Some(&Variant::from(("org.mpris.MediaPlayer2.Player",))),
			Some(VariantTy::TUPLE),
			DBusCallFlags::NONE,
			3000,
			None::<&Cancellable>)
	}

	pub fn call_player_fn<T: FnOnce(Result<glib::Variant, glib::Error>) + 'static>(&self, name: &str, method: &str, params: Option<&Variant>, cb: T) {
		self.dbus_conn.call(
			Some(name),
			"/org/mpris/MediaPlayer2",
			"org.mpris.MediaPlayer2.Player",
			method,
			params,
			None,
			DBusCallFlags::NONE,
			-1,
			None::<&Cancellable>,
			cb);
	}

	pub fn get_player_prop(&self, name: &str, field: &str) -> Result<Variant, glib::Error> {
		self.dbus_conn.call_sync(
			Some(name),
			"/org/mpris/MediaPlayer2",
			"org.freedesktop.DBus.Properties",
			"Get",
			Some(&Variant::from(("org.mpris.MediaPlayer2.Player", field))),
			None,
			DBusCallFlags::NONE,
			5000,
			None::<&Cancellable>)
	}

	pub fn set_player_prop<T: FnOnce(Result<glib::Variant, glib::Error>) + 'static>(&self, name: &str, field: &str, value: &Variant, cb: T) {
		self.dbus_conn.call(
			Some(name),
			"/org/mpris/MediaPlayer2",
			"org.freedesktop.DBus.Properties",
			"Set",
			Some(&Variant::from(("org.mpris.MediaPlayer2.Player", field, value.to_variant()))),
			None,
			DBusCallFlags::NONE,
			-1,
			None::<&Cancellable>,
			cb)
	}
}
