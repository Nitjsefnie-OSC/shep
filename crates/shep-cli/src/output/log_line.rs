//! The text policy for a line a sheep wrote, as `bleats` puts it on a
//! screen: keep the line's own colour, strip everything else that could
//! drive a terminal.
//!
//! The third policy beside [`crate::terminal_safe::sanitise`] (strips every
//! escape) and [`super::width::sanitize_cell`] (keeps every well-formed
//! CSI): a log line is the sheep's own styling, so an SGR survives, while a
//! cursor move, a screen clear or an OSC title is not content and cannot be
//! allowed to redraw the row.

use crate::style::Presentation;
use crate::vocabulary::Role;

/// `line` with a well-formed SGR sequence (`ESC [` over parameters of
/// digits, `;` and `:`, final `m`) and a tab kept, and every other
/// escape, control character and C1 introducer dropped.
#[must_use]
pub(crate) fn sanitize_log_line(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            match chars.peek().copied() {
                Some('[') => {
                    chars.next();
                    let mut seq = String::from("\u{1b}[");
                    let mut closed = false;
                    let mut sgr = true;
                    for c in chars.by_ref() {
                        if !is_sgr_parameter(c) && !is_csi_final(c) {
                            sgr = false;
                        }
                        seq.push(c);
                        if is_csi_final(c) {
                            closed = true;
                            sgr = sgr && c == 'm';
                            break;
                        }
                    }
                    // An unterminated CSI drops whole: its tail is not
                    // text, it is a sequence some later byte completes.
                    if closed && sgr {
                        out.push_str(&seq);
                    }
                }
                Some(']') => {
                    // OSC runs to a BEL or an ST (`ESC \`); the title text
                    // inside it is what a window tab would show, not the
                    // sheep's line, so the whole span goes.
                    chars.next();
                    for c in chars.by_ref() {
                        if c == '\u{7}' {
                            break;
                        }
                        if c == '\u{1b}' {
                            if chars.peek().copied() == Some('\\') {
                                chars.next();
                            }
                            break;
                        }
                    }
                }
                _ => {
                    // Bare ESC: the introducer and the character after it
                    // go, the same reading `width::sanitize` gives.
                    chars.next();
                }
            }
        } else if c == '\t' || !c.is_control() {
            out.push(c);
        }
    }
    out
}

/// Whether `c` ends a CSI sequence: the final byte is the one in
/// `\u{40}..=\u{7e}`.
fn is_csi_final(c: char) -> bool {
    ('\u{40}'..='\u{7e}').contains(&c)
}

/// Whether `c` is legal inside an SGR parameter list: digits, the
/// separator `;`, and the `:` sub-parameter separator `SGR 38;2;r;g;b`
/// extended forms use.
fn is_sgr_parameter(c: char) -> bool {
    matches!(c, '0'..='9' | ';' | ':')
}

/// Whether `line` holds an SGR that is not a bare reset (`ESC[m`,
/// `ESC[0m`): the answer to "this line already chose its own colour".
#[must_use]
pub(crate) fn sets_colour(line: &str) -> bool {
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' || chars.peek().copied() != Some('[') {
            continue;
        }
        chars.next();
        let mut params = String::new();
        while let Some(c) = chars.peek().copied() {
            if !is_sgr_parameter(c) {
                break;
            }
            params.push(c);
            chars.next();
        }
        // `ESC[m` and `ESC[0m` turn colour off and add none: a line whose
        // only SGR is one of those is still a plain line.
        if chars.next() == Some('m') && !params.is_empty() && params != "0" {
            return true;
        }
    }
    false
}

/// `line` wrapped in the default colour of `stream` (`"out"` meadow, the
/// stdout green; `"err"` bark, the stderr red), unless `style.colour` is
/// off or the line already carries an SGR of its own.
///
/// Call it on a [`sanitize_log_line`] output: `sets_colour` then reads
/// only the escapes that survived as styling.
#[must_use]
pub(crate) fn paint_default(line: &str, stream: &str, style: Presentation) -> String {
    if !style.colour || sets_colour(line) {
        return line.to_owned();
    }
    let role = if stream == "err" {
        Role::Bark
    } else {
        Role::Meadow
    };
    let painted = super::paint::style_for(role, style.deep_colour);
    format!("{painted}{line}{painted:#}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_sgr_colour_survives_and_a_cursor_move_does_not() {
        let line = "\u{1b}[31mfailed\u{1b}[0m \u{1b}[2Coverwritten";
        assert_eq!(
            sanitize_log_line(line),
            "\u{1b}[31mfailed\u{1b}[0m overwritten",
            "the SGR is the sheep's own styling; `[2C` is a cursor move"
        );
    }

    #[test]
    fn a_clear_screen_an_osc_title_and_a_carriage_return_are_stripped() {
        assert_eq!(
            sanitize_log_line("\u{1b}[2Jbefore\rafter"),
            "beforeafter",
            "a clear screen and a bare CR must not redraw the row"
        );
        assert_eq!(
            sanitize_log_line("\u{1b}]0;pwned\u{7}title gone"),
            "title gone",
            "an OSC runs to its BEL terminator and the whole of it goes"
        );
        assert_eq!(
            sanitize_log_line("a\u{8}b\u{1b}[8c!"),
            "ab!",
            "backspace and the CSI after it drop; the tab is the only control kept"
        );
        assert_eq!(
            sanitize_log_line("clean\u{9b}2Jhere"),
            "clean2Jhere",
            "the single-character CSI introducer is a control character and drops"
        );
        assert_eq!(
            sanitize_log_line("keep\tthe\ttab"),
            "keep\tthe\ttab",
            "a tab is column layout the sheep chose, not a redraw"
        );
    }

    #[test]
    fn a_line_that_opens_a_colour_and_never_resets_counts_as_coloured() {
        assert!(
            sets_colour("\u{1b}[32mno reset here"),
            "an unclosed colour is still the sheep's own colour"
        );
    }

    #[test]
    fn a_line_that_colours_one_word_counts_as_coloured() {
        assert!(
            sets_colour("plain \u{1b}[1mbold\u{1b}[0m plain"),
            "one styled word makes the whole line styled"
        );
    }

    #[test]
    fn a_line_that_only_resets_is_still_plain() {
        assert!(
            !sets_colour("\u{1b}[0mback to normal\u{1b}[m"),
            "a reset with nothing to reset is not a colour; both bare spellings count"
        );
        assert!(!sets_colour("nothing styled at all"));
    }
}
