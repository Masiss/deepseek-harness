//! Contrast checks for theme packs.
//!
//! The hard set protects a pack that does not ship with the app: body text, the
//! accent fill, and the label on an accent fill. A 1px hairline separator is not
//! a WCAG non-text-contrast boundary, so borders are measured for the report
//! only and never reject a pack. Trusted packs report every miss as a warning so
//! a palette change stays visible in the tests that assert the warning list.

use std::fmt;

use super::pack::SchemeColors;
use super::tokens::{Rgba8, Role};

/// Contrast ratio required for body text.
pub(crate) const BODY_TEXT_CONTRAST: f64 = 4.5;

/// Contrast ratio required for an accent fill.
pub(crate) const ACCENT_CONTRAST: f64 = 3.0;

/// One role pair that misses its required contrast ratio.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ContrastIssue {
    /// Foreground role name.
    pub(crate) foreground: &'static str,
    /// Background role name.
    pub(crate) background: &'static str,
    /// Measured ratio.
    pub(crate) ratio: f64,
    /// Ratio the pair must reach.
    pub(crate) required: f64,
}

impl fmt::Display for ContrastIssue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} on {} is {:.2}:1, needs {:.1}:1",
            self.foreground, self.background, self.ratio, self.required
        )
    }
}

/// Contrast misses for one scheme, split by what the caller may do about them.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct ContrastReport {
    /// Pairs an untrusted pack must meet before it renders.
    pub(crate) failures: Vec<ContrastIssue>,
    /// Pairs reported for any pack without blocking it.
    pub(crate) warnings: Vec<ContrastIssue>,
}

/// Measures one scheme against the required pairs.
///
/// @param colors — the parsed roles of the active scheme.
/// @returns the hard-gate failures and the warning-only misses.
pub(crate) fn report(colors: &SchemeColors) -> ContrastReport {
    let mut failures = Vec::new();
    let mut warnings = Vec::new();
    let body_backgrounds = [
        (Role::Canvas, colors.canvas),
        (Role::Surface, colors.surface),
        (Role::SurfaceDeep, colors.surface_deep),
        (Role::SurfaceRaised, colors.surface_raised),
    ];
    for foreground in [Role::Text, Role::TextSecondary] {
        for (background, color) in body_backgrounds {
            require(
                &mut failures,
                colors.color(foreground),
                foreground,
                color,
                background,
                BODY_TEXT_CONTRAST,
            );
        }
    }
    for (background, color) in body_backgrounds {
        require(
            &mut failures,
            colors.accent,
            Role::Accent,
            color,
            background,
            ACCENT_CONTRAST,
        );
    }
    require(
        &mut failures,
        colors.accent_text,
        Role::AccentText,
        colors.accent,
        Role::Accent,
        BODY_TEXT_CONTRAST,
    );

    for (background, color) in body_backgrounds {
        require(
            &mut warnings,
            colors.text_tertiary,
            Role::TextTertiary,
            color,
            background,
            BODY_TEXT_CONTRAST,
        );
        require(
            &mut warnings,
            colors.border,
            Role::Border,
            color,
            background,
            ACCENT_CONTRAST,
        );
        require(
            &mut warnings,
            colors.border_faint,
            Role::BorderFaint,
            color,
            background,
            ACCENT_CONTRAST,
        );
    }
    for foreground in [Role::Success, Role::Danger, Role::Warning] {
        for (background, color) in [
            (Role::Canvas, colors.canvas),
            (Role::Surface, colors.surface),
        ] {
            require(
                &mut warnings,
                colors.color(foreground),
                foreground,
                color,
                background,
                BODY_TEXT_CONTRAST,
            );
        }
    }
    ContrastReport { failures, warnings }
}

/// WCAG 2.1 contrast ratio between a foreground and an opaque background.
///
/// A translucent foreground is composited over the background first; backgrounds
/// are treated as opaque.
pub(crate) fn ratio(foreground: Rgba8, background: Rgba8) -> f64 {
    let foreground = foreground.over(background);
    let (lighter, darker) = if foreground.relative_luminance() >= background.relative_luminance() {
        (
            foreground.relative_luminance(),
            background.relative_luminance(),
        )
    } else {
        (
            background.relative_luminance(),
            foreground.relative_luminance(),
        )
    };
    (lighter + 0.05) / (darker + 0.05)
}

