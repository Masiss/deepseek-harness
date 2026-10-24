//! Client-local appearance preferences, separate from Host-owned settings.

use eframe::egui;
use egui::Color32;
use serde::{Deserialize, Serialize};
use std::error::Error as StdError;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

const SCHEMA_VERSION: u32 = 1;
const UI_SCALE_MIN: f32 = 0.8;
const UI_SCALE_MAX: f32 = 1.5;
const SESSION_PANEL_WIDTH_MIN: f32 = crate::layout::SIDEBAR_MIN;
const SESSION_PANEL_WIDTH_MAX: f32 = crate::layout::SIDEBAR_MAX;
const INVALID_CHORD: &str = "keyboard shortcuts look like Ctrl+Shift+S, or none";

static NEXT_TEMP_FILE: AtomicU64 = AtomicU64::new(0);

/// Selects an explicit theme or follows the desktop environment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeMode {
    /// Follow the system's current theme.
    System,
    /// Use the light egui theme.
    Light,
    /// Use the dark egui theme.
    Dark,
}

impl Default for ThemeMode {
    fn default() -> Self {
        Self::System
    }
}

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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UiDensity {
    /// Use the default egui spacing.
    Comfortable,
    /// Reduce spacing between controls and rows.
    Compact,
}

impl Default for UiDensity {
    fn default() -> Self {
        Self::Comfortable
    }
}

/// Named color pack layered over the chosen light or dark theme.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ThemePack {
    /// The egui theme's own palette.
    Default,
    /// Maximum text and border contrast.
    HighContrast,
    /// The Solarized base palette.
    Solarized,
}

