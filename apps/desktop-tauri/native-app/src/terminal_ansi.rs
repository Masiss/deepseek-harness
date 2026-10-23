//! Terminal output decoding for the native shell: SGR styling plus line assembly.
//!
//! The Host sends raw PTY bytes, including cursor-positioning sequences. This module
//! renders the parts a plain scrollback view can represent — text, line breaks,
//! carriage-return overwrites, and SGR attributes — and drops sequences that only
//! move the cursor or erase the screen.

use eframe::egui::{self, Color32, FontId};

/// One resolved SGR attribute set.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AnsiStyle {
    pub fg: Option<Color32>,
    pub bg: Option<Color32>,
    pub bold: bool,
    pub dim: bool,
    pub italic: bool,
    pub underline: bool,
    pub inverse: bool,
}

/// A styled run of characters on one terminal line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AnsiSpan {
    pub text: String,
    pub style: AnsiStyle,
}

/// One rendered line; an empty line still occupies a row.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AnsiLine {
    pub spans: Vec<AnsiSpan>,
}

const TAB_WIDTH: usize = 8;
const DEFAULT_FOREGROUND: Color32 = Color32::from_rgb(0xd8, 0xde, 0xe9);
const DEFAULT_BACKGROUND: Color32 = Color32::from_rgb(0x1a, 0x1b, 0x26);

/// Palette entry for the 16 basic SGR colors and their bright variants.
fn basic_color(index: u8) -> Color32 {
    const BASIC: [Color32; 16] = [
        Color32::from_rgb(0x1a, 0x1b, 0x26),
        Color32::from_rgb(0xf7, 0x76, 0x8e),
        Color32::from_rgb(0x9e, 0xce, 0x6a),
        Color32::from_rgb(0xe0, 0xaf, 0x68),
        Color32::from_rgb(0x7a, 0xa2, 0xf7),
        Color32::from_rgb(0xbb, 0x9a, 0xf7),
        Color32::from_rgb(0x7d, 0xcf, 0xff),
        Color32::from_rgb(0xa9, 0xb1, 0xd6),
        Color32::from_rgb(0x41, 0x48, 0x68),
        Color32::from_rgb(0xff, 0x89, 0x9f),
        Color32::from_rgb(0xb9, 0xf2, 0x7c),
        Color32::from_rgb(0xff, 0xc7, 0x77),
        Color32::from_rgb(0x8c, 0xb4, 0xff),
        Color32::from_rgb(0xc9, 0x9b, 0xff),
        Color32::from_rgb(0x9d, 0xe6, 0xff),
        Color32::from_rgb(0xd8, 0xde, 0xe9),
    ];
    BASIC[usize::from(index) % BASIC.len()]
}

fn color_256(index: u8) -> Color32 {
    match index {
        0..=15 => basic_color(index),
        16..=231 => {
            let index = u16::from(index) - 16;
            let steps = [0_u8, 95, 135, 175, 215, 255];
            let r = steps[usize::from(index / 36) % 6];
            let g = steps[usize::from((index % 36) / 6) % 6];
            let b = steps[usize::from(index % 6) % 6];
            Color32::from_rgb(r, g, b)
        }
        232..=255 => {
            let level = 8 + (u16::from(index) - 232) * 10;
            let value = u8::try_from(level).unwrap_or(u8::MAX);
            Color32::from_rgb(value, value, value)
        }
    }
}

#[derive(Default)]
struct LineBuffer {
    cells: Vec<(char, AnsiStyle)>,
    cursor: usize,
}

impl LineBuffer {
    fn write(&mut self, character: char, style: AnsiStyle) {
        if character == '\t' {
            let next = (self.cursor / TAB_WIDTH + 1) * TAB_WIDTH;
            while self.cursor < next {
                self.put(' ', style);
            }
            return;
        }
        self.put(character, style);
    }

    fn put(&mut self, character: char, style: AnsiStyle) {
        if self.cursor < self.cells.len() {
            self.cells[self.cursor] = (character, style);
        } else {
            self.cells.push((character, style));
        }
        self.cursor += 1;
    }

    fn take(&mut self) -> AnsiLine {
        let mut line = AnsiLine::default();
        for (character, style) in self.cells.drain(..) {
            match line.spans.last_mut() {
                Some(span) if span.style == style => span.text.push(character),
                _ => line.spans.push(AnsiSpan {
                    text: character.to_string(),
                    style,
                }),
            }
        }
        self.cursor = 0;
        line
    }
}

