//! DSH-style session title helpers.
//!
//! Defaults mirrored from DSH:
//!
//! * MAX_TITLE_BYTES = 80 for a model generated title,
//! * FALLBACK_MAX_WORDS = 5 and FALLBACK_MAX_BYTES = 40 for the deterministic
//!   fallback title.
//!
//! Every entry point first strips terminal control sequences, C0/C1 control
//! characters and invisible formatting characters, then collapses all
//! whitespace to a single space and trims the result.
//!
//! The module is std-only on purpose (no serde, no regex, no other crate) so it
//! can be unit tested on its own and mounted from the Tauri command layer or
//! from storage.

/// Maximum length, in UTF-8 bytes, of a model generated session title.
///
/// DSH default: 80.
pub const MAX_TITLE_BYTES: usize = 80;

/// Maximum number of space separated words kept by the fallback title.
///
/// DSH default: 5.
pub const FALLBACK_MAX_WORDS: usize = 5;

/// Maximum length, in UTF-8 bytes, of a deterministic fallback title.
///
/// DSH default: 40.
pub const FALLBACK_MAX_BYTES: usize = 40;

/// Minimal terminal parser states used while scrubbing a title string.
#[derive(Clone, Copy, PartialEq, Eq)]
enum CleanState {
    /// Normal text.
    Ground,
    /// Just saw ESC; the next character selects the sequence kind.
    Escape,
    /// Inside an ESC + intermediate byte sequence such as ESC ( B.
    EscapeIntermediate,
    /// Inside a CSI (Control Sequence Introducer) sequence.
    Csi,
    /// Inside an OSC / DCS / SOS / PM / APC string sequence.
    Osc,
    /// Inside a string sequence and just saw ESC (possible ESC backslash = ST).
    OscEscape,
}

/// Returns true for invisible formatting characters that must never leak into a
/// session title (zero width spaces/joiners, bidi controls, BOM, soft hyphen).
fn is_invisible_control(ch: char) -> bool {
    matches!(
        ch,
        '\u{00ad}'                 // SOFT HYPHEN
        | '\u{061c}'               // ARABIC LETTER MARK
        | '\u{180e}'               // MONGOLIAN VOWEL SEPARATOR
        | '\u{200b}'..='\u{200f}'  // ZWSP, ZWNJ, ZWJ, LRM, RLM
        | '\u{202a}'..='\u{202e}'  // bidi embeddings / overrides
        | '\u{2060}'..='\u{2064}'  // WORD JOINER ... INVISIBLE PLUS
        | '\u{2066}'..='\u{206f}'  // bidi isolates ... deprecated format chars
        | '\u{feff}'               // ZERO WIDTH NO-BREAK SPACE / BOM
        | '\u{fff9}'..='\u{fffb}'  // interlinear annotation controls
    )
}

/// Scrubs a raw title candidate.
///
/// Removes OSC/CSI/ESC terminal control sequences, C0/C1 control characters and
/// invisible formatting characters, then collapses every run of whitespace into
/// a single space and trims the result. Returns an empty string when nothing
/// printable remains.
pub fn clean_title_text(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut state = CleanState::Ground;
    let mut pending_space = false;

    for ch in input.chars() {
        match state {
            CleanState::Ground => {
                if ch == '\u{1b}' {
                    state = CleanState::Escape;
                } else if ch == '\u{9b}' {
                    // 8-bit CSI.
                    state = CleanState::Csi;
                } else if ch == '\u{9d}' {
                    // 8-bit OSC.
                    state = CleanState::Osc;
                } else if ch.is_whitespace() {
                    pending_space = true;
                } else if ch.is_control() || is_invisible_control(ch) {
                    // Drop control / invisible characters (BEL, NUL, DEL, C1
                    // controls, ZWSP, BOM, bidi overrides ...).
                } else {
                    if pending_space && !out.is_empty() {
                        out.push(' ');
                    }
                    pending_space = false;
                    out.push(ch);
                }
            }
            CleanState::Escape => {
                state = match ch {
                    '[' => CleanState::Csi,
                    ']' => CleanState::Osc,
                    // DCS, SOS, PM and APC are string sequences like OSC.
                    'P' | 'X' | '^' | '_' => CleanState::Osc,
                    '\u{20}'..='\u{2f}' => CleanState::EscapeIntermediate,
                    _ => CleanState::Ground,
                };
            }
            CleanState::EscapeIntermediate => {
                state = match ch {
                    '\u{20}'..='\u{2f}' => CleanState::EscapeIntermediate,
                    '\u{30}'..='\u{7e}' => CleanState::Ground,
                    _ => CleanState::Ground,
                };
            }
            CleanState::Csi => {
                // Parameter bytes 0x30..=0x3F, intermediate bytes 0x20..=0x2F,
                // final byte 0x40..=0x7E. Anything unexpected is consumed too so
                // a malformed sequence never leaks printable characters.
                state = match ch {
                    '\u{30}'..='\u{3f}' | '\u{20}'..='\u{2f}' => CleanState::Csi,
                    '\u{40}'..='\u{7e}' => CleanState::Ground,
                    '\u{1b}' => CleanState::Escape,
                    _ => CleanState::Csi,
                };
            }
            CleanState::Osc => match ch {
                '\u{07}' | '\u{9c}' => state = CleanState::Ground, // BEL / ST
                '\u{1b}' => state = CleanState::OscEscape,
                _ => {}
            },
            CleanState::OscEscape => {
                state = match ch {
                    '\\' => CleanState::Ground, // ESC backslash = ST
                    '\u{1b}' => CleanState::OscEscape,
                    _ => CleanState::Osc,
                };
            }
        }
    }

    out
}

