//! The per-frame theme snapshot: pack colours plus the app's own metrics.

use crate::theme::{resolve, Rgba8, ThemeCatalog, ThemeRequest, ThemeStatus};
use crate::ui_preferences::{UiDensity, UiPreferences};
use gpui::{px, AbsoluteLength, FontFallbacks, FontStyle, FontWeight, Hsla, Pixels, SharedString};

/// Resolved colors and metrics for one frame.
#[derive(Clone)]
pub(crate) struct Theme {
    pub dark: bool,
    pub background: Hsla,
    pub surface: Hsla,
    pub surface_raised: Hsla,
    pub code_background: Hsla,
    pub text: Hsla,
    pub muted: Hsla,
    pub accent: Hsla,
    pub accent_text: Hsla,
    pub selection: Hsla,
    pub border: Hsla,
    pub warn: Hsla,
    pub error: Hsla,
    pub success: Hsla,
    pub radius: f32,
    pub scale: f32,
    pub compact: bool,
    pub sans: SharedString,
    pub mono: SharedString,
    pub fallbacks: Vec<String>,
    /// Whether the resolved pack defines accent families.
    pub(crate) accent_families: bool,
    /// Which pack resolved and what it had to fall back from.
    pub(crate) status: ThemeStatus,
}

impl Theme {
    /// Resolves one frame's colours from the selected pack and the scheme mode.
    ///
    /// @param preferences — effective preferences, including workspace overrides.
    /// @param system_dark — whether the desktop currently reports a dark appearance.
    /// @returns the snapshot every render call site reads.
    #[cfg(test)]
    pub(crate) fn new(preferences: &UiPreferences, system_dark: bool) -> Self {
        let catalog = ThemeCatalog::builtins();
        Self::new_with_catalog(preferences, system_dark, &catalog)
    }

    /// Resolves one frame's colours from an installed-pack catalogue.
    ///
    /// @param preferences — effective preferences, including workspace overrides.
    /// @param system_dark — whether the desktop currently reports a dark appearance.
    /// @param catalog — built-in and user packs discovered for this client.
    /// @returns the snapshot every render call site reads.
    pub(crate) fn new_with_catalog(
        preferences: &UiPreferences,
        system_dark: bool,
        catalog: &ThemeCatalog,
    ) -> Self {
        let dark = preferences.theme.is_dark(system_dark);
        let resolved = resolve(&ThemeRequest {
            pack_id: &preferences.theme_id,
            custom: &preferences.custom_theme,
            catalog,
            accent: preferences.accent_scheme,
            dark,
        });
        let colors = resolved.colors;
        let mut fallbacks = vec![
            "Noto Sans".to_owned(),
            "DejaVu Sans".to_owned(),
            "Liberation Sans".to_owned(),
            "Noto Color Emoji".to_owned(),
        ];
        fallbacks.extend(
            [
                "Noto Sans CJK SC",
                "Noto Sans CJK JP",
                "Noto Sans Vietnamese",
            ]
            .map(str::to_owned),
        );
        Self {
            dark,
            background: convert(colors.canvas),
            surface: convert(colors.surface),
            surface_raised: convert(colors.surface_raised),
            code_background: convert(colors.surface_deep),
            text: convert(colors.text),
            muted: convert(colors.text_secondary),
            accent: convert(colors.accent),
            accent_text: convert(colors.accent_text),
            selection: convert(colors.selection),
            border: convert(colors.border),
            warn: convert(colors.warning),
            error: convert(colors.danger),
            success: convert(colors.success),
            radius: preferences.radius,
            scale: preferences.ui_scale,
            compact: preferences.density == UiDensity::Compact,
            sans: font_name(&preferences.fonts.proportional),
            mono: font_name(&preferences.fonts.monospace),
            fallbacks,
            accent_families: resolved.accent_families,
            status: resolved.status,
        }
    }

    /// Scales a layout value by the persisted interface scale.
    pub(crate) fn sp(&self, value: f32) -> Pixels {
        px(value * self.scale)
    }

