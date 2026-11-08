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

use gpui::{
    div, prelude::*, px, AnyElement, ClipboardItem, Context, CursorStyle, ExternalPaths,
    MouseButton, MouseDownEvent, MouseMoveEvent, StatefulInteractiveElement, Window,
};

use super::git_review;
use super::git_review::short_commit;
use super::widgets;
use crate::command_registry::CommandId;
use crate::layout;
use crate::shell::app::{
    ActiveView, BottomTool, ChangesTab, DesktopApp, PaletteEntry, ResizeTarget,
    MAX_TERMINAL_PANES, TERMINAL_INPUT_KEYS, TERMINAL_TITLE_KEYS,
};
use crate::shell::theme::Theme;

/// Draws the whole window.
pub(crate) fn render(
    app: &mut DesktopApp,
    window: &mut Window,
    cx: &mut Context<DesktopApp>,
) -> AnyElement {
    app.sync_inputs(cx);
    let theme = app.theme.clone();
    let viewport_width: f32 = window.viewport_size().width.into();

    let on_page = app.show_settings
        || app.show_goals
        || app.show_plugins
        || app.show_market
        || app.show_skills
        || app.show_document;
    let show_agent = !on_page && !app.explorer_fullscreen;
    let docks = layout::dock_visibility(viewport_width, app.show_explorer, app.show_changes);
    let dock_explorer = show_agent && docks.explorer;
    let dock_changes = show_agent && docks.changes;

    let central = if app.explorer_fullscreen {
        explorer_panel::fullscreen(app, &theme, viewport_width, cx)
    } else if let Some(page) = pages::active_page(app, &theme, cx) {
        page
    } else {
        agent_column(app, window, &theme, viewport_width, cx)
    };

    let body = div()
        .flex()
        .flex_row()
        .flex_1()
        .min_h(px(0.))
        .w_full()
        .children(dock_explorer.then(|| {
            div()
                .flex()
                .flex_col()
                .debug_selector(|| "explorer-panel".to_owned())
                .w(theme.sp(app.ui_preferences.workspace_prefs(app.selected_workspace.as_deref()).workspace_panel_width.unwrap_or(app.ui_preferences.workspace_panel_width)))
                .min_w(theme.sp(layout::EXPLORER_MIN))
                .max_w(theme.sp(layout::EXPLORER_MAX))
                .min_h(px(0.))
                .border_r_1()
                .border_color(theme.border)
                .bg(theme.background)
                .child(
                    div()
                        .flex_1()
                        .min_h(px(0.))
                        .child(explorer_panel::tree_and_preview(
                            app,
                            &theme,
                            viewport_width <= layout::EXPLORER_SINGLE_PANE_MAX_WIDTH,
                            cx,
                        )),
                )
                .child(explorer_footer(app, &theme, cx))
        }))
        .children(dock_explorer.then(|| panel_divider(&theme, ResizeTarget::Explorer, cx)))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w(px(260.))
                .min_h(px(0.))
                .debug_selector(|| "agent-column".to_owned())
                .child(central),
        )
        .children(dock_changes.then(|| panel_divider(&theme, ResizeTarget::Changes, cx)))
        .children(dock_changes.then(|| git_review::changes_column(app, &theme, cx)));

    let utility = app
        .bottom_tool
        .map(|tool| utility_panel(app, &theme, tool, cx));

    let shell = cx.entity();
    let mut root = div()
        .relative()
        .flex()
        .flex_col()
        .size_full()
        .debug_selector(|| "shell-root".to_owned())
        .bg(theme.background)
        .text_color(theme.text)
        .font(theme.sans_font())
        .text_size(theme.fs(12.5))
        .child(top_bar(app, &theme, cx))
        .child(body)
        .children(utility)
        .child(bottom_bar(app, &theme, cx))
        .can_drop(|value, _, _| value.is::<ExternalPaths>())
        .on_drop(cx.listener(|app, paths: &ExternalPaths, _, cx| {
            app.attach_dropped_paths(paths.paths());
            cx.notify();
        }))
        .on_action(move |_: &crate::shell::Dismiss, _, cx| {
            shell.update(cx, |app, cx| {
                if app.dismiss_overlay() {
                    cx.notify();
                }
            });
        });

    if app.panel_drag.is_some() {
        root = root.child(panel_drag_overlay(cx));
    }
    if app.show_about {
        root = root.child(about_dialog(app, &theme, cx));
    }
    if app.palette_open {
        root = root.child(command_palette(app, window, &theme, cx));
    }

    if app.top_menu_open {
        root = root.child(top_menu(app, &theme, cx));
    }
    if app.show_session_menu {
        root = root.child(session_popover(app, &theme, cx));
    }
    if let Some(prompt) = app.update_prompt.clone() {
        root = root.child(update_prompt(app, &theme, &prompt, cx));
    }
    if app.clear_goal_confirmation.is_some() {
        root = root.child(clear_goal_dialog(&theme, cx));
    }
    if let Some(confirmation) = app.git_confirmation.clone() {
        root = root.child(git_review::confirmation_dialog(app, &theme, confirmation, cx));
    }
    root.into_any_element()
}

/// Wraps the conversation in the V2 Agent tab strip.
fn agent_column(
    app: &mut DesktopApp,
    window: &mut Window,
    theme: &Theme,
    viewport_width: f32,
    cx: &mut Context<DesktopApp>,
) -> AnyElement {
    let title = if app.session_title.is_empty() {
        app.selected_session.clone().unwrap_or_default()
    } else {
        app.session_title.clone()
    };
    let running = app.selected_session_running();
    let tab_ids = app.agent_tabs.clone();
    let mut tabs = div()
        .flex()
        .flex_row()
        .items_center()
        .h_full()
        .min_w(px(0.))
        .overflow_hidden();
    for tab_id in tab_ids {
        let active = app.selected_session.as_deref() == Some(tab_id.as_str());
        let label = app.agent_tab_label(&tab_id);
        let switch_id = tab_id.clone();
        let close_id = tab_id.clone();
        tabs = tabs.child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .h_full()
                .border_b_1()
                .border_color(if active { theme.accent } else { theme.border })
                .bg(if active { theme.background } else { theme.surface })
                .when(active, |element| {
                    element.debug_selector(|| "agent-tab-active".to_owned())
                })
                .child(
                    div()
                        .id(gpui::SharedString::from(format!("agent-tab-label-{tab_id}")))
                        .flex()
                        .items_center()
                        .h_full()
                        .px(theme.sp(8.))
                        .text_size(theme.fs(10.5))
                        .text_color(if active { theme.text } else { theme.muted })
                        .truncate()
                        .child(label)
                        .on_click(cx.listener(move |app, _, _, cx| {
                            app.load_session(switch_id.clone());
                            cx.notify();
                        })),
                )
                .child(
                    div()
                        .id(gpui::SharedString::from(format!("agent-tab-close-{tab_id}")))
                        .flex()
                        .items_center()
                        .h_full()
                        .px(theme.sp(5.))
                        .text_size(theme.fs(10.))
                        .text_color(theme.muted)
                        .child("×")
                        .on_click(cx.listener(move |app, _, _, cx| {
                            app.close_agent_tab(&close_id);
                            cx.notify();
                        })),
                ),
        );
    }
    let strip = div()
        .flex()
        .flex_row()
        .items_end()
        .justify_between()
        .h(theme.sp(layout::AGENT_TAB_HEIGHT))
        .px(theme.sp(8.))
        .border_b_1()
        .border_color(theme.border)
        .bg(theme.surface)
        .child(
            div()
                .debug_selector(|| "agent-tab-strip".to_owned())
                .flex()
                .flex_row()
                .items_center()
                .h_full()
                .min_w(px(0.))
                .overflow_hidden()
                .child(tabs),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(theme.sp(6.))
                .pb(theme.sp(6.))
                .child(
                    div()
                        .text_size(theme.fs(10.))
                        .text_color(theme.muted)
                        .truncate()
                        .child(title),
                )
                .when(running, |element| {
                    element.child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(theme.sp(3.))
                            .text_size(theme.fs(9.5))
                            .text_color(theme.accent)
                            .child("● Running"),
                    )
                }),
        );
    div()
        .flex()
        .flex_col()
        .size_full()
        .min_h(px(0.))
        .child(strip)
        .child(agent::conversation(app, window, theme, viewport_width, cx))
        .into_any_element()
}

