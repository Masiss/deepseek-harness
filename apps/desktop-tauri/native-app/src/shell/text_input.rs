//! Focused text input with IME, selection, and multiline editing for gpui.
//!
//! Adapted from gpui's `examples/input.rs`; the shell needs an entity whose
//! value the application reads and resets, so this widget owns only editing
//! state and emits [`TextInputEvent`] on change and submit.

use std::ops::Range;

use gpui::{
    actions, div, fill, point, prelude::*, px, relative, size, App, Bounds, ClipboardItem,
    Context, CursorStyle, Element, ElementId, ElementInputHandler, Entity, EntityInputHandler,
    EventEmitter, FocusHandle, Focusable, Font, FontWeight, GlobalElementId, Hsla, InspectorElementId,
    KeyBinding, KeyDownEvent, LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    PaintQuad, Pixels, SharedString, Style, TextRun, UTF16Selection, UnderlineStyle, Window,
};
use unicode_segmentation::UnicodeSegmentation;

use super::theme::Theme;

actions!(
    text_input,
    [
        Backspace,
        Delete,
        Left,
        Right,
        Up,
        Down,
        SelectLeft,
        SelectRight,
        SelectAll,
        Home,
        End,
        ShowCharacterPalette,
        EscapeInput,
        Paste,
        Cut,
        Copy,
        Undo,
        Redo,
    ]
);

/// Events a text input emits to its owner.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TextInputEvent {
    /// The value changed; the string is the complete new value.
    Changed(String),
    /// Enter (single line) or Ctrl/Cmd+Enter (multiline) requested submission.
    Submit,
    /// A single-line input received Up/Down for list navigation.
    Navigate(i8),
    /// A single-line input received a named cursor key a terminal can encode.
    Key(&'static str),
}

/// Registers the editing chords every text input uses.
pub(crate) fn bind_keys(cx: &mut App) {
    let mut bindings = Vec::new();
    for context in ["TextInput", "TextInputSingle"] {
        let context = Some(context);
        bindings.extend([
            KeyBinding::new("backspace", Backspace, context),
            KeyBinding::new("delete", Delete, context),
            KeyBinding::new("left", Left, context),
            KeyBinding::new("right", Right, context),
            KeyBinding::new("up", Up, context),
            KeyBinding::new("down", Down, context),
            KeyBinding::new("shift-left", SelectLeft, context),
            KeyBinding::new("shift-right", SelectRight, context),
            KeyBinding::new("shift-up", Up, context),
            KeyBinding::new("shift-down", Down, context),
            KeyBinding::new("home", Home, context),
            KeyBinding::new("end", End, context),
            KeyBinding::new("ctrl-a", SelectAll, context),
            KeyBinding::new("ctrl-c", Copy, context),
            KeyBinding::new("ctrl-x", Cut, context),
            KeyBinding::new("ctrl-v", Paste, context),
            KeyBinding::new("ctrl-z", Undo, context),
            KeyBinding::new("ctrl-shift-z", Redo, context),
            KeyBinding::new("ctrl-shift-p", ShowCharacterPalette, context),
            KeyBinding::new("escape", EscapeInput, context),
        ]);
    }
    cx.bind_keys(bindings);
}

/// Named keys a single-line input reports without an editing action of its own.
fn unbound_named_key(key: &str) -> Option<&'static str> {
    match key {
        "pageup" => Some("pageup"),
        "pagedown" => Some("pagedown"),
        "insert" => Some("insert"),
        "f1" => Some("f1"),
        "f2" => Some("f2"),
        "f3" => Some("f3"),
        "f4" => Some("f4"),
        "f5" => Some("f5"),
        "f6" => Some("f6"),
        "f7" => Some("f7"),
        "f8" => Some("f8"),
        "f9" => Some("f9"),
        "f10" => Some("f10"),
        "f11" => Some("f11"),
        "f12" => Some("f12"),
        _ => None,
    }
}

