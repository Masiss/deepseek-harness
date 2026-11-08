//! Shared element helpers for the gpui shell.

use gpui::{div, prelude::*, px, AnyElement, AnyView, App, Div, IntoElement, SharedString, Window};

use super::theme::Theme;

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

/// A compact button with primary, danger, and enabled variants.
pub(crate) struct Button {
    label: SharedString,
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
            label: label.into(),
            theme: theme.clone(),
            primary: false,
            danger: false,
            enabled: true,
            padding_x: theme.sp(9.0),
            padding_y: theme.sp(3.0),
            min_height: theme.control_height(),
        }
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
            .child(self.label);
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
