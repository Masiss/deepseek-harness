//! Display-only text previews; file access and document selection belong to the caller.

use eframe::egui;

/// Caller-supplied text and identity, kept separate from local filesystem paths.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextDocument {
    /// Display name supplied by the Host or caller.
    pub name: String,
    /// Host-side path for display only; never resolved on the desktop filesystem.
    pub host_path: String,
    /// Decoded text, rendered literally without markup or link interpretation.
    pub text: String,
}

/// Reusable viewer with no owned or persisted document content.
#[derive(Debug, Default)]
pub struct DocumentPreview;

impl DocumentPreview {
    /// Renders selectable text without fetching, editing, or persisting the document.
    ///
    /// Existing content stays visible during loading or failure and is marked as
    /// previously loaded. The caller must clear `document` when changing targets
    /// if the old content should not remain visible. Errors must be display-safe.
    /// Use distinct parent `push_id` scopes for multiple previews in one UI.
    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        document: Option<&TextDocument>,
        loading: bool,
        error: Option<&str>,
    ) {
        ui.push_id("document_preview", |ui| {
            ui.heading("Document preview");
            if loading {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Loading document…");
                });
            }
            if let Some(error) = error {
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(format!("Document read failed: {error}"))
                            .color(ui.visuals().error_fg_color),
                    )
                    .wrap()
                    .selectable(true),
                );
            }
            let Some(document) = document else {
                if !loading && error.is_none() {
                    ui.label("No document selected.");
                }
                return;
            };
            if loading || error.is_some() {
                ui.label("Showing previously loaded document; content may be out of date.");
            }

            // Host identity is literal text, not a local path or an actionable URL.
            ui.add(
                egui::Label::new(egui::RichText::new(format!("File: {}", document.name)).strong())
                    .wrap()
                    .selectable(true),
            );
            ui.add(
                egui::Label::new(format!("Host path: {}", document.host_path))
                    .wrap()
                    .selectable(true),
            );
            ui.separator();

            egui::ScrollArea::vertical()
                .id_salt(("document_text", &document.host_path, &document.name))
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    if document.text.is_empty() {
                        ui.label("This document is empty.");
                    } else {
                        ui.add(
                            egui::Label::new(egui::RichText::new(&document.text).monospace())
                                .wrap()
                                .selectable(true),
                        );
                    }
                });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(
        preview: &mut DocumentPreview,
        document: Option<&TextDocument>,
        loading: bool,
        error: Option<&str>,
        width: f32,
    ) -> Vec<String> {
        let ctx = egui::Context::default();
        ctx.begin_pass(egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(width, 600.0),
            )),
            ..Default::default()
        });
        let mut ui = egui::Ui::new(
            ctx.clone(),
            egui::Id::new("test"),
            egui::UiBuilder::new().max_rect(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(width, 600.0),
            )),
        );
        preview.ui(&mut ui, document, loading, error);
        assert!(ui.min_rect().is_finite());
        assert!(ui.min_rect().width() <= width);
        let mut output = ctx.end_pass();
        output.textures_delta.clear();
        let mut texts = Vec::new();
        for shape in output.shapes {
            collect_text(&shape.shape, &mut texts);
        }
        texts
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

    #[test]
    fn renders_no_selection_loading_and_failure_states() {
        let mut preview = DocumentPreview;
        for (loading, error, expected) in [
            (false, None, "No document selected."),
            (true, None, "Loading document…"),
            (
                false,
                Some("Unavailable"),
                "Document read failed: Unavailable",
            ),
        ] {
            let texts = render(&mut preview, None, loading, error, 320.0);
            assert!(texts.iter().any(|text| text == expected));
            assert!(!texts.iter().any(|text| text == "This document is empty."));
        }
    }

    #[test]
    fn preserves_identity_and_content_during_refresh_and_failure() {
        let document = TextDocument {
            name: "notes.md".into(),
            host_path: "/host/project/notes.md".into(),
            text: "# Literal text <script>not executed</script>".into(),
        };
        let original = document.clone();
        let mut preview = DocumentPreview;
        for (loading, error) in [(false, None), (true, None), (false, Some("Refresh failed"))] {
            let texts = render(&mut preview, Some(&document), loading, error, 320.0);
            for expected in [
                "File: notes.md",
                "Host path: /host/project/notes.md",
                &document.text,
            ] {
                assert!(texts.iter().any(|text| text == expected));
            }
            assert_eq!(
                texts.iter().any(|text| text
                    == "Showing previously loaded document; content may be out of date."),
                loading || error.is_some()
            );
        }
        assert_eq!(document, original);
    }

    #[test]
    fn distinguishes_empty_content_from_no_selection() {
        let document = TextDocument {
            name: "empty.txt".into(),
            host_path: "remote:/empty.txt".into(),
            text: String::new(),
        };
        let texts = render(&mut DocumentPreview, Some(&document), false, None, 320.0);
        assert!(texts.iter().any(|text| text == "This document is empty."));
        assert!(!texts.iter().any(|text| text == "No document selected."));
    }

    #[test]
    fn wraps_long_unicode_content_and_identity_in_narrow_layouts() {
        let document = TextDocument {
            name: "文档🦀e\u{301}".repeat(40),
            host_path: format!("/host/{}", "路径🦀".repeat(100)),
            text: format!("{}\n\tالعربية עברית e\u{301} 👩‍💻\0", "界🦀x".repeat(4_000)),
        };
        let original = document.clone();
        for width in [160.0, 640.0] {
            render(&mut DocumentPreview, Some(&document), false, None, width);
        }
        assert_eq!(document, original);
    }
}
