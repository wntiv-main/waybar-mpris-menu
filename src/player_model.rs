#[derive(PartialEq, Eq)]
pub enum PlayState {
	Stopped,
	Playing,
	Paused,
}

impl PlayState {
	pub fn inverse(&self) -> Self {
		match self {
			Self::Stopped => Self::Playing,
			Self::Playing => Self::Paused,
			Self::Paused => Self::Playing,
		}
	}

	pub fn icon_name(&self) -> &'static str {
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
pub enum LoopState {
	None,
	LoopPlaylist,
	LoopSingle,
}

impl LoopState {
	pub fn next(&self) -> Self {
		match self {
			Self::None => Self::LoopPlaylist,
			Self::LoopPlaylist => Self::LoopSingle,
			Self::LoopSingle => Self::None,
		}
	}

	pub fn icon_name(&self) -> &'static str {
		match self {
			Self::None => "media-playlist-repeat",
			Self::LoopPlaylist => "media-playlist-repeat",
			Self::LoopSingle => "media-playlist-repeat-song",
		}
	}

	pub fn is_active(&self) -> bool {
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

#[derive(Clone, Copy)]
pub enum PlayerInterface {
	Root,
	Player,
	TrackList,
	Playlists,
	Ext,
}

impl Into<&str> for PlayerInterface {
	fn into(self) -> &'static str {
		match self {
			PlayerInterface::Root => "org.mpris.MediaPlayer2",
			PlayerInterface::Player => "org.mpris.MediaPlayer2.Player",
			PlayerInterface::TrackList => "org.mpris.MediaPlayer2.TrackList",
			PlayerInterface::Playlists => "org.mpris.MediaPlayer2.Playlists",
			PlayerInterface::Ext => "com.github.wntiv.MediaPlayer2",
		}
	}
}

impl TryFrom<&str> for PlayerInterface {
	type Error = ();

	fn try_from(value: &str) -> Result<Self, ()> {
		match value {
			"org.mpris.MediaPlayer2" => Ok(Self::Root),
			"org.mpris.MediaPlayer2.Player" => Ok(Self::Player),
			"org.mpris.MediaPlayer2.TrackList" => Ok(Self::TrackList),
			"org.mpris.MediaPlayer2.Playlists" => Ok(Self::Playlists),
			"com.github.wntiv.MediaPlayer2" => Ok(Self::Ext),
			_ => Err(())
		}
	}
}

