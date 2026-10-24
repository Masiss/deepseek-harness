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
