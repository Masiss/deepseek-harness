//! Theme pack documents: the semantic colour schema shared by every pack.
//!
//! A pack carries colours only. Metrics, fonts, and layout stay in the app, so a
//! pack can never change hit targets or shell heights. Both schemes are required;
//! a document that omits one fails to parse instead of reusing the other side.

use std::collections::BTreeMap;
use std::error::Error as StdError;
use std::fmt;

use serde::{Deserialize, Serialize};

use super::tokens::{Rgba8, Role, Scheme};

/// Pack document version this app reads.
pub(crate) const SCHEMA_VERSION: u32 = 1;

/// Longest accepted identity field (`name`, `author`, `license`, `source`).
const IDENTITY_MAX_CHARS: usize = 256;

/// One colour as written in a pack or preferences document.
///
/// Values stay strings so an installed pack and the preferences file keep the
/// `#RRGGBB` spelling a user pasted from a design token file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ThemeColor(pub String);

impl ThemeColor {
    /// Reads `#RRGGBB` or `#RRGGBBAA`; `None` when the value is not a hex colour.
    pub(crate) fn rgba(&self) -> Option<Rgba8> {
        let hex = self.0.strip_prefix('#')?;
        let channel = |range: std::ops::Range<usize>| u8::from_str_radix(&hex[range], 16).ok();
        if hex.len() != 6 && hex.len() != 8 {
            return None;
        }
        if !hex.chars().all(|character| character.is_ascii_hexdigit()) {
            return None;
        }
        Some(Rgba8 {
            r: channel(0..2)?,
            g: channel(2..4)?,
            b: channel(4..6)?,
            a: if hex.len() == 8 { channel(6..8)? } else { 255 },
        })
    }

    /// Reads the opaque `#RRGGBB` the editable custom palette requires.
    pub(crate) fn srgb(&self) -> Option<[u8; 3]> {
        let hex = self.0.strip_prefix('#')?;
        if hex.len() != 6 || !hex.chars().all(|character| character.is_ascii_hexdigit()) {
            return None;
        }
        let channel = |range: std::ops::Range<usize>| u8::from_str_radix(&hex[range], 16).ok();
        Some([channel(0..2)?, channel(2..4)?, channel(4..6)?])
    }
}

/// One theme pack document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ThemePack {
    /// Document format version; [`SCHEMA_VERSION`] is the only accepted value.
    pub(crate) schema_version: u32,
    /// Stable id used by preferences and theme file names.
    pub(crate) id: String,
    /// Display name shown where a pack is listed.
    pub(crate) name: String,
    /// Pack author; an empty string means the pack does not name one.
    pub(crate) author: String,
    /// Licence text; an empty string means the pack does not name one.
    pub(crate) license: String,
    /// Origin recorded by the pack document itself.
    pub(crate) source: String,
    /// Colour roles for both schemes.
    pub(crate) colors: PackColors,
    /// Optional accent families layered over the pack's own accent and selection.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(crate) accents: BTreeMap<String, PackAccent>,
}

impl ThemePack {
    /// Parses and validates one pack document.
    ///
    /// @param json — the complete pack document.
    /// @returns the validated pack, or the first defect found.
    pub(crate) fn parse(json: &str) -> Result<Self, PackError> {
        let pack: Self = serde_json::from_str(json).map_err(PackError::Parse)?;
        pack.validate()?;
        Ok(pack)
    }

