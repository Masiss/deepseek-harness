//! Terminal output decoding for the native shell: SGR styling, cursor addressing,
//! and screen assembly.
//!
//! The Host sends raw PTY bytes. This module keeps a small screen model so the
//! sequences a full-screen program emits — absolute and relative cursor moves,
//! erases, saved cursors — land where the program intended instead of appending
//! text. Attributes resolve into egui paint formats, and the result is rendered as
//! one scrollable monospace layout.

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
/// Rows retained before the oldest scrolls out of the view.
const MAX_ROWS: usize = 5_000;
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

/// One cell: the text drawn at that column and its attributes.
///
/// A cell holds a string so a combining mark can attach to the base character
/// it modifies; a wide character's second column holds an empty string.
type Cell = (String, AnsiStyle);
type Row = Vec<Cell>;

/// The main screen kept aside while an alternate screen is active.
struct SavedScreen {
    rows: Vec<Row>,
    row: usize,
    col: usize,
}

/// Rows of cells plus the cursor a full-screen program moves around.
struct Screen {
    rows: Vec<Row>,
    row: usize,
    col: usize,
    saved: Option<(usize, usize)>,
    /// Terminal width in columns; `0` grows rows without wrapping.
    width: usize,
    /// First row of the scrolling region.
    scroll_top: usize,
    /// Last row of the scrolling region; `None` means the last row on screen.
    scroll_bottom: Option<usize>,
    /// Main screen retained while an alternate screen is active.
    main: Option<Box<SavedScreen>>,
    /// Answers the program asked for through device/cursor queries.
    replies: String,
}

impl Screen {
    fn new(width: usize) -> Self {
        Self {
            rows: vec![Vec::new()],
            row: 0,
            col: 0,
            saved: None,
            width,
            scroll_top: 0,
            scroll_bottom: None,
            main: None,
            replies: String::new(),
        }
    }

    fn ensure_row(&mut self) {
        while self.rows.len() <= self.row {
            self.rows.push(Vec::new());
        }
    }

    /// The inclusive first and last row of the scrolling region.
    fn region(&self) -> (usize, usize) {
        let last = self.rows.len().saturating_sub(1);
        let top = self.scroll_top.min(last);
        let bottom = self.scroll_bottom.unwrap_or(last).min(last).max(top);
        (top, bottom)
    }

    fn put(&mut self, character: char, style: AnsiStyle) {
        let width = unicode_width::UnicodeWidthChar::width(character).unwrap_or(1);
        if width == 0 {
            // A combining mark belongs to the cell behind the cursor.
            if self.col > 0 {
                self.ensure_row();
                if let Some(cell) = self.rows[self.row].get_mut(self.col - 1) {
                    cell.0.push(character);
                }
            }
            return;
        }
        if self.width > 0 && self.col + width > self.width {
            self.line_feed();
            self.col = 0;
        }
        self.ensure_row();
        let line = &mut self.rows[self.row];
        while line.len() < self.col {
            line.push((" ".to_owned(), AnsiStyle::default()));
        }
        let text = character.to_string();
        if self.col < line.len() {
            line[self.col] = (text, style);
        } else {
            line.push((text, style));
        }
        if width == 2 && (self.width == 0 || self.col + 1 < self.width) {
            let filler = self.col + 1;
            while line.len() <= filler {
                line.push((" ".to_owned(), AnsiStyle::default()));
            }
            line[filler] = (String::new(), style);
        }
        self.col += width;
        self.scroll();
    }

    fn write_tab(&mut self, style: AnsiStyle) {
        let next = (self.col / TAB_WIDTH + 1) * TAB_WIDTH;
        while self.col < next {
            if self.width > 0 && self.col >= self.width {
                break;
            }
            self.put(' ', style);
        }
    }

