//! Shell layout: top bar, Explorer, Agent column, utility column, and bottom bar.
//!
//! The V2 composition mirrors the reference shell: a thin hamburger-led top
//! bar, an icon-first navigation rail, the Session list, the conversation with
//! its composer, an optional workspace Explorer, and a compact tool bar. Pages
//! replace the conversation pane, and transient dialogs paint above everything.

mod agent;
mod explorer_panel;
mod pages;
mod sessions;

use gpui::{div, prelude::*, StatefulInteractiveElement, px, AnyElement, Context, SharedString, Window};

use crate::command_registry::CommandId;
use crate::layout;
use crate::shell::app::{ActiveView, BottomTool, DesktopApp};
use crate::shell::theme::Theme;
use super::widgets;

/// Draws the whole window.
pub(crate) fn render(
    app: &mut DesktopApp,
    window: &mut Window,
    cx: &mut Context<DesktopApp>,
) -> AnyElement {
    app.sync_inputs(cx);
    let theme = app.theme.clone();
    let viewport_width: f32 = window.viewport_size().width.into();

    let dock_explorer = app.active_view == ActiveView::Sessions
        && app.show_explorer
        && !app.explorer_fullscreen
        && viewport_width >= layout::EXPLORER_DOCK_MIN_WIDTH;

    let central = if app.explorer_fullscreen {
        explorer_panel::fullscreen(app, &theme, viewport_width, cx)
    } else if let Some(page) = pages::active_page(app, &theme, cx) {
        page
    } else {
        agent::conversation(app, window, &theme, viewport_width, cx)
    };

    let on_page = app.show_settings
        || app.show_goals
        || app.show_plugins
        || app.show_market
        || app.show_skills
        || app.show_document;
    let sessions_panel = if on_page {
        None
    } else {
        Some(sessions::panel(app, &theme, cx))
    };

    let body = div()
        .flex()
        .flex_row()
        .flex_1()
        .min_h(px(0.))
        .w_full()
        .child(nav_rail(app, &theme, cx))
        .children(sessions_panel)
        .child(div().flex().flex_col().flex_1().min_w(px(260.)).min_h(px(0.)).child(central))
        .children(dock_explorer.then(|| {
            div()
                .flex()
                .flex_col()
                .w(theme.sp(app.ui_preferences.workspace_panel_width))
                .min_w(theme.sp(layout::EXPLORER_MIN))
                .max_w(theme.sp(layout::EXPLORER_MAX))
                .min_h(px(0.))
                .border_l_1()
                .border_color(theme.border)
                .bg(theme.background)
                .child(explorer_panel::tree_and_preview(
                    app,
                    &theme,
                    viewport_width <= layout::EXPLORER_SINGLE_PANE_MAX_WIDTH,
                    cx,
                ))
        }));

    let mut root = div()
        .relative()
        .flex()
        .flex_col()
        .size_full()
        .bg(theme.background)
        .text_color(theme.text)
        .font(theme.sans_font())
        .text_size(theme.fs(12.5))
        .child(top_bar(app, &theme, cx))
        .child(body)
        .child(bottom_bar(app, &theme, cx));

    if app.top_menu_open {
        root = root.child(top_menu(&theme, cx));
    }
    if let Some(prompt) = app.update_prompt.clone() {
        root = root.child(update_prompt(app, &theme, &prompt, cx));
    }
    if app.clear_goal_confirmation.is_some() {
        root = root.child(clear_goal_dialog(&theme, cx));
    }
    root.into_any_element()
}

