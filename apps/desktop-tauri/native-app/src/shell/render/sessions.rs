//! Sessions column: inventory, search, and Session actions.

use gpui::{div, prelude::*, StatefulInteractiveElement, px, AnyElement, Context};

use crate::layout;
use crate::session_list;
use crate::shell::app::DesktopApp;
use crate::shell::theme::Theme;
use super::widgets;

/// Draws the Session sidebar.
pub(crate) fn panel(
    app: &mut DesktopApp,
    theme: &Theme,
    cx: &mut Context<DesktopApp>,
) -> AnyElement {
    let connected = app.transport.is_some();
    let busy = app.sessions_busy();
    let status = app.status.clone();
    let sessions_status = app.sessions_status.clone();
    let sessions_error = app.sessions_error.clone();
    let connected_for_retry = connected;

    let header = div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .px(theme.sp(8.))
        .pt(theme.sp(12.))
        .pb(theme.sp(8.))
        .child(
            div()
                .text_size(theme.fs(15.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child("Sessions"),
        )
        .child(
            widgets::Button::new(theme, "↻")
                .enabled(connected && !busy)
                .build()
                .id("sessions-refresh")
                .on_click(cx.listener(|app, _, _, cx| {
                    if app.transport.is_some() && !app.sessions_busy() {
                        app.reload_sessions();
                    }
                    cx.notify();
                })),
        );

    let search = app.text_field("session-search", "Search sessions", false, cx);
    let search_row = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(theme.sp(4.))
        .px(theme.sp(8.))
        .child(
            div()
                .flex_1()
                .min_w(px(60.))
                .h(theme.control_height())
                .px(theme.sp(6.))
                .rounded(theme.corner())
                .border_1()
                .border_color(theme.border)
                .bg(theme.code_background)
                .overflow_hidden()
                .child(crate::shell::text_input::text_input_element(
                    &search,
                    theme,
                    theme.control_height() - px(2.),
                )),
        )
        .child(
            widgets::Button::new(theme, "↵")
                .enabled(connected && !busy)
                .build()
                .id("sessions-search")
                .on_click(cx.listener(|app, _, _, cx| {
                    if app.transport.is_some() && !app.sessions_busy() {
                        app.search_sessions();
                    }
                    cx.notify();
                })),
        );

    let new_session = div()
        .flex()
        .items_center()
        .justify_center()
        .w_full()
        .h(theme.sp(36.))
        .rounded(theme.corner())
        .border_1()
        .border_color(if connected { theme.accent } else { theme.border })
        .bg(theme.code_background)
        .text_color(if connected { theme.accent } else { theme.muted })
        .text_size(theme.fs(12.))
        .id("sessions-new")
        .when(connected && !busy, |element| element.cursor_pointer())
        .when(!connected || busy, |element| element.opacity(0.55))
        .child("＋ New session")
        .on_click(cx.listener(|app, _, _, cx| {
            if app.transport.is_some() && !app.sessions_busy() {
                app.new_session();
            }
            cx.notify();
        }));

    let selected_session = app.selected_session.clone();
    let title_input = app.text_field("session-title", "New session title", false, cx);
    let actions = div()
        .flex()
        .flex_col()
        .gap(theme.sp(4.))
        .px(theme.sp(8.))
        .when(selected_session.is_none(), |element| element.hidden())
        .child(widgets::section_label(theme, "SESSION ACTIONS"))
        .child(
            div()
                .flex()
                .flex_row()
                .gap(theme.sp(4.))
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
                            &title_input,
                            theme,
                            theme.control_height() - px(2.),
                        )),
                )
                .child(
                    widgets::Button::new(theme, "Rename")
                        .enabled(connected)
                        .build()
                        .id("session-rename")
                        .on_click(cx.listener(|app, _, _, cx| {
                            app.rename_selected_session();
                            cx.notify();
                        })),
                )
                .child(
                    widgets::Button::new(theme, "Fork")
                        .enabled(connected)
                        .build()
                        .id("session-fork")
                        .on_click(cx.listener(|app, _, _, cx| {
                            app.fork_selected_session();
                            cx.notify();
                        })),
                ),
        );

    let mut inventory = div()
        .id("session-inventory")
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.))
        .w_full()
        .gap(px(2.))
        .overflow_y_scroll();

    let now = now_millis();
    if let Some(results) = app.session_search.clone() {
        for item in &results.items {
            let id = item.session_id.clone();
            let selected = selected_session.as_deref() == Some(id.as_str());
            inventory = inventory.child(session_row(
                theme,
                &session_list::truncate(&item.session_id, 34),
                &session_list::truncate(&item.snippet, 42),
                false,
                selected,
                cx,
                move |app| app.choose_session(id.clone()),
            ));
        }
        if results.has_more {
            inventory = inventory.child(
                div()
                    .px(theme.sp(10.))
                    .py(theme.sp(4.))
                    .text_size(theme.fs(10.))
                    .text_color(theme.muted)
                    .child("More matches exist; refine your search."),
            );
        }
        if results.items.is_empty() {
            inventory = inventory.child(empty_note(theme, "No matching Sessions"));
        }
    } else if app.sessions.is_empty() {
        inventory = inventory.child(empty_note(
            theme,
            if busy {
                "Loading sessions…"
            } else if connected {
                "No sessions yet."
            } else {
                "Connect to enable sessions"
            },
        ));
    } else {
        for summary in &app.sessions {
            let id = summary.session_id.clone();
            let title = session_list::session_label(summary);
            let mut meta = session_list::relative_time(summary.updated_at, now);
            if summary.blank {
                meta.push_str(" · blank");
            }
            let selected = selected_session.as_deref() == Some(id.as_str());
            inventory = inventory.child(session_row(
                theme,
                &session_list::truncate(&title, 30),
                &meta,
                summary.running,
                selected,
                cx,
                move |app| app.choose_session(id.clone()),
            ));
        }
    }

    let footer = div()
        .flex()
        .flex_col()
        .gap(px(2.))
        .px(theme.sp(10.))
        .py(theme.sp(6.))
        .border_t_1()
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
                        .font_weight(gpui::FontWeight::BOLD)
                        .text_color(theme.accent)
                        .child("DH"),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .text_size(theme.fs(11.))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .child("DeepSeek Harness"),
                        )
                        .child(
                            div()
                                .text_size(theme.fs(9.))
                                .text_color(theme.muted)
                                .child("Native desktop client"),
                        ),
                ),
        )
        .child(
            div()
                .text_size(theme.fs(9.5))
                .text_color(theme.muted)
                .truncate()
                .child(status),
        )
        .child(
            widgets::Button::new(theme, "Open file…")
                .build()
                .id("session-open-file")
                .on_click(cx.listener(|app, _, _, cx| {
                    app.open_file_dialog();
                    cx.notify();
                })),
        );

    let mut column = div()
        .flex()
        .flex_col()
        .w(theme.sp(app.ui_preferences.session_panel_width))
        .min_w(theme.sp(layout::SIDEBAR_MIN))
        .max_w(theme.sp(layout::SIDEBAR_MAX))
        .min_h(px(0.))
        .border_r_1()
        .border_color(theme.border)
        .bg(theme.surface)
        .child(header)
        .child(
            div()
                .flex()
                .flex_col()
                .gap(theme.sp(6.))
                .px(theme.sp(8.))
                .pb(theme.sp(6.))
                .child(new_session)
                .child(search_row),
        )
        .child(actions);
    if !sessions_status.is_empty() {
        column = column.child(
            div()
                .px(theme.sp(10.))
                .pb(theme.sp(4.))
                .text_size(theme.fs(9.5))
                .text_color(theme.muted)
                .child(sessions_status.clone()),
        );
    }
    if let Some(error) = sessions_error {
        column = column.child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(theme.sp(6.))
                .px(theme.sp(10.))
                .child(
                    div()
                        .flex_1()
                        .text_size(theme.fs(10.))
                        .text_color(theme.error)
                        .child(error),
                )
                .child(
                    widgets::Button::new(theme, "Retry")
                        .enabled(connected_for_retry)
                        .build()
                        .id("sessions-retry")
                        .on_click(cx.listener(|app, _, _, cx| {
                            app.reload_sessions();
                            cx.notify();
                        })),
                ),
        );
    }
    column
        .child(div().flex_1().min_h(px(0.)).px(theme.sp(6.)).child(inventory))
        .child(footer)
        .into_any_element()
}