    /// Validates the version, identity, and every colour value.
    pub(crate) fn validate(&self) -> Result<(), PackError> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(PackError::UnsupportedVersion {
                found: self.schema_version,
                supported: SCHEMA_VERSION,
            });
        }
        if !valid_id(&self.id) {
            return Err(PackError::InvalidId(self.id.clone()));
        }
        for (pointer, value) in [
            ("id", &self.id),
            ("name", &self.name),
            ("author", &self.author),
            ("license", &self.license),
            ("source", &self.source),
        ] {
            if value.chars().count() > IDENTITY_MAX_CHARS || value.chars().any(char::is_control) {
                return Err(PackError::InvalidField { pointer });
            }
        }
        if self.name.is_empty() {
            return Err(PackError::MissingField { pointer: "name" });
        }
        if self.source.is_empty() {
            return Err(PackError::MissingField { pointer: "source" });
        }
        for scheme in [Scheme::Dark, Scheme::Light] {
            self.colors.for_scheme(scheme).parse(scheme)?;
        }
        for (name, accent) in &self.accents {
            if !valid_accent_name(name) {
                return Err(PackError::InvalidAccentName(name.clone()));
            }
            for scheme in [Scheme::Dark, Scheme::Light] {
                accent.for_scheme(scheme).parse(scheme, name)?;
            }
        }
        Ok(())
    }

    /// Parses both schemes plus the selected accent family for one scheme.
    ///
    /// @param scheme — the active light or dark scheme.
    /// @param accent — the name of an entry in `accents`; an absent entry keeps
    ///   the pack's own `accent`, `accentText`, and `selection`.
    /// @returns the resolved role table, or the failing pack path.
    pub(crate) fn colors_for(
        &self,
        scheme: Scheme,
        accent: Option<&str>,
    ) -> Result<SchemeColors, PackError> {
        let mut colors = self.colors.for_scheme(scheme).parse(scheme)?;
        if let Some(accent) = accent {
            if let Some(overrides) = self.accents.get(accent) {
                let overrides = overrides.for_scheme(scheme);
                colors.set(
                    Role::Accent,
                    overrides
                        .accent
                        .rgba()
                        .ok_or_else(|| PackError::InvalidColor {
                            pointer: format!("accents.{accent}.{}.accent", scheme.name()),
                        })?,
                );
                colors.set(
                    Role::AccentText,
                    overrides
                        .accent_text
                        .rgba()
                        .ok_or_else(|| PackError::InvalidColor {
                            pointer: format!("accents.{accent}.{}.accentText", scheme.name()),
                        })?,
                );
                colors.set(
                    Role::Selection,
                    overrides
                        .selection
                        .rgba()
                        .ok_or_else(|| PackError::InvalidColor {
                            pointer: format!("accents.{accent}.{}.selection", scheme.name()),
                        })?,
                );
            }
        }
        Ok(colors)
    }

    /// Whether the pack defines accent families at all.
    ///
    /// A pack without any entry keeps its own accent and ignores the accent
    /// preference, so the shell disables the accent controls for it.
    pub(crate) fn has_accent_families(&self) -> bool {
        !self.accents.is_empty()
    }
}

/// Accepts ids that are safe as a stable preference value and file name.
fn valid_id(id: &str) -> bool {
    let bytes = id.as_bytes();
    let Some(first) = bytes.first() else {
        return false;
    };
    if !first.is_ascii_lowercase() && !first.is_ascii_digit() {
        return false;
    }
    bytes.len() <= 64
        && bytes.iter().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'_' | b'-')
        })
}

fn valid_accent_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 32
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

/// Colour roles for both schemes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PackColors {
    /// Roles used while the dark scheme is active.
    pub(crate) dark: PackScheme,
    /// Roles used while the light scheme is active.
    pub(crate) light: PackScheme,
}

impl PackColors {
    fn for_scheme(&self, scheme: Scheme) -> &PackScheme {
        match scheme {
            Scheme::Dark => &self.dark,
            Scheme::Light => &self.light,
        }
    }
}

/// The sixteen colour roles of one scheme as written in a pack document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PackScheme {
    /// Primary workspace background.
    pub(crate) canvas: ThemeColor,
    /// Top and bottom shell bars.
    pub(crate) chrome: ThemeColor,
    /// Cards and dialogs.
    pub(crate) surface: ThemeColor,
    /// Side panels and code backgrounds.
    pub(crate) surface_deep: ThemeColor,
    /// Hover, composer, and popover fill.
    pub(crate) surface_raised: ThemeColor,
    /// Main separators and control outlines.
    pub(crate) border: ThemeColor,
    /// Faint separator between adjacent panel fills.
    pub(crate) border_faint: ThemeColor,
    /// Primary text.
    pub(crate) text: ThemeColor,
    /// Secondary text and hints.
    pub(crate) text_secondary: ThemeColor,
    /// Small readable metadata.
    pub(crate) text_tertiary: ThemeColor,
    /// Action fill, focus ring, and selected outline.
    pub(crate) accent: ThemeColor,
    /// Label colour on an accent fill.
    pub(crate) accent_text: ThemeColor,
    /// Selected text background.
    pub(crate) selection: ThemeColor,
    /// Completed or healthy state.
    pub(crate) success: ThemeColor,
    /// Failed or destructive state.
    pub(crate) danger: ThemeColor,
    /// Attention or caution state.
    pub(crate) warning: ThemeColor,
}

