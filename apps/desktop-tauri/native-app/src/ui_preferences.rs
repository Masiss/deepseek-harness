//! Client-local appearance preferences, separate from Host-owned settings.

#[cfg(feature = "legacy-egui")]
use eframe::egui;
#[cfg(feature = "legacy-egui")]
use egui::Color32;
use crate::command_registry::CommandId;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::error::Error as StdError;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

const SCHEMA_VERSION: u32 = 4;
/// First on-disk format; its fields are all still read, new ones stay defaulted.
const SCHEMA_VERSION_V1: u32 = 1;
/// Largest composer draft persisted for one Session, in bytes.
pub const SESSION_DRAFT_BYTES_MAX: usize = 64 * 1024;
/// Largest number of Session drafts persisted in the preferences file.
pub const SESSION_DRAFTS_MAX: usize = 200;
const UI_SCALE_MIN: f32 = 0.8;
const UI_SCALE_MAX: f32 = 1.5;
const RADIUS_MIN: f32 = 0.0;
const RADIUS_MAX: f32 = 12.0;
const SESSION_PANEL_WIDTH_MIN: f32 = crate::layout::SIDEBAR_MIN;
const SESSION_PANEL_WIDTH_MAX: f32 = crate::layout::SIDEBAR_MAX;
const WORKSPACE_PANEL_WIDTH_MIN: f32 = crate::layout::EXPLORER_MIN;
const CHANGES_PANEL_WIDTH_MIN: f32 = crate::layout::CHANGES_MIN;
const CHANGES_PANEL_WIDTH_MAX: f32 = crate::layout::CHANGES_MAX;
const WORKSPACE_PANEL_WIDTH_MAX: f32 = crate::layout::EXPLORER_MAX;
const INVALID_CHORD: &str = "keyboard shortcuts look like Ctrl+Shift+S, or none";

static NEXT_TEMP_FILE: AtomicU64 = AtomicU64::new(0);

/// Selects an explicit theme or follows the desktop environment.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeMode {
    /// Follow the system's current theme.
    #[default]
    System,
    /// Use the light egui theme.
    Light,
    /// Use the dark egui theme.
    Dark,
}

#[cfg(feature = "legacy-egui")]
impl From<ThemeMode> for egui::ThemePreference {
    fn from(value: ThemeMode) -> Self {
        match value {
            ThemeMode::System => Self::System,
            ThemeMode::Light => Self::Light,
            ThemeMode::Dark => Self::Dark,
        }
    }
}

/// Controls spacing between native interface elements.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UiDensity {
    /// Use the default egui spacing.
    #[default]
    Comfortable,
    /// Reduce spacing between controls and rows.
    Compact,
}

/// Accent family used for active/focus elements; does not change error/warn colors.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AccentScheme {
    /// Harness orange (previous default).
    #[default]
    Harness,
    Blue,
    Teal,
    Violet,
}

impl AccentScheme {
    /// Accent fill for the active light/dark scheme.
    pub fn accent(self, dark: bool) -> Rgba8 {
        match self {
            Self::Harness => {
                if dark {
                    Rgba8::rgb(0xe9, 0x56, 0x18)
                } else {
                    Rgba8::rgb(0xe9, 0x56, 0x18)
                }
            }
            Self::Blue => Rgba8::rgb(0x2e, 0x6b, 0xd8),
            Self::Teal => Rgba8::rgb(0x0e, 0x9a, 0x7a),
            Self::Violet => Rgba8::rgb(0x7b, 0x5c, 0xd8),
        }
    }

    /// Accent text/links.
    pub fn accent_text(self, _dark: bool) -> Rgba8 {
        match self {
            Self::Harness => {
                if _dark {
                    Rgba8::rgb(0xff, 0x9b, 0x70)
                } else {
                    Rgba8::rgb(0xa8, 0x3b, 0x10)
                }
            }
            Self::Blue => Rgba8::rgb(0x2e, 0x6b, 0xd8),
            Self::Teal => Rgba8::rgb(0x0e, 0x9a, 0x7a),
            Self::Violet => Rgba8::rgb(0x7b, 0x5c, 0xd8),
        }
    }
}

/// One keymap preset name.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum KeymapPreset {
    #[default]
    Dsh,
    Zed,
    Vscode,
}

/// Named color pack layered over the chosen light or dark theme.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ThemePack {
    /// The native Harness palette used by the supplied desktop design.
    #[default]
    Default,
    /// Maximum text and border contrast.
    HighContrast,
    /// The Solarized base palette.
    Solarized,
    /// Colors chosen in the appearance editor.
    Custom,
}

/// One role color as written in the preferences file.
///
/// Values stay strings so the JSON editor shows the same `#RRGGBB` a user would
/// paste from a design token file; [`ThemeColor::srgb`] validates the triplet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ThemeColor(pub String);

impl ThemeColor {
    /// Formats an sRGB triplet the way the preferences file stores it.
    #[allow(dead_code)]
    pub fn from_srgb(rgb: [u8; 3]) -> Self {
        Self(format!("#{:02X}{:02X}{:02X}", rgb[0], rgb[1], rgb[2]))
    }

    /// Reads the stored `#RRGGBB`, or `None` when the value is not a triplet.
    pub fn srgb(&self) -> Option<[u8; 3]> {
        let hex = self.0.strip_prefix('#')?;
        if hex.len() != 6 || !hex.chars().all(|character| character.is_ascii_hexdigit()) {
            return None;
        }
        let channel = |range: std::ops::Range<usize>| u8::from_str_radix(&hex[range], 16).ok();
        Some([channel(0..2)?, channel(2..4)?, channel(4..6)?])
    }

    fn rgba8_or(&self, fallback: Rgba8) -> Rgba8 {
        match self.srgb() {
            Some([red, green, blue]) => Rgba8::rgb(red, green, blue),
            None => fallback,
        }
    }
}

/// The role colors of one custom palette.
///
/// The roles follow the design-system token names the shell already speaks:
/// a page background, a raised surface for controls, primary and muted text,
/// one accent, a selection color, a border, and warn/error status colors.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ThemePalette {
    /// Window and panel fill.
    pub background: ThemeColor,
    /// Control, input, and card fill above the background.
    pub surface: ThemeColor,
    /// Primary text and control labels.
    pub text: ThemeColor,
    /// Secondary text, hints, and inactive labels.
    pub muted: ThemeColor,
    /// Links, focused outlines, and emphasized values.
    pub accent: ThemeColor,
    /// Selected text background.
    pub selection: ThemeColor,
    /// Control borders and separators.
    pub border: ThemeColor,
    /// Warning text.
    pub warn: ThemeColor,
    /// Error text.
    pub error: ThemeColor,
}

impl ThemePalette {
    /// Every role paired with the name used in the preferences file.
    pub fn roles(&self) -> [(&'static str, &ThemeColor); 9] {
        [
            ("background", &self.background),
            ("surface", &self.surface),
            ("text", &self.text),
            ("muted", &self.muted),
            ("accent", &self.accent),
            ("selection", &self.selection),
            ("border", &self.border),
            ("warn", &self.warn),
            ("error", &self.error),
        ]
    }

    /// Every role as an editable field.
#[cfg(feature = "legacy-egui")]
    pub fn roles_mut(&mut self) -> [(&'static str, &mut ThemeColor); 9] {
        [
            ("background", &mut self.background),
            ("surface", &mut self.surface),
            ("text", &mut self.text),
            ("muted", &mut self.muted),
            ("accent", &mut self.accent),
            ("selection", &mut self.selection),
            ("border", &mut self.border),
            ("warn", &mut self.warn),
            ("error", &mut self.error),
        ]
    }

    fn dark_default() -> Self {
        Self {
            background: ThemeColor("#171A1F".to_owned()),
            surface: ThemeColor("#22272E".to_owned()),
            text: ThemeColor("#EEF1F4".to_owned()),
            muted: ThemeColor("#929BA6".to_owned()),
            accent: ThemeColor("#E95618".to_owned()),
            selection: ThemeColor("#3B2A23".to_owned()),
            border: ThemeColor("#353D47".to_owned()),
            warn: ThemeColor("#E7C979".to_owned()),
            error: ThemeColor("#FF9B70".to_owned()),
        }
    }

    fn light_default() -> Self {
        Self {
            background: ThemeColor("#F1F2F3".to_owned()),
            surface: ThemeColor("#FFFFFF".to_owned()),
            text: ThemeColor("#20201F".to_owned()),
            muted: ThemeColor("#70706B".to_owned()),
            accent: ThemeColor("#E95618".to_owned()),
            selection: ThemeColor("#FFF1E9".to_owned()),
            border: ThemeColor("#E6E6E3".to_owned()),
            warn: ThemeColor("#795B1A".to_owned()),
            error: ThemeColor("#9C3916".to_owned()),
        }
    }
}

/// Custom palettes for both egui color schemes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CustomTheme {
    /// Colors used while the dark scheme is active.
    pub dark: ThemePalette,
    /// Colors used while the light scheme is active.
    pub light: ThemePalette,
}

impl Default for CustomTheme {
    fn default() -> Self {
        Self {
            dark: ThemePalette::dark_default(),
            light: ThemePalette::light_default(),
        }
    }
}

impl CustomTheme {
    fn for_scheme(&self, dark: bool) -> &ThemePalette {
        if dark {
            &self.dark
        } else {
            &self.light
        }
    }

