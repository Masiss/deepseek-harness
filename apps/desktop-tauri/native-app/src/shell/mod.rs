//! gpui presentation layer for the native Desktop shell.

mod app;
mod render;
mod terminal_state;
mod theme;
mod text_input;
mod widgets;

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use gpui::{
    actions, px, size, App, AppContext, Application, Bounds, KeyBinding, Point, TitlebarOptions,
    WindowBounds, WindowOptions,
};

use crate::command_registry::CommandId;
use crate::ui_preferences::UiPreferences;

actions!(
    shell,
    [
        Quit,
        NewSession,
        ToggleSettings,
        ToggleExplorer,
        ToggleTerminal,
        TogglePlugins,
        ToggleSkills,
        ToggleMarketplace,
        ToggleGoals,
    ]
);

/// Process-level services the shell is handed before it can draw.
pub(crate) struct ShellServices {
    pub quitting: Arc<AtomicBool>,
    pub show_requested: Arc<AtomicBool>,
    pub hide_requested: Arc<AtomicBool>,
    pub hidden: Arc<AtomicBool>,
    pub tray_available: bool,
    pub ui_preferences_path: PathBuf,
    /// Whether to start the Desktop Host; tests bring their own transport.
    pub spawn_host: bool,
    /// Commands dispatched by window shortcuts, drained by the poll loop.
    pub commands: Arc<Mutex<Vec<CommandId>>>,
}

/// Opens the native window and runs the shell until the app quits.
pub(crate) fn run(services: ShellServices) {
    let preferences = UiPreferences::load(&services.ui_preferences_path).unwrap_or_default();
    let commands = services.commands.clone();
    let app = Application::new();
    app.run(move |cx| {
        text_input::bind_keys(cx);
        bind_shortcuts(cx, &preferences, &commands);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                Point::new(px(60.), px(40.)),
                size(px(1440.), px(900.)),
            ))),
            titlebar: Some(TitlebarOptions {
                title: Some("DeepSeek Harness".into()),
                appears_transparent: false,
                traffic_light_position: None,
            }),
            window_min_size: Some(size(px(960.), px(620.))),
            app_id: Some("deepseek-harness".into()),
            ..Default::default()
        };
        let services = services;
        let _ = cx.open_window(options, move |window, cx| {
            cx.new(|cx| app::DesktopApp::new(services, window, cx))
        });
    });
}

/// Registers the configurable shell chords plus the fixed view shortcuts.
fn bind_shortcuts(cx: &mut App, preferences: &UiPreferences, commands: &Arc<Mutex<Vec<CommandId>>>) {
    cx.on_action(|_: &Quit, cx: &mut App| cx.quit());
    // gpui dispatches typed actions, so each chord needs its own handler.
    cx.on_action({
        let commands = commands.clone();
        move |_: &NewSession, _: &mut App| {
            commands
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(CommandId::NewSession);
        }
    });
    cx.on_action({
        let commands = commands.clone();
        move |_: &ToggleSettings, _: &mut App| {
            commands
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(CommandId::ToggleSettings);
        }
    });
    cx.on_action({
        let commands = commands.clone();
        move |_: &ToggleExplorer, _: &mut App| {
            commands
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(CommandId::ToggleExplorer);
        }
    });
    cx.on_action({
        let commands = commands.clone();
        move |_: &ToggleTerminal, _: &mut App| {
            commands
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(CommandId::ToggleTerminal);
        }
    });
    cx.on_action({
        let commands = commands.clone();
        move |_: &TogglePlugins, _: &mut App| {
            commands
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(CommandId::TogglePlugins);
        }
    });
    cx.on_action({
        let commands = commands.clone();
        move |_: &ToggleSkills, _: &mut App| {
            commands
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(CommandId::ToggleSkills);
        }
    });
    cx.on_action({
        let commands = commands.clone();
        move |_: &ToggleMarketplace, _: &mut App| {
            commands
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(CommandId::ToggleMarketplace);
        }
    });
    cx.on_action({
        let commands = commands.clone();
        move |_: &ToggleGoals, _: &mut App| {
            commands
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(CommandId::ToggleGoals);
        }
    });

    let mut bindings = Vec::new();
    let mut configured = Vec::new();
    if let Some(binding) = preference_binding(&preferences.keybindings.new_session, NewSession) {
        configured.push(binding);
    }
    if let Some(binding) = preference_binding(&preferences.keybindings.open_settings, ToggleSettings) {
        configured.push(binding);
    }
    if let Some(binding) = preference_binding(&preferences.keybindings.open_marketplace, ToggleMarketplace) {
        configured.push(binding);
    }
    if let Some(binding) = preference_binding(&preferences.keybindings.open_goals, ToggleGoals) {
        configured.push(binding);
    }
    bindings.extend(configured);
    bindings.push(KeyBinding::new("ctrl-shift-e", ToggleExplorer, None));
    bindings.push(KeyBinding::new("ctrl-shift-t", ToggleTerminal, None));
    bindings.push(KeyBinding::new("ctrl-shift-x", TogglePlugins, None));
    bindings.push(KeyBinding::new("ctrl-shift-k", ToggleSkills, None));
    cx.bind_keys(bindings);
}

