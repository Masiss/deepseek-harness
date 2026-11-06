//! Agent column: Session header, transcript, approvals, questions, and composer.

use gpui::{div, prelude::*, StatefulInteractiveElement, px, AnyElement, Context, Window};

use crate::explorer::{ExplorerRow, ExplorerRowKind};
use crate::layout;
use crate::transcript::{RowKind, TranscriptRow};
use dsh_client_core::chat::PromptMode;
use crate::shell::app::DesktopApp;
use crate::shell::theme::Theme;
use super::widgets;

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
    let attention_height = {
        let reserved = if !app.pending_questions.is_empty() || !app.pending_approvals.is_empty() {
            if !app.pending_questions.is_empty() {
                220.0
            } else if !app.pending_approvals.is_empty() {
                140.0
            } else {
                0.0
            }
        } else {
            0.0
        };
        theme.sp(reserved)
    };
    let composer = composer(app, theme, cx);
    column = if attention_height > px(0.) {
        column.child(div().flex().flex_col().w_full().h(attention_height).child(composer))
    } else {
        column.child(composer)
    };
    column.into_any_element()
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
    let mut pane = div()
        .id("transcript-scroll")
        .flex()
        .flex_col()
        .gap(theme.sp(6.))
        .flex_1()
        .min_h(px(0.))
        .w_full()
        .px(theme.sp(14.))
        .py(theme.sp(10.))
        .overflow_y_scroll()
        .track_scroll(&app.transcript_scroll);

    if app.page_loading && app.transcript.is_empty() {
        pane = pane.child(note(theme, "Loading session…"));
    } else if let Some(error) = app.page_error.clone() {
        pane = pane.child(
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
        pane = pane.child(note(theme, "This session has no events"));
    } else if app.page_has_more {
        let loading = app.page_loading;
        pane = pane.child(if loading {
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
    for row in app.transcript.rows() {
        pane = pane.child(transcript_row(
            theme,
            row,
            feedback.get(row.message_id.as_deref().unwrap_or_default()),
        ));
    }

    if app.assistant_streaming {
        pane = pane.child(streaming_draft(theme, &app.assistant_draft));
    }

    if app.scroll_transcript_to_bottom {
        app.transcript_scroll.scroll_to_bottom();
        app.scroll_transcript_to_bottom = false;
    }
    if app.assistant_streaming {
        app.transcript_scroll.scroll_to_bottom();
    }
    pane.into_any_element()
}

fn row_accent(theme: &Theme, kind: RowKind) -> gpui::Hsla {
    match kind {
        RowKind::User => theme.accent,
        RowKind::Assistant => theme.text,
        RowKind::Reasoning => theme.muted,
        RowKind::ToolResult => theme.muted,
        RowKind::ToolError => theme.error,
        RowKind::Approval => theme.warn,
        RowKind::Job => theme.accent,
        RowKind::Todo => theme.accent,
        RowKind::Deliverable => theme.accent,
        RowKind::Goal => theme.warn,
        RowKind::Notice => theme.muted,
    }
}

fn transcript_row(theme: &Theme, row: &TranscriptRow, rating: Option<&String>) -> AnyElement {
    let accent = row_accent(theme, row.kind);
    let is_user = row.kind == RowKind::User;
    let background = match row.kind {
        RowKind::User => theme.surface_raised,
        RowKind::ToolResult | RowKind::ToolError | RowKind::Job => theme.code_background,
        _ => theme.background,
    };
    let mut card = div()
        .flex()
        .flex_col()
        .gap(theme.sp(3.))
        .w_full()
        .p(theme.sp(10.))
        .rounded(theme.corner())
        .bg(background)
        .when(is_user, |element| {
            element.border_1().border_color(theme.border)
        })
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(theme.sp(6.))
                .child(
                    div()
                        .text_size(theme.fs(9.5))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(accent)
                        .child(row_kind_label(row.kind)),
                )
                .child(
                    div()
                        .flex_1()
                        .text_size(theme.fs(12.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .truncate()
                        .child(row.title.clone()),
                )
                .when_some(rating, |element, rating| {
                    element.child(
                        div()
                            .text_size(theme.fs(9.5))
                            .text_color(theme.muted)
                            .child(format!("Feedback: {rating}")),
                    )
                }),
        )
        .when(!row.text.is_empty(), |element| {
            element.child(
                div()
                    .w_full()
                    .text_size(theme.fs(12.5))
                    .when(
                        matches!(
                            row.kind,
                            RowKind::ToolResult | RowKind::ToolError | RowKind::Job
                        ),
                        |element| element.font(theme.mono_font()).text_size(theme.fs(11.)),
                    )
                    .child(row.text.clone()),
            )
        });
    if matches!(row.kind, RowKind::Reasoning | RowKind::Notice) {
        card = card.text_color(theme.muted);
    }
    if matches!(row.kind, RowKind::ToolError) {
        card = card.border_1().border_color(theme.error);
    }
    card.into_any_element()
}

fn row_kind_label(kind: RowKind) -> &'static str {
    match kind {
        RowKind::User => "YOU",
        RowKind::Assistant => "ASSISTANT",
        RowKind::Reasoning => "REASONING",
        RowKind::ToolResult => "TOOL",
        RowKind::ToolError => "TOOL ERROR",
        RowKind::Approval => "APPROVAL",
        RowKind::Job => "JOB",
        RowKind::Todo => "TASKS",
        RowKind::Deliverable => "DELIVERABLE",
        RowKind::Goal => "GOAL",
        RowKind::Notice => "NOTICE",
    }
}

fn streaming_draft(theme: &Theme, draft: &str) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap(theme.sp(3.))
        .w_full()
        .p(theme.sp(10.))
        .rounded(theme.corner())
        .bg(theme.surface)
        .child(
            div()
                .text_size(theme.fs(9.5))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(theme.accent)
                .child("ASSISTANT · STREAMING"),
        )
        .child(
            div()
                .w_full()
                .text_size(theme.fs(12.5))
                .child(if draft.is_empty() { "…".to_owned() } else { draft.to_owned() }),
        )
        .into_any_element()
}

fn attention_pane(
    app: &mut DesktopApp,
    theme: &Theme,
    cx: &mut Context<DesktopApp>,
) -> AnyElement {
    let mut pane = div().flex().flex_col().gap(theme.sp(6.)).px(theme.sp(14.));

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
                        .child(format!("{}{}", if multi { "☐ " } else { "" }, label))
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
        pane = pane.child(widgets::card(theme, body));
    }

    if let Some(pending) = app.pending_approvals.first().cloned() {
        let queued = app.pending_approvals.len();
        let mut card = div()
            .flex()
            .flex_col()
            .gap(theme.sp(5.))
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
                            .child(format!("Permission: {}", pending.request.tool_name)),
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
        if let Some(reason) = &pending.request.reason {
            card = card.child(
                div()
                    .text_size(theme.fs(11.))
                    .text_color(theme.muted)
                    .child(reason.clone()),
            );
        }
        card = card.child(
            div()
                .flex()
                .flex_row()
                .gap(theme.sp(6.))
                .child(
                    widgets::Button::new(theme, "Allow once")
                        .primary()
                        .build()
                        .id("approval-allow")
                        .on_click(cx.listener(|app, _, _, cx| {
                            app.decide_approval(true);
                            cx.notify();
                        })),
                )
                .child(
                    widgets::Button::new(theme, "Reject")
                        .danger()
                        .build()
                        .id("approval-reject")
                        .on_click(cx.listener(|app, _, _, cx| {
                            app.decide_approval(false);
                            cx.notify();
                        })),
                )
                .child(
                    widgets::Button::new(theme, "Delegate")
                        .build()
                        .id("approval-delegate")
                        .on_click(cx.listener(|app, _, _, cx| {
                            app.delegate_approval();
                            cx.notify();
                        })),
                ),
        );
        pane = pane.child(widgets::card(theme, card));
    }

    pane.into_any_element()
}

const QUESTION_KEYS: [&str; 4] = ["question-0", "question-1", "question-2", "question-3"];

fn composer(
    app: &mut DesktopApp,
    theme: &Theme,
    cx: &mut Context<DesktopApp>,
) -> AnyElement {
    let input = app.text_field("composer", "Message (Ctrl+Enter to send)", true, cx);
    if let Some(text) = app.pending_composer_text.take() {
        input.update(cx, |input, cx| input.set_text(text, cx));
    }
    let enabled = !app.chat_loading && app.transport.is_some();
    let can_send = enabled
        && !app.chat_text.trim().is_empty()
        && app.attachment_receiver.is_none();
    let attachment_name = app.attachment_name.clone();
    let uploading = app.attachment_receiver.is_some();
    let has_attachment = app.attachment.is_some();
    let model_label = app.model_label();
    let menu_open = app.model_menu_open;
    let busy = app.chat_loading;
    let transport = app.transport.is_some();

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
                .flex_row()
                .items_center()
                .gap(theme.sp(6.))
                .child(
                    widgets::Button::new(theme, "＋ Attach")
                        .enabled(!uploading && !busy)
                        .build()
                        .id("composer-attach")
                        .on_click(cx.listener(|app, _, _, cx| {
                            app.pick_attachment();
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
                        widgets::Button::new(theme, "×")
                            .build()
                            .id("composer-clear-attachment")
                            .on_click(cx.listener(|app, _, _, cx| {
                                app.attachment = None;
                                app.attachment_name.clear();
                                cx.notify();
                            })),
                    )
                })
                .child(
                    widgets::Button::new(theme, model_label)
                        .enabled(transport)
                        .build()
                        .id("composer-model")
                        .on_click(cx.listener(move |app, _, _, cx| {
                            app.model_menu_open = !menu_open;
                            cx.notify();
                        })),
                )
                .child(
                    widgets::Button::new(
                        theme,
                        if app.prompt_mode == PromptMode::Queue {
                            "Queue"
                        } else {
                            "Queue"
                        },
                    )
                    .enabled(transport && app.prompt_mode != PromptMode::Queue)
                    .build()
                    .id("composer-mode-queue")
                    .on_click(cx.listener(|app, _, _, cx| {
                        app.prompt_mode = PromptMode::Queue;
                        cx.notify();
                    })),
                )
                .child(
                    widgets::Button::new(theme, "Steer")
                        .enabled(transport && app.prompt_mode != PromptMode::Steer)
                        .build()
                        .id("composer-mode-steer")
                        .on_click(cx.listener(|app, _, _, cx| {
                            app.prompt_mode = PromptMode::Steer;
                            cx.notify();
                        })),
                )
                .child(
                    widgets::Button::new(theme, "History")
                        .enabled(!app.prompt_history.is_empty())
                        .build()
                        .id("composer-history")
                        .on_click(cx.listener(|app, _, _, cx| {
                            app.prompt_history_open = !app.prompt_history_open;
                            cx.notify();
                        })),
                )
                .child(
                    widgets::Button::new(theme, "Next")
                        .enabled(!app.prompt_history.is_empty())
                        .build()
                        .id("composer-history-next")
                        .on_click(cx.listener(|app, _, _, cx| {
                            app.recall_prompt_history(-1);
                            if let Some(text) = app.pending_composer_text.clone() {
                                app.set_input("composer", &text, cx);
                            }
                            cx.notify();
                        })),
                )
                .child(
                    widgets::Button::new(theme, "＠")
                        .enabled(transport)
                        .build()
                        .id("composer-file-picker")
                        .on_click(cx.listener(|app, _, _, cx| {
                            app.file_picker_open = !app.file_picker_open;
                            if app.file_picker_open {
                                app.file_picker_query.clear();
                                app.set_input("file-picker-query", "", cx);
                            }
                            cx.notify();
                        })),
                )
                .child(div().flex_1())
                .when(busy, |element| {
                    element.child(
                        widgets::Button::new(theme, "Stop")
                            .danger()
                            .build()
                            .id("composer-stop")
                            .on_click(cx.listener(|app, _, _, cx| {
                                app.cancel_turn();
                                cx.notify();
                            })),
                    )
                })
                .when(busy, |element| {
                    element.child(
                        widgets::Button::new(theme, "Queue")
                            .build()
                            .id("composer-queue")
                            .on_click(cx.listener(|app, _, _, cx| {
                                app.queue_prompt();
                                app.set_input("composer", "", cx);
                                cx.notify();
                            })),
                    )
                })
                .child(
                    widgets::Button::new(theme, "Send")
                        .primary()
                        .enabled(can_send)
                        .build()
                        .id("composer-send")
                        .on_click(cx.listener(|app, _, _, cx| {
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
        .px(theme.sp(14.))
        .pb(theme.sp(8.))
        .child(div().w_full().max_w(theme.sp(800.)).child(body));
    if app.prompt_history_open {
        let prompts: Vec<String> = app.prompt_history.clone();
        let mut list = div()
            .flex()
            .flex_col()
            .w_full()
            .max_w(theme.sp(800.))
            .max_h(theme.sp(140.))
            .id("prompt-history-scroll").overflow_y_scroll()
            .bg(theme.surface_raised)
            .rounded(theme.corner())
            .border_1()
            .border_color(theme.border)
            .px(theme.sp(6.))
            .py(theme.sp(4.))
            .gap(px(1.));
        if prompts.is_empty() {
            list = list.child(
                div()
                    .text_size(theme.fs(10.))
                    .text_color(theme.muted)
                    .child("No prompt history yet."),
            );
        }
        for (index, prompt) in prompts.into_iter().rev().enumerate() {
            let text = prompt.clone();
            list = list.child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .w_full()
                    .px(theme.sp(6.))
                    .py(px(2.))
                    .rounded(theme.corner())
                    .cursor_pointer()
                    .hover(|style| style.bg(theme.surface))
                    .id(gpui::SharedString::from(format!("prompt-history-{index}")))
                    .child(
                        div()
                            .text_size(theme.fs(10.5))
                            .truncate()
                            .child(prompt),
                    )
                    .on_click(cx.listener(move |app, _, _, cx| {
                        app.chat_text = text.clone();
                        app.pending_composer_text = Some(text.clone());
                        app.prompt_history_open = false;
                        app.set_input("composer", &text, cx);
                        cx.notify();
                    })),
            );
        }
        column = column.child(list);
    }
    if app.file_picker_open {
        let query_input = app.text_field("file-picker-query", "Filter loaded files by name or path", false, cx);
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
            .max_w(theme.sp(800.))
            .max_h(theme.sp(140.))
            .id("file-picker-scroll").id("prompt-history-scroll").overflow_y_scroll()
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
                    .max_w(theme.sp(800.))
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
    if menu_open {
        column = column.child(model_menu(app, theme, cx));
    }
    column.into_any_element()
}

fn model_menu(
    app: &mut DesktopApp,
    theme: &Theme,
    cx: &mut Context<DesktopApp>,
) -> AnyElement {
    let mut list = div()
        .flex()
        .flex_col()
        .gap(px(1.))
        .w_full()
        .max_w(theme.sp(800.))
        .mt(theme.sp(4.))
        .p(theme.sp(6.))
        .rounded(theme.corner())
        .border_1()
        .border_color(theme.border)
        .bg(theme.surface_raised)
        .shadow_lg()
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_size(theme.fs(10.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child("Model"),
                )
                .child(
                    widgets::Button::new(theme, "↻")
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
                    widgets::Button::new(theme, "＋ New Session")
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
                    widgets::Button::new(theme, "＋ New Session")
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
                    widgets::Button::new(theme, "＋ New Session")
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
