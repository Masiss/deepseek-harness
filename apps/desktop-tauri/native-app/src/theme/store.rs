//! Installed theme packs and resolution into one scheme.
//!
//! The catalogue holds the compiled-in packs, user files and cached GitHub
//! documents discovered by [`super::user::ThemeCatalog`], and the editable
//! `custom` palette migrated from preferences. File and fetched packs resolve
//! through [`resolve_pack_document`] with [`PackTrust::Untrusted`], the only path
//! where a contrast failure rejects a pack.

use std::sync::OnceLock;

use super::contrast;
use super::pack::{SchemeColors, ThemePack};
use super::tokens::{blend_hsl, Rgba8, Scheme};
use super::user::{ThemeCatalog, ThemeSource};
use crate::ui_preferences::{AccentScheme, CustomTheme};

/// Id of the pack a new installation selects.
pub(crate) const DEFAULT_PACK_ID: &str = "dsh";

/// Id of the pack used when the selected one cannot be resolved.
pub(crate) const FALLBACK_PACK_ID: &str = "harness";

/// Id of the pack built from the editable `customTheme` preferences.
pub(crate) const CUSTOM_PACK_ID: &str = "custom";

const BUILTIN_SOURCES: [(&str, &str); 4] = [
    ("dsh", include_str!("packs/dsh.json")),
    ("harness", include_str!("packs/harness.json")),
    ("high-contrast", include_str!("packs/high-contrast.json")),
    ("solarized", include_str!("packs/solarized.json")),
];

static BUILTIN_PACKS: OnceLock<Vec<ThemePack>> = OnceLock::new();

/// The packs compiled into the app, in the order a pack list shows them.
///
/// A built-in document that fails validation is a build defect: the unit tests
/// parse every one, so resolution may treat the catalogue as complete.
pub(crate) fn builtin_packs() -> &'static [ThemePack] {
    BUILTIN_PACKS.get_or_init(|| {
        BUILTIN_SOURCES
            .iter()
            .map(|(id, json)| {
                ThemePack::parse(json).unwrap_or_else(|error| {
                    panic!("built-in theme pack `{id}` is invalid: {error}")
                })
            })
            .collect()
    })
}

/// The compiled-in pack with this id, when the app ships one.
pub(crate) fn builtin_pack(id: &str) -> Option<&'static ThemePack> {
    builtin_packs().iter().find(|pack| pack.id == id)
}

/// Where a pack document came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PackTrust {
    /// Shipped with the app, or migrated from preferences the app itself wrote.
    Trusted,
    /// Read from a user file.
    Untrusted,
}

/// One frame's theme input.
pub(crate) struct ThemeRequest<'a> {
    /// Pack id stored in preferences.
    pub(crate) pack_id: &'a str,
    /// Editable custom palettes, used when `pack_id` is `custom`.
    pub(crate) custom: &'a CustomTheme,
    /// Installed built-in, user, and cached packs.
    pub(crate) catalog: &'a ThemeCatalog,
    /// Accent family preference.
    pub(crate) accent: AccentScheme,
    /// Whether the dark scheme is active.
    pub(crate) dark: bool,
}

/// The colours resolved for one frame and the status that produced them.
pub(crate) struct ResolvedTheme {
    /// Colours of the active scheme.
    pub(crate) colors: SchemeColors,
    /// Whether the pack used defines accent families.
    pub(crate) accent_families: bool,
    /// Pack used, plus any rejection or contrast notice.
    pub(crate) status: ThemeStatus,
}

/// Resolution status a Preferences notice can present.
#[derive(Clone, Debug)]
#[allow(dead_code)] // Settings shows errors; pack id, trust, and warnings stay exposed for a later notice.
pub(crate) struct ThemeStatus {
    /// Pack id that produced the colours, including a fallback.
    pub(crate) pack_id: String,
    /// Whether the pack came from a trusted source.
    pub(crate) trusted: bool,
    /// Why the selected pack was replaced; empty when it resolved.
    pub(crate) errors: Vec<String>,
    /// Contrast pairs the used pack misses; never blocks rendering.
    pub(crate) warnings: Vec<String>,
}

/// One resolved pack with its warning list.
struct PackOutcome {
    pack_id: String,
    trusted: bool,
    colors: SchemeColors,
    accent_families: bool,
    warnings: Vec<String>,
}