/// Converts a `Ctrl+Shift+S`-style preference chord into gpui keystroke syntax.
fn preference_chord(text: &str) -> Option<String> {
    let mut parts = Vec::new();
    let mut key = None;
    for token in text.split('+').map(str::trim).filter(|token| !token.is_empty()) {
        match token.to_ascii_lowercase().as_str() {
            "ctrl" | "control" | "cmd" | "command" => parts.push("ctrl".to_owned()),
            "shift" => parts.push("shift".to_owned()),
            "alt" | "option" => parts.push("alt".to_owned()),
            _ => key = Some(normalize_key(token)),
        }
    }
    let key = key?;
    if key == "none" {
        return None;
    }
    parts.push(key);
    Some(parts.join("-"))
}

/// Builds one configured chord binding, if the preference names a live chord.
fn preference_binding(
    text: &str,
    action: impl gpui::Action + 'static,
) -> Option<gpui::KeyBinding> {
    let keystroke = preference_chord(text)?;
    if keystroke == "none" {
        return None;
    }
    Some(gpui::KeyBinding::new(&keystroke, action, None))
}

fn normalize_key(token: &str) -> String {
    match token.to_ascii_lowercase().as_str() {
        "comma" => ",".to_owned(),
        "period" => ".".to_owned(),
        "plus" => "+".to_owned(),
        "minus" => "-".to_owned(),
        "space" => "space".to_owned(),
        "escape" | "esc" => "escape".to_owned(),
        other => other.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestAction;

    impl gpui::Action for TestAction {
        fn boxed_clone(&self) -> Box<dyn gpui::Action> {
            Box::new(TestAction)
        }

        fn partial_eq(&self, action: &dyn gpui::Action) -> bool {
            action.name() == "shell::tests::TestAction"
        }

        fn name(&self) -> &'static str {
            "shell::tests::TestAction"
        }

        fn name_for_type() -> &'static str {
            "shell::tests::TestAction"
        }

        fn build(
            _: serde_json::Value,
        ) -> std::result::Result<Box<dyn gpui::Action + 'static>, gpui::private::anyhow::Error> {
            unimplemented!("test action has no serialized form")
        }
    }

    #[test]
    fn preference_chord_rejects_empty_input() {
        assert!(preference_chord("").is_none());
    }

    #[test]
    fn preference_chord_rejects_the_chord_word() {
        assert!(preference_chord("none").is_none());
    }

    #[test]
    fn preference_chord_maps_ctrl_shift_s() {
        assert_eq!(preference_chord("Ctrl+Shift+S").as_deref(), Some("ctrl-shift-s"));
    }

    #[test]
    fn preference_chord_keeps_letter_keys() {
        assert_eq!(preference_chord("ctrl+shift+e").as_deref(), Some("ctrl-shift-e"));
    }

    #[test]
    fn preference_chord_maps_modified_keys() {
        assert_eq!(
            preference_chord("ctrl+shift+alt+x").as_deref(),
            Some("ctrl-shift-alt-x")
        );
        assert_eq!(
            preference_chord("control+option+shift+escape").as_deref(),
            Some("ctrl-alt-shift-escape")
        );
    }

    #[test]
    fn preference_chord_requires_a_key() {
        assert!(preference_chord("ctrl+shift").is_none());
    }

    #[test]
    fn normalize_key_maps_aliases() {
        assert_eq!(normalize_key("Comma"), ",");
        assert_eq!(normalize_key("PERIOD"), ".");
        assert_eq!(normalize_key("Plus"), "+");
        assert_eq!(normalize_key("Minus"), "-");
        assert_eq!(normalize_key("Space"), "space");
        assert_eq!(normalize_key("Escape"), "escape");
    }

    #[test]
    fn normalize_key_keeps_digits_and_letters_lowercased() {
        assert_eq!(normalize_key("F5"), "f5");
        assert_eq!(normalize_key("A"), "a");
    }

    #[test]
    fn preference_binding_rejects_chord_word() {
        assert!(preference_binding("none", TestAction).is_none());
    }

    #[test]
    fn preference_binding_accepts_live_chord() {
        assert!(preference_binding("ctrl+shift+s", TestAction).is_some());
    }
}