fn top_bar(app: &mut DesktopApp, theme: &Theme, cx: &mut Context<DesktopApp>) -> AnyElement {
    let title = if app.session_title.is_empty() {
        app.selected_session.clone().unwrap_or_default()
    } else {
        app.session_title.clone()
    };
    let menu_open = app.top_menu_open;
    let session_menu_open = app.show_session_menu;
    let changes_open = app.show_changes;
    let workspace = workspace_label(app);

    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(theme.sp(6.))
        .debug_selector(|| "top-bar".to_owned())
        .h(theme.sp(layout::TOP_BAR_HEIGHT))
        .px(theme.sp(8.))
        .border_b_1()
        .border_color(theme.border)
        .bg(theme.surface)
        .child(
            widgets::Button::new(theme, "☰")
                .enabled(true)
                .build()
                .id("top-menu")
                .cursor_pointer()
                .on_click(cx.listener(move |app, _, _, cx| {
                    app.top_menu_open = !menu_open;
                    app.show_session_menu = false;
                    cx.notify();
                })),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(theme.sp(4.))
                .px(theme.sp(6.))
                .h(theme.control_height())
                .rounded(theme.corner())
                .cursor_pointer()
                .hover(|style| style.bg(theme.surface_raised))
                .id("top-workspace")
                .on_click(cx.listener(move |app, _, _, cx| {
                    app.show_session_menu = !session_menu_open;
                    app.top_menu_open = false;
                    cx.notify();
                }))
                .child(
                    div()
                        .text_size(theme.fs(11.))
                        .text_color(theme.text)
                        .truncate()
                        .child(workspace),
                )
                .child(
                    div()
                        .text_size(theme.fs(9.))
                        .text_color(theme.muted)
                        .child("▾"),
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
                .text_color(theme.muted)
                .child(div().truncate().child(title)),
        )
        .child(
            widgets::Button::new(theme, "Search ⌕")
                .build()
                .id("top-search")
                .cursor_pointer()
                .on_click(cx.listener(move |app, _, _, cx| {
                    app.show_session_menu = !session_menu_open;
                    app.top_menu_open = false;
                    cx.notify();
                })),
        )
        .child(
            widgets::Button::new(
                theme,
                if changes_open {
                    "Changes ●"
                } else {
                    "Changes"
                },
            )
            .build()
            .id("top-changes")
            .debug_selector(|| "toggle-changes".to_owned())
            .cursor_pointer()
            .on_click(cx.listener(|app, _, _, cx| {
                app.run_command(CommandId::ToggleChanges);
                cx.notify();
            })),
        )
        .into_any_element()
}

/// Display name of the selected Session's workspace directory.
fn workspace_label(app: &DesktopApp) -> String {
    workspace_name(
        app.selected_session
            .as_ref()
            .and_then(|id| {
                app.sessions
                    .iter()
                    .find(|summary| &summary.session_id == id)
            })
            .and_then(|summary| summary.cwd.as_deref()),
    )
    .to_owned()
}

/// Last path segment of a workspace path, with a neutral label when there is none.
fn workspace_name(cwd: Option<&str>) -> &str {
    cwd.and_then(|cwd| cwd.trim_end_matches(['/', '\\']).rsplit(['/', '\\']).next())
        .filter(|name| !name.is_empty())
        .unwrap_or("No workspace")
}

#[cfg(test)]
mod tests {
    use super::{diagnostic_paths, workspace_name};

    #[test]
    fn diagnostic_uris_map_to_workspace_relative_paths() {
        assert_eq!(
            diagnostic_paths("file:///work/src/a.ts", "/work"),
            ("src/a.ts".to_owned(), Some("src/a.ts".to_owned()))
        );
        assert_eq!(
            diagnostic_paths("file:///work/a%20b.ts", "/work"),
            ("a b.ts".to_owned(), Some("a b.ts".to_owned()))
        );
        assert_eq!(
            diagnostic_paths("file:///elsewhere/a.ts", "/work"),
            ("/elsewhere/a.ts".to_owned(), Some("/elsewhere/a.ts".to_owned()))
        );
    }

    #[test]
    fn workspace_name_reports_the_last_directory_segment() {
        assert_eq!(workspace_name(Some("/home/dev/project/")), "project");
        assert_eq!(workspace_name(Some("C:\\work\\project")), "project");
        assert_eq!(workspace_name(Some("relative")), "relative");
    }

    #[test]
    fn workspace_name_falls_back_when_no_directory_is_known() {
        assert_eq!(workspace_name(None), "No workspace");
        assert_eq!(workspace_name(Some("")), "No workspace");
        assert_eq!(workspace_name(Some("/")), "No workspace");
    }
}

fn top_menu(app: &mut DesktopApp, theme: &Theme, cx: &mut Context<DesktopApp>) -> AnyElement {
    let shortcuts: Vec<String> = CommandId::MENU
        .iter()
        .map(|command| {
            app.ui_preferences
                .keybindings
                .chord_for(*command)
                .unwrap_or_default()
        })
        .collect();

    let mut entries = div().flex().flex_col().gap(px(1.)).p(theme.sp(4.));
    let mut section = "";
    for (command, shortcut) in CommandId::MENU.into_iter().zip(shortcuts) {
        if command.group() != section {
            section = command.group();
            entries = entries.child(widgets::section_label(theme, &section.to_uppercase()));
        }
        let hint = if shortcut.is_empty() || shortcut.eq_ignore_ascii_case("none") {
            String::new()
        } else {
            shortcut
        };
        entries = entries.child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .gap(theme.sp(16.))
                .px(theme.sp(10.))
                .py(theme.sp(5.))
                .rounded(theme.corner())
                .hover(|style| style.bg(theme.surface_raised))
                .cursor_pointer()
                .text_size(theme.fs(11.5))
                .id(gpui::SharedString::from(format!(
                    "menu-{}",
                    command.label()
                )))
                .child(command.label())
                .child(
                    div()
                        .text_size(theme.fs(9.5))
                        .text_color(theme.muted)
                        .child(hint),
                )
                .on_click(cx.listener(move |app, _, _, cx| {
                    app.run_command(command);
                    app.top_menu_open = false;
                    cx.notify();
                })),
        );
    }

    div()
        .absolute()
        .inset_0()
        .child(
            div()
                .absolute()
                .inset_0()
                .id("menu-backdrop")
                .on_click(cx.listener(|app, _, _, cx| {
                    app.top_menu_open = false;
                    cx.notify();
                })),
        )
        .child(
            div()
                .absolute()
                .top(theme.sp(layout::TOP_BAR_HEIGHT + 2.))
                .left(theme.sp(8.))
                .w(theme.sp(230.))
                .rounded(theme.corner())
                .border_1()
                .border_color(theme.border)
                .bg(theme.surface_raised)
                .shadow_lg()
                .child(entries),
        )
        .into_any_element()
}

