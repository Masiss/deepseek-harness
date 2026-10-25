//! Session-scoped workspace directory tree and read-only text preview.

use crate::accessible_button_label;
use dsh_client_core::{
    transport::HttpTransport,
    workspace_files::{
        WorkspaceDirectoryListing, WorkspaceFileRange, WorkspaceFileScope, WorkspaceFileText,
        WorkspaceFilesClient,
    },
};
use eframe::egui;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::mpsc;

const FIRST_PAGE_LINES: u64 = 300;

struct PendingListing {
    session_id: String,
    path: String,
    receiver: mpsc::Receiver<Result<WorkspaceDirectoryListing, String>>,
}

struct PendingRead {
    session_id: String,
    path: String,
    append: bool,
    expected_version: Option<String>,
    receiver: mpsc::Receiver<Result<WorkspaceFileText, String>>,
}

/// State for one Session's workspace tree and text preview.
#[derive(Default)]
pub(crate) struct WorkspaceExplorer {
    session_id: Option<String>,
    directories: BTreeMap<String, WorkspaceDirectoryListing>,
    expanded: BTreeSet<String>,
    list_errors: BTreeMap<String, String>,
    pending_listings: Vec<PendingListing>,
    selected_path: Option<String>,
    text: Option<WorkspaceFileText>,
    pending_read: Option<PendingRead>,
    read_error: Option<String>,
    single_pane: bool,
    show_file_pane: bool,
}

impl WorkspaceExplorer {
    /// Starts a new root listing when the displayed Session changes.
    pub(crate) fn select_session(
        &mut self,
        session_id: Option<&str>,
        transport: Option<&HttpTransport>,
    ) {
        if self.session_id.as_deref() == session_id {
            return;
        }
        self.session_id = session_id.map(str::to_owned);
        self.directories.clear();
        self.expanded.clear();
        self.list_errors.clear();
        self.pending_listings.clear();
        self.selected_path = None;
        self.text = None;
        self.pending_read = None;
        self.read_error = None;
        self.show_file_pane = false;
        if let (Some(session_id), Some(transport)) = (self.session_id.clone(), transport) {
            self.expanded.insert(String::new());
            self.start_listing(transport, session_id, String::new());
        }
    }

    /// Refreshes the Session root after reconnect or a user refresh.
    pub(crate) fn refresh_root(&mut self, transport: &HttpTransport) {
        let Some(session_id) = self.session_id.clone() else {
            return;
        };
        self.start_listing(transport, session_id, String::new());
    }

    /// Applies completed Host reads on the UI thread.
    pub(crate) fn poll(&mut self) {
        let mut index = 0;
        while index < self.pending_listings.len() {
            match self.pending_listings[index].receiver.try_recv() {
                Ok(result) => {
                    let pending = self.pending_listings.remove(index);
                    if self.session_id.as_deref() != Some(pending.session_id.as_str()) {
                        continue;
                    }
                    match result {
                        Ok(listing) => {
                            let path = normalized(&listing.path);
                            self.list_errors.remove(&path);
                            self.directories.insert(path, listing);
                        }
                        Err(error) => {
                            self.list_errors.insert(pending.path, error);
                        }
                    }
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    let pending = self.pending_listings.remove(index);
                    if self.session_id.as_deref() == Some(pending.session_id.as_str()) {
                        self.list_errors.insert(
                            pending.path,
                            "Directory request stopped unexpectedly".to_owned(),
                        );
                    }
                }
                Err(mpsc::TryRecvError::Empty) => index += 1,
            }
        }

        let Some(pending) = &self.pending_read else {
            return;
        };
        let result = match pending.receiver.try_recv() {
            Ok(result) => Some(result),
            Err(mpsc::TryRecvError::Disconnected) => {
                Some(Err("File preview stopped unexpectedly".to_owned()))
            }
            Err(mpsc::TryRecvError::Empty) => None,
        };
        let Some(result) = result else {
            return;
        };
        let Some(pending) = self.pending_read.take() else {
            return;
        };
        if self.session_id.as_deref() != Some(pending.session_id.as_str())
            || self.selected_path.as_deref() != Some(pending.path.as_str())
        {
            return;
        }
        match result {
            Ok(page)
                if pending
                    .expected_version
                    .as_deref()
                    .is_some_and(|version| version != page.stat.version) =>
            {
                self.read_error = Some(
                    "File changed while it was being previewed. Select it again to reload."
                        .to_owned(),
                );
            }
            Ok(page) if pending.append => {
                if let Some(text) = &mut self.text {
                    if !text.text.is_empty() && !page.text.is_empty() {
                        text.text.push('\n');
                    }
                    text.text.push_str(&page.text);
                    text.lines = text.lines.saturating_add(page.lines);
                    text.eof = page.eof;
                    self.read_error = None;
                }
            }
            Ok(page) => {
                self.text = Some(page);
                self.read_error = None;
            }
            Err(error) if error.contains("workspace-file/not-text") => {
                self.read_error = Some(
                    "This file is not UTF-8 text. Use Document preview for supported images and Office files.".to_owned(),
                );
            }
            Err(error) => self.read_error = Some(error),
        }
    }

