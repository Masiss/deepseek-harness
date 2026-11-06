//! Encodes terminal input that the program's enabled modes require.
//!
//! The screen parser reports which private modes the program turned on. Input
//! the user sends through the terminal panel is then shaped here: pasted text
//! is bracketed when the program asked for bracketed paste, and pointer
//! gestures become the mouse protocol the program selected.

use crate::terminal_ansi::{InputModes, MouseReporting};

/// One pointer gesture over the terminal grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseAction {
    /// A button went down; `0` left, `1` middle, `2` right.
    Press(u8),
    /// A button came up.
    Release(u8),
    /// The pointer moved while a button was held.
    Drag(u8),
    /// The pointer moved with no button held.
    Move,
    /// A wheel tick; `true` scrolls up.
    Wheel(bool),
}

impl MouseAction {
    /// The X10/SGR button code for this gesture, before the motion bit.
    fn button(self) -> u8 {
        match self {
            MouseAction::Press(button)
            | MouseAction::Release(button)
            | MouseAction::Drag(button) => button & 3,
            MouseAction::Move => 3,
            MouseAction::Wheel(true) => 64,
            MouseAction::Wheel(false) => 65,
        }
    }
}

/// Encodes pasted text for the program's current input mode.
///
/// @param text - Clipboard text exactly as received, including newlines.
/// @param bracketed - Whether the program enabled bracketed paste.
/// @returns The bytes to write to the terminal.
pub fn paste(text: &str, bracketed: bool) -> String {
    if !bracketed {
        return text.to_owned();
    }
    format!("\u{1b}[200~{text}\u{1b}[201~")
}

/// Encodes one pointer gesture, or `None` when the program does not track it.
///
/// @param modes - Input modes the program enabled.
/// @param action - Gesture to report.
/// @param col - One-based column under the pointer.
/// @param row - One-based row under the pointer.
/// @returns The bytes to write, or `None` when the gesture is not tracked.
pub fn mouse(modes: InputModes, action: MouseAction, col: usize, row: usize) -> Option<String> {
    let tracking = modes.mouse_reporting();
    if tracking == MouseReporting::Off {
        return None;
    }
    let motion = matches!(action, MouseAction::Drag(_) | MouseAction::Move);
    if motion
        && !matches!(
            tracking,
            MouseReporting::ButtonDrag | MouseReporting::AnyMotion
        )
    {
        return None;
    }
    if matches!(action, MouseAction::Move) && tracking != MouseReporting::AnyMotion {
        return None;
    }
    let x = col.max(1);
    let y = row.max(1);
    let mut code = action.button();
    if motion {
        code |= 32;
    }
    if modes.sgr_mouse {
        let final_byte = if matches!(action, MouseAction::Release(_)) {
            'm'
        } else {
            'M'
        };
        return Some(format!("\u{1b}[<{code};{x};{y}{final_byte}"));
    }
    // X10 carries one byte per field; the release code is button 3.
    if matches!(action, MouseAction::Release(_)) {
        code = 3;
    }
    if x > 223 || y > 223 {
        return None;
    }
    Some(format!(
        "\u{1b}[M{}{}{}",
        char::from(32 + code),
        char::from(32 + x as u8),
        char::from(32 + y as u8)
    ))
}

/// Encodes one named key press for the attached program.
///
/// @param key - gpui key name: `enter`, `tab`, `backspace`, `escape`, `delete`,
/// `insert`, `home`, `end`, `pageup`, `pagedown`, `up`, `down`, `left`, `right`,
/// `space`, or `f1` through `f12`.
/// @returns The bytes to write, or `None` for a key the terminal does not receive.
pub fn key(key: &str) -> Option<String> {
    Some(
        match key {
            "enter" => "\r",
            "tab" => "\t",
            "backspace" => "\u{7f}",
            "escape" => "\u{1b}",
            "delete" => "\u{1b}[3~",
            "insert" => "\u{1b}[2~",
            "home" => "\u{1b}[H",
            "end" => "\u{1b}[F",
            "pageup" => "\u{1b}[5~",
            "pagedown" => "\u{1b}[6~",
            "up" => "\u{1b}[A",
            "down" => "\u{1b}[B",
            "right" => "\u{1b}[C",
            "left" => "\u{1b}[D",
            "space" => " ",
            "ctrl-c" => "\u{3}",
            _ => return function_key(key),
        }
        .to_owned(),
    )
}

/// One function key's conventional sequence.
fn function_key(key: &str) -> Option<String> {
    let number: u8 = key.strip_prefix('f')?.parse().ok()?;
    Some(
        match number {
            1 => "\u{1b}OP",
            2 => "\u{1b}OQ",
            3 => "\u{1b}OR",
            4 => "\u{1b}OS",
            5 => "\u{1b}[15~",
            6 => "\u{1b}[17~",
            7 => "\u{1b}[18~",
            8 => "\u{1b}[19~",
            9 => "\u{1b}[20~",
            10 => "\u{1b}[21~",
            11 => "\u{1b}[23~",
            12 => "\u{1b}[24~",
            _ => return None,
        }
        .to_owned(),
    )
}