impl Default for ThemePack {
    fn default() -> Self {
        Self::Default
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
}

impl Default for Keybindings {
    fn default() -> Self {
        Self {
            open_settings: default_open_settings(),
            open_marketplace: default_open_marketplace(),
            open_goals: default_open_goals(),
        }
    }
}

impl Keybindings {
    /// The parsed chords for this set, or `None` for a disabled entry.
    pub fn chords(&self) -> Result<[Option<KeyChord>; 3], UiPreferencesError> {
        Ok([
            KeyChord::parse(&self.open_settings)?,
            KeyChord::parse(&self.open_marketplace)?,
            KeyChord::parse(&self.open_goals)?,
        ])
    }
}

/// Looks a key up in egui's name table, accepting lower-case names.
fn key_by_name(name: &str) -> Option<egui::Key> {
    egui::Key::from_name(name).or_else(|| {
        let mut characters = name.chars();
        let capitalized = characters
            .next()
            .map(|first| first.to_ascii_uppercase().to_string() + characters.as_str())?;
        egui::Key::from_name(&capitalized)
    })
}

/// One parsed shortcut chord.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyChord {
    /// Modifiers the chord requires.
    pub modifiers: egui::Modifiers,
    /// Key the chord presses.
    pub key: egui::Key,
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
        let mut modifiers = egui::Modifiers::NONE;
        let mut key = None;
        for part in trimmed.split('+') {
            let name = part.trim();
            match name.to_ascii_lowercase().as_str() {
                "ctrl" | "control" if key.is_none() => modifiers.ctrl = true,
                "alt" | "option" if key.is_none() => modifiers.alt = true,
                "shift" if key.is_none() => modifiers.shift = true,
                "cmd" | "command" | "super" | "meta" if key.is_none() => modifiers.mac_cmd = true,
                "ctrl" | "control" | "alt" | "option" | "shift" | "cmd" | "command" | "super"
                | "meta" => return Err(UiPreferencesError::InvalidValue(INVALID_CHORD)),
                "" => return Err(UiPreferencesError::InvalidValue(INVALID_CHORD)),
                other => {
                    let Some(parsed) = key_by_name(other) else {
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
        modifiers.command = modifiers.ctrl || modifiers.mac_cmd;
        Ok(Some(Self { modifiers, key }))
    }

    /// Consumes the chord when the frame reports it, so one press fires once.
    ///
    /// @param input - This frame's input state.
    /// @returns Whether the chord fired.
    pub fn consume(&self, input: &mut egui::InputState) -> bool {
        input.consume_key(self.modifiers, self.key)
    }
}

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
    /// Client-local keyboard shortcuts.
    #[serde(default)]
    pub keybindings: Keybindings,
    /// Initial width of the resizable Sessions panel.
    #[serde(default = "default_session_panel_width")]
    pub session_panel_width: f32,
}

impl Default for UiPreferences {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            theme: ThemeMode::default(),
            ui_scale: default_ui_scale(),
            density: UiDensity::default(),
            theme_pack: ThemePack::default(),
            keybindings: Keybindings::default(),
            session_panel_width: default_session_panel_width(),
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
        let preferences: Self =
            serde_json::from_slice(&bytes).map_err(|source| UiPreferencesError::Parse {
                path: path.to_owned(),
                source,
            })?;
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
        let chords = self.keybindings.chords()?;
        let enabled: Vec<KeyChord> = chords.iter().flatten().copied().collect();
        for (index, chord) in enabled.iter().enumerate() {
            if enabled[index + 1..].contains(chord) {
                return Err(UiPreferencesError::InvalidValue(
                    "each keyboard shortcut must use a different chord",
                ));
            }
        }
        Ok(())
    }

    /// Parses a complete JSON preferences object and validates its values.
    pub fn parse_json(text: &str) -> Result<Self, UiPreferencesError> {
        let preferences: Self = serde_json::from_str(text).map_err(UiPreferencesError::Json)?;
        preferences.validate()?;
        Ok(preferences)
    }

    /// Serializes the current preferences as editable JSON.
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
    context.all_styles_mut(|style| {
        if style.spacing.item_spacing != item_spacing {
            style.spacing.item_spacing = item_spacing;
        }
        if style.spacing.button_padding != button_padding {
            style.spacing.button_padding = button_padding;
        }
        apply_theme_pack(&mut style.visuals, preferences.theme_pack);
    });
}

/// One color pack resolved for the active light or dark scheme.
struct Palette {
    panel: Color32,
    extreme: Color32,
    text: Color32,
    selection: Color32,
    accent: Color32,
    widget: Color32,
    stroke: Color32,
}

fn palette(pack: ThemePack, dark: bool) -> Option<Palette> {
    match (pack, dark) {
        (ThemePack::Default, _) => None,
        (ThemePack::HighContrast, true) => Some(Palette {
            panel: Color32::BLACK,
            extreme: Color32::BLACK,
            text: Color32::WHITE,
            selection: Color32::from_rgb(0xff, 0xf2, 0x00),
            accent: Color32::from_rgb(0x00, 0xe5, 0xff),
            widget: Color32::BLACK,
            stroke: Color32::WHITE,
        }),
        (ThemePack::HighContrast, false) => Some(Palette {
            panel: Color32::WHITE,
            extreme: Color32::WHITE,
            text: Color32::BLACK,
            selection: Color32::from_rgb(0x00, 0x00, 0xff),
            accent: Color32::from_rgb(0x00, 0x44, 0xaa),
            widget: Color32::WHITE,
            stroke: Color32::BLACK,
        }),
        (ThemePack::Solarized, true) => Some(Palette {
            panel: Color32::from_rgb(0x00, 0x2b, 0x36),
            extreme: Color32::from_rgb(0x07, 0x36, 0x42),
            text: Color32::from_rgb(0x83, 0x94, 0x96),
            selection: Color32::from_rgb(0x26, 0x8b, 0xd2),
            accent: Color32::from_rgb(0x2a, 0xa1, 0x98),
            widget: Color32::from_rgb(0x07, 0x36, 0x42),
            stroke: Color32::from_rgb(0x58, 0x6e, 0x75),
        }),
        (ThemePack::Solarized, false) => Some(Palette {
            panel: Color32::from_rgb(0xfd, 0xf6, 0xe3),
            extreme: Color32::from_rgb(0xee, 0xe8, 0xd5),
            text: Color32::from_rgb(0x65, 0x7b, 0x83),
            selection: Color32::from_rgb(0x26, 0x8b, 0xd2),
            accent: Color32::from_rgb(0x2a, 0xa1, 0x98),
            widget: Color32::from_rgb(0xee, 0xe8, 0xd5),
            stroke: Color32::from_rgb(0x93, 0xa1, 0xa1),
        }),
    }
}

fn apply_theme_pack(visuals: &mut egui::Visuals, pack: ThemePack) {
    let Some(palette) = palette(pack, visuals.dark_mode) else {
        return;
    };
    visuals.panel_fill = palette.panel;
    visuals.window_fill = palette.panel;
    visuals.extreme_bg_color = palette.extreme;
    visuals.faint_bg_color = palette.panel;
    visuals.override_text_color = Some(palette.text);
    visuals.hyperlink_color = palette.accent;
    visuals.selection.bg_fill = palette.selection;
    visuals.selection.stroke.color = palette.text;
    visuals.widgets.noninteractive.bg_fill = palette.panel;
    visuals.widgets.noninteractive.fg_stroke.color = palette.text;
    for widget in [
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
        &mut visuals.widgets.open,
    ] {
        widget.bg_fill = palette.widget;
        widget.fg_stroke.color = palette.text;
        widget.bg_stroke.color = palette.stroke;
    }
    visuals.widgets.noninteractive.bg_stroke.color = palette.stroke;
    visuals.warn_fg_color = palette.accent;
    visuals.error_fg_color = palette.selection;
}

/// Actions returned by the local preferences editor.
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

/// Draft editor for client-local appearance preferences.
pub struct UiPreferencesPanel {
    draft: UiPreferences,
    config_text: String,
    config_dirty: bool,
    config_error: Option<String>,
    status: Option<String>,
    load_error: Option<String>,
}

impl UiPreferencesPanel {
    /// Creates the editor from loaded preferences and an optional load diagnostic.
    pub fn new(preferences: UiPreferences, load_error: Option<String>) -> Self {
        let config_text = preferences
            .to_pretty_json()
            .unwrap_or_else(|_| "{}".to_owned());
        Self {
            draft: preferences,
            config_text,
            config_dirty: false,
            config_error: None,
            status: None,
            load_error,
        }
    }