/// Bottom utility panel for Terminal, Jobs, Schedules, and Deliverables.
fn utility_panel(
    app: &mut DesktopApp,
    theme: &Theme,
    tool: BottomTool,
    cx: &mut Context<DesktopApp>,
) -> AnyElement {
    let (title, count) = match tool {
        BottomTool::Terminal => ("Terminal", app.terminal.items.len()),
        BottomTool::Jobs => ("Jobs", app.jobs.len()),
        BottomTool::Schedules => ("Schedules", app.schedules.len()),
        BottomTool::Deliverables => ("Deliverables", app.deliverables.len()),
        BottomTool::Diagnostics => ("Diagnostics", app.diagnostics.len()),
    };
    let header = div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .px(theme.sp(10.))
        .py(theme.sp(4.))
        .border_b_1()
        .border_color(theme.border)
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(theme.sp(6.))
                .child(
                    div()
                        .text_size(theme.fs(11.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child(format!("{title} ({count})")),
                ),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(theme.sp(4.))
                .child(
                    div()
                        .id("utility-close")
                        .cursor_pointer()
                        .px(theme.sp(4.))
                        .rounded(theme.corner())
                        .hover(|style| style.bg(theme.surface_raised))
                        .child("×")
                        .on_click(cx.listener(|app, _, _, cx| {
                            app.bottom_tool = None;
                            cx.notify();
                        })),
                ),
        );

    let body = match tool {
        BottomTool::Jobs => jobs_panel(app, theme, cx),
        BottomTool::Schedules => schedules_panel(app, theme, cx),
        BottomTool::Deliverables => deliverables_panel(app, theme, cx),
        BottomTool::Diagnostics => diagnostics_panel(app, theme, cx),
        BottomTool::Terminal => terminal_panel(app, theme, cx),
    };

    div()
        .flex()
        .flex_col()
        .w_full()
        .h(theme.sp(252.))
        .min_h(px(0.))
        .border_t_1()
        .border_color(theme.border)
        .bg(theme.surface)
        .child(header)
        .child(body)
        .into_any_element()
}

fn utility_action(
    theme: &Theme,
    id: &'static str,
    label: &str,
    cx: &mut Context<DesktopApp>,
    action: impl Fn(&mut DesktopApp) + 'static,
) -> AnyElement {
    div()
        .id(id)
        .cursor_pointer()
        .px(theme.sp(6.))
        .py(px(1.))
        .rounded(theme.corner())
        .text_color(theme.text)
        .hover(|style| style.bg(theme.surface_raised))
        .child(label.to_owned())
        .on_click(cx.listener(move |app, _, _, cx| {
            action(app);
            cx.notify();
        }))
        .into_any_element()
}

/// Like [`utility_action`], for an action that needs the entity context.
fn utility_action_with_cx(
    theme: &Theme,
    id: &'static str,
    label: &str,
    cx: &mut Context<DesktopApp>,
    action: impl Fn(&mut DesktopApp, &mut Context<DesktopApp>) + 'static,
) -> AnyElement {
    div()
        .id(id)
        .cursor_pointer()
        .px(theme.sp(6.))
        .py(px(1.))
        .rounded(theme.corner())
        .text_color(theme.text)
        .hover(|style| style.bg(theme.surface_raised))
        .child(label.to_owned())
        .on_click(cx.listener(move |app, _, _, cx| {
            action(app, cx);
            cx.notify();
        }))
        .into_any_element()
}

fn jobs_panel(
    app: &mut DesktopApp,
    theme: &Theme,
    cx: &mut Context<DesktopApp>,
) -> AnyElement {
    let rows: Vec<(String, String, String)> = app
        .jobs
        .iter()
        .map(|job| (job.id.clone(), job.label.clone(), format!("{} · {}", job.kind, job.status)))
        .collect();
    let selected = app.job_selected.clone();
    let output = app.job_output.clone();
    let status = app.job_status.clone();
    let mut list = div().flex().flex_col().gap(px(1.)).w_full();
    for (id, label, meta) in rows {
        let is_selected = selected.as_deref() == Some(id.as_str());
        list = list.child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .w_full()
                .px(theme.sp(8.))
                .py(px(1.))
                .rounded(theme.corner())
                .when(is_selected, |element| element.bg(theme.selection))
                .hover(|style| style.bg(theme.surface_raised))
                .id(gpui::SharedString::from(format!("job-{id}")))
                .cursor_pointer()
                .child(
                    div()
                        .text_size(theme.fs(10.5))
                        .truncate()
                        .child(label),
                )
                .child(
                    div()
                        .text_size(theme.fs(9.5))
                        .text_color(theme.muted)
                        .child(meta),
                )
                .on_click(cx.listener(move |app, _, _, cx| {
                    app.start_job_follow(id.clone());
                    cx.notify();
                })),
        );
    }
    if app.jobs.is_empty() {
        list = list.child(
            div()
                .px(theme.sp(8.))
                .py(theme.sp(4.))
                .text_size(theme.fs(10.))
                .text_color(theme.muted)
                .child(if app.jobs_receiver.is_some() {
                    "Loading jobs…"
                } else {
                    "No jobs for this Session."
                }),
        );
    }
    div()
        .flex()
        .flex_col()
        .gap(px(2.))
        .flex_1()
        .min_h(px(0.))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(theme.sp(6.))
                .px(theme.sp(8.))
                .child(div().flex_1().text_size(theme.fs(10.)).text_color(theme.muted).child("Background work for this Session"))
                .child(utility_action(theme, "jobs-refresh", "↻", cx, |app| {
                    app.jobs_session = None;
                    app.start_job_list();
                }))
                .when_some(app.jobs_error.clone(), |element, error| {
                    element.child(div().text_size(theme.fs(10.)).text_color(theme.error).child(error))
                }),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .px(theme.sp(8.))
                .max_h(theme.sp(84.))
                .id("utility-scroll").overflow_y_scroll()
                .child(list),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_h(px(0.))
                .border_t_1()
                .border_color(theme.border)
                .px(theme.sp(8.))
                .py(px(2.))
                .when(!status.is_empty(), |element| {
                    element.child(
                        div()
                            .text_size(theme.fs(9.5))
                            .text_color(theme.muted)
                            .child(status),
                    )
                })
                .child(
                    div()
                        .flex_1()
                        .min_h(px(0.))
                        .id("utility-scroll").overflow_y_scroll()
                        .font(theme.mono_font())
                        .text_size(theme.fs(10.))
                        .child(if output.is_empty() {
                            "Select a job to follow its output.".to_owned()
                        } else {
                            output
                        }),
                ),
        )
        .into_any_element()
}