impl PackScheme {
    /// Parses every role; a failure names the role's path in the pack document.
    fn parse(&self, scheme: Scheme) -> Result<SchemeColors, PackError> {
        let color = |role: Role, value: &ThemeColor| {
            value.rgba().ok_or_else(|| PackError::InvalidColor {
                pointer: format!("colors.{}.{}", scheme.name(), role.name()),
            })
        };
        Ok(SchemeColors {
            canvas: color(Role::Canvas, &self.canvas)?,
            chrome: color(Role::Chrome, &self.chrome)?,
            surface: color(Role::Surface, &self.surface)?,
            surface_deep: color(Role::SurfaceDeep, &self.surface_deep)?,
            surface_raised: color(Role::SurfaceRaised, &self.surface_raised)?,
            border: color(Role::Border, &self.border)?,
            border_faint: color(Role::BorderFaint, &self.border_faint)?,
            text: color(Role::Text, &self.text)?,
            text_secondary: color(Role::TextSecondary, &self.text_secondary)?,
            text_tertiary: color(Role::TextTertiary, &self.text_tertiary)?,
            accent: color(Role::Accent, &self.accent)?,
            accent_text: color(Role::AccentText, &self.accent_text)?,
            selection: color(Role::Selection, &self.selection)?,
            success: color(Role::Success, &self.success)?,
            danger: color(Role::Danger, &self.danger)?,
            warning: color(Role::Warning, &self.warning)?,
        })
    }
}

/// One accent family: replaces `accent`, `accentText`, and `selection`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PackAccent {
    pub(crate) dark: PackAccentColors,
    pub(crate) light: PackAccentColors,
}

impl PackAccent {
    fn for_scheme(&self, scheme: Scheme) -> &PackAccentColors {
        match scheme {
            Scheme::Dark => &self.dark,
            Scheme::Light => &self.light,
        }
    }
}

/// The colours one accent family replaces in one scheme.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PackAccentColors {
    /// Action fill and focus ring for this accent family.
    pub(crate) accent: ThemeColor,
    /// Label colour on the accent fill.
    pub(crate) accent_text: ThemeColor,
    /// Selected text background paired with this accent.
    pub(crate) selection: ThemeColor,
}

impl PackAccentColors {
    fn parse(&self, scheme: Scheme, family: &str) -> Result<(), PackError> {
        for (role, value) in [
            (Role::Accent, &self.accent),
            (Role::AccentText, &self.accent_text),
            (Role::Selection, &self.selection),
        ] {
            if value.rgba().is_none() {
                return Err(PackError::InvalidColor {
                    pointer: format!("accents.{family}.{}.{}", scheme.name(), role.name()),
                });
            }
        }
        Ok(())
    }
}

/// Parsed colour roles for one scheme, ready to hand to the shell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SchemeColors {
    pub(crate) canvas: Rgba8,
    pub(crate) chrome: Rgba8,
    pub(crate) surface: Rgba8,
    pub(crate) surface_deep: Rgba8,
    pub(crate) surface_raised: Rgba8,
    pub(crate) border: Rgba8,
    pub(crate) border_faint: Rgba8,
    pub(crate) text: Rgba8,
    pub(crate) text_secondary: Rgba8,
    pub(crate) text_tertiary: Rgba8,
    pub(crate) accent: Rgba8,
    pub(crate) accent_text: Rgba8,
    pub(crate) selection: Rgba8,
    pub(crate) success: Rgba8,
    pub(crate) danger: Rgba8,
    pub(crate) warning: Rgba8,
}