    /// Drops the oldest rows once the screen holds more than it retains.
    fn scroll(&mut self) {
        if self.rows.len() <= MAX_ROWS {
            return;
        }
        let excess = self.rows.len() - MAX_ROWS;
        self.rows.drain(..excess);
        self.row = self.row.saturating_sub(excess);
        self.scroll_top = self.scroll_top.saturating_sub(excess);
        if let Some(bottom) = &mut self.scroll_bottom {
            *bottom = bottom.saturating_sub(excess);
        }
    }

    /// Moves down one row; inside an explicit region the bottom edge scrolls.
    fn line_feed(&mut self) {
        if let Some(bottom) = self.scroll_bottom {
            if self.row >= bottom {
                self.scroll_up(1);
                return;
            }
        }
        self.row += 1;
        self.ensure_row();
        self.scroll();
    }

    /// Carriage return plus line feed, the shape a shell line ends with.
    fn newline(&mut self) {
        self.col = 0;
        self.line_feed();
    }

    fn reverse_index(&mut self) {
        if self.scroll_bottom.is_some() {
            let (top, _) = self.region();
            if self.row <= top {
                self.scroll_down(1);
                return;
            }
        }
        self.row = self.row.saturating_sub(1);
    }

    fn scroll_up(&mut self, count: usize) {
        let (top, bottom) = self.region();
        for _ in 0..count {
            if top >= self.rows.len() {
                break;
            }
            self.rows.remove(top);
            self.rows
                .insert(bottom.min(self.rows.len()), Vec::new());
        }
        if self.row > bottom {
            self.row = bottom;
        }
    }

    fn scroll_down(&mut self, count: usize) {
        let (top, bottom) = self.region();
        for _ in 0..count {
            if bottom >= self.rows.len() {
                self.ensure_row();
            }
            if bottom >= self.rows.len() {
                break;
            }
            self.rows.remove(bottom);
            self.rows.insert(top.min(self.rows.len()), Vec::new());
        }
    }

    fn move_to(&mut self, row: usize, col: usize) {
        self.row = if self.width > 0 {
            row.min(MAX_ROWS.saturating_sub(1))
        } else {
            row
        };
        self.col = if self.width > 0 {
            col.min(self.width.saturating_sub(1))
        } else {
            col
        };
        self.ensure_row();
    }

    fn move_by(&mut self, rows: isize, cols: isize) {
        let row = self.row.saturating_add_signed(rows);
        let col = self.col.saturating_add_signed(cols);
        self.move_to(row, col);
    }

    fn erase_in_line(&mut self, mode: u16) {
        self.ensure_row();
        let line = &mut self.rows[self.row];
        match mode {
            1 => {
                let end = self.col.min(line.len().saturating_sub(1));
                for cell in line.iter_mut().take(end + 1) {
                    *cell = (" ".to_owned(), AnsiStyle::default());
                }
            }
            2 => line.clear(),
            _ => {
                if self.col < line.len() {
                    line.truncate(self.col);
                }
            }
        }
    }

    fn erase_in_display(&mut self, mode: u16) {
        match mode {
            1 => {
                for index in 0..self.row {
                    self.rows[index].clear();
                }
                self.erase_in_line(1);
            }
            2 | 3 => {
                for line in &mut self.rows {
                    line.clear();
                }
                self.row = 0;
                self.col = 0;
            }
            _ => {
                self.erase_in_line(0);
                for index in self.row + 1..self.rows.len() {
                    self.rows[index].clear();
                }
            }
        }
    }

    fn insert_lines(&mut self, count: usize) {
        let (top, bottom) = self.region();
        if self.row < top || self.row > bottom {
            return;
        }
        for _ in 0..count {
            self.rows.remove(bottom.min(self.rows.len().saturating_sub(1)));
            self.rows
                .insert(self.row.min(self.rows.len()), Vec::new());
        }
    }

    fn delete_lines(&mut self, count: usize) {
        let (top, bottom) = self.region();
        if self.row < top || self.row > bottom {
            return;
        }
        for _ in 0..count {
            if self.row >= self.rows.len() {
                break;
            }
            self.rows.remove(self.row);
            self.rows
                .insert(bottom.min(self.rows.len()), Vec::new());
        }
    }

