//! Shared element helpers for the gpui shell.

use gpui::{
    div, prelude::*, px, AnyElement, AnyView, App, Div, Hsla, IntoElement, Pixels, SharedString,
    Svg, Window,
};

use super::theme::Theme;
use crate::assets::Icon;

/// Draws one icon of the embedded outline set.
///
/// @param name — icon to draw.
/// @param size — edge length, already scaled by the caller.
/// @param color — stroke colour; every icon SVG strokes `currentColor`.
/// @returns the SVG element, exposed as the `icon-<name>` test selector.
pub(crate) fn icon(name: Icon, size: Pixels, color: Hsla) -> Svg {
    gpui::svg()
        .path(name.path())
        .size(size)
        .text_color(color)
        .flex_none()
        .debug_selector(move || format!("icon-{}", name.name()))
}

/// Wraps an element in a padded, bordered card.
pub(crate) fn card(theme: &Theme, child: impl IntoElement) -> Div {
    div()
        .w_full()
        .p(theme.sp(10.0))
        .rounded(theme.corner())
        .border_1()
        .border_color(theme.border)
        .bg(theme.surface)
        .child(child)
}

/// What a [`Button`] shows.
enum ButtonContent {
    /// A text label.
    Text(SharedString),
    /// One icon of the embedded outline set.
    Icon(Icon),
}

/// A compact button with primary, danger, and enabled variants.
pub(crate) struct Button {
    content: ButtonContent,
    leading: Option<Icon>,
    trailing: Option<Icon>,
    theme: Theme,
    primary: bool,
    danger: bool,
    enabled: bool,
    padding_x: gpui::Pixels,
    padding_y: gpui::Pixels,
    min_height: gpui::Pixels,
}

impl Button {
    /// Builds a neutral button.
    pub(crate) fn new(theme: &Theme, label: impl Into<SharedString>) -> Self {
        Self {
            content: ButtonContent::Text(label.into()),
            leading: None,
            trailing: None,
            theme: theme.clone(),
            primary: false,
            danger: false,
            enabled: true,
            padding_x: theme.sp(9.0),
            padding_y: theme.sp(3.0),
            min_height: theme.control_height(),
        }
    }

    /// Builds a neutral button that shows only an icon.
    ///
    /// @param theme — frame palette and metrics.
    /// @param name — icon drawn at the button's foreground colour.
    /// @returns a neutral button, for the rest of its builder chain.
    pub(crate) fn icon_only(theme: &Theme, name: Icon) -> Self {
        Self {
            content: ButtonContent::Icon(name),
            leading: None,
            trailing: None,
            theme: theme.clone(),
            primary: false,
            danger: false,
            enabled: true,
            padding_x: theme.sp(9.0),
            padding_y: theme.sp(3.0),
            min_height: theme.control_height(),
        }
    }

    /// Draws an icon before the label, sized to the button's text.
    ///
    /// @param name — icon drawn at the button's foreground colour.
    /// @returns the button, for the rest of its builder chain.
    pub(crate) fn leading_icon(mut self, name: Icon) -> Self {
        self.leading = Some(name);
        self
    }

    /// Draws an icon after the label, sized to the button's text.
    ///
    /// @param name — icon drawn at the button's foreground colour.
    /// @returns the button, for the rest of its builder chain.
    pub(crate) fn trailing_icon(mut self, name: Icon) -> Self {
        self.trailing = Some(name);
        self
    }

    /// Uses the accent fill for the primary action.
    pub(crate) fn primary(mut self) -> Self {
        self.primary = true;
        self
    }

    /// Uses the error color for destructive actions.
    pub(crate) fn danger(mut self) -> Self {
        self.danger = true;
        self
    }

    /// Disables interaction and dims the label.
    pub(crate) fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Builds the element; the caller attaches the click handler.
    pub(crate) fn build(self) -> Div {
        let theme = self.theme;
        let (background, foreground) = if self.primary {
            (theme.accent, theme.accent_text)
        } else if self.danger {
            (theme.error, theme.background)
        } else {
            (theme.surface, theme.text)
        };
        let mut button = div()
            .flex()
            .items_center()
            .justify_center()
            .gap(px(4.))
            .px(self.padding_x)
            .py(self.padding_y)
            .min_h(self.min_height)
            .rounded(theme.corner())
            .border_1()
            .border_color(theme.border)
            .bg(background)
            .text_color(foreground)
            .text_size(theme.fs(11.5))
            .children(
                self.leading
                    .map(|name| icon(name, theme.sp(13.), foreground).into_any_element()),
            )
            .child(match self.content {
                ButtonContent::Text(label) => label.into_any_element(),
                ButtonContent::Icon(name) => {
                    icon(name, theme.sp(15.), foreground).into_any_element()
                }
            })
            .children(
                self.trailing
                    .map(|name| icon(name, theme.sp(13.), foreground).into_any_element()),
            );
        if self.enabled {
            button = button
                .cursor_pointer()
                .hover(|style| style.bg(theme.surface_raised));
        } else {
            button = button.opacity(0.45);
        }
        button
    }
}

/// A section label above a group of controls.
pub(crate) fn section_label(theme: &Theme, text: impl Into<SharedString>) -> AnyElement {
    div()
        .px(theme.sp(10.0))
        .pt(theme.sp(8.0))
        .pb(theme.sp(3.0))
        .text_size(theme.fs(9.5))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(theme.muted)
        .child(text.into())
        .into_any_element()
}

/// A one-pixel separator.
pub(crate) fn separator(theme: &Theme) -> Div {
    div().w_full().h(px(1.)).bg(theme.border)
}

/// The one-line explanation a disabled control shows as its tooltip.
pub(crate) struct HintView {
    theme: Theme,
    text: SharedString,
}

impl HintView {
    /// Builds the tooltip callback for one control.
    ///
    /// @param theme — the frame's snapshot, cloned into the tooltip view.
    /// @param text — the explanation shown while the control is hovered.
    /// @returns a tooltip builder for [`gpui::InteractiveElement::tooltip`].
    pub(crate) fn tooltip(
        theme: &Theme,
        text: impl Into<SharedString>,
    ) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
        let theme = theme.clone();
        let text = text.into();
        move |_window, cx| {
            cx.new(|_| Self {
                theme: theme.clone(),
                text: text.clone(),
            })
            .into()
        }
    }
}

impl gpui::Render for HintView {
    fn render(&mut self, _window: &mut Window, _cx: &mut gpui::Context<Self>) -> impl IntoElement {
        div()
            .px(self.theme.sp(8.))
            .py(self.theme.sp(4.))
            .rounded(self.theme.corner())
            .border_1()
            .border_color(self.theme.border)
            .bg(self.theme.surface_raised)
            .text_size(self.theme.fs(10.5))
            .text_color(self.theme.muted)
            .child(self.text.clone())
    }
}