impl SchemeColors {
    /// The colour of one semantic role.
    pub(crate) fn color(&self, role: Role) -> Rgba8 {
        match role {
            Role::Canvas => self.canvas,
            Role::Chrome => self.chrome,
            Role::Surface => self.surface,
            Role::SurfaceDeep => self.surface_deep,
            Role::SurfaceRaised => self.surface_raised,
            Role::Border => self.border,
            Role::BorderFaint => self.border_faint,
            Role::Text => self.text,
            Role::TextSecondary => self.text_secondary,
            Role::TextTertiary => self.text_tertiary,
            Role::Accent => self.accent,
            Role::AccentText => self.accent_text,
            Role::Selection => self.selection,
            Role::Success => self.success,
            Role::Danger => self.danger,
            Role::Warning => self.warning,
        }
    }

    /// Replaces one role, used when an accent family is applied.
    pub(crate) fn set(&mut self, role: Role, color: Rgba8) {
        match role {
            Role::Canvas => self.canvas = color,
            Role::Chrome => self.chrome = color,
            Role::Surface => self.surface = color,
            Role::SurfaceDeep => self.surface_deep = color,
            Role::SurfaceRaised => self.surface_raised = color,
            Role::Border => self.border = color,
            Role::BorderFaint => self.border_faint = color,
            Role::Text => self.text = color,
            Role::TextSecondary => self.text_secondary = color,
            Role::TextTertiary => self.text_tertiary = color,
            Role::Accent => self.accent = color,
            Role::AccentText => self.accent_text = color,
            Role::Selection => self.selection = color,
            Role::Success => self.success = color,
            Role::Danger => self.danger = color,
            Role::Warning => self.warning = color,
        }
    }
}

/// Why a pack document cannot be used.
#[derive(Debug)]
pub(crate) enum PackError {
    /// The document is not valid JSON or does not match the schema.
    Parse(serde_json::Error),
    /// The document was written for another pack schema generation.
    UnsupportedVersion { found: u32, supported: u32 },
    /// The id is not usable as a stable pack identifier.
    InvalidId(String),
    /// A required identity field is empty.
    MissingField { pointer: &'static str },
    /// An identity field is longer than allowed or contains control characters.
    InvalidField { pointer: &'static str },
    /// An accent family name is not usable.
    InvalidAccentName(String),
    /// A colour role is not a hex colour; `pointer` names its pack path.
    InvalidColor { pointer: String },
}

impl fmt::Display for PackError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse(source) => write!(f, "the theme pack document is invalid: {source}"),
            Self::UnsupportedVersion { found, supported } => write!(
                f,
                "theme pack schema {found} is unsupported; this app reads {supported}"
            ),
            Self::InvalidId(id) => write!(
                f,
                "theme pack id `{id}` must match ^[a-z0-9][a-z0-9._-]{{0,63}}$"
            ),
            Self::MissingField { pointer } => write!(f, "theme pack {pointer} must not be empty"),
            Self::InvalidField { pointer } => write!(
                f,
                "theme pack {pointer} must be a string of at most {IDENTITY_MAX_CHARS} characters without control characters"
            ),
            Self::InvalidAccentName(name) => write!(
                f,
                "theme pack accent `{name}` must match ^[a-z0-9-]{{1,32}}$"
            ),
            Self::InvalidColor { pointer } => {
                write!(f, "theme pack {pointer} must look like #RRGGBB or #RRGGBBAA")
            }
        }
    }
}