/// Editing state for one text field.
pub(crate) struct TextInput {
    focus_handle: FocusHandle,
    content: SharedString,
    placeholder: SharedString,
    multiline: bool,
    selected_range: Range<usize>,
    selection_reversed: bool,
    marked_range: Option<Range<usize>>,
    last_layout: Vec<gpui::WrappedLine>,
    last_bounds: Option<Bounds<Pixels>>,
    last_scroll: Pixels,
    is_selecting: bool,
    undo: Vec<String>,
    redo: Vec<String>,
}

impl EventEmitter<TextInputEvent> for TextInput {}

impl TextInput {
    /// Creates an input entity with a placeholder and multiline capability.
    pub(crate) fn new(
        placeholder: impl Into<SharedString>,
        multiline: bool,
        cx: &mut Context<Self>,
    ) -> Entity<Self> {
        cx.new(|cx| Self::state(placeholder, multiline, cx))
    }

    /// Builds the editing value inside an owner's `Context::new` closure.
    pub(crate) fn state(
        placeholder: impl Into<SharedString>,
        multiline: bool,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            content: SharedString::default(),
            placeholder: placeholder.into(),
            multiline,
            selected_range: 0..0,
            selection_reversed: false,
            marked_range: None,
            last_layout: Vec::new(),
            last_bounds: None,
            last_scroll: px(0.),
            is_selecting: false,
            undo: Vec::new(),
            redo: Vec::new(),
        }
    }

    /// The current value.
    pub(crate) fn text(&self) -> &str {
        &self.content
    }

    /// Replaces the value without emitting a change event.
    pub(crate) fn set_text(&mut self, text: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.content = text.into();
        self.selected_range = self.content.len()..self.content.len();
        self.marked_range = None;
        self.undo.clear();
        self.redo.clear();
        cx.notify();
    }

    /// Places the caret at the start of one 1-based line.
    ///
    /// The viewport follows the caret, so this scrolls an off-screen line into
    /// view without a separate scroll offset.
    pub(crate) fn reveal_line(&mut self, line: u64, cx: &mut Context<Self>) {
        let offset = line_start_offset(&self.content, line);
        self.selected_range = offset..offset;
        self.selection_reversed = false;
        cx.notify();
    }

    /// Places the caret at the end and requests focus.
    pub(crate) fn focus(&mut self, window: &mut Window, _cx: &mut Context<Self>) {
        self.selected_range = self.content.len()..self.content.len();
        self.focus_handle.focus(window);
    }

    /// Whether this input currently owns the keyboard.
    pub(crate) fn is_focused(&self, window: &Window) -> bool {
        self.focus_handle.is_focused(window)
    }

    /// Selects the whole value, used when a field is opened by a shortcut.
    pub(crate) fn select_all_text(&mut self, cx: &mut Context<Self>) {
        self.selected_range = 0..self.content.len();
        self.selection_reversed = false;
        cx.notify();
    }

    fn push_undo(&mut self) {
        if self.undo.last().is_some_and(|last| last == &self.content) {
            return;
        }
        self.undo.push(self.content.to_string());
        if self.undo.len() > 100 {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    fn undo(&mut self, _: &Undo, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(previous) = self.undo.pop() {
            self.redo.push(self.content.to_string());
            self.content = previous.into();
            self.selected_range = self.content.len()..self.content.len();
            self.emit_changed(cx);
            cx.notify();
            window.invalidate_character_coordinates();
        }
    }

    fn redo(&mut self, _: &Redo, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(next) = self.redo.pop() {
            self.undo.push(self.content.to_string());
            self.content = next.into();
            self.selected_range = self.content.len()..self.content.len();
            self.emit_changed(cx);
            cx.notify();
            window.invalidate_character_coordinates();
        }
    }

    fn emit_changed(&self, cx: &mut Context<Self>) {
        cx.emit(TextInputEvent::Changed(self.content.to_string()));
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        cx.emit(TextInputEvent::Submit);
        window.invalidate_character_coordinates();
    }

    fn cursor_offset(&self) -> usize {
        if self.selection_reversed {
            self.selected_range.start
        } else {
            self.selected_range.end
        }
    }

    fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.selected_range = offset..offset;
        self.selection_reversed = false;
        cx.notify();
    }

    fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        if self.selection_reversed {
            self.selected_range.start = offset;
        } else {
            self.selected_range.end = offset;
        }
        if self.selected_range.end < self.selected_range.start {
            self.selection_reversed = !self.selection_reversed;
            self.selected_range = self.selected_range.end..self.selected_range.start;
        }
        cx.notify();
    }

    fn previous_boundary(&self, offset: usize) -> usize {
        self.content[..offset]
            .grapheme_indices(true)
            .next_back()
            .map_or(0, |(index, _)| index)
    }

    fn next_boundary(&self, offset: usize) -> usize {
        self.content[offset..]
            .grapheme_indices(true)
            .nth(1)
            .map_or(self.content.len(), |(index, _)| offset + index)
    }

    fn left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.move_to(self.previous_boundary(self.cursor_offset()), cx);
        } else {
            self.move_to(self.selected_range.start, cx);
        }
        self.emit_named_key("left", cx);
    }

    fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.move_to(self.next_boundary(self.selected_range.end), cx);
        } else {
            self.move_to(self.selected_range.end, cx);
        }
        self.emit_named_key("right", cx);
    }

    fn up(&mut self, _: &Up, _: &mut Window, cx: &mut Context<Self>) {
        if self.multiline {
            self.move_vertical(-1, cx);
        } else {
            cx.emit(TextInputEvent::Navigate(-1));
        }
    }

    fn down(&mut self, _: &Down, _: &mut Window, cx: &mut Context<Self>) {
        if self.multiline {
            self.move_vertical(1, cx);
        } else {
            cx.emit(TextInputEvent::Navigate(1));
        }
    }

    fn move_vertical(&mut self, direction: i32, cx: &mut Context<Self>) {
        if self.last_layout.is_empty() {
            return;
        }
        let caret = self.cursor_offset();
        let (line_index, line_start) = self.line_for_offset(caret);
        let Some(line) = self.last_layout.get(line_index) else {
            return;
        };
        let line_height = self.line_height();
        let Some(position) = line.position_for_index(caret - line_start, line_height) else {
            return;
        };
        let target_y = position.y + line_height * direction as f32;
        let target_line = if direction < 0 {
            line_index.saturating_sub(1)
        } else {
            (line_index + 1).min(self.last_layout.len().saturating_sub(1))
        };
        if target_line == line_index {
            // Move to the start or end of the current line when at an edge.
            let offset = if direction < 0 {
                line_start
            } else {
                line_start + line.len()
            };
            self.move_to(offset, cx);
            return;
        }
        let Some(target) = self.last_layout.get(target_line) else {
            return;
        };
        let starts = self.line_starts();
        let base = starts[target_line];
        let x = position.x;
        let index = target.closest_index_for_position(
            point(x, target_y.min(target.size(line_height).height - px(1.0))),
            line_height,
        );
        let index = index.unwrap_or_else(|index| index);
        self.move_to(base + index.min(target.len()), cx);
    }

    fn line_starts(&self) -> Vec<usize> {
        let mut starts = Vec::with_capacity(self.last_layout.len());
        let mut offset = 0;
        for line in &self.last_layout {
            starts.push(offset);
            offset += line.len() + 1;
        }
        starts
    }

    fn line_for_offset(&self, offset: usize) -> (usize, usize) {
        let starts = self.line_starts();
        // Before the first paint there is no layout to index: a key action can
        // arrive while the field is focused but unpainted.
        if starts.is_empty() {
            return (0, 0);
        }
        let index = starts
            .iter()
            .enumerate()
            .rev()
            .find(|(_, start)| **start <= offset)
            .map_or(0, |(index, _)| index);
        (index, starts[index])
    }

    fn line_height(&self) -> Pixels {
        px(19.0)
    }

    fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.previous_boundary(self.cursor_offset()), cx);
    }

    fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.next_boundary(self.cursor_offset()), cx);
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, cx);
        self.select_to(self.content.len(), cx);
    }

    fn home(&mut self, _: &Home, _: &mut Window, cx: &mut Context<Self>) {
        let caret = self.cursor_offset();
        let (_, start) = self.line_for_offset(caret);
        self.move_to(start, cx);
        self.emit_named_key("home", cx);
    }

    fn end(&mut self, _: &End, _: &mut Window, cx: &mut Context<Self>) {
        let caret = self.cursor_offset();
        let (index, start) = self.line_for_offset(caret);
        let end = self
            .last_layout
            .get(index)
            .map_or(self.content.len(), |line| start + line.len());
        self.move_to(end, cx);
        self.emit_named_key("end", cx);
    }

    /// Reports Escape for terminal input, then lets the shell dismiss overlays.
    fn escape_input(&mut self, _: &EscapeInput, _: &mut Window, cx: &mut Context<Self>) {
        self.emit_named_key("escape", cx);
        cx.propagate();
    }

    /// Reports a cursor key only for a single-line field.
    fn emit_named_key(&self, key: &'static str, cx: &mut Context<Self>) {
        if !self.multiline {
            cx.emit(TextInputEvent::Key(key));
        }
    }

    fn backspace(&mut self, _: &Backspace, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.select_to(self.previous_boundary(self.cursor_offset()), cx);
        }
        self.replace_text_in_range(None, "", window, cx);
        self.emit_named_key("backspace", cx);
    }

    fn delete(&mut self, _: &Delete, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.select_to(self.next_boundary(self.selected_range.end), cx);
        }
        self.replace_text_in_range(None, "", window, cx);
        self.emit_named_key("delete", cx);
    }

    fn on_mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.is_selecting = true;
        if event.modifiers.shift {
            self.select_to(self.index_for_mouse_position(event.position), cx);
        } else {
            self.move_to(self.index_for_mouse_position(event.position), cx);
        }
        window.focus(&self.focus_handle);
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        self.is_selecting = false;
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.is_selecting {
            self.select_to(self.index_for_mouse_position(event.position), cx);
        }
    }

    fn show_character_palette(
        &mut self,
        _: &ShowCharacterPalette,
        window: &mut Window,
        _: &mut Context<Self>,
    ) {
        window.show_character_palette();
    }

    fn paste(&mut self, _: &Paste, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.replace_text_in_range(None, &text, window, cx);
        }
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.emit_named_key("ctrl-c", cx);
            return;
        }
        cx.write_to_clipboard(ClipboardItem::new_string(
            self.content[self.selected_range.clone()].to_string(),
        ));
    }

    fn cut(&mut self, _: &Cut, window: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.content[self.selected_range.clone()].to_string(),
            ));
            self.replace_text_in_range(None, "", window, cx);
        }
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        if keystroke.key == "enter" {
            let submit = !self.multiline || keystroke.modifiers.control || keystroke.modifiers.platform;
            if submit {
                self.submit(window, cx);
            } else {
                self.replace_text_in_range(None, "\n", window, cx);
            }
        }
        if let Some(key) = unbound_named_key(keystroke.key.as_str()) {
            self.emit_named_key(key, cx);
        }
    }

    fn index_for_mouse_position(&self, position: gpui::Point<Pixels>) -> usize {
        let Some(bounds) = self.last_bounds else {
            return self.content.len();
        };
        let Some(local) = bounds.localize(&position) else {
            return self.content.len();
        };
        let local = point(local.x, local.y + self.last_scroll);
        let line_height = self.line_height();
        let mut line_top = px(0.);
        let mut line_start = 0;
        for line in &self.last_layout {
            let line_height_total = line.size(line_height).height;
            if local.y > line_top + line_height_total && line_height_total > px(0.) {
                line_top += line_height_total;
                line_start += line.len() + 1;
                continue;
            }
            let within = point(local.x, local.y - line_top);
            return match line.index_for_position(within, line_height) {
                Ok(index) => line_start + index,
                Err(index) => line_start + index,
            };
        }
        self.content.len()
    }

    fn range_from_utf16(&self, range_utf16: &Range<usize>) -> Range<usize> {
        let mut utf8 = 0;
        let mut start = None;
        let mut end = None;
        for (index, character) in self.content.char_indices() {
            let utf16 = character.len_utf16();
            if start.is_none() && utf8 >= range_utf16.start {
                start = Some(index);
            }
            if end.is_none() && utf8 >= range_utf16.end {
                end = Some(index);
                break;
            }
            utf8 += utf16;
        }
        start.unwrap_or(self.content.len())..end.unwrap_or(self.content.len())
    }

    fn offset_to_utf16(&self, offset: usize) -> usize {
        self.content[..offset.min(self.content.len())]
            .chars()
            .map(char::len_utf16)
            .sum()
    }

    fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_to_utf16(range.start)..self.offset_to_utf16(range.end)
    }
}

