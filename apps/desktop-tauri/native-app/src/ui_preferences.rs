//! Client-local appearance preferences, separate from Host-owned settings.

use crate::command_registry::CommandId;
use crate::theme::{ThemeColor, DEFAULT_PACK_ID};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::error::Error as StdError;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

const SCHEMA_VERSION: u32 = 6;
/// First on-disk format this app upgrades in place; every field is still read.
const SCHEMA_VERSION_V1: u32 = 1;
/// Last generation that named its colour pack with the removed `themePack` enum.
const SCHEMA_VERSION_V5: u32 = 5;
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
    /// Use the light theme.
    Light,
    /// Use the dark theme.
    Dark,
}

impl ThemeMode {
    /// Resolves the persisted mode against the desktop's current appearance.
    ///
    /// @param system_dark — whether the desktop reports a dark appearance.
    /// @returns whether the dark scheme is active.
    pub fn is_dark(self, system_dark: bool) -> bool {
        match self {
            Self::System => system_dark,
            Self::Light => false,
            Self::Dark => true,
        }
    }
}

/// Controls spacing between native interface elements.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UiDensity {
    /// Use the default spacing.
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
    /// Name of the family in a pack's optional `accents` map.
    ///
    /// Harness keeps the pack's own accent, so it names no override. Packs that
    /// predate accent families define none and keep their own accent too.
    pub fn pack_accent_key(self) -> Option<&'static str> {
        match self {
            Self::Harness => None,
            Self::Blue => Some("blue"),
            Self::Teal => Some("teal"),
            Self::Violet => Some("violet"),
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

/// Custom palettes for both color schemes.
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

/// Font families chosen for the interface and for code.
///
/// A family name matches an installed face; an empty name keeps the bundled
/// face, which is what every preference file written before this field had.
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
    "A", "B", "C", "D", "E", "F", "G", "H", "I", "J", "K", "L", "M", "N", "O", "P", "Q", "R", "S",
    "T", "U", "V", "W", "X", "Y", "Z", "0", "1", "2", "3", "4", "5", "6", "7", "8", "9",
];

/// Persisted preferences owned by the native client, not the Host.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UiPreferences {
    /// Version of the on-disk preferences format.
    #[serde(default = "schema_version")]
    pub schema_version: u32,
    /// Theme selection for the application.
    #[serde(default)]
    pub theme: ThemeMode,
    /// Zoom factor applied to the whole interface.
    #[serde(default = "default_ui_scale")]
    pub ui_scale: f32,
    /// Spacing density for controls and panels.
    #[serde(default)]
    pub density: UiDensity,
    /// Stable id of the colour pack selected for the application.
    ///
    /// Unknown ids are not a load error: resolution falls back to the Harness
    /// pack and reports the reason, so a removed pack cannot break preferences.
    #[serde(default = "default_theme_id")]
    pub theme_id: String,
    /// Palette used when `themeId` is `custom`.
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
    /// Appearance overrides applied only while one Host workspace is selected.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub workspace_overrides: BTreeMap<String, WorkspacePrefs>,
}

/// Per-workspace appearance overrides; `None` inherits the global value.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WorkspacePrefs {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub theme: Option<ThemeMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub density: Option<UiDensity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_panel_width: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_workspace_explorer: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_panel_width: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub changes_panel_width: Option<f32>,
}

