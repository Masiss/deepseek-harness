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

const SCHEMA_VERSION: u32 = 2;
/// First on-disk format; its fields are all still read, new ones stay defaulted.
const SCHEMA_VERSION_V1: u32 = 1;
const UI_SCALE_MIN: f32 = 0.8;
const UI_SCALE_MAX: f32 = 1.5;
const RADIUS_MIN: f32 = 0.0;
const RADIUS_MAX: f32 = 12.0;
const SESSION_PANEL_WIDTH_MIN: f32 = crate::layout::SIDEBAR_MIN;
const SESSION_PANEL_WIDTH_MAX: f32 = crate::layout::SIDEBAR_MAX;
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

/// Named color pack layered over the chosen light or dark theme.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ThemePack {
    /// The egui theme's own palette.
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

    fn color_or(&self, fallback: Color32) -> Color32 {
        match self.srgb() {
            Some([red, green, blue]) => Color32::from_rgb(red, green, blue),
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
            background: ThemeColor("#1B1B1B".to_owned()),
            surface: ThemeColor("#232323".to_owned()),
            text: ThemeColor("#E6E6E6".to_owned()),
            muted: ThemeColor("#A0A0A0".to_owned()),
            accent: ThemeColor("#2F6FEB".to_owned()),
            selection: ThemeColor("#2F6FEB".to_owned()),
            border: ThemeColor("#3A3A3A".to_owned()),
            warn: ThemeColor("#EAB308".to_owned()),
            error: ThemeColor("#DC2626".to_owned()),
        }
    }

    fn light_default() -> Self {
        Self {
            background: ThemeColor("#FAFAFA".to_owned()),
            surface: ThemeColor("#FFFFFF".to_owned()),
            text: ThemeColor("#111111".to_owned()),
            muted: ThemeColor("#6B6B6B".to_owned()),
            accent: ThemeColor("#2F6FEB".to_owned()),
            selection: ThemeColor("#2F6FEB".to_owned()),
            border: ThemeColor("#E5E5E5".to_owned()),
            warn: ThemeColor("#A16207".to_owned()),
            error: ThemeColor("#DC2626".to_owned()),
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
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FontSettings {
    /// Family used for interface text.
    #[serde(default)]
    pub proportional: String,
    /// Family used for code, transcripts, and terminals.
    #[serde(default)]
    pub monospace: String,
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
    /// The parsed chords for this set, or `None` for a disabled entry.
    pub fn chords(&self) -> Result<[Option<KeyChord>; 4], UiPreferencesError> {
        Ok([
            KeyChord::parse(&self.open_settings)?,
            KeyChord::parse(&self.open_marketplace)?,
            KeyChord::parse(&self.open_goals)?,
            KeyChord::parse(&self.new_session)?,
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
            custom_theme: CustomTheme::default(),
            fonts: FontSettings::default(),
            radius: default_radius(),
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
        let mut preferences: Self =
            serde_json::from_slice(&bytes).map_err(|source| UiPreferencesError::Parse {
                path: path.to_owned(),
                source,
            })?;
        // Version 1 files predate the custom palette, fonts, and radius; every new
        // field defaults, so the generation upgrade keeps the stored choices.
        if preferences.schema_version == SCHEMA_VERSION_V1 {
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
        if !self.radius.is_finite() || !(RADIUS_MIN..=RADIUS_MAX).contains(&self.radius) {
            return Err(UiPreferencesError::InvalidValue(
                "radius must be from 0 to 12",
            ));
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
        let mut preferences: Self = serde_json::from_str(text).map_err(UiPreferencesError::Json)?;
        if preferences.schema_version == SCHEMA_VERSION_V1 {
            preferences.schema_version = SCHEMA_VERSION;
        }
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
        );
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
    muted: Color32,
    warn: Color32,
    error: Color32,
}

fn palette(pack: ThemePack, dark: bool, custom: &CustomTheme) -> Option<Palette> {
    match (pack, dark) {
        (ThemePack::Default, _) => None,
        (ThemePack::Custom, dark) => {
            let palette = custom.for_scheme(dark);
            let fallback = if dark {
                Color32::from_rgb(0x1b, 0x1b, 0x1b)
            } else {
                Color32::from_rgb(0xfa, 0xfa, 0xfa)
            };
            Some(Palette {
                panel: palette.background.color_or(fallback),
                extreme: palette.surface.color_or(fallback),
                text: palette.text.color_or(Color32::WHITE),
                selection: palette.selection.color_or(Color32::BLUE),
                accent: palette.accent.color_or(Color32::LIGHT_BLUE),
                widget: palette.surface.color_or(fallback),
                stroke: palette.border.color_or(Color32::GRAY),
                muted: palette.muted.color_or(Color32::GRAY),
                warn: palette.warn.color_or(Color32::YELLOW),
                error: palette.error.color_or(Color32::RED),
            })
        }
        (ThemePack::HighContrast, true) => Some(Palette {
            panel: Color32::BLACK,
            extreme: Color32::BLACK,
            text: Color32::WHITE,
            selection: Color32::from_rgb(0xff, 0xf2, 0x00),
            accent: Color32::from_rgb(0x00, 0xe5, 0xff),
            widget: Color32::BLACK,
            stroke: Color32::WHITE,
            muted: Color32::WHITE,
            warn: Color32::from_rgb(0xff, 0xd6, 0x00),
            error: Color32::from_rgb(0xff, 0x6b, 0x6b),
        }),
        (ThemePack::HighContrast, false) => Some(Palette {
            panel: Color32::WHITE,
            extreme: Color32::WHITE,
            text: Color32::BLACK,
            selection: Color32::from_rgb(0x00, 0x00, 0xff),
            accent: Color32::from_rgb(0x00, 0x44, 0xaa),
            widget: Color32::WHITE,
            stroke: Color32::BLACK,
            muted: Color32::BLACK,
            warn: Color32::from_rgb(0x7a, 0x4b, 0x00),
            error: Color32::from_rgb(0xb3, 0x00, 0x00),
        }),
        (ThemePack::Solarized, true) => Some(Palette {
            panel: Color32::from_rgb(0x00, 0x2b, 0x36),
            extreme: Color32::from_rgb(0x07, 0x36, 0x42),
            text: Color32::from_rgb(0x83, 0x94, 0x96),
            selection: Color32::from_rgb(0x26, 0x8b, 0xd2),
            accent: Color32::from_rgb(0x2a, 0xa1, 0x98),
            widget: Color32::from_rgb(0x07, 0x36, 0x42),
            stroke: Color32::from_rgb(0x58, 0x6e, 0x75),
            muted: Color32::from_rgb(0x93, 0xa1, 0xa1),
            warn: Color32::from_rgb(0xb5, 0x89, 0x00),
            error: Color32::from_rgb(0xdc, 0x32, 0x2f),
        }),
        (ThemePack::Solarized, false) => Some(Palette {
            panel: Color32::from_rgb(0xfd, 0xf6, 0xe3),
            extreme: Color32::from_rgb(0xee, 0xe8, 0xd5),
            text: Color32::from_rgb(0x65, 0x7b, 0x83),
            selection: Color32::from_rgb(0x26, 0x8b, 0xd2),
            accent: Color32::from_rgb(0x2a, 0xa1, 0x98),
            widget: Color32::from_rgb(0xee, 0xe8, 0xd5),
            stroke: Color32::from_rgb(0x93, 0xa1, 0xa1),
            muted: Color32::from_rgb(0x65, 0x7b, 0x83),
            warn: Color32::from_rgb(0xa1, 0x62, 0x07),
            error: Color32::from_rgb(0xdc, 0x32, 0x2f),
        }),
    }
}

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

fn apply_theme_pack(visuals: &mut egui::Visuals, pack: ThemePack, custom: &CustomTheme) {
    let Some(palette) = palette(pack, visuals.dark_mode, custom) else {
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
    visuals.widgets.noninteractive.fg_stroke.color = palette.muted;
    visuals.warn_fg_color = palette.warn;
    visuals.error_fg_color = palette.error;
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
                (ThemePack::Default, "Default"),
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
                *self.draft.custom_theme.for_scheme_mut(dark) = defaults.for_scheme(dark).clone();
                changed = true;
            }
            let _ = changed;
        } else {
            ui.small("Choose Custom to edit every role color for the active light or dark theme.");
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

        ui.add_space(8.0);
        ui.separator();
        ui.collapsing("Keyboard shortcuts", |ui| {
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

/// Draws one section title with the spacing the appearance editor relies on.
fn section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(10.0);
    ui.label(egui::RichText::new(title).strong());
}

/// Draws a swatch, its editable hex value, and the role name on one row.
///
/// @returns Whether the color changed this frame.
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
            r#"{"schemaVersion":3}"#,
            r#"{"uiScale":3.0}"#,
            r#"{"sessionPanelWidth":500.0}"#,
            r#"{"theme":"sepia"}"#,
            r#"{"unknown":true}"#,
        ] {
            assert!(UiPreferences::parse_json(text).is_err(), "{text}");
        }
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
        assert_eq!(loaded.fonts, FontSettings::default());
        assert_eq!(loaded.custom_theme, CustomTheme::default());
    }

    #[test]
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