impl Focusable for TextInput {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EntityInputHandler for TextInput {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.range_from_utf16(&range_utf16);
        adjusted_range.replace(self.range_to_utf16(&range));
        Some(self.content[range].to_string())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.range_to_utf16(&self.selected_range),
            reversed: self.selection_reversed,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.marked_range
            .as_ref()
            .map(|range| self.range_to_utf16(range))
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        self.marked_range = None;
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|range_utf16| self.range_from_utf16(range_utf16))
            .or_else(|| self.marked_range.clone())
            .unwrap_or_else(|| self.selected_range.clone());
        self.push_undo();
        self.content =
            (self.content[0..range.start].to_owned() + new_text + &self.content[range.end..])
                .into();
        self.selected_range = range.start + new_text.len()..range.start + new_text.len();
        self.marked_range.take();
        self.emit_changed(cx);
        cx.notify();
        window.invalidate_character_coordinates();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|range_utf16| self.range_from_utf16(range_utf16))
            .or_else(|| self.marked_range.clone())
            .unwrap_or_else(|| self.selected_range.clone());
        if self.marked_range.is_none() {
            self.push_undo();
        }
        self.content =
            (self.content[0..range.start].to_owned() + new_text + &self.content[range.end..])
                .into();
        self.marked_range = if new_text.is_empty() {
            None
        } else {
            Some(range.start..range.start + new_text.len())
        };
        self.selected_range = new_selected_range_utf16
            .as_ref()
            .map(|range_utf16| self.range_from_utf16(range_utf16))
            .map(|new_range| new_range.start + range.start..new_range.end + range.start)
            .unwrap_or_else(|| range.start + new_text.len()..range.start + new_text.len());
        if self.marked_range.is_none() {
            self.emit_changed(cx);
        }
        cx.notify();
        window.invalidate_character_coordinates();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let range = self.range_from_utf16(&range_utf16);
        let (line_index, start) = self.line_for_offset(range.start);
        let line = self.last_layout.get(line_index)?;
        let line_height = self.line_height();
        let position = line.position_for_index(range.start - start, line_height)?;
        let end_position = line
            .position_for_index((range.end - start).min(line.len()), line_height)
            .unwrap_or(position);
        Some(Bounds::from_corners(
            point(bounds.left() + position.x, bounds.top() + position.y),
            point(bounds.left() + end_position.x, bounds.top() + end_position.y + line_height),
        ))
    }

    fn character_index_for_point(
        &mut self,
        point: gpui::Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        Some(self.index_for_mouse_position(point))
    }
}

