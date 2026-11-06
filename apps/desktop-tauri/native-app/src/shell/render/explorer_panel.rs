//! Workspace Explorer column: repository tree and read-only source preview.

use gpui::{div, prelude::*, px, AnyElement, Context, StatefulInteractiveElement};

use super::widgets;
use crate::explorer::{ExplorerRow, ExplorerRowKind};
use crate::layout;
use crate::shell::app::{ActiveView, DesktopApp};
use crate::shell::theme::Theme;

/// Draws the docked Explorer column.
pub(crate) fn tree_and_preview(
    app: &mut DesktopApp,
    theme: &Theme,
    single_pane: bool,
    cx: &mut Context<DesktopApp>,
) -> AnyElement {
    let transport = app.transport.clone();
    let streams = app.streams.clone();
    app.explorer.prepare(
        transport.as_ref(),
        streams.as_ref(),
        app.selected_session.as_deref(),
        single_pane,
    );
    let mut column = div()
        .flex()
        .flex_col()
        .size_full()
        .min_h(px(0.))
        .child(header(theme, transport.is_some(), cx))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(theme.sp(4.))
                .px(theme.sp(10.))
                .pb(theme.sp(6.))
                .child(
                    div()
                        .text_size(theme.fs(10.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child("Read only"),
                )
                .child(
                    div()
                        .text_size(theme.fs(10.))
                        .text_color(theme.muted)
                        .child("· scoped to this Session’s workspace"),
                ),
        )
        .child(widgets::separator(theme));

    if app.explorer.session().is_none() {
        return column
            .child(padded_note(
                theme,
                "Select a Session to browse its workspace.",
            ))
            .into_any_element();
    }

    let watch_error = app.explorer.watch_error().map(str::to_owned);
    if let Some(error) = watch_error {
        column = column.child(
            div()
                .px(theme.sp(10.))
                .pb(theme.sp(4.))
                .text_size(theme.fs(10.))
                .text_color(theme.muted)
                .child(format!("⚠ {error}")),
        );
    }

    if single_pane && app.explorer.preview_pane_visible() {
        return column
            .child(preview(app, theme, transport.is_some(), cx))
            .into_any_element();
    }

    let filter = app.text_field("explorer-query", "Filter files", false, cx);
    column = column.child(
        div()
            .px(theme.sp(10.))
            .pb(theme.sp(4.))
            .child(crate::shell::text_input::text_input_element(
                &filter,
                theme,
                theme.sp(160.),
            )),
    );
    let mut tree = div()
        .flex()
        .flex_col()
        .gap(px(1.))
        .w_full()
        .child(widgets::section_label(theme, "REPOSITORY TREE"));

    let query = app.input_text("explorer-query", cx).to_ascii_lowercase();
    let rows: Vec<_> = app
        .explorer
        .visible_rows()
        .into_iter()
        .filter(|row| {
            query.is_empty()
                || row.label.to_ascii_lowercase().contains(&query)
                || row.path.to_ascii_lowercase().contains(&query)
        })
        .collect();
    for row in rows {
        tree = tree.child(tree_row(theme, &row, transport.is_some(), cx));
    }
    column = column.child(
        div()
            .id("explorer-tree")
            .flex()
            .flex_col()
            .max_h(theme.sp(if single_pane { 10_000. } else { 220. }))
            .min_h(px(0.))
            .px(theme.sp(6.))
            .py(theme.sp(4.))
            .overflow_y_scroll()
            .child(tree),
    );
    if transport.is_none() {
        column = column.child(padded_note(
            theme,
            "Host disconnected · loaded files are retained",
        ));
    }
    if !single_pane {
        column = column.child(widgets::separator(theme)).child(preview(
            app,
            theme,
            transport.is_some(),
            cx,
        ));
    }
    column.into_any_element()
}

/// Draws the Explorer as the whole central surface.
pub(crate) fn fullscreen(
    app: &mut DesktopApp,
    theme: &Theme,
    viewport_width: f32,
    cx: &mut Context<DesktopApp>,
) -> AnyElement {
    let single_pane = viewport_width <= layout::EXPLORER_SINGLE_PANE_MAX_WIDTH;
    div()
        .flex()
        .flex_col()
        .size_full()
        .min_h(px(0.))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(theme.sp(8.))
                .px(theme.sp(10.))
                .py(theme.sp(6.))
                .border_b_1()
                .border_color(theme.border)
                .bg(theme.surface)
                .child(
                    widgets::Button::new(theme, "Back to Session")
                        .build()
                        .id("explorer-back")
                        .on_click(cx.listener(|app, _, _, cx| {
                            app.explorer_fullscreen = false;
                            app.switch_view(ActiveView::Sessions);
                            cx.notify();
                        })),
                )
                .child(
                    div()
                        .text_size(theme.fs(11.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child("Workspace Explorer"),
                ),
        )
        .child(
            div()
                .flex()
                .flex_1()
                .min_h(px(0.))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .when(single_pane, |element| element.w_full())
                        .when(!single_pane, |element| element.w(theme.sp(380.)))
                        .min_h(px(0.))
                        .border_r_1()
                        .border_color(theme.border)
                        .child(tree_and_preview(app, theme, single_pane, cx)),
                )
                .children((!single_pane).then(|| {
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w(px(0.))
                        .min_h(px(0.))
                        .child(preview(app, theme, app.transport.is_some(), cx))
                })),
        )
        .into_any_element()
}

fn header(theme: &Theme, connected: bool, cx: &mut Context<DesktopApp>) -> AnyElement {
    div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .px(theme.sp(10.))
        .pt(theme.sp(10.))
        .pb(theme.sp(4.))
        .child(
            div()
                .text_size(theme.fs(13.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child("Workspace Explorer"),
        )
        .child(
            widgets::Button::new(theme, "↻")
                .enabled(connected)
                .build()
                .id("explorer-refresh")
                .on_click(cx.listener(|app, _, _, cx| {
                    if let Some(transport) = app.transport.clone() {
                        app.explorer.refresh_root(&transport);
                    }
                    cx.notify();
                })),
        )
        .into_any_element()
}

fn tree_row(
    theme: &Theme,
    row: &ExplorerRow,
    connected: bool,
    cx: &mut Context<DesktopApp>,
) -> AnyElement {
    let indent = theme.sp(4.0 + row.depth as f32 * 12.0);
    let path = row.path.clone();
    match &row.kind {
        ExplorerRowKind::Directory { expanded } => {
            let expanded = *expanded;
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(theme.sp(4.))
                .w_full()
                .min_h(theme.sp(22.))
                .pl(indent)
                .pr(theme.sp(6.))
                .rounded(theme.corner())
                .cursor_pointer()
                .hover(|style| style.bg(theme.surface_raised))
                .id(gpui::SharedString::from(format!("dir-{path}")))
                .on_click(cx.listener(move |app, _, _, cx| {
                    if let Some(session_id) = app.selected_session.clone() {
                        let transport = app.transport.clone();
                        app.explorer
                            .toggle_directory(transport.as_ref(), &session_id, &path);
                    }
                    cx.notify();
                }))
                .child(
                    div()
                        .text_size(theme.fs(9.))
                        .text_color(theme.muted)
                        .child(if expanded { "▾" } else { "▸" }),
                )
                .child(
                    div()
                        .text_size(theme.fs(11.5))
                        .text_color(theme.text)
                        .truncate()
                        .child(format!("▰ {}", row.label)),
                )
                .into_any_element()
        }
        ExplorerRowKind::File { selected } => {
            let selected = *selected;
            div()
                .flex()
                .flex_row()
                .items_center()
                .w_full()
                .min_h(theme.sp(22.))
                .pl(indent + theme.sp(12.))
                .pr(theme.sp(6.))
                .rounded(theme.corner())
                .cursor_pointer()
                .when(selected, |element| element.bg(theme.selection))
                .when(!selected, |element| {
                    element.hover(|style| style.bg(theme.surface_raised))
                })
                .id(gpui::SharedString::from(format!("file-{path}")))
                .on_click(cx.listener(move |app, _, _, cx| {
                    if let Some(session_id) = app.selected_session.clone() {
                        let transport = app.transport.clone();
                        app.explorer
                            .open_file(transport.as_ref(), &session_id, &path);
                    }
                    cx.notify();
                }))
                .child(
                    div()
                        .text_size(theme.fs(11.5))
                        .text_color(if selected { theme.accent } else { theme.text })
                        .truncate()
                        .child(format!("▤ {}", row.label)),
                )
                .into_any_element()
        }
        ExplorerRowKind::Error => div()
            .flex()
            .flex_row()
            .items_center()
            .gap(theme.sp(6.))
            .w_full()
            .pl(indent)
            .child(
                div()
                    .flex_1()
                    .text_size(theme.fs(10.5))
                    .text_color(theme.error)
                    .child(row.label.clone()),
            )
            .when(connected, |element| {
                element.child(
                    widgets::Button::new(theme, "Retry")
                        .build()
                        .id(gpui::SharedString::from(format!("retry-{path}")))
                        .on_click(cx.listener(move |app, _, _, cx| {
                            if let Some(session_id) = app.selected_session.clone() {
                                let transport = app.transport.clone();
                                app.explorer
                                    .retry_listing(transport.as_ref(), &session_id, &path);
                            }
                            cx.notify();
                        })),
                )
            })
            .into_any_element(),
        ExplorerRowKind::Loading => div()
            .pl(indent + theme.sp(12.))
            .py(theme.sp(2.))
            .text_size(theme.fs(10.))
            .text_color(theme.muted)
            .child(row.label.clone())
            .into_any_element(),
        ExplorerRowKind::Symlink | ExplorerRowKind::Unsupported => div()
            .pl(indent + theme.sp(12.))
            .py(theme.sp(2.))
            .text_size(theme.fs(10.))
            .text_color(theme.muted)
            .child(row.label.clone())
            .into_any_element(),
        ExplorerRowKind::Notice => div()
            .pl(indent + theme.sp(12.))
            .py(theme.sp(2.))
            .text_size(theme.fs(10.))
            .text_color(theme.muted)
            .child(row.label.clone())
            .into_any_element(),
    }
}

fn preview(
    app: &mut DesktopApp,
    theme: &Theme,
    connected: bool,
    cx: &mut Context<DesktopApp>,
) -> AnyElement {
    let path = app.explorer.preview_path().map(str::to_owned);
    let pending = app.explorer.preview_pending();
    let error = app.explorer.preview_error().map(str::to_owned);
    let stale = app.explorer.preview_stale();
    let absent = app.explorer.preview_absent();
    let text = app.explorer.preview().cloned();

    let mut pane = div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.))
        .px(theme.sp(10.))
        .py(theme.sp(6.))
        .gap(theme.sp(4.));

    let mut header = div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .child(
            div()
                .text_size(theme.fs(9.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(theme.muted)
                .child("SOURCE PREVIEW"),
        );
    if let (Some(path), true) = (path.clone(), connected) {
        header = header.child(
            widgets::Button::new(theme, "↻")
                .build()
                .id("explorer-reload-preview")
                .on_click(cx.listener(move |app, _, _, cx| {
                    if let Some(session_id) = app.selected_session.clone() {
                        let transport = app.transport.clone();
                        app.explorer
                            .reload_preview(transport.as_ref(), &session_id, &path);
                    }
                    cx.notify();
                })),
        );
    }
    pane = pane.child(header);

    let Some(path) = path else {
        return pane
            .child(padded_note(theme, "Select a file to preview its contents."))
            .into_any_element();
    };

    let mut crumbs = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(theme.sp(3.))
        .flex_wrap()
        .child(
            div()
                .text_size(theme.fs(10.))
                .text_color(theme.muted)
                .font(theme.mono_font())
                .child(path.clone()),
        )
        .child(div().flex_1())
        .child(
            div()
                .text_size(theme.fs(9.))
                .text_color(theme.muted)
                .child("Read only"),
        );
    if app.explorer.preview_pane_visible() {
        crumbs = crumbs.child(
            widgets::Button::new(theme, "‹ Files")
                .build()
                .id("explorer-close-preview")
                .on_click(cx.listener(|app, _, _, cx| {
                    app.explorer.close_preview();
                    cx.notify();
                })),
        );
    }
    pane = pane.child(crumbs).child(widgets::separator(theme));

    if absent {
        pane = pane.child(
            div()
                .px(theme.sp(8.))
                .py(theme.sp(4.))
                .rounded(theme.corner())
                .bg(theme.surface_raised)
                .text_size(theme.fs(10.))
                .text_color(theme.warn)
                .child("This file no longer exists on disk. The last read is shown below."),
        );
    } else if stale {
        pane = pane.child(
            div()
                .px(theme.sp(8.))
                .py(theme.sp(4.))
                .rounded(theme.corner())
                .bg(theme.surface_raised)
                .text_size(theme.fs(10.))
                .text_color(theme.warn)
                .child("This file changed on disk. Reload to see the new content."),
        );
    }

    if pending {
        pane = pane.child(
            div()
                .text_size(theme.fs(10.5))
                .text_color(theme.muted)
                .child("Reading text file…"),
        );
    }
    if let Some(error) = error {
        pane = pane.child(
            div()
                .text_size(theme.fs(10.5))
                .text_color(theme.error)
                .child(error),
        );
    }

    let Some(text) = text else {
        if !pending {
            return pane
                .child(padded_note(
                    theme,
                    "Choose a UTF-8 text file. Images and Office documents use Document preview.",
                ))
                .into_any_element();
        }
        return pane.into_any_element();
    };

    if text.text.is_empty() {
        pane = pane.child(padded_note(theme, "This text file is empty."));
    } else {
        let mut lines = div()
            .flex()
            .flex_col()
            .w_full()
            .id("explorer-source")
            .flex_1()
            .min_h(px(0.))
            .overflow_scroll();
        for (index, line) in text.text.split('\n').enumerate() {
            let number = text.offset + index as u64;
            lines = lines.child(
                div()
                    .flex()
                    .flex_row()
                    .w_full()
                    .gap(theme.sp(6.))
                    .child(
                        div()
                            .w(theme.sp(38.))
                            .text_size(theme.fs(10.))
                            .text_color(theme.muted)
                            .font(theme.mono_font())
                            .child(number.to_string()),
                    )
                    .child(
                        div()
                            .text_size(theme.fs(10.5))
                            .font(theme.mono_font())
                            .whitespace_nowrap()
                            .child(line.to_owned()),
                    ),
            );
        }
        pane = pane.child(lines);
    }

    if let Some(bytes) = text.stat.bytes {
        pane = pane.child(
            div()
                .text_size(theme.fs(9.5))
                .text_color(theme.muted)
                .child(format!("{bytes} bytes · read only")),
        );
    }
    if !text.eof {
        let reload_path = path.clone();
        pane = pane.child(
            div()
                .flex()
                .flex_col()
                .gap(theme.sp(4.))
                .child(
                    div()
                        .text_size(theme.fs(9.5))
                        .text_color(theme.muted)
                        .child("Preview limited to a page of lines."),
                )
                .child(
                    widgets::Button::new(theme, "Load next lines")
                        .enabled(connected)
                        .build()
                        .id("explorer-next-page")
                        .on_click(cx.listener(move |app, _, _, cx| {
                            if let Some(session_id) = app.selected_session.clone() {
                                let transport = app.transport.clone();
                                app.explorer.load_next_preview_page(
                                    transport.as_ref(),
                                    &session_id,
                                    &reload_path,
                                );
                            }
                            cx.notify();
                        })),
                ),
        );
    }
    pane.into_any_element()
}

fn padded_note(theme: &Theme, text: &str) -> AnyElement {
    div()
        .px(theme.sp(10.))
        .py(theme.sp(8.))
        .text_size(theme.fs(10.5))
        .text_color(theme.muted)
        .child(text.to_owned())
        .into_any_element()
}