    /// The palette the appearance editor mutates for the active scheme.
#[cfg(feature = "legacy-egui")]
    pub fn for_scheme_mut(&mut self, dark: bool) -> &mut ThemePalette {
        if dark {
            &mut self.dark
        } else {
            &mut self.light
        }
    }
}

/// Font families chosen for the interface and for code.
///
/// A family name matches an installed face; an empty name keeps the face egui
/// bundles, which is what every preference file written before this field had.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FontSettings {
    /// Family used for interface text.
    #[serde(default = "default_interface_font")]
    pub proportional: String,
    /// Family used for code, transcripts, and terminals.
    #[serde(default = "default_code_font")]
    pub monospace: String,
}

impl Default for FontSettings {
    fn default() -> Self {
        Self {
            proportional: default_interface_font(),
            monospace: default_code_font(),
        }
    }
}

/// Client-local shortcuts, written as `Ctrl+Shift+S` or `none`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Keybindings {
    /// Toggles the Settings view.
    #[serde(default = "default_open_settings")]
    pub open_settings: String,
    /// Toggles the Marketplace view.
    #[serde(default = "default_open_marketplace")]
    pub open_marketplace: String,
    /// Toggles the Goals view.
    #[serde(default = "default_open_goals")]
    pub open_goals: String,
    /// Creates a Session and focuses the composer.
    #[serde(default = "default_new_session")]
    pub new_session: String,
}

impl Default for Keybindings {
    fn default() -> Self {
        Self {
            open_settings: default_open_settings(),
            open_marketplace: default_open_marketplace(),
            open_goals: default_open_goals(),
            new_session: default_new_session(),
        }
    }
}

impl Keybindings {
    /// Chord text for one command.
    ///
    /// @param command - Command whose shortcut is requested.
    /// @returns The configured chord where the editor owns it, otherwise the
    /// registry default, or `None` when the command has no shortcut.
    pub fn chord_for(&self, command: CommandId) -> Option<String> {
        let configured = match command {
            CommandId::ToggleSettings => Some(&self.open_settings),
            CommandId::ToggleMarketplace => Some(&self.open_marketplace),
            CommandId::ToggleGoals => Some(&self.open_goals),
            CommandId::NewSession => Some(&self.new_session),
            _ => None,
        };
        match configured {
            Some(text) => Some(text.clone()),
            None => command.default_chord().map(str::to_owned),
        }
    }

    /// Consumes the chord bound to one shared command, if any.
    ///
    /// @param command - The shared command to look up.
    /// @param input - This frame's input state.
    /// @returns Whether the bound chord fired.
    #[cfg(feature = "legacy-egui")]
    pub fn consume_command(
        &mut self,
        command: CommandId,
        input: &mut egui::InputState,
    ) -> bool {
        let chord = match command {
            CommandId::ToggleSettings => KeyChord::parse(&self.open_settings).ok().flatten(),
            CommandId::ToggleMarketplace => {
                KeyChord::parse(&self.open_marketplace).ok().flatten()
            }
            CommandId::ToggleGoals => KeyChord::parse(&self.open_goals).ok().flatten(),
            CommandId::NewSession => KeyChord::parse(&self.new_session).ok().flatten(),
            _ => None,
        };
        chord.is_some_and(|chord| chord.consume(input))
    }

    /// Applies one preset to the four overridable shortcuts.
    pub fn apply_preset(&mut self, preset: KeymapPreset) {
        match preset {
            KeymapPreset::Dsh => {
                self.open_settings = default_open_settings();
                self.open_marketplace = default_open_marketplace();
                self.open_goals = default_open_goals();
                self.new_session = default_new_session();
            }
            KeymapPreset::Zed => {
                self.open_settings = "Ctrl+Comma".to_owned();
                self.open_marketplace = "Ctrl+Shift+M".to_owned();
                self.open_goals = "Ctrl+Shift+G".to_owned();
                self.new_session = "Ctrl+N".to_owned();
            }
            KeymapPreset::Vscode => {
                self.open_settings = "Ctrl+Comma".to_owned();
                self.open_marketplace = "Ctrl+Shift+X".to_owned();
                self.open_goals = "Ctrl+Shift+G".to_owned();
                self.new_session = "Ctrl+N".to_owned();
            }
        }
    }
}

/// Looks a key up in egui's name table, accepting lower-case names.
#[cfg(feature = "legacy-egui")]
fn key_by_name(name: &str) -> Option<egui::Key> {
    egui::Key::from_name(name).or_else(|| {
        let mut characters = name.chars();
        let capitalized = characters
            .next()
            .map(|first| first.to_ascii_uppercase().to_string() + characters.as_str())?;
        egui::Key::from_name(&capitalized)
    })
}

/// One parsed shortcut chord, independent of the renderer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyChord {
    /// Whether Ctrl must be held.
    pub ctrl: bool,
    /// Whether Alt must be held.
    pub alt: bool,
    /// Whether Shift must be held.
    pub shift: bool,
    /// Whether the platform command key (Super or Command) must be held.
    pub cmd: bool,
    /// Canonical name of the pressed key, matching [`KeyChord::parse`] input.
    pub key: &'static str,
}

impl KeyChord {
    /// Parses `Ctrl+Shift+S`-style text; an empty value or `none` disables the chord.
    ///
    /// @param text - Chord as written in the preferences editor.
    /// @returns The parsed chord, `None` when disabled, or a validation error.
    pub fn parse(text: &str) -> Result<Option<Self>, UiPreferencesError> {
        let trimmed = text.trim();
        if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("none") {
            return Ok(None);
        }
        let (mut ctrl, mut alt, mut shift, mut cmd) = (false, false, false, false);
        let mut key = None;
        for part in trimmed.split('+') {
            let name = part.trim();
            match name.to_ascii_lowercase().as_str() {
                "ctrl" | "control" if key.is_none() => ctrl = true,
                "alt" | "option" if key.is_none() => alt = true,
                "shift" if key.is_none() => shift = true,
                "cmd" | "command" | "super" | "meta" if key.is_none() => cmd = true,
                "ctrl" | "control" | "alt" | "option" | "shift" | "cmd" | "command" | "super"
                | "meta" => return Err(UiPreferencesError::InvalidValue(INVALID_CHORD)),
                "" => return Err(UiPreferencesError::InvalidValue(INVALID_CHORD)),
                other => {
                    let Some(parsed) = canonical_key(other) else {
                        return Err(UiPreferencesError::InvalidValue(INVALID_CHORD));
                    };
                    if key.replace(parsed).is_some() {
                        return Err(UiPreferencesError::InvalidValue(INVALID_CHORD));
                    }
                }
            }
        }
        let Some(key) = key else {
            return Err(UiPreferencesError::InvalidValue(INVALID_CHORD));
        };
        Ok(Some(Self {
            ctrl,
            alt,
            shift,
            cmd,
            key,
        }))
    }

    /// Consumes the chord when the frame reports it, so one press fires once.
    ///
    /// @param input - This frame's input state.
    /// @returns Whether the chord fired.
    #[cfg(feature = "legacy-egui")]
    pub fn consume(&self, input: &mut egui::InputState) -> bool {
        let Some(key) = key_by_name(self.key) else {
            return false;
        };
        let modifiers = egui::Modifiers {
            alt: self.alt,
            ctrl: self.ctrl,
            shift: self.shift,
            mac_cmd: self.cmd,
            command: self.ctrl || self.cmd,
        };
        input.consume_key(modifiers, key)
    }
}

/// Key names a chord may press, written the way the preferences editor shows them.
const NAMED_KEYS: [&str; 48] = [
    "Comma",
    "Period",
    "Slash",
    "Semicolon",
    "Quote",
    "Backslash",
    "Minus",
    "Equals",
    "Backtick",
    "Space",
    "Tab",
    "Enter",
    "Escape",
    "Backspace",
    "Delete",
    "Insert",
    "Home",
    "End",
    "PageUp",
    "PageDown",
    "ArrowUp",
    "ArrowDown",
    "ArrowLeft",
    "ArrowRight",
    "F1",
    "F2",
    "F3",
    "F4",
    "F5",
    "F6",
    "F7",
    "F8",
    "F9",
    "F10",
    "F11",
    "F12",
    "F13",
    "F14",
    "F15",
    "F16",
    "F17",
    "F18",
    "F19",
    "F20",
    "F21",
    "F22",
    "F23",
    "F24",
];