struct TextInputElement {
    input: Entity<TextInput>,
    theme: Theme,
    height: Pixels,
}

struct TextInputPrepaint {
    lines: Vec<gpui::WrappedLine>,
    cursor: Option<PaintQuad>,
    line_height: Pixels,
    scroll: Pixels,
}

impl IntoElement for TextInputElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for TextInputElement {
    type RequestLayoutState = ();
    type PrepaintState = TextInputPrepaint;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = self.height.into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let input = self.input.read(cx);
        let content = input.content.clone();
        let selected_range = input.selected_range.clone();
        let marked_range = input.marked_range.clone();
        let cursor = input.cursor_offset();
        let theme = &self.theme;
        let line_height = input.line_height();

        let (display_text, text_color) = if content.is_empty() {
            (input.placeholder.clone(), theme.muted)
        } else {
            (content.clone(), theme.text)
        };
        let font_size = window.text_style().font_size.to_pixels(window.rem_size());
        let font = theme.sans_font();
        let runs = build_runs(
            &display_text,
            selected_range.clone(),
            marked_range.clone(),
            font,
            text_color,
            theme.selection,
            theme.accent,
        );

        let wrap_width = bounds.size.width;
        let lines: Vec<gpui::WrappedLine> = window
            .text_system()
            .shape_text(display_text.clone(), font_size, &runs, Some(wrap_width), None)
            .map(|lines| lines.into_iter().collect())
            .unwrap_or_default();

