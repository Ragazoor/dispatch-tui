//! Single-line text-field caret mechanics shared by every `InputMode` text
//! field (task title, todo title, epic title, base branch, repo-path /
//! quick-dispatch query, filter-preset name).
//!
//! The caret is a **character** index into the buffer — the count of chars to
//! the left of the caret — with invariant `0 <= caret <= buffer.chars().count()`.
//! Storing a char index (rather than a raw byte offset) keeps clamping and word
//! motion simple and makes it impossible for the caret to land mid-codepoint;
//! we convert to a byte offset only at the `String::insert`/`remove` and render
//! call sites via [`byte_offset`].

/// Number of chars in `buf` — the maximum valid caret value.
pub fn len(buf: &str) -> usize {
    buf.chars().count()
}

/// Byte offset of the char-caret `caret` within `buf`.
///
/// Returns `buf.len()` when the caret is at (or past) the end — the common
/// "caret at end" case and every empty buffer. Never panics: callers pass the
/// result straight to `String::insert`/`String::remove`.
pub fn byte_offset(buf: &str, caret: usize) -> usize {
    buf.char_indices()
        .nth(caret)
        .map(|(b, _)| b)
        .unwrap_or(buf.len())
}

/// Insert `c` at the caret, returning the advanced caret.
pub fn insert(buf: &mut String, caret: usize, c: char) -> usize {
    let at = byte_offset(buf, caret);
    buf.insert(at, c);
    caret + 1
}

/// Delete the char immediately left of the caret (Backspace). No-op returning
/// `0` when the caret is already at the start.
pub fn delete_before(buf: &mut String, caret: usize) -> usize {
    if caret == 0 {
        return 0;
    }
    let at = byte_offset(buf, caret - 1);
    buf.remove(at);
    caret - 1
}

/// Delete the char at the caret (Delete / forward-delete). No-op returning the
/// caret unchanged when the caret is at the end.
pub fn delete_after(buf: &mut String, caret: usize) -> usize {
    if caret >= len(buf) {
        return caret;
    }
    let at = byte_offset(buf, caret);
    buf.remove(at);
    caret
}

/// Move one char left, clamped at the start.
pub fn move_left(caret: usize) -> usize {
    caret.saturating_sub(1)
}

/// Move one char right, clamped at the end.
pub fn move_right(buf: &str, caret: usize) -> usize {
    (caret + 1).min(len(buf))
}

/// Jump to the start of the buffer.
pub fn home() -> usize {
    0
}

/// Jump to the end of the buffer.
pub fn end(buf: &str) -> usize {
    len(buf)
}

/// A "word" character for word-motion purposes: alphanumeric or `_`.
///
/// Everything else — whitespace **and** punctuation/path separators (`/`, `-`,
/// `.`, …) — is a boundary. This matches the default word motion in readline,
/// emacs, and most editors, and in particular makes `Ctrl+Left/Right` step
/// through path segments (`/home/user/project`) and hyphenated names
/// (`foo-bar`) rather than jumping the whole field in one keystroke.
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Move left to the start of the previous word: skip any boundary chars, then
/// skip the word chars. See [`is_word_char`] for what counts as a word.
pub fn word_left(buf: &str, caret: usize) -> usize {
    let chars: Vec<char> = buf.chars().collect();
    let mut i = caret.min(chars.len());
    while i > 0 && !is_word_char(chars[i - 1]) {
        i -= 1;
    }
    while i > 0 && is_word_char(chars[i - 1]) {
        i -= 1;
    }
    i
}