    fn insert_chars(&mut self, count: usize) {
        self.ensure_row();
        let limit = if self.width > 0 { self.width } else { usize::MAX };
        let line = &mut self.rows[self.row];
        for _ in 0..count {
            if line.len() >= limit {
                break;
            }
            if self.col <= line.len() {
                line.insert(self.col, (" ".to_owned(), AnsiStyle::default()));
            }
        }
    }

    fn delete_chars(&mut self, count: usize) {
        if self.row >= self.rows.len() {
            return;
        }
        let line = &mut self.rows[self.row];
        for _ in 0..count {
            if self.col < line.len() {
                line.remove(self.col);
            }
        }
    }

    fn set_scroll_region(&mut self, top: usize, bottom: usize) {
        let last = self.rows.len().saturating_sub(1);
        let top = top.max(1) - 1;
        let bottom = if bottom == 0 { None } else { Some(bottom - 1) };
        if bottom.is_some_and(|bottom| bottom <= top) || top > last {
            self.scroll_top = 0;
            self.scroll_bottom = None;
        } else {
            self.scroll_top = top;
            self.scroll_bottom = bottom;
        }
        self.move_to(self.scroll_top, 0);
    }

    fn enter_alternate(&mut self) {
        if self.main.is_some() {
            return;
        }
        self.main = Some(Box::new(SavedScreen {
            rows: std::mem::replace(&mut self.rows, vec![Vec::new()]),
            row: self.row,
            col: self.col,
        }));
        self.row = 0;
        self.col = 0;
        self.saved = None;
        self.scroll_top = 0;
        self.scroll_bottom = None;
    }

    fn leave_alternate(&mut self) {
        let Some(saved) = self.main.take() else {
            return;
        };
        self.rows = saved.rows;
        self.row = saved.row;
        self.col = saved.col;
        self.saved = None;
        self.scroll_top = 0;
        self.scroll_bottom = None;
    }

    /// Answers a device or cursor query the way a screen terminal would.
    fn reply(&mut self, text: String) {
        self.replies.push_str(&text);
    }

    /// Emits the written rows, stopping at the last one with visible cells.
    fn into_lines(mut self) -> (Vec<AnsiLine>, String) {
        self.ensure_row();
        let last = self
            .rows
            .iter()
            .rposition(|line| line.iter().any(|cell| !cell.0.is_empty()))
            .unwrap_or(0);
        let lines = self
            .rows
            .into_iter()
            .take(last + 1)
            .map(|mut cells| {
                while cells
                    .last()
                    .is_some_and(|(text, style)| *text == " " && *style == AnsiStyle::default())
                {
                    cells.pop();
                }
                let mut line = AnsiLine::default();
                for (text, style) in cells {
                    if text.is_empty() {
                        continue;
                    }
                    match line.spans.last_mut() {
                        Some(span) if span.style == style => span.text.push_str(&text),
                        _ => line.spans.push(AnsiSpan { text, style }),
                    }
                }
                line
            })
            .collect();
        (lines, self.replies)
    }
}

/// Rendered lines plus the replies a program asked the terminal to send back.
pub struct TerminalScreen {
    /// Rows to paint.
    pub lines: Vec<AnsiLine>,
    /// Bytes the program expects on its input (device and cursor queries).
    pub replies: String,
}