/// Resolves the selected pack for one scheme.
///
/// An unknown id, a custom palette that cannot be parsed, or a rejected untrusted
/// pack falls back to `harness` and records the concrete reason in the status.
///
/// @param request — pack selection, custom palettes, accent preference, scheme.
/// @returns the active scheme's colours and their status.
pub(crate) fn resolve(request: &ThemeRequest<'_>) -> ResolvedTheme {
    let scheme = Scheme::of(request.dark);
    let mut errors = Vec::new();
    let requested = if request.pack_id == CUSTOM_PACK_ID {
        Ok(custom_outcome(request.custom, scheme))
    } else {
        match request.catalog.find(request.pack_id) {
            Some(theme) if theme.source == ThemeSource::Builtin => {
                builtin_outcome(&theme.pack, request.accent, scheme)
            }
            Some(theme) => external_outcome(&theme.pack, request.accent, scheme),
            None => Err(vec![format!(
                "theme `{}` is not installed",
                request.pack_id
            )]),
        }
    };
    let outcome = match requested {
        Ok(outcome) => outcome,
        Err(mut failures) => {
            errors.append(&mut failures);
            errors.push(format!("using the `{FALLBACK_PACK_ID}` theme instead"));
            let pack = builtin_pack(FALLBACK_PACK_ID).expect("the fallback theme pack is built in");
            builtin_outcome(pack, request.accent, scheme).expect("the fallback theme pack resolves")
        }
    };
    ResolvedTheme {
        colors: outcome.colors,
        accent_families: outcome.accent_families,
        status: ThemeStatus {
            pack_id: outcome.pack_id,
            trusted: outcome.trusted,
            errors,
            warnings: outcome.warnings,
        },
    }
}

/// Resolves one parsed pack document under a trust policy.
///
/// A trusted pack reports contrast failures as warnings; an untrusted pack
/// returns them as errors so the caller can fall back to another pack.
///
/// @param pack — a validated pack document.
/// @param trust — whether the document ships with the app or came from outside.
/// @param accent — accent family preference.
/// @param scheme — the active light or dark scheme.
/// @returns the colours and warning list, or the reasons the pack was rejected.
pub(crate) fn resolve_pack_document(
    pack: &ThemePack,
    trust: PackTrust,
    accent: AccentScheme,
    scheme: Scheme,
) -> Result<(SchemeColors, Vec<String>), Vec<String>> {
    let colors = pack
        .colors_for(scheme, accent.pack_accent_key())
        .map_err(|error| vec![format!("theme `{}` cannot be used: {error}", pack.id)])?;
    let report = contrast::report(&colors);
    let mut warnings: Vec<String> = report.warnings.iter().map(ToString::to_string).collect();
    if report.failures.is_empty() {
        return Ok((colors, warnings));
    }
    let failures: Vec<String> = report.failures.iter().map(ToString::to_string).collect();
    match trust {
        PackTrust::Trusted => {
            warnings.extend(failures);
            Ok((colors, warnings))
        }
        PackTrust::Untrusted => Err(failures),
    }
}

fn builtin_outcome(
    pack: &ThemePack,
    accent: AccentScheme,
    scheme: Scheme,
) -> Result<PackOutcome, Vec<String>> {
    let (colors, warnings) = resolve_pack_document(pack, PackTrust::Trusted, accent, scheme)?;
    Ok(PackOutcome {
        pack_id: pack.id.clone(),
        trusted: true,
        colors,
        accent_families: pack.has_accent_families(),
        warnings,
    })
}

/// Resolves a file or fetched pack under the untrusted contrast gate.
fn external_outcome(
    pack: &ThemePack,
    accent: AccentScheme,
    scheme: Scheme,
) -> Result<PackOutcome, Vec<String>> {
    let (colors, warnings) = resolve_pack_document(pack, PackTrust::Untrusted, accent, scheme)
        .map_err(|failures| {
            failures
                .into_iter()
                .map(|reason| format!("theme `{}` was rejected: {reason}", pack.id))
                .collect::<Vec<String>>()
        })?;
    Ok(PackOutcome {
        pack_id: pack.id.clone(),
        trusted: false,
        colors,
        accent_families: pack.has_accent_families(),
        warnings,
    })
}