/// Resolves a case-insensitive key name to its canonical spelling.
///
/// @param name - One non-modifier chord part.
/// @returns The canonical name, or `None` when the key is unsupported.
fn canonical_key(name: &str) -> Option<&'static str> {
    let mut characters = name.chars();
    if let (Some(character), None) = (characters.next(), characters.next()) {
        let upper = character.to_ascii_uppercase();
        if upper.is_ascii_uppercase() || upper.is_ascii_digit() {
            return LETTER_KEYS
                .iter()
                .copied()
                .find(|candidate| candidate.starts_with(upper));
        }
    }
    NAMED_KEYS
        .iter()
        .copied()
        .find(|candidate| candidate.eq_ignore_ascii_case(name))
}

/// Canonical names for the single-character keys.
const LETTER_KEYS: [&str; 36] = [
    "A", "B", "C", "D", "E", "F", "G", "H", "I", "J", "K", "L", "M", "N", "O", "P", "Q", "R",
    "S", "T", "U", "V", "W", "X", "Y", "Z", "0", "1", "2", "3", "4", "5", "6", "7", "8", "9",
];

/// Persisted preferences owned by the native client, not the Host.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UiPreferences {
    /// Version of the on-disk preferences format.
    #[serde(default = "schema_version")]
    pub schema_version: u32,
    /// Theme selection for the egui application.
    #[serde(default)]
    pub theme: ThemeMode,
    /// egui zoom factor applied to the whole interface.
    #[serde(default = "default_ui_scale")]
    pub ui_scale: f32,
    /// Spacing density for controls and panels.
    #[serde(default)]
    pub density: UiDensity,
    /// Named color pack layered over the light or dark theme.
    #[serde(default)]
    pub theme_pack: ThemePack,
    /// Palette used when `themePack` is `custom`.
    #[serde(default)]
    pub custom_theme: CustomTheme,
    /// Font families chosen for the interface and for code.
    #[serde(default)]
    pub fonts: FontSettings,
    /// Corner radius of panels and controls.
    #[serde(default = "default_radius")]
    pub radius: f32,
    /// Client-local keyboard shortcuts.
    #[serde(default)]
    pub keybindings: Keybindings,
    /// Accent family for active/focus elements.
    #[serde(default)]
    pub accent_scheme: AccentScheme,
    /// Keymap preset applied to the four overridable shortcuts.
    #[serde(default)]
    pub keymap_preset: KeymapPreset,
    /// Initial width of the resizable Sessions panel.
    #[serde(default = "default_session_panel_width")]
    pub session_panel_width: f32,
    /// Whether the Session workspace Explorer opens by default.
    #[serde(default = "default_workspace_explorer_visible")]
    pub show_workspace_explorer: bool,
    /// Initial width of the resizable workspace Explorer.
    #[serde(default = "default_workspace_panel_width")]
    pub workspace_panel_width: f32,
    /// Initial width of the docked Changes column.
    #[serde(default = "default_changes_panel_width")]
    pub changes_panel_width: f32,
    /// Whether startup reopens the last Session the user had open.
    #[serde(default = "default_restore_last_session")]
    pub restore_last_session: bool,
    /// Session the user opened most recently, for the startup restore.
    #[serde(default)]
    pub last_session_id: Option<String>,
    /// Unsent composer text per Session, keyed by Session id.
    #[serde(default)]
    pub session_drafts: BTreeMap<String, String>,
}

impl Default for UiPreferences {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            theme: ThemeMode::default(),
            ui_scale: default_ui_scale(),
            density: UiDensity::default(),
            theme_pack: ThemePack::default(),
            custom_theme: CustomTheme::default(),
            fonts: FontSettings::default(),
            radius: default_radius(),
            accent_scheme: AccentScheme::default(),
            keymap_preset: KeymapPreset::default(),
            keybindings: Keybindings::default(),
            session_panel_width: default_session_panel_width(),
            show_workspace_explorer: default_workspace_explorer_visible(),
            workspace_panel_width: default_workspace_panel_width(),
            changes_panel_width: default_changes_panel_width(),
            restore_last_session: default_restore_last_session(),
            last_session_id: None,
            session_drafts: BTreeMap::new(),
        }
    }
}

impl UiPreferences {
    /// Returns the per-user file used for native appearance preferences.
    pub fn config_path() -> PathBuf {
        let home = std::env::var_os("DSH_HOME")
            .map(PathBuf::from)
            .or_else(|| dirs::home_dir().map(|path| path.join(".deepseek-harness")))
            .unwrap_or_else(|| PathBuf::from("."));
        home.join("native-ui-preferences.json")
    }

    /// Loads and validates a preferences file, using defaults only when it is absent.
    pub fn load(path: &Path) -> Result<Self, UiPreferencesError> {
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(source) => {
                return Err(UiPreferencesError::Io {
                    operation: "read",
                    path: path.to_owned(),
                    source,
                });
            }
        };
        let mut preferences: Self =
            serde_json::from_slice(&bytes).map_err(|source| UiPreferencesError::Parse {
                path: path.to_owned(),
                source,
            })?;
        // Version 1 files predate the custom palette, fonts, and radius; every new
        // field defaults, so the generation upgrade keeps the stored choices.
        if (SCHEMA_VERSION_V1..SCHEMA_VERSION).contains(&preferences.schema_version) {
            preferences.schema_version = SCHEMA_VERSION;
        }
        preferences.validate()?;
        Ok(preferences)
    }

    /// Validates the versioned fields before applying or saving preferences.
    pub fn validate(&self) -> Result<(), UiPreferencesError> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(UiPreferencesError::UnsupportedVersion {
                found: self.schema_version,
                supported: SCHEMA_VERSION,
            });
        }
        if !self.ui_scale.is_finite() || !(UI_SCALE_MIN..=UI_SCALE_MAX).contains(&self.ui_scale) {
            return Err(UiPreferencesError::InvalidValue(
                "uiScale must be from 0.8 to 1.5",
            ));
        }
        if !self.session_panel_width.is_finite()
            || !(SESSION_PANEL_WIDTH_MIN..=SESSION_PANEL_WIDTH_MAX)
                .contains(&self.session_panel_width)
        {
            return Err(UiPreferencesError::InvalidValue(
                "sessionPanelWidth must be from 220 to 400",
            ));
        }
        if !self.workspace_panel_width.is_finite()
            || !(WORKSPACE_PANEL_WIDTH_MIN..=WORKSPACE_PANEL_WIDTH_MAX)
                .contains(&self.workspace_panel_width)
        {
            return Err(UiPreferencesError::InvalidValue(
                "workspacePanelWidth must be within the Explorer panel range",
            ));
        }
        if !self.changes_panel_width.is_finite()
            || !(CHANGES_PANEL_WIDTH_MIN..=CHANGES_PANEL_WIDTH_MAX)
                .contains(&self.changes_panel_width)
        {
            return Err(UiPreferencesError::InvalidValue(
                "changesPanelWidth must be within the Changes panel range",
            ));
        }
        if !self.radius.is_finite() || !(RADIUS_MIN..=RADIUS_MAX).contains(&self.radius) {
            return Err(UiPreferencesError::InvalidValue(
                "radius must be from 0 to 12",
            ));
        }
        if self
            .last_session_id
            .as_ref()
            .is_some_and(|id| id.is_empty() || id.chars().count() > 256)
        {
            return Err(UiPreferencesError::InvalidValue(
                "lastSessionId must be a non-empty String of at most 256 characters",
            ));
        }
        if self.session_drafts.len() > SESSION_DRAFTS_MAX {
            return Err(UiPreferencesError::InvalidValue(
                "sessionDrafts must hold at most 200 Sessions",
            ));
        }
        for (session_id, draft) in &self.session_drafts {
            if session_id.is_empty() || session_id.chars().count() > 256 {
                return Err(UiPreferencesError::InvalidValue(
                    "each sessionDrafts key must be a non-empty Session id of at most 256 characters",
                ));
            }
            if draft.len() > SESSION_DRAFT_BYTES_MAX {
                return Err(UiPreferencesError::InvalidValue(
                    "each sessionDrafts value must be at most 64 KiB",
                ));
            }
        }
        for (scheme, palette) in [
            ("dark", &self.custom_theme.dark),
            ("light", &self.custom_theme.light),
        ] {
            for (role, color) in palette.roles() {
                if color.srgb().is_none() {
                    return Err(UiPreferencesError::InvalidThemeColor { scheme, role });
                }
            }
        }
        for (role, family) in [
            ("interface", &self.fonts.proportional),
            ("code", &self.fonts.monospace),
        ] {
            if family.chars().count() > 128 || family.chars().any(char::is_control) {
                return Err(UiPreferencesError::InvalidFontFamily { role });
            }
        }
        let mut enabled: Vec<KeyChord> = Vec::new();
        for command in CommandId::ALL {
            let Some(text) = self.keybindings.chord_for(command) else {
                continue;
            };
            let Some(chord) = KeyChord::parse(&text)? else {
                continue;
            };
            if enabled.contains(&chord) {
                return Err(UiPreferencesError::InvalidValue(
                    "each keyboard shortcut must use a different chord",
                ));
            }
            enabled.push(chord);
        }
        Ok(())
    }

    /// Parses a complete JSON preferences object and validates its values.
    #[allow(dead_code)]
    pub fn parse_json(text: &str) -> Result<Self, UiPreferencesError> {
        let mut preferences: Self = serde_json::from_str(text).map_err(UiPreferencesError::Json)?;
        if (SCHEMA_VERSION_V1..SCHEMA_VERSION).contains(&preferences.schema_version) {
            preferences.schema_version = SCHEMA_VERSION;
        }
        preferences.validate()?;
        Ok(preferences)
    }

    /// Serializes the current preferences as editable JSON.
    #[allow(dead_code)]
    pub fn to_pretty_json(&self) -> Result<String, UiPreferencesError> {
        serde_json::to_string_pretty(self).map_err(UiPreferencesError::Json)
    }

    /// Replaces the preferences file atomically after validating and syncing its contents.
    pub fn save(&self, path: &Path) -> Result<(), UiPreferencesError> {
        self.validate()?;
        let bytes = serde_json::to_vec_pretty(self).map_err(UiPreferencesError::Json)?;
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent).map_err(|source| UiPreferencesError::Io {
            operation: "create the settings directory for",
            path: path.to_owned(),
            source,
        })?;

        let sequence = NEXT_TEMP_FILE.fetch_add(1, Ordering::Relaxed);
        let temporary = parent.join(format!(
            ".native-ui-preferences.{}.{}.tmp",
            std::process::id(),
            sequence
        ));
        let mut temporary_file = TemporaryFile::new(temporary);
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(temporary_file.path())
            .map_err(|source| UiPreferencesError::Io {
                operation: "create a temporary settings file for",
                path: path.to_owned(),
                source,
            })?;
        file.write_all(&bytes)
            .and_then(|()| file.write_all(b"\n"))
            .map_err(|source| UiPreferencesError::Io {
                operation: "write",
                path: path.to_owned(),
                source,
            })?;
        file.sync_all().map_err(|source| UiPreferencesError::Io {
            operation: "sync",
            path: path.to_owned(),
            source,
        })?;
        drop(file);
        fs::rename(temporary_file.path(), path).map_err(|source| UiPreferencesError::Io {
            operation: "replace",
            path: path.to_owned(),
            source,
        })?;
        temporary_file.commit();
        Ok(())
    }
}

