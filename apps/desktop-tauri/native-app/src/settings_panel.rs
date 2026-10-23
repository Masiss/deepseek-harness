//! Transport-independent viewer and optimistic editor for Host-redacted settings.

use dsh_client_core::settings::{
    SettingsDescribeValue, SettingsMutateRequest, SettingsNamespaceView, SettingsPathOp,
};
use eframe::egui;

/// An explicit user action; the caller performs HTTP outside rendering.
pub enum SettingsAction {
    Refresh,
    Mutate(SettingsMutateRequest),
}

#[derive(Debug)]
struct SettingsEditor {
    original: SettingsNamespaceView,
    text: String,
    blocked: bool,
}

impl SettingsEditor {
    fn new(namespace: &SettingsNamespaceView) -> Result<Self, String> {
        let user = namespace
            .user
            .clone()
            .unwrap_or_else(|| serde_json::json!({}));
        if !user.is_object() {
            return Err("The user override is not a JSON object; editing is unavailable.".into());
        }
        Ok(Self {
            original: namespace.clone(),
            text: json_text(&user, true).map_err(|_| "Cannot format user overrides.")?,
            blocked: false,
        })
    }

    fn request(&self) -> Result<SettingsMutateRequest, String> {
        if self.blocked {
            return Err("Refresh settings and discard this draft before editing again.".into());
        }
        let value: serde_json::Value = serde_json::from_str(&self.text).map_err(|error| {
            format!(
                "Invalid JSON at line {}, column {}.",
                error.line(),
                error.column()
            )
        })?;
        let edited = value
            .as_object()
            .ok_or("Enter a JSON object, not an array or scalar.")?;
        let original = self
            .original
            .user
            .as_ref()
            .and_then(serde_json::Value::as_object);
        let mut ops = Vec::new();
        diff_section(
            original,
            edited,
            &self.original.secrets,
            &mut Vec::new(),
            &mut ops,
        )?;
        ops.sort_by(|left, right| path_of(left).cmp(path_of(right)));
        if ops.is_empty() {
            return Err("No user override changes to save.".into());
        }
        Ok(SettingsMutateRequest {
            ns: self.original.ns.clone(),
            ops,
            expected_revision: Some(self.original.revision),
        })
    }
}

fn path_of(op: &SettingsPathOp) -> &[String] {
    match op {
        SettingsPathOp::Set { path, .. } | SettingsPathOp::Unset { path } => path,
    }
}

/// Emits one path operation per changed leaf, preserving redacted secret slots.
fn diff_section(
    original: Option<&serde_json::Map<String, serde_json::Value>>,
    edited: &serde_json::Map<String, serde_json::Value>,
    secrets: &[dsh_client_core::settings::SettingsSecretView],
    prefix: &mut Vec<String>,
    ops: &mut Vec<SettingsPathOp>,
) -> Result<(), String> {
    let secret_below = |path: &[String]| {
        secrets
            .iter()
            .any(|secret| secret.path.len() > path.len() && secret.path.starts_with(path))
    };
    for (key, value) in edited {
        if original.and_then(|original| original.get(key)) == Some(value) {
            continue;
        }
        prefix.push(key.clone());
        let is_secret = secrets.iter().any(|secret| secret.path == *prefix);
        if is_secret {
            return Err("Fields containing secret slots cannot be changed in this editor.".into());
        }
        match (original.and_then(|original| original.get(key)), value.as_object()) {
            (Some(previous), Some(nested)) if previous.is_object() => {
                diff_section(
                    previous.as_object(),
                    nested,
                    secrets,
                    prefix,
                    ops,
                )?;
            }
            (_, _) if secret_below(prefix) => {
                return Err(
                    "Replacing a section that contains secret slots is unavailable; edit individual fields."
                        .into(),
                );
            }
            _ => ops.push(SettingsPathOp::Set {
                path: prefix.clone(),
                value: value.clone(),
            }),
        }
        prefix.pop();
    }
    if let Some(original) = original {
        for key in original.keys() {
            if edited.contains_key(key) {
                continue;
            }
            prefix.push(key.clone());
            ops.push(SettingsPathOp::Unset {
                path: prefix.clone(),
            });
            prefix.pop();
        }
    }
    Ok(())
}

/// Local selection, formatting preference, and a draft captured at one revision.
#[derive(Debug)]
pub struct SettingsPanel {
    selected_namespace: Option<String>,
    pretty_json: bool,
    editor: Option<SettingsEditor>,
    edit_message: Option<String>,
    requires_refresh: bool,
}

impl Default for SettingsPanel {
    fn default() -> Self {
        Self {
            selected_namespace: None,
            pretty_json: true,
            editor: None,
            edit_message: None,
            requires_refresh: false,
        }
    }
}