/// Builds the `custom` pack from the editable palettes kept in preferences.
///
/// Roles the v5 custom editor never exposed reuse the closest editable role, so
/// a migrated custom theme renders as it did: `chrome` takes the background,
/// `surfaceDeep` the surface, the faint border the border, tertiary text muted
/// text, and the raised surface keeps the v5 HSL blend toward text. The legacy
/// palette defines no accent families, so `accentScheme` does not restyle it.
fn custom_outcome(custom: &CustomTheme, scheme: Scheme) -> PackOutcome {
    let colors = custom_colors(custom, scheme);
    let report = contrast::report(&colors);
    let mut warnings: Vec<String> = report.warnings.iter().map(ToString::to_string).collect();
    warnings.extend(report.failures.iter().map(ToString::to_string));
    PackOutcome {
        pack_id: CUSTOM_PACK_ID.to_owned(),
        trusted: true,
        colors,
        accent_families: false,
        warnings,
    }
}

fn custom_colors(custom: &CustomTheme, scheme: Scheme) -> SchemeColors {
    let dark = scheme.is_dark();
    let palette = if dark { &custom.dark } else { &custom.light };
    let background = if dark {
        Rgba8::rgb(0x1b, 0x1b, 0x1b)
    } else {
        Rgba8::rgb(0xfa, 0xfa, 0xfa)
    };
    let grey = Rgba8::rgb(0xa0, 0xa0, 0xa0);
    let canvas = palette.background.rgba().unwrap_or(background);
    let surface = palette.surface.rgba().unwrap_or(background);
    let text = palette.text.rgba().unwrap_or(Rgba8::WHITE);
    let border = palette.border.rgba().unwrap_or(grey);
    let muted = palette.muted.rgba().unwrap_or(grey);
    let accent = palette
        .accent
        .rgba()
        .unwrap_or(Rgba8::rgb(0xad, 0xd8, 0xe6));
    SchemeColors {
        canvas,
        chrome: canvas,
        surface,
        surface_deep: surface,
        surface_raised: blend_hsl(surface, text, if dark { 0.06 } else { 0.04 }),
        border,
        border_faint: border,
        text,
        text_secondary: muted,
        text_tertiary: muted,
        accent,
        accent_text: accent,
        selection: palette.selection.rgba().unwrap_or(Rgba8::rgb(0, 0, 255)),
        // The v5 custom palette had no success colour; state colours that the
        // shell does not render yet use the Harness green.
        success: if dark {
            Rgba8::rgb(0x6f, 0xcf, 0x97)
        } else {
            Rgba8::rgb(0x2e, 0x7d, 0x32)
        },
        danger: palette.error.rgba().unwrap_or(Rgba8::rgb(255, 0, 0)),
        warning: palette.warn.rgba().unwrap_or(Rgba8::rgb(255, 255, 0)),
    }
}

#[cfg(test)]
mod tests {
    use super::super::tokens::Role;
    use super::*;
    use serde_json::json;