/// Applies persisted appearance choices without replacing unrelated egui style values.
#[cfg(feature = "legacy-egui")]
pub fn apply(context: &egui::Context, preferences: &UiPreferences) {
    let theme: egui::ThemePreference = preferences.theme.into();
    if context.options(|options| options.theme_preference) != theme {
        context.set_theme(theme);
    }
    if (context.zoom_factor() - preferences.ui_scale).abs() > f32::EPSILON {
        context.set_zoom_factor(preferences.ui_scale);
    }

    let (item_spacing, button_padding) = match preferences.density {
        UiDensity::Comfortable => (egui::vec2(8.0, 8.0), egui::vec2(8.0, 4.0)),
        UiDensity::Compact => (egui::vec2(5.0, 4.0), egui::vec2(6.0, 2.0)),
    };
    let radius = egui::CornerRadius::same(preferences.radius.round() as u8);
    context.all_styles_mut(|style| {
        if style.spacing.item_spacing != item_spacing {
            style.spacing.item_spacing = item_spacing;
        }
        if style.spacing.button_padding != button_padding {
            style.spacing.button_padding = button_padding;
        }
        apply_corner_radius(&mut style.visuals, radius);
        apply_theme_pack(
            &mut style.visuals,
            preferences.theme_pack,
            &preferences.custom_theme,
            preferences.accent_scheme,
        );
    });
}

/// One opaque 8-bit sRGB color, shared by both renderers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Rgba8 {
    pub(crate) r: u8,
    pub(crate) g: u8,
    pub(crate) b: u8,
    pub(crate) a: u8,
}

impl Rgba8 {
    pub(crate) const BLACK: Self = Self::rgb(0, 0, 0);
    pub(crate) const WHITE: Self = Self::rgb(255, 255, 255);
    pub(crate) const RED: Self = Self::rgb(255, 0, 0);
    pub(crate) const YELLOW: Self = Self::rgb(255, 255, 0);
    pub(crate) const GRAY: Self = Self::rgb(160, 160, 160);
    pub(crate) const BLUE: Self = Self::rgb(0, 0, 255);
    pub(crate) const LIGHT_BLUE: Self = Self::rgb(0xad, 0xd8, 0xe6);

    /// Builds an opaque color.
    pub(crate) const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    /// Converts to the egui paint color used by the retired shell.
    #[cfg(feature = "legacy-egui")]
    pub(crate) fn to_color32(self) -> Color32 {
        Color32::from_rgba_unmultiplied(self.r, self.g, self.b, self.a)
    }
}

/// One color pack resolved for the active light or dark scheme.
pub(crate) struct Palette {
    pub(crate) panel: Rgba8,
    pub(crate) extreme: Rgba8,
    pub(crate) text: Rgba8,
    pub(crate) selection: Rgba8,
    pub(crate) accent: Rgba8,
    pub(crate) accent_text: Rgba8,
    pub(crate) widget: Rgba8,
    pub(crate) stroke: Rgba8,
    pub(crate) muted: Rgba8,
    pub(crate) warn: Rgba8,
    pub(crate) error: Rgba8,
}

pub(crate) fn palette(
    pack: ThemePack,
    dark: bool,
    custom: &CustomTheme,
    accent: AccentScheme,
) -> Option<Palette> {
    match (pack, dark) {
        (ThemePack::Default, dark) => {
            let palette = if dark {
                ThemePalette::dark_default()
            } else {
                ThemePalette::light_default()
            };
            Some(Palette {
                panel: palette.background.rgba8_or(Rgba8::BLACK),
                extreme: palette.surface.rgba8_or(Rgba8::BLACK),
                text: palette.text.rgba8_or(Rgba8::WHITE),
                selection: palette.selection.rgba8_or(Rgba8::BLUE),
                accent: accent.accent(dark),
                accent_text: accent.accent_text(dark),
                widget: palette.surface.rgba8_or(Rgba8::BLACK),
                stroke: palette.border.rgba8_or(Rgba8::GRAY),
                muted: palette.muted.rgba8_or(Rgba8::GRAY),
                warn: palette.warn.rgba8_or(Rgba8::YELLOW),
                error: palette.error.rgba8_or(Rgba8::RED),
            })
        }
        (ThemePack::Custom, dark) => {
            let palette = custom.for_scheme(dark);
            let fallback = if dark {
                Rgba8::rgb(0x1b, 0x1b, 0x1b)
            } else {
                Rgba8::rgb(0xfa, 0xfa, 0xfa)
            };
            Some(Palette {
                panel: palette.background.rgba8_or(fallback),
                extreme: palette.surface.rgba8_or(fallback),
                text: palette.text.rgba8_or(Rgba8::WHITE),
                selection: palette.selection.rgba8_or(Rgba8::BLUE),
                accent: palette.accent.rgba8_or(Rgba8::LIGHT_BLUE),
                accent_text: palette.accent.rgba8_or(Rgba8::LIGHT_BLUE),
                widget: palette.surface.rgba8_or(fallback),
                stroke: palette.border.rgba8_or(Rgba8::GRAY),
                muted: palette.muted.rgba8_or(Rgba8::GRAY),
                warn: palette.warn.rgba8_or(Rgba8::YELLOW),
                error: palette.error.rgba8_or(Rgba8::RED),
            })
        }
        (ThemePack::HighContrast, true) => Some(Palette {
            panel: Rgba8::BLACK,
            extreme: Rgba8::BLACK,
            text: Rgba8::WHITE,
            selection: Rgba8::rgb(0xff, 0xf2, 0x00),
            accent: Rgba8::rgb(0x00, 0xe5, 0xff),
            accent_text: Rgba8::rgb(0x00, 0xe5, 0xff),
            widget: Rgba8::BLACK,
            stroke: Rgba8::WHITE,
            muted: Rgba8::WHITE,
            warn: Rgba8::rgb(0xff, 0xd6, 0x00),
            error: Rgba8::rgb(0xff, 0x6b, 0x6b),
        }),
        (ThemePack::HighContrast, false) => Some(Palette {
            panel: Rgba8::WHITE,
            extreme: Rgba8::WHITE,
            text: Rgba8::BLACK,
            selection: Rgba8::rgb(0x00, 0x00, 0xff),
            accent: Rgba8::rgb(0x00, 0x44, 0xaa),
            accent_text: Rgba8::rgb(0x00, 0x44, 0xaa),
            widget: Rgba8::WHITE,
            stroke: Rgba8::BLACK,
            muted: Rgba8::BLACK,
            warn: Rgba8::rgb(0x7a, 0x4b, 0x00),
            error: Rgba8::rgb(0xb3, 0x00, 0x00),
        }),
        (ThemePack::Solarized, true) => Some(Palette {
            panel: Rgba8::rgb(0x00, 0x2b, 0x36),
            extreme: Rgba8::rgb(0x07, 0x36, 0x42),
            text: Rgba8::rgb(0x83, 0x94, 0x96),
            selection: Rgba8::rgb(0x26, 0x8b, 0xd2),
            accent: Rgba8::rgb(0x2a, 0xa1, 0x98),
            accent_text: Rgba8::rgb(0x2a, 0xa1, 0x98),
            widget: Rgba8::rgb(0x07, 0x36, 0x42),
            stroke: Rgba8::rgb(0x58, 0x6e, 0x75),
            muted: Rgba8::rgb(0x93, 0xa1, 0xa1),
            warn: Rgba8::rgb(0xb5, 0x89, 0x00),
            error: Rgba8::rgb(0xdc, 0x32, 0x2f),
        }),
        (ThemePack::Solarized, false) => Some(Palette {
            panel: Rgba8::rgb(0xfd, 0xf6, 0xe3),
            extreme: Rgba8::rgb(0xee, 0xe8, 0xd5),
            text: Rgba8::rgb(0x65, 0x7b, 0x83),
            selection: Rgba8::rgb(0x26, 0x8b, 0xd2),
            accent: Rgba8::rgb(0x2a, 0xa1, 0x98),
            accent_text: Rgba8::rgb(0x2a, 0xa1, 0x98),
            widget: Rgba8::rgb(0xee, 0xe8, 0xd5),
            stroke: Rgba8::rgb(0x93, 0xa1, 0xa1),
            muted: Rgba8::rgb(0x65, 0x7b, 0x83),
            warn: Rgba8::rgb(0xa1, 0x62, 0x07),
            error: Rgba8::rgb(0xdc, 0x32, 0x2f),
        }),
    }
}