        // Keep the caret inside the fixed viewport by scrolling minimally.
        let mut scroll = px(0.);
        let total: Pixels = lines
            .iter()
            .fold(px(0.), |total, line| total + line.size(line_height).height);
        let visible = bounds.size.height;
        if content.is_empty() {
            scroll = px(0.);
        } else {
            let (line_index, line_start) = input.line_for_offset(cursor);
            let caret_y: Pixels = lines
                .iter()
                .take(line_index)
                .fold(px(0.), |total, line| total + line.size(line_height).height);
            let caret_local = lines
                .get(line_index)
                .and_then(|line| line.position_for_index(cursor.saturating_sub(line_start), line_height))
                .map_or(caret_y, |position| caret_y + position.y);
            if caret_local < input.last_scroll {
                scroll = caret_local;
            } else if caret_local + line_height > input.last_scroll + visible {
                scroll = caret_local + line_height - visible;
            } else {
                scroll = input.last_scroll;
            }
            scroll = scroll.clamp(px(0.), (total - visible).max(px(0.)));
        }

        let cursor = if selected_range.is_empty() && input.focus_handle.is_focused(window) {
            let (line_index, line_start) = input.line_for_offset(cursor);
            lines.get(line_index).and_then(|line| {
                let position =
                    line.position_for_index(cursor.saturating_sub(line_start), line_height)?;
                let line_top: Pixels = lines
                    .iter()
                    .take(line_index)
                    .fold(px(0.), |total, line| total + line.size(line_height).height);
                Some(fill(
                    Bounds::new(
                        point(
                            bounds.left() + position.x,
                            bounds.top() + line_top + position.y - scroll,
                        ),
                        size(px(1.5), line_height),
                    ),
                    theme.text,
                ))
            })
        } else {
            None
        };

