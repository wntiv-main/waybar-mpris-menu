use std::{cell::RefCell, collections::HashMap, rc::{Rc, Weak}};

use gdk::glib::{SignalHandlerId, clone};
use soup::prelude::*;
use waybar_cffi::{
	Module, gtk::{
		Adjustment, Box as GtkBox, Button, Image, Label, Scale, ToggleButton, Window, gdk::{AnchorHints, EventMask, Rectangle}, gdk_pixbuf::Pixbuf, gio::{Cancellable, DBusCallFlags, DBusConnection, DBusSignalFlags, MemoryInputStream, bus_get_sync}, glib::{self, Cast, MainContext, Variant, VariantDict, VariantTy, clone::Downgrade, variant::ObjectPath}, prelude::WidgetExtManual, traits::{AdjustmentExt, ButtonExt, ContainerExt, GtkWindowExt, ImageExt, LabelExt, RangeExt, ScaleExt, ToggleButtonExt, WidgetExt}},
		serde, waybar_module
};

struct MprisWidget;

#[derive(PartialEq, Eq)]
enum PlayState {
	Stopped,
	Playing,
	Paused,
}

impl PlayState {
	fn inverse(&self) -> Self {
		match self {
			Self::Stopped => Self::Playing,
			Self::Playing => Self::Paused,
			Self::Paused => Self::Playing,
		}
	}

	fn icon_name(&self) -> &'static str {
		match self {
			Self::Stopped => "media-playback-stop",
			Self::Playing => "media-playback-start",
			Self::Paused => "media-playback-pause",
		}
	}
}

impl Into<&str> for PlayState {
    fn into(self) -> &'static str {
        match self {
			Self::Stopped => "Stopped",
			Self::Playing => "Playing",
			Self::Paused => "Paused",
		}
    }
}

impl TryFrom<&str> for PlayState {
	type Error = ();

	fn try_from(value: &str) -> Result<Self, ()> {
		match value {
			"Stopped" => Ok(Self::Stopped),
			"Playing" => Ok(Self::Playing),
			"Paused" => Ok(Self::Paused),
			_ => Err(())
		}
	}
}

#[derive(PartialEq, Eq, Clone, Copy)]
enum LoopState {
	None,
	LoopPlaylist,
	LoopSingle,
}

impl LoopState {
	fn next(&self) -> Self {
		match self {
			Self::None => Self::LoopPlaylist,
			Self::LoopPlaylist => Self::LoopSingle,
			Self::LoopSingle => Self::None,
		}
	}

	fn icon_name(&self) -> &'static str {
		match self {
			Self::None => "media-playlist-repeat",
			Self::LoopPlaylist => "media-playlist-repeat",
			Self::LoopSingle => "media-playlist-repeat-song",
		}
	}

	fn is_active(&self) -> bool {
		*self != Self::None
	}
}

impl Into<&str> for LoopState {
    fn into(self) -> &'static str {
        match self {
			Self::None => "None",
			Self::LoopSingle => "Track",
			Self::LoopPlaylist => "Playlist",
		}
    }
}

impl TryFrom<&str> for LoopState {
	type Error = ();

	fn try_from(value: &str) -> Result<Self, ()> {
		match value {
			"None" => Ok(Self::None),
			"Track" => Ok(Self::LoopSingle),
			"Playlist" => Ok(Self::LoopPlaylist),
			_ => Err(())
		}
	}
}

struct PlayerData {
	current_track: Option<ObjectPath>,
	art_url: Option<String>,

	play_state: PlayState,
	shuffle_state: bool,
	loop_state: LoopState,

	rate: f64,

	can_control: bool,
	can_seek: bool,
	can_play: bool,
	can_pause: bool,
	can_go_prev: bool,
	can_go_next: bool,

	has_shuf: bool,
	has_loop: bool,

	position_base: i64,
	clock_needs_aligned: bool,

	is_dragging_playback: bool,
	is_dragging_volume: bool,
	is_dragging_rate: bool,

