//! Question card state and rendering for forwarded `ask_user_question` requests.
//!
//! Option labels are the answer encoding; free text rides the `custom` field of
//! the same answer item. A skipped question sends an empty item so the model
//! receives a definite "no answer" instead of a hanging call.

use dsh_client_core::user_questions::{
    AskUserQuestionAnswer, AskUserQuestionAnswerItem, PendingQuestion, UserQuestion,
};

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

    /// The questions this card asks, in order.
    #[allow(dead_code)]
    pub(crate) fn questions(&self) -> &[UserQuestion] {
        &self.questions
    }

    /// Selected option labels for the question at `index`.
    #[allow(dead_code)]
    pub(crate) fn chosen(&self, index: usize) -> &[String] {
        self.selected.get(index).map_or(&[], Vec::as_slice)
    }

    /// The typed free-form answer for the question at `index`.
    #[allow(dead_code)]
    pub(crate) fn custom(&self, index: usize) -> &str {
        self.custom.get(index).map_or("", String::as_str)
    }

    /// Selects an option, replacing the choice unless the question is multi-select.
    #[allow(dead_code)]
    pub(crate) fn choose(&mut self, index: usize, label: &str) {
        let multi = self
            .questions
            .get(index)
            .is_some_and(|question| question.multi_select);
        let Some(selected) = self.selected.get_mut(index) else {
            return;
        };
        if multi {
            toggle(selected, label);
        } else {
            *selected = vec![label.to_owned()];
        }
    }

    /// Records the typed answer for the question at `index`.
    #[allow(dead_code)]
    pub(crate) fn set_custom(&mut self, index: usize, text: &str) {
        if let Some(custom) = self.custom.get_mut(index) {
            *custom = text.to_owned();
        }
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
                .map(
                    |label| dsh_client_core::user_questions::UserQuestionOption {
                        label: (*label).into(),
                        description: None,
                    },
                )
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
}

#[cfg(test)]
mod selection_tests {
    use super::*;

    fn card(multi: bool) -> QuestionCard {
        QuestionCard::for_test(vec![UserQuestion {
            id: "pick".into(),
            question: "Pick".into(),
            detail: None,
            header: None,
            options: ["Alpha", "Beta"]
                .iter()
                .map(|label| dsh_client_core::user_questions::UserQuestionOption {
                    label: (*label).into(),
                    description: None,
                })
                .collect(),
            multi_select: multi,
        }])
    }

    #[test]
    fn single_select_replaces_the_previous_option() {
        let mut card = card(false);
        card.choose(0, "Alpha");
        card.choose(0, "Beta");
        assert_eq!(card.chosen(0), ["Beta"]);
    }

    #[test]
    fn multi_select_toggles_options_in_click_order() {
        let mut card = card(true);
        card.choose(0, "Alpha");
        card.choose(0, "Beta");
        card.choose(0, "Alpha");
        assert_eq!(card.chosen(0), ["Beta"]);
    }
}
