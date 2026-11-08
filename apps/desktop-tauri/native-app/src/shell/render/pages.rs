//! Full-pane surfaces that replace the conversation (Plugins, Skills,
//! Marketplace, Goals, and the Document preview) and the Preferences overlay.

use gpui::{div, prelude::*, StatefulInteractiveElement, px, AnyElement, Context};

use crate::assets::Icon;
use crate::layout;
use crate::theme::{
    resolve, ContrastStatus, ThemeRequest, ThemeSource, CUSTOM_PACK_ID,
};
use crate::transcript::RowKind;
use crate::ui_preferences::{AccentScheme, KeymapPreset, ThemeMode, UiDensity};
use crate::shell::app::{ActiveView, DesktopApp, GoalAction, PreferencesSection};
use crate::shell::theme::Theme;
use super::widgets;

/// Returns the active page surface, or `None` while the conversation is shown.
///
/// Preferences is an overlay, so it never replaces the conversation here.
pub(crate) fn active_page(
    app: &mut DesktopApp,
    theme: &Theme,
    cx: &mut Context<DesktopApp>,
) -> Option<AnyElement> {
    let page = if app.show_goals {
        goals(app, theme, cx)
    } else if app.show_plugins {
        plugins(app, theme, cx)
    } else if app.show_market {
        marketplace(app, theme, cx)
    } else if app.show_skills {
        skills(app, theme, cx)
    } else if app.show_document {
        document(app, theme, cx)
    } else {
        return None;
    };
    Some(
        div()
            .flex()
            .flex_col()
            .size_full()
            .min_h(px(0.))
            .child(page_header(app, theme, cx))
            .child(page)
            .into_any_element(),
    )
}