        TextInputPrepaint {
            lines,
            cursor,
            line_height,
            scroll,
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus_handle = self.input.read(cx).focus_handle.clone();
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.input.clone()),
            cx,
        );
        let mut y = bounds.top() - prepaint.scroll;
        for line in &prepaint.lines {
            let height = line.size(prepaint.line_height).height;
            line.paint(
                point(bounds.left(), y),
                prepaint.line_height,
                gpui::TextAlign::Left,
                Some(bounds),
                window,
                cx,
            )
            .ok();
            y += height;
        }
        if let Some(cursor) = prepaint.cursor.take() {
            window.paint_quad(cursor);
        }
        let scroll = prepaint.scroll;
        self.input.update(cx, |input, _cx| {
            input.last_layout = prepaint.lines.clone();
            input.last_bounds = Some(bounds);
            input.last_scroll = scroll;
        });
    }
}

/// Builds text runs with selection and IME-marked decorations.
fn build_runs(
    text: &str,
    selected: Range<usize>,
    marked: Option<Range<usize>>,
    font: Font,
    color: Hsla,
    selection: Hsla,
    accent: Hsla,
) -> Vec<TextRun> {
    let mut boundaries = vec![0, text.len(), selected.start, selected.end];
    if let Some(marked) = marked.as_ref() {
        boundaries.extend([marked.start, marked.end]);
    }
    boundaries.sort_unstable();
    boundaries.dedup();
    let mut runs = Vec::new();
    for pair in boundaries.windows(2) {
        let start = pair[0];
        let end = pair[1];
        if start >= end {
            continue;
        }
        let in_selection = start >= selected.start && end <= selected.end && !selected.is_empty();
        let in_marked = marked
            .as_ref()
            .is_some_and(|marked| start >= marked.start && end <= marked.end);
        runs.push(TextRun {
            len: end - start,
            font: font.clone(),
            color,
            background_color: in_selection.then_some(selection),
            underline: in_marked.then_some(UnderlineStyle {
                color: Some(accent),
                thickness: px(1.0),
                wavy: false,
            }),
            strikethrough: None,
        });
    }
    if runs.is_empty() {
        runs.push(TextRun {
            len: text.len().max(1),
            font,
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        });
    }
    runs
}