/// Parses terminal output into rendered lines, applying SGR state and carriage returns.
pub fn parse(output: &str) -> Vec<AnsiLine> {
    let mut lines = Vec::new();
    let mut line = LineBuffer::default();
    let mut style = AnsiStyle::default();
    let mut characters = output.chars().peekable();
    while let Some(character) = characters.next() {
        match character {
            '\u{1b}' => {
                if characters.peek() == Some(&'[') {
                    characters.next();
                    parse_csi(&mut characters, &mut style, &mut line);
                } else if characters.peek() == Some(&']') {
                    characters.next();
                    for inner in characters.by_ref() {
                        if inner == '\u{7}' {
                            break;
                        }
                    }
                } else {
                    // A two-byte escape sequence has nothing to render; charset
                    // designators carry a third byte.
                    if let Some(next) = characters.next() {
                        if matches!(next, '(' | ')' | '#' | '%') {
                            characters.next();
                        }
                    }
                }
            }
            '\n' => lines.push(line.take()),
            '\r' => line.cursor = 0,
            character if character.is_control() && character != '\t' => {}
            character => line.write(character, style),
        }
    }
    if !line.cells.is_empty() || lines.is_empty() {
        lines.push(line.take());
    }
    lines
}

fn parse_csi(
    characters: &mut std::iter::Peekable<std::str::Chars<'_>>,
    style: &mut AnsiStyle,
    line: &mut LineBuffer,
) {
    let mut parameters = String::new();
    for character in characters.by_ref() {
        if ('\u{40}'..='\u{7e}').contains(&character) {
            if character == 'm' {
                apply_sgr(style, &parameters);
            } else if character == 'K' {
                // Erase to the end of the line.
                line.cells.truncate(line.cursor);
            }
            return;
        }
        parameters.push(character);
    }
}

fn apply_sgr(style: &mut AnsiStyle, parameters: &str) {
    let values: Vec<u16> = parameters
        .split(';')
        .map(|value| value.parse::<u16>().unwrap_or(0))
        .collect();
    let values = if values.is_empty() { vec![0] } else { values };
    let mut index = 0;
    while index < values.len() {
        match values[index] {
            0 => *style = AnsiStyle::default(),
            1 => style.bold = true,
            2 => style.dim = true,
            3 => style.italic = true,
            4 => style.underline = true,
            7 => style.inverse = true,
            22 => {
                style.bold = false;
                style.dim = false;
            }
            23 => style.italic = false,
            24 => style.underline = false,
            27 => style.inverse = false,
            30..=37 => style.fg = Some(basic_color(values[index] as u8 - 30)),
            39 => style.fg = None,
            40..=47 => style.bg = Some(basic_color(values[index] as u8 - 40)),
            49 => style.bg = None,
            90..=97 => style.fg = Some(basic_color(values[index] as u8 - 90 + 8)),
            100..=107 => style.bg = Some(basic_color(values[index] as u8 - 100 + 8)),
            38 | 48 => {
                let foreground = values[index] == 38;
                match values.get(index + 1).copied() {
                    Some(5) => {
                        if let Some(color) = values.get(index + 2) {
                            let color = color_256(u8::try_from(*color).unwrap_or(0));
                            if foreground {
                                style.fg = Some(color);
                            } else {
                                style.bg = Some(color);
                            }
                        }
                        index += 2;
                    }
                    Some(2) => {
                        let (Some(r), Some(g), Some(b)) = (
                            values.get(index + 2),
                            values.get(index + 3),
                            values.get(index + 4),
                        ) else {
                            return;
                        };
                        let color = Color32::from_rgb(
                            u8::try_from(*r).unwrap_or(0),
                            u8::try_from(*g).unwrap_or(0),
                            u8::try_from(*b).unwrap_or(0),
                        );
                        if foreground {
                            style.fg = Some(color);
                        } else {
                            style.bg = Some(color);
                        }
                        index += 4;
                    }
                    _ => return,
                }
            }
            _ => {}
        }
        index += 1;
    }
}