	loop_sig_id: Option<SignalHandlerId>,
	shuf_sig_id: Option<SignalHandlerId>,
}

struct PlayerWidget {
	dbus_conn: Weak<RefCell<DBusConnection>>,
	name: String,
	
	data: Rc<RefCell<PlayerData>>,

	playback_adj: Adjustment,
	volume_adj: Adjustment,
	rate_adj: Adjustment,

	root: GtkBox,
	title: Label,
	album_artist: Label,
	album_cover: Rc<RefCell<Image>>,
	shuf: ToggleButton,
	prev: Button,
	play_pause: Button,
	next: Button,
	loop_: ToggleButton,
	playback: Scale,
	volume_ctl: Scale,
	rate_ctl: Scale,
}

impl PlayerWidget {
	fn new(conn: Rc<RefCell<DBusConnection>>, inst_name: String) -> Self {
		// Initial state
		let resp = conn.borrow().call_sync(
			Some(&inst_name),
			"/org/mpris/MediaPlayer2",
			"org.freedesktop.DBus.Properties",
			"GetAll",
			Some(&Variant::from(("org.mpris.MediaPlayer2.Player",))),
			Some(VariantTy::TUPLE),
			DBusCallFlags::NONE,
			-1,
			None::<&Cancellable>).expect("Could not access properties");
		let props = VariantDict::from(resp.child_value(0));
		let meta_props = props.lookup_value("Metadata", Some(&VariantTy::VARDICT))
			.map(VariantDict::from);

		let play_state = props.lookup_value("PlayStatus", Some(VariantTy::STRING))
			.map(|v| { v.get::<String>()?.as_str().try_into().ok() }).flatten().unwrap_or(PlayState::Stopped);
		let loop_state = props.lookup_value("LoopStatus", Some(VariantTy::STRING))
			.map(|v| { v.get::<String>()?.as_str().try_into().ok() }).flatten().unwrap_or(LoopState::None);
		let shuffle_state = props.lookup_value("Shuffle", Some(VariantTy::BOOLEAN))
			.map_or(false, |v| { v.get::<bool>().unwrap_or(false) });

		let min_rate = props.lookup_value("MinimumRate", Some(VariantTy::DOUBLE))
			.map_or(1., |v| { v.get::<f64>().unwrap_or(1.) });
		let max_rate = props.lookup_value("MaximumRate", Some(VariantTy::DOUBLE))
			.map_or(1., |v| { v.get::<f64>().unwrap_or(1.) }).min(4.);
		let rate = props.lookup_value("Rate", Some(VariantTy::DOUBLE))
			.map_or(1., |v| { v.get::<f64>().unwrap_or(1.) });

		let volume = props.lookup_value("Volume", Some(VariantTy::DOUBLE))
			.map_or(1., |v| { v.get::<f64>().unwrap_or(1.) });

		let root = GtkBox::new(waybar_cffi::gtk::Orientation::Vertical, 0);
		let header = GtkBox::new(waybar_cffi::gtk::Orientation::Horizontal, 0);

		// Metadata
		let album_cover = Image::new();
		let info = GtkBox::new(waybar_cffi::gtk::Orientation::Vertical, 0);
		let title = Label::new(None);
		let album_artist = Label::new(None);

		// Controls
		let controls = GtkBox::new(waybar_cffi::gtk::Orientation::Horizontal, 0);
		let prev = Button::from_icon_name(Some("media-skip-backward"), waybar_cffi::gtk::IconSize::Button);
		let play_pause = Button::from_icon_name(Some(play_state.inverse().icon_name()), waybar_cffi::gtk::IconSize::Button);
		let next = Button::from_icon_name(Some("media-skip-forward"), waybar_cffi::gtk::IconSize::Button);

		let shuf_icon = Image::from_icon_name(Some("media-playlist-shuffle"), waybar_cffi::gtk::IconSize::Button);
		let shuf = ToggleButton::new();
		shuf.set_image(Some(&shuf_icon));
		shuf.set_active(shuffle_state);

		let loop_icon = Image::from_icon_name(Some(loop_state.icon_name()), waybar_cffi::gtk::IconSize::Button);
		let loop_ = ToggleButton::new();
		loop_.set_image(Some(&loop_icon));
		loop_.set_active(loop_state.is_active());

		controls.add(&shuf);
		controls.add(&prev);
		controls.add(&play_pause);
		controls.add(&next);
		controls.add(&loop_);

		info.add(&title);
		info.add(&album_artist);
		info.add(&controls);

		let playback_adj = Adjustment::new(0., 0., 0., 1., 5000., 0.);
		let playback = Scale::new(waybar_cffi::gtk::Orientation::Horizontal, Some(&playback_adj));
		playback.set_draw_value(false);
		let volume_adj = Adjustment::new(volume, 0., 1.25, 0.01, 0.1, 0.);
		let volume_ctl = Scale::new(waybar_cffi::gtk::Orientation::Vertical, Some(&volume_adj));
		volume_ctl.set_draw_value(false);
		volume_ctl.set_inverted(true);
		let rate_adj = Adjustment::new(rate, min_rate, max_rate, 0.1, 0.25, 0.);
		let rate_ctl = Scale::new(waybar_cffi::gtk::Orientation::Vertical, Some(&rate_adj));
		rate_ctl.set_draw_value(false);
		rate_ctl.set_inverted(true);

		header.add(&album_cover);
		header.add(&info);
		header.add(&volume_ctl);
		header.add(&rate_ctl);
		
		root.add(&header);
		root.add(&playback);

		root.show_all();
		let result = PlayerWidget {
			dbus_conn: conn.downgrade(),
			name: inst_name,

			data: Rc::new(RefCell::new(PlayerData {
				current_track: meta_props.as_ref()
					.map(|m| { m.lookup_value("mpris:trackid", Some(VariantTy::OBJECT_PATH)) }).flatten()
					.map(|v| { v.get::<ObjectPath>() }).flatten(),
				art_url: None, // Init later to set art properly

				position_base: 0,
				clock_needs_aligned: true,

				is_dragging_playback: false,
				is_dragging_volume: false,
				is_dragging_rate: false,

				play_state,
				loop_state,
				shuffle_state,

				rate,

				has_loop: props.contains("LoopStatus"),
				has_shuf: props.contains("Shuffle"),

				loop_sig_id: None,
				shuf_sig_id: None,
				
				can_control: props.lookup_value("CanControl", Some(VariantTy::BOOLEAN))
					.map_or(false, |v| { v.get::<bool>().unwrap_or(false) }),
				can_seek: props.lookup_value("CanSeek", Some(VariantTy::BOOLEAN))
					.map_or(false, |v| { v.get::<bool>().unwrap_or(false) }),
				can_play: props.lookup_value("CanPlay", Some(VariantTy::BOOLEAN))
					.map_or(false, |v| { v.get::<bool>().unwrap_or(false) }),
				can_pause: props.lookup_value("CanPause", Some(VariantTy::BOOLEAN))
					.map_or(false, |v| { v.get::<bool>().unwrap_or(false) }),
				can_go_next: props.lookup_value("CanGoNext", Some(VariantTy::BOOLEAN))
					.map_or(false, |v| { v.get::<bool>().unwrap_or(false) }),
				can_go_prev: props.lookup_value("CanGoPrevious", Some(VariantTy::BOOLEAN))
					.map_or(false, |v| { v.get::<bool>().unwrap_or(false) }),
			})),

			playback_adj,
			volume_adj,
			rate_adj,
			
			root,
			title,
			album_artist,
			album_cover: Rc::new(RefCell::new(album_cover)),
			shuf,
			prev,
			play_pause,
			next,
			loop_,
			playback,
			volume_ctl,
			rate_ctl,
		};
		meta_props.map(|m| { result.update_metadata(m) });
		result
	}