/// Returns the active accent fill for buttons that carry the primary action.
#[cfg(feature = "legacy-egui")]
pub(crate) fn accent_fill(preferences: &UiPreferences, dark: bool) -> Color32 {
    palette(preferences.theme_pack, dark, &preferences.custom_theme, preferences.accent_scheme).map_or_else(
        || Color32::from_rgb(0xe9, 0x56, 0x18),
        |palette| palette.accent.to_color32(),
    )
}

#[cfg(feature = "legacy-egui")]
fn apply_corner_radius(visuals: &mut egui::Visuals, radius: egui::CornerRadius) {
    visuals.window_corner_radius = radius;
    visuals.menu_corner_radius = radius;
    for widget in [
        &mut visuals.widgets.noninteractive,
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
        &mut visuals.widgets.open,
    ] {
        widget.corner_radius = radius;
    }
}

#[cfg(feature = "legacy-egui")]
fn apply_theme_pack(visuals: &mut egui::Visuals, pack: ThemePack, custom: &CustomTheme, accent: AccentScheme) {
    let Some(palette) = palette(pack, visuals.dark_mode, custom, accent) else {
        return;
    };
    visuals.panel_fill = palette.panel.to_color32();
    visuals.window_fill = palette.panel.to_color32();
    visuals.extreme_bg_color = palette.extreme.to_color32();
    visuals.faint_bg_color = palette.panel.to_color32();
    visuals.override_text_color = Some(palette.text.to_color32());
    visuals.hyperlink_color = palette.accent_text.to_color32();
    visuals.selection.bg_fill = palette.selection.to_color32();
    visuals.selection.stroke.color = palette.text.to_color32();
    visuals.widgets.noninteractive.bg_fill = palette.panel.to_color32();
    visuals.widgets.noninteractive.fg_stroke.color = palette.text.to_color32();
    for widget in [
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
        &mut visuals.widgets.open,
    ] {
        widget.bg_fill = palette.widget.to_color32();
        widget.fg_stroke.color = palette.text.to_color32();
        widget.bg_stroke.color = palette.stroke.to_color32();
    }
    visuals.widgets.noninteractive.bg_stroke.color = palette.stroke.to_color32();
    visuals.widgets.noninteractive.fg_stroke.color = palette.muted.to_color32();
    visuals.warn_fg_color = palette.warn.to_color32();
    visuals.error_fg_color = palette.error.to_color32();
}

/// Actions returned by the local preferences editor.
#[cfg(feature = "legacy-egui")]
pub enum UiPreferencesAction {
    /// Apply a draft for this session without writing it to disk.
    Preview(UiPreferences),
    /// Persist and apply a complete validated preferences value.
    Save(UiPreferences),
    /// Reload the on-disk preferences file.
    Reload,
    /// Persist and apply the default preferences.
    Reset,
}

#[cfg(feature = "legacy-egui")]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum UiPreferencesTab {
    #[default]
    Appearance,
    Layout,
    Shortcuts,
    Advanced,
}

/// Draft editor for client-local appearance preferences.
#[cfg(feature = "legacy-egui")]
pub struct UiPreferencesPanel {
    draft: UiPreferences,
    tab: UiPreferencesTab,
    config_text: String,
    config_dirty: bool,
    config_error: Option<String>,
    status: Option<String>,
    load_error: Option<String>,
}

#[cfg(feature = "legacy-egui")]
impl UiPreferencesPanel {
    /// Creates the editor from loaded preferences and an optional load diagnostic.
    pub fn new(preferences: UiPreferences, load_error: Option<String>) -> Self {
        let config_text = preferences
            .to_pretty_json()
            .unwrap_or_else(|_| "{}".to_owned());
        Self {
            draft: preferences,
            tab: UiPreferencesTab::Appearance,
            config_text,
            config_dirty: false,
            config_error: None,
            status: None,
            load_error,
        }
    }