    fn synthetic(text: &str, accent_text: &str) -> ThemePack {
        let document = json!({
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
                    "text": text, "textSecondary": "#cccccc", "textTertiary": "#bbbbbb",
                    "accent": "#4a99f7", "accentText": accent_text, "selection": "#18324b",
                    "success": "#53d49b", "danger": "#f17c80", "warning": "#e9b46b"
                },
                "light": {
                    "canvas": "#ffffff", "chrome": "#f8f8f8", "surface": "#ffffff",
                    "surfaceDeep": "#f4f4f4", "surfaceRaised": "#eeeeee",
                    "border": "#cccccc", "borderFaint": "#dddddd",
                    "text": text, "textSecondary": "#444444", "textTertiary": "#555555",
                    "accent": "#1f6fbf", "accentText": accent_text, "selection": "#e1eefb",
                    "success": "#1f7a4d", "danger": "#b4232c", "warning": "#8a5a00"
                }
            }
        });
        ThemePack::parse(&document.to_string()).unwrap()
    }

    #[test]
    fn the_binary_ships_the_four_builtin_packs() {
        let ids: Vec<&str> = builtin_packs()
            .iter()
            .map(|pack| pack.id.as_str())
            .collect();
        assert_eq!(ids, ["dsh", "harness", "high-contrast", "solarized"]);
        for pack in builtin_packs() {
            for scheme in [Scheme::Dark, Scheme::Light] {
                assert!(
                    pack.colors_for(scheme, None).is_ok(),
                    "{} {}",
                    pack.id,
                    scheme.name()
                );
            }
        }
        assert_eq!(builtin_pack(DEFAULT_PACK_ID).unwrap().name, "DSH");
        assert_eq!(builtin_pack(FALLBACK_PACK_ID).unwrap().id, "harness");
        assert!(builtin_pack("custom").is_none());
    }

    #[test]
    fn harness_keeps_the_v5_palette_role_for_role() {
        let pack = builtin_pack("harness").unwrap();

        let dark = pack.colors_for(Scheme::Dark, None).unwrap();
        for (role, expected) in [
            (Role::Canvas, Rgba8::rgb(0x17, 0x1a, 0x1f)),
            (Role::Surface, Rgba8::rgb(0x22, 0x27, 0x2e)),
            (Role::SurfaceDeep, Rgba8::rgb(0x22, 0x27, 0x2e)),
            (Role::SurfaceRaised, Rgba8::rgb(0x2c, 0x34, 0x3c)),
            (Role::Border, Rgba8::rgb(0x35, 0x3d, 0x47)),
            (Role::Text, Rgba8::rgb(0xee, 0xf1, 0xf4)),
            (Role::TextSecondary, Rgba8::rgb(0x92, 0x9b, 0xa6)),
            (Role::Accent, Rgba8::rgb(0xe9, 0x56, 0x18)),
            (Role::AccentText, Rgba8::rgb(0xff, 0x9b, 0x70)),
            (Role::Selection, Rgba8::rgb(0x3b, 0x2a, 0x23)),
            (Role::Danger, Rgba8::rgb(0xff, 0x9b, 0x70)),
            (Role::Warning, Rgba8::rgb(0xe7, 0xc9, 0x79)),
        ] {
            assert_eq!(dark.color(role), expected, "{}", role.name());
        }

        let light = pack.colors_for(Scheme::Light, None).unwrap();
        for (role, expected) in [
            (Role::Canvas, Rgba8::rgb(0xf1, 0xf2, 0xf3)),
            (Role::Surface, Rgba8::WHITE),
            (Role::SurfaceDeep, Rgba8::WHITE),
            (Role::SurfaceRaised, Rgba8::rgb(0xf6, 0xf6, 0xf6)),
            (Role::Border, Rgba8::rgb(0xe6, 0xe6, 0xe3)),
            (Role::Text, Rgba8::rgb(0x20, 0x20, 0x1f)),
            (Role::TextSecondary, Rgba8::rgb(0x70, 0x70, 0x6b)),
            (Role::Accent, Rgba8::rgb(0xe9, 0x56, 0x18)),
            (Role::AccentText, Rgba8::rgb(0xa8, 0x3b, 0x10)),
            (Role::Selection, Rgba8::rgb(0xff, 0xf1, 0xe9)),
            (Role::Danger, Rgba8::rgb(0x9c, 0x39, 0x16)),
            (Role::Warning, Rgba8::rgb(0x79, 0x5b, 0x1a)),
        ] {
            assert_eq!(light.color(role), expected, "{}", role.name());
        }
    }

    #[test]
    fn custom_colours_keep_the_v5_derivations() {
        let custom = CustomTheme::default();
        let dark = custom_colors(&custom, Scheme::Dark);
        assert_eq!(dark.canvas, Rgba8::rgb(0x17, 0x1a, 0x1f));
        assert_eq!(dark.chrome, dark.canvas);
        assert_eq!(dark.surface, Rgba8::rgb(0x22, 0x27, 0x2e));
        assert_eq!(dark.surface_deep, dark.surface);
        assert_eq!(dark.surface_raised, Rgba8::rgb(0x2c, 0x34, 0x3c));
        assert_eq!(dark.border, dark.border_faint);
        assert_eq!(dark.text_secondary, dark.text_tertiary);
        // The v5 custom palette used the accent for accent text and ignored accent families.
        assert_eq!(dark.accent, Rgba8::rgb(0xe9, 0x56, 0x18));
        assert_eq!(dark.accent_text, dark.accent);
        assert_eq!(dark.danger, Rgba8::rgb(0xff, 0x9b, 0x70));
        assert_eq!(dark.warning, Rgba8::rgb(0xe7, 0xc9, 0x79));

        let light = custom_colors(&custom, Scheme::Light);
        assert_eq!(light.canvas, Rgba8::rgb(0xf1, 0xf2, 0xf3));
        assert_eq!(light.surface, Rgba8::WHITE);
        assert_eq!(light.surface_raised, Rgba8::rgb(0xf6, 0xf6, 0xf6));
    }

    #[test]
    fn grandfathered_packs_report_a_stable_warning_list() {
        let expected: [((&str, &str), &[&str]); 8] = [
            (
                ("dsh", "dark"),
                &[
                    "border on canvas is 1.44:1, needs 3.0:1",
                    "borderFaint on canvas is 1.24:1, needs 3.0:1",
                    "border on surface is 1.39:1, needs 3.0:1",
                    "borderFaint on surface is 1.19:1, needs 3.0:1",
                    "border on surfaceDeep is 1.46:1, needs 3.0:1",
                    "borderFaint on surfaceDeep is 1.25:1, needs 3.0:1",
                    "border on surfaceRaised is 1.30:1, needs 3.0:1",
                    "borderFaint on surfaceRaised is 1.12:1, needs 3.0:1",
                ],
            ),
            (
                ("dsh", "light"),
                &[
                    "border on canvas is 1.25:1, needs 3.0:1",
                    "borderFaint on canvas is 1.14:1, needs 3.0:1",
                    "border on surface is 1.33:1, needs 3.0:1",
                    "borderFaint on surface is 1.21:1, needs 3.0:1",
                    "border on surfaceDeep is 1.27:1, needs 3.0:1",
                    "borderFaint on surfaceDeep is 1.16:1, needs 3.0:1",
                    "border on surfaceRaised is 1.19:1, needs 3.0:1",
                    "borderFaint on surfaceRaised is 1.08:1, needs 3.0:1",
                    "accentText on accent is 4.15:1, needs 4.5:1",
                ],
            ),
            (
                ("harness", "dark"),
                &[
                    "border on canvas is 1.59:1, needs 3.0:1",
                    "borderFaint on canvas is 1.59:1, needs 3.0:1",
                    "border on surface is 1.37:1, needs 3.0:1",
                    "borderFaint on surface is 1.37:1, needs 3.0:1",
                    "border on surfaceDeep is 1.37:1, needs 3.0:1",
                    "borderFaint on surfaceDeep is 1.37:1, needs 3.0:1",
                    "textTertiary on surfaceRaised is 4.49:1, needs 4.5:1",
                    "border on surfaceRaised is 1.15:1, needs 3.0:1",
                    "borderFaint on surfaceRaised is 1.15:1, needs 3.0:1",
                    "textSecondary on surfaceRaised is 4.49:1, needs 4.5:1",
                    "accentText on accent is 1.75:1, needs 4.5:1",
                ],
            ),
            (
                ("harness", "light"),
                &[
                    "textTertiary on canvas is 4.44:1, needs 4.5:1",
                    "border on canvas is 1.12:1, needs 3.0:1",
                    "borderFaint on canvas is 1.12:1, needs 3.0:1",
                    "border on surface is 1.25:1, needs 3.0:1",
                    "borderFaint on surface is 1.25:1, needs 3.0:1",
                    "border on surfaceDeep is 1.25:1, needs 3.0:1",
                    "borderFaint on surfaceDeep is 1.25:1, needs 3.0:1",
                    "border on surfaceRaised is 1.16:1, needs 3.0:1",
                    "borderFaint on surfaceRaised is 1.16:1, needs 3.0:1",
                    "textSecondary on canvas is 4.44:1, needs 4.5:1",
                    "accentText on accent is 1.76:1, needs 4.5:1",
                ],
            ),
            (
                ("high-contrast", "dark"),
                &["accentText on accent is 1.00:1, needs 4.5:1"],
            ),
            (
                ("high-contrast", "light"),
                &["accentText on accent is 1.00:1, needs 4.5:1"],
            ),
            (
                ("solarized", "dark"),
                &[
                    "border on canvas is 2.79:1, needs 3.0:1",
                    "borderFaint on canvas is 2.79:1, needs 3.0:1",
                    "border on surface is 2.42:1, needs 3.0:1",
                    "borderFaint on surface is 2.42:1, needs 3.0:1",
                    "border on surfaceDeep is 2.42:1, needs 3.0:1",
                    "borderFaint on surfaceDeep is 2.42:1, needs 3.0:1",
                    "textTertiary on surfaceRaised is 4.00:1, needs 4.5:1",
                    "border on surfaceRaised is 1.99:1, needs 3.0:1",
                    "borderFaint on surfaceRaised is 1.99:1, needs 3.0:1",
                    "success on surface is 4.06:1, needs 4.5:1",
                    "danger on canvas is 3.25:1, needs 4.5:1",
                    "danger on surface is 2.81:1, needs 4.5:1",
                    "warning on surface is 4.05:1, needs 4.5:1",
                    "text on surface is 4.11:1, needs 4.5:1",
                    "text on surfaceDeep is 4.11:1, needs 4.5:1",
                    "text on surfaceRaised is 3.38:1, needs 4.5:1",
                    "textSecondary on surfaceRaised is 4.00:1, needs 4.5:1",
                    "accentText on accent is 1.00:1, needs 4.5:1",
                ],
            ),
            (
                ("solarized", "light"),
                &[
                    "textTertiary on canvas is 4.13:1, needs 4.5:1",
                    "border on canvas is 2.48:1, needs 3.0:1",
                    "borderFaint on canvas is 2.48:1, needs 3.0:1",
                    "textTertiary on surface is 3.64:1, needs 4.5:1",
                    "border on surface is 2.18:1, needs 3.0:1",
                    "borderFaint on surface is 2.18:1, needs 3.0:1",
                    "textTertiary on surfaceDeep is 3.64:1, needs 4.5:1",
                    "border on surfaceDeep is 2.18:1, needs 3.0:1",
                    "borderFaint on surfaceDeep is 2.18:1, needs 3.0:1",
                    "textTertiary on surfaceRaised is 3.38:1, needs 4.5:1",
                    "border on surfaceRaised is 2.03:1, needs 3.0:1",
                    "borderFaint on surfaceRaised is 2.03:1, needs 3.0:1",
                    "success on canvas is 2.97:1, needs 4.5:1",
                    "success on surface is 2.62:1, needs 4.5:1",
                    "danger on canvas is 4.29:1, needs 4.5:1",
                    "danger on surface is 3.77:1, needs 4.5:1",
                    "warning on surface is 4.02:1, needs 4.5:1",
                    "text on canvas is 4.13:1, needs 4.5:1",
                    "text on surface is 3.64:1, needs 4.5:1",
                    "text on surfaceDeep is 3.64:1, needs 4.5:1",
                    "text on surfaceRaised is 3.38:1, needs 4.5:1",
                    "textSecondary on canvas is 4.13:1, needs 4.5:1",
                    "textSecondary on surface is 3.64:1, needs 4.5:1",
                    "textSecondary on surfaceDeep is 3.64:1, needs 4.5:1",
                    "textSecondary on surfaceRaised is 3.38:1, needs 4.5:1",
                    "accent on canvas is 2.93:1, needs 3.0:1",
                    "accent on surface is 2.58:1, needs 3.0:1",
                    "accent on surfaceDeep is 2.58:1, needs 3.0:1",
                    "accent on surfaceRaised is 2.40:1, needs 3.0:1",
                    "accentText on accent is 1.00:1, needs 4.5:1",
                ],
            ),
        ];
        for ((pack, scheme), issues) in expected {
            let pack = builtin_pack(pack).unwrap();
            let scheme = if scheme == "dark" {
                Scheme::Dark
            } else {
                Scheme::Light
            };
            let (_, warnings) =
                resolve_pack_document(pack, PackTrust::Trusted, AccentScheme::Harness, scheme)
                    .unwrap();
            let expected: Vec<String> = issues.iter().map(|issue| (*issue).to_owned()).collect();
            assert_eq!(warnings, expected, "{} {}", pack.id, scheme.name());
        }
    }

    #[test]
    fn an_untrusted_pack_is_rejected_only_when_the_hard_gate_fails() {
        let passing = synthetic("#ffffff", "#0b1a2a");
        let (colors, warnings) = resolve_pack_document(
            &passing,
            PackTrust::Untrusted,
            AccentScheme::Harness,
            Scheme::Dark,
        )
        .unwrap();
        assert_eq!(colors.text, Rgba8::WHITE);
        assert!(warnings.iter().all(|warning| warning.contains("on")));

        let failing = synthetic("#777777", "#0b1a2a");
        let failures = resolve_pack_document(
            &failing,
            PackTrust::Untrusted,
            AccentScheme::Harness,
            Scheme::Dark,
        )
        .unwrap_err();
        assert!(
            failures
                .iter()
                .any(|failure| failure.starts_with("text on")),
            "{failures:?}"
        );
        assert!(failures
            .iter()
            .all(|failure| failure.contains("needs 4.5:1") || failure.contains("needs 3.0:1")));

        // The same document is trusted when the app shipped it: warnings, never a rejection.
        let (_, trusted_warnings) = resolve_pack_document(
            &failing,
            PackTrust::Trusted,
            AccentScheme::Harness,
            Scheme::Dark,
        )
        .unwrap();
        assert!(trusted_warnings
            .iter()
            .any(|warning| warning.starts_with("text on")));
    }

    #[test]
    fn accent_families_replace_accent_accent_text_and_selection() {
        let pack = builtin_pack("harness").unwrap();
        let colors = pack
            .colors_for(Scheme::Light, AccentScheme::Blue.pack_accent_key())
            .unwrap();
        assert_eq!(colors.accent, Rgba8::rgb(0x2e, 0x6b, 0xd8));
        assert_eq!(colors.accent_text, Rgba8::rgb(0x2e, 0x6b, 0xd8));
        assert_eq!(colors.selection, Rgba8::rgb(0xff, 0xf1, 0xe9));

        // Packs without the family keep their own accent, as Solarized always did.
        let colors = pack.colors_for(Scheme::Light, None).unwrap();
        assert_eq!(colors.accent, Rgba8::rgb(0xe9, 0x56, 0x18));
        let solarized = builtin_pack("solarized").unwrap();
        let colors = solarized
            .colors_for(Scheme::Dark, AccentScheme::Violet.pack_accent_key())
            .unwrap();
        assert_eq!(colors.accent, Rgba8::rgb(0x2a, 0xa1, 0x98));
    }

    #[test]
    fn an_unknown_pack_resolves_harness_and_names_the_reason() {
        let custom = CustomTheme::default();
        let catalog = ThemeCatalog::builtins();
        let resolved = resolve(&ThemeRequest {
            pack_id: "missing",
            custom: &custom,
            catalog: &catalog,
            accent: AccentScheme::Harness,
            dark: false,
        });
        assert_eq!(resolved.status.pack_id, FALLBACK_PACK_ID);
        assert!(resolved.status.trusted);
        assert_eq!(
            resolved.status.errors,
            [
                "theme `missing` is not installed".to_owned(),
                "using the `harness` theme instead".to_owned(),
            ]
        );
        assert_eq!(resolved.colors.canvas, Rgba8::rgb(0xf1, 0xf2, 0xf3));
    }

    #[test]
    fn accent_family_support_follows_the_resolved_pack() {
        let custom = CustomTheme::default();
        let catalog = ThemeCatalog::builtins();
        for (pack_id, expected) in [
            ("harness", true),
            ("dsh", true),
            ("solarized", false),
            ("high-contrast", false),
            (CUSTOM_PACK_ID, false),
            // An unknown id resolves the Harness pack, which defines families.
            ("missing", true),
        ] {
            let resolved = resolve(&ThemeRequest {
                pack_id,
                custom: &custom,
                catalog: &catalog,
                accent: AccentScheme::Blue,
                dark: false,
            });
            assert_eq!(resolved.accent_families, expected, "{pack_id}");
        }
        assert!(builtin_pack("harness").unwrap().has_accent_families());
        assert!(!builtin_pack("solarized").unwrap().has_accent_families());
    }

    #[test]
    fn the_custom_pack_resolves_as_a_trusted_pack() {
        let custom = CustomTheme::default();
        let catalog = ThemeCatalog::builtins();
        let resolved = resolve(&ThemeRequest {
            pack_id: CUSTOM_PACK_ID,
            custom: &custom,
            catalog: &catalog,
            accent: AccentScheme::Teal,
            dark: true,
        });
        assert_eq!(resolved.status.pack_id, CUSTOM_PACK_ID);
        assert!(resolved.status.trusted);
        assert!(resolved.status.errors.is_empty());
    }
}