	fn update_sensitivity(&self) {
		let data = self.data.borrow();
		self.play_pause.set_sensitive(data.can_control && (
			if data.play_state == PlayState::Playing {data.can_pause} else {data.can_play}));
		self.prev.set_sensitive(data.can_control && data.can_go_prev);
		self.next.set_sensitive(data.can_control && data.can_go_next);
		self.loop_.set_sensitive(data.can_control && data.has_loop);
		self.shuf.set_sensitive(data.can_control && data.has_shuf);
		self.playback.set_sensitive(data.can_control && data.can_seek);
	}

	fn update_metadata(&self, meta_props: VariantDict) {
		self.data.borrow_mut().clock_needs_aligned = true;
		if let Some(trackid) = meta_props.lookup_value("mpris:trackid", Some(VariantTy::OBJECT_PATH))
				.map(|v| { v.get::<ObjectPath>() }).flatten() {
			self.data.borrow_mut().current_track.replace(trackid);
		}

		if let Some(title) = meta_props.lookup_value("xesam:title", Some(&VariantTy::STRING))
				.map(|v| { v.get::<String>() }).flatten() {
			self.title.set_label(&title);
		}
		
		let mut subtitle = String::new();
		if let Some(album) = meta_props.lookup_value("xesam:album", Some(&VariantTy::STRING))
				.map(|v| { v.get::<String>() }).flatten() {
			subtitle += &album;
		}

		if let Some(artist) = meta_props.lookup_value("xesam:artist", Some(&VariantTy::STRING_ARRAY))
				.map(|v| { v.get::<Vec<String>>() }).flatten() {
			if !subtitle.is_empty() { subtitle += " - "; }
			subtitle += &artist.join(", ");
		}
		self.album_artist.set_label(&subtitle);

		if let Some(duration) = meta_props.lookup_value("mpris:length", Some(&VariantTy::INT64))
				.map(|v| { v.get::<i64>() }).flatten() {
			self.playback_adj.set_upper(duration as f64);
			// self.playback.set_adjustment(&Adjustment::new(
			// 	0., 0., duration as f64, 1., 0., 0.));
		}

		if let Some(art_url) = meta_props.lookup_value("mpris:artUrl", Some(&VariantTy::STRING))
				.map(|v| { v.get::<String>().expect("url type is string") }) {
			if self.data.borrow().art_url.as_ref().is_none_or(|url| { art_url != *url }) {
				self.data.borrow_mut().art_url = Some(art_url.clone());
				let _album_cover = self.album_cover.clone();
				MainContext::default().spawn_local(Self::set_art(_album_cover, art_url));
			}
		}
	}