fn empty_note(theme: &Theme, text: &str) -> AnyElement {
    div()
        .px(theme.sp(10.))
        .py(theme.sp(8.))
        .text_size(theme.fs(11.))
        .text_color(theme.muted)
        .child(text.to_owned())
        .into_any_element()
}

fn session_row(
    theme: &Theme,
    title: &str,
    meta: &str,
    running: bool,
    selected: bool,
    cx: &mut Context<DesktopApp>,
    action: impl Fn(&mut DesktopApp) + 'static,
) -> AnyElement {
    let title = title.to_owned();
    let meta = meta.to_owned();
    let primary = if selected {
        theme.accent
    } else {
        theme.text
    };
    let row = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(theme.sp(6.))
        .w_full()
        .min_h(theme.sp(40.))
        .px(theme.sp(8.))
        .py(theme.sp(4.))
        .rounded(theme.corner())
        .id(gpui::SharedString::from(format!("session-{title}-{meta}")))
        .cursor_pointer()
        .when(selected, |element| element.bg(theme.selection))
        .when(!selected, |element| {
            element.hover(|style| style.bg(theme.surface_raised))
        })
        .on_click(cx.listener(move |app, _, _, cx| {
            action(app);
            cx.notify();
        }))
        .when(running, |element| {
            element.child(
                div()
                    .text_size(theme.fs(9.))
                    .text_color(theme.accent)
                    .child("●"),
            )
        })
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w(px(0.))
                .child(
                    div()
                        .text_size(theme.fs(12.))
                        .text_color(primary)
                        .truncate()
                        .child(title),
                )
                .child(
                    div()
                        .text_size(theme.fs(9.5))
                        .text_color(theme.muted)
                        .truncate()
                        .child(meta),
                ),
        );
    row.into_any_element()
}

/// Unix time in milliseconds, tolerant of clock errors.
pub(crate) fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| {
            i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
        })
}