    /// Draws local preferences and returns a persistence action for its owner.
    ///
    /// @param ui - Panel that hosts the editor.
    /// @param path - Preferences file shown in the JSON section.
    /// @param catalog - Installed families, or `None` while the scan runs.
    /// @returns The action the owning shell should perform, if any.
    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        path: &Path,
        catalog: Option<&crate::fonts::FontCatalog>,
    ) -> Option<UiPreferencesAction> {
        let mut action = None;
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.heading(egui::RichText::new("Settings").size(16.0));
                ui.label(
                    egui::RichText::new("Local interface preferences · Host settings are separate")
                        .size(10.0)
                        .weak(),
                );
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(egui::RichText::new("CLIENT-LOCAL").size(9.0).weak());
            });
        });
        if let Some(error) = &self.load_error {
            ui.colored_label(
                ui.visuals().error_fg_color,
                format!("Could not load UI preferences: {error}"),
            );
            ui.label("Defaults are active. The existing file was left unchanged.");
        }
        if let Some(error) = &self.config_error {
            ui.colored_label(
                ui.visuals().error_fg_color,
                format!("UI config is invalid: {error}"),
            );
        }
        if let Some(status) = &self.status {
            ui.label(status);
        }

        ui.horizontal(|ui| {
            for (tab, label) in [
                (UiPreferencesTab::Appearance, "Appearance"),
                (UiPreferencesTab::Layout, "Layout"),
                (UiPreferencesTab::Shortcuts, "Shortcuts"),
                (UiPreferencesTab::Advanced, "Advanced"),
            ] {
                ui.selectable_value(&mut self.tab, tab, label);
            }
        });
        ui.separator();

        let original_draft = self.draft.clone();
        if self.tab == UiPreferencesTab::Appearance {
            section(ui, "Appearance");
            egui::ComboBox::from_label("Theme")
                .selected_text(theme_label(self.draft.theme))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.draft.theme, ThemeMode::System, "System");
                    ui.selectable_value(&mut self.draft.theme, ThemeMode::Light, "Light");
                    ui.selectable_value(&mut self.draft.theme, ThemeMode::Dark, "Dark");
                });
            ui.add(
                egui::Slider::new(&mut self.draft.ui_scale, UI_SCALE_MIN..=UI_SCALE_MAX)
                    .step_by(0.05)
                    .text("UI scale"),
            );
            ui.horizontal(|ui| {
                ui.label("Density");
                ui.selectable_value(
                    &mut self.draft.density,
                    UiDensity::Comfortable,
                    "Comfortable",
                );
                ui.selectable_value(&mut self.draft.density, UiDensity::Compact, "Compact");
            });
            ui.add(
                egui::Slider::new(&mut self.draft.radius, RADIUS_MIN..=RADIUS_MAX)
                    .step_by(1.0)
                    .text("Corner radius"),
            );

            section(ui, "Colors");
            ui.horizontal_wrapped(|ui| {
                ui.label("Theme pack");
                for (pack, label) in [
                    (ThemePack::Default, "Harness"),
                    (ThemePack::HighContrast, "High contrast"),
                    (ThemePack::Solarized, "Solarized"),
                    (ThemePack::Custom, "Custom"),
                ] {
                    ui.selectable_value(&mut self.draft.theme_pack, pack, label);
                }
            });
            if self.draft.theme_pack == ThemePack::Custom {
                let dark = ui.visuals().dark_mode;
                ui.small(format!(
                    "Editing the {} palette. The Theme above chooses which palette renders.",
                    if dark { "dark" } else { "light" }
                ));
                let mut changed = false;
                for (role, color) in self.draft.custom_theme.for_scheme_mut(dark).roles_mut() {
                    changed |= color_row(ui, role, color);
                }
                if ui.button("Reset custom colors").clicked() {
                    let defaults = CustomTheme::default();
                    *self.draft.custom_theme.for_scheme_mut(dark) =
                        defaults.for_scheme(dark).clone();
                    changed = true;
                }
                let _ = changed;
            } else {
                ui.small(
                    "Choose Custom to edit every role color for the active light or dark theme.",
                );
            }

            section(ui, "Fonts");
            font_row(
                ui,
                "Interface font",
                &mut self.draft.fonts.proportional,
                catalog,
            );
            font_row(ui, "Code font", &mut self.draft.fonts.monospace, catalog);
            let missing: Vec<&str> = [&self.draft.fonts.proportional, &self.draft.fonts.monospace]
                .into_iter()
                .filter(|family| {
                    !family.is_empty()
                        && catalog.is_some_and(|catalog| catalog.file_for(family).is_none())
                })
                .map(String::as_str)
                .collect();
            if !missing.is_empty() {
                ui.colored_label(
                    ui.visuals().error_fg_color,
                    format!(
                    "Not installed: {}. The bundled face is used until another family is chosen.",
                    missing.join(", ")
                ),
                );
            }
            let sample = egui::RichText::new("Aa 0123 — việt · →  { code }");
            ui.label(sample);
            if catalog.is_none() {
                ui.small("Scanning installed fonts…");
            }
        }

        if self.tab == UiPreferencesTab::Shortcuts {
            ui.add_space(8.0);
            ui.separator();
            ui.collapsing("Keyboard shortcuts", |ui| {
                ui.horizontal(|ui| {
                    ui.label("Keymap preset");
                    for (label, preset) in [
                        ("DSH", KeymapPreset::Dsh),
                        ("Zed", KeymapPreset::Zed),
                        ("VS Code", KeymapPreset::Vscode),
                    ] {
                        let current = self.draft.keymap_preset == preset;
                        if ui
                            .add_enabled(!current, egui::Button::new(label).small())
                            .clicked()
                        {
                            self.draft.keymap_preset = preset;
                            self.draft.keybindings.apply_preset(preset);
                        }
                    }
                });
                for (label, value) in [
                    ("Open settings", &mut self.draft.keybindings.open_settings),
                    (
                        "Open marketplace",
                        &mut self.draft.keybindings.open_marketplace,
                    ),
                    ("Open goals", &mut self.draft.keybindings.open_goals),
                    ("New session", &mut self.draft.keybindings.new_session),
                ] {
                    ui.horizontal(|ui| {
                        ui.label(label);
                        ui.add(egui::TextEdit::singleline(value).desired_width(160.0));
                    });
                }
                ui.small("Write a chord like Ctrl+Shift+S, or none to disable it.");
            });
        }

        if self.tab == UiPreferencesTab::Layout {
            ui.add(
                egui::Slider::new(
                    &mut self.draft.session_panel_width,
                    SESSION_PANEL_WIDTH_MIN..=SESSION_PANEL_WIDTH_MAX,
                )
                .step_by(10.0)
                .text("Sessions panel default width"),
            );
            ui.small("The default width applies when the Sessions panel opens on the next launch.");
            section(ui, "Workspace Explorer");
            ui.checkbox(
                &mut self.draft.show_workspace_explorer,
                "Show the Explorer beside the conversation",
            );
            ui.add(
                egui::Slider::new(
                    &mut self.draft.workspace_panel_width,
                    WORKSPACE_PANEL_WIDTH_MIN..=WORKSPACE_PANEL_WIDTH_MAX,
                )
                .step_by(10.0)
                .text("Explorer panel default width"),
            );
            ui.small(
                "The dock opens above 1240px. Drag its divider to resize it for this session.",
            );
        }
        if self.draft != original_draft {
            self.status =
                Some("Form controls changed; JSON draft refreshed and remains unsaved.".into());
            self.config_text = self
                .draft
                .to_pretty_json()
                .unwrap_or_else(|_| "{}".to_owned());
            self.config_dirty = false;
            self.config_error = None;
        }

        if self.tab == UiPreferencesTab::Advanced {
            ui.collapsing("Advanced JSON config", |ui| {
                ui.label(path.display().to_string());
                if ui
                    .add(
                        egui::TextEdit::multiline(&mut self.config_text)
                            .code_editor()
                            .desired_rows(10)
                            .desired_width(f32::INFINITY),
                    )
                    .changed()
                {
                    self.config_dirty = true;
                }
                if ui.button("Apply JSON draft").clicked() {
                    if let Some(preferences) = self.parse_config_draft() {
                        self.status =
                            Some("Draft applied for this session; save to keep it.".into());
                        action = Some(UiPreferencesAction::Preview(preferences));
                    }
                }
            });
        }

        ui.horizontal(|ui| {
            if ui.button("Preview changes").clicked() {
                if let Some(preferences) = self.parse_config_draft() {
                    self.status =
                        Some("Preview active for this session; changes are not saved.".into());
                    action = Some(UiPreferencesAction::Preview(preferences));
                }
            }
            if ui.button("Save preferences").clicked() {
                if let Some(preferences) = self.parse_config_draft() {
                    action = Some(UiPreferencesAction::Save(preferences));
                }
            }
            if ui.button("Reset to defaults").clicked() {
                self.draft = UiPreferences::default();
                self.config_text = self
                    .draft
                    .to_pretty_json()
                    .unwrap_or_else(|_| "{}".to_owned());
                self.config_dirty = false;
                self.config_error = None;
                action = Some(UiPreferencesAction::Reset);
            }
            if ui.button("Reload file").clicked() {
                action = Some(UiPreferencesAction::Reload);
            }
        });
        action
    }

    /// Marks a successfully persisted value as the current editor draft.
    pub fn saved(&mut self, preferences: UiPreferences) {
        self.draft = preferences;
        self.config_text = self
            .draft
            .to_pretty_json()
            .unwrap_or_else(|_| "{}".to_owned());
        self.config_dirty = false;
        self.config_error = None;
        self.load_error = None;
        self.status = Some("UI preferences saved.".into());
    }

    /// Keeps the draft visible and reports a failed load or save.
    pub fn failed(&mut self, error: String) {
        self.status = Some(error);
    }

    /// Replaces the draft after reloading an on-disk value.
    pub fn reloaded(&mut self, preferences: UiPreferences) {
        self.draft = preferences;
        self.config_text = self
            .draft
            .to_pretty_json()
            .unwrap_or_else(|_| "{}".to_owned());
        self.config_dirty = false;
        self.config_error = None;
        self.load_error = None;
        self.status = Some("UI preferences reloaded.".into());
    }

    fn parse_config_draft(&mut self) -> Option<UiPreferences> {
        if !self.config_dirty {
            self.config_error = None;
            return Some(self.draft.clone());
        }
        match UiPreferences::parse_json(&self.config_text) {
            Ok(preferences) => {
                self.draft = preferences.clone();
                self.config_dirty = false;
                self.config_error = None;
                Some(preferences)
            }
            Err(error) => {
                self.config_error = Some(error.to_string());
                None
            }
        }
    }
}

/// Draws one section title with the spacing the appearance editor relies on.
#[cfg(feature = "legacy-egui")]
fn section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(10.0);
    ui.label(egui::RichText::new(title).strong());
}

/// Draws a swatch, its editable hex value, and the role name on one row.
///
/// @returns Whether the color changed this frame.
#[cfg(feature = "legacy-egui")]
fn color_row(ui: &mut egui::Ui, role: &str, color: &mut ThemeColor) -> bool {
    let mut rgb = color.srgb().unwrap_or([0, 0, 0]);
    let mut changed = false;
    ui.horizontal(|ui| {
        if ui.color_edit_button_srgb(&mut rgb).changed() {
            *color = ThemeColor::from_srgb(rgb);
            changed = true;
        }
        let text_color = if color.srgb().is_some() {
            ui.visuals().text_color()
        } else {
            ui.visuals().error_fg_color
        };
        let response = ui.add(
            egui::TextEdit::singleline(&mut color.0)
                .desired_width(88.0)
                .text_color(text_color),
        );
        changed |= response.changed();
        ui.label(role);
    });
    changed
}

/// Draws a family picker that keeps the bundled face available as the default.
#[cfg(feature = "legacy-egui")]
fn font_row(
    ui: &mut egui::Ui,
    label: &str,
    family: &mut String,
    catalog: Option<&crate::fonts::FontCatalog>,
) {
    ui.horizontal(|ui| {
        ui.label(label);
        let selected = if family.is_empty() {
            "Bundled (default)"
        } else {
            family.as_str()
        };
        egui::ComboBox::from_id_salt(label)
            .selected_text(selected)
            .width(240.0)
            .show_ui(ui, |ui| {
                ui.selectable_value(family, String::new(), "Bundled (default)");
                if let Some(catalog) = catalog {
                    for installed in catalog.families() {
                        ui.selectable_value(family, installed.name.clone(), &installed.name);
                    }
                }
            });
    });
}