	fn update_seek(&self, position: i64) {
		let mut data = self.data.borrow_mut();
		if let Some(clock) = self.playback.frame_clock() {
			data.position_base = clock.frame_time() - ((position as f64 / data.rate) as i64);
		} else {
			data.clock_needs_aligned = true;
		}
	}

	fn update_prop(&self, prop: &str, value: Variant) {
		match prop {
			"Metadata" => {
				let meta_props = VariantDict::from(value);
				self.update_metadata(meta_props);
			}
			"CanControl" => {
				self.data.borrow_mut().can_control = value.get::<bool>().unwrap_or(false);
				self.update_sensitivity();
			}
			"CanSeek" => {
				self.data.borrow_mut().can_seek = value.get::<bool>().unwrap_or(false);
				let data = self.data.borrow();
				if data.can_control {
					self.playback.set_sensitive(data.can_control && data.can_seek);
				}
			}
			"CanPause" => {
				self.data.borrow_mut().can_pause = value.get::<bool>().unwrap_or(false);
				let data = self.data.borrow();
				if data.play_state == PlayState::Playing && data.can_control {
					self.play_pause.set_sensitive(data.can_pause);
				}
			}
			"CanPlay" => {
				self.data.borrow_mut().can_play = value.get::<bool>().unwrap_or(false);
				let data = self.data.borrow();
				if data.play_state != PlayState::Playing && data.can_control {
					self.play_pause.set_sensitive(data.can_play);
				}
			}
			"CanGoNext" => {
				self.data.borrow_mut().can_go_next = value.get::<bool>().unwrap_or(false);
				let data = self.data.borrow();
				if data.can_control {
					self.next.set_sensitive(data.can_go_next);
				}
			}
			"CanGoPrevious" => {
				self.data.borrow_mut().can_go_prev = value.get::<bool>().unwrap_or(false);
				let data = self.data.borrow();
				if data.can_control {
					self.prev.set_sensitive(data.can_go_prev);
				}
			}
			"Shuffle" => {
				if { let data = self.data.borrow();
						data.can_control && !data.has_shuf } {
					self.shuf.set_sensitive(true);
				}
				{
					let mut data = self.data.borrow_mut();
					data.has_shuf = true;
					data.shuffle_state = value.get::<bool>().unwrap_or(false);
				}
				let data = self.data.borrow();
				self.shuf.block_signal(data.shuf_sig_id.as_ref().unwrap());
				self.shuf.set_active(data.shuffle_state);
				self.shuf.block_signal(data.shuf_sig_id.as_ref().unwrap());
			}
			"LoopStatus" => {
				if { let data = self.data.borrow();
						data.can_control && !data.has_loop } {
					self.loop_.set_sensitive(true);
				}
				{
					let mut data = self.data.borrow_mut();
					data.has_loop = true;
					data.loop_state = value.get::<String>().map(|s| { LoopState::try_from(s.as_str()).ok() })
						.flatten().unwrap_or(LoopState::None);
				}
				let data = self.data.borrow();
				let new_icon = Image::from_icon_name(Some(data.loop_state.icon_name()), waybar_cffi::gtk::IconSize::Button);
				self.loop_.set_image(Some(&new_icon));
				
				self.loop_.block_signal(data.loop_sig_id.as_ref().unwrap());
				self.loop_.set_active(data.loop_state.is_active());
				self.loop_.unblock_signal(data.loop_sig_id.as_ref().unwrap());
			}
			"PlaybackStatus" => {
				{
					let mut data = self.data.borrow_mut();
					data.play_state = value.get::<String>().map(|s| { PlayState::try_from(s.as_str()).ok() })
						.flatten().unwrap_or(PlayState::Stopped);
					data.clock_needs_aligned = true;
				}
				let data = self.data.borrow();
				let new_icon = Image::from_icon_name(Some(data.play_state.inverse().icon_name()), waybar_cffi::gtk::IconSize::Button);
				self.play_pause.set_image(Some(&new_icon));
				if data.can_control {
					self.play_pause.set_sensitive(if data.play_state == PlayState::Playing {data.can_pause} else {data.can_play});
				}
			}
			"MinimumRate" => {
				let min_rate = value.get::<f64>().unwrap_or(1.);
				self.rate_adj.set_lower(min_rate);
			}
			"MaximumRate" => {
				let max_rate = value.get::<f64>().unwrap_or(1.).min(4.);
				self.rate_adj.set_upper(max_rate);
			}
			"Rate" => {
				{
					let mut data = self.data.borrow_mut();
					data.rate = value.get::<f64>().unwrap_or(1.);
					data.clock_needs_aligned = true;
				}
				let data = self.data.borrow();
				if !data.is_dragging_rate {
					self.rate_adj.set_value(data.rate);
				}
			}
			"Volume" => {
				let volume = value.get::<f64>().unwrap_or(1.);
				if !self.data.borrow().is_dragging_volume {
					self.volume_adj.set_value(volume);
				}
			}
			other => {
				eprintln!("Unhandled property update on {}", other);
			}
		}
	}

