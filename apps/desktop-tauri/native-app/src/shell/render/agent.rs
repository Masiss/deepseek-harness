//! Agent column: Session header, transcript, approvals, questions, and composer.

use gpui::{div, prelude::*, px, AnyElement, Context, Div, Window};

use crate::assets::Icon;
use crate::explorer::{ExplorerRow, ExplorerRowKind};
use crate::layout;
use crate::session_list::relative_time;
use crate::transcript::{RowKind, TranscriptRow};
use dsh_client_core::chat::PromptMode;
use crate::shell::app::DesktopApp;
use crate::shell::theme::Theme;
use super::sessions::now_millis;
use super::widgets;

/// V2 conversation reading column, centered inside the middle pane.
const READING_COLUMN_WIDTH: f32 = 790.0;
/// V2 composer shell width, pinned above the status bar.
const COMPOSER_WIDTH: f32 = 826.0;
/// Avatar diameter from the approved reference.
const AVATAR_SIZE: f32 = 29.0;
/// Transcript body size from the approved reference.
const BODY_TEXT_SIZE: f32 = 12.6;
/// Gap between the avatar column and the message body.
const AVATAR_GAP: f32 = 13.0;

/// Draws the conversation surface for the selected Session.
pub(crate) fn conversation(
    app: &mut DesktopApp,
    window: &mut Window,
    theme: &Theme,
    viewport_width: f32,
    cx: &mut Context<DesktopApp>,
) -> AnyElement {
    let mut column = div()
        .flex()
        .flex_col()
        .size_full()
        .min_h(px(0.))
        .bg(theme.background);
    if app.focus_composer {
        app.focus_input("composer", window, cx);
        app.focus_composer = false;
    }

    if app.transport.is_none() {
        column = column.child(disconnected_banner(app, theme, cx));
    }

    if app.selected_session.is_none() {
        return column.child(empty_state(app, theme, cx)).into_any_element();
    }

    column = column.child(session_tab(app, theme, viewport_width, cx));
    column = column.child(transcript_pane(app, theme, cx));
    column = column.child(attention_pane(app, theme, cx));
    column.child(composer(app, theme, cx)).into_any_element()
}

fn disconnected_banner(
    app: &mut DesktopApp,
    theme: &Theme,
    cx: &mut Context<DesktopApp>,
) -> AnyElement {
    let reconnecting = app.host_receiver.is_some() || app.connect_receiver.is_some();
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
            div()
                .text_size(theme.fs(11.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(theme.warn)
                .child("Host disconnected"),
        )
        .child(
            div()
                .flex_1()
                .text_size(theme.fs(10.5))
                .text_color(theme.muted)
                .child("Loaded Session content is retained. Reconnect to continue working."),
        )
        .when(reconnecting, |element| {
            element.child(
                div()
                    .text_size(theme.fs(10.))
                    .text_color(theme.muted)
                    .child("Reconnecting…"),
            )
        })
        .when(!reconnecting, |element| {
            element.child(
                widgets::Button::new(theme, "Retry connection")
                    .build()
                    .id("agent-retry-connection")
                    .on_click(cx.listener(|app, _, _, cx| {
                        app.connect();
                        cx.notify();
                    })),
            )
        })
        .into_any_element()
}

fn session_tab(
    app: &mut DesktopApp,
    theme: &Theme,
    viewport_width: f32,
    cx: &mut Context<DesktopApp>,
) -> AnyElement {
    let title = if app.session_title.is_empty() {
        app.selected_session
            .clone()
            .unwrap_or_else(|| "Session".to_owned())
    } else {
        app.session_title.clone()
    };
    let wide = viewport_width >= layout::EXPLORER_DOCK_MIN_WIDTH;
    let state = if app.transport.is_none() {
        "Session · Host disconnected".to_owned()
    } else if app.chat_loading || app.assistant_streaming {
        "Streaming · Host connected".to_owned()
    } else {
        format!("{} · Host connected", app.transcript.run_state(false)).to_owned()
    };
    let explorer_shown = app.show_explorer;
    div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .h(theme.sp(layout::AGENT_TAB_HEIGHT))
        .px(theme.sp(10.))
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
                        .text_size(theme.fs(12.5))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .truncate()
                        .child(title),
                )
                .child(
                    div()
                        .text_size(theme.fs(9.))
                        .text_color(theme.muted)
                        .child(state),
                ),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(theme.sp(4.))
                .child(
                    widgets::Button::new(theme, if explorer_shown { "Hide Explorer" } else { "Show Explorer" })
                        .build()
                        .id("agent-toggle-explorer")
                        .on_click(cx.listener(move |app, _, _, cx| {
                            if wide {
                                app.show_explorer = !explorer_shown;
                            } else {
                                app.switch_view(crate::shell::app::ActiveView::Explorer);
                            }
                            cx.notify();
                        })),
                ),
        )
        .into_any_element()
}

fn transcript_pane(
    app: &mut DesktopApp,
    theme: &Theme,
    cx: &mut Context<DesktopApp>,
) -> AnyElement {
    let mut content = div()
        .flex()
        .flex_col()
        .gap(theme.sp(14.))
        .w_full()
        .max_w(theme.sp(READING_COLUMN_WIDTH))
        .mx_auto()
        .debug_selector(|| "transcript-reading-column".to_owned());

    if app.page_loading && app.transcript.is_empty() {
        content = content.child(note(theme, "Loading session…"));
    } else if let Some(error) = app.page_error.clone() {
        content = content.child(
            div()
                .flex()
                .flex_col()
                .gap(theme.sp(4.))
                .child(
                    div()
                        .text_size(theme.fs(11.))
                        .text_color(theme.error)
                        .child(error),
                )
                .child(
                    widgets::Button::new(theme, "Retry session load")
                        .build()
                        .id("agent-retry-session")
                        .on_click(cx.listener(|app, _, _, cx| {
                            if let Some(session_id) = app.selected_session.clone() {
                                app.load_session(session_id);
                            }
                            cx.notify();
                        })),
                ),
        );
    } else if app.transcript.is_empty() {
        content = content.child(note(theme, "This session has no events"));
    } else if app.page_has_more {
        let loading = app.page_loading;
        content = content.child(if loading {
            note(theme, "Loading older events…").into_any_element()
        } else {
            widgets::Button::new(theme, "Load older events")
                .build()
                .id("agent-load-older")
                .on_click(cx.listener(|app, _, _, cx| {
                    app.load_older_session_page();
                    cx.notify();
                }))
                .into_any_element()
        });
    }

    let feedback = app
        .feedback_items
        .iter()
        .map(|item| (item.message_id.clone(), item.rating.clone()))
        .collect::<std::collections::BTreeMap<_, _>>();
    let now = now_millis();
    let mut index = 0;
    while index < app.transcript.rows().len() {
        let kind = app.transcript.rows()[index].kind;
        if is_tool_row(kind) {
            let mut end = index + 1;
            while end < app.transcript.rows().len()
                && is_tool_row(app.transcript.rows()[end].kind)
            {
                end += 1;
            }
            content = content.child(tool_activity_group(app, theme, index, end, cx));
            index = end;
            continue;
        }
        let row = &app.transcript.rows()[index];
        let time = app
            .transcript
            .time_of(row.seq)
            .filter(|time| *time > 0)
            .map(|time| relative_time(time, now));
        content = content.child(transcript_row(
            theme,
            row,
            feedback.get(row.message_id.as_deref().unwrap_or_default()),
            time.as_deref(),
        ));
        index += 1;
    }

    if app.assistant_streaming {
        content = content.child(streaming_draft(theme, &app.assistant_draft));
    }

    if app.scroll_transcript_to_bottom {
        app.transcript_scroll.scroll_to_bottom();
        app.scroll_transcript_to_bottom = false;
    }
    if app.assistant_streaming {
        app.transcript_scroll.scroll_to_bottom();
    }

    div()
        .id("transcript-scroll")
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.))
        .w_full()
        .px(theme.sp(14.))
        .py(theme.sp(14.))
        .overflow_y_scroll()
        .track_scroll(&app.transcript_scroll)
        .child(content)
        .into_any_element()
}

/// Whether a row belongs to the grouped tool-activity section.
fn is_tool_row(kind: RowKind) -> bool {
    matches!(kind, RowKind::ToolResult | RowKind::ToolError)
}

