//! Colour primitives and the semantic role vocabulary shared by theme packs.

/// One 8-bit sRGB colour with an alpha channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgba8 {
    /// Red channel.
    pub r: u8,
    /// Green channel.
    pub g: u8,
    /// Blue channel.
    pub b: u8,
    /// Alpha channel; 255 is opaque.
    pub a: u8,
}

impl Rgba8 {
    /// Opaque white.
    pub(crate) const WHITE: Self = Self::rgb(255, 255, 255);

    /// Builds an opaque colour.
    pub(crate) const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    /// Composites this colour over an opaque background.
    pub(crate) fn over(self, background: Self) -> Self {
        if self.a == 255 {
            return self;
        }
        let alpha = f64::from(self.a) / 255.0;
        let mix = |foreground: u8, backdrop: u8| {
            (f64::from(foreground) * alpha + f64::from(backdrop) * (1.0 - alpha)).round() as u8
        };
        Self {
            r: mix(self.r, background.r),
            g: mix(self.g, background.g),
            b: mix(self.b, background.b),
            a: 255,
        }
    }

    /// WCAG 2.1 relative luminance, from 0 for black to 1 for white.
    pub(crate) fn relative_luminance(self) -> f64 {
        let channel = |value: u8| {
            let value = f64::from(value) / 255.0;
            if value <= 0.039_28 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(self.r) + 0.7152 * channel(self.g) + 0.0722 * channel(self.b)
    }
}

/// One semantic colour role every pack defines for both schemes.
///
/// Roles name the design-system vocabulary; components resolve their colours
/// through a role, so a pack can restyle the shell without touching layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// Primary workspace background.
    Canvas,
    /// Top and bottom shell bars.
    Chrome,
    /// Cards and dialogs.
    Surface,
    /// Side panels and code backgrounds.
    SurfaceDeep,
    /// Hover, composer, and popover fill.
    SurfaceRaised,
    /// Main separators and control outlines.
    Border,
    /// Faint separator between adjacent panel fills.
    BorderFaint,
    /// Primary text.
    Text,
    /// Secondary text and hints.
    TextSecondary,
    /// Small readable metadata.
    TextTertiary,
    /// Action fill, focus ring, and selected outline.
    Accent,
    /// Label colour on an accent fill.
    AccentText,
    /// Selected text background.
    Selection,
    /// Completed or healthy state.
    Success,
    /// Failed or destructive state.
    Danger,
    /// Attention or caution state.
    Warning,
}

impl Role {
    /// The role name used in pack files and contrast reports.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Canvas => "canvas",
            Self::Chrome => "chrome",
            Self::Surface => "surface",
            Self::SurfaceDeep => "surfaceDeep",
            Self::SurfaceRaised => "surfaceRaised",
            Self::Border => "border",
            Self::BorderFaint => "borderFaint",
            Self::Text => "text",
            Self::TextSecondary => "textSecondary",
            Self::TextTertiary => "textTertiary",
            Self::Accent => "accent",
            Self::AccentText => "accentText",
            Self::Selection => "selection",
            Self::Success => "success",
            Self::Danger => "danger",
            Self::Warning => "warning",
        }
    }
}

/// One of the two colour schemes every pack defines.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scheme {
    /// Light surfaces with dark text.
    Light,
    /// Dark surfaces with light text.
    Dark,
}

impl Scheme {
    /// The scheme for a resolved light/dark flag.
    pub(crate) fn of(dark: bool) -> Self {
        if dark {
            Self::Dark
        } else {
            Self::Light
        }
    }

    /// The scheme name used in pack files and contrast reports.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }

    /// Whether this is the dark scheme.
    pub(crate) fn is_dark(self) -> bool {
        matches!(self, Self::Dark)
    }
}

/// Blends `base` toward `tint` in HSL, adopting the tint's hue.
///
/// The v5 custom theme derived its raised surface this way, so migrated custom
/// packs keep the exact fill they rendered before theme packs existed.
pub(crate) fn blend_hsl(base: Rgba8, tint: Rgba8, amount: f32) -> Rgba8 {
    if amount <= 0.0 {
        return base;
    }
    if amount >= 1.0 {
        return tint;
    }
    let (_, base_saturation, base_lightness) = hsl(base);
    let (tint_hue, tint_saturation, tint_lightness) = hsl(tint);
    let saturation =
        (base_saturation + (tint_saturation - base_saturation) * amount).clamp(0.0, 1.0);
    let lightness = (base_lightness + (tint_lightness - base_lightness) * amount).clamp(0.0, 1.0);
    rgb_from_hsl(tint_hue, saturation, lightness)
}

fn hsl(color: Rgba8) -> (f32, f32, f32) {
    let red = f32::from(color.r) / 255.0;
    let green = f32::from(color.g) / 255.0;
    let blue = f32::from(color.b) / 255.0;
    let max = red.max(green.max(blue));
    let min = red.min(green.min(blue));
    let delta = max - min;
    let lightness = (max + min) / 2.0;
    let saturation = if lightness == 0.0 || lightness == 1.0 {
        0.0
    } else if lightness < 0.5 {
        delta / (2.0 * lightness)
    } else {
        delta / (2.0 - 2.0 * lightness)
    };
    let hue = if delta == 0.0 {
        0.0
    } else if max == red {
        ((green - blue) / delta).rem_euclid(6.0) / 6.0
    } else if max == green {
        ((blue - red) / delta + 2.0) / 6.0
    } else {
        ((red - green) / delta + 4.0) / 6.0
    };
    (hue, saturation, lightness)
}

fn rgb_from_hsl(hue: f32, saturation: f32, lightness: f32) -> Rgba8 {
    let chroma = (1.0 - (2.0 * lightness - 1.0).abs()) * saturation;
    let second = chroma * (1.0 - ((hue * 6.0) % 2.0 - 1.0).abs());
    let offset = lightness - chroma / 2.0;
    let high = chroma + offset;
    let middle = second + offset;
    let (red, green, blue) = match (hue * 6.0).floor() as i32 {
        0 | 6 => (high, middle, offset),
        1 => (middle, high, offset),
        2 => (offset, high, middle),
        3 => (offset, middle, high),
        4 => (middle, offset, high),
        _ => (high, offset, middle),
    };
    let channel = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    Rgba8::rgb(channel(red), channel(green), channel(blue))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opaque_colours_composite_to_themselves() {
        let color = Rgba8::rgb(0x12, 0x34, 0x56);
        assert_eq!(color.over(Rgba8::WHITE), color);
    }

    #[test]
    fn translucent_colours_mix_toward_the_background() {
        let color = Rgba8 {
            r: 255,
            g: 255,
            b: 255,
            a: 128,
        };
        let mixed = color.over(Rgba8::rgb(0, 0, 0));
        assert_eq!((mixed.r, mixed.g, mixed.b, mixed.a), (128, 128, 128, 255));
    }

    #[test]
    fn luminance_runs_from_black_to_white() {
        assert_eq!(Rgba8::rgb(0, 0, 0).relative_luminance(), 0.0);
        assert!((Rgba8::WHITE.relative_luminance() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn blend_moves_by_the_requested_amount_and_takes_the_tint_hue() {
        let base = Rgba8::rgb(0x22, 0x27, 0x2e);
        let tint = Rgba8::rgb(0xee, 0xf1, 0xf4);
        assert_eq!(blend_hsl(base, tint, 0.0), base);
        assert_eq!(blend_hsl(base, tint, 1.0), tint);
        let raised = blend_hsl(base, tint, 0.06);
        assert_ne!(raised, base);
        assert!(raised.relative_luminance() > base.relative_luminance());
    }
}