	async fn set_art(album_cover: Rc<RefCell<Image>>, art_url: String) {
		let pixbuf = async {
			let session = soup::Session::new();
			let message = soup::Message::new("GET", &art_url).unwrap();

			// 2. Send the request and get the body bytes
			// Note: Soup3's send_and_read_async returns glib::Bytes directly
			let bytes = session
				.send_and_read_future(&message, glib::Priority::DEFAULT)
				.await
				.ok()?;

			// 3. Wrap bytes in a MemoryInputStream
			let stream = MemoryInputStream::from_bytes(&bytes);

			// 4. Load the Pixbuf from the stream
			Pixbuf::from_stream_future(&stream).await
				.ok().map(|p| {
					let dest_height = 64;
					let dest_width = p.width() * dest_height / p.height();
					p.scale_simple(dest_width, dest_height, gdk::gdk_pixbuf::InterpType::Bilinear)
				}).flatten()
		}.await;
		if let Some(data) = pixbuf {
			album_cover.borrow().set_from_pixbuf(Some(&data));
		}
	}

	fn call_fn(&self, method: &str, params: Option<&Variant>) {
		self.dbus_conn.upgrade().map(|p| {
			p.borrow().call(
				Some(&self.name),
				"/org/mpris/MediaPlayer2",
				"org.mpris.MediaPlayer2.Player",
				method,
				params,
				None,
				DBusCallFlags::NONE,
				-1,
				None::<&Cancellable>,
				|res| { let _ = res.map_err(|e| { eprintln!("dbus error {}", e) }); });
		});
	}