/// Avatar fill, foreground, and glyph for one transcript row.
fn avatar(theme: &Theme, kind: RowKind) -> AnyElement {
    let (background, foreground, glyph) = match kind {
        RowKind::User => (theme.surface_raised, theme.text, "U"),
        RowKind::Assistant => (theme.accent, theme.accent_text, "D"),
        RowKind::Reasoning => (theme.code_background, theme.muted, "R"),
        RowKind::ToolResult => (theme.code_background, theme.muted, "T"),
        RowKind::ToolError => (theme.error, theme.background, "!"),
        RowKind::Approval => (theme.warn, theme.background, "!"),
        RowKind::Job => (theme.code_background, theme.accent, "J"),
        RowKind::Todo => (theme.code_background, theme.accent, "T"),
        RowKind::Deliverable => (theme.code_background, theme.accent, "F"),
        RowKind::Goal => (theme.code_background, theme.warn, "G"),
        RowKind::Notice => (theme.code_background, theme.muted, "·"),
    };
    div()
        .flex()
        .items_center()
        .justify_center()
        .flex_none()
        .w(theme.sp(AVATAR_SIZE))
        .h(theme.sp(AVATAR_SIZE))
        .rounded_full()
        .bg(background)
        .text_color(foreground)
        .text_size(theme.fs(12.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .child(glyph)
        .into_any_element()
}

/// Speaker name for one row kind.
fn speaker_name(kind: RowKind) -> &'static str {
    match kind {
        RowKind::User => "You",
        RowKind::Assistant => "DSH",
        RowKind::Reasoning => "Reasoning",
        RowKind::ToolResult | RowKind::ToolError => "Tool",
        RowKind::Approval => "Approval",
        RowKind::Job => "Job",
        RowKind::Todo => "Tasks",
        RowKind::Deliverable => "Deliverables",
        RowKind::Goal => "Goal",
        RowKind::Notice => "Notice",
    }
}

/// A row title worth showing beside its speaker, when it adds information.
fn row_detail(row: &TranscriptRow) -> Option<&str> {
    match row.kind {
        RowKind::Approval
        | RowKind::Job
        | RowKind::Todo
        | RowKind::Deliverable
        | RowKind::Goal
        | RowKind::Notice => Some(row.title.as_str()).filter(|title| !title.is_empty()),
        RowKind::User => Some(row.title.as_str()).filter(|title| *title != "You"),
        _ => None,
    }
}

fn transcript_row(
    theme: &Theme,
    row: &TranscriptRow,
    rating: Option<&String>,
    time: Option<&str>,
) -> AnyElement {
    let accent = match row.kind {
        RowKind::User => theme.accent,
        RowKind::Assistant => theme.text,
        RowKind::Reasoning | RowKind::Notice | RowKind::ToolResult => theme.muted,
        RowKind::ToolError => theme.error,
        RowKind::Approval | RowKind::Goal => theme.warn,
        RowKind::Job | RowKind::Todo | RowKind::Deliverable => theme.accent,
    };
    let mut header = div()
        .flex()
        .flex_row()
        .items_baseline()
        .gap(theme.sp(7.))
        .w_full()
        .child(
            div()
                .text_size(theme.fs(12.5))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(accent)
                .child(speaker_name(row.kind)),
        )
        .when_some(row_detail(row), |element, detail| {
            element.child(
                div()
                    .min_w(px(0.))
                    .truncate()
                    .text_size(theme.fs(11.))
                    .text_color(theme.muted)
                    .child(detail.to_owned()),
            )
        })
        .child(div().flex_1());
    if let Some(rating) = rating {
        header = header.child(
            div()
                .text_size(theme.fs(10.))
                .text_color(theme.muted)
                .child(format!("Feedback {rating}")),
        );
    }
    if let Some(time) = time {
        header = header.child(
            div()
                .text_size(theme.fs(11.))
                .text_color(theme.muted)
                .child(time.to_owned()),
        );
    }

    let mut body = div()
        .flex()
        .flex_col()
        .gap(theme.sp(8.))
        .w_full()
        .min_w(px(0.))
        .child(header);
    if !row.text.is_empty() {
        let muted_body = matches!(row.kind, RowKind::Reasoning | RowKind::Notice);
        body = body.child(blocks_element(theme, &row.text, muted_body));
    }

    let mut element = div()
        .flex()
        .flex_row()
        .items_start()
        .gap(theme.sp(AVATAR_GAP))
        .w_full()
        .child(avatar(theme, row.kind))
        .child(body);
    match row.kind {
        RowKind::ToolError => {
            element = element
                .border_l_2()
                .border_color(theme.error)
                .pl(theme.sp(8.));
        }
        RowKind::Approval => {
            element = element
                .border_l_2()
                .border_color(theme.warn)
                .pl(theme.sp(8.));
        }
        _ => {}
    }
    element.into_any_element()
}

fn streaming_draft(theme: &Theme, draft: &str) -> AnyElement {
    div()
        .flex()
        .flex_row()
        .items_start()
        .gap(theme.sp(AVATAR_GAP))
        .w_full()
        .child(avatar(theme, RowKind::Assistant))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(theme.sp(8.))
                .min_w(px(0.))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_baseline()
                        .gap(theme.sp(7.))
                        .child(
                            div()
                                .text_size(theme.fs(12.5))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(theme.accent)
                                .child("DSH"),
                        )
                        .child(
                            div()
                                .text_size(theme.fs(11.))
                                .text_color(theme.muted)
                                .child("Streaming"),
                        ),
                )
                .child(blocks_element(
                    theme,
                    if draft.is_empty() { "…" } else { draft },
                    false,
                )),
        )
        .into_any_element()
}

/// One parsed transcript-text block.
enum TextBlock {
    Heading { level: u8, text: String },
    Paragraph(String),
    List { ordered: bool, items: Vec<ListItem> },
    Code { language: String, text: String },
    Rule,
}

/// One list item with its rendered marker.
struct ListItem {
    marker: String,
    text: String,
}

/// Splits transcript text into the simple block forms the runtime emits.
///
/// Handles ATX headings, `-`/`*`/`•` bullets, numbered items, fenced code
/// blocks, horizontal rules, and paragraphs separated by blank lines. Anything
/// else stays literal text.
///
/// @param text - Projected row text.
/// @returns Blocks in source order.
fn text_blocks(text: &str) -> Vec<TextBlock> {
    let mut blocks = Vec::new();
    let mut paragraph: Vec<String> = Vec::new();
    let mut list: Option<(bool, Vec<ListItem>)> = None;
    let mut in_code = false;
    let mut language = String::new();
    let mut code_lines: Vec<String> = Vec::new();

    for line in text.split('\n') {
        let trimmed = line.trim_start();
        if in_code {
            if trimmed.starts_with("```") {
                blocks.push(TextBlock::Code {
                    language: std::mem::take(&mut language),
                    text: code_lines.join("\n"),
                });
                code_lines.clear();
                in_code = false;
            } else {
                code_lines.push(line.to_owned());
            }
            continue;
        }
        if let Some(info) = trimmed.strip_prefix("```") {
            push_paragraph(&mut blocks, &mut paragraph);
            push_list(&mut blocks, &mut list);
            language = info.trim().to_owned();
            in_code = true;
            continue;
        }
        if trimmed.is_empty() {
            push_paragraph(&mut blocks, &mut paragraph);
            push_list(&mut blocks, &mut list);
            continue;
        }
        if trimmed == "---" || trimmed == "***" || trimmed == "___" {
            push_paragraph(&mut blocks, &mut paragraph);
            push_list(&mut blocks, &mut list);
            blocks.push(TextBlock::Rule);
            continue;
        }
        if let Some((level, heading)) = heading_line(trimmed) {
            push_paragraph(&mut blocks, &mut paragraph);
            push_list(&mut blocks, &mut list);
            blocks.push(TextBlock::Heading {
                level,
                text: heading.to_owned(),
            });
            continue;
        }
        if let Some(item) = bullet_line(trimmed) {
            push_paragraph(&mut blocks, &mut paragraph);
            let entry = ListItem {
                marker: "•".to_owned(),
                text: item.to_owned(),
            };
            push_item(&mut blocks, &mut list, false, entry);
            continue;
        }
        if let Some(item) = numbered_line(trimmed) {
            push_paragraph(&mut blocks, &mut paragraph);
            push_item(&mut blocks, &mut list, true, item);
            continue;
        }
        push_list(&mut blocks, &mut list);
        paragraph.push(line.to_owned());
    }
    if in_code {
        blocks.push(TextBlock::Code {
            language,
            text: code_lines.join("\n"),
        });
    }
    push_paragraph(&mut blocks, &mut paragraph);
    push_list(&mut blocks, &mut list);
    blocks
}

/// Flushes accumulated paragraph lines as one block.
fn push_paragraph(blocks: &mut Vec<TextBlock>, paragraph: &mut Vec<String>) {
    if !paragraph.is_empty() {
        blocks.push(TextBlock::Paragraph(paragraph.join("\n")));
        paragraph.clear();
    }
}