#[cfg(feature = "legacy-egui")]
fn theme_label(theme: ThemeMode) -> &'static str {
    match theme {
        ThemeMode::System => "System",
        ThemeMode::Light => "Light",
        ThemeMode::Dark => "Dark",
    }
}

/// An on-disk error names the operation and retains its source error.
#[derive(Debug)]
pub enum UiPreferencesError {
    Io {
        operation: &'static str,
        path: PathBuf,
        source: io::Error,
    },
    Parse {
        path: PathBuf,
        source: serde_json::Error,
    },
    Json(serde_json::Error),
    UnsupportedVersion {
        found: u32,
        supported: u32,
    },
    InvalidValue(&'static str),
    InvalidThemeColor {
        scheme: &'static str,
        role: &'static str,
    },
    InvalidFontFamily {
        role: &'static str,
    },
}

impl fmt::Display for UiPreferencesError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io {
                operation,
                path,
                source,
            } => {
                write!(f, "could not {operation} {}: {source}", path.display())
            }
            Self::Parse { path, source } => {
                write!(f, "could not parse {}: {source}", path.display())
            }
            Self::Json(source) => write!(f, "invalid UI preferences JSON: {source}"),
            Self::UnsupportedVersion { found, supported } => {
                write!(
                    f,
                    "settings version {found} is unsupported; this app reads {supported}"
                )
            }
            Self::InvalidValue(message) => f.write_str(message),
            Self::InvalidThemeColor { scheme, role } => {
                write!(f, "customTheme.{scheme}.{role} must look like #RRGGBB")
            }
            Self::InvalidFontFamily { role } => write!(
                f,
                "the {role} font family must be the name of an installed font"
            ),
        }
    }
}

impl StdError for UiPreferencesError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Parse { source, .. } | Self::Json(source) => Some(source),
            Self::UnsupportedVersion { .. }
            | Self::InvalidValue(_)
            | Self::InvalidThemeColor { .. }
            | Self::InvalidFontFamily { .. } => None,
        }
    }
}

struct TemporaryFile {
    path: PathBuf,
    committed: bool,
}

impl TemporaryFile {
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            committed: false,
        }
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn commit(&mut self) {
        self.committed = true;
    }
}

impl Drop for TemporaryFile {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        if let Err(error) = fs::remove_file(&self.path) {
            if error.kind() != io::ErrorKind::NotFound {
                eprintln!(
                    "UI preferences temporary-file cleanup failed for {}: {error}",
                    self.path.display()
                );
            }
        }
    }
}

fn schema_version() -> u32 {
    SCHEMA_VERSION
}

fn default_ui_scale() -> f32 {
    1.0
}

fn default_radius() -> f32 {
    6.0
}

fn default_session_panel_width() -> f32 {
    crate::layout::SIDEBAR_DEFAULT
}

fn default_workspace_explorer_visible() -> bool {
    true
}

fn default_workspace_panel_width() -> f32 {
    crate::layout::EXPLORER_DEFAULT
}

fn default_changes_panel_width() -> f32 {
    crate::layout::CHANGES_DEFAULT
}

fn default_restore_last_session() -> bool {
    true
}

fn default_interface_font() -> String {
    "Noto Sans".to_owned()
}

fn default_code_font() -> String {
    "Noto Sans Mono".to_owned()
}

fn default_open_settings() -> String {
    "Ctrl+Comma".to_owned()
}

fn default_open_marketplace() -> String {
    "Ctrl+Shift+M".to_owned()
}

fn default_open_goals() -> String {
    "Ctrl+Shift+G".to_owned()
}