    /// Renders the current Session's workspace tree and read-only text pane.
    pub(crate) fn ui(
        &mut self,
        ui: &mut egui::Ui,
        transport: Option<&HttpTransport>,
        session_id: Option<&str>,
        single_pane: bool,
    ) {
        self.select_session(session_id, transport);
        self.single_pane = single_pane;
        if !single_pane {
            self.show_file_pane = false;
        }
        ui.horizontal(|ui| {
            if single_pane && self.show_file_pane {
                if ui.small_button("‹ Files").clicked() {
                    self.show_file_pane = false;
                }
            }
            ui.heading(egui::RichText::new("Workspace Explorer").size(13.0));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(transport) = transport {
                    let refresh = ui.small_button("↻");
                    accessible_button_label(&refresh, "Refresh workspace");
                    if refresh.on_hover_text("Refresh workspace").clicked() {
                        self.refresh_root(transport);
                    }
                }
            });
        });
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Read only").size(10.0).strong());
            ui.label(
                egui::RichText::new("· scoped to this Session’s workspace")
                    .size(10.0)
                    .weak(),
            );
        });
        ui.separator();

        let Some(session_id) = self.session_id.clone() else {
            ui.label("Select a Session to browse its workspace.");
            return;
        };
        let Some(transport) = transport else {
            ui.label(
                egui::RichText::new("Host disconnected · loaded files are retained")
                    .size(10.0)
                    .weak(),
            );
            self.render_panes(ui, None, &session_id);
            return;
        };
        self.render_panes(ui, Some(transport), &session_id);
    }

    fn render_panes(
        &mut self,
        ui: &mut egui::Ui,
        transport: Option<&HttpTransport>,
        session_id: &str,
    ) {
        if self.single_pane {
            if self.show_file_pane {
                self.render_preview(ui, transport, session_id);
            } else {
                ui.label(
                    egui::RichText::new("REPOSITORY TREE")
                        .size(9.0)
                        .strong()
                        .weak(),
                );
                egui::ScrollArea::vertical()
                    .id_salt("workspace-explorer-tree")
                    .auto_shrink([false, false])
                    .show(ui, |ui| self.render_tree(ui, transport, session_id));
            }
            return;
        }
        ui.label(
            egui::RichText::new("REPOSITORY TREE")
                .size(9.0)
                .strong()
                .weak(),
        );
        egui::ScrollArea::vertical()
            .id_salt("workspace-explorer-tree")
            .max_height(210.0)
            .show(ui, |ui| self.render_tree(ui, transport, session_id));
        ui.separator();
        self.render_preview(ui, transport, session_id);
    }

    fn render_tree(
        &mut self,
        ui: &mut egui::Ui,
        transport: Option<&HttpTransport>,
        session_id: &str,
    ) {
        let Some(root) = self.directories.get("").cloned() else {
            if let Some(error) = self.list_errors.get("") {
                ui.colored_label(
                    ui.visuals().error_fg_color,
                    format!("Could not list workspace: {error}"),
                );
                if let Some(transport) = transport {
                    if ui.button("Retry workspace listing").clicked() {
                        self.start_listing(transport, session_id.to_owned(), String::new());
                    }
                }
            } else if self.is_listing_pending("") {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Loading workspace…");
                });
            } else {
                ui.label("Workspace tree is not loaded.");
            }
            return;
        };
        if root.entries.is_empty() {
            ui.label("This workspace is empty.");
            return;
        }
        self.render_directory(ui, "", 0, transport, session_id);
        if root.truncated {
            ui.label(
                egui::RichText::new("More entries omitted by the Host limit.")
                    .size(10.0)
                    .weak(),
            );
        }
    }

    fn render_directory(
        &mut self,
        ui: &mut egui::Ui,
        parent: &str,
        depth: usize,
        transport: Option<&HttpTransport>,
        session_id: &str,
    ) {
        let Some(listing) = self.directories.get(parent).cloned() else {
            return;
        };
        for entry in listing.entries {
            let path = join_workspace_path(parent, &entry.name);
            match entry.entry_type.as_str() {
                "directory" => {
                    let expanded = self.expanded.contains(&path);
                    let label = if expanded { "▾" } else { "▸" };
                    ui.horizontal(|ui| {
                        ui.add_space(depth as f32 * 12.0);
                        if ui.small_button(label).clicked() {
                            if expanded {
                                self.expanded.remove(&path);
                            } else {
                                self.expanded.insert(path.clone());
                                if !self.directories.contains_key(&path) {
                                    if let Some(transport) = transport {
                                        self.start_listing(
                                            transport,
                                            session_id.to_owned(),
                                            path.clone(),
                                        );
                                    }
                                }
                            }
                        }
                        let response = ui.selectable_label(
                            false,
                            egui::RichText::new(format!("▰ {}", entry.name)).size(11.0),
                        );
                        accessible_button_label(&response, &format!("{} directory", entry.name));
                        if response.clicked() && !expanded {
                            self.expanded.insert(path.clone());
                            if !self.directories.contains_key(&path) {
                                if let Some(transport) = transport {
                                    self.start_listing(
                                        transport,
                                        session_id.to_owned(),
                                        path.clone(),
                                    );
                                }
                            }
                        }
                    });
                    if expanded {
                        if let Some(error) = self.list_errors.get(&path).cloned() {
                            ui.horizontal(|ui| {
                                ui.add_space((depth + 1) as f32 * 12.0);
                                ui.colored_label(ui.visuals().error_fg_color, &error);
                                if let Some(transport) = transport {
                                    if ui.small_button("Retry").clicked() {
                                        self.start_listing(
                                            transport,
                                            session_id.to_owned(),
                                            path.clone(),
                                        );
                                    }
                                }
                            });
                        } else if self.is_listing_pending(&path) {
                            ui.horizontal(|ui| {
                                ui.add_space((depth + 1) as f32 * 12.0);
                                ui.spinner();
                                ui.small("Loading…");
                            });
                        } else if self.directories.contains_key(&path) {
                            self.render_directory(ui, &path, depth + 1, transport, session_id);
                        }
                    }
                }
                "file" => {
                    ui.horizontal(|ui| {
                        ui.add_space((depth + 1) as f32 * 12.0);
                        let selected = self.selected_path.as_deref() == Some(path.as_str());
                        let response = ui.selectable_label(selected, format!("▤ {}", entry.name));
                        accessible_button_label(&response, &format!("Preview {}", entry.name));
                        if response.clicked() {
                            if let Some(transport) = transport {
                                self.select_file(transport, session_id.to_owned(), path.clone());
                                self.show_file_pane = self.single_pane;
                            }
                        }
                    });
                }
                "symlink" => {
                    ui.horizontal(|ui| {
                        ui.add_space((depth + 1) as f32 * 12.0);
                        ui.small(format!("↗ {} · link", entry.name));
                    });
                }
                _ => {
                    ui.horizontal(|ui| {
                        ui.add_space((depth + 1) as f32 * 12.0);
                        ui.small(format!("{} · unsupported entry", entry.name));
                    });
                }
            }
        }
        if listing.truncated {
            ui.horizontal(|ui| {
                ui.add_space((depth + 1) as f32 * 12.0);
                ui.small("More entries were omitted by the Host limit.");
            });
        }
    }

    fn render_preview(
        &mut self,
        ui: &mut egui::Ui,
        transport: Option<&HttpTransport>,
        session_id: &str,
    ) {
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("SOURCE PREVIEW")
                    .size(9.0)
                    .strong()
                    .weak(),
            );
            if let (Some(path), Some(transport)) = (self.selected_path.clone(), transport) {
                if ui
                    .small_button("↻")
                    .on_hover_text("Reload source")
                    .clicked()
                {
                    self.select_file(transport, session_id.to_owned(), path);
                }
            }
        });
        let Some(path) = self.selected_path.clone() else {
            ui.label(
                egui::RichText::new("Select a file to preview its contents.")
                    .size(11.0)
                    .weak(),
            );
            return;
        };
        ui.horizontal_wrapped(|ui| {
            for part in path.split('/') {
                ui.label(egui::RichText::new(part).monospace().size(10.0).weak());
                if part != path.rsplit('/').next().unwrap_or(&path) {
                    ui.label(egui::RichText::new("›").size(10.0).weak());
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(egui::RichText::new("Read only").size(9.0).weak());
            });
        });
        ui.separator();
        if self.pending_read.is_some() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Reading text file…");
            });
        }
        if let Some(error) = &self.read_error {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
        let Some(text) = &self.text else {
            if self.read_error.is_none() && self.pending_read.is_none() {
                ui.label(
                    "Choose a UTF-8 text file. Images and Office documents use Document preview.",
                );
            }
            return;
        };
        if text.text.is_empty() {
            ui.label("This text file is empty.");
        } else {
            egui::ScrollArea::both()
                .id_salt(("workspace-source", &text.stat.version))
                .auto_shrink([false, false])
                .max_height(ui.available_height().max(80.0))
                .show(ui, |ui| {
                    let extension = path.rsplit_once('.').map(|(_, extension)| extension);
                    for (index, line) in text.text.split('\n').enumerate() {
                        ui.horizontal(|ui| {
                            ui.add_sized(
                                [35.0, 18.0],
                                egui::Label::new(
                                    egui::RichText::new((text.offset + index as u64).to_string())
                                        .monospace()
                                        .size(10.0)
                                        .weak(),
                                ),
                            );
                            ui.add(
                                egui::Label::new(source_line_job(line, extension, ui.visuals()))
                                    .selectable(true)
                                    .wrap_mode(egui::TextWrapMode::Extend),
                            );
                        });
                    }
                });
        }
        if let Some(bytes) = text.stat.bytes {
            ui.small(format!("{bytes} bytes · read only"));
        }
        let mut next_page = None;
        if !text.eof {
            ui.horizontal(|ui| {
                ui.small("Preview limited to a page of lines.");
                if ui.button("Load next lines").clicked() {
                    next_page = Some((
                        text.offset.saturating_add(text.lines),
                        text.stat.version.clone(),
                    ));
                }
            });
        }
        if let (Some((offset, version)), Some(transport)) = (next_page, transport) {
            self.read_page(
                transport,
                session_id.to_owned(),
                path,
                offset,
                true,
                Some(version),
            );
        }
    }

    fn select_file(&mut self, transport: &HttpTransport, session_id: String, path: String) {
        if self.selected_path.as_deref() == Some(path.as_str()) && self.pending_read.is_some() {
            return;
        }
        self.selected_path = Some(path.clone());
        self.text = None;
        self.read_error = None;
        self.show_file_pane = self.single_pane;
        self.read_page(transport, session_id, path, 1, false, None);
    }

    fn read_page(
        &mut self,
        transport: &HttpTransport,
        session_id: String,
        path: String,
        offset: u64,
        append: bool,
        expected_version: Option<String>,
    ) {
        let (sender, receiver) = mpsc::channel();
        let worker_transport = transport.clone();
        let worker_session = session_id.clone();
        let worker_path = path.clone();
        std::thread::spawn(move || {
            let result = WorkspaceFilesClient::new(&worker_transport)
                .read(
                    WorkspaceFileScope {
                        session_id: worker_session,
                    },
                    worker_path,
                    WorkspaceFileRange {
                        offset: Some(offset),
                        limit: Some(FIRST_PAGE_LINES),
                    },
                )
                .map_err(|error| error.to_string());
            let _ = sender.send(result);
        });
        self.pending_read = Some(PendingRead {
            session_id,
            path,
            append,
            expected_version,
            receiver,
        });
        self.read_error = None;
    }

    fn start_listing(&mut self, transport: &HttpTransport, session_id: String, path: String) {
        if self
            .pending_listings
            .iter()
            .any(|pending| pending.session_id == session_id && pending.path == path)
        {
            return;
        }
        let (sender, receiver) = mpsc::channel();
        let worker_transport = transport.clone();
        let worker_session = session_id.clone();
        let request_path = if path.is_empty() {
            ".".to_owned()
        } else {
            path.clone()
        };
        std::thread::spawn(move || {
            let result = WorkspaceFilesClient::new(&worker_transport)
                .list(
                    WorkspaceFileScope {
                        session_id: worker_session,
                    },
                    request_path,
                )
                .map_err(|error| error.to_string());
            let _ = sender.send(result);
        });
        self.list_errors.remove(&path);
        self.pending_listings.push(PendingListing {
            session_id,
            path,
            receiver,
        });
    }

    fn is_listing_pending(&self, path: &str) -> bool {
        self.pending_listings
            .iter()
            .any(|pending| pending.path == path)
    }

    #[cfg(test)]
    pub(crate) fn read_error(&self) -> Option<&str> {
        self.read_error.as_deref()
    }

    #[cfg(test)]
    pub(crate) fn selected_path(&self) -> Option<&str> {
        self.selected_path.as_deref()
    }

    #[cfg(test)]
    pub(crate) fn has_directory(&self, path: &str) -> bool {
        self.directories.contains_key(path)
    }

    #[cfg(test)]
    pub(crate) fn is_expanded(&self, path: &str) -> bool {
        self.expanded.contains(path)
    }

    #[cfg(test)]
    pub(crate) fn preview_contains(&self, value: &str) -> bool {
        self.text
            .as_ref()
            .is_some_and(|text| text.text.contains(value))
    }

    #[cfg(test)]
    pub(crate) fn is_showing_file(&self) -> bool {
        self.single_pane && self.show_file_pane
    }
}