/// Move right to the start of the next word: skip the current word chars, then
/// skip the following boundary chars. See [`is_word_char`].
pub fn word_right(buf: &str, caret: usize) -> usize {
    let chars: Vec<char> = buf.chars().collect();
    let n = chars.len();
    let mut i = caret.min(n);
    while i < n && is_word_char(chars[i]) {
        i += 1;
    }
    while i < n && !is_word_char(chars[i]) {
        i += 1;
    }
    i
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn insert_mid_string_advances_caret() {
        let mut buf = "ac".to_string();
        let caret = insert(&mut buf, 1, 'b');
        assert_eq!(buf, "abc");
        assert_eq!(caret, 2);
    }

    #[test]
    fn insert_at_end_and_start() {
        let mut buf = "bc".to_string();
        let caret = insert(&mut buf, 0, 'a');
        assert_eq!(buf, "abc");
        assert_eq!(caret, 1);
        let end_caret = len(&buf);
        let caret = insert(&mut buf, end_caret, 'd');
        assert_eq!(buf, "abcd");
        assert_eq!(caret, 4);
    }

    #[test]
    fn delete_before_removes_left_char() {
        let mut buf = "abc".to_string();
        let caret = delete_before(&mut buf, 2);
        assert_eq!(buf, "ac");
        assert_eq!(caret, 1);
    }

    #[test]
    fn delete_before_at_start_is_noop() {
        let mut buf = "abc".to_string();
        let caret = delete_before(&mut buf, 0);
        assert_eq!(buf, "abc");
        assert_eq!(caret, 0);
    }

    #[test]
    fn delete_after_removes_char_at_caret() {
        let mut buf = "abc".to_string();
        let caret = delete_after(&mut buf, 1);
        assert_eq!(buf, "ac");
        assert_eq!(caret, 1);
    }

    #[test]
    fn delete_after_at_end_is_noop() {
        let mut buf = "abc".to_string();
        let caret = delete_after(&mut buf, 3);
        assert_eq!(buf, "abc");
        assert_eq!(caret, 3);
    }

    #[test]
    fn move_left_right_clamp() {
        assert_eq!(move_left(0), 0);
        assert_eq!(move_left(2), 1);
        let buf = "ab";
        assert_eq!(move_right(buf, 2), 2);
        assert_eq!(move_right(buf, 0), 1);
    }

    #[test]
    fn home_and_end() {
        assert_eq!(home(), 0);
        assert_eq!(end("abc"), 3);
        assert_eq!(end(""), 0);
    }

    #[test]
    fn word_left_across_multiple_spaces() {
        let buf = "foo   bar";
        // caret at end -> start of "bar"
        assert_eq!(word_left(buf, 9), 6);
        // from start of "bar" -> start of "foo"
        assert_eq!(word_left(buf, 6), 0);
        assert_eq!(word_left(buf, 0), 0);
    }

    #[test]
    fn word_right_across_multiple_spaces() {
        let buf = "foo   bar";
        // from start -> start of "bar" (skips "foo" then the spaces)
        assert_eq!(word_right(buf, 0), 6);
        // from start of "bar" -> end
        assert_eq!(word_right(buf, 6), 9);
        assert_eq!(word_right(buf, 9), 9);
    }

    #[test]
    fn word_motion_breaks_on_punctuation() {
        let buf = "foo-bar baz";
        // hyphen is a boundary: from 0, word_right stops at the start of "bar"
        assert_eq!(word_right(buf, 0), 4);
        // from the end, word_left steps back to the start of "baz"
        assert_eq!(word_left(buf, 11), 8);
        // then to the start of "bar"
        assert_eq!(word_left(buf, 8), 4);
    }

    #[test]
    fn word_motion_steps_through_path_segments() {
        let buf = "/home/user/proj";
        // from the end, word_left lands at the start of the last segment
        assert_eq!(word_left(buf, 15), 11);
        assert_eq!(word_left(buf, 11), 6);
        assert_eq!(word_left(buf, 6), 1);
        // from the start, word_right advances segment by segment
        assert_eq!(word_right(buf, 0), 1);
        assert_eq!(word_right(buf, 1), 6);
    }

    #[test]
    fn multibyte_insert_delete_and_move() {
        // "é" is 2 bytes, "ü" is 2 bytes — caret is char-based throughout.
        let mut buf = "éü".to_string();
        assert_eq!(len(&buf), 2);
        // insert between the two multibyte chars
        let caret = insert(&mut buf, 1, 'x');
        assert_eq!(buf, "éxü");
        assert_eq!(caret, 2);
        // move across and delete the char before the caret
        let caret = move_left(caret);
        assert_eq!(caret, 1);
        let caret = delete_before(&mut buf, caret);
        assert_eq!(buf, "xü");
        assert_eq!(caret, 0);
    }

    #[test]
    fn byte_offset_end_of_buffer_does_not_panic() {
        // ASCII: caret == len -> byte len
        assert_eq!(byte_offset("abc", 3), 3);
        // multibyte: caret == len -> total byte length, not char count
        let s = "éü"; // 4 bytes, 2 chars
        assert_eq!(byte_offset(s, 2), 4);
        assert_eq!(byte_offset(s, 1), 2);
        // empty buffer
        assert_eq!(byte_offset("", 0), 0);
    }
}