fn require(
    issues: &mut Vec<ContrastIssue>,
    foreground: Rgba8,
    foreground_role: Role,
    background: Rgba8,
    background_role: Role,
    required: f64,
) {
    let ratio = ratio(foreground, background);
    if ratio < required {
        issues.push(ContrastIssue {
            foreground: foreground_role.name(),
            background: background_role.name(),
            ratio,
            required,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn colors() -> SchemeColors {
        SchemeColors {
            canvas: Rgba8::rgb(0, 0, 0),
            chrome: Rgba8::rgb(0, 0, 0),
            surface: Rgba8::rgb(0x11, 0x11, 0x11),
            surface_deep: Rgba8::rgb(0x0b, 0x0b, 0x0b),
            surface_raised: Rgba8::rgb(0x1a, 0x1a, 0x1a),
            border: Rgba8::rgb(0x33, 0x33, 0x33),
            border_faint: Rgba8::rgb(0x2a, 0x2a, 0x2a),
            text: Rgba8::WHITE,
            text_secondary: Rgba8::rgb(0xcc, 0xcc, 0xcc),
            text_tertiary: Rgba8::rgb(0xbb, 0xbb, 0xbb),
            accent: Rgba8::rgb(0x4a, 0x99, 0xf7),
            accent_text: Rgba8::rgb(0x0b, 0x1a, 0x2a),
            selection: Rgba8::rgb(0x18, 0x32, 0x4b),
            success: Rgba8::rgb(0x53, 0xd4, 0x9b),
            danger: Rgba8::rgb(0xf1, 0x7c, 0x80),
            warning: Rgba8::rgb(0xe9, 0xb4, 0x6b),
        }
    }

    #[test]
    fn accepts_body_text_and_accent_pairs_above_their_ratio() {
        let report = report(&colors());
        assert!(report.failures.is_empty(), "{:?}", report.failures);
    }

    #[test]
    fn rejects_body_text_below_four_and_a_half_to_one() {
        let mut colors = colors();
        colors.text = Rgba8::rgb(0x77, 0x77, 0x77);
        colors.text_secondary = Rgba8::rgb(0x76, 0x76, 0x76);
        let report = report(&colors);
        // #777777 on #111111 is 3.85:1, so both body roles miss every background.
        assert!(report
            .failures
            .iter()
            .any(|issue| issue.foreground == "text" && issue.background == "surface"));
        assert!(report
            .failures
            .iter()
            .any(|issue| issue.foreground == "textSecondary"));
        assert!(report
            .failures
            .iter()
            .all(|issue| issue.required == BODY_TEXT_CONTRAST));
    }

    #[test]
    fn rejects_an_accent_fill_below_three_to_one_and_a_dark_label_on_it() {
        let mut colors = colors();
        colors.accent = Rgba8::rgb(0x35, 0x35, 0x35);
        let report = report(&colors);
        assert!(report
            .failures
            .iter()
            .any(|issue| issue.foreground == "accent" && issue.required == ACCENT_CONTRAST));
        assert!(report
            .failures
            .iter()
            .any(|issue| issue.foreground == "accentText" && issue.background == "accent"));
    }

    #[test]
    fn borders_are_reported_but_never_fail_the_gate() {
        // #333333 on #111111 is 1.51:1, far below the 3:1 boundary figure.
        let report = report(&colors());
        assert!(report
            .warnings
            .iter()
            .any(|issue| issue.foreground == "border" && issue.background == "surface"));
        assert!(!report
            .failures
            .iter()
            .any(|issue| issue.foreground == "border"));
    }

    #[test]
    fn ratios_follow_the_wcag_formula_and_composite_translucent_foregrounds() {
        assert!((ratio(Rgba8::WHITE, Rgba8::rgb(0, 0, 0)) - 21.0).abs() < 1e-9);
        assert!((ratio(Rgba8::rgb(0x76, 0x76, 0x76), Rgba8::WHITE) - 4.54).abs() < 0.01);
        assert!((ratio(Rgba8::rgb(0x77, 0x77, 0x77), Rgba8::WHITE) - 4.48).abs() < 0.01);
        assert!(
            (ratio(Rgba8::rgb(0x80, 0x80, 0x80), Rgba8::rgb(0, 0, 0))
                - ratio(
                    Rgba8 {
                        r: 0xff,
                        g: 0xff,
                        b: 0xff,
                        a: 0x80
                    },
                    Rgba8::rgb(0, 0, 0)
                ))
            .abs()
                < 0.05
        );
    }

    #[test]
    fn issues_name_the_role_pair_and_the_measured_ratio() {
        let issue = ContrastIssue {
            foreground: "textSecondary",
            background: "surfaceRaised",
            ratio: 4.444,
            required: 4.5,
        };
        assert_eq!(
            issue.to_string(),
            "textSecondary on surfaceRaised is 4.44:1, needs 4.5:1"
        );
    }
}