fn default_new_session() -> String {
    "Ctrl+N".to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU64;

    static NEXT_TEST_DIR: AtomicU64 = AtomicU64::new(0);

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            for _ in 0..64 {
                let sequence = NEXT_TEST_DIR.fetch_add(1, Ordering::Relaxed);
                let path = std::env::temp_dir().join(format!(
                    "dsh-ui-preferences-{}-{sequence}",
                    std::process::id()
                ));
                match fs::create_dir(&path) {
                    Ok(()) => return Self(path),
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                    Err(error) => panic!("unique test directory should be created: {error}"),
                }
            }
            panic!("could not allocate an isolated preferences test directory");
        }

        fn file(&self) -> PathBuf {
            self.0.join("native-ui-preferences.json")
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            if let Err(error) = fs::remove_dir_all(&self.0) {
                if error.kind() != io::ErrorKind::NotFound {
                    eprintln!(
                        "test preferences directory cleanup failed for {}: {error}",
                        self.0.display()
                    );
                }
            }
        }
    }

    #[test]
    fn missing_file_uses_system_theme_and_default_layout() {
        let directory = TestDirectory::new();
        assert_eq!(
            UiPreferences::load(&directory.file()).unwrap(),
            UiPreferences::default()
        );
    }

    #[test]
    fn save_and_load_round_trip_preferences() {
        let directory = TestDirectory::new();
        let path = directory.file();
        let mut preferences = UiPreferences {
            theme: ThemeMode::Dark,
            ui_scale: 1.2,
            density: UiDensity::Compact,
            session_panel_width: 320.0,
            show_workspace_explorer: false,
            workspace_panel_width: 440.0,
            restore_last_session: false,
            last_session_id: Some("session-42".to_owned()),
            ..UiPreferences::default()
        };
        preferences
            .session_drafts
            .insert("session-42".to_owned(), "half-written prompt".to_owned());
        preferences.save(&path).unwrap();
        assert_eq!(UiPreferences::load(&path).unwrap(), preferences);
        preferences.save(&path).unwrap();
        assert_eq!(UiPreferences::load(&path).unwrap(), preferences);
    }

    #[test]
    fn rejects_unsupported_versions_invalid_values_and_unknown_fields() {
        for text in [
            r#"{"schemaVersion":5}"#,
            r#"{"uiScale":3.0}"#,
            r#"{"sessionPanelWidth":500.0}"#,
            r#"{"workspacePanelWidth":600.0}"#,
            r#"{"theme":"sepia"}"#,
            r#"{"unknown":true}"#,
            r#"{"lastSessionId":""}"#,
        ] {
            assert!(UiPreferences::parse_json(text).is_err(), "{text}");
        }
    }

    #[test]
    fn rejects_oversized_session_drafts() {
        let mut preferences = UiPreferences::default();
        preferences.session_drafts.insert(
            "s1".to_owned(),
            "x".repeat(SESSION_DRAFT_BYTES_MAX + 1),
        );
        assert!(preferences.validate().is_err());

        let mut preferences = UiPreferences::default();
        for index in 0..=SESSION_DRAFTS_MAX {
            preferences
                .session_drafts
                .insert(format!("s{index}"), "draft".to_owned());
        }
        assert!(preferences.validate().is_err());
    }

    #[test]
    fn reads_a_version_three_file_and_defaults_session_restore_state() {
        let directory = TestDirectory::new();
        let path = directory.file();
        fs::write(
            &path,
            r#"{"schemaVersion":3,"theme":"dark","sessionPanelWidth":320.0}"#,
        )
        .unwrap();
        let loaded = UiPreferences::load(&path).unwrap();
        assert_eq!(loaded.schema_version, SCHEMA_VERSION);
        assert_eq!(loaded.theme, ThemeMode::Dark);
        assert!(loaded.restore_last_session);
        assert_eq!(loaded.last_session_id, None);
        assert!(loaded.session_drafts.is_empty());
    }

    #[test]
    fn reads_a_version_one_file_and_keeps_its_choices() {
        let directory = TestDirectory::new();
        let path = directory.file();
        fs::write(
            &path,
            r#"{"schemaVersion":1,"theme":"dark","uiScale":1.1,"density":"compact",
                "themePack":"solarized","sessionPanelWidth":300.0,
                "keybindings":{"openSettings":"Ctrl+Shift+S"}}"#,
        )
        .unwrap();
        let loaded = UiPreferences::load(&path).unwrap();
        assert_eq!(loaded.schema_version, SCHEMA_VERSION);
        assert_eq!(loaded.theme, ThemeMode::Dark);
        assert_eq!(loaded.theme_pack, ThemePack::Solarized);
        assert_eq!(loaded.session_panel_width, 300.0);
        assert!(loaded.show_workspace_explorer);
        assert_eq!(
            loaded.workspace_panel_width,
            default_workspace_panel_width()
        );
        assert_eq!(loaded.fonts, FontSettings::default());
        assert_eq!(loaded.custom_theme, CustomTheme::default());
    }

    #[test]
    fn reads_a_version_two_file_and_defaults_workspace_explorer_preferences() {
        let directory = TestDirectory::new();
        let path = directory.file();
        fs::write(
            &path,
            r#"{"schemaVersion":2,"theme":"dark","sessionPanelWidth":320.0}"#,
        )
        .unwrap();
        let loaded = UiPreferences::load(&path).unwrap();
        assert_eq!(loaded.schema_version, SCHEMA_VERSION);
        assert_eq!(loaded.theme, ThemeMode::Dark);
        assert_eq!(loaded.session_panel_width, 320.0);
        assert!(loaded.show_workspace_explorer);
        assert_eq!(
            loaded.workspace_panel_width,
            default_workspace_panel_width()
        );
    }

    #[test]
    #[cfg(feature = "legacy-egui")]
    fn custom_pack_paints_the_configured_roles_and_radius() {
        let context = egui::Context::default();
        let mut preferences = UiPreferences {
            theme_pack: ThemePack::Custom,
            theme: ThemeMode::Dark,
            radius: 2.0,
            ..UiPreferences::default()
        };
        preferences.custom_theme.dark.background = ThemeColor("#101820".to_owned());
        preferences.custom_theme.dark.text = ThemeColor("#F0EAD6".to_owned());
        preferences.custom_theme.dark.border = ThemeColor("#334455".to_owned());
        preferences.custom_theme.light.background = ThemeColor("#FDFDF5".to_owned());
        apply(&context, &preferences);

        let dark = context.style_of(egui::Theme::Dark).visuals.clone();
        assert_eq!(dark.panel_fill, egui::Color32::from_rgb(0x10, 0x18, 0x20));
        assert_eq!(
            dark.override_text_color,
            Some(egui::Color32::from_rgb(0xF0, 0xEA, 0xD6))
        );
        assert_eq!(
            dark.widgets.inactive.bg_stroke.color,
            egui::Color32::from_rgb(0x33, 0x44, 0x55)
        );
        assert_eq!(
            dark.widgets.inactive.corner_radius,
            egui::CornerRadius::same(2)
        );
        let light = context.style_of(egui::Theme::Light).visuals.clone();
        assert_eq!(light.panel_fill, egui::Color32::from_rgb(0xFD, 0xFD, 0xF5));
    }

    #[test]
    fn rejects_malformed_role_colors_radius_and_font_names() {
        let mut preferences = UiPreferences::default();
        preferences.custom_theme.dark.accent = ThemeColor("blue".to_owned());
        let error = preferences.validate().unwrap_err().to_string();
        assert!(error.contains("customTheme.dark.accent"), "{error}");
        assert!(error.contains("#RRGGBB"), "{error}");

        let preferences = UiPreferences {
            radius: 40.0,
            ..UiPreferences::default()
        };
        assert!(preferences.validate().is_err());

        let mut preferences = UiPreferences::default();
        preferences.fonts.monospace = "bad\nname".to_owned();
        assert!(preferences.validate().is_err());
    }

    #[test]
    fn parses_and_formats_role_colors() {
        let color = ThemeColor::from_srgb([0x2f, 0x6f, 0xeb]);
        assert_eq!(color.0, "#2F6FEB");
        assert_eq!(color.srgb(), Some([0x2f, 0x6f, 0xeb]));
        for text in ["2F6FEB", "#2F6F", "#2F6FEBB", "#GGGGGG"] {
            assert_eq!(ThemeColor(text.to_owned()).srgb(), None, "{text}");
        }
    }

    #[test]
    fn invalid_save_keeps_existing_file_unchanged() {
        let directory = TestDirectory::new();
        let path = directory.file();
        let original = UiPreferences::default();
        original.save(&path).unwrap();
        let mut invalid = original.clone();
        invalid.ui_scale = f32::NAN;
        assert!(invalid.save(&path).is_err());
        assert_eq!(UiPreferences::load(&path).unwrap(), original);
    }

    #[test]
    fn failed_replace_keeps_target_and_removes_temporary_file() {
        let directory = TestDirectory::new();
        let target = directory.0.join("target-directory");
        fs::create_dir(&target).unwrap();

        assert!(UiPreferences::default().save(&target).is_err());
        assert!(target.is_dir());
        assert_eq!(
            fs::read_dir(&directory.0)
                .unwrap()
                .filter_map(Result::ok)
                .count(),
            1
        );
    }

    #[test]
    fn parses_chords_and_rejects_ambiguous_ones() {
        let chord = KeyChord::parse("ctrl+shift+s").unwrap().unwrap();
        assert!(chord.ctrl && chord.shift && !chord.alt && !chord.cmd);
        assert_eq!(chord.key, "S");
        assert_eq!(KeyChord::parse("none").unwrap(), None);
        assert_eq!(KeyChord::parse("").unwrap(), None);
        for text in ["Ctrl+", "Ctrl", "Ctrl+S+Shift", "Ctrl+NotAKey"] {
            assert!(KeyChord::parse(text).is_err(), "{text}");
        }
    }

    #[test]
    fn rejects_duplicate_shortcuts_and_keeps_the_defaults_distinct() {
        assert!(UiPreferences::default().validate().is_ok());
        let mut preferences = UiPreferences::default();
        preferences.keybindings.open_goals = preferences.keybindings.open_settings.clone();
        assert!(preferences.validate().is_err());
    }

    #[test]
    fn theme_pack_colors_survive_a_save_and_load_round_trip() {
        let directory = TestDirectory::new();
        let path = directory.file();
        let preferences = UiPreferences {
            theme_pack: ThemePack::Solarized,
            keybindings: Keybindings {
                open_settings: "Cmd+Comma".to_owned(),
                ..Keybindings::default()
            },
            ..UiPreferences::default()
        };
        preferences.save(&path).unwrap();
        assert_eq!(UiPreferences::load(&path).unwrap(), preferences);
    }

    #[test]
    #[cfg(feature = "legacy-egui")]
    fn theme_pack_overrides_the_egui_palette_for_both_schemes() {
        let context = egui::Context::default();
        let preferences = UiPreferences {
            theme_pack: ThemePack::HighContrast,
            theme: ThemeMode::Dark,
            ..UiPreferences::default()
        };
        apply(&context, &preferences);
        let dark = context.style_of(egui::Theme::Dark).visuals.clone();
        assert_eq!(dark.panel_fill, egui::Color32::BLACK);
        assert_eq!(dark.override_text_color, Some(egui::Color32::WHITE));
        let light = context.style_of(egui::Theme::Light).visuals.clone();
        assert_eq!(light.panel_fill, egui::Color32::WHITE);
    }

    #[test]
    fn default_theme_uses_the_harness_dark_and_light_tokens() {
        let custom = CustomTheme::default();
        let dark = palette(ThemePack::Default, true, &custom, AccentScheme::default()).expect("dark Harness palette");
        assert_eq!(dark.panel, Rgba8::rgb(0x17, 0x1a, 0x1f));
        assert_eq!(dark.extreme, Rgba8::rgb(0x22, 0x27, 0x2e));
        assert_eq!(dark.accent, Rgba8::rgb(0xe9, 0x56, 0x18));
        assert_eq!(dark.accent_text, Rgba8::rgb(0xff, 0x9b, 0x70));

        let light = palette(ThemePack::Default, false, &custom, AccentScheme::default()).expect("light Harness palette");
        assert_eq!(light.panel, Rgba8::rgb(0xf1, 0xf2, 0xf3));
        assert_eq!(light.accent, Rgba8::rgb(0xe9, 0x56, 0x18));
        assert_eq!(light.accent_text, Rgba8::rgb(0xa8, 0x3b, 0x10));
    }

    #[test]
    #[cfg(feature = "legacy-egui")]
    fn applies_theme_preference_and_density_to_both_color_schemes() {
        let context = egui::Context::default();
        let preferences = UiPreferences {
            theme: ThemeMode::Dark,
            density: UiDensity::Compact,
            ..UiPreferences::default()
        };

        apply(&context, &preferences);

        assert_eq!(
            context.options(|options| options.theme_preference),
            egui::ThemePreference::Dark
        );
        for theme in [egui::Theme::Dark, egui::Theme::Light] {
            let spacing = context.style_of(theme).spacing.clone();
            assert_eq!(spacing.item_spacing, egui::vec2(5.0, 4.0));
            assert_eq!(spacing.button_padding, egui::vec2(6.0, 2.0));
        }
    }

    #[test]
    fn parses_named_function_and_command_keys() {
        let chord = KeyChord::parse("Cmd+Comma").unwrap().unwrap();
        assert!(chord.cmd && !chord.ctrl && !chord.alt && !chord.shift);
        assert_eq!(chord.key, "Comma");

        assert_eq!(KeyChord::parse("F12").unwrap().unwrap().key, "F12");
        assert_eq!(KeyChord::parse("f5").unwrap().unwrap().key, "F5");
        assert_eq!(
            KeyChord::parse("Ctrl+PageDown").unwrap().unwrap().key,
            "PageDown"
        );
        assert!(KeyChord::parse("Ctrl+Shift").is_err());
        assert!(KeyChord::parse("Ctrl+Comma+Period").is_err());
    }
}