impl Render for TextInput {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let multiline = self.multiline;
        div()
            .key_context(if multiline { "TextInput" } else { "TextInputSingle" })
            .track_focus(&self.focus_handle(cx))
            .cursor(CursorStyle::IBeam)
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::up))
            .on_action(cx.listener(Self::down))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::home))
            .on_action(cx.listener(Self::end))
            .on_action(cx.listener(Self::show_character_palette))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::undo))
            .on_action(cx.listener(Self::redo))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_key_down(cx.listener(Self::on_key_down))
            .size_full()
    }
}

/// Renders an input entity with the shell theme and a fixed viewport height.
pub(crate) fn text_input_element(
    input: &Entity<TextInput>,
    theme: &Theme,
    height: Pixels,
) -> impl IntoElement {
    div()
        .w_full()
        .text_size(theme.fs(13.0))
        .line_height(theme.fs(19.0))
        .font_weight(FontWeight::NORMAL)
        .font(theme.sans_font())
        .child(TextInputElement {
            input: input.clone(),
            theme: theme.clone(),
            height,
        })
}

/// Byte offset of the first character of a 1-based line, clamped to the text.
fn line_start_offset(content: &str, line: u64) -> usize {
    let mut offset = 0;
    let mut remaining = line.saturating_sub(1);
    while remaining > 0 {
        let Some(next) = content[offset..].find('\n') else {
            return content.len();
        };
        offset += next + 1;
        remaining -= 1;
    }
    offset
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reveal_line_offsets_count_newlines_and_clamp_past_the_end() {
        let content = "one\ntwo\nthree";
        assert_eq!(line_start_offset(content, 1), 0);
        assert_eq!(line_start_offset(content, 2), 4);
        assert_eq!(line_start_offset(content, 3), 8);
        assert_eq!(line_start_offset(content, 99), content.len());
        assert_eq!(line_start_offset("", 3), 0);
        assert_eq!(line_start_offset("one\n", 2), 4);
    }

    #[test]
    fn unbound_named_keys_cover_page_function_and_insert_keys() {
        assert_eq!(unbound_named_key("pageup"), Some("pageup"));
        assert_eq!(unbound_named_key("pagedown"), Some("pagedown"));
        assert_eq!(unbound_named_key("insert"), Some("insert"));
        assert_eq!(unbound_named_key("f1"), Some("f1"));
        assert_eq!(unbound_named_key("f12"), Some("f12"));
        assert_eq!(unbound_named_key("f13"), None);
        assert_eq!(unbound_named_key("enter"), None);
        assert_eq!(unbound_named_key("a"), None);
    }

    fn test_font() -> Font {
        Font {
            family: "Sans".into(),
            features: Default::default(),
            fallbacks: None,
            weight: FontWeight::NORMAL,
            style: gpui::FontStyle::Normal,
        }
    }

    #[test]
    fn ime_marked_text_renders_underlined_and_selected_text_highlighted() {
        let color = Hsla { h: 0., s: 0., l: 1., a: 1. };
        let selection = Hsla { h: 0., s: 0., l: 0., a: 1. };
        let accent = Hsla { h: 0., s: 1., l: 0.5, a: 1. };
        // "hello" with "ell" marked by IME composition.
        let runs = build_runs("hello", 0..0, Some(1..4), test_font(), color, selection, accent);
        // Runs cover the whole text exactly once.
        assert_eq!(runs.iter().map(|run| run.len).sum::<usize>(), 5);
        // The marked slice carries the IME underline; outer runs do not.
        let marked = runs.iter().filter(|run| run.underline.is_some()).count();
        assert_eq!(marked, 1);
        // Selection highlights without an IME mark when nothing is composing.
        let plain = build_runs("hello", 1..3, None, test_font(), color, selection, accent);
        assert_eq!(plain.iter().map(|run| run.len).sum::<usize>(), 5);
        assert_eq!(plain.iter().filter(|run| run.underline.is_some()).count(), 0);
        assert_eq!(plain.iter().filter(|run| run.background_color.is_some()).count(), 1);
    }
}