/// Truncates input to at most max_bytes UTF-8 bytes without ever splitting a
/// Unicode code point. Returns a copy of the input when it already fits.
pub fn truncate_title_utf8(input: &str, max_bytes: usize) -> String {
    if input.len() <= max_bytes {
        return input.to_string();
    }
    let mut end = max_bytes;
    while end > 0 && !input.is_char_boundary(end) {
        end -= 1;
    }
    input[..end].to_string()
}

/// Cleans a raw title candidate and caps it at MAX_TITLE_BYTES.
///
/// May return an empty string when the input holds no printable text.
pub fn normalize_session_title(input: &str) -> String {
    let cleaned = clean_title_text(input);
    let truncated = truncate_title_utf8(&cleaned, MAX_TITLE_BYTES);
    truncated.trim_end().to_string()
}

/// Builds the deterministic fallback title used when the model produced nothing
/// usable: cleaned text, first FALLBACK_MAX_WORDS space separated words, then
/// capped at FALLBACK_MAX_BYTES bytes and trimmed.
///
/// For text without spaces (for example a Chinese sentence) this is equivalent
/// to a 40 byte UTF-8 truncation (about 13 CJK characters).
pub fn fallback_session_title(input: &str) -> String {
    let cleaned = clean_title_text(input);
    let joined = cleaned
        .split(' ')
        .filter(|word| !word.is_empty())
        .take(FALLBACK_MAX_WORDS)
        .collect::<Vec<&str>>()
        .join(" ");
    let truncated = truncate_title_utf8(&joined, FALLBACK_MAX_BYTES);
    truncated.trim_end().to_string()
}

/// Removes emphasis asterisks wrapping the whole line (*title*, **title**,
/// ***title***). The line is returned unchanged when the asterisk runs are
/// longer than three or when the inner text itself still contains an asterisk,
/// so double-emphasised text such as **a*b** is preserved verbatim.
///
/// A line made only of asterisks is stripped to an empty string: there is no
/// title to recover from it.
fn strip_wrapping_asterisks(line: &str) -> &str {
    let leading = line.bytes().take_while(|byte| *byte == b'*').count();
    if !(1..=3).contains(&leading) {
        return line;
    }
    let trailing = line.bytes().rev().take_while(|byte| *byte == b'*').count();
    if !(1..=3).contains(&trailing) {
        return line;
    }
    // Asterisks are single bytes, so leading / trailing are valid byte offsets.
    let total_chars = line.chars().count();
    if leading + trailing >= total_chars {
        return "";
    }
    let inner = &line[leading..line.len() - trailing];
    if inner.contains('*') {
        return line;
    }
    inner.trim()
}

