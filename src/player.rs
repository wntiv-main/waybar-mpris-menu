use std::{cell::RefCell, rc::{Rc, Weak}};

use gdk::glib::{SignalHandlerId, clone};
use soup::prelude::*;
use waybar_cffi::{
	gtk::{
		Adjustment, Box as GtkBox, Button, Image, Label, Scale, ToggleButton,
		gdk_pixbuf::Pixbuf,
		gio::{MemoryInputStream},
		glib::{self, MainContext, Variant, VariantDict, VariantTy, clone::Downgrade, variant::ObjectPath},
		prelude::WidgetExtManual,
		traits::{AdjustmentExt, ButtonExt, ContainerExt, ImageExt, LabelExt, RangeExt, ScaleExt, ToggleButtonExt, WidgetExt}
	},
};

use crate::{player_manager::PlayerManager, player_model::{ LoopState, PlayState }};

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

pub struct PlayerWidget {
	manager: Weak<PlayerManager>,
	name: String,
	
	data: Rc<RefCell<PlayerData>>,

	playback_adj: Adjustment,
	volume_adj: Adjustment,
	rate_adj: Adjustment,

	root: GtkBox,
	title: Label,
	album_artist: Label,
	album_cover: Image,
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
	pub fn get_root<'a>(&'a self) -> &'a GtkBox {
		return &self.root
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

	pub fn update_seek(&self, position: i64) {
		let mut data = self.data.borrow_mut();
		if let Some(clock) = self.playback.frame_clock() {
			data.position_base = clock.frame_time() - ((position as f64 / data.rate) as i64);
		} else {
			data.clock_needs_aligned = true;
		}
	}

	pub fn update_prop(&self, prop: &str, value: Variant) {
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

	async fn set_art(album_cover: Image, art_url: String) {
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
			album_cover.set_from_pixbuf(Some(&data));
		}
	}

	fn call_fn(&self, method: &str, params: Option<&Variant>) {
		self.manager.upgrade().inspect(|pm| {
			let cb: Box<dyn FnOnce(Result<Variant, glib::Error>)> = if cfg!(debug_assertions) {
				let name = self.name.to_string();
				let method = method.to_string();
				let params_str = params.map_or("".to_string(), |p| {
					p.print(true).to_string()
				});
				Box::new(move |res| {
					let _ = res.inspect_err(move |e| {
						eprintln!("Error calling {}({}) as {}: {}", method, params_str, name, e);
					});
				})
			} else {Box::new(|_res| {})};
			pm.call_player_fn(
				&self.name,
				method,
				params,
				cb);
		});
	}

	fn get_prop(&self, field: &str) -> Option<Variant> {
		self.manager.upgrade().map(|pm| {
			match pm.get_player_prop(&self.name, field) {
				Ok(vt) => { Some(vt.child_get(0)) }
				Err(error) => {
					if cfg!(debug_assertions) {
						eprintln!("Error getting {} from {}: {}", field, self.name, error);
					}
					None
				}
			}
		}).flatten()
	}

	fn set_prop(&self, field: &str, value: &Variant) {
		self.manager.upgrade().map(|pm| {
			let cb: Box<dyn FnOnce(Result<Variant, glib::Error>)> = if cfg!(debug_assertions) {
				let name = self.name.to_string();
				let field = field.to_string();
				let value_str = value.print(true).to_string();
				Box::new(move |res| {
					let _ = res.inspect_err(move |e| {
						eprintln!("Error setting {} to {} on {}: {}", field, value_str, name, e);
					});
				})
			} else {Box::new(|_res| {})};
			pm.set_player_prop(
				&self.name,
				field,
				value,
				cb);
		});
	}

	// Need to return an Rc because we need to hold weak references to most our fields for callbacks
	pub fn new(manager: &Rc<PlayerManager>, inst_name: String) -> Result<Rc<Self>, glib::Error> {
		// Initial state
		let resp = manager.get_all_player_data(&inst_name)?;
		let props = VariantDict::from(resp.child_value(0));
		let meta_props = props.lookup_value("Metadata", Some(&VariantTy::VARDICT))
			.map(VariantDict::from);
		let play_state: PlayState = props.lookup_value("PlaybackStatus", Some(VariantTy::STRING))
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
			manager: manager.downgrade(),
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
			album_cover: album_cover,
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
		Ok(result.apply_event_listeners())
	}

	fn apply_event_listeners(self) -> Rc<Self> {
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