fn top_bar(
    app: &mut DesktopApp,
    theme: &Theme,
    cx: &mut Context<DesktopApp>,
) -> AnyElement {
    let title: SharedString = if app.session_title.is_empty() {
        app.selected_session
            .clone()
            .unwrap_or_else(|| "DeepSeek Harness".to_owned())
            .into()
    } else {
        app.session_title.clone().into()
    };
    let connected = app.transport.is_some();
    let state_color = if connected { theme.accent } else { theme.warn };
    let menu_open = app.top_menu_open;

    div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(theme.sp(8.))
        .h(theme.sp(layout::TOP_BAR_HEIGHT))
        .px(theme.sp(8.))
        .border_b_1()
        .border_color(theme.border)
        .bg(theme.surface)
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(theme.sp(6.))
                .child(
                    widgets::Button::new(theme, "☰")
                        .enabled(true)
                        .build()
                        .id("top-menu")
                        .cursor_pointer()
                        .on_click(cx.listener(move |app, _, _, cx| {
                            app.top_menu_open = !menu_open;
                            cx.notify();
                        })),
                )
                .child(
                    div()
                        .text_size(theme.fs(11.))
                        .text_color(theme.muted)
                        .child("Workspace"),
                ),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .flex()
                .justify_center()
                .text_size(theme.fs(11.5))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child(div().truncate().child(title)),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(theme.sp(5.))
                .child(div().size(px(6.)).rounded_full().bg(state_color))
                .child(
                    div()
                        .text_size(theme.fs(10.))
                        .text_color(theme.muted)
                        .child(if connected { "Host connected" } else { "Host offline" }),
                )
                .child(
                    widgets::Button::new(theme, "Search ⌕")
                        .build()
                        .id("top-search")
                        .cursor_pointer()
                        .on_click(cx.listener(|app, _, _, cx| {
                            app.active_view = ActiveView::Sessions;
                            app.focus_composer = false;
                            cx.notify();
                        })),
                ),
        )
        .into_any_element()
}

fn top_menu(
    theme: &Theme,
    cx: &mut Context<DesktopApp>,
) -> AnyElement {
    let row = |label: &str| {
        div()
            .px(theme.sp(10.))
            .py(theme.sp(5.))
            .rounded(theme.corner())
            .hover(|style| style.bg(theme.surface_raised))
            .cursor_pointer()
            .text_size(theme.fs(11.5))
            .child(label.to_owned())
    };
    div()
        .absolute()
        .inset_0()
        .child(
            div()
                .absolute()
                .top(theme.sp(layout::TOP_BAR_HEIGHT + 2.))
                .left(theme.sp(8.))
                .w(theme.sp(210.))
                .flex()
                .flex_col()
                .p(theme.sp(4.))
                .gap(px(1.))
                .rounded(theme.corner())
                .border_1()
                .border_color(theme.border)
                .bg(theme.surface_raised)
                .shadow_lg()
                .child(widgets::section_label(theme, "WORKSPACE"))
                .child(row("New session").id("menu-new-session").on_click(cx.listener(
                    |app, _, _, cx| {
                        app.run_command(CommandId::NewSession);
                        app.top_menu_open = false;
                        cx.notify();
                    },
                )))
                .child(row("Toggle Explorer").id("menu-explorer").on_click(cx.listener(
                    |app, _, _, cx| {
                        app.run_command(CommandId::ToggleExplorer);
                        app.top_menu_open = false;
                        cx.notify();
                    },
                )))
                .child(widgets::section_label(theme, "CUSTOMIZE"))
                .child(row("Preferences…").id("menu-settings").on_click(cx.listener(
                    |app, _, _, cx| {
                        app.run_command(CommandId::ToggleSettings);
                        app.top_menu_open = false;
                        cx.notify();
                    },
                )))
                .child(row("Extensions").id("menu-plugins").on_click(cx.listener(
                    |app, _, _, cx| {
                        app.run_command(CommandId::TogglePlugins);
                        app.top_menu_open = false;
                        cx.notify();
                    },
                )))
                .child(row("Skills").id("menu-skills").on_click(cx.listener(
                    |app, _, _, cx| {
                        app.run_command(CommandId::ToggleSkills);
                        app.top_menu_open = false;
                        cx.notify();
                    },
                )))
                .child(row("Marketplace").id("menu-market").on_click(cx.listener(
                    |app, _, _, cx| {
                        app.run_command(CommandId::ToggleMarketplace);
                        app.top_menu_open = false;
                        cx.notify();
                    },
                )))
                .child(row("Goals").id("menu-goals").on_click(cx.listener(
                    |app, _, _, cx| {
                        app.run_command(CommandId::ToggleGoals);
                        app.top_menu_open = false;
                        cx.notify();
                    },
                )))
                .child(widgets::section_label(theme, "APPLICATION"))
                .child(row("Quit").id("menu-quit").on_click(cx.listener(
                    |app, _, _, cx| {
                        app.quitting
                            .store(true, std::sync::atomic::Ordering::Release);
                        app.close_requested = true;
                        cx.notify();
                    },
                ))),
        )
        .into_any_element()
}

