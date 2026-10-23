//! Question card state and rendering for forwarded `ask_user_question` requests.
//!
//! Option labels are the answer encoding; free text rides the `custom` field of
//! the same answer item. A skipped question sends an empty item so the model
//! receives a definite "no answer" instead of a hanging call.

use dsh_client_core::user_questions::{
    AskUserQuestionAnswer, AskUserQuestionAnswerItem, PendingQuestion, UserQuestion,
};
use eframe::egui;

/// What the user did with a card.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuestionAction {
    /// Submit the current selections and typed answers.
    Answer,
    /// Submit every question as skipped.
    Skip,
}

/// One live question request and the user's draft answers.
pub struct QuestionCard {
    pending: Option<PendingQuestion>,
    questions: Vec<UserQuestion>,
    selected: Vec<Vec<String>>,
    custom: Vec<String>,
}

impl QuestionCard {
    /// Wraps a live waterfall request.
    pub fn new(pending: PendingQuestion) -> Self {
        let questions = pending.request.questions.clone();
        let selected = questions.iter().map(|_| Vec::new()).collect();
        let custom = questions.iter().map(|_| String::new()).collect();
        Self {
            pending: Some(pending),
            questions,
            selected,
            custom,
        }
    }

    #[cfg(test)]
    fn for_test(questions: Vec<UserQuestion>) -> Self {
        let selected = questions.iter().map(|_| Vec::new()).collect();
        let custom = questions.iter().map(|_| String::new()).collect();
        Self {
            pending: None,
            questions,
            selected,
            custom,
        }
    }

    /// The live request, absent only for previews.
    pub fn pending(&self) -> Option<&PendingQuestion> {
        self.pending.as_ref()
    }

    /// Builds the batch from the current selections.
    pub fn answer(&self) -> AskUserQuestionAnswer {
        AskUserQuestionAnswer {
            answers: self
                .questions
                .iter()
                .enumerate()
                .map(|(index, question)| AskUserQuestionAnswerItem {
                    id: question.id.clone(),
                    selected: self.selected[index].clone(),
                    custom: Some(self.custom[index].trim().to_owned())
                        .filter(|text| !text.is_empty()),
                })
                .collect(),
        }
    }

    /// Builds a batch that marks every question as skipped.
    pub fn skip(&self) -> AskUserQuestionAnswer {
        AskUserQuestionAnswer {
            answers: self
                .questions
                .iter()
                .map(|question| AskUserQuestionAnswerItem {
                    id: question.id.clone(),
                    selected: Vec::new(),
                    custom: None,
                })
                .collect(),
        }
    }

    /// Renders the card and reports a submitted action.
    pub fn render(&mut self, ui: &mut egui::Ui) -> Option<QuestionAction> {
        let mut action = None;
        ui.separator();
        for (index, question) in self.questions.iter().enumerate() {
            if let Some(header) = &question.header {
                ui.heading(header);
            }
            ui.add(egui::Label::new(egui::RichText::new(&question.question).strong()).wrap());
            if let Some(detail) = &question.detail {
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(detail).color(ui.visuals().weak_text_color()),
                    )
                    .wrap(),
                );
            }
            if !question.options.is_empty() {
                ui.horizontal_wrapped(|ui| {
                    for option in &question.options {
                        let chosen = self.selected[index].contains(&option.label);
                        if question.multi_select {
                            let mut checked = chosen;
                            if ui.checkbox(&mut checked, &option.label).changed() {
                                toggle(&mut self.selected[index], &option.label);
                            }
                        } else if ui
                            .selectable_label(chosen, &option.label)
                            .on_hover_text(option.description.as_deref().unwrap_or_default())
                            .clicked()
                        {
                            self.selected[index] = vec![option.label.clone()];
                        }
                    }
                });
            }
            ui.add(
                egui::TextEdit::singleline(&mut self.custom[index])
                    .hint_text("Or type an answer")
                    .desired_width(f32::INFINITY),
            );
        }
        ui.horizontal(|ui| {
            if ui.button("Answer").clicked() {
                action = Some(QuestionAction::Answer);
            }
            if ui.button("Skip").clicked() {
                action = Some(QuestionAction::Skip);
            }
        });
        action
    }
}

fn toggle(selected: &mut Vec<String>, label: &str) {
    if let Some(index) = selected.iter().position(|item| item == label) {
        selected.remove(index);
    } else {
        selected.push(label.to_owned());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn question(id: &str, options: &[&str], multi: bool) -> UserQuestion {
        UserQuestion {
            id: id.into(),
            question: format!("Question {id}?"),
            detail: Some("detail text".into()),
            header: Some("Header".into()),
            options: options
                .iter()
                .map(|label| dsh_client_core::user_questions::UserQuestionOption {
                    label: (*label).into(),
                    description: None,
                })
                .collect(),
            multi_select: multi,
        }
    }

    #[test]
    fn builds_answers_from_selections_and_custom_text() {
        let mut card = QuestionCard::for_test(vec![question("one", &["Yes", "No"], false)]);
        card.selected[0] = vec!["Yes".into()];
        card.custom[0] = "  note  ".into();
        let answer = card.answer();
        assert_eq!(answer.answers[0].id, "one");
        assert_eq!(answer.answers[0].selected, ["Yes"]);
        assert_eq!(answer.answers[0].custom.as_deref(), Some("note"));

        let skipped = card.skip();
        assert!(skipped.answers[0].selected.is_empty());
        assert_eq!(skipped.answers[0].custom, None);
    }

    #[test]
    fn keeps_multiple_selections_in_click_order() {
        let mut selected = Vec::new();
        toggle(&mut selected, "A");
        toggle(&mut selected, "B");
        toggle(&mut selected, "A");
        assert_eq!(selected, ["B"]);
    }

    #[test]
    fn renders_the_question_options_and_controls() {
        let ctx = egui::Context::default();
        ctx.begin_pass(egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(640.0, 480.0),
            )),
            ..Default::default()
        });
        let mut ui = egui::Ui::new(
            ctx.clone(),
            egui::Id::new("question-card"),
            egui::UiBuilder::new().max_rect(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(640.0, 480.0),
            )),
        );
        let mut card = QuestionCard::for_test(vec![question("one", &["Yes", "No"], true)]);
        assert!(card.render(&mut ui).is_none());
        let mut output = ctx.end_pass();
        output.textures_delta.clear();
        let mut texts = Vec::new();
        for clipped in output.shapes {
            collect_text(&clipped.shape, &mut texts);
        }
        for expected in ["Header", "Question one?", "detail text", "Yes", "No", "Answer", "Skip"] {
            assert!(texts.iter().any(|text| text == expected), "{texts:?}");
        }
    }

    fn collect_text(shape: &egui::Shape, texts: &mut Vec<String>) {
        match shape {
            egui::Shape::Text(text) => texts.push(text.galley.job.text.clone()),
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    collect_text(shape, texts);
                }
            }
            _ => {}
        }
    }
}