/// Flushes the open list when its kind ends.
fn push_list(blocks: &mut Vec<TextBlock>, list: &mut Option<(bool, Vec<ListItem>)>) {
    if let Some((ordered, items)) = list.take() {
        blocks.push(TextBlock::List { ordered, items });
    }
}

/// Appends one item, switching list kind when the marker family changes.
fn push_item(
    blocks: &mut Vec<TextBlock>,
    list: &mut Option<(bool, Vec<ListItem>)>,
    ordered: bool,
    item: ListItem,
) {
    match list {
        Some((current, items)) if *current == ordered => items.push(item),
        Some(_) => {
            push_list(blocks, list);
            *list = Some((ordered, vec![item]));
        }
        None => *list = Some((ordered, vec![item])),
    }
}

/// Recognizes an ATX heading and returns its level and text.
fn heading_line(line: &str) -> Option<(u8, &str)> {
    let hashes = line.len() - line.trim_start_matches('#').len();
    if !(1..=6).contains(&hashes) {
        return None;
    }
    let rest = line[hashes..].strip_prefix(' ')?;
    Some((hashes as u8, rest.trim_end_matches('#')))
}

/// Recognizes a bullet marker and returns its text.
fn bullet_line(line: &str) -> Option<&str> {
    ["- ", "* ", "• "]
        .iter()
        .find_map(|marker| line.strip_prefix(marker))
}

/// Recognizes a numbered marker and returns the marker and text.
fn numbered_line(line: &str) -> Option<ListItem> {
    let (digits, rest) = line.split_once('.')?;
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let text = rest.strip_prefix(' ')?;
    Some(ListItem {
        marker: format!("{digits}."),
        text: text.to_owned(),
    })
}

/// Renders parsed text as stacked blocks.
fn blocks_element(theme: &Theme, text: &str, muted: bool) -> AnyElement {
    let mut column = div().flex().flex_col().gap(theme.sp(8.)).w_full();
    for block in text_blocks(text) {
        column = column.child(block_element(theme, &block, muted));
    }
    column.into_any_element()
}