    /// A length for text, in scaled pixels.
    pub(crate) fn fs(&self, value: f32) -> AbsoluteLength {
        px(value * self.scale).into()
    }

    /// Gap between stacked controls.
    pub(crate) fn gap(&self) -> Pixels {
        if self.compact {
            self.sp(4.0)
        } else {
            self.sp(8.0)
        }
    }

    /// Height of a standard control.
    pub(crate) fn control_height(&self) -> Pixels {
        self.sp(if self.compact { 22.0 } else { 26.0 })
    }

    /// Rounded corner radius in scaled pixels.
    pub(crate) fn corner(&self) -> Pixels {
        self.sp(self.radius)
    }

    /// The proportional font, including installed fallback families.
    pub(crate) fn sans_font(&self) -> gpui::Font {
        gpui::Font {
            family: self.sans.clone(),
            features: Default::default(),
            fallbacks: Some(FontFallbacks::from_fonts(self.fallbacks.clone())),
            weight: FontWeight::NORMAL,
            style: FontStyle::Normal,
        }
    }

    /// The monospace font used by code, transcripts, and terminals.
    pub(crate) fn mono_font(&self) -> gpui::Font {
        gpui::Font {
            family: self.mono.clone(),
            features: Default::default(),
            fallbacks: Some(FontFallbacks::from_fonts(self.fallbacks.clone())),
            weight: FontWeight::NORMAL,
            style: FontStyle::Normal,
        }
    }
}

fn font_name(configured: &str) -> SharedString {
    if configured.trim().is_empty() {
        ".SystemUIFont".into()
    } else {
        configured.to_owned().into()
    }
}