impl SettingsPanel {
    /// Renders caller-supplied settings and returns explicit read/write actions.
    ///
    /// `description` must contain only Host-redacted data. `loading` and `error`
    /// describe the caller's read operation; an existing description remains
    /// visible, explicitly marked as previously loaded, during refresh or failure.
    /// `error` must be a display-safe message without credentials or raw responses.
    /// Callers rendering multiple viewers in one parent should use distinct `push_id` scopes.
    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        description: Option<&SettingsDescribeValue>,
        loading: bool,
        error: Option<&str>,
        saving: bool,
    ) -> Option<SettingsAction> {
        let mut action = None;
        ui.push_id("settings_panel", |ui| {
            ui.heading("Settings");
            if ui.add_enabled(!loading && !saving, egui::Button::new("Refresh settings")).clicked() {
                action = Some(SettingsAction::Refresh);
            }
            if saving {
                ui.label("Saving user overrides…");
            }
            if self.requires_refresh {
                ui.label("Refresh settings successfully before starting another edit.");
            }
            if let Some(message) = &self.edit_message {
                ui.label(message);
            }
            if loading {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Loading settings…");
                });
            }
            if let Some(error) = error {
                ui.colored_label(
                    ui.visuals().error_fg_color,
                    format!("Settings read failed: {error}"),
                );
            }
            let Some(description) = description else {
                if !loading && error.is_none() {
                    ui.label("No settings have been loaded.");
                }
                return;
            };
            if loading || error.is_some() {
                ui.label("Showing previously loaded settings; values may be out of date.");
            }
            ui.label(if description.writable {
                "Provider: writable."
            } else {
                "Provider: read-only."
            });
            ui.label(if description.has_document {
                "Settings document available."
            } else {
                "No settings document available."
            });

            self.reconcile_selection(&description.namespaces);
            if description.namespaces.is_empty() {
                ui.label("No settings namespaces are available.");
                return;
            }

            egui::ComboBox::from_id_salt("namespace")
                .selected_text(self.selected_namespace.as_deref().unwrap_or_default())
                .show_ui(ui, |ui| {
                    for namespace in &description.namespaces {
                        ui.selectable_value(
                            &mut self.selected_namespace,
                            Some(namespace.ns.clone()),
                            &namespace.ns,
                        );
                    }
                });
            ui.checkbox(&mut self.pretty_json, "Pretty-print JSON");
            ui.separator();

            if let Some(namespace) = description
                .namespaces
                .iter()
                .find(|namespace| Some(&namespace.ns) == self.selected_namespace.as_ref())
            {
                ui.push_id(&namespace.ns, |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("namespace_details")
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            self.namespace_ui(ui, namespace);
                            if description.writable {
                                ui.separator();
                                if self.editor.is_none() {
                                    if ui.add_enabled(!loading && error.is_none() && !saving && !self.requires_refresh, egui::Button::new("Edit user overrides")).clicked() {
                                        match SettingsEditor::new(namespace) {
                                            Ok(editor) => { self.editor = Some(editor); self.edit_message = None; }
                                            Err(message) => self.edit_message = Some(message),
                                        }
                                    }
                                } else if self.editor.as_ref().is_some_and(|editor| editor.original.ns == namespace.ns) {
                                    ui.label("User overrides only. Changed fields become path operations, removed fields are unset, and fields containing secret slots cannot be changed.");
                                    let Some(editor) = self.editor.as_mut() else {
                                        return;
                                    };
                                    ui.label(format!("Draft revision: {}", editor.original.revision));
                                    ui.add_enabled(!saving, egui::TextEdit::multiline(&mut editor.text).code_editor().desired_rows(12).desired_width(f32::INFINITY));
                                    let mut next_message = None;
                                    let mut discard = false;
                                    ui.horizontal(|ui| {
                                        if ui.add_enabled(!loading && error.is_none() && !saving && !editor.blocked, egui::Button::new("Save settings")).clicked() {
                                            match editor.request() {
                                                Ok(request) => action = Some(SettingsAction::Mutate(request)),
                                                Err(message) => next_message = Some(message),
                                            }
                                        }
                                        if ui.add_enabled(!saving, egui::Button::new("Reset to defaults")).clicked() {
                                            editor.text = "{}".into();
                                        }
                                        if ui.add_enabled(!saving, egui::Button::new("Discard draft")).clicked() {
                                            discard = true;
                                        }
                                    });
                                    if let Some(message) = next_message {
                                        self.edit_message = Some(message);
                                    }
                                    if discard {
                                        self.editor = None;
                                        self.edit_message = None;
                                    }
                                } else {
                                    ui.label("A draft is open in another namespace.");
                                }
                            }
                        });
                });
            }
        });
        action
    }

    /// Settles a write without changing a failed draft or its expected revision.
    pub fn finish_update(&mut self, error: Option<String>) {
        self.requires_refresh = true;
        if let Some(error) = error {
            if let Some(editor) = &mut self.editor {
                editor.blocked = true;
            }
            self.edit_message = Some(error);
        } else {
            self.editor = None;
            self.edit_message = Some("Settings saved.".into());
        }
    }

    /// Allows new drafts after a successful read; existing drafts keep their revision.
    pub fn refreshed(&mut self) {
        self.requires_refresh = false;
    }

    fn reconcile_selection(&mut self, namespaces: &[SettingsNamespaceView]) {
        if !namespaces
            .iter()
            .any(|namespace| Some(&namespace.ns) == self.selected_namespace.as_ref())
        {
            self.selected_namespace = namespaces.first().map(|namespace| namespace.ns.clone());
        }
    }

    fn namespace_ui(&self, ui: &mut egui::Ui, namespace: &SettingsNamespaceView) {
        ui.heading(&namespace.ns);
        ui.label(format!("Revision: {}", namespace.revision));
        ui.label(format!("Applies: {}", namespace.applies));
        ui.label(if namespace.auto_generate {
            "Document generation: enabled"
        } else {
            "Document generation: disabled"
        });
        ui.label("Secret values are redacted by the Host.");
        ui.strong("Current values (redacted)");
        json_ui(ui, &namespace.value, self.pretty_json);

        if !namespace.secrets.is_empty() {
            ui.collapsing("Secret slots (status only)", |ui| {
                for secret in &namespace.secrets {
                    // JSON preserves path segments containing dots, slashes, or empty strings.
                    ui.label(format!(
                        "{}: {}",
                        serde_json::json!(secret.path),
                        if secret.set { "set" } else { "not set" }
                    ));
                }
            });
        }
        ui.collapsing("Schema", |ui| {
            json_ui(ui, &namespace.schema, self.pretty_json);
        });
    }
}

