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
    let height = (ui.available_height() - 140.0).max(80.0);
    egui::ScrollArea::vertical()
        .id_salt("chat-transcript")
        .auto_shrink([false, false])
        .stick_to_bottom(true)
        .max_height(height)
        .show(ui, |ui| {
            if transcript.is_empty() && !loading {
                ui.label("No messages in this window yet.");
            }
            for row in transcript.rows() {
                let color = match row.kind {
                    RowKind::User => egui::Color32::LIGHT_BLUE,
                    RowKind::Assistant => ui.visuals().strong_text_color(),
                    RowKind::Job => egui::Color32::LIGHT_GREEN,
                    RowKind::ToolResult => {
                        if row.title == "Tool error" {
                            egui::Color32::LIGHT_RED
                        } else {
                            ui.visuals().weak_text_color()
                        }
                    }
                    RowKind::Approval => egui::Color32::YELLOW,
                    RowKind::Reasoning => ui.visuals().weak_text_color(),
                    RowKind::Todo => egui::Color32::LIGHT_GREEN,
                    RowKind::Deliverable => egui::Color32::LIGHT_BLUE,
                    RowKind::Goal => egui::Color32::YELLOW,
                    RowKind::Notice => egui::Color32::LIGHT_GRAY,
                };
                ui.add(
                    egui::Label::new(egui::RichText::new(&row.title).strong().color(color))
                        .wrap()
                        .selectable(true),
                );
                if !row.text.is_empty() {
                    let text = egui::RichText::new(&row.text);
                    let text = match row.kind {
                        RowKind::ToolResult | RowKind::Job | RowKind::Todo => text.monospace(),
                        RowKind::Reasoning => text.italics(),
                        _ => text,
                    };
                    ui.add(egui::Label::new(text).wrap().selectable(true));
                }
                if row.kind == RowKind::Assistant {
                    if let Some(message_id) = &row.message_id {
                        ui.horizontal(|ui| {
                            if let Some(rating) = feedback.get(message_id) {
                                ui.label(format!("Feedback: {rating}"));
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
                ui.separator();
            }
            if !draft.trim().is_empty() {
                ui.add(
                    egui::Label::new(
                        egui::RichText::new("Assistant (streaming)")
                            .strong()
                            .color(ui.visuals().strong_text_color()),
                    )
                    .wrap(),
                );
                ui.add(egui::Label::new(draft).wrap().selectable(true));
                ui.separator();
            }
        });
}