/// Parses terminal output into rendered lines and the replies it asked for.
///
/// @param output - Raw PTY output.
/// @param width - Terminal width in columns, or `0` for an unbounded row.
/// @returns The painted rows and any reply bytes the program is waiting for.
pub fn parse_screen(output: &str, width: usize) -> TerminalScreen {
    let mut screen = Screen::new(width);
    let mut style = AnsiStyle::default();
    let mut characters = output.chars().peekable();
    while let Some(character) = characters.next() {
        match character {
            '\u{1b}' => match characters.peek().copied() {
                Some('[') => {
                    characters.next();
                    parse_csi(&mut characters, &mut style, &mut screen);
                }
                Some(']') => {
                    characters.next();
                    for inner in characters.by_ref() {
                        if inner == '\u{7}' {
                            break;
                        }
                    }
                }
                Some('D') => {
                    characters.next();
                    screen.col = 0;
                    screen.line_feed();
                }
                Some('M') => {
                    characters.next();
                    screen.reverse_index();
                }
                Some('E') => {
                    characters.next();
                    screen.newline();
                }
                Some('7') => {
                    characters.next();
                    screen.saved = Some((screen.row, screen.col));
                }
                Some('8') => {
                    characters.next();
                    if let Some((row, col)) = screen.saved {
                        screen.move_to(row, col);
                    }
                }
                Some('c') => {
                    characters.next();
                    screen = Screen::new(width);
                }
                Some('(' | ')' | '#' | '%') => {
                    characters.next();
                    characters.next();
                }
                _ => {
                    characters.next();
                }
            },
            '\n' => screen.newline(),
            '\r' => screen.col = 0,
            '\t' => screen.write_tab(style),
            character if character.is_control() => {}
            character => screen.put(character, style),
        }
    }
    let (lines, replies) = screen.into_lines();
    TerminalScreen { lines, replies }
}

fn parse_csi(
    characters: &mut std::iter::Peekable<std::str::Chars<'_>>,
    style: &mut AnsiStyle,
    screen: &mut Screen,
) {
    let mut parameters = String::new();
    for character in characters.by_ref() {
        if ('\u{40}'..='\u{7e}').contains(&character) {
            if let Some(private) = parameters.strip_prefix('?') {
                if matches!(character, 'h' | 'l') {
                    apply_private_mode(screen, private, character == 'h');
                }
                return;
            }
            match character {
                'm' => apply_sgr(style, &parameters),
                'K' => screen.erase_in_line(csi_value(&parameters, 0, 0)),
                'J' => screen.erase_in_display(csi_value(&parameters, 0, 0)),
                'H' | 'f' => {
                    let row = csi_value(&parameters, 0, 1).max(1) as usize - 1;
                    let col = csi_value(&parameters, 1, 1).max(1) as usize - 1;
                    screen.move_to(row, col);
                }
                'A' => screen.move_by(-(csi_value(&parameters, 0, 1) as isize), 0),
                'B' => screen.move_by(csi_value(&parameters, 0, 1) as isize, 0),
                'C' => screen.move_by(0, csi_value(&parameters, 0, 1) as isize),
                'D' => screen.move_by(0, -(csi_value(&parameters, 0, 1) as isize)),
                'G' => screen.move_to(screen.row, csi_value(&parameters, 0, 1).max(1) as usize - 1),
                'd' => screen.move_to(csi_value(&parameters, 0, 1).max(1) as usize - 1, screen.col),
                'L' => screen.insert_lines(csi_value(&parameters, 0, 1) as usize),
                'M' => screen.delete_lines(csi_value(&parameters, 0, 1) as usize),
                '@' => screen.insert_chars(csi_value(&parameters, 0, 1) as usize),
                'P' => screen.delete_chars(csi_value(&parameters, 0, 1) as usize),
                'r' => {
                    let top = csi_value(&parameters, 0, 1) as usize;
                    let bottom = csi_value(&parameters, 1, 0) as usize;
                    screen.set_scroll_region(top, bottom);
                }
                'n' if matches!(csi_value(&parameters, 0, 0), 5 | 6) => {
                    let reply = format!("\u{1b}[{};{}R", screen.row + 1, screen.col + 1);
                    screen.reply(reply);
                }
                'c' if csi_value(&parameters, 0, 0) == 0 => {
                    screen.reply("\u{1b}[?1;2c".to_owned());
                }
                's' => screen.saved = Some((screen.row, screen.col)),
                'u' => {
                    if let Some((row, col)) = screen.saved {
                        screen.move_to(row, col);
                    }
                }
                // Mode switches (hidden cursor, autowrap) and every other
                // sequence carry no cell change in this view.
                _ => {}
            }
            return;
        }
        parameters.push(character);
    }
}