/// Encodes one vertical navigation step as the cursor key the program expects.
///
/// @param delta - `-1` for Up, `1` for Down.
/// @returns The bytes to write, or `None` for any other step.
pub fn arrow(delta: i8) -> Option<String> {
    match delta {
        -1 => key("up"),
        1 => key("down"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn modes(tracking: MouseReporting, sgr: bool) -> InputModes {
        let mut modes = InputModes {
            bracketed_paste: false,
            sgr_mouse: sgr,
            ..InputModes::default()
        };
        match tracking {
            MouseReporting::Off => {}
            MouseReporting::Press => modes.mouse_press = true,
            MouseReporting::ButtonDrag => modes.mouse_drag = true,
            MouseReporting::AnyMotion => modes.mouse_motion = true,
        }
        modes
    }

    #[test]
    fn named_keys_use_terminal_sequences() {
        assert_eq!(key("enter").as_deref(), Some("\r"));
        assert_eq!(key("tab").as_deref(), Some("\t"));
        assert_eq!(key("backspace").as_deref(), Some("\u{7f}"));
        assert_eq!(key("home").as_deref(), Some("\u{1b}[H"));
        assert_eq!(key("end").as_deref(), Some("\u{1b}[F"));
        assert_eq!(key("delete").as_deref(), Some("\u{1b}[3~"));
        assert_eq!(key("insert").as_deref(), Some("\u{1b}[2~"));
        assert_eq!(key("pageup").as_deref(), Some("\u{1b}[5~"));
        assert_eq!(key("pagedown").as_deref(), Some("\u{1b}[6~"));
        assert_eq!(key("left").as_deref(), Some("\u{1b}[D"));
        assert_eq!(key("right").as_deref(), Some("\u{1b}[C"));
        assert_eq!(key("space").as_deref(), Some(" "));
        assert_eq!(key("ctrl-c").as_deref(), Some("\u{3}"));
        assert_eq!(key("mediaplay"), None);
    }

    #[test]
    fn function_keys_use_their_conventional_sequences() {
        assert_eq!(key("f1").as_deref(), Some("\u{1b}OP"));
        assert_eq!(key("f4").as_deref(), Some("\u{1b}OS"));
        assert_eq!(key("f5").as_deref(), Some("\u{1b}[15~"));
        assert_eq!(key("f12").as_deref(), Some("\u{1b}[24~"));
        assert_eq!(key("f13"), None);
    }

    #[test]
    fn arrow_encodes_only_vertical_steps() {
        assert_eq!(arrow(-1).as_deref(), Some("\u{1b}[A"));
        assert_eq!(arrow(1).as_deref(), Some("\u{1b}[B"));
        assert_eq!(arrow(0), None);
        assert_eq!(arrow(2), None);
    }

    #[test]
    fn bracketed_paste_wraps_only_when_enabled() {
        assert_eq!(paste("a\nb", false), "a\nb");
        assert_eq!(paste("a\nb", true), "\u{1b}[200~a\nb\u{1b}[201~");
        assert_eq!(paste("", true), "\u{1b}[200~\u{1b}[201~");
    }

    #[test]
    fn sgr_reports_press_release_drag_and_wheel() {
        let sgr = modes(MouseReporting::AnyMotion, true);
        assert_eq!(
            mouse(sgr, MouseAction::Press(0), 3, 2).unwrap(),
            "\u{1b}[<0;3;2M"
        );
        assert_eq!(
            mouse(sgr, MouseAction::Release(0), 3, 2).unwrap(),
            "\u{1b}[<0;3;2m"
        );
        assert_eq!(
            mouse(sgr, MouseAction::Drag(0), 4, 2).unwrap(),
            "\u{1b}[<32;4;2M"
        );
        assert_eq!(
            mouse(sgr, MouseAction::Move, 5, 2).unwrap(),
            "\u{1b}[<35;5;2M"
        );
        assert_eq!(
            mouse(sgr, MouseAction::Wheel(true), 5, 2).unwrap(),
            "\u{1b}[<64;5;2M"
        );
        assert_eq!(
            mouse(sgr, MouseAction::Wheel(false), 5, 2).unwrap(),
            "\u{1b}[<65;5;2M"
        );
        // Coordinates stay one-based and are not clamped by X10 limits here.
        assert_eq!(
            mouse(sgr, MouseAction::Press(2), 500, 300).unwrap(),
            "\u{1b}[<2;500;300M"
        );
    }

    #[test]
    fn x10_uses_single_byte_fields_and_button_three_for_release() {
        let x10 = modes(MouseReporting::Press, false);
        assert_eq!(
            mouse(x10, MouseAction::Press(0), 3, 2).unwrap(),
            "\u{1b}[M\x20\x23\x22"
        );
        assert_eq!(
            mouse(x10, MouseAction::Release(0), 3, 2).unwrap(),
            "\u{1b}[M\x23\x23\x22"
        );
        // X10 cannot address past column/row 223.
        assert_eq!(mouse(x10, MouseAction::Press(0), 224, 2), None);
    }

    #[test]
    fn gestures_follow_the_tracking_mode_the_program_chose() {
        assert_eq!(
            mouse(
                modes(MouseReporting::Off, true),
                MouseAction::Press(0),
                1,
                1
            ),
            None
        );
        // Press-only tracking drops motion at any button state.
        let press = modes(MouseReporting::Press, true);
        assert_eq!(mouse(press, MouseAction::Drag(0), 1, 1), None);
        assert_eq!(mouse(press, MouseAction::Move, 1, 1), None);
        assert!(mouse(press, MouseAction::Press(0), 1, 1).is_some());
        // Button-drag tracking reports drags but not bare motion.
        let drag = modes(MouseReporting::ButtonDrag, true);
        assert!(mouse(drag, MouseAction::Drag(1), 1, 1).is_some());
        assert_eq!(mouse(drag, MouseAction::Move, 1, 1), None);
    }
}
