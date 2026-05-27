use std::sync::LazyLock;

use regex::Regex;
use waybar_cffi::serde;

static NEVER_REGEX: LazyLock<Regex> = LazyLock::new(|| {
	Regex::new(r"^[]$").unwrap()
});

static DEFAULT_IGNORED_REGEX: LazyLock<Regex> = LazyLock::new(|| {
	Regex::new(r"\bplayerctld$").unwrap()
});

#[derive(serde::Deserialize)]
pub struct Config {
	ignored_players: Option<DeserialRegex>,
	max_title_length: Option<usize>,
	max_subtitle_length: Option<usize>,
}

struct DeserialRegex(Regex);

impl From<Regex> for DeserialRegex {
	fn from(value: Regex) -> Self {
		return Self(value)
	}
}
impl<'a> Into<&'a Regex> for &'a DeserialRegex {
	fn into(self) -> &'a Regex {
		&self.0
	}
}

impl<'de> serde::Deserialize<'de> for DeserialRegex {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where
		D: serde::Deserializer<'de> {
		<&str>::deserialize(deserializer)
			.map_or_else(|e| { Err(e) }, |rx| {
				Regex::new(rx)
					.map(DeserialRegex::from)
					.map_err(serde::de::Error::custom)
			})
	}
}

impl Config {
	pub fn ignored_players(&self) -> &Regex {
		self.ignored_players.as_ref()
			.map_or_else(|| { &*DEFAULT_IGNORED_REGEX }, <&DeserialRegex>::into)
	}

	pub fn max_title_length(&self) -> usize { self.max_title_length.unwrap_or(50) }
	pub fn max_subtitle_length(&self) -> usize { self.max_subtitle_length.unwrap_or(50) }
}
