
pub fn truncate_string(mut string: String, max_length: usize) -> String {
	if max_length > 0 && string.len() > max_length {
		string.truncate(string.floor_char_boundary(max_length - 3));
		string + "..."
	} else {
		string
	}
}

pub fn truncate_str(string: &str, max_length: usize) -> String {
	if max_length > 0 && string.len() > max_length {
		string.split_at(string.floor_char_boundary(max_length - 3)).0.to_owned() + "..."
	} else {
		string.to_owned()
	}
}
