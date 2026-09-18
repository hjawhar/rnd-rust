//! mIRC-compatible text formatting codes.
//!
//! | Byte      | Meaning                                                  |
//! |-----------|----------------------------------------------------------|
//! | `\x02`    | bold toggle                                              |
//! | `\x03`    | color: `FG[,BG]` follow as up to 2 decimal digits each   |
//! | `\x04`    | hex color: `RRGGBB[,RRGGBB]` hex digits follow           |
//! | `\x0F`    | reset — clear all formatting                             |
//! | `\x11`    | monospace toggle (IRCv3 formatting spec)                 |
//! | `\x16`    | reverse video toggle                                     |
//! | `\x1D`    | italic toggle                                            |
//! | `\x1E`    | strikethrough toggle                                     |
//! | `\x1F`    | underline toggle                                         |
//!
//! [`parse_styled`] walks the input and emits a sequence of [`StyledSpan`]s,
//! each annotated with the [`Style`] active for its bytes. Formatting
//! bytes themselves are consumed and never appear in a span's text.

use bytes::Bytes;

/// Control bytes — public so downstream code can strip formatting when
/// it wants to (e.g. for log files or notifications).
pub mod control {
    /// Bold toggle.
    pub const BOLD: u8 = 0x02;
    /// Color with palette digits to follow.
    pub const COLOR: u8 = 0x03;
    /// Hex color.
    pub const HEX_COLOR: u8 = 0x04;
    /// Reset.
    pub const RESET: u8 = 0x0F;
    /// Monospace.
    pub const MONOSPACE: u8 = 0x11;
    /// Reverse video.
    pub const REVERSE: u8 = 0x16;
    /// Italic.
    pub const ITALIC: u8 = 0x1D;
    /// Strikethrough.
    pub const STRIKETHROUGH: u8 = 0x1E;
    /// Underline.
    pub const UNDERLINE: u8 = 0x1F;
}

/// Foreground / background color reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Color {
    /// mIRC palette index (0-98).
    Palette(u8),
    /// 24-bit RGB value.
    Rgb(u8, u8, u8),
}

/// Active text style.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)] // IRC formatting is a flag bag
pub struct Style {
    /// Foreground color.
    pub fg: Option<Color>,
    /// Background color.
    pub bg: Option<Color>,
    /// Bold.
    pub bold: bool,
    /// Italic.
    pub italic: bool,
    /// Underline.
    pub underline: bool,
    /// Strikethrough.
    pub strikethrough: bool,
    /// Monospace.
    pub monospace: bool,
    /// Reverse video.
    pub reverse: bool,
}

/// A slice of text carrying uniform style.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StyledSpan {
    /// Bytes of this run (never empty).
    pub text: Bytes,
    /// Style active for the whole run.
    pub style: Style,
}

/// Walk `input` and return a vector of [`StyledSpan`]s with the
/// formatting-control bytes removed.
#[must_use]
pub fn parse_styled(input: &Bytes) -> Vec<StyledSpan> {
    let bytes: &[u8] = input.as_ref();
    let mut out = Vec::new();
    let mut current = Style::default();
    let mut run_start: Option<usize> = None;
    let mut i = 0usize;

    while i < bytes.len() {
        let b = bytes[i];
        match b {
            control::BOLD => {
                flush_run(&mut out, input, run_start.take(), i, current);
                current.bold = !current.bold;
                i += 1;
            }
            control::ITALIC => {
                flush_run(&mut out, input, run_start.take(), i, current);
                current.italic = !current.italic;
                i += 1;
            }
            control::UNDERLINE => {
                flush_run(&mut out, input, run_start.take(), i, current);
                current.underline = !current.underline;
                i += 1;
            }
            control::STRIKETHROUGH => {
                flush_run(&mut out, input, run_start.take(), i, current);
                current.strikethrough = !current.strikethrough;
                i += 1;
            }
            control::MONOSPACE => {
                flush_run(&mut out, input, run_start.take(), i, current);
                current.monospace = !current.monospace;
                i += 1;
            }
            control::REVERSE => {
                flush_run(&mut out, input, run_start.take(), i, current);
                current.reverse = !current.reverse;
                i += 1;
            }
            control::RESET => {
                flush_run(&mut out, input, run_start.take(), i, current);
                current = Style::default();
                i += 1;
            }
            control::COLOR => {
                flush_run(&mut out, input, run_start.take(), i, current);
                i += 1;
                let (fg, consumed) = read_palette_color(&bytes[i..]);
                i += consumed;
                if consumed == 0 {
                    // Bare ^C resets colors.
                    current.fg = None;
                    current.bg = None;
                } else {
                    current.fg = fg;
                    if bytes.get(i) == Some(&b',') {
                        // Possible background.
                        let (bg, bg_consumed) = read_palette_color(&bytes[i + 1..]);
                        if bg_consumed > 0 {
                            current.bg = bg;
                            i += 1 + bg_consumed;
                        }
                    }
                }
            }
            control::HEX_COLOR => {
                flush_run(&mut out, input, run_start.take(), i, current);
                i += 1;
                let (fg, consumed) = read_hex_color(&bytes[i..]);
                i += consumed;
                if consumed == 0 {
                    current.fg = None;
                    current.bg = None;
                } else {
                    current.fg = fg;
                    if bytes.get(i) == Some(&b',') {
                        let (bg, bg_consumed) = read_hex_color(&bytes[i + 1..]);
                        if bg_consumed > 0 {
                            current.bg = bg;
                            i += 1 + bg_consumed;
                        }
                    }
                }
            }
            _ => {
                if run_start.is_none() {
                    run_start = Some(i);
                }
                i += 1;
            }
        }
    }
    flush_run(&mut out, input, run_start, bytes.len(), current);
    out
}