fn schedules_panel(
    app: &mut DesktopApp,
    theme: &Theme,
    cx: &mut Context<DesktopApp>,
) -> AnyElement {
    let rows: Vec<(String, String)> = app
        .schedules
        .iter()
        .map(|record| {
            let id = record
                .get("id")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_owned();
            let title = record
                .get("title")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
                .unwrap_or_else(|| id.clone());
            let kind = record
                .get("kind")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("schedule");
            let next = record
                .get("scheduledAt")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            (id, format!("{title} · {kind} · {next}"))
        })
        .collect();
    let mut list = div().flex().flex_col().gap(px(2.)).w_full();
    for (id, label) in rows {
        let delete_id = id.clone();
        list = list.child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .w_full()
                .px(theme.sp(8.))
                .py(px(2.))
                .border_b_1()
                .border_color(theme.border)
                .child(
                    div()
                        .text_size(theme.fs(10.5))
                        .truncate()
                        .child(label),
                )
                .child(utility_action(theme, "schedule-delete", "Delete", cx, move |app| {
                    app.delete_schedule(delete_id.clone());
                })),
        );
    }
    if app.schedules.is_empty() {
        list = list.child(
            div()
                .px(theme.sp(8.))
                .py(theme.sp(4.))
                .text_size(theme.fs(10.))
                .text_color(theme.muted)
                .child(if app.schedules_receiver.is_some() {
                    "Loading schedules…"
                } else {
                    "No schedules for this Session."
                }),
        );
    }
    div()
        .flex()
        .flex_col()
        .gap(px(2.))
        .flex_1()
        .min_h(px(0.))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(theme.sp(6.))
                .px(theme.sp(8.))
                .child(div().flex_1().text_size(theme.fs(10.)).text_color(theme.muted).child("Scheduled actions for this Session"))
                .child(utility_action(theme, "schedules-refresh", "↻", cx, |app| {
                    app.schedules_session = None;
                    app.start_schedules();
                }))
                .when_some(app.schedules_error.clone(), |element, error| {
                    element.child(div().text_size(theme.fs(10.)).text_color(theme.error).child(error))
                }),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_h(px(0.))
                .id("utility-scroll").overflow_y_scroll()
                .child(list),
        )
        .into_any_element()
}

fn deliverables_panel(
    app: &mut DesktopApp,
    theme: &Theme,
    cx: &mut Context<DesktopApp>,
) -> AnyElement {
    let files: Vec<(String, Option<String>)> = app
        .deliverables
        .iter()
        .map(|file| (file.path.clone(), file.description.clone()))
        .collect();
    let mut list = div().flex().flex_col().gap(px(2.)).w_full();
    for (path, description) in files {
        let open_path = path.clone();
        list = list.child(
            div()
                .flex()
                .flex_col()
                .w_full()
                .px(theme.sp(8.))
                .py(px(2.))
                .id(gpui::SharedString::from(format!("deliverable-{path}")))
                .cursor_pointer()
                .hover(|style| style.bg(theme.surface_raised))
                .child(
                    div()
                        .font(theme.mono_font())
                        .text_size(theme.fs(10.5))
                        .text_color(theme.accent)
                        .child(path),
                )
                .when_some(description, |element, description| {
                    element.child(
                        div()
                            .text_size(theme.fs(9.5))
                            .text_color(theme.muted)
                            .child(description),
                    )
                })
                .on_click(cx.listener(move |app, _, _, cx| {
                    app.preview_path = open_path.clone();
                    app.switch_view(ActiveView::Document);
                    if DesktopApp::is_office_path_now(&open_path) {
                        app.read_office_document();
                    } else {
                        app.read_document();
                    }
                    cx.notify();
                })),
        );
    }
    if app.deliverables.is_empty() {
        list = list.child(
            div()
                .px(theme.sp(8.))
                .py(theme.sp(4.))
                .text_size(theme.fs(10.))
                .text_color(theme.muted)
                .child("Files presented by the assistant appear here."),
        );
    }
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.))
        .id("utility-scroll").overflow_y_scroll()
        .child(list)
        .into_any_element()
}

/// Decode a `file:` URI into a host path, tolerating percent escapes.
fn file_uri_path(uri: &str) -> Option<String> {
    let rest = uri.strip_prefix("file://")?;
    let bytes = rest.as_bytes();
    let mut decoded = String::with_capacity(rest.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).ok()?;
            let value = u8::from_str_radix(hex, 16).ok()?;
            decoded.push(char::from(value));
            index += 3;
        } else {
            decoded.push(char::from(bytes[index]));
            index += 1;
        }
    }
    Some(decoded)
}

/// Workspace-relative display and open path for one diagnostic URI.
fn diagnostic_paths(uri: &str, workspace: &str) -> (String, Option<String>) {
    let Some(path) = file_uri_path(uri) else {
        return (uri.to_owned(), None);
    };
    if workspace.is_empty() {
        return (path.clone(), Some(path));
    }
    let prefix = format!("{}/", workspace.trim_end_matches('/'));
    match path.strip_prefix(&prefix) {
        Some(relative) => (relative.to_owned(), Some(relative.to_owned())),
        None => (path.clone(), Some(path)),
    }
}

struct DiagnosticRow {
    display: String,
    open_path: Option<String>,
    line: u32,
    character: u32,
    severity: String,
    message: String,
    source: String,
}

fn diagnostics_panel(
    app: &mut DesktopApp,
    theme: &Theme,
    cx: &mut Context<DesktopApp>,
) -> AnyElement {
    let workspace = app.diagnostics_workspace.clone().unwrap_or_default();
    let items: Vec<DiagnosticRow> = app
        .diagnostics
        .iter()
        .map(|item| {
            let (display, open_path) = diagnostic_paths(&item.uri, &workspace);
            DiagnosticRow {
                display,
                open_path,
                line: item.range.start.line,
                character: item.range.start.character,
                severity: item.severity.clone(),
                message: item.message.clone(),
                source: item
                    .source
                    .clone()
                    .or_else(|| item.code.clone())
                    .unwrap_or_default(),
            }
        })
        .collect();
    let mut list = div().flex().flex_col().gap(px(2.)).w_full().px(theme.sp(8.)).py(theme.sp(4.));
    list = list.child(
        div()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .child(
                div()
                    .text_size(theme.fs(9.5))
                    .text_color(theme.muted)
                    .child(if app.diagnostics_loading {
                        "Reading diagnostics…".to_owned()
                    } else if workspace.is_empty() {
                        "No workspace directory for this Session.".to_owned()
                    } else {
                        format!("Workspace {workspace}")
                    }),
            )
            .child(
                div()
                    .id("diagnostics-refresh")
                    .debug_selector(|| "diagnostics-refresh".to_owned())
                    .cursor_pointer()
                    .px(theme.sp(5.))
                    .rounded(theme.corner())
                    .hover(|style| style.bg(theme.surface_raised))
                    .child("Refresh")
                    .on_click(cx.listener(|app, _, _, cx| {
                        app.refresh_diagnostics();
                        cx.notify();
                    })),
            ),
    );
    if let Some(error) = app.diagnostics_error.clone() {
        list = list.child(
            div()
                .text_size(theme.fs(10.))
                .text_color(theme.error)
                .child(error),
        );
    }
    if items.is_empty() && !app.diagnostics_loading && app.diagnostics_error.is_none() {
        list = list.child(
            div()
                .text_size(theme.fs(10.))
                .text_color(theme.muted)
                .child("No diagnostics. Configured language servers publish after they open a file."),
        );
    }
    for row in items {
        let color = match row.severity.as_str() {
            "error" => theme.error,
            "warning" => theme.warn,
            "information" => theme.accent,
            _ => theme.muted,
        };
        let glyph = match row.severity.as_str() {
            "error" => "●",
            "warning" => "▲",
            "information" => "ℹ",
            _ => "·",
        };
        let open_path = row.open_path.clone();
        let line = row.line;
        list = list.child(
            div()
                .flex()
                .flex_row()
                .items_start()
                .gap(theme.sp(6.))
                .w_full()
                .px(theme.sp(2.))
                .py(px(2.))
                .rounded(theme.corner())
                .id(gpui::SharedString::from(format!(
                    "diagnostic-{}:{}:{}",
                    row.display, row.line, row.character
                )))
                .hover(|style| style.bg(theme.surface_raised))
                .child(
                    div()
                        .text_size(theme.fs(9.5))
                        .text_color(color)
                        .child(glyph),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .gap(theme.sp(6.))
                                .font(theme.mono_font())
                                .text_size(theme.fs(9.5))
                                .text_color(theme.accent)
                                .child(format!(
                                    "{}:{}:{}",
                                    row.display,
                                    row.line + 1,
                                    row.character + 1
                                ))
                                .child(
                                    div()
                                        .text_size(theme.fs(9.))
                                        .text_color(theme.muted)
                                        .child(row.source),
                                ),
                        )
                        .child(
                            div()
                                .text_size(theme.fs(10.))
                                .text_color(theme.text)
                                .child(row.message),
                        ),
                )
                .on_click(cx.listener(move |app, _, _, cx| {
                    let Some(open_path) = open_path.clone() else {
                        return;
                    };
                    app.preview_path = open_path.clone();
                    app.preview_target_line = Some((open_path, u64::from(line) + 1));
                    app.switch_view(ActiveView::Document);
                    app.read_document();
                    cx.notify();
                })),
        );
    }
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.))
        .id("utility-scroll")
        .debug_selector(|| "diagnostics-body".to_owned())
        .overflow_y_scroll()
        .child(list)
        .into_any_element()
}

