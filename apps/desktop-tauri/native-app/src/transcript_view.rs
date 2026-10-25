//! Rendering for the bounded native chat transcript.

use crate::transcript::{RowKind, Transcript};
use eframe::egui;

/// Renders the projected transcript with bounded height.
pub(super) fn render_transcript(
    ui: &mut egui::Ui,
    transcript: &Transcript,
    draft: &str,
    feedback: &std::collections::BTreeMap<String, String>,
    loading: bool,
    streaming: bool,
    feedback_actions: &mut Vec<(String, String)>,
) {
    if loading || streaming {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label("Waiting for assistant…");
        });
    }
    let height = (ui.available_height() - 155.0).max(80.0);
    egui::ScrollArea::vertical()
        .id_salt("chat-transcript")
        .auto_shrink([false, false])
        .stick_to_bottom(true)
        .max_height(height)
        .show(ui, |ui| {
            if transcript.is_empty() && !loading {
                ui.add_space(18.0);
                ui.label(
                    egui::RichText::new("No messages in this Session yet.")
                        .size(12.0)
                        .weak(),
                );
            }
            for row in transcript.rows() {
                ui.add_space(14.0);
                let assistant = row.kind == RowKind::Assistant;
                let avatar_fill = if assistant {
                    ui.visuals().selection.bg_fill
                } else {
                    ui.visuals().widgets.inactive.bg_fill
                };
                let avatar_text = if assistant {
                    ui.visuals().hyperlink_color
                } else {
                    ui.visuals().weak_text_color()
                };
                ui.horizontal(|ui| {
                    egui::Frame::new()
                        .fill(avatar_fill)
                        .corner_radius(6.0)
                        .inner_margin(egui::Margin::symmetric(6, 4))
                        .show(ui, |ui| {
                            ui.label(
                                egui::RichText::new(if row.kind == RowKind::User {
                                    "Y"
                                } else if assistant {
                                    "D"
                                } else {
                                    "·"
                                })
                                .size(10.0)
                                .strong()
                                .color(avatar_text),
                            );
                        });
                    ui.label(egui::RichText::new(&row.title).size(12.0).strong());
                });
                if !row.text.is_empty() {
                    let text = egui::RichText::new(&row.text).size(13.0);
                    let text = match row.kind {
                        RowKind::ToolResult | RowKind::Job | RowKind::Todo => {
                            text.monospace().size(11.0)
                        }
                        RowKind::Reasoning => text.italics().color(ui.visuals().weak_text_color()),
                        _ => text,
                    };
                    if matches!(row.kind, RowKind::ToolResult | RowKind::Job) {
                        egui::Frame::new()
                            .fill(ui.visuals().extreme_bg_color)
                            .stroke(egui::Stroke::new(
                                1.0,
                                ui.visuals().widgets.noninteractive.bg_stroke.color,
                            ))
                            .corner_radius(6.0)
                            .inner_margin(egui::Margin::symmetric(11, 8))
                            .show(ui, |ui| {
                                ui.add(egui::Label::new(text).wrap().selectable(true));
                            });
                    } else {
                        ui.horizontal(|ui| {
                            ui.add_space(34.0);
                            ui.add(egui::Label::new(text).wrap().selectable(true));
                        });
                    }
                }
                if row.kind == RowKind::Assistant {
                    if let Some(message_id) = &row.message_id {
                        ui.horizontal(|ui| {
                            ui.add_space(34.0);
                            if let Some(rating) = feedback.get(message_id) {
                                ui.label(
                                    egui::RichText::new(format!("Feedback: {rating}"))
                                        .size(10.0)
                                        .weak(),
                                );
                            }
                            if ui.small_button("👍").clicked() {
                                feedback_actions.push((message_id.clone(), "positive".into()));
                            }
                            if ui.small_button("👎").clicked() {
                                feedback_actions.push((message_id.clone(), "negative".into()));
                            }
                        });
                    }
                }
            }
            if !draft.trim().is_empty() {
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("D")
                            .size(11.0)
                            .strong()
                            .color(ui.visuals().hyperlink_color),
                    );
                    ui.label(
                        egui::RichText::new("Assistant · streaming")
                            .size(12.0)
                            .strong(),
                    );
                });
                ui.horizontal(|ui| {
                    ui.add_space(34.0);
                    ui.add(
                        egui::Label::new(egui::RichText::new(draft).size(13.0))
                            .wrap()
                            .selectable(true),
                    );
                });
            }
        });
}