fn nav_rail(
    app: &mut DesktopApp,
    theme: &Theme,
    cx: &mut Context<DesktopApp>,
) -> AnyElement {
    let items = [
        ("▤", "Sessions", ActiveView::Sessions),
        ("▱", "Explorer", ActiveView::Explorer),
        ("▥", "Files", ActiveView::Document),
        ("◈", "Plugins", ActiveView::Plugins),
        ("✧", "Skills", ActiveView::Skills),
        ("◍", "Market", ActiveView::Marketplace),
        ("◎", "Goals", ActiveView::Goals),
        ("⚙", "Settings", ActiveView::Settings),
    ];
    let active = app.active_view;
    let mut rail = div()
        .flex()
        .flex_col()
        .items_center()
        .w(theme.sp(layout::NAV_RAIL_WIDTH))
        .min_w(theme.sp(layout::NAV_RAIL_WIDTH))
        .h_full()
        .pt(theme.sp(8.))
        .gap(theme.sp(4.))
        .border_r_1()
        .border_color(theme.border)
        .bg(theme.surface)
        .child(
            div()
                .pb(theme.sp(6.))
                .text_size(theme.fs(11.))
                .font_weight(gpui::FontWeight::BOLD)
                .text_color(theme.accent)
                .child("DSH"),
        );
    for (glyph, label, view) in items {
        let selected = active == view;
        let color = if selected { theme.accent } else { theme.muted };
        rail = rail.child(
            div()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .w(theme.sp(66.))
                .h(theme.sp(44.))
                .rounded(theme.corner())
                .id(gpui::SharedString::from(format!("nav-{}", view as u8)))
                .cursor_pointer()
                .when(selected, |element| element.bg(theme.selection))
                .hover(|style| style.bg(theme.surface_raised))
                .child(
                    div()
                        .text_size(theme.fs(15.))
                        .text_color(color)
                        .child(glyph),
                )
                .child(
                    div()
                        .text_size(theme.fs(9.))
                        .text_color(color)
                        .child(label),
                )
                .on_click(cx.listener(move |app, _, _, cx| {
                    app.switch_view(view);
                    cx.notify();
                })),
        );
    }
    rail.into_any_element()
}

fn bottom_bar(
    app: &mut DesktopApp,
    theme: &Theme,
    cx: &mut Context<DesktopApp>,
) -> AnyElement {
    let selected_session = app.selected_session.is_some();
    let status = app.status.clone();
    let bar = div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .h(theme.sp(layout::BOTTOM_BAR_HEIGHT))
        .px(theme.sp(10.))
        .border_t_1()
        .border_color(theme.border)
        .bg(theme.surface)
        .text_size(theme.fs(10.))
        .text_color(theme.muted)
        .child(div().truncate().child(status))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(theme.sp(4.))
                .children((0..4).map(|index| {
                    let (tool, label, glyph) = [
                        (BottomTool::Terminal, "Terminal", "▭"),
                        (BottomTool::Jobs, "Jobs", "◷"),
                        (BottomTool::Schedules, "Schedules", "◔"),
                        (BottomTool::Deliverables, "Deliverables", "▣"),
                    ][index];
                    let active = app.bottom_tool == Some(tool);
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(theme.sp(4.))
                        .px(theme.sp(7.))
                        .py(px(2.))
                        .rounded(theme.corner())
                        .id(("bottom-tool", index))
                        .cursor_pointer()
                        .when(active, |element| element.bg(theme.selection))
                        .when(!selected_session, |element| element.opacity(0.45))
                        .hover(|style| style.bg(theme.surface_raised))
                        .text_color(if active { theme.accent } else { theme.text })
                        .child(glyph)
                        .child(label)
                        .on_click(cx.listener(move |app, _, _, cx| {
                            if app.selected_session.is_none() {
                                return;
                            }
                            app.bottom_tool = if app.bottom_tool == Some(tool) {
                                None
                            } else {
                                Some(tool)
                            };
                            cx.notify();
                        }))
                })),
        );
    bar.into_any_element()
}