/// Applies the private modes this view models: the alternate screen buffer.
fn apply_private_mode(screen: &mut Screen, parameters: &str, enabled: bool) {
    for parameter in parameters.split(';') {
        match parameter.trim() {
            "47" | "1047" | "1049" if enabled => screen.enter_alternate(),
            "47" | "1047" | "1049" => screen.leave_alternate(),
            _ => {}
        }
    }
}

/// One CSI parameter by position, falling back to `default` when it is absent or blank.
fn csi_value(parameters: &str, index: usize, default: u16) -> u16 {
    let private = parameters.starts_with('?');
    let parameters = parameters.trim_start_matches('?');
    if private {
        return default;
    }
    parameters
        .split(';')
        .nth(index)
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(default)
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

    fn parse(output: &str) -> Vec<AnsiLine> {
        parse_screen(output, 0).lines
    }

    fn parse_sized(output: &str, width: usize) -> Vec<AnsiLine> {
        parse_screen(output, width).lines
    }


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
    fn alternate_screen_restores_the_main_screen_on_exit() {
        let lines = parse("main\u{1b}[?1049halt\u{1b}[?1049l tail");
        assert_eq!(text(&lines), "main tail");
    }

    #[test]
    fn scroll_region_confines_scrolling_to_its_rows() {
        let lines = parse("1\n2\n3\n4\n5\u{1b}[2;4r\n\n\n\n");
        assert_eq!(text(&lines), "1\n4\n\n\n5");
    }

    #[test]
    fn insert_and_delete_lines_shift_the_region() {
        let inserted = parse("a\nb\nc\u{1b}[2;1H\u{1b}[1L");
        assert_eq!(text(&inserted), "a\n\nb");
        let deleted = parse("a\nb\nc\u{1b}[2;1H\u{1b}[1M");
        assert_eq!(text(&deleted), "a\nc");
    }

    #[test]
    fn wide_and_combining_characters_keep_column_positions() {
        let lines = parse_sized("\u{6f22}\u{5b57}x", 6);
        assert_eq!(text(&lines), "\u{6f22}\u{5b57}x");
        let combining = parse("e\u{301}x");
        assert_eq!(text(&combining), "e\u{301}x");
    }

    #[test]
    fn cursor_and_device_queries_produce_replies() {
        let screen = parse_screen("ab\u{1b}[6n\u{1b}[c", 80);
        assert_eq!(screen.replies, "\u{1b}[1;3R\u{1b}[?1;2c");
        assert_eq!(text(&screen.lines), "ab");
    }

    #[test]
    fn absolute_cursor_moves_rewrite_existing_rows() {
        let lines = parse("one\r\n\u{1b}[2Ktwo\u{1b}[1;1Hthree\n");
        assert_eq!(text(&lines), "three\ntwo");
    }

    #[test]
    fn absolute_and_relative_moves_address_one_screen() {
        let lines = parse("\u{1b}[2;3Habc\u{1b}[1;1Htop\u{1b}[1Bdown\u{1b}[2D!\u{1b}[3;1Hx");
        assert_eq!(text(&lines), "top\n  ado!n\nx");
    }

    #[test]
    fn save_and_restore_return_to_the_marked_cell() {
        let lines = parse("a\u{1b}[s\u{1b}[2;1Hb\u{1b}[u!");
        assert_eq!(text(&lines), "a!\nb");
    }

    #[test]
    fn erase_display_clears_the_requested_region() {
        let all = parse("one\ntwo\u{1b}[2Jok");
        assert_eq!(text(&all), "ok");
        let below = parse("one\ntwo\u{1b}[1;2H\u{1b}[0J");
        assert_eq!(text(&below), "o");
    }

    #[test]
    fn known_width_wraps_and_clamps_the_cursor() {
        let lines = parse_sized("abcdef", 4);
        assert_eq!(text(&lines), "abcd\nef");
        let clamped = parse_sized("\u{1b}[1;99Hx", 4);
        assert_eq!(text(&clamped), "   x");
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