fn normalized(path: &str) -> String {
    if path == "." || path == "/" {
        String::new()
    } else {
        path.trim_matches('/').to_owned()
    }
}

fn join_workspace_path(parent: &str, child: &str) -> String {
    if parent.is_empty() {
        child.to_owned()
    } else {
        format!("{parent}/{child}")
    }
}

fn source_line_job(
    line: &str,
    extension: Option<&str>,
    visuals: &egui::Visuals,
) -> egui::text::LayoutJob {
    let dark = visuals.dark_mode;
    let comment_color = visuals.weak_text_color();
    let keyword_color = visuals.hyperlink_color;
    let string_color = if dark {
        egui::Color32::from_rgb(0x7f, 0xc9, 0x9a)
    } else {
        egui::Color32::from_rgb(0x26, 0x70, 0x4a)
    };
    let type_color = if dark {
        egui::Color32::from_rgb(0x87, 0xb9, 0xed)
    } else {
        egui::Color32::from_rgb(0x24, 0x5b, 0x88)
    };
    let number_color = if dark {
        egui::Color32::from_rgb(0xe3, 0xc4, 0x77)
    } else {
        egui::Color32::from_rgb(0x8b, 0x6b, 0x2b)
    };
    let rust = extension == Some("rs");
    let toml = extension == Some("toml");
    let mut job = egui::text::LayoutJob::default();
    let mut index = 0;
    while index < line.len() {
        let rest = &line[index..];
        if (rust && rest.starts_with("//")) || (toml && rest.starts_with('#')) {
            append_source_span(&mut job, rest, comment_color, true);
            break;
        }
        let character = rest
            .chars()
            .next()
            .expect("index remains on a character boundary");
        if character == '"' || (rust && character == '\'') {
            let quote = character;
            let mut end = index + character.len_utf8();
            let mut escaped = false;
            while end < line.len() {
                let next = line[end..]
                    .chars()
                    .next()
                    .expect("string remains valid UTF-8");
                end += next.len_utf8();
                if escaped {
                    escaped = false;
                } else if next == '\\' {
                    escaped = true;
                } else if next == quote {
                    break;
                }
            }
            append_source_span(&mut job, &line[index..end], string_color, false);
            index = end;
            continue;
        }
        if character == '_' || character.is_alphabetic() {
            let mut end = index + character.len_utf8();
            while end < line.len() {
                let next = line[end..]
                    .chars()
                    .next()
                    .expect("identifier remains valid UTF-8");
                if !(next == '_' || next.is_alphanumeric()) {
                    break;
                }
                end += next.len_utf8();
            }
            let word = &line[index..end];
            let color = if is_source_keyword(word, rust, toml) {
                keyword_color
            } else if word.chars().next().is_some_and(char::is_uppercase) {
                type_color
            } else {
                visuals.text_color()
            };
            append_source_span(&mut job, word, color, false);
            index = end;
            continue;
        }
        if character.is_ascii_digit() {
            let mut end = index + character.len_utf8();
            while end < line.len() {
                let next = line[end..]
                    .chars()
                    .next()
                    .expect("number remains valid UTF-8");
                if !(next.is_ascii_digit() || next == '.' || next == '_') {
                    break;
                }
                end += next.len_utf8();
            }
            append_source_span(&mut job, &line[index..end], number_color, false);
            index = end;
            continue;
        }
        append_source_span(
            &mut job,
            &line[index..index + character.len_utf8()],
            visuals.text_color(),
            false,
        );
        index += character.len_utf8();
    }
    job
}

fn append_source_span(
    job: &mut egui::text::LayoutJob,
    text: &str,
    color: egui::Color32,
    italics: bool,
) {
    job.append(
        text,
        0.0,
        egui::TextFormat {
            font_id: egui::FontId::monospace(11.0),
            color,
            italics,
            ..Default::default()
        },
    );
}

fn is_source_keyword(word: &str, rust: bool, toml: bool) -> bool {
    (rust
        && matches!(
            word,
            "use"
                | "pub"
                | "fn"
                | "let"
                | "mut"
                | "struct"
                | "impl"
                | "mod"
                | "const"
                | "static"
                | "enum"
                | "match"
                | "if"
                | "else"
                | "for"
                | "in"
                | "return"
                | "self"
                | "true"
                | "false"
                | "async"
                | "await"
        ))
        || (toml && matches!(word, "true" | "false"))
}