	fn set_prop(&self, field: &str, value: &Variant) {
		self.dbus_conn.upgrade().map(|p| {
			p.borrow().call(
				Some(&self.name),
				"/org/mpris/MediaPlayer2",
				"org.freedesktop.DBus.Properties",
				"Set",
				Some(&Variant::from(("org.mpris.MediaPlayer2.Player", field, value.to_variant()))),
				None,
				DBusCallFlags::NONE,
				-1,
				None::<&Cancellable>,
				|res| {
					let _ = res.map_err(|e| { eprintln!("dbus err on {}", e) });
				});
		});
	}

	fn get_prop(&self, field: &str) -> Option<Variant> {
		self.dbus_conn.upgrade().map(|p| {
			p.borrow().call_sync(
				Some(&self.name),
				"/org/mpris/MediaPlayer2",
				"org.freedesktop.DBus.Properties",
				"Get",
				Some(&Variant::from(("org.mpris.MediaPlayer2.Player", field))),
				Some(VariantTy::TUPLE),
				DBusCallFlags::NONE,
				-1,
				None::<&Cancellable>)
				.map_err(|e| { eprintln!("dbus err {}", e) }).ok()
				.map(|vt| { vt.child_get(0) })
		}).flatten()
	}

	fn initialize(self) -> Rc<Self> {
		let s = Rc::new(self);

		s.prev.connect_clicked(clone!(@weak s => move |_| {
			s.call_fn("Previous", None);
		}));
		s.play_pause.connect_clicked(clone!(@weak s => move |_| {
			s.call_fn("PlayPause", None);
		}));
		s.next.connect_clicked(clone!(@weak s => move |_| {
			s.call_fn("Next", None);
		}));
		s.data.borrow_mut().shuf_sig_id = Some(s.shuf.connect_clicked(clone!(@weak s => move |slf| {
			let shuffle_state = s.data.borrow().shuffle_state;

			slf.block_signal(s.data.borrow().shuf_sig_id.as_ref().unwrap());
			slf.set_active(shuffle_state); // reset state until change confirmed
			slf.unblock_signal(s.data.borrow().shuf_sig_id.as_ref().unwrap());

			s.set_prop("Shuffle", &Variant::from(!shuffle_state));
		})));
		s.data.borrow_mut().loop_sig_id = Some(s.loop_.connect_clicked(clone!(@weak s => move |slf| {
			let loop_state = &s.data.borrow().loop_state;

			slf.block_signal(s.data.borrow().loop_sig_id.as_ref().unwrap());
			slf.set_active(loop_state.is_active()); // reset state until change confirmed
			slf.unblock_signal(s.data.borrow().loop_sig_id.as_ref().unwrap());

			let next_state = Into::<&str>::into(loop_state.next());
			s.set_prop("LoopStatus", &Variant::from(next_state));
		})));
		// rate control
		s.rate_ctl.connect_change_value(clone!(@weak s => @default-return gdk::glib::Propagation::Proceed, move |_slf, _st, value| {
			s.set_prop("Rate", &Variant::from(value.max(0.01)));
			gdk::glib::Propagation::Proceed
		}));
		s.rate_ctl.connect_button_press_event(clone!(@weak s => @default-return gdk::glib::Propagation::Proceed, move |_slf, _e| {
			s.data.borrow_mut().is_dragging_rate = true;
			gdk::glib::Propagation::Proceed
		}));
		s.rate_ctl.connect_button_release_event(clone!(@weak s => @default-return gdk::glib::Propagation::Proceed, move |_slf, _e| {
			s.data.borrow_mut().is_dragging_rate = false;
			gdk::glib::Propagation::Proceed
		}));
		// volume control
		s.volume_ctl.connect_change_value(clone!(@weak s => @default-return gdk::glib::Propagation::Proceed, move |_slf, _st, value| {
			s.set_prop("Volume", &Variant::from(value));
			gdk::glib::Propagation::Proceed
		}));
		s.volume_ctl.connect_button_press_event(clone!(@weak s => @default-return gdk::glib::Propagation::Proceed, move |_slf, _e| {
			s.data.borrow_mut().is_dragging_volume = true;
			gdk::glib::Propagation::Proceed
		}));
		s.volume_ctl.connect_button_release_event(clone!(@weak s => @default-return gdk::glib::Propagation::Proceed, move |_slf, _e| {
			s.data.borrow_mut().is_dragging_volume = false;
			gdk::glib::Propagation::Proceed
		}));
		// seek control
		s.playback.connect_change_value(clone!(@weak s => @default-return gdk::glib::Propagation::Proceed, move |_slf, _st, value| {
			let position = value as i64;
			if let Some(trackid) = &s.data.borrow().current_track {
				s.call_fn("SetPosition", Some(&Variant::from((trackid, position))));
			}
			gdk::glib::Propagation::Proceed
		}));
		s.playback.connect_button_press_event(clone!(@weak s => @default-return gdk::glib::Propagation::Proceed, move |_slf, _e| {
			s.data.borrow_mut().is_dragging_playback = true;
			gdk::glib::Propagation::Proceed
		}));
		s.playback.connect_button_release_event(clone!(@weak s => @default-return gdk::glib::Propagation::Proceed, move |_slf, _e| {
			s.data.borrow_mut().is_dragging_playback = false;
			gdk::glib::Propagation::Proceed
		}));
		s.playback.add_tick_callback(clone!(@weak s => @default-return glib::ControlFlow::Continue, move |_pb, fc| {
			let rate = s.data.borrow().rate;
			let position = {
				let mut data = s.data.borrow_mut();
				if data.is_dragging_playback || data.play_state != PlayState::Playing { return glib::ControlFlow::Continue; }
				let now = fc.frame_time();
				if data.clock_needs_aligned {
					let position = s.get_prop("Position")
						.map(|v| { v.get::<i64>() })
						.flatten().unwrap_or(0);
					data.position_base = now - ((position as f64 / rate) as i64);
					data.clock_needs_aligned = false;
					position
				} else {
					now - data.position_base
				}
			};
			s.playback_adj.set_value(position as f64 * rate);
			glib::ControlFlow::Continue
		}));
		s
	}
}

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
							let player = PlayerWidget::new(dbus.clone(), new_owner.clone()).initialize();
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
						let player = PlayerWidget::new(dbus.clone(), sender.to_string()).initialize();
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
						let player = PlayerWidget::new(dbus.clone(), sender.to_string()).initialize();
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