fn page_header(
    app: &mut DesktopApp,
    theme: &Theme,
    cx: &mut Context<DesktopApp>,
) -> AnyElement {
    let (title, subtitle) = match app.active_view {
        ActiveView::Settings => (
            "Preferences",
            if theme.dark { "Dark interface preferences" } else { "Light interface preferences" },
        ),
        ActiveView::Plugins => (
            "Plugins",
            "Extensions configured by the active profile and managed through the Host.",
        ),
        ActiveView::Skills => ("Skills", "Instructions available to this Session."),
        ActiveView::Marketplace => (
            "Marketplace",
            "Search configured indexes and install profile extensions.",
        ),
        ActiveView::Goals => (
            "Session goal",
            "Keep the task, completion state, and recent plan activity together.",
        ),
        ActiveView::Document => (
            "Document preview",
            "Files scoped to this Session’s workspace; text documents can be edited and saved back to the Host.",
        ),
        ActiveView::Sessions | ActiveView::Explorer => ("Workspace", ""),
    };
    div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .px(theme.sp(16.))
        .py(theme.sp(10.))
        .border_b_1()
        .border_color(theme.border)
        .bg(theme.surface)
        .child(
            div()
                .flex()
                .flex_col()
                .min_w(px(0.))
                .child(
                    div()
                        .text_size(theme.fs(15.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child(title),
                )
                .child(
                    div()
                        .text_size(theme.fs(10.5))
                        .text_color(theme.muted)
                        .child(subtitle),
                ),
        )
        .child(
            widgets::Button::new(theme, "Close")
                .build()
                .id("page-close")
                .on_click(cx.listener(|app, _, _, cx| {
                    app.switch_view(ActiveView::Sessions);
                    cx.notify();
                })),
        )
        .into_any_element()
}

fn body(theme: &Theme) -> gpui::Stateful<gpui::Div> {
    div()
        .id("page-body")
        .flex()
        .flex_col()
        .gap(theme.sp(8.))
        .flex_1()
        .min_h(px(0.))
        .w_full()
        .px(theme.sp(16.))
        .py(theme.sp(12.))
        .overflow_y_scroll()
}

fn section(theme: &Theme, title: &str) -> AnyElement {
    div()
        .pt(theme.sp(4.))
        .text_size(theme.fs(11.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .child(title.to_owned())
        .into_any_element()
}

fn line(theme: &Theme, text: impl Into<String>) -> AnyElement {
    div()
        .text_size(theme.fs(11.))
        .text_color(theme.muted)
        .child(text.into())
        .into_any_element()
}

/// Builds one accent-family button.
///
/// A pack that defines no accent families keeps its own accent, so its buttons
/// are disabled and explain why instead of silently ignoring the preference.
fn accent_button(
    theme: &Theme,
    label: &'static str,
    id: &'static str,
    scheme: AccentScheme,
    selected: bool,
    available: bool,
    cx: &mut Context<DesktopApp>,
) -> gpui::Stateful<gpui::Div> {
    widgets::Button::new(theme, label)
        .enabled(available && !selected)
        .build()
        .id(id)
        .when(!available, |button| {
            button.tooltip(widgets::HintView::tooltip(
                theme,
                "This theme pack defines no accent families. Choose DSH or Harness to use Accent.",
            ))
        })
        .when(available, |button| {
            button.on_click(cx.listener(move |app, _, window, cx| {
                app.update_preferences(window, |preferences| {
                    preferences.accent_scheme = scheme;
                });
                cx.notify();
            }))
        })
}

/// Draws the Preferences overlay over the unchanged shell.
///
/// The window is capped by the viewport insets from the V2 reference, and its
/// section content scrolls inside the window so the shell keeps its geometry.
///
/// @param app - Shell state, including the selected section and settings data.
/// @param theme - Frame theme resolved from the active preferences.
/// @param viewport_width - Window width the overlay must fit inside.
/// @param viewport_height - Window height the overlay must fit inside.
/// @param cx - Context used for the overlay's click handlers.
/// @returns The overlay layer painted above the shell.
pub(crate) fn preferences_overlay(
    app: &mut DesktopApp,
    theme: &Theme,
    viewport_width: f32,
    viewport_height: f32,
    cx: &mut Context<DesktopApp>,
) -> AnyElement {
    let scale = theme.scale;
    let width = (910. * scale).min((viewport_width - 60. * scale).max(240. * scale));
    let height = (680. * scale).min((viewport_height - 70. * scale).max(400. * scale));
    let focus = app.preferences_focus.clone();
    let section = app.preferences_section;

    div()
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .p(theme.sp(15.))
        .child(
            div()
                .absolute()
                .inset_0()
                .id("preferences-backdrop")
                .debug_selector(|| "preferences-backdrop".to_owned())
                .bg(gpui::black().opacity(0.52))
                .on_click(cx.listener(|app, _, _, cx| {
                    app.close_preferences();
                    cx.notify();
                })),
        )
        .child(
            div()
                .id("preferences-window")
                .debug_selector(|| "preferences-window".to_owned())
                .key_context("Preferences")
                .track_focus(&focus)
                .on_click(cx.listener(|_, _, _, cx| cx.stop_propagation()))
                .flex()
                .flex_col()
                .w(px(width))
                .h(px(height))
                .overflow_hidden()
                .rounded(theme.corner())
                .border_1()
                .border_color(theme.border)
                .bg(theme.surface)
                .shadow_lg()
                .child(preferences_header(theme, cx))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .flex_1()
                        .min_h(px(0.))
                        .child(preferences_nav(theme, section, cx))
                        .child(preferences_main(app, theme, cx)),
                )
                .child(preferences_footer(theme)),
        )
        .into_any_element()
}

/// Title row with the overlay's close control.
fn preferences_header(theme: &Theme, cx: &mut Context<DesktopApp>) -> AnyElement {
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(theme.sp(10.))
        .h(theme.sp(47.))
        .px(theme.sp(16.))
        .border_b_1()
        .border_color(theme.border)
        .child(
            div()
                .text_size(theme.fs(13.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child("Preferences"),
        )
        .child(div().flex_1())
        .child(
            widgets::Button::new(theme, "Close")
                .build()
                .id("preferences-close")
                .debug_selector(|| "preferences-close".to_owned())
                .on_click(cx.listener(|app, _, _, cx| {
                    app.close_preferences();
                    cx.notify();
                })),
        )
        .into_any_element()
}

/// Left navigation column, grouped like the reference window.
fn preferences_nav(
    theme: &Theme,
    current: PreferencesSection,
    cx: &mut Context<DesktopApp>,
) -> AnyElement {
    div()
        .debug_selector(|| "preferences-nav".to_owned())
        .flex()
        .flex_col()
        .flex_none()
        .gap(theme.sp(2.))
        .w(theme.sp(198.))
        .p(theme.sp(9.))
        .bg(theme.code_background)
        .border_r_1()
        .border_color(theme.border)
        .child(preferences_nav_item(
            theme,
            PreferencesSection::General,
            "General",
            current,
            cx,
        ))
        .child(preferences_nav_item(
            theme,
            PreferencesSection::Appearance,
            "Appearance",
            current,
            cx,
        ))
        .child(preferences_nav_item(
            theme,
            PreferencesSection::Keymap,
            "Keymap",
            current,
            cx,
        ))
        .child(preferences_nav_caption(theme, "EXTEND"))
        .child(preferences_nav_item(
            theme,
            PreferencesSection::Extensions,
            "Extensions",
            current,
            cx,
        ))
        .child(preferences_nav_item(
            theme,
            PreferencesSection::Skills,
            "Skills",
            current,
            cx,
        ))
        .child(preferences_nav_item(
            theme,
            PreferencesSection::Marketplace,
            "Marketplace",
            current,
            cx,
        ))
        .child(preferences_nav_caption(theme, "MORE"))
        .child(preferences_nav_item(
            theme,
            PreferencesSection::About,
            "About",
            current,
            cx,
        ))
        .into_any_element()
}

/// One selectable row in the Preferences navigation column.
fn preferences_nav_item(
    theme: &Theme,
    section: PreferencesSection,
    label: &'static str,
    current: PreferencesSection,
    cx: &mut Context<DesktopApp>,
) -> AnyElement {
    let active = section == current;
    div()
        .flex()
        .items_center()
        .h(theme.sp(35.))
        .w_full()
        .px(theme.sp(11.))
        .rounded(theme.corner())
        .text_size(theme.fs(12.))
        .text_color(if active { theme.text } else { theme.muted })
        .when(active, |row| row.bg(theme.selection))
        .when(!active, |row| {
            row.cursor_pointer()
                .hover(|style| style.bg(theme.surface_raised))
        })
        .id(gpui::SharedString::from(format!("preferences-nav-{label}")))
        .debug_selector(move || format!("preferences-nav-{label}"))
        .child(label)
        .on_click(cx.listener(move |app, _, _, cx| {
            app.preferences_section = section;
            cx.notify();
        }))
        .into_any_element()
}

/// Group caption between navigation clusters.
fn preferences_nav_caption(theme: &Theme, text: &'static str) -> AnyElement {
    div()
        .px(theme.sp(11.))
        .pt(theme.sp(20.))
        .pb(theme.sp(5.))
        .text_size(theme.fs(9.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(theme.muted)
        .child(text)
        .into_any_element()
}

/// Footer with the storage note and the keyboard hint.
fn preferences_footer(theme: &Theme) -> AnyElement {
    div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .px(theme.sp(15.))
        .py(theme.sp(8.))
        .border_t_1()
        .border_color(theme.border)
        .child(
            div()
                .text_size(theme.fs(10.))
                .text_color(theme.muted)
                .child("Preferences are stored locally with this client."),
        )
        .child(
            div()
                .text_size(theme.fs(10.))
                .text_color(theme.muted)
                .child("Esc to close"),
        )
        .into_any_element()
}

/// Scrollable section body; only this pane scrolls, never the shell.
fn preferences_main(
    app: &mut DesktopApp,
    theme: &Theme,
    cx: &mut Context<DesktopApp>,
) -> AnyElement {
    let mut main = div()
        .id("preferences-main")
        .debug_selector(|| "preferences-main".to_owned())
        .flex()
        .flex_col()
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .px(theme.sp(32.))
        .py(theme.sp(26.))
        .overflow_y_scroll();
    match app.preferences_section {
        PreferencesSection::General => main = main.child(preferences_general(app, theme, cx)),
        PreferencesSection::Appearance => {
            main = main.child(preferences_appearance(app, theme, cx));
        }
        PreferencesSection::Keymap => main = main.child(preferences_keymap(app, theme, cx)),
        PreferencesSection::Extensions => {
            main = main.child(preferences_page_link(
                theme,
                "Extensions",
                "Extensions are installed and enabled by the active profile on the Extensions page.",
                "preferences-open-extensions",
                "Open Extensions",
                ActiveView::Plugins,
                cx,
            ));
        }
        PreferencesSection::Skills => {
            main = main.child(preferences_page_link(
                theme,
                "Skills",
                "Skills available to this Session are listed on the Skills page.",
                "preferences-open-skills",
                "Open Skills",
                ActiveView::Skills,
                cx,
            ));
        }
        PreferencesSection::Marketplace => {
            main = main.child(preferences_page_link(
                theme,
                "Marketplace",
                "Search configured indexes and install profile extensions from the Marketplace page.",
                "preferences-open-marketplace",
                "Open Marketplace",
                ActiveView::Marketplace,
                cx,
            ));
        }
        PreferencesSection::About => main = main.child(preferences_about(app, theme, cx)),
    }
    main.into_any_element()
}

/// Section heading with the short description under it.
fn preferences_page_head(theme: &Theme, title: &str, description: &str) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap(theme.sp(5.))
        .mb(theme.sp(24.))
        .child(
            div()
                .text_size(theme.fs(18.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child(title.to_owned()),
        )
        .child(
            div()
                .text_size(theme.fs(11.5))
                .text_color(theme.muted)
                .child(description.to_owned()),
        )
        .into_any_element()
}

/// Startup, layout, updates, and the Host settings namespaces.
fn preferences_general(
    app: &mut DesktopApp,
    theme: &Theme,
    cx: &mut Context<DesktopApp>,
) -> AnyElement {
    let explorer = app.show_explorer;
    let restore_last_session = app.ui_preferences.restore_last_session;
    let compact = app.ui_preferences.density == UiDensity::Compact;
    let workspace_selected = app.selected_workspace.is_some();
    let message = app.preferences_message.clone();
    let update_status = app.update_status.clone();
    let checking = app.update_open_receiver.is_some();
    let connected = app.transport.is_some();
    let settings_error = app.settings_error.clone();
    let settings_loading = app.settings_loading;
    let namespaces = app
        .settings_description
        .as_ref()
        .map(|describe| {
            describe
                .namespaces
                .iter()
                .map(|namespace| (namespace.ns.clone(), namespace.applies.clone()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let mut content = div().flex().flex_col().gap(theme.sp(8.)).child(preferences_page_head(
        theme,
        "General",
        "Startup, layout, updates, and the settings namespaces the Host reports.",
    ));

    content = content
        .child(section(theme, "LAYOUT"))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(theme.sp(6.))
                .child(line(theme, "Workspace Explorer"))
                .child(
                    widgets::Button::new(theme, if explorer { "Shown" } else { "Hidden" })
                        .primary()
                        .build()
                        .id("pref-explorer")
                        .on_click(cx.listener(move |app, _, window, cx| {
                            app.update_preferences(window, |preferences| {
                                preferences.show_workspace_explorer = !explorer;
                            });
                            app.show_explorer = !explorer;
                            app.ui_preferences.show_workspace_explorer = !explorer;
                            cx.notify();
                        })),
                ),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(theme.sp(6.))
                .child(line(theme, "Restore last Session"))
                .child(
                    widgets::Button::new(theme, if restore_last_session { "On" } else { "Off" })
                        .build()
                        .id("pref-restore-session")
                        .debug_selector(|| "pref-restore-session".to_owned())
                        .on_click(cx.listener(move |app, _, window, cx| {
                            app.update_preferences(window, |preferences| {
                                preferences.restore_last_session = !restore_last_session;
                            });
                            cx.notify();
                        })),
                ),
        )
        .child(line(
            theme,
            "Unsent composer text is kept per Session and saved with this client.",
        ))
        .when_some(message, |element, message| {
            element.child(
                div()
                    .text_size(theme.fs(10.5))
                    .text_color(theme.muted)
                    .child(message),
            )
        })
        .child(widgets::separator(theme))
        .child(section(theme, "WORKSPACE APPEARANCE"))
        .child(line(
            theme,
            match app.selected_workspace.as_deref() {
                Some(id) => format!("Overrides for {id} (empty inherits global)"),
                None => "Select a workspace in the Sessions panel to set its overrides.".to_owned(),
            },
        ))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(theme.sp(6.))
                .child(line(theme, "Explorer"))
                .child(
                    widgets::Button::new(theme, if app.show_explorer { "Show" } else { "Hide" })
                        .enabled(workspace_selected)
                        .build()
                        .id("pref-workspace-explorer")
                        .on_click(cx.listener(|app, _, window, cx| {
                            let workspace = app.selected_workspace.clone();
                            app.update_preferences(window, |preferences| {
                                if let Some(id) = workspace {
                                    let next = !preferences
                                        .workspace_prefs(Some(&id))
                                        .show_workspace_explorer
                                        .unwrap_or(preferences.show_workspace_explorer);
                                    preferences
                                        .workspace_overrides
                                        .entry(id)
                                        .or_default()
                                        .show_workspace_explorer = Some(next);
                                }
                            });
                            let id = app.selected_workspace.clone();
                            app.show_explorer = app
                                .ui_preferences
                                .workspace_prefs(id.as_deref())
                                .show_workspace_explorer
                                .unwrap_or(app.ui_preferences.show_workspace_explorer);
                            cx.notify();
                        })),
                )
                .child(
                    widgets::Button::new(theme, "Clear override")
                        .enabled(workspace_selected)
                        .build()
                        .id("pref-workspace-clear")
                        .on_click(cx.listener(|app, _, window, cx| {
                            let workspace = app.selected_workspace.clone();
                            app.update_preferences(window, |preferences| {
                                if let Some(id) = workspace {
                                    preferences.workspace_overrides.remove(&id);
                                }
                            });
                            cx.notify();
                        })),
                ),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(theme.sp(6.))
                .child(line(theme, "Density"))
                .child(
                    widgets::Button::new(theme, "Compact")
                        .enabled(workspace_selected && !compact)
                        .build()
                        .id("pref-workspace-compact")
                        .on_click(cx.listener(|app, _, window, cx| {
                            let workspace = app.selected_workspace.clone();
                            app.update_preferences(window, |preferences| {
                                if let Some(id) = workspace {
                                    preferences
                                        .workspace_overrides
                                        .entry(id)
                                        .or_default()
                                        .density = Some(UiDensity::Compact);
                                }
                            });
                            cx.notify();
                        })),
                )
                .child(
                    widgets::Button::new(theme, "Clear density")
                        .enabled(workspace_selected)
                        .build()
                        .id("pref-workspace-density-clear")
                        .on_click(cx.listener(|app, _, window, cx| {
                            let workspace = app.selected_workspace.clone();
                            app.update_preferences(window, |preferences| {
                                if let Some(id) = workspace {
                                    let remove = match preferences.workspace_overrides.get_mut(&id) {
                                        Some(entry) => {
                                            entry.density = None;
                                            entry.theme.is_none()
                                                && entry.session_panel_width.is_none()
                                                && entry.show_workspace_explorer.is_none()
                                                && entry.workspace_panel_width.is_none()
                                                && entry.changes_panel_width.is_none()
                                        }
                                        None => false,
                                    };
                                    if remove {
                                        preferences.workspace_overrides.remove(&id);
                                    }
                                }
                            });
                            cx.notify();
                        })),
                ),
        )
        .child(widgets::separator(theme))
        .child(section(theme, "DESKTOP UPDATES"))
        .child(line(
            theme,
            "Check the signed feed and confirm any download or installation here.",
        ))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(theme.sp(6.))
                .child(
                    widgets::Button::new(theme, "Check for updates")
                        .enabled(connected && !checking)
                        .build()
                        .id("pref-updates")
                        .on_click(cx.listener(|app, _, _, cx| {
                            app.open_updates();
                            cx.notify();
                        })),
                )
                .when(checking, |element| element.child(line(theme, "Checking…")))
                .when_some(update_status, |element, status| {
                    element.child(line(theme, status))
                }),
        )
        .child(widgets::separator(theme))
        .child(section(theme, "HOST SETTINGS"))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(theme.sp(6.))
                .child(
                    widgets::Button::new(theme, "Refresh")
                        .enabled(connected && !settings_loading)
                        .build()
                        .id("pref-refresh-settings")
                        .on_click(cx.listener(|app, _, _, cx| {
                            app.refresh_settings();
                            cx.notify();
                        })),
                )
                .when(settings_loading, |element| element.child(line(theme, "Loading…")))
                .when_some(settings_error, |element, error| {
                    element.child(
                        div()
                            .text_size(theme.fs(10.5))
                            .text_color(theme.error)
                            .child(error),
                    )
                }),
        );
    if namespaces.is_empty() {
        content = content.child(line(
            theme,
            if connected {
                "No settings namespaces are reported yet."
            } else {
                "Connect to the Host to view or change agent and plugin settings."
            },
        ));
    }
    for (name, applies) in namespaces {
        content = content.child(
            div()
                .flex()
                .flex_col()
                .child(
                    div()
                        .text_size(theme.fs(11.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child(name),
                )
                .child(line(theme, applies)),
        );
    }
    content.into_any_element()
}

/// Theme pack, accent family, density, and interface scale.
fn preferences_appearance(
    app: &mut DesktopApp,
    theme: &Theme,
    cx: &mut Context<DesktopApp>,
) -> AnyElement {
    // The preview follows the resolved scheme, so System mode and workspace
    // overrides show the colours the shell is actually rendering.
    let dark = theme.dark;
    let compact = app.ui_preferences.density == UiDensity::Compact;
    let scale = app.ui_preferences.ui_scale;
    let accent = app.ui_preferences.accent_scheme;
    let theme_notice = theme.status.errors.join("; ");
    let selected_theme = app.ui_preferences.theme_id.clone();
    let mut theme_rows: Vec<(String, String, ThemeSource, Option<ContrastStatus>, bool)> = app
        .theme_catalog
        .themes()
        .iter()
        .map(|installed| {
            let id = installed.pack.id.clone();
            (
                id.clone(),
                installed.pack.name.clone(),
                installed.source,
                app.theme_catalog.contrast_status(&id, accent, dark),
                true,
            )
        })
        .collect();
    // The legacy custom palette is not a file-backed pack, so it appears in
    // the list but its file actions stay disabled with a stated reason.
    let custom = resolve(&ThemeRequest {
        pack_id: CUSTOM_PACK_ID,
        custom: &app.ui_preferences.custom_theme,
        catalog: &app.theme_catalog,
        accent,
        dark,
    });
    theme_rows.push((
        CUSTOM_PACK_ID.to_owned(),
        "Custom".to_owned(),
        ThemeSource::Builtin,
        Some(if custom.status.warnings.is_empty() {
            ContrastStatus::Pass
        } else {
            ContrastStatus::Warnings(custom.status.warnings.len())
        }),
        false,
    ));
    let theme_errors = app.theme_catalog.errors().to_vec();
    let theme_remove_confirm = app.theme_remove_confirm.clone();
    let theme_pending_replace = app.theme_pending_import.is_some();
    let theme_message = app.theme_message.clone();
    let editor_id = app.theme_editor.as_ref().map(|editor| editor.id.clone());

    let mut content = div().flex().flex_col().gap(theme.sp(8.)).child(preferences_page_head(
        theme,
        "Appearance",
        "Theme pack, accent, density, and text scale for the shell.",
    ));
    content = content
        .child(section(theme, "COLOR"))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(theme.sp(6.))
                .child(line(theme, "Theme"))
                .child(
                    widgets::Button::new(theme, "Light")
                        .enabled(dark)
                        .build()
                        .id("pref-light")
                        .on_click(cx.listener(|app, _, window, cx| {
                            app.update_preferences(window, |preferences| {
                                preferences.theme = ThemeMode::Light;
                            });
                            cx.notify();
                        })),
                )
                .child(
                    widgets::Button::new(theme, "Dark")
                        .enabled(!dark)
                        .build()
                        .id("pref-dark")
                        .on_click(cx.listener(|app, _, window, cx| {
                            app.update_preferences(window, |preferences| {
                                preferences.theme = ThemeMode::Dark;
                            });
                            cx.notify();
                        })),
                ),
        )
        .child(section(theme, "PACKS"))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .flex_wrap()
                .gap(theme.sp(6.))
                .child(
                    widgets::Button::new(theme, "Add theme file…")
                        .build()
                        .id("pref-theme-add")
                        .debug_selector(|| "pref-theme-add".to_owned())
                        .on_click(cx.listener(|app, _, _, cx| {
                            app.add_theme_file();
                            cx.notify();
                        })),
                )
                .when(theme_pending_replace, |element| {
                    element.child(
                        widgets::Button::new(theme, "Replace theme file")
                            .primary()
                            .build()
                            .id("pref-theme-replace")
                            .debug_selector(|| "pref-theme-replace".to_owned())
                            .on_click(cx.listener(|app, _, window, cx| {
                                app.replace_theme_file(window);
                                cx.notify();
                            })),
                    )
                }),
        )
        .when_some(theme_message, |element, message| {
            element.child(
                div()
                    .text_size(theme.fs(10.5))
                    .text_color(theme.muted)
                    .child(message),
            )
        })
        .children(theme_rows.into_iter().map(
            |(id, name, source, contrast, file_backed)| {
                theme_pack_row(
                    theme,
                    id,
                    name,
                    source,
                    contrast,
                    file_backed,
                    selected_theme.as_str(),
                    theme_remove_confirm.as_deref(),
                    cx,
                )
            },
        ))
        .children(theme_errors.into_iter().map(|error| {
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(theme.sp(6.))
                .debug_selector({
                    let file_name = error.file_name.clone();
                    move || format!("pref-theme-error-{file_name}")
                })
                .child(
                    div()
                        .text_size(theme.fs(11.))
                        .text_color(theme.error)
                        .child(error.file_name),
                )
                .child(
                    div()
                        .text_size(theme.fs(10.5))
                        .text_color(theme.muted)
                        .child(error.reason),
                )
                .into_any_element()
        }))
        .when(!theme_notice.is_empty(), |element| {
            element.child(line(theme, theme_notice))
        })
        .when_some(editor_id, |element, editor_id| {
            element.child(theme_editor_panel(app, theme, &editor_id, cx))
        })
        .child(section(theme, "ACCENT"))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .flex_wrap()
                .gap(theme.sp(6.))
                .child(accent_button(
                    theme,
                    "Harness",
                    "pref-accent-harness",
                    AccentScheme::Harness,
                    accent == AccentScheme::Harness,
                    theme.accent_families,
                    cx,
                ))
                .child(accent_button(
                    theme,
                    "Blue",
                    "pref-accent-blue",
                    AccentScheme::Blue,
                    accent == AccentScheme::Blue,
                    theme.accent_families,
                    cx,
                ))
                .child(accent_button(
                    theme,
                    "Teal",
                    "pref-accent-teal",
                    AccentScheme::Teal,
                    accent == AccentScheme::Teal,
                    theme.accent_families,
                    cx,
                ))
                .child(accent_button(
                    theme,
                    "Violet",
                    "pref-accent-violet",
                    AccentScheme::Violet,
                    accent == AccentScheme::Violet,
                    theme.accent_families,
                    cx,
                )),
        )
        .child(section(theme, "DENSITY"))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(theme.sp(6.))
                .child(line(theme, "Density"))
                .child(
                    widgets::Button::new(theme, "Comfortable")
                        .enabled(compact)
                        .build()
                        .id("pref-comfortable")
                        .on_click(cx.listener(|app, _, window, cx| {
                            app.update_preferences(window, |preferences| {
                                preferences.density = UiDensity::Comfortable;
                            });
                            cx.notify();
                        })),
                )
                .child(
                    widgets::Button::new(theme, "Compact")
                        .enabled(!compact)
                        .build()
                        .id("pref-compact")
                        .on_click(cx.listener(|app, _, window, cx| {
                            app.update_preferences(window, |preferences| {
                                preferences.density = UiDensity::Compact;
                            });
                            cx.notify();
                        })),
                ),
        )
        .child(section(theme, "SCALE"))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(theme.sp(6.))
                .child(line(theme, format!("Interface scale · {scale:.2}×")))
                .child(
                    widgets::Button::icon_only(theme, Icon::Minus)
                        .build()
                        .id("pref-scale-down")
                        .debug_selector(|| "pref-scale-down".to_owned())
                        .on_click(cx.listener(|app, _, window, cx| {
                            app.update_preferences(window, |preferences| {
                                preferences.ui_scale =
                                    (preferences.ui_scale - 0.05).clamp(0.8, 1.6);
                            });
                            cx.notify();
                        })),
                )
                .child(
                    widgets::Button::icon_only(theme, Icon::Plus)
                        .build()
                        .id("pref-scale-up")
                        .debug_selector(|| "pref-scale-up".to_owned())
                        .on_click(cx.listener(|app, _, window, cx| {
                            app.update_preferences(window, |preferences| {
                                preferences.ui_scale =
                                    (preferences.ui_scale + 0.05).clamp(0.8, 1.6);
                            });
                            cx.notify();
                        })),
                ),
        );
    content.into_any_element()
}

/// One installed pack with its source, contrast status, and file actions.
fn theme_pack_row(
    theme: &Theme,
    id: String,
    name: String,
    source: ThemeSource,
    contrast: Option<ContrastStatus>,
    file_backed: bool,
    selected: &str,
    remove_confirm: Option<&str>,
    cx: &mut Context<DesktopApp>,
) -> AnyElement {
    let is_user = source == ThemeSource::User;
    let already_selected = selected == id;
    let confirming_remove = remove_confirm == Some(id.as_str());
    let row_selector = id.clone();

    let use_id = id.clone();
    let use_selector = id.clone();
    let duplicate_id = id.clone();
    let duplicate_selector = id.clone();
    let edit_id = id.clone();
    let edit_selector = id.clone();
    let export_id = id.clone();
    let export_selector = id.clone();
    let remove_id = id.clone();
    let remove_selector = id.clone();

    div()
        .flex()
        .flex_row()
        .items_center()
        .flex_wrap()
        .gap(theme.sp(6.))
        .py(theme.sp(3.))
        .debug_selector(move || format!("pref-theme-row-{row_selector}"))
        .child(
            div()
                .text_size(theme.fs(11.5))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child(name),
        )
        .child(theme_source_badge(theme, source, &id))
        .child(theme_contrast_badge(theme, &id, &contrast))
        .child(
            widgets::Button::new(theme, "Use")
                .enabled(!already_selected)
                .build()
                .id(gpui::SharedString::from(format!("pref-pack-{id}")))
                .debug_selector(move || format!("pref-pack-{use_selector}"))
                .when(already_selected, |button| {
                    button.tooltip(widgets::HintView::tooltip(
                        theme,
                        "This theme is already active.",
                    ))
                })
                .when(!already_selected, |button| {
                    button.on_click(cx.listener(move |app, _, window, cx| {
                        let id = use_id.clone();
                        app.update_preferences(window, |preferences| {
                            preferences.theme_id = id;
                        });
                        cx.notify();
                    }))
                }),
        )
        .child(
            widgets::Button::new(theme, "Duplicate")
                .enabled(file_backed)
                .build()
                .id(gpui::SharedString::from(format!(
                    "pref-theme-duplicate-{duplicate_id}"
                )))
                .debug_selector(move || format!("pref-theme-duplicate-{duplicate_selector}"))
                .when(!file_backed, |button| {
                    button.tooltip(widgets::HintView::tooltip(
                        theme,
                        "The legacy custom palette is not a theme file; choose a built-in or user pack to copy.",
                    ))
                })
                .when(file_backed, |button| {
                    button.on_click(cx.listener(move |app, _, window, cx| {
                        app.duplicate_theme(&duplicate_id, window);
                        cx.notify();
                    }))
                }),
        )
        .child(
            widgets::Button::new(theme, "Edit JSON")
                .enabled(is_user)
                .build()
                .id(gpui::SharedString::from(format!(
                    "pref-theme-edit-{edit_id}"
                )))
                .debug_selector(move || format!("pref-theme-edit-{edit_selector}"))
                .when(!is_user, |button| {
                    button.tooltip(widgets::HintView::tooltip(
                        theme,
                        "Built-in themes are read-only; Duplicate one to edit its colours.",
                    ))
                })
                .when(is_user, |button| {
                    button.on_click(cx.listener(move |app, _, _, cx| {
                        app.edit_theme(&edit_id);
                        cx.notify();
                    }))
                }),
        )
        .child(
            widgets::Button::new(theme, "Export…")
                .enabled(file_backed)
                .build()
                .id(gpui::SharedString::from(format!(
                    "pref-theme-export-{export_id}"
                )))
                .debug_selector(move || format!("pref-theme-export-{export_selector}"))
                .when(!file_backed, |button| {
                    button.tooltip(widgets::HintView::tooltip(
                        theme,
                        "The legacy custom palette is not a theme file to export.",
                    ))
                })
                .when(file_backed, |button| {
                    button.on_click(cx.listener(move |app, _, _, cx| {
                        app.export_theme(&export_id);
                        cx.notify();
                    }))
                }),
        )
        .child(
            widgets::Button::new(theme, if confirming_remove { "Confirm remove" } else { "Remove" })
                .enabled(is_user)
                .danger()
                .build()
                .id(gpui::SharedString::from(format!(
                    "pref-theme-remove-{remove_id}"
                )))
                .debug_selector(move || format!("pref-theme-remove-{remove_selector}"))
                .when(!is_user, |button| {
                    button.tooltip(widgets::HintView::tooltip(
                        theme,
                        "Built-in themes cannot be removed.",
                    ))
                })
                .when(is_user, |button| {
                    button.on_click(cx.listener(move |app, _, window, cx| {
                        app.remove_theme(&remove_id, window);
                        cx.notify();
                    }))
                }),
        )
        .into_any_element()
}

/// Source badge shown beside one installed pack.
fn theme_source_badge(theme: &Theme, source: ThemeSource, id: &str) -> AnyElement {
    let selector = id.to_owned();
    div()
        .px(theme.sp(5.))
        .py(theme.sp(1.))
        .rounded(theme.corner())
        .border_1()
        .border_color(theme.border)
        .bg(theme.surface_raised)
        .text_size(theme.fs(9.5))
        .text_color(match source {
            ThemeSource::Builtin => theme.muted,
            ThemeSource::User => theme.accent,
        })
        .debug_selector(move || format!("pref-theme-source-{selector}"))
        .child(source.label())
        .into_any_element()
}

/// Contrast gate outcome shown beside one installed pack.
fn theme_contrast_badge(
    theme: &Theme,
    id: &str,
    contrast: &Option<ContrastStatus>,
) -> AnyElement {
    let (text, color) = match contrast {
        Some(ContrastStatus::Pass) => ("contrast ok".to_owned(), theme.success),
        Some(ContrastStatus::Warnings(count)) => {
            (format!("contrast warnings ({count})"), theme.warn)
        }
        Some(ContrastStatus::Rejected(reasons)) => (
            format!(
                "contrast rejected: {}",
                reasons
                    .first()
                    .map(String::as_str)
                    .unwrap_or("required pair failed")
            ),
            theme.error,
        ),
        None => ("contrast unavailable".to_owned(), theme.error),
    };
    let selector = id.to_owned();
    div()
        .text_size(theme.fs(10.))
        .text_color(color)
        .debug_selector(move || format!("pref-theme-contrast-{selector}"))
        .child(text)
        .into_any_element()
}

/// JSON editor for one installed user pack.
fn theme_editor_panel(
    app: &mut DesktopApp,
    theme: &Theme,
    editor_id: &str,
    cx: &mut Context<DesktopApp>,
) -> AnyElement {
    let pending = app
        .theme_editor
        .as_mut()
        .and_then(|editor| editor.pending_text.take());
    let input = app.text_field("theme-editor", "Theme pack JSON", true, cx);
    if let Some(text) = pending {
        input.update(cx, |input, cx| input.set_text(text, cx));
    }
    div()
        .flex()
        .flex_col()
        .gap(theme.sp(6.))
        .child(section(theme, "EDIT THEME JSON"))
        .child(line(
            theme,
            format!("Editing `{editor_id}`. Save validates the complete document before writing."),
        ))
        .child(
            div()
                .id("pref-theme-editor")
                .debug_selector(|| "pref-theme-editor".to_owned())
                .flex()
                .flex_col()
                .w_full()
                .min_h(theme.sp(220.))
                .p(theme.sp(8.))
                .rounded(theme.corner())
                .border_1()
                .border_color(theme.border)
                .bg(theme.code_background)
                .font(theme.mono_font())
                .overflow_hidden()
                .child(crate::shell::text_input::text_input_element(
                    &input,
                    theme,
                    theme.sp(220.),
                )),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(theme.sp(6.))
                .child(
                    widgets::Button::new(theme, "Save")
                        .primary()
                        .build()
                        .id("pref-theme-save")
                        .debug_selector(|| "pref-theme-save".to_owned())
                        .on_click(cx.listener(|app, _, window, cx| {
                            let text = app.input_text("theme-editor", cx);
                            app.save_theme_editor(&text, window);
                            cx.notify();
                        })),
                )
                .child(
                    widgets::Button::new(theme, "Cancel")
                        .build()
                        .id("pref-theme-cancel")
                        .debug_selector(|| "pref-theme-cancel".to_owned())
                        .on_click(cx.listener(|app, _, _, cx| {
                            app.cancel_theme_editor();
                            app.set_input("theme-editor", "", cx);
                            cx.notify();
                        })),
                ),
        )
        .into_any_element()
}

/// Keyboard profile selection shared by the menu, palette, and dispatcher.
fn preferences_keymap(
    app: &mut DesktopApp,
    theme: &Theme,
    cx: &mut Context<DesktopApp>,
) -> AnyElement {
    let preset = app.ui_preferences.keymap_preset;
    div()
        .flex()
        .flex_col()
        .gap(theme.sp(8.))
        .child(preferences_page_head(
            theme,
            "Keymap",
            "Keyboard profiles applied to the commands in the menu and palette.",
        ))
        .child(section(theme, "PRESET"))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(theme.sp(6.))
                .child(line(theme, "Keymap preset"))
                .child(
                    widgets::Button::new(theme, "DSH")
                        .enabled(preset != KeymapPreset::Dsh)
                        .build()
                        .id("pref-keymap-dsh")
                        .on_click(cx.listener(|app, _, window, cx| {
                            app.update_preferences(window, |preferences| {
                                preferences.keymap_preset = KeymapPreset::Dsh;
                                preferences.keybindings.apply_preset(KeymapPreset::Dsh);
                            });
                            cx.notify();
                        })),
                )
                .child(
                    widgets::Button::new(theme, "Zed")
                        .enabled(preset != KeymapPreset::Zed)
                        .build()
                        .id("pref-keymap-zed")
                        .debug_selector(|| "pref-keymap-zed".to_owned())
                        .on_click(cx.listener(|app, _, window, cx| {
                            app.update_preferences(window, |preferences| {
                                preferences.keymap_preset = KeymapPreset::Zed;
                                preferences.keybindings.apply_preset(KeymapPreset::Zed);
                            });
                            cx.notify();
                        })),
                )
                .child(
                    widgets::Button::new(theme, "VS Code")
                        .enabled(preset != KeymapPreset::Vscode)
                        .build()
                        .id("pref-keymap-vscode")
                        .on_click(cx.listener(|app, _, window, cx| {
                            app.update_preferences(window, |preferences| {
                                preferences.keymap_preset = KeymapPreset::Vscode;
                                preferences.keybindings.apply_preset(KeymapPreset::Vscode);
                            });
                            cx.notify();
                        })),
                ),
        )
        .into_any_element()
}

/// Section body that deep-links to one of the shell's full pages.
///
/// Extensions, Skills, and Marketplace keep their own page layout, so the
/// overlay carries the navigation entry without duplicating their content.
fn preferences_page_link(
    theme: &Theme,
    title: &str,
    description: &str,
    button_id: &'static str,
    button_label: &'static str,
    view: ActiveView,
    cx: &mut Context<DesktopApp>,
) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap(theme.sp(10.))
        .child(preferences_page_head(theme, title, description))
        .child(
            widgets::Button::new(theme, button_label)
                .primary()
                .build()
                .id(button_id)
                .debug_selector(move || format!("{button_id}-button"))
                .on_click(cx.listener(move |app, _, _, cx| {
                    app.switch_view(view);
                    cx.notify();
                })),
        )
        .into_any_element()
}

/// Client version, Host connection, and the About dialog entry point.
fn preferences_about(
    app: &mut DesktopApp,
    theme: &Theme,
    cx: &mut Context<DesktopApp>,
) -> AnyElement {
    let version = env!("CARGO_PKG_VERSION");
    let host = app
        .host_url
        .clone()
        .unwrap_or_else(|| "not connected".to_owned());
    div()
        .flex()
        .flex_col()
        .gap(theme.sp(8.))
        .child(preferences_page_head(
            theme,
            "About",
            "The native desktop client built over the Desktop Host API.",
        ))
        .child(section(theme, "APPLICATION"))
        .child(line(theme, format!("Version · {version}")))
        .child(line(theme, "Runtime · Rust + gpui shell over the Desktop Host API."))
        .child(line(theme, format!("Host · {host}")))
        .child(
            widgets::Button::new(theme, "About DSH")
                .build()
                .id("pref-about-dialog")
                .on_click(cx.listener(|app, _, _, cx| {
                    app.show_about = true;
                    cx.notify();
                })),
        )
        .into_any_element()
}

fn plugins(
    app: &mut DesktopApp,
    theme: &Theme,
    cx: &mut Context<DesktopApp>,
) -> AnyElement {
    let loading = app.plugins_loading;
    let error = app.plugins_error.clone();
    let status = app.plugin_write_status.clone();
    let busy = app.plugin_write_receiver.is_some();
    let snapshot = app.plugins_snapshot.clone();
    let managed = app.managed_plugins.clone();
    let bundles = app.plugin_bundles.clone();

    let mut content = body(theme).child(
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(theme.sp(6.))
            .child(
                widgets::Button::new(theme, "Refresh")
                    .enabled(!loading)
                    .build()
                    .id("plugins-refresh")
                    .on_click(cx.listener(|app, _, _, cx| {
                        app.refresh_plugins();
                        cx.notify();
                    })),
            )
            .when(loading, |element| element.child(line(theme, "Loading plugins…")))
            .when_some(error.clone(), |element, error| {
                element.child(
                    div()
                        .text_size(theme.fs(10.5))
                        .text_color(theme.error)
                        .child(error),
                )
            })
            .when_some(status, |element, status| element.child(line(theme, status))),
    );

    let Some(snapshot) = snapshot else {
        return content
            .child(line(theme, "Open this tab to load the plugin inventory."))
            .into_any_element();
    };
    if snapshot.management_available == Some(false) {
        content = content.child(line(theme, "Plugin management is unavailable for this Host."));
    }
    let failed = snapshot
        .entries
        .iter()
        .filter(|entry| entry.fiber_phase == Some(dsh_client_core::plugins::PluginFiberPhase::Failed))
        .count();
    let active = snapshot
        .entries
        .iter()
        .filter(|entry| entry.fiber_phase == Some(dsh_client_core::plugins::PluginFiberPhase::Active))
        .count();
    content = content.child(section(theme, "HEALTH")).child(
        div()
            .text_size(theme.fs(10.5))
            .text_color(if failed > 0 { theme.error } else { theme.muted })
            .child(if failed > 0 {
                format!(
                    "{active} active · {failed} failed — the Loader reported a failed fiber; check the Host log for the diagnostic"
                )
            } else {
                format!("{active} active · no failed fibers reported")
            }),
    );
    content = content.child(section(theme, "CONFIGURED PLUGINS"));
    if snapshot.entries.is_empty() {
        content = content.child(line(theme, "No configured plugins."));
    }
    for entry in &snapshot.entries {
        let phase = entry.fiber_phase.map_or_else(
            || "unknown".to_owned(),
            |phase| format!("{phase:?}").to_lowercase(),
        );
        let state = if entry.enabled { "enabled" } else { "disabled" };
        let entry_id = entry.entry_id.clone();
        let enabled = entry.enabled;
        let patchable = managed
            .iter()
            .find(|plugin| plugin.entry_id == entry.entry_id)
            .is_some_and(|plugin| plugin.patch_id.is_some());
        let read_only = managed
            .iter()
            .find(|plugin| plugin.entry_id == entry.entry_id)
            .and_then(|plugin| plugin.read_only_reason.clone());
        content = content.child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(theme.sp(6.))
                .child(
                    div()
                        .flex_1()
                        .text_size(theme.fs(11.))
                        .child(format!("{} · {state} · {phase}", entry.module_name)),
                )
                .when(patchable, |element| {
                    element.child(
                        widgets::Button::new(theme, if enabled { "Disable" } else { "Enable" })
                            .enabled(!busy)
                            .build()
                            .id(gpui::SharedString::from(format!("plugin-toggle-{entry_id}")))
                            .on_click(cx.listener(move |app, _, _, cx| {
                                app.set_plugin_enabled(entry_id.clone(), !enabled);
                                cx.notify();
                            })),
                    )
                })
                .when_some(read_only, |element, reason| {
                    element.child(line(theme, format!("read-only: {reason}")))
                }),
        );
    }
    if !bundles.is_empty() {
        content = content
            .child(widgets::separator(theme))
            .child(section(theme, "BUNDLES"));
        for bundle in &bundles {
            let name = bundle.name.clone();
            let remove_name = bundle.name.clone();
            let enabled = bundle.enabled;
            let state = if enabled { "enabled" } else { "disabled" };
            content = content.child(
                div()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(theme.sp(6.))
                            .child(
                                div()
                                    .flex_1()
                                    .text_size(theme.fs(11.))
                                    .child(format!(
                                        "{} · {state} · {}",
                                        bundle.name, bundle.availability
                                    )),
                            )
                            .child(
                                widgets::Button::new(
                                    theme,
                                    if enabled { "Disable" } else { "Enable" },
                                )
                                .enabled(!busy)
                                .build()
                                .id(gpui::SharedString::from(format!("bundle-toggle-{name}")))
                                .on_click(cx.listener(move |app, _, _, cx| {
                                    app.set_bundle_enabled(name.clone(), !enabled);
                                    cx.notify();
                                })),
                            )
                            .child(
                                widgets::Button::new(theme, "Remove")
                                    .danger()
                                    .enabled(!busy)
                                    .build()
                                    .id(gpui::SharedString::from(format!("bundle-remove-{remove_name}")))
                                    .on_click(cx.listener(move |app, _, _, cx| {
                                        app.remove_bundle(remove_name.clone());
                                        cx.notify();
                                    })),
                            ),
                    )
                    .when_some(bundle.description.clone(), |element, description| {
                        element.child(line(theme, description))
                    }),
            );
        }
    }
    if let Some(presets) = &snapshot.agent_presets {
        content = content
            .child(widgets::separator(theme))
            .child(section(theme, "AGENT PRESETS"));
        for preset in presets {
            let name = preset.name.clone().unwrap_or_else(|| preset.id.clone());
            content = content.child(line(theme, format!("{name} · {} row(s)", preset.rows.len())));
        }
    }
    content.into_any_element()
}

fn skills(
    app: &mut DesktopApp,
    theme: &Theme,
    cx: &mut Context<DesktopApp>,
) -> AnyElement {
    let loading = app.skills_loading;
    let error = app.skills_error.clone();
    let skills = app.skills.clone();
    let skills_busy = app.skills_write_receiver.is_some();
    let skills_status = app.skills_status.clone();
    let selected = app.selected_skill.clone();
    let has_session = app.selected_session.is_some();

    let mut content = body(theme);
    if !has_session {
        return content
            .child(line(theme, "Open this tab with a session selected to load skills."))
            .into_any_element();
    }
    content = content.child(
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(theme.sp(6.))
            .child(
                widgets::Button::new(theme, "Refresh")
                    .enabled(!loading)
                    .build()
                    .id("skills-refresh")
                    .on_click(cx.listener(|app, _, _, cx| {
                        app.refresh_skills();
                        cx.notify();
                    })),
            )
            .when(loading, |element| element.child(line(theme, "Loading skills…")))
            .when_some(error.clone(), |element, error| {
                element.child(
                    div()
                        .text_size(theme.fs(10.5))
                        .text_color(theme.error)
                        .child(error),
                )
            }),
    );
    let Some(skills) = skills else {
        return content
            .child(line(theme, "Skills load when this tab opens with a Session."))
            .into_any_element();
    };
    if let Some(status) = skills_status {
        content = content.child(line(theme, status));
    }
    content = content.child(line(
        theme,
        "Disabling filters the skill out of every Host catalog read: model skills, /skill, and this list.",
    ));
    let chosen = skills
        .skills
        .iter()
        .find(|skill| selected.as_deref() == Some(skill.name.as_str()))
        .cloned();
    let mut list = div()
        .flex()
        .flex_row()
        .gap(theme.sp(12.))
        .flex_1()
        .min_h(px(0.));
    let mut names = div()
        .id("skills-list")
        .flex()
        .flex_col()
        .gap(px(1.))
        .w(theme.sp(260.))
        .overflow_y_scroll();
    if skills.skills.is_empty() {
        names = names.child(line(theme, "No skills are visible to this Session."));
    }
    for skill in &skills.skills {
        let name = skill.name.clone();
        let toggle_name = skill.name.clone();
        let current = selected.as_deref() == Some(name.as_str());
        names = names.child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(theme.sp(4.))
                .px(theme.sp(8.))
                .py(theme.sp(5.))
                .rounded(theme.corner())
                .cursor_pointer()
                .when(current, |element| element.bg(theme.selection))
                .hover(|style| style.bg(theme.surface))
                .id(gpui::SharedString::from(format!("skill-{name}")))
                .child(
                    div()
                        .flex_1()
                        .text_size(theme.fs(11.5))
                        .truncate()
                        .child(name.clone()),
                )
                .child(
                    widgets::Button::new(theme, "Disable")
                        .enabled(!skills_busy)
                        .build()
                        .id(gpui::SharedString::from(format!("skill-disable-{name}")))
                        .on_click(cx.listener(move |app, _, _, cx| {
                            app.set_skill_enabled(toggle_name.clone(), false);
                            cx.notify();
                        })),
                )
                .on_click(cx.listener(move |app, _, _, cx| {
                    app.selected_skill = Some(name.clone());
                    cx.notify();
                })),
        );
    }
    if !skills.disabled_skills.is_empty() {
        names = names.child(widgets::section_label(theme, "DISABLED"));
        for disabled in &skills.disabled_skills {
            let name = disabled.clone();
            let enable_name = disabled.clone();
            names = names.child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(theme.sp(4.))
                    .px(theme.sp(8.))
                    .py(theme.sp(4.))
                    .child(
                        div()
                            .flex_1()
                            .text_size(theme.fs(11.))
                            .text_color(theme.muted)
                            .truncate()
                            .child(name),
                    )
                    .child(
                        widgets::Button::new(theme, "Enable")
                            .enabled(!skills_busy)
                            .build()
                            .id(gpui::SharedString::from(format!("skill-enable-{enable_name}")))
                            .on_click(cx.listener(move |app, _, _, cx| {
                                app.set_skill_enabled(enable_name.clone(), true);
                                cx.notify();
                            })),
                    ),
            );
        }
    }
    let mut detail = div().flex().flex_col().gap(theme.sp(6.)).flex_1().min_w(px(0.));
    if let Some(skill) = chosen {
        detail = detail
            .child(
                div()
                    .text_size(theme.fs(14.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child(skill.name.clone()),
            )
            .child(line(theme, skill.description.clone()));
        if let Some(when) = skill.when_to_use.clone() {
            detail = detail
                .child(section(theme, "When it applies"))
                .child(line(theme, when));
        }
        let mut source_facts = Vec::new();
        if let Some(source) = skill.source.clone() {
            source_facts.push(format!("scope {source}"));
        }
        if let Some(provider) = skill.provider.clone() {
            source_facts.push(format!("provider {provider}"));
        }
        source_facts.push(if skill.model_invocable {
            "model + /command".to_owned()
        } else {
            "/command only".to_owned()
        });
        detail = detail
            .child(section(theme, "Source"))
            .child(line(theme, source_facts.join(" · ")));
        if let Some(path) = skill.path.clone() {
            detail = detail.child(line(theme, path));
        }
    } else {
        detail = detail.child(line(theme, "Select a skill to read its description."));
    }
    list = list.child(names).child(detail);
    content.child(list).into_any_element()
}

/// Reads index text that is either a plain string or a language map.
///
/// The shell has no locale switch today, so the English member wins and any
/// other language acts as a fallback rather than being dropped.
fn localized_text(value: &serde_json::Value) -> Option<String> {
    if let Some(text) = value.as_str() {
        return Some(text.to_owned());
    }
    let object = value.as_object()?;
    object
        .get("en")
        .and_then(serde_json::Value::as_str)
        .or_else(|| object.values().find_map(serde_json::Value::as_str))
        .map(str::to_owned)
}

fn marketplace(
    app: &mut DesktopApp,
    theme: &Theme,
    cx: &mut Context<DesktopApp>,
) -> AnyElement {
    let loading = app.market_loading;
    let status = app.market_status.clone();
    let indexes = app.market_indexes.len();
    let results = app.market_results.clone();
    let connected = app.transport.is_some();
    let query = app.text_field("market-query", "Search text", false, cx);

    let mut content = body(theme)
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(theme.sp(6.))
                .child(
                    widgets::Button::new(theme, "Refresh indexes")
                        .enabled(connected && !loading)
                        .build()
                        .id("market-refresh")
                        .on_click(cx.listener(|app, _, _, cx| {
                            app.refresh_market_indexes();
                            cx.notify();
                        })),
                )
                .when(loading, |element| element.child(line(theme, "Loading…")))
                .when(indexes == 0 && !loading, |element| {
                    element.child(line(theme, "No market indexes configured."))
                })
                .when_some(status, |element, status| element.child(line(theme, status))),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(theme.sp(6.))
                .child(
                    div()
                        .w(theme.sp(240.))
                        .h(theme.control_height())
                        .px(theme.sp(6.))
                        .rounded(theme.corner())
                        .border_1()
                        .border_color(theme.border)
                        .bg(theme.code_background)
                        .overflow_hidden()
                        .child(crate::shell::text_input::text_input_element(
                            &query,
                            theme,
                            theme.control_height() - px(2.),
                        )),
                )
                .child(
                    widgets::Button::new(theme, "Search")
                        .enabled(connected && !loading)
                        .build()
                        .id("market-search")
                        .on_click(cx.listener(|app, _, _, cx| {
                            app.search_market();
                            cx.notify();
                        })),
                ),
        );
    for entry in &results {
        let name = entry
            .get("name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("plugin")
            .to_owned();
        let description = entry
            .get("description")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let install = entry
            .get("versions")
            .and_then(serde_json::Value::as_array)
            .and_then(|versions| versions.first())
            .and_then(|version| version.get("install"))
            .cloned();
        let spec = install
            .as_ref()
            .and_then(|install| install.get("spec"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned);
        let installation = entry.get("installation");
        let install_status = installation
            .and_then(|value| value.get("status"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("not-installed")
            .to_owned();
        let installed_version = installation
            .and_then(|value| value.get("installedVersion"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let default_version = entry
            .get("defaultVersion")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let author = entry
            .get("author")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let repository_host = entry
            .get("repository")
            .and_then(|value| value.get("url"))
            .and_then(serde_json::Value::as_str)
            .and_then(|url| url.split('/').nth(2).map(str::to_owned))
            .unwrap_or_default();
        let compatibility = entry
            .get("versions")
            .and_then(serde_json::Value::as_array)
            .and_then(|versions| versions.first())
            .and_then(|version| version.get("compatibility"))
            .and_then(|value| value.get("compatible"))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(true);
        let mut trust = Vec::new();
        if !author.is_empty() {
            trust.push(format!("by {author}"));
        }
        if !repository_host.is_empty() {
            trust.push(format!("source on {repository_host}"));
        }
        match install_status.as_str() {
            "installed" => trust.push(if installed_version.is_empty() {
                "installed".to_owned()
            } else {
                format!("installed v{installed_version}")
            }),
            "update-available" => trust.push(format!(
                "update from v{} to v{default_version}",
                installed_version
            )),
            "different-version" => trust.push(if installed_version.is_empty() {
                "different installed version".to_owned()
            } else {
                format!("installed v{installed_version} differs from index")
            }),
            "provided" => trust.push("provided by this DSH".to_owned()),
            _ => {}
        }
        if !compatibility {
            trust.push("not compatible with this DSH version".to_owned());
        }
        let trust_line = trust.join(" · ");
        // Declared capabilities travel with the index; an index that declares
        // none says so instead of implying "no permissions needed".
        let permissions = entry
            .get("permissions")
            .and_then(serde_json::Value::as_array)
            .map(|list| {
                list.iter()
                    .filter_map(|permission| {
                        let id = permission.get("id").and_then(serde_json::Value::as_str)?;
                        Some(match permission.get("description").and_then(localized_text) {
                            Some(description) => format!("{id} — {description}"),
                            None => id.to_owned(),
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let permissions_line = if permissions.is_empty() {
            "The index declares no capabilities for this plugin.".to_owned()
        } else {
            format!("Capabilities: {}", permissions.join(" · "))
        };
        // The changelog belongs to the version the action would install.
        let target_version = if default_version.is_empty() {
            installed_version.as_str()
        } else {
            default_version.as_str()
        };
        let changelog = entry
            .get("versions")
            .and_then(serde_json::Value::as_array)
            .and_then(|versions| {
                versions.iter().find(|version| {
                    version.get("version").and_then(serde_json::Value::as_str)
                        == Some(target_version)
                })
            })
            .and_then(|version| version.get("changelog"))
            .and_then(localized_text);
        let update_available = install_status == "update-available";
        let update_spec = spec.clone();
        let update_version = default_version.clone();
        content = content.child(
            div()
                .flex()
                .flex_col()
                .gap(px(2.))
                .p(theme.sp(8.))
                .rounded(theme.corner())
                .bg(theme.surface)
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(theme.sp(6.))
                        .child(
                            div()
                                .flex_1()
                                .text_size(theme.fs(11.5))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .child(name.clone()),
                        )
                        .when_some(spec.clone(), |element, spec| {
                            element.child(
                                widgets::Button::new(theme, "Install")
                                    .enabled(connected && !loading && install_status == "not-installed")
                                    .build()
                                    .id(gpui::SharedString::from(format!("market-install-{name}")))
                                    .on_click(cx.listener(move |app, _, _, cx| {
                                        app.install_entry(spec.clone());
                                        cx.notify();
                                    })),
                            )
                        })
                        .when(
                            update_available,
                            |element| {
                                element.child(
                                    widgets::Button::new(theme, "Update")
                                        .enabled(connected && !loading)
                                        .build()
                                        .id(gpui::SharedString::from(format!("market-update-{name}")))
                                        .on_click(cx.listener(move |app, _, _, cx| {
                                            if let Some(spec) = &update_spec {
                                                app.update_market_entry(
                                                    spec.clone(),
                                                    name.clone(),
                                                    update_version.clone(),
                                                );
                                            }
                                            cx.notify();
                                        })),
                                )
                            },
                        ),
                )
                .when(!trust_line.is_empty(), |element| {
                    element.child(line(theme, trust_line))
                })
                .when(!description.is_empty(), |element| {
                    element.child(line(theme, description))
                })
                .child(line(theme, permissions_line))
                .when_some(changelog, |element, changelog| {
                    element.child(line(theme, format!("Changes in v{target_version}: {changelog}")))
                }),
        );
    }
    content.into_any_element()
}

fn goals(
    app: &mut DesktopApp,
    theme: &Theme,
    cx: &mut Context<DesktopApp>,
) -> AnyElement {
    let Some(session_id) = app.selected_session.clone() else {
        return body(theme)
            .child(line(theme, "Select a Session to manage its goal."))
            .into_any_element();
    };
    let goal = app.goal.clone();
    let loading = app.goal_loading;
    let mutating = app.goal_mutation_loading;
    let status = app.goal_status.clone();
    let error = app.goal_error.clone();
    let plan_status = app.plan_command_status.clone();
    let handoff_input = app.text_field("handoff-path", "HANDOFF.md", false, cx);
    let handoff_status = app.handoff_status.clone();
    let handoff_preview = app.handoff_preview.clone();
    let handoff_writing = app.handoff_receiver.is_some();
    let handoff_ready = handoff_preview.is_some();
    let plan_loading = app.plan_command_loading;
    let busy = app.chat_loading || app.assistant_streaming;
    let objective_input = app.text_field("goal-objective", "Goal objective", true, cx);
    let rounds_input = app.text_field("goal-rounds", "Maximum rounds", false, cx);
    app.goal_objective = objective_input.read(cx).text().to_owned();
    app.goal_max_rounds = rounds_input.read(cx).text().to_owned();

    let mut content = body(theme)
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(theme.sp(6.))
                .child(
                    widgets::Button::new(theme, "Refresh")
                        .enabled(!loading && !mutating)
                        .build()
                        .id("goal-refresh")
                        .on_click(cx.listener(|app, _, _, cx| {
                            app.refresh_goal();
                            cx.notify();
                        })),
                )
                .child(line(theme, format!("Session: {session_id}")))
                .when_some(status, |element, status| element.child(line(theme, status)))
                .when_some(error.clone(), |element, error| {
                    element.child(
                        div()
                            .text_size(theme.fs(10.5))
                            .text_color(theme.error)
                            .child(error),
                    )
                }),
        );

    if loading && goal.is_none() {
        return content.child(line(theme, "Loading goal…")).into_any_element();
    }

    let input_box = |input: &gpui::Entity<crate::shell::text_input::TextInput>, height: gpui::Pixels| {
        div()
            .w_full()
            .h(height)
            .px(theme.sp(6.))
            .rounded(theme.corner())
            .border_1()
            .border_color(theme.border)
            .bg(theme.code_background)
            .overflow_hidden()
            .child(crate::shell::text_input::text_input_element(
                input,
                theme,
                height - px(2.),
            ))
    };

    match goal.clone() {
        Some(goal) if goal.phase != dsh_client_core::goals::GoalPhase::Complete => {
            content = content
                .child(section(
                    theme,
                    &format!(
                        "{} · {:?} · {:?}",
                        goal.id, goal.phase, goal.activation
                    ),
                ))
                .child(line(
                    theme,
                    format!(
                        "{} of {} rounds used",
                        goal.rounds_started, goal.max_goal_rounds
                    ),
                ))
                .child(input_box(&objective_input, theme.sp(72.)))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(theme.sp(6.))
                        .child(line(theme, "Maximum rounds"))
                        .child(
                            div()
                                .w(theme.sp(90.))
                                .h(theme.control_height())
                                .px(theme.sp(6.))
                                .rounded(theme.corner())
                                .border_1()
                                .border_color(theme.border)
                                .bg(theme.code_background)
                                .overflow_hidden()
                                .child(crate::shell::text_input::text_input_element(
                                    &rounds_input,
                                    theme,
                                    theme.control_height() - px(2.),
                                )),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .gap(theme.sp(6.))
                        .child(
                            widgets::Button::new(theme, "Save changes")
                                .enabled(!mutating)
                                .build()
                                .id("goal-save")
                                .on_click(cx.listener(|app, _, _, cx| {
                                    app.save_goal_edits();
                                    cx.notify();
                                })),
                        )
                        .when(
                            goal.phase == dsh_client_core::goals::GoalPhase::Active
                                && goal.activation
                                    == dsh_client_core::goals::GoalActivation::Armed,
                            |element| {
                                element.child(
                                    widgets::Button::new(theme, "Pause")
                                        .enabled(!mutating)
                                        .build()
                                        .id("goal-pause")
                                        .on_click(cx.listener(|app, _, _, cx| {
                                            app.change_goal(GoalAction::Pause);
                                            cx.notify();
                                        })),
                                )
                            },
                        )
                        .when(
                            goal.phase != dsh_client_core::goals::GoalPhase::Active
                                || goal.activation
                                    == dsh_client_core::goals::GoalActivation::Disarmed,
                            |element| {
                                element.child(
                                    widgets::Button::new(theme, "Resume")
                                        .enabled(!mutating)
                                        .build()
                                        .id("goal-resume")
                                        .on_click(cx.listener(|app, _, _, cx| {
                                            app.change_goal(GoalAction::Resume);
                                            cx.notify();
                                        })),
                                )
                            },
                        )
                        .child(
                            widgets::Button::new(theme, "Complete")
                                .enabled(!mutating)
                                .build()
                                .id("goal-complete")
                                .on_click(cx.listener(|app, _, _, cx| {
                                    app.change_goal(GoalAction::Complete);
                                    cx.notify();
                                })),
                        )
                        .child(
                            widgets::Button::new(theme, "Clear")
                                .danger()
                                .enabled(!mutating)
                                .build()
                                .id("goal-clear")
                                .on_click(cx.listener(|app, _, _, cx| {
                                    app.change_goal(GoalAction::Clear);
                                    cx.notify();
                                })),
                        ),
                );
        }
        _ => {
            if goal
                .as_ref()
                .is_some_and(|goal| goal.phase == dsh_client_core::goals::GoalPhase::Complete)
            {
                content = content
                    .child(line(theme, "The previous goal is complete. Create another goal for this Session."));
            } else {
                content = content.child(line(theme, "No goal is active for this Session."));
            }
            content = content
                .child(input_box(&objective_input, theme.sp(72.)))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(theme.sp(6.))
                        .child(line(theme, "Maximum rounds (optional)"))
                        .child(
                            div()
                                .w(theme.sp(90.))
                                .h(theme.control_height())
                                .px(theme.sp(6.))
                                .rounded(theme.corner())
                                .border_1()
                                .border_color(theme.border)
                                .bg(theme.code_background)
                                .overflow_hidden()
                                .child(crate::shell::text_input::text_input_element(
                                    &rounds_input,
                                    theme,
                                    theme.control_height() - px(2.),
                                )),
                        ),
                )
                .child(
                    widgets::Button::new(theme, "Create goal")
                        .primary()
                        .enabled(!mutating)
                        .build()
                        .id("goal-create")
                        .on_click(cx.listener(|app, _, _, cx| {
                            app.create_goal();
                            cx.notify();
                        })),
                );
        }
    }

    let work_rows = app
        .transcript
        .rows()
        .iter()
        .rev()
        .filter(|row| matches!(row.kind, RowKind::Todo | RowKind::Goal | RowKind::Notice))
        .take(12)
        .cloned()
        .collect::<Vec<_>>();
    content = content
        .child(widgets::separator(theme))
        .child(section(theme, "RECENT TASKS AND PLAN CHANGES"));
    if work_rows.is_empty() {
        content = content.child(line(
            theme,
            "Task and plan updates will appear here when the assistant records them.",
        ));
    }
    for row in work_rows {
        content = content
            .child(
                div()
                    .text_size(theme.fs(11.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child(format!("{} · sequence {}", row.title, row.seq)),
            )
            .child(line(theme, row.text));
    }
    content
        .child(widgets::separator(theme))
        .child(section(theme, "PLAN MODE"))
        .child(
            div()
                .flex()
                .flex_row()
                .gap(theme.sp(6.))
                .child(
                    widgets::Button::new(theme, "Enter plan mode")
                        .enabled(!plan_loading && !busy)
                        .build()
                        .id("plan-enter")
                        .on_click(cx.listener(|app, _, _, cx| {
                            app.run_plan_command(true);
                            cx.notify();
                        })),
                )
                .child(
                    widgets::Button::new(theme, "Leave plan mode")
                        .enabled(!plan_loading && !busy)
                        .build()
                        .id("plan-leave")
                        .on_click(cx.listener(|app, _, _, cx| {
                            app.run_plan_command(false);
                            cx.notify();
                        })),
                ),
        )
        .when_some(plan_status, |element, status| element.child(line(theme, status)))
        .child(widgets::separator(theme))
        .child(section(theme, "HANDOFF"))
        .child(line(
            theme,
            "Summarizes this Session from its log: last turn state, goal, recent requests, recorded tool failures, and artifacts.",
        ))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(theme.sp(6.))
                .child(
                    div()
                        .w(theme.sp(220.))
                        .h(theme.control_height())
                        .px(theme.sp(6.))
                        .rounded(theme.corner())
                        .border_1()
                        .border_color(theme.border)
                        .bg(theme.code_background)
                        .overflow_hidden()
                        .child(crate::shell::text_input::text_input_element(
                            &handoff_input,
                            theme,
                            theme.control_height() - px(2.),
                        )),
                )
                .child(
                    widgets::Button::new(theme, "Preview")
                        .enabled(!handoff_writing)
                        .build()
                        .id("handoff-preview")
                        .on_click(cx.listener(|app, _, _, cx| {
                            app.generate_handoff();
                            cx.notify();
                        })),
                )
                .child(
                    widgets::Button::new(theme, "Save")
                        .primary()
                        .enabled(!handoff_writing && handoff_ready)
                        .build()
                        .id("handoff-save")
                        .on_click(cx.listener(|app, _, _, cx| {
                            app.write_handoff(false);
                            cx.notify();
                        })),
                )
                .child(
                    widgets::Button::new(theme, "Overwrite")
                        .enabled(!handoff_writing && handoff_ready)
                        .build()
                        .id("handoff-overwrite")
                        .on_click(cx.listener(|app, _, _, cx| {
                            app.write_handoff(true);
                            cx.notify();
                        })),
                ),
        )
        .when_some(handoff_status, |element, status| element.child(line(theme, status)))
        .when_some(handoff_preview, |element, preview| {
            element.child(
                div()
                    .id("handoff-preview-text")
                    .max_h(theme.sp(220.))
                    .overflow_y_scroll()
                    .p(theme.sp(8.))
                    .rounded(theme.corner())
                    .bg(theme.code_background)
                    .font(theme.mono_font())
                    .text_size(theme.fs(10.5))
                    .child(preview),
            )
        })
        .into_any_element()
}

fn document(
    app: &mut DesktopApp,
    theme: &Theme,
    cx: &mut Context<DesktopApp>,
) -> AnyElement {
    let loading = app.preview_loading;
    let error = app.preview_error.clone();
    let text = app.preview_document.clone();
    let image = app.preview_image.clone();
    let path = app.preview_path.clone();
    let office = DesktopApp::is_office_path_now(&path);
    let input = app.text_field("document-path", "Host file path", false, cx);
    let rename_input = app.text_field("document-new-path", "New path", false, cx);

    let mut content = body(theme)
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(theme.sp(6.))
                .child(
                    div()
                        .flex_1()
                        .h(theme.control_height())
                        .px(theme.sp(6.))
                        .rounded(theme.corner())
                        .border_1()
                        .border_color(theme.border)
                        .bg(theme.code_background)
                        .overflow_hidden()
                        .child(crate::shell::text_input::text_input_element(
                            &input,
                            theme,
                            theme.control_height() - px(2.),
                        )),
                )
                .child(
                    widgets::Button::new(theme, "Preview")
                        .enabled(!loading)
                        .build()
                        .id("document-preview")
                        .on_click(cx.listener(|app, _, _, cx| {
                            app.read_document();
                            cx.notify();
                        })),
                )
                .when(office, |element| {
                    element.child(
                        widgets::Button::new(theme, "Render Office preview")
                            .enabled(!loading)
                            .build()
                            .id("document-office")
                            .on_click(cx.listener(|app, _, _, cx| {
                                app.read_office_document();
                                cx.notify();
                            })),
                    )
                }),
        )
        .when(loading, |element| element.child(line(theme, "Loading document…")))
        .when_some(error.clone(), |element, error| {
            element.child(
                div()
                    .text_size(theme.fs(11.))
                    .text_color(theme.error)
                    .child(error),
            )
        });

    if let Some(text) = text {
        if let Some(pending) = app.pending_preview_text.take() {
            app.set_input("document-body", &pending, cx);
        }
        let body_input = app.text_field("document-body", "Document text", true, cx);
        if let Some((target, line)) = app.preview_target_line.take() {
            if target == app.preview_path {
                body_input.update(cx, |input, cx| input.reveal_line(line, cx));
            }
        }
        content = content
            .child(
                div()
                    .text_size(theme.fs(13.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child(text.name.clone()),
            )
            .child(line(theme, text.host_path.clone()))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(theme.sp(6.))
                    .child(
                        widgets::Button::new(theme, "Save to Host")
                            .enabled(app.transport.is_some() && app.preview_save_receiver.is_none())
                            .build()
                            .id("document-save-host")
                            .on_click(cx.listener(|app, _, _, cx| {
                                app.save_preview_document();
                                cx.notify();
                            })),
                    )
                    .when_some(app.preview_save_status.clone(), |element, status| {
                        element.child(line(theme, status))
                    }),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(theme.sp(6.))
                    .child(
                        div()
                            .w(theme.sp(220.))
                            .h(theme.control_height())
                            .px(theme.sp(6.))
                            .rounded(theme.corner())
                            .border_1()
                            .border_color(theme.border)
                            .bg(theme.code_background)
                            .overflow_hidden()
                            .child(crate::shell::text_input::text_input_element(
                                &rename_input,
                                theme,
                                theme.control_height() - px(2.),
                            )),
                    )
                    .child(
                        widgets::Button::new(theme, "Rename")
                            .enabled(app.transport.is_some() && app.document_op_receiver.is_none())
                            .build()
                            .id("document-rename")
                            .on_click(cx.listener(|app, _, _, cx| {
                                app.rename_preview_document();
                                cx.notify();
                            })),
                    )
                    .child(
                        widgets::Button::new(
                            theme,
                            if app.document_delete_confirm {
                                "Confirm delete"
                            } else {
                                "Delete"
                            },
                        )
                        .enabled(app.transport.is_some() && app.document_op_receiver.is_none())
                        .build()
                        .id("document-delete")
                        .on_click(cx.listener(|app, _, _, cx| {
                            app.delete_preview_document();
                            cx.notify();
                        })),
                    )
                    .when_some(app.document_op_status.clone(), |element, status| {
                        element.child(line(theme, status))
                    }),
            )
            .child(
                div()
                    .id("document-text")
                    .flex()
                    .flex_col()
                    .w_full()
                    .min_h(theme.sp(200.))
                    .p(theme.sp(8.))
                    .rounded(theme.corner())
                    .border_1()
                    .border_color(theme.border)
                    .bg(theme.code_background)
                    .font(theme.mono_font())
                    .overflow_hidden()
                    .child(crate::shell::text_input::text_input_element(
                        &body_input,
                        theme,
                        theme.sp(200.),
                    )),
            );
    } else if let Some(image) = image {
    } else if let Some(image) = image {
        content = content
            .child(
                div()
                    .text_size(theme.fs(13.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child(image.name.clone()),
            )
            .child(line(theme, image.host_path.clone()))
            .child(line(
                theme,
                format!("{} × {} pixels", image.size[0], image.size[1]),
            ))
            .child(line(
                theme,
                "The decoded image is ready; scaled rendering is pending the gpui image pipeline.",
            ));
    } else if !loading && error.is_none() {
        content = content.child(line(theme, "No document is loaded."));
    }
    content.into_any_element()
}

/// Keeps the layout module's constants referenced from one place.
#[allow(dead_code)]
fn explorer_min() -> f32 {
    layout::EXPLORER_MIN
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command_registry::CommandId;
    use crate::harness::Harness;

    /// Selectors whose geometry must not move while the overlay is open.
    const SHELL_SELECTORS: [&str; 6] = [
        "shell-root",
        "top-bar",
        "explorer-panel",
        "agent-column",
        "changes-panel",
        "bottom-bar",
    ];

    #[test]
    fn preferences_overlay_centers_over_an_unchanged_shell() {
        let mut harness = Harness::open(1366., 768.);
        let before = harness.rect_report(&SHELL_SELECTORS);

        harness.run_command(CommandId::ToggleSettings);
        assert!(harness.with(|app, _| app.show_settings));

        let root = harness.bounds("shell-root").expect("shell root renders");
        let window = harness
            .bounds("preferences-window")
            .expect("Preferences window renders");
        assert_eq!(f32::from(window.size.width), 910.);
        assert_eq!(f32::from(window.size.height), 680.);
        let centered_x = f32::from(root.origin.x)
            + (f32::from(root.size.width) - f32::from(window.size.width)) / 2.;
        let centered_y = f32::from(root.origin.y)
            + (f32::from(root.size.height) - f32::from(window.size.height)) / 2.;
        assert!((f32::from(window.origin.x) - centered_x).abs() <= 1.);
        assert!((f32::from(window.origin.y) - centered_y).abs() <= 1.);

        // The shell keeps its geometry and stays visible behind the backdrop.
        let after = harness.rect_report(&SHELL_SELECTORS);
        assert_eq!(before, after);
        assert!(harness.bounds("explorer-panel").is_some());
        assert!(harness.bounds("agent-column").is_some());
        assert!(harness.bounds("changes-panel").is_some());

        // Only the section body scrolls; the window itself stays fixed.
        let main = harness
            .bounds("preferences-main")
            .expect("section body renders");
        assert!(f32::from(main.size.height) < f32::from(window.size.height));
    }

    #[test]
    fn preferences_navigation_lists_every_reference_section() {
        let mut harness = Harness::open(1366., 768.);
        harness.run_command(CommandId::ToggleSettings);
        for selector in [
            "preferences-nav-General",
            "preferences-nav-Appearance",
            "preferences-nav-Keymap",
            "preferences-nav-Extensions",
            "preferences-nav-Skills",
            "preferences-nav-Marketplace",
            "preferences-nav-About",
        ] {
            assert!(
                harness.bounds(selector).is_some(),
                "{selector} did not render"
            );
        }
    }

    #[test]
    fn escape_and_the_backdrop_close_preferences() {
        let mut harness = Harness::open(1366., 768.);
        harness.run_command(CommandId::ToggleSettings);
        harness.keys("escape");
        assert!(!harness.with(|app, _| app.show_settings));

        harness.run_command(CommandId::ToggleSettings);
        harness.click_at(5., 5.);
        assert!(!harness.with(|app, _| app.show_settings));
    }

    #[test]
    fn preferences_restores_focus_to_the_input_that_opened_it() {
        let mut harness = Harness::open(1366., 768.);
        // The Document page renders its path input without a Host connection.
        harness.with(|app, cx| {
            app.switch_view(ActiveView::Document);
            cx.notify();
        });
        harness.settle();
        harness.focus_input("document-path");
        assert!(harness.input_focused("document-path"));

        harness.run_command(CommandId::ToggleSettings);
        assert!(harness.preferences_focused());
        assert!(!harness.input_focused("document-path"));

        harness.keys("escape");
        assert!(!harness.with(|app, _| app.show_settings));
        assert!(harness.input_focused("document-path"));
    }

    #[test]
    fn preferences_sections_keep_the_existing_actions() {
        let mut harness = Harness::open(1366., 768.);
        harness.run_command(CommandId::ToggleSettings);

        harness.click("preferences-nav-Keymap");
        assert_eq!(
            harness.with(|app, _| app.preferences_section),
            PreferencesSection::Keymap
        );
        assert!(harness.click("pref-keymap-zed"));
        assert_eq!(
            harness.with(|app, _| app.ui_preferences.keymap_preset),
            KeymapPreset::Zed
        );

        harness.click("preferences-nav-General");
        let restore = harness.with(|app, _| app.ui_preferences.restore_last_session);
        harness.click("pref-restore-session");
        assert_eq!(
            harness.with(|app, _| app.ui_preferences.restore_last_session),
            !restore
        );
        assert!(harness.with(|app, _| app.show_settings));
    }

    /// A valid user pack based on the shipped DSH document.
    fn user_theme_document(id: &str, name: &str) -> String {
        let mut document: serde_json::Value =
            serde_json::from_str(include_str!("../../theme/packs/dsh.json")).unwrap();
        document["id"] = serde_json::json!(id);
        document["name"] = serde_json::json!(name);
        document["source"] = serde_json::json!("user");
        serde_json::to_string_pretty(&document).unwrap()
    }

    fn install_test_theme(harness: &mut Harness, id: &str, name: &str) -> std::path::PathBuf {
        harness.with(|app, _| {
            std::fs::create_dir_all(&app.ui_themes_path).expect("theme test directory");
            let path = app.ui_themes_path.join(format!("{id}.json"));
            std::fs::write(&path, user_theme_document(id, name)).expect("user theme file");
            app.theme_catalog = crate::theme::ThemeCatalog::load(&app.ui_themes_path);
            path
        })
    }

    #[test]
    fn preferences_lists_installed_theme_packs_and_opens_the_editor() {
        let mut harness = Harness::open(1366., 768.);
        install_test_theme(&mut harness, "forest", "Forest");
        harness.run_command(CommandId::ToggleSettings);
        harness.click("preferences-nav-Appearance");

        for selector in [
            "pref-theme-row-forest",
            "pref-theme-source-forest",
            "pref-theme-contrast-forest",
            "pref-theme-row-dsh",
            "pref-theme-add",
            "pref-theme-edit-forest",
            "pref-theme-remove-forest",
        ] {
            assert!(
                harness.bounds(selector).is_some(),
                "{selector} did not render"
            );
        }

        harness.click("pref-theme-edit-forest");
        assert!(harness.with(|app, _| app.theme_editor.is_some()));
        assert!(harness.bounds("pref-theme-editor").is_some());
        assert!(harness.bounds("pref-theme-save").is_some());
        // The editor panel scrolls below the first viewport; the render test
        // checks it is present, then closes it through the same state action.
        harness.with(|app, cx| {
            app.cancel_theme_editor();
            cx.notify();
        });
        assert!(harness.with(|app, _| app.theme_editor.is_none()));
        assert!(harness.with(|app, _| app.ui_themes_path.join("forest.json").exists()));
    }

    #[test]
    fn removing_a_user_theme_requires_confirmation() {
        let mut harness = Harness::open(1366., 768.);
        let path = install_test_theme(&mut harness, "forest", "Forest");
        harness.run_command(CommandId::ToggleSettings);
        harness.click("preferences-nav-Appearance");

        harness.click("pref-theme-remove-forest");
        assert!(path.exists());
        assert_eq!(
            harness.with(|app, _| app.theme_remove_confirm.clone()),
            Some("forest".to_owned())
        );

        // The confirmation row sits below the first viewport; drive the same
        // action with the window so the second click's state path still runs.
        harness.with_window(|app, window, _| app.remove_theme("forest", window));
        assert!(!path.exists());
        assert!(harness.with(|app, _| {
            app.theme_remove_confirm.is_none() && app.theme_catalog.find("forest").is_none()
        }));
    }

    #[test]
    fn preferences_window_fits_the_reference_insets_on_a_small_viewport() {
        let mut harness = Harness::open(960., 620.);
        harness.run_command(CommandId::ToggleSettings);
        let root = harness.bounds("shell-root").expect("shell root renders");
        let window = harness
            .bounds("preferences-window")
            .expect("Preferences window renders");
        assert_eq!(f32::from(window.size.width), 900.);
        assert_eq!(f32::from(window.size.height), 550.);
        assert!(window.origin.x >= root.origin.x);
        assert!(window.origin.y >= root.origin.y);
        assert!(window.origin.x + window.size.width <= root.origin.x + root.size.width);
        assert!(window.origin.y + window.size.height <= root.origin.y + root.size.height);
    }
}