fn convert(color: Rgba8) -> Hsla {
    let Rgba8 { r, g, b, a } = color;
    gpui::rgba(u32::from_be_bytes([r, g, b, a])).into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui_preferences::{AccentScheme, ThemeMode};

    #[test]
    fn theme_sp_scales_layout_by_persisted_scale() {
        let preferences = UiPreferences {
            ui_scale: 2.0,
            radius: 4.0,
            ..UiPreferences::default()
        };
        let theme = Theme::new(&preferences, false);
        assert_eq!(theme.scale, 2.0);
        assert_eq!(theme.sp(10.), px(20.));
        assert_eq!(theme.corner(), px(8.));
    }

    #[test]
    fn theme_metrics_follow_density() {
        let compact = UiPreferences {
            density: UiDensity::Compact,
            ..UiPreferences::default()
        };
        let theme = Theme::new(&compact, false);
        assert_eq!(theme.gap(), px(4.));
        assert_eq!(theme.control_height(), px(22.));

        let regular = UiPreferences::default();
        let theme = Theme::new(&regular, false);
        assert_eq!(theme.gap(), px(8.));
        assert_eq!(theme.control_height(), px(26.));
    }

    #[test]
    fn dark_tracks_the_persisted_mode_instead_of_the_desktop() {
        for (mode, system_dark, expected) in [
            (ThemeMode::System, true, true),
            (ThemeMode::System, false, false),
            (ThemeMode::Light, true, false),
            (ThemeMode::Light, false, false),
            (ThemeMode::Dark, true, true),
            (ThemeMode::Dark, false, true),
        ] {
            let preferences = UiPreferences {
                theme: mode,
                ..UiPreferences::default()
            };
            let theme = Theme::new(&preferences, system_dark);
            assert_eq!(
                theme.dark, expected,
                "{mode:?} with system_dark={system_dark}"
            );
        }
    }

    #[test]
    fn harness_pack_keeps_the_palette_that_predates_theme_packs() {
        let dark = Theme::new(
            &UiPreferences {
                theme_id: "harness".to_owned(),
                theme: ThemeMode::Dark,
                ..UiPreferences::default()
            },
            false,
        );
        assert_eq!(dark.background, convert(Rgba8::rgb(0x17, 0x1a, 0x1f)));
        assert_eq!(dark.surface, convert(Rgba8::rgb(0x22, 0x27, 0x2e)));
        assert_eq!(dark.surface_raised, convert(Rgba8::rgb(0x2c, 0x34, 0x3c)));
        assert_eq!(dark.code_background, convert(Rgba8::rgb(0x22, 0x27, 0x2e)));
        assert_eq!(dark.text, convert(Rgba8::rgb(0xee, 0xf1, 0xf4)));
        assert_eq!(dark.muted, convert(Rgba8::rgb(0x92, 0x9b, 0xa6)));
        assert_eq!(dark.accent, convert(Rgba8::rgb(0xe9, 0x56, 0x18)));
        assert_eq!(dark.accent_text, convert(Rgba8::rgb(0xff, 0x9b, 0x70)));
        assert_eq!(dark.selection, convert(Rgba8::rgb(0x3b, 0x2a, 0x23)));
        assert_eq!(dark.border, convert(Rgba8::rgb(0x35, 0x3d, 0x47)));
        assert_eq!(dark.warn, convert(Rgba8::rgb(0xe7, 0xc9, 0x79)));
        assert_eq!(dark.error, convert(Rgba8::rgb(0xff, 0x9b, 0x70)));

        let light = Theme::new(
            &UiPreferences {
                theme_id: "harness".to_owned(),
                theme: ThemeMode::Light,
                ..UiPreferences::default()
            },
            true,
        );
        assert_eq!(light.background, convert(Rgba8::rgb(0xf1, 0xf2, 0xf3)));
        assert_eq!(light.surface, convert(Rgba8::rgb(0xff, 0xff, 0xff)));
        assert_eq!(light.surface_raised, convert(Rgba8::rgb(0xf6, 0xf6, 0xf6)));
        assert_eq!(light.accent, convert(Rgba8::rgb(0xe9, 0x56, 0x18)));
        assert_eq!(light.accent_text, convert(Rgba8::rgb(0xa8, 0x3b, 0x10)));
    }

    #[test]
    fn success_tracks_the_pack_role() {
        let harness_dark = Theme::new(
            &UiPreferences {
                theme_id: "harness".to_owned(),
                theme: ThemeMode::Dark,
                ..UiPreferences::default()
            },
            false,
        );
        assert_eq!(
            harness_dark.success,
            convert(Rgba8::rgb(0x6f, 0xcf, 0x97))
        );

        let dsh_light = Theme::new(
            &UiPreferences {
                theme: ThemeMode::Light,
                ..UiPreferences::default()
            },
            true,
        );
        assert_eq!(dsh_light.success, convert(Rgba8::rgb(0x1f, 0x7a, 0x4d)));
    }

    #[test]
    fn accent_families_replace_accent_and_selection_for_packs_that_define_them() {
        let theme = Theme::new(
            &UiPreferences {
                theme_id: "harness".to_owned(),
                theme: ThemeMode::Dark,
                accent_scheme: AccentScheme::Blue,
                ..UiPreferences::default()
            },
            true,
        );
        assert_eq!(theme.accent, convert(Rgba8::rgb(0x2e, 0x6b, 0xd8)));
        assert_eq!(theme.selection, convert(Rgba8::rgb(0x3b, 0x2a, 0x23)));

        // Packs without accent families keep their own accent.
        let theme = Theme::new(
            &UiPreferences {
                theme_id: "solarized".to_owned(),
                theme: ThemeMode::Dark,
                accent_scheme: AccentScheme::Violet,
                ..UiPreferences::default()
            },
            true,
        );
        assert_eq!(theme.accent, convert(Rgba8::rgb(0x2a, 0xa1, 0x98)));
    }

    #[test]
    fn an_unknown_pack_falls_back_to_harness_with_a_reason() {
        let theme = Theme::new(
            &UiPreferences {
                theme_id: "missing-pack".to_owned(),
                ..UiPreferences::default()
            },
            false,
        );
        assert_eq!(theme.status.pack_id, "harness");
        assert!(theme.status.trusted);
        assert_eq!(
            theme.status.errors,
            [
                "theme `missing-pack` is not installed".to_owned(),
                "using the `harness` theme instead".to_owned(),
            ]
        );
        assert_eq!(theme.background, convert(Rgba8::rgb(0xf1, 0xf2, 0xf3)));
    }

    #[test]
    fn new_installations_resolve_the_dsh_pack() {
        let preferences = UiPreferences::default();
        assert_eq!(preferences.theme_id, "dsh");
        let theme = Theme::new(&preferences, false);
        assert_eq!(theme.status.pack_id, "dsh");
        assert!(theme.status.errors.is_empty());
        assert_eq!(theme.background, convert(Rgba8::rgb(0xf6, 0xf8, 0xfb)));
        assert_eq!(theme.accent, convert(Rgba8::rgb(0x25, 0x7f, 0xd3)));
    }

    #[test]
    fn the_custom_pack_stays_selectable_after_a_builtin_pack() {
        let mut preferences = UiPreferences {
            theme_id: "custom".to_owned(),
            theme: ThemeMode::Light,
            ..UiPreferences::default()
        };
        assert_eq!(Theme::new(&preferences, false).status.pack_id, "custom");

        preferences.theme_id = "dsh".to_owned();
        assert_eq!(Theme::new(&preferences, false).status.pack_id, "dsh");

        preferences.theme_id = "custom".to_owned();
        let theme = Theme::new(&preferences, false);
        assert_eq!(theme.status.pack_id, "custom");
        assert_eq!(theme.background, convert(Rgba8::rgb(0xf1, 0xf2, 0xf3)));
    }

    #[test]
    fn accent_controls_follow_the_resolved_pack() {
        for (pack_id, expected) in [
            ("harness", true),
            ("dsh", true),
            ("solarized", false),
            ("high-contrast", false),
            ("custom", false),
        ] {
            let theme = Theme::new(
                &UiPreferences {
                    theme_id: pack_id.to_owned(),
                    ..UiPreferences::default()
                },
                false,
            );
            assert_eq!(theme.accent_families, expected, "{pack_id}");
        }
        // A missing pack falls back to Harness, whose accents do work.
        let theme = Theme::new(
            &UiPreferences {
                theme_id: "missing".to_owned(),
                ..UiPreferences::default()
            },
            false,
        );
        assert!(theme.accent_families);
    }

    #[test]
    fn the_status_note_names_the_requested_pack_and_the_fallback() {
        let theme = Theme::new(
            &UiPreferences {
                theme_id: "gone".to_owned(),
                ..UiPreferences::default()
            },
            false,
        );
        assert_eq!(
            theme.status.errors.join("; "),
            "theme `gone` is not installed; using the `harness` theme instead"
        );
    }

    #[test]
    fn a_custom_theme_keeps_the_rendering_the_custom_editor_produced() {
        let theme = Theme::new(
            &UiPreferences {
                theme_id: "custom".to_owned(),
                theme: ThemeMode::Dark,
                accent_scheme: AccentScheme::Blue,
                ..UiPreferences::default()
            },
            false,
        );
        // The legacy custom palette ignores the accent family preference.
        assert_eq!(theme.accent, convert(Rgba8::rgb(0xe9, 0x56, 0x18)));
        assert_eq!(theme.background, convert(Rgba8::rgb(0x17, 0x1a, 0x1f)));
        assert_eq!(theme.code_background, convert(Rgba8::rgb(0x22, 0x27, 0x2e)));
        assert_eq!(theme.status.pack_id, "custom");
    }
}

#[cfg(test)]
mod convert_tests {
    use super::*;

    #[test]
    fn convert_preserves_the_source_alpha() {
        let color = Rgba8 {
            r: 0x10,
            g: 0x20,
            b: 0x30,
            a: 0x80,
        };
        let converted = convert(color);
        assert!((converted.a - (0x80 as f32 / 255.0)).abs() < 1e-6);
    }
}