/// Parses a model generated title.
///
/// Takes the first non-empty line (split on newline, then trimmed), unwraps a
/// whole-line emphasis marker, normalizes to MAX_TITLE_BYTES and returns None
/// when no printable title remains.
pub fn title_from_model_output(text: &str) -> Option<String> {
    let line = text
        .split('\n')
        .map(str::trim)
        .find(|line| !line.is_empty())?;
    let normalized = normalize_session_title(strip_wrapping_asterisks(line));
    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

/// Prompt used to ask a model for a short session title (DSH style).
///
/// Deliberately a single line: the reply itself must be one plain line without
/// Markdown, XML, quotes, prefixes or explanations.
pub fn title_system_prompt() -> &'static str {
    "Create a concise title for an AI coding-assistant session from the supplied human messages. \
Return only the title on one line, plain text, no quotes/prefix/explanation/Markdown/XML/control codes; \
use the language of the messages; aim for about 5 words or 10 CJK characters; \
if little to name, return a short best-effort title such as Greeting instead of explaining."
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dsh_default_constants_match() {
        assert_eq!(MAX_TITLE_BYTES, 80);
        assert_eq!(FALLBACK_MAX_WORDS, 5);
        assert_eq!(FALLBACK_MAX_BYTES, 40);
    }

    #[test]
    fn strips_csi_sequences() {
        assert_eq!(clean_title_text("\u{1b}[31mhello\u{1b}[0m"), "hello");
        assert_eq!(clean_title_text("a\u{1b}[2Jb\u{1b}[Hc"), "abc");
        assert_eq!(clean_title_text("a\u{9b}31mb"), "ab");
        // Unterminated CSI must not leak its payload.
        assert_eq!(clean_title_text("tail\u{1b}[31m"), "tail");
    }

    #[test]
    fn strips_osc_and_escape_sequences() {
        assert_eq!(
            clean_title_text("before\u{1b}]0;window title\u{07}after"),
            "beforeafter"
        );
        assert_eq!(
            clean_title_text("a\u{1b}]8;;https://example.com\u{1b}\\b"),
            "ab"
        );
        assert_eq!(clean_title_text("a\u{9d}window\u{07}b"), "ab");
        assert_eq!(clean_title_text("a\u{1b}Pq#0;2;0;0;0\u{1b}\\b"), "ab");
        assert_eq!(clean_title_text("a\u{1b}(Bb"), "ab");
        assert_eq!(clean_title_text("\u{1b}7abc\u{1b}8"), "abc");
        assert_eq!(clean_title_text("tail\u{1b}"), "tail");
    }

    #[test]
    fn drops_control_and_invisible_characters() {
        assert_eq!(
            clean_title_text("a\u{0007}\u{0000}\u{007f}b\u{200b}\u{feff}\u{202e}c"),
            "abc"
        );
        assert_eq!(clean_title_text("co\u{00ad}op"), "coop");
        assert_eq!(clean_title_text("a\u{200d}b"), "ab");
        assert_eq!(clean_title_text("\u{fff9}note\u{fffb}"), "note");
    }

    #[test]
    fn collapses_whitespace_and_trims() {
        assert_eq!(clean_title_text("  hello \t\n\r  world  "), "hello world");
        assert_eq!(clean_title_text("a\u{00a0}\u{3000}b"), "a b");
        assert_eq!(clean_title_text("\t \n "), "");
        assert_eq!(clean_title_text(""), "");
    }

    #[test]
    fn truncates_utf8_without_splitting_codepoints() {
        assert_eq!(
            truncate_title_utf8("\u{1f44d}\u{1f44d}\u{1f44d}", 5),
            "\u{1f44d}"
        );
        assert_eq!(
            truncate_title_utf8("\u{1f44d}\u{1f44d}\u{1f44d}", 12),
            "\u{1f44d}\u{1f44d}\u{1f44d}"
        );
        assert_eq!(truncate_title_utf8("abc", 0), "");
        assert_eq!(truncate_title_utf8("abc", 3), "abc");
        assert_eq!(truncate_title_utf8("abc", 80), "abc");
        assert_eq!(truncate_title_utf8("\u{4f60}\u{597d}", 4), "\u{4f60}");
        // Only 2 of the 3 bytes of one CJK character fit, so nothing is kept.
        assert_eq!(truncate_title_utf8("\u{4f60}\u{597d}", 2), "");
    }

    #[test]
    fn normalize_cleans_and_caps_at_80_bytes() {
        assert_eq!(
            normalize_session_title("  \u{1b}[31m Hello \u{1b}[0m  World  "),
            "Hello World"
        );
        assert_eq!(normalize_session_title(""), "");
        assert_eq!(normalize_session_title("\u{1b}[0m\u{1b}[31m"), "");

        let ascii = "a".repeat(100);
        let normalized = normalize_session_title(&ascii);
        assert_eq!(normalized.len(), MAX_TITLE_BYTES);
        assert_eq!(normalized, "a".repeat(MAX_TITLE_BYTES));

        // 30 CJK characters are 90 bytes, so only 26 characters (78 bytes) fit.
        let cjk = "测".repeat(30);
        let normalized = normalize_session_title(&cjk);
        assert_eq!(normalized.chars().count(), 26);
        assert_eq!(normalized.len(), 26 * 3);
        assert!(normalized.len() <= MAX_TITLE_BYTES);
    }

    #[test]
    fn fallback_caps_english_at_five_words() {
        assert_eq!(
            fallback_session_title("one two three four five six seven"),
            "one two three four five"
        );
        assert_eq!(
            fallback_session_title("  one   two \t three\nfour five  "),
            "one two three four five"
        );
    }

    #[test]
    fn fallback_caps_ascii_at_40_bytes() {
        let input = "aaaaaaaaa bbbbbbbbb ccccccccc ddddddddd eeeeeeeee";
        let title = fallback_session_title(input);
        assert!(title.len() <= FALLBACK_MAX_BYTES);
        assert_eq!(title, "aaaaaaaaa bbbbbbbbb ccccccccc ddddddddd");
    }

    #[test]
    fn fallback_caps_chinese_at_40_bytes() {
        // 23 CJK characters = 69 bytes; 40 bytes fit exactly 13 characters.
        let input = "这是一个很长的中文标题用来测试字节截断是否安全";
        let title = fallback_session_title(input);
        assert_eq!(title.len(), 39);
        assert_eq!(title.chars().count(), 13);
        assert_eq!(title, "这是一个很长的中文标题用来");
    }

    #[test]
    fn fallback_ignores_control_sequences_when_splitting() {
        assert_eq!(
            fallback_session_title("  \u{1b}[31mhello\u{1b}[0m   world  "),
            "hello world"
        );
        assert_eq!(fallback_session_title(""), "");
        assert_eq!(fallback_session_title("   \t\n "), "");
        assert_eq!(fallback_session_title("\u{1b}[2K\u{1b}[0m"), "");
    }

    #[test]
    fn model_output_unwraps_emphasis() {
        assert_eq!(title_from_model_output("**标题**"), Some("标题".to_string()));
        assert_eq!(title_from_model_output("* Title *"), Some("Title".to_string()));
        assert_eq!(title_from_model_output("***Deep***"), Some("Deep".to_string()));
        assert_eq!(title_from_model_output("**Title**"), Some("Title".to_string()));
    }

    #[test]
    fn model_output_takes_first_non_empty_line() {
        assert_eq!(
            title_from_model_output("# 标题\n正文内容"),
            Some("# 标题".to_string())
        );
        assert_eq!(
            title_from_model_output("\r\n  \r\n  Real Title  \r\nsecond line"),
            Some("Real Title".to_string())
        );
        assert_eq!(
            title_from_model_output("\n\n\nOnly line\n"),
            Some("Only line".to_string())
        );
        assert_eq!(
            title_from_model_output("  \u{1b}[31m Cleaned Title \u{1b}[0m  \nignored"),
            Some("Cleaned Title".to_string())
        );
    }

    #[test]
    fn model_output_keeps_inner_asterisks() {
        assert_eq!(title_from_model_output("**a*b**"), Some("**a*b**".to_string()));
        assert_eq!(title_from_model_output("*a*b"), Some("*a*b".to_string()));
        assert_eq!(title_from_model_output("**"), None);
        assert_eq!(title_from_model_output("***"), None);
    }

    #[test]
    fn model_output_normalizes_and_returns_none_for_nothing() {
        assert_eq!(title_from_model_output(""), None);
        assert_eq!(title_from_model_output("   \n\t\n \n"), None);
        assert_eq!(title_from_model_output("\u{1b}[31m\u{1b}[0m"), None);

        let long = "b".repeat(120);
        let parsed = title_from_model_output(&long).expect("title");
        assert_eq!(parsed.len(), MAX_TITLE_BYTES);
        assert_eq!(parsed, "b".repeat(MAX_TITLE_BYTES));
    }

    #[test]
    fn system_prompt_is_single_line_and_specific() {
        let prompt = title_system_prompt();
        assert!(!prompt.is_empty());
        assert!(!prompt.contains('\n'));
        assert!(prompt.contains("Return only the title"));
        assert!(prompt.contains("Greeting"));
    }
}