fn json_ui(ui: &mut egui::Ui, value: &serde_json::Value, pretty: bool) {
    match json_text(value, pretty) {
        Ok(text) => {
            ui.add(egui::Label::new(egui::RichText::new(text).monospace()).wrap());
        }
        Err(error) => {
            ui.colored_label(
                ui.visuals().error_fg_color,
                format!("Cannot display JSON: {error}"),
            );
        }
    }
}

fn json_text(value: &serde_json::Value, pretty: bool) -> serde_json::Result<String> {
    if pretty {
        serde_json::to_string_pretty(value)
    } else {
        serde_json::to_string(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn namespace(name: &str) -> SettingsNamespaceView {
        SettingsNamespaceView {
            auto_generate: false,
            ns: name.into(),
            schema: serde_json::json!({"type": "object"}),
            value: serde_json::json!({"token": "[REDACTED]", "enabled": true}),
            base: None,
            user: None,
            applies: "live".into(),
            secrets: vec![],
            revision: 1,
        }
    }

    #[test]
    fn editor_uses_user_overrides_and_captured_revision_only() {
        let mut ns = namespace("demo");
        ns.user = Some(serde_json::json!({"enabled": false, "token": "[REDACTED]"}));
        ns.secrets
            .push(dsh_client_core::settings::SettingsSecretView {
                path: vec!["token".into()],
                set: true,
            });
        let mut editor = SettingsEditor::new(&ns).unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&editor.text).unwrap(),
            ns.user.clone().unwrap()
        );
        editor.text = r#"{"enabled":true,"token":"[REDACTED]"}"#.into();
        let request = editor.request().unwrap();
        assert_eq!(request.ns, "demo");
        assert_eq!(request.expected_revision, Some(1));
        assert_eq!(
            serde_json::to_value(&request.ops).unwrap(),
            serde_json::json!([{"op":"set","path":["enabled"],"value":true}])
        );
    }

    #[test]
    fn editor_rejects_invalid_nonobject_deleted_unchanged_and_secret_fields() {
        let mut ns = namespace("demo");
        ns.user = Some(serde_json::json!({"x":1,"auth":{"token":"[REDACTED]"}}));
        ns.secrets
            .push(dsh_client_core::settings::SettingsSecretView {
                path: vec!["auth".into(), "token".into()],
                set: true,
            });
        let mut editor = SettingsEditor::new(&ns).unwrap();
        assert!(editor.request().is_err());
        for text in [
            "{",
            "[]",
            "null",
            "42",
            r#"{"auth":{"token":"new"}}"#,
        ] {
            editor.text = text.into();
            assert!(editor.request().is_err(), "{text}");
        }
        editor.text = r#"{"x":1,"auth":{"other":true}}"#.into();
        let cleared = editor.request().unwrap();
        assert_eq!(
            serde_json::to_value(&cleared.ops).unwrap(),
            serde_json::json!([
                {"op":"set","path":["auth","other"],"value":true},
                {"op":"unset","path":["auth","token"]}
            ])
        );
        editor.text = "{}".into();
        let removed = editor.request().unwrap();
        assert_eq!(
            serde_json::to_value(&removed.ops).unwrap(),
            serde_json::json!([{"op":"unset","path":["auth"]},{"op":"unset","path":["x"]}])
        );
        ns.user = Some(serde_json::json!([]));
        assert!(SettingsEditor::new(&ns).is_err());
    }

    #[test]
    fn failed_save_retains_draft_and_blocks_resubmission_after_refresh() {
        let mut panel = SettingsPanel::default();
        let mut editor = SettingsEditor::new(&namespace("demo")).unwrap();
        assert_eq!(editor.text, "{}");
        editor.text = r#"{"enabled":true}"#.into();
        panel.editor = Some(editor);
        panel.finish_update(Some("Conflict".into()));
        assert!(panel.requires_refresh);
        panel.refreshed();
        let editor = panel.editor.as_ref().unwrap();
        assert_eq!(editor.original.revision, 1);
        assert_eq!(editor.text, r#"{"enabled":true}"#);
        assert!(editor.request().is_err());
        panel.finish_update(None);
        assert!(panel.editor.is_none());
        assert!(panel.requires_refresh);
    }

    #[test]
    fn object_field_removal_becomes_an_unset_operation() {
        let mut ns = namespace("demo");
        ns.user = Some(serde_json::json!({"nested":{"a":1,"b":2},"items":[1,2]}));
        let mut editor = SettingsEditor::new(&ns).unwrap();
        editor.text = r#"{"nested":{"a":1},"items":[2,1]}"#.into();
        let request = editor.request().unwrap();
        assert_eq!(
            serde_json::to_value(&request.ops).unwrap(),
            serde_json::json!([
                {"op":"set","path":["items"],"value":[2,1]},
                {"op":"unset","path":["nested","b"]}
            ])
        );
    }

    #[test]
    fn selects_first_namespace_and_preserves_selection_across_reordering() {
        let mut panel = SettingsPanel::default();
        panel.reconcile_selection(&[namespace("a"), namespace("b")]);
        assert_eq!(panel.selected_namespace.as_deref(), Some("a"));
        panel.selected_namespace = Some("b".into());
        panel.reconcile_selection(&[namespace("b"), namespace("a")]);
        assert_eq!(panel.selected_namespace.as_deref(), Some("b"));
    }

    #[test]
    fn replaces_removed_selection_and_clears_it_for_empty_descriptions() {
        let mut panel = SettingsPanel::default();
        panel.reconcile_selection(&[namespace("a")]);
        panel.reconcile_selection(&[namespace("b")]);
        assert_eq!(panel.selected_namespace.as_deref(), Some("b"));
        panel.reconcile_selection(&[]);
        assert_eq!(panel.selected_namespace, None);
    }

    #[test]
    fn json_formats_preserve_redaction_and_json_types() {
        let value = serde_json::json!({
            "token": "[REDACTED]", "items": [null, true, 42, "line\nbreak"]
        });
        let compact = json_text(&value, false).unwrap();
        let pretty = json_text(&value, true).unwrap();
        assert!(!compact.contains('\n'));
        assert!(pretty.contains('\n'));
        for text in [compact, pretty] {
            assert!(text.contains("[REDACTED]"));
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(&text).unwrap(),
                value
            );
        }
    }

    #[test]
    fn renders_loading_empty_error_and_cached_descriptions_without_mutating_data() {
        let ctx = egui::Context::default();
        let mut panel = SettingsPanel::default();
        let mut description = SettingsDescribeValue {
            writable: false,
            has_document: false,
            namespaces: vec![namespace("demo")],
        };
        let original = description.clone();
        for (data, loading, error) in [
            (None, false, None),
            (None, true, None),
            (None, false, Some("Unavailable")),
            (Some(&description), false, None),
            (Some(&description), true, None),
            (Some(&description), false, Some("Refresh failed")),
        ] {
            ctx.begin_pass(egui::RawInput::default());
            let mut ui = egui::Ui::new(ctx.clone(), egui::Id::new("test"), Default::default());
            panel.ui(&mut ui, data, loading, error, false);
            let mut output = ctx.end_pass();
            output.textures_delta.clear();
            assert!(!output.shapes.is_empty());
        }
        assert_eq!(description, original);
        assert_eq!(panel.selected_namespace.as_deref(), Some("demo"));

        description.writable = true;
        description.has_document = true;
        description.namespaces.clear();
        ctx.begin_pass(egui::RawInput::default());
        let mut ui = egui::Ui::new(ctx.clone(), egui::Id::new("test"), Default::default());
        panel.ui(&mut ui, Some(&description), false, None, false);
        ctx.end_pass().textures_delta.clear();
        assert_eq!(panel.selected_namespace, None);
    }
}