fn terminal_panel(
    app: &mut DesktopApp,
    theme: &Theme,
    cx: &mut Context<DesktopApp>,
) -> AnyElement {
    let pane_count = app.terminal_pane_count();
    for index in 0..pane_count {
        if let Some(panel) = app.terminal_pane_mut(index) {
            panel.refresh_screen();
        }
    }
    let items: Vec<(String, String, String)> = app
        .terminal
        .items
        .iter()
        .map(|item| {
            (
                item.id.clone(),
                item.title.clone(),
                format!("{:?} · {}×{}", item.state, item.cols, item.rows),
            )
        })
        .collect();
    let focus = app.terminal_focus;
    let selected = app
        .terminal_pane_at(focus)
        .and_then(|panel| panel.selected.clone());
    let raw = app
        .terminal_pane_at(focus)
        .is_some_and(|panel| panel.raw_keys);
    let mut list = div().flex().flex_row().items_center().gap(theme.sp(4.)).flex_wrap();
    for (id, title, meta) in items {
        let active = selected.as_deref() == Some(id.as_str());
        let target = id.clone();
        list = list.child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(theme.sp(4.))
                .px(theme.sp(6.))
                .py(px(1.))
                .rounded(theme.corner())
                .when(active, |element| element.bg(theme.selection))
                .hover(|style| style.bg(theme.surface_raised))
                .id(gpui::SharedString::from(format!("terminal-{id}")))
                .cursor_pointer()
                .child(
                    div()
                        .text_size(theme.fs(10.5))
                        .child(format!("{title}  {meta}")),
                )
                .on_click(cx.listener(move |app, _, _, cx| {
                    let index = app.terminal_focus;
                    if let Some(panel) = app.terminal_pane_mut(index) {
                        panel.select_terminal(target.clone());
                    }
                    cx.notify();
                })),
        );
    }
    let mut panes: Vec<AnyElement> = Vec::new();
    for index in 0..pane_count {
        panes.push(terminal_pane(app, theme, cx, index));
        if index + 1 < pane_count {
            panes.push(terminal_divider(theme, index, cx));
        }
    }
    let remove_pane = if focus > 0 {
        Some(utility_action(
            theme,
            "terminal-remove-pane",
            "Remove pane",
            cx,
            |app| app.remove_terminal_pane(app.terminal_focus),
        ))
    } else {
        None
    };
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.))
        .px(theme.sp(8.))
        .gap(px(2.))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(theme.sp(4.))
                .flex_wrap()
                .child(utility_action(theme, "terminal-refresh", "Refresh", cx, |app| {
                    if let Some(transport) = app.transport.clone() {
                        app.terminal.request_list(transport.clone());
                        for pane in &mut app.terminal_extra {
                            pane.request_list(transport.clone());
                        }
                    }
                }))
                .child(utility_action(theme, "terminal-create", "New", cx, |app| {
                    if let Some(transport) = app.transport.clone() {
                        let index = app.terminal_focus;
                        if let Some(panel) = app.terminal_pane_mut(index) {
                            panel.create_default(transport);
                        }
                    }
                }))
                .child(utility_action(
                    theme,
                    "terminal-add-pane",
                    if pane_count >= MAX_TERMINAL_PANES {
                        "Pane limit reached"
                    } else {
                        "Add pane"
                    },
                    cx,
                    |app| app.add_terminal_pane(),
                ))
                .children(remove_pane)
                .child(utility_action_with_cx(
                    theme,
                    "terminal-raw-keys",
                    if raw { "Raw keys: on" } else { "Raw keys: off" },
                    cx,
                    |app, cx| app.toggle_terminal_raw_keys(cx),
                ))
                .child(
                    div()
                        .text_size(theme.fs(9.5))
                        .text_color(theme.muted)
                        .child(if raw {
                            "Raw keys send every keystroke immediately, including arrows, Home/End, PageUp/PageDown, Delete, and F1–F12. Click the pane's input line first."
                        } else {
                            "Type a line in the pane's input and press Enter to send it; the tab strip fills the focused pane."
                        }),
                ),
        )
        .child(list)
        .child(
            div()
                .flex()
                .flex_row()
                .flex_1()
                .min_h(px(0.))
                .gap(theme.sp(6.))
                .children(panes),
        )
        .into_any_element()
}