impl Default for UiPreferences {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            theme: ThemeMode::default(),
            ui_scale: default_ui_scale(),
            density: UiDensity::default(),
            theme_id: default_theme_id(),
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
            workspace_overrides: BTreeMap::new(),
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
        let document: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|source| UiPreferencesError::Parse {
                path: path.to_owned(),
                source,
            })?;
        Self::from_document(document)
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
        for (workspace_id, prefs) in &self.workspace_overrides {
            if workspace_id.is_empty() || workspace_id.chars().count() > 256 {
                return Err(UiPreferencesError::InvalidValue(
                    "each workspaceOverrides key must be a non-empty workspace id of at most 256 characters",
                ));
            }
            if prefs.session_panel_width.is_some_and(|width| {
                !width.is_finite()
                    || !(SESSION_PANEL_WIDTH_MIN..=SESSION_PANEL_WIDTH_MAX).contains(&width)
            }) {
                return Err(UiPreferencesError::InvalidValue(
                    "workspaceOverrides sessionPanelWidth must be from 220 to 400",
                ));
            }
            if prefs.workspace_panel_width.is_some_and(|width| {
                !width.is_finite()
                    || !(WORKSPACE_PANEL_WIDTH_MIN..=WORKSPACE_PANEL_WIDTH_MAX).contains(&width)
            }) {
                return Err(UiPreferencesError::InvalidValue(
                    "workspaceOverrides workspacePanelWidth must be within the Explorer panel range",
                ));
            }
            if prefs.changes_panel_width.is_some_and(|width| {
                !width.is_finite()
                    || !(CHANGES_PANEL_WIDTH_MIN..=CHANGES_PANEL_WIDTH_MAX).contains(&width)
            }) {
                return Err(UiPreferencesError::InvalidValue(
                    "workspaceOverrides changesPanelWidth must be within the Changes panel range",
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

    /// Resolves display preferences for one selected workspace, applying its
    /// overrides over the global values. An unset override inherits globally.
    pub fn workspace_prefs(&self, workspace: Option<&str>) -> WorkspacePrefs {
        let mut effective = WorkspacePrefs {
            theme: Some(self.theme),
            density: Some(self.density),
            session_panel_width: Some(self.session_panel_width),
            show_workspace_explorer: Some(self.show_workspace_explorer),
            workspace_panel_width: Some(self.workspace_panel_width),
            changes_panel_width: Some(self.changes_panel_width),
        };
        if let Some(workspace) = workspace {
            if let Some(override_) = self.workspace_overrides.get(workspace) {
                if override_.theme.is_some() {
                    effective.theme = override_.theme;
                }
                if override_.density.is_some() {
                    effective.density = override_.density;
                }
                if override_.session_panel_width.is_some() {
                    effective.session_panel_width = override_.session_panel_width;
                }
                if override_.show_workspace_explorer.is_some() {
                    effective.show_workspace_explorer = override_.show_workspace_explorer;
                }
                if override_.workspace_panel_width.is_some() {
                    effective.workspace_panel_width = override_.workspace_panel_width;
                }
                if override_.changes_panel_width.is_some() {
                    effective.changes_panel_width = override_.changes_panel_width;
                }
            }
        }
        effective
    }

    /// Records a panel width as an override for the selected workspace, or
    /// updates the global value when no workspace is selected.
    pub fn set_workspace_panel_width(&mut self, workspace: Option<&str>, width: f32) {
        if let Some(workspace) = workspace {
            self.workspace_overrides
                .entry(workspace.to_owned())
                .or_default()
                .workspace_panel_width = Some(width);
        } else {
            self.workspace_panel_width = width;
        }
    }

    /// Clears a workspace's Explorer width override and restores the default.
    ///
    /// A double-click on the divider is the reset gesture, so it also drops the
    /// override instead of pinning the default as an explicit workspace value.
    pub fn reset_workspace_panel_width(&mut self, workspace: Option<&str>) {
        if let Some(workspace) = workspace {
            if let Some(prefs) = self.workspace_overrides.get_mut(workspace) {
                prefs.workspace_panel_width = None;
                if prefs.theme.is_none()
                    && prefs.density.is_none()
                    && prefs.session_panel_width.is_none()
                    && prefs.show_workspace_explorer.is_none()
                    && prefs.workspace_panel_width.is_none()
                    && prefs.changes_panel_width.is_none()
                {
                    self.workspace_overrides.remove(workspace);
                }
            }
        } else {
            self.workspace_panel_width = default_workspace_panel_width();
        }
    }

    /// Clears a workspace's Changes width override and restores the default.
    pub fn reset_changes_panel_width(&mut self, workspace: Option<&str>) {
        if let Some(workspace) = workspace {
            if let Some(prefs) = self.workspace_overrides.get_mut(workspace) {
                prefs.changes_panel_width = None;
                if prefs.theme.is_none()
                    && prefs.density.is_none()
                    && prefs.session_panel_width.is_none()
                    && prefs.show_workspace_explorer.is_none()
                    && prefs.workspace_panel_width.is_none()
                    && prefs.changes_panel_width.is_none()
                {
                    self.workspace_overrides.remove(workspace);
                }
            }
        } else {
            self.changes_panel_width = default_changes_panel_width();
        }
    }

    /// Records a Changes width as an override for the selected workspace, or
    /// updates the global value when no workspace is selected.
    pub fn set_changes_panel_width(&mut self, workspace: Option<&str>, width: f32) {
        if let Some(workspace) = workspace {
            self.workspace_overrides
                .entry(workspace.to_owned())
                .or_default()
                .changes_panel_width = Some(width);
        } else {
            self.changes_panel_width = width;
        }
    }

    /// Parses a complete JSON preferences object and validates its values.
    #[allow(dead_code)]
    pub fn parse_json(text: &str) -> Result<Self, UiPreferencesError> {
        let document: serde_json::Value =
            serde_json::from_str(text).map_err(UiPreferencesError::Json)?;
        Self::from_document(document)
    }

    /// Applies the readable generation upgrade, then deserializes and validates.
    fn from_document(mut document: serde_json::Value) -> Result<Self, UiPreferencesError> {
        migrate_document(&mut document)?;
        let preferences: Self =
            serde_json::from_value(document).map_err(UiPreferencesError::Json)?;
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

/// Returns the active accent fill for buttons that carry the primary action.

/// Actions returned by the local preferences editor.

/// Draft editor for client-local appearance preferences.

/// Draws one section title with the spacing the appearance editor relies on.

/// Draws a swatch, its editable hex value, and the role name on one row.
///
/// @returns Whether the color changed this frame.

/// Draws a family picker that keeps the bundled face available as the default.

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

fn default_theme_id() -> String {
    DEFAULT_PACK_ID.to_owned()
}

/// Rewrites one parsed preferences document to the current generation.
///
/// v1–v5 named the colour pack with the removed `themePack` enum. `default`
/// becomes `harness` because that is the palette those files rendered with; the
/// other names are already stable ids. An unrecognised name is kept so theme
/// resolution reports the unknown pack and falls back, instead of the whole
/// preferences file failing to load.
fn migrate_document(document: &mut serde_json::Value) -> Result<(), UiPreferencesError> {
    let Some(object) = document.as_object_mut() else {
        return Err(UiPreferencesError::InvalidValue(
            "the preferences file must be a JSON object",
        ));
    };
    let version = object
        .get("schemaVersion")
        .and_then(serde_json::Value::as_u64);
    let legacy = version.is_none_or(|version| version <= u64::from(SCHEMA_VERSION_V5));
    if legacy {
        if let Some(pack) = object.remove("themePack") {
            if !object.contains_key("themeId") {
                let Some(name) = pack.as_str() else {
                    return Err(UiPreferencesError::InvalidValue(
                        "themePack must be a theme pack name",
                    ));
                };
                let theme_id = if name == "default" { "harness" } else { name };
                object.insert(
                    "themeId".to_owned(),
                    serde_json::Value::String(theme_id.to_owned()),
                );
            }
        }
    }
    if version.is_some_and(|version| {
        (u64::from(SCHEMA_VERSION_V1)..u64::from(SCHEMA_VERSION)).contains(&version)
    }) {
        object.insert(
            "schemaVersion".to_owned(),
            serde_json::Value::from(SCHEMA_VERSION),
        );
    }
    Ok(())
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
            r#"{"schemaVersion":7}"#,
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

    fn expect_workspace_panel_width(preferences: &UiPreferences, workspace: &str, expected: f32) {
        assert_eq!(
            preferences
                .workspace_prefs(Some(workspace))
                .workspace_panel_width
                .unwrap_or(preferences.workspace_panel_width),
            expected
        );
    }

    fn expect_changes_panel_width(preferences: &UiPreferences, workspace: &str, expected: f32) {
        assert_eq!(
            preferences
                .workspace_prefs(Some(workspace))
                .changes_panel_width
                .unwrap_or(preferences.changes_panel_width),
            expected
        );
    }

    #[test]
    fn resetting_a_panel_width_drops_the_override_and_uses_the_default() {
        let mut preferences = UiPreferences::default();
        preferences.workspace_overrides.insert(
            "w1".to_owned(),
            WorkspacePrefs {
                workspace_panel_width: Some(300.),
                changes_panel_width: Some(280.),
                ..WorkspacePrefs::default()
            },
        );
        preferences.reset_workspace_panel_width(Some("w1"));
        expect_workspace_panel_width(&preferences, "w1", default_workspace_panel_width());
        expect_changes_panel_width(&preferences, "w1", 280.);
        preferences.reset_changes_panel_width(Some("w1"));
        expect_changes_panel_width(&preferences, "w1", default_changes_panel_width());
        // The last override is gone, so the workspace key is not retained.
        assert!(!preferences.workspace_overrides.contains_key("w1"));

        preferences.workspace_panel_width = 300.;
        preferences.reset_workspace_panel_width(None);
        assert_eq!(
            preferences.workspace_panel_width,
            default_workspace_panel_width()
        );
        preferences.changes_panel_width = 280.;
        preferences.reset_changes_panel_width(None);
        assert_eq!(
            preferences.changes_panel_width,
            default_changes_panel_width()
        );
    }

    #[test]
    fn resetting_one_width_keeps_the_workspace_other_overrides() {
        let mut preferences = UiPreferences::default();
        preferences.workspace_overrides.insert(
            "w1".to_owned(),
            WorkspacePrefs {
                workspace_panel_width: Some(300.),
                theme: Some(ThemeMode::Light),
                ..WorkspacePrefs::default()
            },
        );
        preferences.reset_workspace_panel_width(Some("w1"));
        assert_eq!(
            preferences
                .workspace_overrides
                .get("w1")
                .and_then(|prefs| prefs.theme),
            Some(ThemeMode::Light)
        );
    }

    #[test]
    fn workspace_overrides_resolve_and_persist() {
        let directory = TestDirectory::new();
        let path = directory.file();
        let mut preferences = UiPreferences::default();
        preferences.workspace_overrides.insert(
            "w1".to_owned(),
            WorkspacePrefs {
                theme: Some(ThemeMode::Dark),
                density: Some(UiDensity::Compact),
                workspace_panel_width: Some(300.0),
                ..WorkspacePrefs::default()
            },
        );
        preferences.save(&path).unwrap();
        let loaded = UiPreferences::load(&path).unwrap();
        assert_eq!(loaded.schema_version, SCHEMA_VERSION);
        let effective = loaded.workspace_prefs(Some("w1"));
        assert_eq!(effective.theme, Some(ThemeMode::Dark));
        assert_eq!(effective.density, Some(UiDensity::Compact));
        assert_eq!(effective.workspace_panel_width, Some(300.0));
        // Unrelated workspaces and no selection inherit the global values.
        let other = loaded.workspace_prefs(Some("w2"));
        assert_eq!(other.theme, Some(loaded.theme));
        assert_eq!(other.density, Some(loaded.density));
        let global = loaded.workspace_prefs(None);
        assert_eq!(global.theme, Some(loaded.theme));
        // A panel drag records into the selected workspace's override.
        let mut dragged = loaded.clone();
        dragged.set_workspace_panel_width(Some("w1"), 250.0);
        assert_eq!(
            dragged.workspace_prefs(Some("w1")).workspace_panel_width,
            Some(250.0)
        );
        dragged.set_changes_panel_width(None, 420.0);
        assert_eq!(dragged.changes_panel_width, 420.0);
    }

    #[test]
    fn rejects_out_of_range_workspace_override_widths() {
        let mut preferences = UiPreferences::default();
        preferences.workspace_overrides.insert(
            "w1".to_owned(),
            WorkspacePrefs {
                workspace_panel_width: Some(600.0),
                ..WorkspacePrefs::default()
            },
        );
        assert!(preferences.validate().is_err());
    }

    #[test]
    fn rejects_oversized_session_drafts() {
        let mut preferences = UiPreferences::default();
        preferences
            .session_drafts
            .insert("s1".to_owned(), "x".repeat(SESSION_DRAFT_BYTES_MAX + 1));
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
        assert_eq!(loaded.theme_id, "solarized");
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
    fn selected_theme_id_survives_a_save_and_load_round_trip() {
        let directory = TestDirectory::new();
        let path = directory.file();
        let preferences = UiPreferences {
            theme_id: "solarized".to_owned(),
            keybindings: Keybindings {
                open_settings: "Cmd+Comma".to_owned(),
                ..Keybindings::default()
            },
            ..UiPreferences::default()
        };
        preferences.save(&path).unwrap();
        let loaded = UiPreferences::load(&path).unwrap();
        assert_eq!(loaded, preferences);
        assert_eq!(loaded.theme_id, "solarized");
    }

    #[test]
    fn missing_preferences_select_the_dsh_pack() {
        let directory = TestDirectory::new();
        let loaded = UiPreferences::load(&directory.file()).unwrap();
        assert_eq!(loaded.theme_id, DEFAULT_PACK_ID);
        assert_eq!(loaded.theme_id, "dsh");
    }

    #[test]
    fn migrates_every_v5_pack_name_to_its_stable_id() {
        for (pack, theme_id) in [
            ("default", "harness"),
            ("high-contrast", "high-contrast"),
            ("solarized", "solarized"),
            ("custom", "custom"),
        ] {
            let directory = TestDirectory::new();
            let path = directory.file();
            fs::write(
                &path,
                format!(r#"{{"schemaVersion":5,"theme":"dark","themePack":"{pack}"}}"#),
            )
            .unwrap();
            let loaded = UiPreferences::load(&path).unwrap();
            assert_eq!(loaded.schema_version, SCHEMA_VERSION);
            assert_eq!(loaded.theme_id, theme_id, "{pack}");
            assert_eq!(loaded.theme, ThemeMode::Dark);
            // The removed field must not survive the next write.
            loaded.save(&path).unwrap();
            let saved = fs::read_to_string(&path).unwrap();
            assert!(!saved.contains("themePack"), "{saved}");
        }
    }

    #[test]
    fn keeps_an_unrecognised_v5_pack_name_so_resolution_can_report_it() {
        let directory = TestDirectory::new();
        let path = directory.file();
        fs::write(&path, r#"{"schemaVersion":4,"themePack":"sepia"}"#).unwrap();
        let loaded = UiPreferences::load(&path).unwrap();
        assert_eq!(loaded.theme_id, "sepia");
    }

    #[test]
    fn rejects_a_non_string_v5_pack_name_and_ignores_theme_pack_in_a_v6_file() {
        let directory = TestDirectory::new();
        let path = directory.file();
        fs::write(&path, r#"{"schemaVersion":5,"themePack":5}"#).unwrap();
        assert!(UiPreferences::load(&path).is_err());

        fs::write(
            &path,
            r#"{"schemaVersion":6,"themeId":"dsh","themePack":"solarized"}"#,
        )
        .unwrap();
        // A v6 document keeps `deny_unknown_fields` semantics instead of migrating.
        assert!(UiPreferences::load(&path).is_err());
    }

    #[test]
    fn an_explicit_v6_theme_id_wins_over_the_legacy_field() {
        let mut document = serde_json::json!({
            "schemaVersion": 5,
            "themePack": "solarized",
            "themeId": "custom",
        });
        let preferences = UiPreferences::from_document(document.take()).unwrap();
        assert_eq!(preferences.theme_id, "custom");
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