fn flush_run(
    out: &mut Vec<StyledSpan>,
    input: &Bytes,
    start: Option<usize>,
    end: usize,
    style: Style,
) {
    if let Some(s) = start {
        if end > s {
            out.push(StyledSpan {
                text: input.slice(s..end),
                style,
            });
        }
    }
}

fn read_palette_color(bytes: &[u8]) -> (Option<Color>, usize) {
    // Up to 2 decimal digits.
    let mut value = 0u16;
    let mut consumed = 0usize;
    while consumed < 2 {
        match bytes.get(consumed) {
            Some(b) if b.is_ascii_digit() => {
                value = value * 10 + u16::from(*b - b'0');
                consumed += 1;
            }
            _ => break,
        }
    }
    if consumed == 0 {
        return (None, 0);
    }
    #[allow(clippy::cast_possible_truncation)]
    let palette = Color::Palette(value.min(98) as u8);
    (Some(palette), consumed)
}

fn read_hex_color(bytes: &[u8]) -> (Option<Color>, usize) {
    // Exactly 6 hex digits.
    if bytes.len() < 6 {
        return (None, 0);
    }
    let hex = &bytes[..6];
    if !hex.iter().all(u8::is_ascii_hexdigit) {
        return (None, 0);
    }
    let r = hex_pair(hex[0], hex[1]);
    let g = hex_pair(hex[2], hex[3]);
    let b = hex_pair(hex[4], hex[5]);
    (Some(Color::Rgb(r, g, b)), 6)
}

fn hex_pair(hi: u8, lo: u8) -> u8 {
    hex_digit(hi) * 16 + hex_digit(lo)
}

const fn hex_digit(b: u8) -> u8 {
    match b {
        b'0'..=b'9' => b - b'0',
        b'a'..=b'f' => b - b'a' + 10,
        b'A'..=b'F' => b - b'A' + 10,
        _ => 0,
    }
}

/// Strip every formatting byte from `input`, preserving the underlying
/// text. Useful for logs and notifications that don't render colors.
#[must_use]
pub fn strip_formatting(input: &Bytes) -> Bytes {
    parse_styled(input)
        .into_iter()
        .flat_map(|span| span.text.as_ref().to_vec())
        .collect::<Vec<u8>>()
        .into()
}

#[cfg(test)]
mod tests {
    use super::{Color, Style, StyledSpan, parse_styled, strip_formatting};
    use bytes::Bytes;

    fn parse(s: &[u8]) -> Vec<StyledSpan> {
        parse_styled(&Bytes::copy_from_slice(s))
    }

    #[test]
    fn plain_text_is_one_span() {
        let spans = parse(b"hello");
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].text.as_ref(), b"hello");
        assert_eq!(spans[0].style, Style::default());
    }

    #[test]
    fn bold_toggle_produces_styled_runs() {
        let spans = parse(b"a\x02b\x02c");
        assert_eq!(spans.len(), 3);
        assert!(!spans[0].style.bold);
        assert!(spans[1].style.bold);
        assert!(!spans[2].style.bold);
        assert_eq!(spans[0].text.as_ref(), b"a");
        assert_eq!(spans[1].text.as_ref(), b"b");
        assert_eq!(spans[2].text.as_ref(), b"c");
    }

    #[test]
    fn palette_color_with_fg_and_bg() {
        let spans = parse(b"\x034,10red on blue");
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].style.fg, Some(Color::Palette(4)));
        assert_eq!(spans[0].style.bg, Some(Color::Palette(10)));
    }

    #[test]
    fn palette_color_single_digit() {
        let spans = parse(b"\x033green");
        assert_eq!(spans[0].style.fg, Some(Color::Palette(3)));
    }

    #[test]
    fn bare_color_byte_resets_colors() {
        let spans = parse(b"\x034red\x03reset");
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].style.fg, Some(Color::Palette(4)));
        assert!(spans[1].style.fg.is_none());
    }

    #[test]
    fn hex_color_is_24bit() {
        let spans = parse(b"\x04FF8800hex");
        assert_eq!(spans[0].style.fg, Some(Color::Rgb(0xFF, 0x88, 0x00)));
    }

    #[test]
    fn reset_clears_all_styles() {
        let spans = parse(b"\x02\x1Dmixed\x0Fplain");
        assert!(spans[0].style.bold);
        assert!(spans[0].style.italic);
        assert_eq!(spans[1].style, Style::default());
    }

    #[test]
    fn strip_removes_control_bytes() {
        let raw = Bytes::from_static(b"\x02hi\x02 \x034there\x0F!");
        assert_eq!(strip_formatting(&raw).as_ref(), b"hi there!");
    }

    #[test]
    fn unicode_text_runs_preserved() {
        // Non-ASCII bytes flow through as-is; we don't interpret UTF-8.
        let spans = parse("\x02café\x02".as_bytes());
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].text.as_ref(), "café".as_bytes());
        assert!(spans[0].style.bold);
    }
}
