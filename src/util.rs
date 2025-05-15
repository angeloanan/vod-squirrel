/// Truncates a string to a maximum length, adding `...` to the end if it was truncated.
///
/// This function will continuously try to reduce length if string is being
/// truncated in the middle of a UTF codepoint
///
/// # Arguments
/// * `string` - The string to truncate
/// * `max_length` - The maximum length of the string
///
/// # Panics
/// Should never panic, unwrap is safe.
pub fn truncate_string(string: &impl ToString, max_length: usize) -> String {
    let string = string.to_string();
    if string.len() <= max_length {
        return string;
    }

    let mut attempted_len = max_length;
    let mut truncated = string.get(..attempted_len - 3);
    while truncated.is_none() {
        attempted_len -= 1;
        truncated = string.get(..attempted_len - 3);
    }

    // SAFETY: Should never panic due to the above
    format!("{}...", truncated.unwrap())
}