fn update_prompt(
    app: &mut DesktopApp,
    theme: &Theme,
    prompt: &crate::host::UpdatePrompt,
    cx: &mut Context<DesktopApp>,
) -> AnyElement {
    let version = prompt.version.clone();
    let action = prompt.action.clone();
    let error = app.update_error.clone();
    let status = app.update_status.clone();
    dialog_shell(theme, "Software update", |body| {
        body.child(
            div()
                .flex()
                .flex_col()
                .gap(theme.gap())
                .child(
                    div()
                        .text_size(theme.fs(12.))
                        .child(format!(
                            "Version {version} is available ({action}). Confirm the download or installation."
                        )),
                )
                .when_some(error.clone(), |element, error| {
                    element.child(
                        div()
                            .text_size(theme.fs(11.))
                            .text_color(theme.error)
                            .child(error),
                    )
                })
                .when_some(status.clone(), |element, status| {
                    element.child(
                        div()
                            .text_size(theme.fs(11.))
                            .text_color(theme.muted)
                            .child(status),
                    )
                })
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .justify_end()
                        .gap(theme.sp(6.))
                        .child(
                            widgets::Button::new(theme, "Later")
                                .build()
                                .id("update-later")
                                .cursor_pointer()
                                .on_click(cx.listener(|app, _, _, cx| {
                                    app.update_prompt = None;
                                    cx.notify();
                                })),
                        )
                        .child(
                            widgets::Button::new(theme, "Open download")
                                .primary()
                                .build()
                                .id("update-open")
                                .cursor_pointer()
                                .on_click(cx.listener(|app, _, _, _cx| {
                                    app.open_updates();
                                })),
                        ),
                ),
        )
    })
}

fn clear_goal_dialog(
    theme: &Theme,
    cx: &mut Context<DesktopApp>,
) -> AnyElement {
    dialog_shell(theme, "Clear current goal?", |body| {
        body.child(
            div()
                .flex()
                .flex_col()
                .gap(theme.gap())
                .child(
                    div()
                        .text_size(theme.fs(11.5))
                        .text_color(theme.muted)
                        .child(
                            "The goal will be removed from this Session. Its history remains in the session log.",
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .justify_end()
                        .gap(theme.sp(6.))
                        .child(
                            widgets::Button::new(theme, "Keep goal")
                                .build()
                                .id("clear-goal-cancel")
                                .cursor_pointer()
                                .on_click(cx.listener(|app, _, _, cx| {
                                    app.clear_goal_confirmation = None;
                                    cx.notify();
                                })),
                        )
                        .child(
                            widgets::Button::new(theme, "Clear goal")
                                .danger()
                                .build()
                                .id("clear-goal-confirm")
                                .cursor_pointer()
                                .on_click(cx.listener(|app, _, _, cx| {
                                    app.confirm_clear_goal();
                                    cx.notify();
                                })),
                        ),
                ),
        )
    })
}

/// Centers a modal card over a dimmed backdrop; the caller fills the card body.
fn dialog_shell(
    theme: &Theme,
    title: &str,
    body: impl FnOnce(gpui::Div) -> gpui::Div,
) -> AnyElement {
    div()
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .bg(gpui::black().opacity(0.35))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(theme.gap())
                .w(theme.sp(520.))
                .p(theme.sp(16.))
                .rounded(theme.corner())
                .border_1()
                .border_color(theme.border)
                .bg(theme.surface_raised)
                .shadow_lg()
                .child(
                    div()
                        .text_size(theme.fs(14.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child(title.to_owned()),
                )
                .child(body(div())),
        )
        .into_any_element()
}