impl StdError for PackError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::Parse(source) => Some(source),
            Self::UnsupportedVersion { .. }
            | Self::InvalidId(_)
            | Self::MissingField { .. }
            | Self::InvalidField { .. }
            | Self::InvalidAccentName(_)
            | Self::InvalidColor { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    fn document() -> Value {
        json!({
            "schemaVersion": 1,
            "id": "sample",
            "name": "Sample",
            "author": "",
            "license": "",
            "source": "test",
            "colors": {
                "dark": {
                    "canvas": "#000000", "chrome": "#000000", "surface": "#111111",
                    "surfaceDeep": "#0b0b0b", "surfaceRaised": "#1a1a1a",
                    "border": "#333333", "borderFaint": "#2a2a2a",
                    "text": "#ffffff", "textSecondary": "#cccccc", "textTertiary": "#bbbbbb",
                    "accent": "#4a99f7", "accentText": "#0b1a2a", "selection": "#18324b",
                    "success": "#53d49b", "danger": "#f17c80", "warning": "#e9b46b"
                },
                "light": {
                    "canvas": "#ffffff", "chrome": "#f8f8f8", "surface": "#ffffff",
                    "surfaceDeep": "#f4f4f4", "surfaceRaised": "#eeeeee",
                    "border": "#cccccc", "borderFaint": "#dddddd",
                    "text": "#111111", "textSecondary": "#444444", "textTertiary": "#555555",
                    "accent": "#1f6fbf", "accentText": "#ffffff", "selection": "#e1eefb",
                    "success": "#1f7a4d", "danger": "#b4232c", "warning": "#8a5a00"
                }
            }
        })
    }

    fn with_accents(mut document: Value) -> Value {
        document["accents"] = json!({
            "teal": {
                "dark": { "accent": "#2fbfa8", "accentText": "#05231f", "selection": "#123a36" },
                "light": { "accent": "#0f8f7d", "accentText": "#ffffff", "selection": "#dff4ef" }
            }
        });
        document
    }

    fn parse(document: &Value) -> Result<ThemePack, PackError> {
        ThemePack::parse(&document.to_string())
    }

    #[test]
    fn parses_every_role_of_both_schemes() {
        let pack = parse(&document()).unwrap();
        assert_eq!(pack.id, "sample");
        let dark = pack.colors_for(Scheme::Dark, None).unwrap();
        assert_eq!(dark.canvas, Rgba8::rgb(0, 0, 0));
        assert_eq!(dark.surface_raised, Rgba8::rgb(0x1a, 0x1a, 0x1a));
        assert_eq!(dark.color(Role::Warning), Rgba8::rgb(0xe9, 0xb4, 0x6b));
        let light = pack.colors_for(Scheme::Light, None).unwrap();
        assert_eq!(light.canvas, Rgba8::WHITE);
        assert_eq!(light.color(Role::Danger), Rgba8::rgb(0xb4, 0x23, 0x2c));
    }

    #[test]
    fn rejects_a_document_that_omits_a_scheme_or_a_role() {
        let mut without_light = document();
        without_light["colors"] = json!({ "dark": without_light["colors"]["dark"].clone() });
        let error = parse(&without_light).unwrap_err().to_string();
        assert!(error.contains("missing field `light`"), "{error}");

        let mut without_role = document();
        without_role["colors"]["light"]
            .as_object_mut()
            .unwrap()
            .remove("textSecondary");
        let error = parse(&without_role).unwrap_err().to_string();
        assert!(error.contains("missing field `textSecondary`"), "{error}");
    }

    #[test]
    fn rejects_unknown_keys_and_unsupported_versions() {
        let mut with_extra = document();
        with_extra["radius"] = json!(6);
        let error = parse(&with_extra).unwrap_err().to_string();
        assert!(error.contains("unknown field `radius`"), "{error}");

        let mut next_generation = document();
        next_generation["schemaVersion"] = json!(2);
        let error = parse(&next_generation).unwrap_err().to_string();
        assert!(error.contains("schema 2 is unsupported"), "{error}");
    }

    #[test]
    fn rejects_malformed_ids_names_and_colours() {
        let mut uppercase_id = document();
        uppercase_id["id"] = json!("Sample");
        assert!(parse(&uppercase_id)
            .unwrap_err()
            .to_string()
            .contains("theme pack id"));

        let mut empty_id = document();
        empty_id["id"] = json!("");
        assert!(parse(&empty_id)
            .unwrap_err()
            .to_string()
            .contains("theme pack id"));

        let mut empty_name = document();
        empty_name["name"] = json!("");
        assert!(parse(&empty_name)
            .unwrap_err()
            .to_string()
            .contains("theme pack name must not be empty"));

        let mut control_author = document();
        control_author["author"] = json!("a\nb");
        assert!(parse(&control_author)
            .unwrap_err()
            .to_string()
            .contains("must be a string of at most"));

        let mut colour_word = document();
        colour_word["colors"]["dark"]["text"] = json!("white");
        let error = parse(&colour_word).unwrap_err().to_string();
        assert!(error.contains("colors.dark.text must look like"), "{error}");

        let mut short_colour = document();
        short_colour["colors"]["light"]["accent"] = json!("#4a9");
        let error = parse(&short_colour).unwrap_err().to_string();
        assert!(
            error.contains("colors.light.accent must look like"),
            "{error}"
        );
    }

    #[test]
    fn reports_the_pointer_of_the_failing_light_role() {
        let mut document = document();
        document["colors"]["light"]["textSecondary"] = json!("grey");
        let error = parse(&document).unwrap_err().to_string();
        assert!(error.contains("colors.light.textSecondary"), "{error}");
    }

    #[test]
    fn applies_a_selected_accent_family_and_keeps_the_pack_accent_otherwise() {
        let pack = parse(&with_accents(document())).unwrap();
        let plain = pack.colors_for(Scheme::Light, None).unwrap();
        assert_eq!(plain.accent, Rgba8::rgb(0x1f, 0x6f, 0xbf));
        let teal = pack.colors_for(Scheme::Light, Some("teal")).unwrap();
        assert_eq!(teal.accent, Rgba8::rgb(0x0f, 0x8f, 0x7d));
        assert_eq!(teal.accent_text, Rgba8::WHITE);
        assert_eq!(teal.selection, Rgba8::rgb(0xdf, 0xf4, 0xef));
        // A family the pack does not define keeps the pack's own accent.
        let missing = pack.colors_for(Scheme::Dark, Some("violet")).unwrap();
        assert_eq!(missing.accent, Rgba8::rgb(0x4a, 0x99, 0xf7));
    }

    #[test]
    fn accent_family_support_requires_an_accents_map() {
        assert!(!parse(&document()).unwrap().has_accent_families());
        assert!(parse(&with_accents(document()))
            .unwrap()
            .has_accent_families());
    }

    #[test]
    fn rejects_a_malformed_accent_colour_or_name_with_its_pointer() {
        let mut malformed = with_accents(document());
        malformed["accents"]["teal"]["dark"]["selection"] = json!("teal");
        let error = parse(&malformed).unwrap_err().to_string();
        assert!(error.contains("accents.teal.dark.selection"), "{error}");

        let mut named = with_accents(document());
        named["accents"] = json!({
            "Teal": {
                "dark": { "accent": "#2fbfa8", "accentText": "#05231f", "selection": "#123a36" },
                "light": { "accent": "#0f8f7d", "accentText": "#ffffff", "selection": "#dff4ef" }
            }
        });
        let error = parse(&named).unwrap_err().to_string();
        assert!(error.contains("accent `Teal`"), "{error}");
    }

    #[test]
    fn parses_role_colours_with_and_without_alpha() {
        assert_eq!(
            ThemeColor("#2F6FEB".to_owned()).rgba(),
            Some(Rgba8::rgb(0x2f, 0x6f, 0xeb))
        );
        assert_eq!(
            ThemeColor("#2F6FEB80".to_owned()).rgba(),
            Some(Rgba8 {
                r: 0x2f,
                g: 0x6f,
                b: 0xeb,
                a: 0x80
            })
        );
        assert_eq!(
            ThemeColor("#2F6FEB".to_owned()).srgb(),
            Some([0x2f, 0x6f, 0xeb])
        );
        assert_eq!(ThemeColor("#2F6FEB80".to_owned()).srgb(), None);
        for text in ["2F6FEB", "#2F6F", "#2F6FEBB", "#GGGGGG", ""] {
            assert_eq!(ThemeColor(text.to_owned()).rgba(), None, "{text}");
        }
    }
}
