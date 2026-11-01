//! Design tokens for the gpui shell, resolved from the persisted preferences.

use crate::ui_preferences::{self, UiDensity, UiPreferences};
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
    pub radius: f32,
    pub scale: f32,
    pub compact: bool,
    pub sans: SharedString,
    pub mono: SharedString,
    pub fallbacks: Vec<String>,
}

impl Theme {
    /// Resolves a palette for the active light/dark scheme.
    pub(crate) fn new(preferences: &UiPreferences, dark: bool) -> Self {
        let palette = ui_preferences::palette(preferences.theme_pack, dark, &preferences.custom_theme);
        let (background, surface, code_background, text, muted, accent, accent_text, selection, border, warn, error) =
            match palette {
                Some(palette) => (
                    convert(palette.panel),
                    convert(palette.widget),
                    convert(palette.extreme),
                    convert(palette.text),
                    convert(palette.muted),
                    convert(palette.accent),
                    convert(palette.accent_text),
                    convert(palette.selection),
                    convert(palette.stroke),
                    convert(palette.warn),
                    convert(palette.error),
                ),
                None => default_palette(dark),
            };
        let surface_raised = blend(surface, text, if dark { 0.06 } else { 0.04 });
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
            background,
            surface,
            surface_raised,
            code_background,
            text,
            muted,
            accent,
            accent_text,
            selection,
            border,
            warn,
            error,
            radius: preferences.radius,
            scale: preferences.ui_scale,
            compact: preferences.density == UiDensity::Compact,
            sans: font_name(&preferences.fonts.proportional),
            mono: font_name(&preferences.fonts.monospace),
            fallbacks,
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

#[cfg(test)]
use crate::ui_preferences::ThemePack;

fn convert(color: ui_preferences::Rgba8) -> Hsla {
    let ui_preferences::Rgba8 { r, g, b, a } = color;
    gpui::rgba(u32::from_be_bytes([r, g, b, a])).into()
}

fn blend(base: Hsla, tint: Hsla, amount: f32) -> Hsla {
    let mut out = base;
    out.h = tint.h;
    out.s = (base.s + (tint.s - base.s) * amount).clamp(0.0, 1.0);
    out.l = (base.l + (tint.l - base.l) * amount).clamp(0.0, 1.0);
    out
}

fn default_palette(
    dark: bool,
) -> (
    Hsla,
    Hsla,
    Hsla,
    Hsla,
    Hsla,
    Hsla,
    Hsla,
    Hsla,
    Hsla,
    Hsla,
    Hsla,
) {
    if dark {
        (
            convert(ui_preferences::Rgba8::rgb(0x17, 0x1a, 0x1f)),
            convert(ui_preferences::Rgba8::rgb(0x22, 0x27, 0x2e)),
            convert(ui_preferences::Rgba8::rgb(0x12, 0x14, 0x18)),
            convert(ui_preferences::Rgba8::rgb(0xee, 0xf1, 0xf4)),
            convert(ui_preferences::Rgba8::rgb(0x92, 0x9b, 0xa6)),
            convert(ui_preferences::Rgba8::rgb(0xe9, 0x56, 0x18)),
            convert(ui_preferences::Rgba8::rgb(0xff, 0x9b, 0x70)),
            convert(ui_preferences::Rgba8::rgb(0x3b, 0x2a, 0x23)),
            convert(ui_preferences::Rgba8::rgb(0x35, 0x3d, 0x47)),
            convert(ui_preferences::Rgba8::rgb(0xe7, 0xc9, 0x79)),
            convert(ui_preferences::Rgba8::rgb(0xff, 0x9b, 0x70)),
        )
    } else {
        (
            convert(ui_preferences::Rgba8::rgb(0xf1, 0xf2, 0xf3)),
            convert(ui_preferences::Rgba8::WHITE),
            convert(ui_preferences::Rgba8::rgb(0xfa, 0xfa, 0xfa)),
            convert(ui_preferences::Rgba8::rgb(0x20, 0x20, 0x1f)),
            convert(ui_preferences::Rgba8::rgb(0x70, 0x70, 0x6b)),
            convert(ui_preferences::Rgba8::rgb(0xe9, 0x56, 0x18)),
            convert(ui_preferences::Rgba8::rgb(0xa8, 0x3b, 0x10)),
            convert(ui_preferences::Rgba8::rgb(0xff, 0xf1, 0xe9)),
            convert(ui_preferences::Rgba8::rgb(0xe6, 0xe6, 0xe3)),
            convert(ui_preferences::Rgba8::rgb(0x79, 0x5b, 0x1a)),
            convert(ui_preferences::Rgba8::rgb(0x9c, 0x39, 0x16)),
        )
    }
}

/// Default theme used by tests that do not exercise preferences.
#[cfg(test)]
pub(crate) fn test_theme(dark: bool) -> Theme {
    let mut preferences = UiPreferences::default();
    preferences.theme_pack = ThemePack::Default;
    Theme::new(&preferences, dark)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_sp_scales_layout_by_persisted_scale() {
        let mut preferences = UiPreferences::default();
        preferences.ui_scale = 2.0;
        preferences.radius = 4.0;
        let theme = Theme::new(&preferences, false);
        assert_eq!(theme.scale, 2.0);
        assert_eq!(theme.sp(10.), px(20.));
        assert_eq!(theme.corner(), px(8.));
    }

    #[test]
    fn theme_metrics_follow_density() {
        let mut compact = UiPreferences::default();
        compact.density = UiDensity::Compact;
        let theme = Theme::new(&compact, false);
        assert_eq!(theme.gap(), px(4.));
        assert_eq!(theme.control_height(), px(22.));

        let regular = UiPreferences::default();
        let theme = Theme::new(&regular, false);
        assert_eq!(theme.gap(), px(8.));
        assert_eq!(theme.control_height(), px(26.));
    }

    #[test]
    fn theme_dark_tracks_persisted_scheme() {
        assert!(test_theme(true).dark);
        assert!(!test_theme(false).dark);
    }
}

#[cfg(test)]
mod palette_tests {
    use super::*;

    fn sample(h: f32, s: f32, l: f32) -> Hsla {
        Hsla { h, s, l, a: 1.0 }
    }

    #[test]
    fn blend_keeps_the_base_saturation_at_zero_amount() {
        let base = sample(0.1, 0.2, 0.3);
        let tint = sample(0.9, 0.8, 0.7);
        let out = blend(base, tint, 0.0);
        assert_eq!(out.h, tint.h);
        assert!((out.s - base.s).abs() < 1e-6);
        assert!((out.l - base.l).abs() < 1e-6);
    }

    #[test]
    fn blend_reaches_the_tint_saturation_at_full_amount() {
        let base = sample(0.1, 0.2, 0.3);
        let tint = sample(0.9, 0.8, 0.7);
        let out = blend(base, tint, 1.0);
        assert!((out.s - tint.s).abs() < 1e-6);
        assert!((out.l - tint.l).abs() < 1e-6);
    }

    #[test]
    fn blend_clamps_out_of_range_values() {
        let base = sample(0.0, -0.5, 1.5);
        let tint = sample(0.0, -1.0, 2.0);
        let out = blend(base, tint, 1.0);
        assert!((0.0..=1.0).contains(&out.s));
        assert!((0.0..=1.0).contains(&out.l));
    }

    #[test]
    fn convert_preserves_the_source_alpha() {
        let color = ui_preferences::Rgba8 { r: 0x10, g: 0x20, b: 0x30, a: 0x80 };
        let converted = convert(color);
        assert!((converted.a - (0x80 as f32 / 255.0)).abs() < 1e-6);
    }
}