/// Renders one parsed block.
fn block_element(theme: &Theme, block: &TextBlock, muted: bool) -> AnyElement {
    match block {
        TextBlock::Heading { level, text } => {
            let size = match level {
                1 => 15.0,
                2 => 13.6,
                _ => 12.9,
            };
            div()
                .w_full()
                .pt(theme.sp(4.))
                .text_size(theme.fs(size))
                .line_height(theme.fs(size + 4.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(if muted { theme.muted } else { theme.text })
                .child(text.clone())
                .into_any_element()
        }
        TextBlock::Paragraph(text) => div()
            .w_full()
            .text_size(theme.fs(BODY_TEXT_SIZE))
            .line_height(theme.fs(21.2))
            .text_color(if muted { theme.muted } else { theme.text })
            .child(text.clone())
            .into_any_element(),
        TextBlock::List { ordered, items } => {
            let mut column = div().flex().flex_col().gap(theme.sp(3.)).w_full();
            for item in items {
                let marker = if *ordered {
                    item.marker.clone()
                } else {
                    "•".to_owned()
                };
                column = column.child(
                    div()
                        .flex()
                        .flex_row()
                        .items_start()
                        .gap(theme.sp(8.))
                        .w_full()
                        .child(
                            div()
                                .flex_none()
                                .min_w(theme.sp(14.))
                                .text_size(theme.fs(BODY_TEXT_SIZE))
                                .line_height(theme.fs(21.2))
                                .text_color(theme.muted)
                                .child(marker),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .text_size(theme.fs(BODY_TEXT_SIZE))
                                .line_height(theme.fs(21.2))
                                .text_color(if muted { theme.muted } else { theme.text })
                                .child(item.text.clone()),
                        ),
                );
            }
            column.into_any_element()
        }
        TextBlock::Code { language, text } => div()
            .flex()
            .flex_col()
            .gap(theme.sp(3.))
            .w_full()
            .p(theme.sp(8.))
            .rounded(theme.corner())
            .border_1()
            .border_color(theme.border)
            .bg(theme.code_background)
            .when(!language.is_empty(), |element| {
                element.child(
                    div()
                        .text_size(theme.fs(9.5))
                        .text_color(theme.muted)
                        .child(language.clone()),
                )
            })
            .child(
                div()
                    .w_full()
                    .font(theme.mono_font())
                    .text_size(theme.fs(11.8))
                    .line_height(theme.fs(17.))
                    .text_color(theme.text)
                    .child(text.clone()),
            )
            .into_any_element(),
        TextBlock::Rule => div()
            .w_full()
            .h(px(1.))
            .my(theme.sp(4.))
            .bg(theme.border)
            .into_any_element(),
    }
}

/// Renders one contiguous run of tool rows as a collapsible activity section.
fn tool_activity_group(
    app: &mut DesktopApp,
    theme: &Theme,
    start: usize,
    end: usize,
    cx: &mut Context<DesktopApp>,
) -> AnyElement {
    let collapsed = app.transcript.tool_group_collapsed();
    let count = end - start;
    let first_seq = app.transcript.rows()[start].seq;
    let has_error = app.transcript.rows()[start..end]
        .iter()
        .any(|row| row.kind == RowKind::ToolError);
    let expanded = !collapsed || has_error;
    let mut known_duration = 0;
    let mut total_duration = 0;
    {
        let rows = app.transcript.rows();
        for row in &rows[start..end] {
            if let Some(duration) = app.transcript.duration_of(row.seq) {
                known_duration += 1;
                total_duration += duration;
            }
        }
    }
    let mut list = div()
        .flex()
        .flex_col()
        .w_full()
        .pl(theme.sp(8.))
        .border_l_1()
        .border_color(theme.border);

    if expanded {
        let rows = app.transcript.rows();
        for index in start..end {
            let row = &rows[index];
            list = list.child(activity_row(
                theme,
                row,
                app.transcript.tool_name_of(row.seq),
                app.transcript.duration_of(row.seq),
            ));
        }
    }
    let duration_label = (known_duration > 0).then(|| format_duration(total_duration));
    let header = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(theme.sp(8.))
        .w_full()
        .py(theme.sp(5.))
        .cursor_pointer()
        .hover(|style| style.text_color(theme.accent))
        .text_size(theme.fs(12.))
        .font_weight(gpui::FontWeight::MEDIUM)
        .id(gpui::SharedString::from(format!(
            "tool-activity-toggle-{first_seq}"
        )))
        .child(widgets::icon(
            if expanded {
                Icon::ChevronDown
            } else {
                Icon::ChevronRight
            },
            theme.sp(15.),
            theme.muted,
        ))
        .child(div().child("Tool activity"))
        .child(
            div()
                .text_color(theme.muted)
                .child(format!("· {count}")),
        )
        .when_some(duration_label, |element, duration| {
            element.child(
                div()
                    .text_color(theme.muted)
                    .child(format!("· {duration}")),
            )
        })
        .on_click(cx.listener(|app, _, _, cx| {
            app.transcript.toggle_tool_group();
            cx.notify();
        }));

    let header = header
        .border_t_1()
        .border_color(theme.border);
    let mut section = div()
        .flex()
        .flex_col()
        .w_full()
        .pl(theme.sp(44.))
        .pt(theme.sp(4.))
        .child(header);
    if expanded {
        section = section.child(list);
    }
    section.into_any_element()
}

/// Renders one tool row inside the activity section.
fn activity_row(
    theme: &Theme,
    row: &TranscriptRow,
    tool: Option<&str>,
    duration: Option<i64>,
) -> AnyElement {
    let failed = row.kind == RowKind::ToolError;
    let (body, summary) = split_tool_text(&row.text);
    let detail = body
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or(&row.title)
        .chars()
        .take(160)
        .collect::<String>();
    let mut content = div()
        .flex()
        .flex_col()
        .gap(theme.sp(3.))
        .flex_1()
        .min_w(px(0.))
        .child(
            div()
                .min_w(px(0.))
                .truncate()
                .text_size(theme.fs(11.6))
                .text_color(theme.text)
                .child(detail),
        );
    if !body.trim().is_empty() {
        content = content.child(
            div()
                .w_full()
                .font(theme.mono_font())
                .text_size(theme.fs(11.2))
                .line_height(theme.fs(16.))
                .text_color(theme.muted)
                .child(body.to_owned()),
        );
    }
    if let Some(meta) = summary.and_then(summary_without_duration) {
        content = content.child(
            div()
                .text_size(theme.fs(10.5))
                .text_color(theme.muted)
                .child(meta.to_owned()),
        );
    }
    div()
        .flex()
        .flex_row()
        .items_start()
        .gap(theme.sp(8.))
        .w_full()
        .py(theme.sp(3.))
        .child(
            div()
                .flex_none()
                .w(theme.sp(16.))
                .text_size(theme.fs(11.6))
                .text_color(if failed { theme.error } else { theme.accent })
                .child(if failed { "!" } else { "✓" }),
        )
        .child(
            div()
                .flex_none()
                .w(theme.sp(52.))
                .truncate()
                .text_size(theme.fs(11.6))
                .text_color(theme.text)
                .child(tool.unwrap_or("tool").to_owned()),
        )
        .child(content)
        .child(
            div()
                .flex_none()
                .text_size(theme.fs(10.8))
                .text_color(theme.muted)
                .child(duration.map_or_else(|| "—".to_owned(), format_duration)),
        )
        .into_any_element()
}

/// Splits the projection's trailing tool summary line from the output body.
fn split_tool_text(text: &str) -> (&str, Option<&str>) {
    if let Some(index) = text.rfind("\n— ") {
        return (&text[..index], Some(&text[index + "\n— ".len()..]));
    }
    if let Some(summary) = text.strip_prefix("— ") {
        return ("", Some(summary));
    }
    (text, None)
}

/// Drops the duration part of a tool summary already shown in the duration column.
fn summary_without_duration(summary: &str) -> Option<&str> {
    summary
        .split_once(" · ")
        .map(|(_, rest)| rest)
        .filter(|rest| !rest.is_empty())
}

/// Formats a millisecond duration for the activity column.
fn format_duration(ms: i64) -> String {
    if ms < 1_000 {
        format!("{ms}ms")
    } else if ms < 60_000 {
        format!("{:.1}s", ms as f64 / 1_000.0)
    } else {
        let seconds = ms / 1_000;
        format!("{}m {}s", seconds / 60, seconds % 60)
    }
}

fn attention_pane(
    app: &mut DesktopApp,
    theme: &Theme,
    cx: &mut Context<DesktopApp>,
) -> AnyElement {
    let has_attention = !app.pending_questions.is_empty() || !app.pending_approvals.is_empty();
    if !has_attention {
        return div().into_any_element();
    }
    let mut column = div()
        .flex()
        .flex_col()
        .gap(theme.sp(8.))
        .w_full()
        .max_w(theme.sp(READING_COLUMN_WIDTH))
        .mx_auto();

    let question_count = app
        .pending_questions
        .first()
        .map_or(0, |card| card.questions().len());
    let question_inputs: Vec<gpui::Entity<crate::shell::text_input::TextInput>> = (0..question_count)
        .map(|index| {
            app.text_field(
                QUESTION_KEYS[index.min(QUESTION_KEYS.len() - 1)],
                "Or type an answer",
                false,
                cx,
            )
        })
        .collect();
    let custom_values: Vec<String> = question_inputs
        .iter()
        .map(|input| input.read(cx).text().to_owned())
        .collect();
    if let Some(card) = app.pending_questions.first_mut() {
        for (index, value) in custom_values.iter().enumerate() {
            card.set_custom(index, value);
        }
    }

    if let Some(card) = app.pending_questions.first_mut() {
        let questions = card.questions().to_vec();
        let mut body = div()
            .flex()
            .flex_col()
            .gap(theme.sp(6.))
            .w_full()
            .child(
                div()
                    .text_size(theme.fs(12.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child("The agent needs an answer"),
            );
        for (index, question) in questions.iter().enumerate() {
            if let Some(header) = &question.header {
                body = body.child(
                    div()
                        .text_size(theme.fs(11.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(theme.accent)
                        .child(header.clone()),
                );
            }
            body = body.child(
                div()
                    .text_size(theme.fs(12.))
                    .child(question.question.clone()),
            );
            if let Some(detail) = &question.detail {
                body = body.child(
                    div()
                        .text_size(theme.fs(10.5))
                        .text_color(theme.muted)
                        .child(detail.clone()),
                );
            }
            let chosen = card.chosen(index).to_vec();
            let multi = question.multi_select;
            let options: Vec<String> = question
                .options
                .iter()
                .map(|option| option.label.clone())
                .collect();
            let mut option_row = div().flex().flex_row().gap(theme.sp(4.)).flex_wrap();
            for label in options {
                let selected = chosen.contains(&label);
                let key = format!("question-{index}-{label}");
                let value = label.clone();
                let mark = if selected {
                    "\u{2713} "
                } else if multi {
                    "\u{2610} "
                } else {
                    ""
                };
                option_row = option_row.child(
                    div()
                        .px(theme.sp(8.))
                        .py(theme.sp(3.))
                        .rounded(theme.corner())
                        .border_1()
                        .border_color(if selected { theme.accent } else { theme.border })
                        .bg(if selected { theme.selection } else { theme.surface })
                        .cursor_pointer()
                        .text_size(theme.fs(11.))
                        .id(gpui::SharedString::from(key))
                        .child(format!("{mark}{label}"))
                        .on_click(cx.listener(move |app, _, _, cx| {
                            if let Some(card) = app.pending_questions.first_mut() {
                                card.choose(index, &value);
                            }
                            cx.notify();
                        })),
                );
            }
            body = body.child(option_row);
            let input = question_inputs[index].clone();
            body = body.child(
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
            );
        }
        body = body.child(
            div()
                .flex()
                .flex_row()
                .justify_end()
                .gap(theme.sp(6.))
                .child(
                    widgets::Button::new(theme, "Skip")
                        .build()
                        .id("question-skip")
                        .on_click(cx.listener(|app, _, _, cx| {
                            app.submit_question(true);
                            cx.notify();
                        })),
                )
                .child(
                    widgets::Button::new(theme, "Answer")
                        .primary()
                        .build()
                        .id("question-answer")
                        .on_click(cx.listener(|app, _, _, cx| {
                            app.submit_question(false);
                            cx.notify();
                        })),
                ),
        );
        column = column.child(widgets::card(theme, body));
    }

    let scope = app
        .pending_approvals
        .first()
        .and_then(|pending| pending.request.call_id.as_deref())
        .and_then(|call_id| app.transcript.call_scope(call_id))
        .map(str::to_owned);

    if let Some(pending) = app.pending_approvals.first().cloned() {
        let queued = app.pending_approvals.len();
        let reason = pending
            .request
            .display_reason
            .as_ref()
            .map(|display| display.en.clone())
            .or_else(|| pending.request.reason.clone());
        let mut card = div()
            .flex()
            .flex_col()
            .gap(theme.sp(6.))
            .w_full()
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(theme.sp(6.))
                    .child(
                        div()
                            .text_size(theme.fs(12.))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(theme.warn)
                            .child("Approval required"),
                    )
                    .child(
                        div()
                            .text_size(theme.fs(11.5))
                            .text_color(theme.text)
                            .child(pending.request.tool_name.clone()),
                    )
                    .when(queued > 1, |element| {
                        element.child(
                            div()
                                .text_size(theme.fs(10.))
                                .text_color(theme.muted)
                                .child(format!("{queued} queued")),
                        )
                    }),
            );
        if let Some(reason) = reason {
            card = card.child(
                div()
                    .text_size(theme.fs(11.))
                    .text_color(theme.muted)
                    .child(reason),
            );
        }
        if let Some(scope) = scope {
            card = card.child(
                div()
                    .w_full()
                    .p(theme.sp(8.))
                    .rounded(theme.corner())
                    .border_1()
                    .border_color(theme.border)
                    .bg(theme.code_background)
                    .debug_selector(|| "approval-scope".to_owned())
                    .font(theme.mono_font())
                    .text_size(theme.fs(11.2))
                    .text_color(theme.text)
                    .child(scope),
            );
        }
        card = card.child(
            div()
                .flex()
                .flex_row()
                .justify_end()
                .gap(theme.sp(6.))
                .child(
                    widgets::Button::new(theme, "Allow once")
                        .primary()
                        .build()
                        .id("approval-allow")
                        .debug_selector(|| "approval-allow".to_owned())
                        .on_click(cx.listener(|app, _, _, cx| {
                            app.decide_approval(true);
                            cx.notify();
                        })),
                )
                .child(
                    widgets::Button::new(theme, "Deny")
                        .danger()
                        .build()
                        .id("approval-deny")
                        .debug_selector(|| "approval-deny".to_owned())
                        .on_click(cx.listener(|app, _, _, cx| {
                            app.decide_approval(false);
                            cx.notify();
                        })),
                ),
        );
        column = column.child(widgets::card(theme, card));
    }

    div()
        .flex()
        .flex_col()
        .items_center()
        .w_full()
        .px(theme.sp(14.))
        .child(column)
        .into_any_element()
}

const QUESTION_KEYS: [&str; 4] = ["question-0", "question-1", "question-2", "question-3"];

fn composer(
    app: &mut DesktopApp,
    theme: &Theme,
    cx: &mut Context<DesktopApp>,
) -> AnyElement {
    let input = app.text_field("composer", "Message DeepSeek · Ctrl+Enter to send", true, cx);
    if let Some(text) = app.pending_composer_text.take() {
        input.update(cx, |input, cx| input.set_text(text, cx));
    }
    let transport = app.transport.is_some();
    let busy = app.chat_loading;
    let uploading = app.attachment_receiver.is_some();
    let enabled = !busy && transport;
    let can_send = enabled
        && !app.chat_text.trim().is_empty()
        && app.attachment_receiver.is_none();
    let attachment_name = app.attachment_name.clone();
    let has_attachment = app.attachment.is_some();
    let model_label = app.model_label();
    let menu_open = app.model_menu_open;
    let mode_label = current_mode_label(app);
    let attach_enabled = !uploading && !busy;
    let attach_hint = if busy {
        "Wait for the current turn before attaching."
    } else if uploading {
        "The current attachment is still uploading."
    } else {
        "Attach a file to the next prompt."
    };
    let send_hint = if busy {
        "Stop the current turn."
    } else if !transport {
        "Connect to the Host to send."
    } else if uploading {
        "Wait for the attachment upload."
    } else if app.chat_text.trim().is_empty() {
        "Type a message first."
    } else {
        "Send (Ctrl+Enter)."
    };
    let disabled_reason = if busy {
        None
    } else if !transport {
        Some("Connect to the Host before sending.")
    } else if uploading {
        Some("Wait for the attachment to finish uploading.")
    } else if app.chat_text.trim().is_empty() {
        Some("Type a message first.")
    } else {
        None
    };

    let mut body = div()
        .flex()
        .flex_col()
        .gap(theme.sp(6.))
        .w_full()
        .p(theme.sp(8.))
        .rounded(theme.sp(10.))
        .border_1()
        .border_color(theme.border)
        .bg(theme.code_background)
        .debug_selector(|| "composer-shell".to_owned())
        .child(
            div()
                .w_full()
                .min_h(theme.sp(54.))
                .child(crate::shell::text_input::text_input_element(
                    &input,
                    theme,
                    theme.sp(60.),
                )),
        )
        .when(app.chat_text.trim().ends_with('@'), |element| {
            element.child(
                div()
                    .text_size(theme.fs(10.))
                    .text_color(theme.muted)
                    .child("Type a workspace-relative path after @, then send."),
            )
        })
        .child(
            div()
                .flex()
                .flex_col()
                .gap(theme.sp(4.))
                .w_full()
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .w_full()
                        .gap(theme.sp(8.))
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap(px(2.))
                                .min_w(px(0.))
                                .child(
                                    icon_button(
                                        theme,
                                        "composer-attach",
                                        Icon::Plus,
                                        attach_hint,
                                        attach_enabled,
                                    )
                                    .on_click(cx.listener(|app, _, _, cx| {
                                            if app.chat_loading || app.attachment_receiver.is_some() {
                                                return;
                                            }
                                            app.pick_attachment();
                                            cx.notify();
                                        })),
                                )
                                .child(
                                    icon_button(
                                        theme,
                                        "composer-file-picker",
                                        Icon::AtSign,
                                        "Insert a workspace file reference.",
                                        transport,
                                    )
                                    .on_click(cx.listener(|app, _, _, cx| {
                                        if app.transport.is_none() {
                                            return;
                                        }
                                        app.file_picker_open = !app.file_picker_open;
                                        if app.file_picker_open {
                                            app.file_picker_query.clear();
                                            app.set_input("file-picker-query", "", cx);
                                        }
                                        cx.notify();
                                    })),
                                )
                                .when(!attachment_name.is_empty(), |element| {
                                    element.child(
                                        div()
                                            .max_w(theme.sp(180.))
                                            .truncate()
                                            .text_size(theme.fs(10.))
                                            .text_color(theme.muted)
                                            .child(attachment_name.clone()),
                                    )
                                })
                                .when(uploading, |element| {
                                    element.child(
                                        div()
                                            .text_size(theme.fs(10.))
                                            .text_color(theme.muted)
                                            .child("Uploading…"),
                                    )
                                })
                                .when(has_attachment, |element| {
                                    element.child(
                                        icon_button(
                                            theme,
                                            "composer-clear-attachment",
                                            Icon::X,
                                            "Remove the attachment.",
                                            true,
                                        )
                                        .on_click(cx.listener(|app, _, _, cx| {
                                            app.attachment = None;
                                            app.attachment_name.clear();
                                            cx.notify();
                                        })),
                                    )
                                }),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap(px(4.))
                                .child(
                                    widgets::Button::new(theme, model_label.clone())
                                        .trailing_icon(Icon::ChevronDown)
                                        .enabled(transport)
                                        .build()
                                        .id("composer-model")
                                    .debug_selector(|| "composer-model".to_owned())
                                    .when(!transport, |button| {
                                        button.tooltip(widgets::HintView::tooltip(
                                            theme,
                                            "Connect to the Host to choose a model.",
                                        ))
                                    })
                                    .on_click(cx.listener(move |app, _, _, cx| {
                                        app.model_menu_open = !menu_open;
                                        cx.notify();
                                    })),
                                )
                                .child(
                                    widgets::Button::new(theme, mode_label.clone())
                                        .trailing_icon(Icon::ChevronDown)
                                        .enabled(transport)
                                        .build()
                                        .id("composer-mode")
                                    .debug_selector(|| "composer-mode".to_owned())
                                    .when(!transport, |button| {
                                        button.tooltip(widgets::HintView::tooltip(
                                            theme,
                                            "Connect to the Host to choose a send mode.",
                                        ))
                                    })
                                    .on_click(cx.listener(move |app, _, _, cx| {
                                        app.model_menu_open = !menu_open;
                                        cx.notify();
                                    })),
                                )
                                .child(
                                    icon_button(
                                        theme,
                                        if busy { "composer-stop" } else { "composer-send" },
                                        if busy { Icon::Stop } else { Icon::Send },
                                        send_hint,
                                        busy || can_send,
                                    )
                                    .on_click(cx.listener(|app, _, _, cx| {
                                        if app.chat_loading {
                                            app.cancel_turn();
                                            cx.notify();
                                            return;
                                        }
                                        if !app.chat_text.trim().is_empty()
                                            && app.transport.is_some()
                                            && app.attachment_receiver.is_none()
                                        {
                                            app.send_prompt();
                                            app.set_input("composer", "", cx);
                                        }
                                        cx.notify();
                                    })),
                                ),
                        ),
                )
                .when_some(disabled_reason, |element, reason| {
                    element.child(
                        div()
                            .text_size(theme.fs(9.5))
                            .text_color(theme.muted)
                            .child(reason),
                    )
                }),
        );
    if let Some(error) = &app.attachment_error {
        body = body.child(
            div()
                .text_size(theme.fs(10.))
                .text_color(theme.error)
                .child(error.clone()),
        );
    }

    let mut column = div()
        .flex()
        .flex_col()
        .items_center()
        .w_full()
        .px(theme.sp(14.))
        .pb(theme.sp(8.));
    if menu_open {
        column = column.child(composer_menu(app, theme, cx));
    }
    if app.file_picker_open {
        let query_input = app.text_field(
            "file-picker-query",
            "Filter loaded files by name or path",
            false,
            cx,
        );
        let query = app.file_picker_query.to_ascii_lowercase();
        let rows: Vec<ExplorerRow> = app
            .explorer
            .visible_rows()
            .into_iter()
            .filter(|row| {
                matches!(row.kind, ExplorerRowKind::File { .. })
                    && (query.is_empty()
                        || row.label.to_ascii_lowercase().contains(&query)
                        || row.path.to_ascii_lowercase().contains(&query))
            })
            .take(12)
            .collect();
        let mut list = div()
            .flex()
            .flex_col()
            .w_full()
            .max_w(theme.sp(COMPOSER_WIDTH))
            .max_h(theme.sp(140.))
            .id("file-picker-scroll")
            .overflow_y_scroll()
            .bg(theme.surface_raised)
            .rounded(theme.corner())
            .border_1()
            .border_color(theme.border)
            .px(theme.sp(6.))
            .py(theme.sp(4.))
            .gap(px(1.));
        if rows.is_empty() {
            list = list.child(
                div()
                    .text_size(theme.fs(10.))
                    .text_color(theme.muted)
                    .child("No loaded files match."),
            );
        }
        for row in rows {
            let path = row.path.clone();
            list = list.child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .px(theme.sp(6.))
                    .py(px(1.))
                    .rounded(theme.corner())
                    .cursor_pointer()
                    .hover(|style| style.bg(theme.surface))
                    .id(gpui::SharedString::from(format!("file-pick-{path}")))
                    .child(
                        div()
                            .text_size(theme.fs(10.5))
                            .truncate()
                            .child(row.label),
                    )
                    .child(
                        div()
                            .text_size(theme.fs(9.))
                            .text_color(theme.muted)
                            .truncate()
                            .child(path.clone()),
                    )
                    .on_click(cx.listener(move |app, _, _, cx| {
                        app.insert_file_reference(&path);
                        let text = app.chat_text.clone();
                        app.set_input("composer", &text, cx);
                        app.set_input("file-picker-query", "", cx);
                        cx.notify();
                    })),
            );
        }
        column = column
            .child(
                div()
                    .w_full()
                    .max_w(theme.sp(COMPOSER_WIDTH))
                    .h(theme.control_height())
                    .px(theme.sp(6.))
                    .rounded(theme.corner())
                    .border_1()
                    .border_color(theme.border)
                    .bg(theme.surface_raised)
                    .overflow_hidden()
                    .child(crate::shell::text_input::text_input_element(
                        &query_input,
                        theme,
                        theme.control_height() - px(2.),
                    )),
            )
            .child(list);
    }
    column = column.child(
        div()
            .w_full()
            .max_w(theme.sp(COMPOSER_WIDTH))
            .mx_auto()
            .child(body),
    );
    column.into_any_element()
}

/// The mode the composer will send with, naming plan mode when it is on.
fn current_mode_label(app: &DesktopApp) -> &'static str {
    if app.transcript.plan_active() {
        return "Plan";
    }
    match app.prompt_mode {
        PromptMode::Queue => "Queue",
        PromptMode::Steer => "Steer",
    }
}

/// A compact icon control for the composer's tool row.
///
/// @param theme — frame palette and metrics.
/// @param id — element id, also the harness selector.
/// @param icon — icon drawn inside the 27px hit area.
/// @param hint — tooltip shown while the control is hovered.
/// @param enabled — whether the control accepts a click.
/// @returns the control, ready for its click handler.
fn icon_button(
    theme: &Theme,
    id: &'static str,
    icon: Icon,
    hint: &'static str,
    enabled: bool,
) -> gpui::Stateful<Div> {
    let button = div()
        .flex()
        .items_center()
        .justify_center()
        .flex_none()
        .w(theme.sp(27.))
        .h(theme.sp(27.))
        .rounded(theme.corner())
        .text_color(theme.muted)
        .child(widgets::icon(icon, theme.sp(15.), theme.muted))
        .id(id)
        .debug_selector(move || id.to_owned())
        .tooltip(widgets::HintView::tooltip(theme, hint));
    if enabled {
        button
            .cursor_pointer()
            .hover(|style| style.bg(theme.surface_raised))
    } else {
        button.opacity(0.45)
    }
}

/// One selectable row inside the composer menu.
///
/// @param theme — frame palette and metrics.
/// @param id — element id, also the harness selector.
/// @param label — row text.
/// @param selected — whether the row's mode is the current one.
/// @param enabled — whether the row accepts a click.
/// @param hint — tooltip shown while the row is hovered.
/// @returns the row, ready for its click handler.
fn menu_row(
    theme: &Theme,
    id: &'static str,
    label: String,
    selected: bool,
    enabled: bool,
    hint: &'static str,
) -> gpui::Stateful<Div> {
    menu_row_with_arrow(theme, id, None, label, selected, enabled, hint)
}

/// One menu row that can end with a direction arrow.
///
/// The prompt-history rows use the arrow to show which way through the history
/// the row recalls, so the row reads the same as the arrow keys it mirrors.
///
/// @param theme — frame palette and metrics.
/// @param id — element id, also the harness selector.
/// @param arrow — trailing direction icon, or `None` for a plain row.
/// @param label — row text.
/// @param selected — whether the row's mode is the current one.
/// @param enabled — whether the row accepts a click.
/// @param hint — tooltip shown while the row is hovered.
/// @returns the row, ready for its click handler.
fn menu_row_with_arrow(
    theme: &Theme,
    id: &'static str,
    arrow: Option<Icon>,
    label: String,
    selected: bool,
    enabled: bool,
    hint: &'static str,
) -> gpui::Stateful<Div> {
    let row = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(theme.sp(6.))
        .w_full()
        .px(theme.sp(8.))
        .py(theme.sp(4.))
        .rounded(theme.corner())
        .text_size(theme.fs(11.))
        .text_color(if enabled { theme.text } else { theme.muted })
        .child(
            div()
                .flex_none()
                .w(theme.sp(12.))
                .text_color(theme.accent)
                .child(if selected { "\u{2713}" } else { "" }),
        )
        .child(label)
        .children(arrow.map(|name| {
            div()
                .flex_1()
                .flex()
                .justify_end()
                .child(widgets::icon(name, theme.sp(15.), theme.muted))
        }))
        .id(id)
        .debug_selector(move || id.to_owned())
        .tooltip(widgets::HintView::tooltip(theme, hint));
    if enabled {
        row.cursor_pointer()
            .hover(|style| style.bg(theme.surface))
    } else {
        row.opacity(0.45)
    }
}

/// Composer popover: send mode, running-turn action, prompt history, and model.
fn composer_menu(
    app: &mut DesktopApp,
    theme: &Theme,
    cx: &mut Context<DesktopApp>,
) -> AnyElement {
    let transport = app.transport.is_some();
    let busy = app.chat_loading;
    let plan_available = app.plan_command_available();
    let plan_active = app.transcript.plan_active();
    let queue_mode = app.prompt_mode == PromptMode::Queue;
    let steer_mode = app.prompt_mode == PromptMode::Steer;
    let has_text = !app.chat_text.trim().is_empty();
    let has_history = !app.prompt_history.is_empty();
    let mut list = div()
        .flex()
        .flex_col()
        .gap(px(1.))
        .w_full()
        .max_w(theme.sp(COMPOSER_WIDTH))
        .mx_auto()
        .mb(theme.sp(4.))
        .p(theme.sp(6.))
        .rounded(theme.corner())
        .border_1()
        .border_color(theme.border)
        .bg(theme.surface_raised)
        .shadow_lg()
        .debug_selector(|| "composer-menu".to_owned())
        .child(
            div()
                .px(theme.sp(8.))
                .pt(theme.sp(3.))
                .pb(theme.sp(2.))
                .text_size(theme.fs(9.5))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(theme.muted)
                .child("Mode"),
        )
        .child(
            menu_row(
                theme,
                "composer-mode-queue",
                "Queue".to_owned(),
                queue_mode,
                transport,
                if transport {
                    "Send the next prompt to the run queue."
                } else {
                    "Connect to the Host to choose a send mode."
                },
            )
            .on_click(cx.listener(|app, _, _, cx| {
                if app.transport.is_none() {
                    return;
                }
                app.prompt_mode = PromptMode::Queue;
                app.model_menu_open = false;
                cx.notify();
            })),
        )
        .child(
            menu_row(
                theme,
                "composer-mode-steer",
                "Steer".to_owned(),
                steer_mode,
                transport,
                if transport {
                    "Steer the running turn with the next prompt."
                } else {
                    "Connect to the Host to choose a send mode."
                },
            )
            .on_click(cx.listener(|app, _, _, cx| {
                if app.transport.is_none() {
                    return;
                }
                app.prompt_mode = PromptMode::Steer;
                app.model_menu_open = false;
                cx.notify();
            })),
        )
        .child(
            menu_row(
                theme,
                "composer-mode-plan",
                format!("Plan mode · {}", if plan_active { "on" } else { "off" }),
                plan_active,
                transport && plan_available,
                if plan_available {
                    "Toggle plan mode through the runtime's /plan command."
                } else {
                    "This profile has no /plan command."
                },
            )
            .on_click(cx.listener(|app, _, _, cx| {
                if app.transport.is_none() || !app.plan_command_available() {
                    return;
                }
                app.toggle_plan_mode();
                app.model_menu_open = false;
                cx.notify();
            })),
        )
        .child(
            menu_row(
                theme,
                "composer-mode-ask",
                "Ask".to_owned(),
                false,
                false,
                "Ask is not available in this profile.",
            ),
        )
        .when(busy, |element| {
            element
                .child(
                    div()
                        .px(theme.sp(8.))
                        .pt(theme.sp(6.))
                        .pb(theme.sp(2.))
                        .text_size(theme.fs(9.5))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(theme.muted)
                        .child("While running"),
                )
                .child(
                    menu_row(
                        theme,
                        "composer-queue",
                        if steer_mode {
                            "Steer this run".to_owned()
                        } else {
                            "Queue this message".to_owned()
                        },
                        false,
                        transport && has_text,
                        if has_text {
                            "Send the draft with the selected mode."
                        } else {
                            "Type a message first."
                        },
                    )
                    .on_click(cx.listener(|app, _, _, cx| {
                        if app.chat_loading
                            && !app.chat_text.trim().is_empty()
                            && app.transport.is_some()
                        {
                            app.queue_prompt();
                            app.set_input("composer", "", cx);
                            app.model_menu_open = false;
                        }
                        cx.notify();
                    })),
                )
        })
        .child(
            div()
                .px(theme.sp(8.))
                .pt(theme.sp(6.))
                .pb(theme.sp(2.))
                .text_size(theme.fs(9.5))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(theme.muted)
                .child("Prompt history"),
        )
        .child(
            menu_row_with_arrow(
                theme,
                "composer-history",
                Some(Icon::ChevronUp),
                "Previous prompt".to_owned(),
                false,
                has_history,
                if has_history {
                    "Recall an earlier prompt into the composer."
                } else {
                    "No prompt history yet."
                },
            )
            .on_click(cx.listener(|app, _, _, cx| {
                if app.prompt_history.is_empty() {
                    return;
                }
                app.recall_prompt_history(1);
                if let Some(text) = app.pending_composer_text.clone() {
                    app.set_input("composer", &text, cx);
                }
                app.model_menu_open = false;
                cx.notify();
            })),
        )
        .child(
            menu_row_with_arrow(
                theme,
                "composer-history-next",
                Some(Icon::ChevronDown),
                "Next prompt".to_owned(),
                false,
                has_history,
                if has_history {
                    "Recall a newer prompt into the composer."
                } else {
                    "No prompt history yet."
                },
            )
            .on_click(cx.listener(|app, _, _, cx| {
                if app.prompt_history.is_empty() {
                    return;
                }
                app.recall_prompt_history(-1);
                if let Some(text) = app.pending_composer_text.clone() {
                    app.set_input("composer", &text, cx);
                }
                app.model_menu_open = false;
                cx.notify();
            })),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .px(theme.sp(8.))
                .pt(theme.sp(6.))
                .pb(theme.sp(2.))
                .child(
                    div()
                        .text_size(theme.fs(9.5))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(theme.muted)
                        .child("Model"),
                )
                .child(
                    widgets::Button::icon_only(theme, Icon::Sync)
                        .build()
                        .id("model-reload")
                        .on_click(cx.listener(|app, _, _, cx| {
                            app.refresh_models();
                            cx.notify();
                        })),
                ),
        );
    if app.model_receiver.is_some() {
        list = list.child(note(theme, "Loading models from Host…"));
    } else if let Some(error) = &app.model_error {
        list = list.child(
            div()
                .text_size(theme.fs(10.))
                .text_color(theme.error)
                .child(error.clone()),
        );
    } else if app.model_catalog_empty_now() {
        list = list.child(note(
            theme,
            "No model providers are available. The Host controls provider configuration.",
        ));
        list = list.child(
            widgets::Button::new(theme, "Host settings")
                .build()
                .id("model-host-settings")
                .on_click(cx.listener(|app, _, _, cx| {
                    app.switch_view(crate::shell::app::ActiveView::Settings);
                    app.model_menu_open = false;
                    cx.notify();
                })),
        );
    } else if let Some(catalog) = app.model_catalog.clone() {
        for group in &catalog.groups {
            list = list.child(
                div()
                    .px(theme.sp(8.))
                    .pt(theme.sp(4.))
                    .text_size(theme.fs(10.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(theme.muted)
                    .child(group.name.clone()),
            );
            for model in &group.models {
                let provider = group.id.clone();
                let model_id = model.id.clone();
                let effort = model
                    .reasoning
                    .as_ref()
                    .and_then(|reasoning| reasoning.default_effort.clone());
                let current = app.model_selection.as_ref().is_some_and(|selection| {
                    selection.provider == provider && selection.model == model_id
                });
                let label = format!("{} · {}", model.name, model.id);
                list = list.child(
                    div()
                        .px(theme.sp(8.))
                        .py(theme.sp(4.))
                        .rounded(theme.corner())
                        .cursor_pointer()
                        .when(current, |element| element.bg(theme.selection))
                        .hover(|style| style.bg(theme.surface))
                        .text_size(theme.fs(11.))
                        .id(gpui::SharedString::from(format!("model-{provider}-{model_id}")))
                        .child(label)
                        .on_click(cx.listener(move |app, _, _, cx| {
                            app.select_model_ids(&provider, &model_id, effort.clone());
                            app.model_menu_open = false;
                            cx.notify();
                        })),
                );
            }
        }
        let default = catalog.default.clone();
        if !default.model.is_empty() && app.model_selection.as_ref() != Some(&default) {
            list = list.child(
                widgets::Button::new(theme, "Use profile default")
                    .build()
                    .id("model-default")
                    .on_click(cx.listener(move |app, _, _, cx| {
                        app.select_model_ids(
                            &default.provider,
                            &default.model,
                            default.reasoning_effort.clone(),
                        );
                        app.model_menu_open = false;
                        cx.notify();
                    })),
            );
        }
    }
    list.into_any_element()
}

fn empty_state(
    app: &mut DesktopApp,
    theme: &Theme,
    cx: &mut Context<DesktopApp>,
) -> AnyElement {
    let connected = app.transport.is_some();
    let loading = app.sessions_busy();
    let empty_search = app
        .session_search
        .as_ref()
        .is_some_and(|results| results.items.is_empty());
    let has_sessions = !app.sessions.is_empty();
    let no_provider = app.model_catalog_empty_now();
    let model_error = app.model_error.clone();
    let default_model = app.model_catalog.as_ref().map(|catalog| catalog.default.model.clone());

    if !connected {
        return centered(
            theme,
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap(theme.sp(6.))
                .child(note(theme, "Host unavailable"))
                .child(heading(theme, "The Host isn’t reachable"))
                .child(
                    div()
                        .text_size(theme.fs(13.))
                        .text_color(theme.muted)
                        .child("A new Session needs the Host. Retry the connection to continue."),
                )
                .child(
                    widgets::Button::new(theme, "New Session")
                        .leading_icon(Icon::Plus)
                        .enabled(false)
                        .build()
                        .id("empty-new-session"),
                ),
        );
    }
    if empty_search {
        return centered(
            theme,
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap(theme.sp(6.))
                .child(heading(theme, "No matching Sessions"))
                .child(
                    div()
                        .text_size(theme.fs(12.))
                        .text_color(theme.muted)
                        .child("Try a different search or clear the current query."),
                )
                .child(
                    widgets::Button::new(theme, "Clear search")
                        .build()
                        .id("empty-clear-search")
                        .on_click(cx.listener(|app, _, _, cx| {
                            app.set_input("session-search", "", cx);
                            app.reload_sessions();
                            cx.notify();
                        })),
                ),
        );
    }
    if let Some(error) = app.sessions_error.clone() {
        return centered(
            theme,
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap(theme.sp(6.))
                .child(heading(theme, "Could not load Sessions"))
                .child(
                    div()
                        .text_size(theme.fs(12.))
                        .text_color(theme.error)
                        .child(error),
                )
                .child(
                    widgets::Button::new(theme, "Retry Session list")
                        .build()
                        .id("empty-retry-sessions")
                        .on_click(cx.listener(|app, _, _, cx| {
                            app.reload_sessions();
                            cx.notify();
                        })),
                ),
        );
    }
    if loading {
        return centered(
            theme,
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap(theme.sp(6.))
                .child(heading(theme, "Loading sessions…")),
        );
    }
    if has_sessions {
        return centered(
            theme,
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap(theme.sp(6.))
                .child(heading(theme, "Choose a Session"))
                .child(
                    div()
                        .text_size(theme.fs(13.))
                        .text_color(theme.muted)
                        .child("Open a Session from the sidebar, or start another one."),
                )
                .child(
                    widgets::Button::new(theme, "New Session")
                        .leading_icon(Icon::Plus)
                        .primary()
                        .build()
                        .id("empty-new-session-2")
                        .on_click(cx.listener(|app, _, _, cx| {
                            app.new_session();
                            cx.notify();
                        })),
                ),
        );
    }
    let mut card = div()
        .flex()
        .flex_col()
        .gap(theme.sp(8.))
        .w(theme.sp(520.))
        .p(theme.sp(18.))
        .rounded(theme.sp(10.))
        .border_1()
        .border_color(theme.border)
        .bg(theme.surface)
        .child(note(theme, if no_provider { "No provider" } else { "First run" }))
        .child(heading(
            theme,
            if no_provider {
                "No model provider is available"
            } else {
                "Start your first Session"
            },
        ))
        .child(
            div()
                .text_size(theme.fs(13.))
                .text_color(theme.muted)
                .child(if no_provider {
                    if model_error.is_some() {
                        "The Host could not load its model catalog. A Session can still be created; check Host configuration before sending a prompt."
                    } else {
                        "You can create a Session now. Sending a prompt needs a model provider configured by the Host."
                    }
                } else {
                    "Create a Session to begin. Each Session keeps its own conversation and workspace in Explorer."
                }),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(theme.sp(6.))
                .child(
                    widgets::Button::new(theme, "New Session")
                        .leading_icon(Icon::Plus)
                        .primary()
                        .build()
                        .id("onboarding-new-session")
                        .on_click(cx.listener(|app, _, _, cx| {
                            app.new_session();
                            cx.notify();
                        })),
                )
                .when(no_provider, |element| {
                    element.child(
                        widgets::Button::new(theme, "Host settings")
                            .build()
                            .id("onboarding-host-settings")
                            .on_click(cx.listener(|app, _, _, cx| {
                                app.switch_view(crate::shell::app::ActiveView::Settings);
                                cx.notify();
                            })),
                    )
                }),
        );
    if let Some(error) = model_error {
        card = card.child(
            div()
                .text_size(theme.fs(11.))
                .text_color(theme.warn)
                .child(error),
        );
    } else if let Some(default) = default_model.filter(|model| !model.is_empty()) {
        card = card.child(
            div()
                .text_size(theme.fs(11.))
                .text_color(theme.muted)
                .child(format!("Profile default · {default}")),
        );
    }
    centered(theme, card)
}

fn centered(theme: &Theme, child: impl IntoElement) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .size_full()
        .p(theme.sp(24.))
        .child(child)
        .into_any_element()
}

fn heading(theme: &Theme, text: &str) -> AnyElement {
    div()
        .text_size(theme.fs(20.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .child(text.to_owned())
        .into_any_element()
}

fn note(theme: &Theme, text: &str) -> AnyElement {
    div()
        .text_size(theme.fs(11.))
        .text_color(theme.muted)
        .child(text.to_owned())
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_blocks_render_headings_bullets_code_and_paragraphs() {
        let blocks = text_blocks(
            "# Summary\nIntro line\n- first\n- second\n1. one\n2. two\n```rust\nfn main() {}\n```\nDone.",
        );
        assert!(matches!(
            &blocks[0],
            TextBlock::Heading { level: 1, text } if text == "Summary"
        ));
        assert!(matches!(&blocks[1], TextBlock::Paragraph(text) if text == "Intro line"));
        let TextBlock::List { ordered: false, items } = &blocks[2] else {
            panic!("expected a bullet list");
        };
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].marker, "•");
        assert_eq!(items[0].text, "first");
        let TextBlock::List { ordered: true, items } = &blocks[3] else {
            panic!("expected numbered items");
        };
        assert_eq!(items[1].marker, "2.");
        let TextBlock::Code { language, text } = &blocks[4] else {
            panic!("expected a code block");
        };
        assert_eq!(language, "rust");
        assert_eq!(text, "fn main() {}");
        assert!(matches!(&blocks[5], TextBlock::Paragraph(text) if text == "Done."));
    }

    #[test]
    fn text_blocks_keep_unclosed_fences_and_rules() {
        let blocks = text_blocks("above\n---\n```sh\ncargo test");
        assert!(matches!(&blocks[0], TextBlock::Paragraph(text) if text == "above"));
        assert!(matches!(&blocks[1], TextBlock::Rule));
        let TextBlock::Code { language, text } = &blocks[2] else {
            panic!("expected the unclosed fence to stay code");
        };
        assert_eq!(language, "sh");
        assert_eq!(text, "cargo test");
    }

    #[test]
    fn tool_text_separates_the_projection_summary_from_output() {
        let (body, summary) = split_tool_text("out\n[stderr]\nboom\n— 150ms · stderr 4B");
        assert_eq!(body, "out\n[stderr]\nboom");
        assert_eq!(summary, Some("150ms · stderr 4B"));
        assert_eq!(summary_without_duration("150ms · stderr 4B"), Some("stderr 4B"));
        assert_eq!(summary_without_duration("150ms"), None);
        assert_eq!(split_tool_text("out"), ("out", None));
    }

    #[test]
    fn durations_read_in_activity_units() {
        assert_eq!(format_duration(640), "640ms");
        assert_eq!(format_duration(1_500), "1.5s");
        assert_eq!(format_duration(65_000), "1m 5s");
    }

    #[test]
    fn conversation_column_and_composer_hold_the_v2_width_caps() {
        let mut harness = crate::harness::Harness::open_with(1920., 1080., |app| {
            app.selected_session = Some("layout-session".to_owned());
        });
        let reading = harness
            .bounds("transcript-reading-column")
            .expect("conversation reading column renders");
        assert!(
            (f32::from(reading.size.width) - READING_COLUMN_WIDTH).abs() < 0.5,
            "reading column was {}",
            f32::from(reading.size.width)
        );
        let agent = harness.bounds("agent-column").expect("agent column renders");
        let expected_x = agent.origin.x + (agent.size.width - reading.size.width) / 2.0;
        assert!(
            f32::from(reading.origin.x - expected_x).abs() < 1.0,
            "reading column was not centered: x={} agent={}",
            f32::from(reading.origin.x),
            f32::from(agent.origin.x)
        );
        let composer = harness
            .bounds("composer-shell")
            .expect("composer renders");
        assert!(
            (f32::from(composer.size.width) - COMPOSER_WIDTH).abs() < 0.5,
            "composer was {}",
            f32::from(composer.size.width)
        );
        for selector in [
            "composer-attach",
            "composer-file-picker",
            "composer-model",
            "composer-mode",
            "composer-send",
        ] {
            assert!(harness.bounds(selector).is_some(), "{selector} missing");
        }
    }

    #[test]
    fn composer_mode_selector_reaches_queue_steer_plan_and_ask() {
        let mut harness = crate::harness::Harness::open_with(1920., 1080., |app| {
            app.selected_session = Some("mode-session".to_owned());
        });
        assert!(harness.click("composer-mode"), "mode button did not click");
        assert!(harness.bounds("composer-menu").is_some(), "menu did not open");
        for selector in [
            "composer-mode-queue",
            "composer-mode-steer",
            "composer-mode-plan",
            "composer-mode-ask",
        ] {
            assert!(harness.bounds(selector).is_some(), "{selector} missing");
        }
    }

    #[test]
    fn approval_card_shows_the_scoped_command_and_only_once_or_deny() {
        use dsh_client_core::follow::SessionWireEvent;
        let mut harness = crate::harness::Harness::open_with(1920., 1080., |app| {
            app.selected_session = Some("approval-session".to_owned());
            app.transcript.apply(&SessionWireEvent {
                event_type: "tool/call".into(),
                seq: 1,
                time: 1_760_000_000_000,
                data: serde_json::json!({
                    "callId": "call-1",
                    "name": "bash",
                    "arguments": "{\"command\":\"cargo test --release\"}"
                }),
                ignorable: None,
                source_event_seqs: None,
                surface_op: None,
            });
            let mut request = serde_json::Map::new();
            request.insert(
                "toolName".to_owned(),
                serde_json::Value::String("bash".to_owned()),
            );
            request.insert(
                "callId".to_owned(),
                serde_json::Value::String("call-1".to_owned()),
            );
            let invocation = dsh_client_core::remote_events::RemoteEventInvocation::for_test(
                "approval/request",
                request,
            );
            app.pending_approvals.push(
                dsh_client_core::approvals::PendingApproval::from_invocation(invocation)
                    .expect("approval request"),
            );
        });
        assert!(harness.bounds("approval-scope").is_some());
        assert!(harness.bounds("approval-allow").is_some());
        assert!(harness.bounds("approval-deny").is_some());
        assert!(harness.bounds("approval-delegate").is_none());
    }
}