    /// Draws local preferences and returns a persistence action for its owner.
    pub fn ui(&mut self, ui: &mut egui::Ui, path: &Path) -> Option<UiPreferencesAction> {
        let mut action = None;
        ui.heading("Interface preferences");
        ui.label("These settings belong to the native app. Host settings below change agent configuration.");
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

        let original_draft = self.draft.clone();
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
        ui.horizontal(|ui| {
            ui.label("Theme pack");
            for (pack, label) in [
                (ThemePack::Default, "Default"),
                (ThemePack::HighContrast, "High contrast"),
                (ThemePack::Solarized, "Solarized"),
            ] {
                ui.selectable_value(&mut self.draft.theme_pack, pack, label);
            }
        });
        ui.collapsing("Keyboard shortcuts", |ui| {
            for (label, value) in [
                ("Open settings", &mut self.draft.keybindings.open_settings),
                (
                    "Open marketplace",
                    &mut self.draft.keybindings.open_marketplace,
                ),
                ("Open goals", &mut self.draft.keybindings.open_goals),
            ] {
                ui.horizontal(|ui| {
                    ui.label(label);
                    ui.add(egui::TextEdit::singleline(value).desired_width(160.0));
                });
            }
            ui.small("Write a chord like Ctrl+Shift+S, or none to disable it.");
        });
        ui.add(
            egui::Slider::new(
                &mut self.draft.session_panel_width,
                SESSION_PANEL_WIDTH_MIN..=SESSION_PANEL_WIDTH_MAX,
            )
            .step_by(10.0)
            .text("Sessions panel default width"),
        );
        ui.small("The default width applies when the Sessions panel opens on the next launch.");
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
                    self.status = Some("Draft applied for this session; save to keep it.".into());
                    action = Some(UiPreferencesAction::Preview(preferences));
                }
            }
        });

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
        }
    }
}

impl StdError for UiPreferencesError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Parse { source, .. } | Self::Json(source) => Some(source),
            Self::UnsupportedVersion { .. } | Self::InvalidValue(_) => None,
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

fn default_session_panel_width() -> f32 {
    crate::layout::SIDEBAR_DEFAULT
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
        let preferences = UiPreferences {
            theme: ThemeMode::Dark,
            ui_scale: 1.2,
            density: UiDensity::Compact,
            session_panel_width: 320.0,
            ..UiPreferences::default()
        };
        preferences.save(&path).unwrap();
        assert_eq!(UiPreferences::load(&path).unwrap(), preferences);
        preferences.save(&path).unwrap();
        assert_eq!(UiPreferences::load(&path).unwrap(), preferences);
    }

    #[test]
    fn rejects_unsupported_versions_invalid_values_and_unknown_fields() {
        for text in [
            r#"{"schemaVersion":2}"#,
            r#"{"uiScale":3.0}"#,
            r#"{"sessionPanelWidth":500.0}"#,
            r#"{"theme":"sepia"}"#,
            r#"{"unknown":true}"#,
        ] {
            assert!(UiPreferences::parse_json(text).is_err(), "{text}");
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
        assert!(chord.modifiers.ctrl && chord.modifiers.shift);
        assert_eq!(chord.key, egui::Key::S);
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
}