/// One terminal pane: its state line, retained screen, and input row.
fn terminal_pane(
    app: &mut DesktopApp,
    theme: &Theme,
    cx: &mut Context<DesktopApp>,
    index: usize,
) -> AnyElement {
    let input = app.text_field(TERMINAL_INPUT_KEYS[index], "Terminal input", false, cx);
    let title_input = app.text_field(TERMINAL_TITLE_KEYS[index], "Terminal title", false, cx);
    let title_value = title_input.read(cx).text().to_owned();
    let pane_count = app.terminal_pane_count();
    let Some(panel) = app.terminal_pane_at(index) else {
        return div().into_any_element();
    };
    let lifecycle = panel.lifecycle_label();
    let status = panel.status.clone();
    let error = panel.error.clone();
    let label = if panel.title.is_empty() {
        panel
            .selected
            .clone()
            .unwrap_or_else(|| "No terminal".to_owned())
    } else {
        panel.title.clone()
    };
    let focused = index == app.terminal_focus;
    let fraction = app.terminal_pane_fraction(index);
    let output = panel
        .parsed
        .iter()
        .enumerate()
        .map(|(line_index, line)| {
            let spans: Vec<AnyElement> = line
                .spans
                .iter()
                .map(|span| {
                    let color = span.style.fg.map_or(theme.text, |color| {
                        gpui::rgba(u32::from_be_bytes([
                            color.r, color.g, color.b, color.a,
                        ]))
                        .into()
                    });
                    div()
                        .text_color(color)
                        .when(span.style.bold, |element| {
                            element.font_weight(gpui::FontWeight::BOLD)
                        })
                        .when(span.style.italic, |element| element.italic())
                        .child(span.text.clone())
                        .into_any_element()
                })
                .collect();
            div()
                .flex()
                .flex_row()
                .h(px(14.))
                .when(panel.line_selected(line_index), |element| {
                    element.bg(theme.selection)
                })
                .id(gpui::SharedString::from(format!(
                    "terminal-line-{index}-{line_index}"
                )))
                .cursor_pointer()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |app, event: &MouseDownEvent, _, cx| {
                        if let Some(panel) = app.terminal_pane_mut(index) {
                            if event.modifiers.shift {
                                panel.extend_selection(line_index);
                            } else {
                                panel.begin_selection(line_index);
                            }
                            panel.status = "Output lines selected · Copy copies them".to_owned();
                        }
                        cx.notify();
                    }),
                )
                .children(if spans.is_empty() {
                    vec![div().child(" ").into_any_element()]
                } else {
                    spans
                })
                .into_any_element()
        })
        .collect::<Vec<_>>();
    div()
        .flex()
        .flex_col()
        .flex_basis(gpui::relative(fraction))
        .flex_shrink()
        .min_w(px(0.))
        .min_h(px(0.))
        .gap(px(2.))
        .when(pane_count > 1 && focused, |element| {
            element
                .rounded(theme.corner())
                .p(theme.sp(3.))
                .bg(theme.surface)
        })
        .id(gpui::SharedString::from(format!("terminal-pane-{index}")))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(theme.sp(4.))
                .flex_wrap()
                .cursor_pointer()
                .id(gpui::SharedString::from(format!(
                    "terminal-pane-focus-{index}"
                )))
                .on_click(cx.listener(move |app, _, _, cx| {
                    app.focus_terminal_pane(index);
                    cx.notify();
                }))
                .child(
                    div()
                        .text_size(theme.fs(9.5))
                        .text_color(if focused { theme.accent } else { theme.muted })
                        .child(format!("Pane {} · {label} · {lifecycle}", index + 1)),
                )
                .child(utility_action_with_cx(
                    theme,
                    TERMINAL_PANE_IDS[index].7,
                    "Copy",
                    cx,
                    move |app, cx| {
                        if let Some(panel) = app.terminal_pane_mut(index) {
                            match panel.selected_output() {
                                Some(text) => {
                                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                                    panel.status = "Copied selected lines".to_owned();
                                }
                                None => {
                                    panel.status = "Select output lines before copying".to_owned();
                                }
                            }
                        }
                    },
                ))
                .child(utility_action(
                    theme,
                    TERMINAL_PANE_IDS[index].0,
                    "Attach",
                    cx,
                    move |app| {
                        if let Some(streams) = app.streams.clone() {
                            if let Some(panel) = app.terminal_pane_mut(index) {
                                panel.attach_selected(streams);
                            }
                        }
                    },
                ))
                .child(utility_action(
                    theme,
                    TERMINAL_PANE_IDS[index].1,
                    "Detach",
                    cx,
                    move |app| {
                        if let Some(panel) = app.terminal_pane_mut(index) {
                            panel.detach();
                        }
                    },
                ))
                .child(utility_action(
                    theme,
                    TERMINAL_PANE_IDS[index].2,
                    "Close",
                    cx,
                    move |app| {
                        if let Some(transport) = app.transport.clone() {
                            if let Some(panel) = app.terminal_pane_mut(index) {
                                panel.close_selected(transport);
                            }
                        }
                    },
                ))
                .when(index > 0, |element| {
                    element.child(utility_action(
                        theme,
                        TERMINAL_PANE_IDS[index].3,
                        "Remove pane",
                        cx,
                        move |app| app.remove_terminal_pane(index),
                    ))
                }),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(theme.sp(6.))
                .text_size(theme.fs(9.5))
                .text_color(theme.muted)
                .child(status)
                .when_some(error, |element, error| {
                    element.child(div().text_color(theme.error).child(error))
                }),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_h(px(0.))
                .id(gpui::SharedString::from(format!(
                    "terminal-scroll-{index}"
                )))
                .overflow_y_scroll()
                .font(theme.mono_font())
                .text_size(theme.fs(9.5))
                .bg(theme.code_background)
                .children(output),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(theme.sp(4.))
                .child(crate::shell::text_input::text_input_element(
                    &input,
                    theme,
                    theme.sp(if pane_count > 1 { 80. } else { 120. }),
                ))
                .child(utility_action(
                    theme,
                    TERMINAL_PANE_IDS[index].4,
                    "Ctrl+C",
                    cx,
                    move |app| {
                        if let Some(panel) = app.terminal_pane_mut(index) {
                            panel.send_encoded("\u{3}");
                        }
                    },
                ))
                .child(utility_action(
                    theme,
                    TERMINAL_PANE_IDS[index].5,
                    "Esc",
                    cx,
                    move |app| {
                        if let Some(panel) = app.terminal_pane_mut(index) {
                            panel.send_encoded("\u{1b}");
                        }
                    },
                ))
                .child(title_input)
                .child(utility_action(
                    theme,
                    TERMINAL_PANE_IDS[index].6,
                    "Rename",
                    cx,
                    move |app| {
                        if let Some(transport) = app.transport.clone() {
                            if let Some(panel) = app.terminal_pane_mut(index) {
                                panel.rename_selected(transport, title_value.clone());
                            }
                        }
                    },
                )),
        )
        .into_any_element()
}

/// Widget ids for the per-pane actions, indexed by pane.
const TERMINAL_PANE_IDS: [(&str, &str, &str, &str, &str, &str, &str, &str); MAX_TERMINAL_PANES] = [
    (
        "terminal-attach-0",
        "terminal-detach-0",
        "terminal-close-0",
        "terminal-remove-0",
        "terminal-ctrl-c-0",
        "terminal-esc-0",
        "terminal-rename-0",
        "terminal-copy-0",
    ),
    (
        "terminal-attach-1",
        "terminal-detach-1",
        "terminal-close-1",
        "terminal-remove-1",
        "terminal-ctrl-c-1",
        "terminal-esc-1",
        "terminal-rename-1",
        "terminal-copy-1",
    ),
    (
        "terminal-attach-2",
        "terminal-detach-2",
        "terminal-close-2",
        "terminal-remove-2",
        "terminal-ctrl-c-2",
        "terminal-esc-2",
        "terminal-rename-2",
        "terminal-copy-2",
    ),
    (
        "terminal-attach-3",
        "terminal-detach-3",
        "terminal-close-3",
        "terminal-remove-3",
        "terminal-ctrl-c-3",
        "terminal-esc-3",
        "terminal-rename-3",
        "terminal-copy-3",
    ),
];

