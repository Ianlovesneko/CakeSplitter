use std::{fmt::Write as _, path::Path};

pub fn terminal_safe(text: &str) -> String {
    let mut safe = String::with_capacity(text.len());
    for character in text.chars() {
        if is_bidi_control(character) {
            write!(&mut safe, "\\u{{{:x}}}", character as u32)
                .expect("writing to a String cannot fail");
        } else if character.is_control() {
            safe.extend(character.escape_default());
        } else {
            safe.push(character);
        }
    }
    safe
}

pub fn json_terminal_safe(text: &str) -> String {
    let mut safe = String::with_capacity(text.len());
    for character in text.chars() {
        if is_bidi_control(character) {
            write!(&mut safe, "\\u{:04x}", character as u32)
                .expect("writing to a String cannot fail");
        } else {
            safe.push(character);
        }
    }
    safe
}

pub fn terminal_path(path: &Path) -> String {
    terminal_safe(&path.display().to_string())
}

pub fn is_bidi_control(character: char) -> bool {
    matches!(
        character,
        '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_terminal_controls_without_hiding_unicode() {
        let safe = terminal_safe("生日蛋糕\u{1b}\r\n\u{202e}");
        assert_eq!(safe, "生日蛋糕\\u{1b}\\r\\n\\u{202e}");
        assert!(!safe.chars().any(char::is_control));
        assert!(!safe.chars().any(is_bidi_control));
    }
}
