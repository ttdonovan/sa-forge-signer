//! Caller-supplied text (a payload `summary`, an `intent`, a halt `reason`), made safe to print on a
//! terminal and bounded before it is stored.

/// Most characters of caller text a terminal prompt prints.
pub const MAX_PRINTED: usize = 200;

/// `text` safe to print on a terminal, at most `max` characters of it: control and bidirectional
/// formatting characters are shown escaped, so caller text cannot move the cursor or reorder a prompt.
pub fn printable(text: &str, max: usize) -> String {
    let mut out = String::new();
    let mut chars = text.chars();
    for c in chars.by_ref().take(max) {
        if c.is_control() || is_bidi(c) {
            out.extend(c.escape_default());
        } else {
            out.push(c);
        }
    }
    if chars.next().is_some() {
        out.push_str(" [truncated]");
    }
    out
}

const fn is_bidi(c: char) -> bool {
    matches!(c, '\u{061C}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

/// Most characters of caller text (`summary`, `intent`) an entry keeps.
pub const MAX_TEXT: usize = 500;

/// At most [`MAX_TEXT`] characters of `text`, marked when cut, so caller text cannot grow the log
/// that every sign reads back.
pub fn clip(text: &str) -> String {
    let mut chars = text.chars();
    let mut out: String = chars.by_ref().take(MAX_TEXT).collect();
    if chars.next().is_some() {
        out.push_str(" [truncated]");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caller_text_cannot_steer_the_terminal() {
        assert_eq!(printable("plain text", 50), "plain text");
        // Cursor up and erase line, a newline, a C1 CSI, DEL and a right-to-left override.
        let shown = printable("ok\u{1b}[1A\u{1b}[2K\nfake\u{9b}2K\u{7f}\u{202e}evil", 100);
        assert!(
            !shown.chars().any(|c| c.is_control() || is_bidi(c)),
            "{shown:?}"
        );
        assert!(
            shown.contains("\\u{1b}[1A") && shown.contains("\\n"),
            "{shown:?}"
        );
        assert_eq!(printable("abcdef", 3), "abc [truncated]");
        assert_eq!(printable("abc", 3), "abc");
    }

    #[test]
    fn clip_cuts_and_marks_long_text() {
        let long = "x".repeat(MAX_TEXT + 10);
        let cut = clip(&long);
        assert!(cut.starts_with(&"x".repeat(MAX_TEXT)) && cut.ends_with(" [truncated]"));
        assert_eq!(clip("short"), "short");
    }
}