fn bottom_bar(app: &mut DesktopApp, theme: &Theme, cx: &mut Context<DesktopApp>) -> AnyElement {
    let selected_session = app.selected_session.is_some();
    let status = app.status.clone();
    let connected = app.transport.is_some();
    let active_tool = app.bottom_tool;
    let git_label = match (&app.git_status, app.git_error.as_deref(), app.git_loading) {
        (Some(status), _, _) => match (&status.branch, status.detached) {
            (Some(branch), _) => format!("⎇ {branch}"),
            (None, true) => format!(
                "⎇ detached {}",
                status.commit.as_deref().map(short_commit).unwrap_or("HEAD")
            ),
            _ => "⎇ No branch".to_owned(),
        },
        (None, Some("No repository"), _) => "⎇ No repository".to_owned(),
        (None, Some(_), _) => "⎇ Git unavailable".to_owned(),
        (None, None, true) => "⎇ Reading repository…".to_owned(),
        (None, None, false) if selected_session => "⎇ Repository unknown".to_owned(),
        (None, None, false) => "⎇ No workspace".to_owned(),
    };
    let git_label = match app.git_status.as_ref().map(|status| status.files.len()) {
        Some(count) if count > 0 => format!("{git_label} · {count} changed"),
        _ => git_label,
    };
    let tools = [
        (BottomTool::Jobs, "Jobs", "◷"),
        (BottomTool::Schedules, "Schedules", "◔"),
        (BottomTool::Deliverables, "Deliverables", "▣"),
        (BottomTool::Diagnostics, "Diagnostics", "⚠"),
        (BottomTool::Terminal, "Terminal", "▭"),
    ];

    div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .debug_selector(|| "bottom-bar".to_owned())
        .h(theme.sp(layout::BOTTOM_BAR_HEIGHT))
        .px(theme.sp(8.))
        .border_t_1()
        .border_color(theme.border)
        .bg(theme.surface)
        .text_size(theme.fs(10.))
        .text_color(theme.muted)
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(theme.sp(2.))
                .child(
                    div()
                        .px(theme.sp(5.))
                        .truncate()
                        .cursor_pointer()
                        .rounded(theme.corner())
                        .hover(|style| style.bg(theme.surface_raised))
                        .id("bottom-git-branch")
                        .child(git_label)
                        .on_click(cx.listener(|app, _, _, cx| {
                            app.show_changes = true;
                            app.changes_tab = ChangesTab::Branches;
                            app.refresh_git_branches();
                            cx.notify();
                        })),
                )
                .child(
                    div()
                        .px(theme.sp(5.))
                        .rounded(theme.corner())
                        .cursor_pointer()
                        .hover(|style| style.bg(theme.surface_raised))
                        .id("bottom-git")
                        .child("Git")
                        .on_click(cx.listener(|app, _, _, cx| {
                            app.show_changes = true;
                            app.changes_tab = ChangesTab::Files;
                            cx.notify();
                        })),
                )
                .child(
                    div()
                        .px(theme.sp(5.))
                        .opacity(0.45)
                        .id("bottom-sync")
                        .child("Sync"),
                ),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .px(theme.sp(8.))
                .truncate()
                .child(status),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(theme.sp(2.))
                .children(tools.map(|(tool, label, glyph)| {
                    let active = active_tool == Some(tool);
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .px(theme.sp(5.))
                        .py(px(1.))
                        .rounded(theme.corner())
                        .id(gpui::SharedString::from(format!("bottom-{label}")))
                        .cursor_pointer()
                        .when(active, |element| element.bg(theme.selection))
                        .when(!selected_session, |element| element.opacity(0.45))
                        .hover(|style| style.bg(theme.surface_raised))
                        .text_color(if active { theme.accent } else { theme.text })
                        .child(glyph)
                        .on_click(cx.listener(move |app, _, _, cx| {
                            if app.selected_session.is_none() {
                                return;
                            }
                            app.bottom_tool = if app.bottom_tool == Some(tool) {
                                None
                            } else {
                                Some(tool)
                            };
                            if app.bottom_tool == Some(BottomTool::Diagnostics) {
                                app.refresh_diagnostics();
                            }
                            cx.notify();
                        }))
                }))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(theme.sp(4.))
                        .px(theme.sp(6.))
                        .child(div().size(px(6.)).rounded_full().bg(if connected {
                            theme.accent
                        } else {
                            theme.warn
                        }))
                        .child(div().text_size(theme.fs(9.5)).child(if connected {
                            "Host connected"
                        } else {
                            "Host offline"
                        })),
                ),
        )
        .into_any_element()
}

/// Four-pixel grab area that resizes one docked panel.
fn panel_divider(
    theme: &Theme,
    target: ResizeTarget,
    cx: &mut Context<DesktopApp>,
) -> AnyElement {
    div()
        .w(px(4.))
        .h_full()
        .cursor(CursorStyle::ResizeColumn)
        .hover(|style| style.bg(theme.border))
        .id(gpui::SharedString::from(match target {
            ResizeTarget::Explorer => "divider-explorer".to_owned(),
            ResizeTarget::Changes => "divider-changes".to_owned(),
            ResizeTarget::TerminalPane(boundary) => format!("divider-terminal-{boundary}"),
        }))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |app, event: &MouseDownEvent, _, cx| {
                app.begin_panel_drag(target, f32::from(event.position.x));
                cx.notify();
            }),
        )
        .into_any_element()
}

/// Four-pixel grab area that resizes the two terminal panes beside it.
fn terminal_divider(
    theme: &Theme,
    boundary: usize,
    cx: &mut Context<DesktopApp>,
) -> AnyElement {
    div()
        .w(px(4.))
        .h_full()
        .cursor(CursorStyle::ResizeColumn)
        .hover(|style| style.bg(theme.border))
        .id(gpui::SharedString::from(format!(
            "terminal-divider-{boundary}"
        )))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |app, event: &MouseDownEvent, _, cx| {
                app.begin_panel_drag(ResizeTarget::TerminalPane(boundary), f32::from(event.position.x));
                cx.notify();
            }),
        )
        .into_any_element()
}

/// Full-window layer that keeps receiving pointer moves while a panel is resized.
fn panel_drag_overlay(cx: &mut Context<DesktopApp>) -> AnyElement {
    div()
        .absolute()
        .inset_0()
        .cursor(CursorStyle::ResizeColumn)
        .id("panel-drag-overlay")
        .on_mouse_move(cx.listener(|app, event: &MouseMoveEvent, _, cx| {
            app.drag_panel(f32::from(event.position.x));
            cx.notify();
        }))
        .on_mouse_up(
            MouseButton::Left,
            cx.listener(|app, _, _, cx| {
                app.finish_panel_drag();
                cx.notify();
            }),
        )
        .on_mouse_up_out(
            MouseButton::Left,
            cx.listener(|app, _, _, cx| {
                app.finish_panel_drag();
                cx.notify();
            }),
        )
        .into_any_element()
}