/// Resolves SGR attributes into concrete paint colors.
fn format_for(style: AnsiStyle) -> egui::TextFormat {
    let (mut foreground, mut background) = (
        style.fg.unwrap_or(DEFAULT_FOREGROUND),
        style.bg.unwrap_or(DEFAULT_BACKGROUND),
    );
    if style.inverse {
        std::mem::swap(&mut foreground, &mut background);
    }
    if style.dim {
        foreground = foreground.gamma_multiply(0.6);
    }
    egui::TextFormat {
        font_id: FontId::monospace(12.0),
        color: foreground,
        background,
        italics: style.italic,
        underline: if style.underline {
            egui::Stroke::new(1.0, foreground)
        } else {
            egui::Stroke::NONE
        },
        ..Default::default()
    }
}

/// Builds a monospace layout for the parsed scrollback.
pub fn layout(lines: &[AnsiLine], wrap_width: f32) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    job.wrap.max_width = wrap_width;
    job.wrap.break_anywhere = true;
    for (line_index, line) in lines.iter().enumerate() {
        if line.spans.is_empty() {
            job.append(" ", 0.0, format_for(AnsiStyle::default()));
        } else {
            for span in &line.spans {
                job.append(&span.text, 0.0, format_for(span.style));
            }
        }
        if line_index + 1 < lines.len() {
            job.append("\n", 0.0, format_for(AnsiStyle::default()));
        }
    }
    job
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(lines: &[AnsiLine]) -> String {
        lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.text.as_str())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn plain_text_keeps_lines_and_ignores_cursor_sequences() {
        let lines = parse("one\r\n\u{1b}[2Ktwo\u{1b}[1;1Hthree\n");
        assert_eq!(text(&lines), "one\ntwothree");
    }

    #[test]
    fn carriage_return_overwrites_the_current_line() {
        let lines = parse("progress 10%\rprogress 90%\n");
        assert_eq!(text(&lines), "progress 90%");
    }

    #[test]
    fn sgr_colors_split_spans_and_reset() {
        let lines = parse("\u{1b}[31mred\u{1b}[0m plain \u{1b}[1;38;2;10;20;30mbold\n");
        let spans = &lines[0].spans;
        assert_eq!(spans.len(), 3);
        assert_eq!(spans[0].text, "red");
        assert_eq!(spans[0].style.fg, Some(basic_color(1)));
        assert_eq!(spans[1].text, " plain ");
        assert_eq!(spans[1].style, AnsiStyle::default());
        assert_eq!(spans[2].text, "bold");
        assert!(spans[2].style.bold);
        assert_eq!(spans[2].style.fg, Some(Color32::from_rgb(10, 20, 30)));
    }

    #[test]
    fn eight_bit_colors_and_inverse_attributes_resolve() {
        let lines = parse("\u{1b}[38;5;196mX\u{1b}[7;44mY");
        assert_eq!(lines[0].spans[0].style.fg, Some(color_256(196)));
        let inverse = lines[0].spans[1].style;
        assert!(inverse.inverse);
        let format = format_for(inverse);
        assert_eq!(format.color, basic_color(4));
        assert_eq!(format.background, color_256(196));
    }

    #[test]
    fn operating_system_titles_and_unknown_sequences_are_dropped() {
        let lines = parse("\u{1b}]0;title\u{7}visible\u{1b}[?25l!\u{1b}(B\n");
        assert_eq!(text(&lines), "visible!");
    }

    #[test]
    fn tabs_expand_to_the_next_eight_column_stop() {
        let lines = parse("a\tb\n\tc");
        assert_eq!(text(&lines), "a       b\n        c");
    }

    #[test]
    fn malformed_or_truncated_sequences_do_not_panic() {
        for input in ["\u{1b}[", "\u{1b}[38;2;1;2m", "\u{1b}]", "\u{1b}"] {
            let lines = parse(input);
            assert!(lines.len() <= 1);
        }
    }

    #[test]
    fn erase_to_end_of_line_truncates_the_visible_suffix() {
        let lines = parse("abcdef\r\u{1b}[3DXY\u{1b}[K\n");
        assert_eq!(text(&lines), "XY");
    }

    #[test]
    fn layout_renders_each_line_with_a_trailing_newline_between_rows() {
        let job = layout(&parse("a\u{1b}[31mb\nc"), 200.0);
        assert_eq!(job.text, "ab\nc");
    }
}