/// Command palette: one list of registry commands with their live shortcuts.
fn command_palette(
    app: &mut DesktopApp,
    window: &mut Window,
    theme: &Theme,
    cx: &mut Context<DesktopApp>,
) -> AnyElement {
    let input = app.text_field("palette-query", "Search commands", false, cx);
    if app.focus_palette {
        app.focus_input("palette-query", window, cx);
        app.focus_palette = false;
    }
    let entries = app.palette_entries();
    let selected_index = app
        .palette_selected
        .min(entries.len().saturating_sub(1));
    let window_start = if selected_index >= 10 {
        selected_index - 9
    } else {
        0
    };
    let mut list = div().flex().flex_col().w_full();
    if entries.is_empty() {
        list = list.child(
            div()
                .px(theme.sp(10.))
                .py(theme.sp(8.))
                .text_size(theme.fs(10.5))
                .text_color(theme.muted)
                .child("No matching commands"),
        );
    }
    for (offset, entry) in entries.iter().skip(window_start).take(10).enumerate() {
        let index = window_start + offset;
        let selected = index == selected_index;
        let (label, hint) = match entry {
            PaletteEntry::Shell(command) => {
                let chord = app
                    .ui_preferences
                    .keybindings
                    .chord_for(*command)
                    .unwrap_or_default();
                let hint = if chord.is_empty() || chord.eq_ignore_ascii_case("none") {
                    command.group().to_owned()
                } else {
                    format!("{} · {}", command.group(), chord)
                };
                (command.label().to_owned(), hint)
            }
            PaletteEntry::Host(command) => (
                format!("/{}", command.name),
                command.description.clone(),
            ),
            PaletteEntry::File(path) => (
                crate::session_list::truncate(
                    path.rsplit('/').next().unwrap_or(path),
                    48,
                )
                .to_owned(),
                format!("File · {}", crate::session_list::truncate(path, 40)),
            ),
        };
        let entry = entry.clone();
        list = list.child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .gap(theme.sp(8.))
                .w_full()
                .px(theme.sp(10.))
                .py(theme.sp(4.))
                .rounded(theme.corner())
                .id(gpui::SharedString::from(format!("palette-{index}")))
                .cursor_pointer()
                .when(selected, |element| element.bg(theme.selection))
                .hover(|style| style.bg(theme.surface_raised))
                .child(
                    div()
                        .text_size(theme.fs(11.))
                        .text_color(theme.text)
                        .child(label),
                )
                .child(
                    div()
                        .text_size(theme.fs(9.5))
                        .text_color(theme.muted)
                        .child(hint),
                )
                .on_click(cx.listener(move |app, _, _, cx| {
                    app.run_palette_entry(entry.clone());
                    cx.notify();
                })),
        );
    }
    if app.host_commands_loading {
        list = list.child(
            div()
                .px(theme.sp(10.))
                .py(theme.sp(4.))
                .text_size(theme.fs(9.5))
                .text_color(theme.muted)
                .child("Loading Host commands…"),
        );
    }
    if let Some(error) = app.host_commands_error.clone() {
        list = list.child(
            div()
                .px(theme.sp(10.))
                .py(theme.sp(4.))
                .text_size(theme.fs(9.5))
                .text_color(theme.error)
                .child(error),
        );
    }

    div()
        .absolute()
        .inset_0()
        .child(
            div()
                .absolute()
                .inset_0()
                .id("palette-backdrop")
                .on_click(cx.listener(|app, _, _, cx| {
                    app.close_palette();
                    cx.notify();
                })),
        )
        .child(
            div()
                .absolute()
                .top_0()
                .left_0()
                .right_0()
                .flex()
                .flex_col()
                .items_center()
                .pt(theme.sp(72.))
                .child(
                    div()
                        .key_context("CommandPalette")
                        .flex()
                        .flex_col()
                        .gap(theme.sp(6.))
                        .w(theme.sp(520.))
                        .max_h(theme.sp(380.))
                        .overflow_hidden()
                        .p(theme.sp(8.))
                        .rounded(theme.corner())
                        .border_1()
                        .border_color(theme.border)
                        .bg(theme.surface_raised)
                        .shadow_lg()
                        .child(
                            div()
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
                            div()
                                .id("palette-list")
                                .flex()
                                .flex_col()
                                .id("utility-scroll").overflow_y_scroll()
                                .child(list),
                        ),
                ),
        )
        .into_any_element()
}

/// About dialog: client version plus the state the shell actually knows.
fn about_dialog(
    app: &mut DesktopApp,
    theme: &Theme,
    cx: &mut Context<DesktopApp>,
) -> AnyElement {
    let version = env!("CARGO_PKG_VERSION");
    let host = app
        .host_url
        .clone()
        .unwrap_or_else(|| "not connected".to_owned());
    dialog_shell(theme, "About DSH", |body| {
        body.child(
            div()
                .flex()
                .flex_col()
                .gap(theme.sp(4.))
                .child(
                    div()
                        .text_size(theme.fs(11.5))
                        .text_color(theme.text)
                        .child(format!("Native desktop client {version}")),
                )
                .child(
                    div()
                        .text_size(theme.fs(10.5))
                        .text_color(theme.muted)
                        .child("Rust + gpui shell over the Desktop Host API."),
                )
                .child(
                    div()
                        .text_size(theme.fs(10.5))
                        .text_color(theme.muted)
                        .child(format!("Host: {host}")),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .justify_end()
                        .child(
                            widgets::Button::new(theme, "Close")
                                .build()
                                .id("about-close")
                                .cursor_pointer()
                                .on_click(cx.listener(|app, _, _, cx| {
                                    app.show_about = false;
                                    cx.notify();
                                })),
                        ),
                ),
        )
    })
}

/// Bottom row of the Explorer column: create a Session or switch to another one.
fn explorer_footer(
    app: &mut DesktopApp,
    theme: &Theme,
    cx: &mut Context<DesktopApp>,
) -> AnyElement {
    let connected = app.transport.is_some();
    let menu_open = app.show_session_menu;
    let title = app
        .selected_session
        .as_ref()
        .map(|_| {
            if app.session_title.is_empty() {
                "Session".to_owned()
            } else {
                app.session_title.clone()
            }
        })
        .unwrap_or_else(|| "No session".to_owned());

    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(theme.sp(4.))
        .h(theme.sp(28.))
        .px(theme.sp(8.))
        .border_t_1()
        .border_color(theme.border)
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .size(theme.sp(18.))
                .rounded(theme.corner())
                .cursor_pointer()
                .hover(|style| style.bg(theme.surface_raised))
                .id("explorer-new-session")
                .text_color(if connected { theme.accent } else { theme.muted })
                .child("＋")
                .on_click(cx.listener(|app, _, _, cx| {
                    if app.transport.is_some() && !app.sessions_busy() {
                        app.run_command(CommandId::NewSession);
                    }
                    cx.notify();
                })),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .flex_1()
                .min_w(px(0.))
                .gap(theme.sp(4.))
                .px(theme.sp(6.))
                .h(theme.control_height())
                .rounded(theme.corner())
                .cursor_pointer()
                .hover(|style| style.bg(theme.surface_raised))
                .id("explorer-session-menu")
                .on_click(cx.listener(move |app, _, _, cx| {
                    app.show_session_menu = !menu_open;
                    app.top_menu_open = false;
                    cx.notify();
                }))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .truncate()
                        .text_size(theme.fs(10.5))
                        .text_color(theme.text)
                        .child(title),
                )
                .child(
                    div()
                        .text_size(theme.fs(9.))
                        .text_color(theme.muted)
                        .child("▾"),
                ),
        )
        .into_any_element()
}

/// Floating Session switcher, anchored under the top bar.
fn session_popover(
    app: &mut DesktopApp,
    theme: &Theme,
    cx: &mut Context<DesktopApp>,
) -> AnyElement {
    let panel = sessions::panel(app, theme, cx);
    div()
        .absolute()
        .inset_0()
        .child(
            div()
                .absolute()
                .inset_0()
                .id("session-menu-backdrop")
                .on_click(cx.listener(|app, _, _, cx| {
                    app.show_session_menu = false;
                    cx.notify();
                })),
        )
        .child(
            div()
                .absolute()
                .top(theme.sp(layout::TOP_BAR_HEIGHT + 2.))
                .left(theme.sp(8.))
                .flex()
                .flex_col()
                .w(theme.sp(app.ui_preferences.workspace_prefs(app.selected_workspace.as_deref()).session_panel_width.unwrap_or(app.ui_preferences.session_panel_width)))
                .h(theme.sp(520.))
                .overflow_hidden()
                .rounded(theme.corner())
                .border_1()
                .border_color(theme.border)
                .bg(theme.surface)
                .shadow_lg()
                .child(panel),
        )
        .into_any_element()
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

fn clear_goal_dialog(theme: &Theme, cx: &mut Context<DesktopApp>) -> AnyElement {
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
pub(super) fn dialog_shell(
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
