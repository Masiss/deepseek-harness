use super::*;

impl DesktopApp {
    pub(crate) fn render_ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if ui.ctx().input(|i| i.viewport().close_requested())
            && self.tray_available
            && !self.quitting.load(std::sync::atomic::Ordering::Acquire)
        {
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.hide_requested
                .store(true, std::sync::atomic::Ordering::Release);
            self.hidden.store(true, Ordering::Release);
        }
        // Navigation rail: compact, icon-first, grouped.
        egui::Panel::left("nav_rail")
            .resizable(false)
            .exact_size(56.0)
            .show(ui, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(8.0);
                    ui.label(egui::RichText::new("DSH").strong().size(11.0));
                    ui.add_space(8.0);
                    let items = [
                        ("S", "Sessions", ActiveView::Sessions),
                        ("D", "Document", ActiveView::Document),
                        ("P", "Plugins", ActiveView::Plugins),
                        ("K", "Skills", ActiveView::Skills),
                        ("M", "Market", ActiveView::Marketplace),
                        ("T", "Settings", ActiveView::Settings),
                    ];
                    for (icon, tooltip, view) in items {
                        let selected = self.active_view == view;
                        let btn = ui.add(
                            egui::Button::new(egui::RichText::new(icon).size(16.0).strong())
                                .min_size(egui::vec2(40.0, 40.0)),
                        );
                        let rect = btn.rect;
                        let clicked = btn.clicked();
                        btn.on_hover_text(tooltip);
                        if selected {
                            ui.painter().rect_filled(
                                rect,
                                8.0,
                                ui.visuals().selection.bg_fill,
                            );
                        }
                        if clicked {
                            self.active_view = view;
                            match view {
                                ActiveView::Sessions => {
                                    self.show_settings = false;
                                    self.show_document = false;
                                    self.show_plugins = false;
                                    self.show_skills = false;
                                    self.show_market = false;
                                }
                                ActiveView::Settings => {
                                    self.open_settings();
                                    self.show_document = false;
                                    self.show_plugins = false;
                                    self.show_skills = false;
                                    self.show_market = false;
                                }
                                ActiveView::Document => {
                                    self.show_document = true;
                                    self.show_settings = false;
                                    self.show_plugins = false;
                                    self.show_skills = false;
                                    self.show_market = false;
                                }
                                ActiveView::Plugins => {
                                    self.show_plugins = true;
                                    self.show_settings = false;
                                    self.show_document = false;
                                    self.show_skills = false;
                                    self.show_market = false;
                                    if self.plugins_snapshot.is_none() {
                                        self.refresh_plugins();
                                    }
                                }
                                ActiveView::Skills => {
                                    self.show_skills = true;
                                    self.show_settings = false;
                                    self.show_document = false;
                                    self.show_plugins = false;
                                    self.show_market = false;
                                    if self.skills.is_none() {
                                        self.refresh_skills();
                                    }
                                }
                                ActiveView::Marketplace => {
                                    self.show_market = true;
                                    self.show_settings = false;
                                    self.show_document = false;
                                    self.show_plugins = false;
                                    self.show_skills = false;
                                    if self.market_indexes.is_empty() {
                                        self.refresh_market_indexes();
                                    }
                                }
                            }
                        }
                    }
                });
            });
        egui::Panel::left("sessions")
            .resizable(true)
            .default_size(280.0)
            .min_size(220.0)
            .show(ui, |ui| {
            if ui
                .selectable_label(
                    !self.show_settings && !self.show_plugins && !self.show_skills,
                    "Sessions",
                )
                .clicked()
            {
                self.show_settings = false;
                self.show_document = false;
                self.show_plugins = false;
                self.show_skills = false;
                self.show_market = false;
            }
            if ui
                .selectable_label(self.show_settings, "Settings")
                .clicked()
            {
                self.open_settings();
                self.show_document = false;
                self.show_plugins = false;
                self.show_skills = false;
            }
            if ui
                .selectable_label(self.show_document, "Document")
                .clicked()
            {
                self.show_document = true;
                self.show_settings = false;
                self.show_plugins = false;
                self.show_skills = false;
            }
            if ui
                .selectable_label(self.show_plugins, "Plugins")
                .clicked()
            {
                self.show_plugins = true;
                self.show_settings = false;
                self.show_document = false;
                self.show_skills = false;
                if self.plugins_snapshot.is_none() {
                    self.refresh_plugins();
                }
            }
            if ui
                .selectable_label(self.show_skills, "Skills")
                .clicked()
            {
                self.show_skills = true;
                self.show_settings = false;
                self.show_document = false;
                self.show_plugins = false;
                if self.skills.is_none() {
                    self.refresh_skills();
                }
            }
            if ui
                .selectable_label(self.show_market, "Marketplace")
                .clicked()
            {
                self.show_market = true;
                self.show_settings = false;
                self.show_document = false;
                self.show_plugins = false;
                self.show_skills = false;
                if self.market_indexes.is_empty() {
                    self.refresh_market_indexes();
                }
            }
            ui.heading("Sessions");
            let connected = self.transport.is_some();
            let busy = self.sessions_receiver.is_some();
            ui.add_enabled_ui(connected && !busy, |ui| {
                ui.horizontal_wrapped(|ui| {
                    if ui.button("Refresh").clicked() {
                        self.session_query.clear();
                        self.refresh_sessions();
                    }
                    if ui.button("New Session").clicked() {
                        self.request_sessions(SessionOperation::Create);
                    }
                });
                let search = ui.add(
                    egui::TextEdit::singleline(&mut self.session_query)
                        .hint_text("Search sessions")
                        .desired_width(170.0),
                );
                if ui.button("Search").clicked()
                    || (search.lost_focus()
                        && ui.input(|input| input.key_pressed(egui::Key::Enter)))
                {
                    if self.session_query.trim().is_empty() {
                        self.refresh_sessions();
                    } else {
                        self.request_sessions(SessionOperation::Search(self.session_query.clone()));
                    }
                }
                if let Some(session_id) = self.selected_session.clone() {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.session_title)
                            .hint_text("New session title")
                            .desired_width(170.0),
                    );
                    ui.horizontal_wrapped(|ui| {
                        if ui
                            .add_enabled(
                                !self.session_title.trim().is_empty(),
                                egui::Button::new("Rename selected"),
                            )
                            .clicked()
                        {
                            self.request_sessions(SessionOperation::Rename(RenameSessionRequest {
                                session_id: session_id.clone(),
                                title: self.session_title.trim().to_owned(),
                            }));
                        }
                        if ui.button("Fork selected").clicked() {
                            self.request_sessions(SessionOperation::Fork(ForkSessionRequest {
                                session_id,
                                at_seq: None,
                                allow_migration: None,
                            }));
                        }
                    });
                }
            });
            if !connected {
                ui.label("Connect to enable sessions");
            }
            if busy {
                ui.spinner();
            }
            if !self.sessions_status.is_empty() {
                ui.small(&self.sessions_status);
            }
            if let Some(error) = &self.sessions_error {
                ui.colored_label(egui::Color32::LIGHT_RED, error);
            }
            ui.add_enabled_ui(!busy, |ui| {
                let mut selected = None;
                egui::ScrollArea::vertical()
                    .id_salt("session-inventory")
                    .max_height(300.0)
                    .show(ui, |ui| {
                        if let Some(results) = &self.session_search {
                            for item in &results.items {
                                let display = if item.session_id.chars().count() > 26 {
                                    format!("{}…", item.session_id.chars().take(25).collect::<String>())
                                } else {
                                    item.session_id.clone()
                                };
                                if ui
                                    .selectable_label(
                                        self.selected_session.as_ref() == Some(&item.session_id),
                                        display,
                                    )
                                    .on_hover_text(format!("{}\n{}", item.session_id, item.snippet))
                                    .clicked()
                                {
                                    selected = Some(item.session_id.clone());
                                }
                            }
                            if results.has_more {
                                ui.small("More matches exist; refine your search.");
                            }
                        } else {
                            let now = now_millis();
                            for summary in &self.sessions {
                                let label = session_label(summary);
                                let running = if summary.running { "● " } else { "" };
                                let blank = if summary.blank { " (blank)" } else { "" };
                                let age = relative_time(summary.updated_at, now);
                                let display = format!("{running}{label}{blank}");
                                let display = if display.chars().count() > 26 {
                                    format!("{}…", display.chars().take(25).collect::<String>())
                                } else {
                                    display
                                };
                                if ui
                                    .selectable_label(
                                        self.selected_session.as_ref()
                                            == Some(&summary.session_id),
                                        format!("{display}  ·  {age}"),
                                    )
                                    .on_hover_text(format!(
                                        "{}\n{}",
                                        summary.session_id, label
                                    ))
                                    .clicked()
                                {
                                    selected = Some(summary.session_id.clone());
                                }
                            }
                        }
                    });
                if let Some(session_id) = selected {
                    self.load_session(session_id);
                }
            });
            if ui.button("Open file").clicked() {
                let _ = dialog::open_file();
            }
            if ui.button("Save file").clicked() {
                let _ = dialog::save_file();
            }
        });
        egui::Panel::bottom("status").show(ui, |ui| {
            ui.label(&self.status);
        });
        if self.selected_session.is_some() {
            egui::Panel::bottom("schedules-panel").show(ui, |ui| {
                egui::CollapsingHeader::new(format!("Schedules ({})", self.schedules.len()))
                    .default_open(false)
                    .show(ui, |ui| {
                        if let Some(error) = &self.schedules_error {
                            ui.colored_label(egui::Color32::LIGHT_RED, error);
                        }
                        let mut deleted = None;
                        egui::ScrollArea::vertical()
                            .id_salt("schedule-list")
                            .max_height(120.0)
                            .show(ui, |ui| {
                                for record in &self.schedules {
                                    let id = record
                                        .get("id")
                                        .and_then(serde_json::Value::as_str)
                                        .unwrap_or_default()
                                        .to_owned();
                                    let title = record
                                        .get("title")
                                        .and_then(serde_json::Value::as_str)
                                        .unwrap_or(&id);
                                    let kind = record
                                        .get("kind")
                                        .and_then(serde_json::Value::as_str)
                                        .unwrap_or("schedule");
                                    let next = record
                                        .get("scheduledAt")
                                        .and_then(serde_json::Value::as_str)
                                        .unwrap_or("");
                                    ui.horizontal_wrapped(|ui| {
                                        ui.label(format!("{title} · {kind} · {next}"));
                                        if ui
                                            .add_enabled(
                                                self.schedule_delete_receiver.is_none(),
                                                egui::Button::new("Delete"),
                                            )
                                            .clicked()
                                        {
                                            deleted = Some(id.clone());
                                        }
                                    });
                                }
                                if self.schedules.is_empty() && self.schedules_receiver.is_none() {
                                    ui.label("No schedules for this Session.");
                                }
                            });
                        if let Some(id) = deleted {
                            self.delete_schedule(id);
                        }
                    });
            });
        }
        if self.selected_session.is_some() {
            egui::Panel::bottom("jobs-panel").show(ui, |ui| {
                egui::CollapsingHeader::new(format!("Jobs ({})", self.jobs.len()))
                    .default_open(false)
                    .show(ui, |ui| {
                        if let Some(error) = &self.jobs_error {
                            ui.colored_label(egui::Color32::LIGHT_RED, error);
                        }
                        let mut next = None;
                        egui::ScrollArea::vertical()
                            .id_salt("jobs-list")
                            .max_height(90.0)
                            .show(ui, |ui| {
                                for job in &self.jobs {
                                    let selected =
                                        self.job_selected.as_deref() == Some(job.id.as_str());
                                    if ui
                                        .selectable_label(
                                            selected,
                                            format!(
                                                "{} · {} · {}",
                                                job.label, job.kind, job.status
                                            ),
                                        )
                                        .clicked()
                                    {
                                        next = Some(job.id.clone());
                                    }
                                }
                            });
                        if let Some(id) = next {
                            self.start_job_follow(id);
                        }
                        if !self.job_status.is_empty() {
                            ui.small(&self.job_status);
                        }
                        if !self.job_output.is_empty() {
                            egui::ScrollArea::vertical()
                                .id_salt("job-output")
                                .max_height(140.0)
                                .stick_to_bottom(true)
                                .show(ui, |ui| {
                                    ui.add(
                                        egui::Label::new(
                                            egui::RichText::new(&self.job_output).monospace(),
                                        )
                                        .selectable(true)
                                        .wrap_mode(egui::TextWrapMode::Extend),
                                    );
                                });
                        }
                    });
            });
        }
        if !self.deliverables.is_empty() {
            egui::Panel::bottom("deliverables-panel").show(ui, |ui| {
                egui::CollapsingHeader::new(format!("Deliverables ({})", self.deliverables.len()))
                    .default_open(true)
                    .show(ui, |ui| {
                        let mut preview = None;
                        for file in &self.deliverables {
                            let label = egui::RichText::new(&file.path).monospace();
                            if ui
                                .link(label)
                                .on_hover_text(
                                    file.description.as_deref().unwrap_or("Open this deliverable"),
                                )
                                .clicked()
                            {
                                preview = Some(file.path.clone());
                            }
                            if let Some(description) = &file.description {
                                ui.small(description);
                            }
                        }
                        if let Some(path) = preview {
                            self.preview_path = path.clone();
                            self.show_document = true;
                            self.show_settings = false;
                            self.show_plugins = false;
                            self.show_skills = false;
                            if Self::is_office_path(&path) {
                                self.read_office_document();
                            } else {
                                self.read_document();
                            }
                        }
                    });
            });
        }
        if self.selected_session.is_some() {
            egui::Panel::bottom("terminal-panel").show(ui, |ui| {
                egui::CollapsingHeader::new("Session terminals")
                    .default_open(true)
                    .show(ui, |ui| {
                        self.terminal
            .ui(ui, self.transport.clone(), self.streams.clone());
                    });
            });
        }
        if std::env::var("DSH_DESKTOP_DEBUG_LAYOUT").as_deref() == Ok("1") {
            let vp = ui.ctx().input(|i| i.viewport().inner_rect).unwrap_or(egui::Rect::ZERO);
            eprintln!(
                "dsh: layout viewport={}x{} nav_rail=56 sessions=280 central={}x{}",
                vp.width() as i32,
                vp.height() as i32,
                (vp.width() - 56.0 - 280.0) as i32,
                (vp.height() - 36.0 - 24.0) as i32
            );
        }
        egui::CentralPanel::default().show(ui, |ui| {
            if !self.show_settings && !self.show_plugins && !self.show_skills && !self.show_market && !self.show_document
                && self.selected_session.is_none()
            {
                ui.vertical_centered(|ui| {
                    ui.add_space(80.0);
                    ui.heading("No session selected");
                    ui.label("Open an existing session from the sidebar, or start a new one.");
                    ui.add_space(12.0);
                    if ui.button("New Session").clicked() {
                        self.request_sessions(SessionOperation::Create);
                    }
                    if ui.button("Refresh sessions").clicked() {
                        self.session_query.clear();
                        self.refresh_sessions();
                    }
                });
                return;
            }
            if self.show_settings {
                let action = self.settings_panel.ui(
                    ui,
                    self.settings_description.as_ref(),
                    self.settings_loading,
                    self.settings_error.as_deref(),
                    self.settings_update_receiver.is_some(),
                );
                match action {
                    Some(SettingsAction::Refresh) => self.refresh_settings(),
                    Some(SettingsAction::Mutate(request)) => self.mutate_settings(request),
                    None => {}
                }
            } else if self.show_plugins {
                ui.horizontal(|ui| {
                    ui.heading("Plugins");
                    if ui
                        .add_enabled(!self.plugins_loading, egui::Button::new("Refresh"))
                        .clicked()
                    {
                        self.refresh_plugins();
                    }
                    if self.plugins_loading {
                        ui.spinner();
                    }
                });
                if let Some(error) = &self.plugins_error {
                    ui.colored_label(egui::Color32::LIGHT_RED, error);
                }
                if let Some(status) = &self.plugin_write_status {
                    ui.label(status);
                }
                let busy = self.plugin_write_receiver.is_some();
                let mut plugin_action = None;
                if let Some(snapshot) = &self.plugins_snapshot {
                    if snapshot.management_available == Some(false) {
                        ui.label("Plugin management is unavailable for this Host.");
                    }
                    egui::ScrollArea::vertical()
                        .id_salt("plugin-inventory")
                        .max_height(420.0)
                        .show(ui, |ui| {
                            if snapshot.entries.is_empty() {
                                ui.label("No configured plugins.");
                            }
                            for entry in &snapshot.entries {
                                let phase = entry.fiber_phase.map_or_else(
                                    || "unknown".to_owned(),
                                    |phase| format!("{phase:?}").to_lowercase(),
                                );
                                let state = if entry.enabled { "enabled" } else { "disabled" };
                                ui.horizontal(|ui| {
                                    ui.label(format!(
                                        "{} · {state} · {phase}",
                                        entry.module_name
                                    ));
                                    let managed = self
                                        .managed_plugins
                                        .iter()
                                        .find(|plugin| plugin.entry_id == entry.entry_id);
                                    match managed {
                                        Some(plugin) if plugin.patch_id.is_some() => {
                                            let label = if entry.enabled {
                                                "Disable"
                                            } else {
                                                "Enable"
                                            };
                                            if ui
                                                .add_enabled(
                                                    !busy,
                                                    egui::Button::new(label).small(),
                                                )
                                                .clicked()
                                            {
                                                plugin_action = Some(
                                                    PluginAction::SetPlugin {
                                                        id: entry.entry_id.clone(),
                                                        enabled: !entry.enabled,
                                                    },
                                                );
                                            }
                                        }
                                        Some(plugin) => {
                                            if let Some(reason) = &plugin.read_only_reason {
                                                ui.small(format!("read-only: {reason}"));
                                            }
                                        }
                                        None => {}
                                    }
                                });
                            }
                            if !self.plugin_bundles.is_empty() {
                                ui.separator();
                                ui.heading("Bundles");
                                for bundle in &self.plugin_bundles {
                                    ui.horizontal(|ui| {
                                        let state = if bundle.enabled {
                                            "enabled"
                                        } else {
                                            "disabled"
                                        };
                                        ui.label(format!(
                                            "{} · {state} · {}",
                                            bundle.name, bundle.availability
                                        ));
                                        let label = if bundle.enabled {
                                            "Disable"
                                        } else {
                                            "Enable"
                                        };
                                        if ui
                                            .add_enabled(
                                                !busy,
                                                egui::Button::new(label).small(),
                                            )
                                            .clicked()
                                        {
                                            plugin_action = Some(PluginAction::SetBundle {
                                                name: bundle.name.clone(),
                                                enabled: !bundle.enabled,
                                            });
                                        }
                                    });
                                    if let Some(description) = &bundle.description {
                                        ui.small(description);
                                    }
                                }
                            }
                            if let Some(presets) = &snapshot.agent_presets {
                                ui.separator();
                                ui.heading("Agent presets");
                                for preset in presets {
                                    let name = preset.name.as_deref().unwrap_or(&preset.id);
                                    ui.label(format!("{name} · {} row(s)", preset.rows.len()));
                                }
                            }
                        });
                } else if !self.plugins_loading {
                    ui.label("Open this tab to load the plugin inventory.");
                }
                if let Some(action) = plugin_action {
                    self.write_plugin(action);
                }
                if let Some(error) = &self.page_error {
                    ui.colored_label(egui::Color32::LIGHT_RED, error);
                }
            } else if self.show_market {
                ui.horizontal(|ui| {
                    ui.heading("Marketplace");
                    if ui
                        .add_enabled(!self.market_loading, egui::Button::new("Refresh indexes"))
                        .clicked()
                    {
                        self.refresh_market_indexes();
                    }
                    if self.market_loading {
                        ui.spinner();
                    }
                });
                if let Some(status) = &self.market_status {
                    ui.label(status);
                }
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.market_query).hint_text("Search text").desired_width(240.0));
                    if ui.add_enabled(!self.market_loading, egui::Button::new("Search")).clicked() {
                        self.search_market();
                    }
                });
                if self.market_indexes.is_empty() && !self.market_loading {
                    ui.label("No market indexes configured.");
                }
                let mut install_spec = None;
                if !self.market_results.is_empty() {
                    egui::ScrollArea::vertical()
                        .id_salt("market-results")
                        .max_height(420.0)
                        .show(ui, |ui| {
                            for entry in &self.market_results {
                                let name = entry
                                    .get("name")
                                    .and_then(serde_json::Value::as_str)
                                    .unwrap_or("plugin");
                                ui.horizontal(|ui| {
                                    ui.label(name);
                                    let default_spec = entry
                                        .get("versions")
                                        .and_then(serde_json::Value::as_array)
                                        .and_then(|versions| versions.first())
                                        .and_then(|version| version.get("install"))
                                        .and_then(|install| install.get("spec"))
                                        .and_then(serde_json::Value::as_str)
                                        .map(str::to_owned);
                                    if let Some(spec) = default_spec {
                                        if ui
                                            .add_enabled(
                                                !self.market_loading,
                                                egui::Button::new("Install").small(),
                                            )
                                            .clicked()
                                        {
                                            install_spec = Some(spec);
                                        }
                                    }
                                });
                                if let Some(description) = entry.get("description").and_then(serde_json::Value::as_str) {
                                    ui.small(description);
                                }
                                ui.separator();
                            }
                        });
                }
                if let Some(spec) = install_spec {
                    self.install_market_entry(spec);
                }
            } else if self.show_skills {
                ui.horizontal(|ui| {
                    ui.heading("Skills");
                    if ui
                        .add_enabled(!self.skills_loading, egui::Button::new("Refresh"))
                        .clicked()
                    {
                        self.refresh_skills();
                    }
                    if self.skills_loading {
                        ui.spinner();
                    }
                });
                if self.selected_session.is_none() {
                    ui.label("Select a session to list the skills its composer can invoke.");
                }
                if let Some(error) = &self.skills_error {
                    ui.colored_label(egui::Color32::LIGHT_RED, error);
                }
                if let Some(skills) = &self.skills {
                    egui::ScrollArea::vertical()
                        .id_salt("skill-list")
                        .max_height(420.0)
                        .show(ui, |ui| {
                            if skills.skills.is_empty() {
                                ui.label("No skills are available to this session.");
                            }
                            for skill in &skills.skills {
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(&skill.name).strong(),
                                    )
                                    .wrap(),
                                );
                                ui.add(
                                    egui::Label::new(&skill.description)
                                        .wrap()
                                        .selectable(true),
                                );
                                if let Some(when) = &skill.when_to_use {
                                    ui.add(
                                        egui::Label::new(
                                            egui::RichText::new(format!("When: {when}"))
                                                .color(ui.visuals().weak_text_color()),
                                        )
                                        .wrap(),
                                    );
                                }
                                if !skill.model_invocable {
                                    ui.small("Not model-invocable");
                                }
                                if let Some(path) = &skill.path {
                                    ui.small(path);
                                }
                                ui.separator();
                            }
                        });
                } else if !self.skills_loading {
                    ui.label("Open this tab with a session selected to load skills.");
                }
            } else if self.show_document {
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.preview_path)
                            .hint_text("Host file path")
                            .desired_width(360.0),
                    );
                    if ui
                        .add_enabled(!self.preview_loading, egui::Button::new("Preview"))
                        .clicked()
                    {
                        if Self::is_office_path(&self.preview_path) {
                            self.read_office_document();
                        } else {
                            self.read_document();
                        }
                    }
                    if Self::is_office_path(&self.preview_path)
                        && ui
                            .add_enabled(!self.preview_loading, egui::Button::new("Render Office preview"))
                            .clicked()
                    {
                        self.read_office_document();
                    }
                });
                if self.preview_image.is_some() {
                    self.image_preview.ui(
                        ui,
                        self.preview_image.as_ref(),
                        self.preview_loading,
                        self.preview_error.as_deref(),
                    );
                } else {
                    self.document_preview.ui(
                        ui,
                        self.preview_document.as_ref(),
                        self.preview_loading,
                        self.preview_error.as_deref(),
                    );
                }
            } else if self.selected_session.is_none() {
                ui.heading("Welcome");
                if let Some(url) = &self.host_url {
                    ui.label(format!("Host: {url}"));
                }
                if self.transport.is_none()
                    && self.host.as_ref().and_then(Host::launch_url).is_some()
                    && ui.button("Connect").clicked()
                {
                    self.connect();
                }
                ui.label(&self.status);
            } else {
                ui.heading(self.selected_session.as_deref().unwrap_or("Session"));
                if self.page_loading {
                    ui.label("Loading session…");
                } else if let Some(error) = &self.page_error {
                    ui.colored_label(egui::Color32::LIGHT_RED, error);
                } else if let Some(page) = &self.session_page {
                    if page.records.is_empty() {
                        ui.label("This session has no events");
                    } else {
                        let calls = SessionViewer::tool_calls(page);
                        egui::ScrollArea::vertical().show(ui, |ui| {
                            for record in &page.records {
                                let event = &record.event;
                                ui.horizontal_wrapped(|ui| {
                                    ui.strong(format!("{}", event.seq));
                                    ui.label(&event.event_type);
                                    ui.label(event_text(&event.data));
                                });
                                ui.separator();
                            }
                            if !calls.is_empty() {
                                ui.heading("Tool calls");
                                for call in calls {
                                    ui.label(format!(
                                        "{} — {} ({})",
                                        call.name,
                                        call.call_id,
                                        match call.status {
                                            ToolCallStatus::Pending => "pending",
                                            ToolCallStatus::Succeeded => "succeeded",
                                            ToolCallStatus::Failed => "failed",
                                            ToolCallStatus::Unknown => "unknown",
                                        }
                                    ));
                                }
                            }
                        });
                        if page.has_more {
                            ui.label("More events are available");
                        }
                    }
                } else {
                    ui.label("Select a session to view its events");
                }
                ui.separator();
                ui.heading("Chat");
                ui.horizontal_wrapped(|ui| {
                    let selected = self.model_selection.as_ref().map_or_else(
                        || "Model: loading…".to_owned(),
                        |selection| format!("{} / {}", selection.provider, selection.model),
                    );
                    egui::ComboBox::from_id_salt("model-selection")
                        .selected_text(selected)
                        .show_ui(ui, |ui| {
                            let mut next = None;
                            if let Some(catalog) = &self.model_catalog {
                                for group in &catalog.groups {
                                    ui.label(egui::RichText::new(&group.name).strong());
                                    for model in &group.models {
                                        let selection = ModelSelection {
                                            provider: group.id.clone(),
                                            model: model.id.clone(),
                                            reasoning_effort: model
                                                .reasoning
                                                .as_ref()
                                                .and_then(|reasoning| {
                                                    reasoning.default_effort.clone()
                                                }),
                                        };
                                        let current = self
                                            .model_selection
                                            .as_ref()
                                            .is_some_and(|current| {
                                                current.provider == selection.provider
                                                    && current.model == selection.model
                                            });
                                        if ui
                                            .selectable_label(
                                                current,
                                                format!("{} · {}", model.name, model.id),
                                            )
                                            .clicked()
                                        {
                                            next = Some(selection);
                                        }
                                    }
                                }
                            }
                            if let Some(selection) = next {
                                self.select_model(selection);
                            }
                        });
                    if ui
                        .add_enabled(self.model_receiver.is_none(), egui::Button::new("Reload"))
                        .clicked()
                    {
                        self.refresh_models();
                    }
                    let selected_model = match (&self.model_catalog, &self.model_selection) {
                        (Some(catalog), Some(selection)) => {
                            catalog_model(catalog, selection).cloned()
                        }
                        _ => None,
                    };
                    if let (Some(selection), Some(reasoning)) = (
                        self.model_selection.clone(),
                        selected_model.and_then(|model| model.reasoning),
                    ) {
                        let current = selection
                            .reasoning_effort
                            .clone()
                            .or_else(|| reasoning.default_effort.clone())
                            .unwrap_or_else(|| "default".to_owned());
                        let mut next_effort = None;
                        egui::ComboBox::from_id_salt("model-effort")
                            .selected_text(format!("Effort: {current}"))
                            .show_ui(ui, |ui| {
                                for effort in &reasoning.efforts {
                                    if ui
                                        .selectable_label(current == effort.id, &effort.name)
                                        .on_hover_text(
                                            effort.description.as_deref().unwrap_or_default(),
                                        )
                                        .clicked()
                                    {
                                        next_effort = Some(effort.id.clone());
                                    }
                                }
                            });
                        if let Some(effort) = next_effort {
                            let mut next = selection;
                            next.reasoning_effort = Some(effort);
                            self.select_model(next);
                        }
                    }
                });
                if let Some(error) = &self.model_error {
                    ui.colored_label(egui::Color32::LIGHT_RED, error);
                }
                ui.add_enabled(
                    !self.chat_loading,
                    egui::TextEdit::multiline(&mut self.chat_text)
                        .hint_text("Message")
                        .desired_rows(3),
                );
                ui.horizontal_wrapped(|ui| {
                    let uploading = self.attachment_receiver.is_some();
                    if ui
                        .add_enabled(
                            !uploading && !self.chat_loading,
                            egui::Button::new("Attach file…"),
                        )
                        .clicked()
                    {
                        self.pick_attachment();
                    }
                    if !self.attachment_name.is_empty() {
                        ui.label(&self.attachment_name);
                    }
                    if uploading {
                        ui.spinner();
                    }
                    if self.attachment.is_some() && ui.button("Remove").clicked() {
                        self.attachment = None;
                        self.attachment_name.clear();
                    }
                });
                if let Some(error) = &self.attachment_error {
                    ui.colored_label(egui::Color32::LIGHT_RED, error);
                }
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            !self.chat_loading
                                && !self.chat_text.trim().is_empty()
                                && self.attachment_receiver.is_none(),
                            egui::Button::new("Send"),
                        )
                        .clicked()
                    {
                        self.send_prompt();
                    }
                    if ui
                        .add_enabled(
                            self.chat_loading && self.cancel_receiver.is_none(),
                            egui::Button::new("Cancel turn"),
                        )
                        .clicked()
                    {
                        self.cancel_turn();
                    }
                });
                let mut feedback_actions = Vec::new();
                if let Some(status) = &self.feedback_status {
                    ui.label(status);
                }
                let feedback: std::collections::BTreeMap<String, String> = self
                    .feedback_items
                    .iter()
                    .map(|item| (item.message_id.clone(), item.rating.clone()))
                    .collect();
                render_transcript(
                    ui,
                    &self.transcript,
                    &self.assistant_draft,
                    &feedback,
                    self.chat_loading,
                    self.assistant_streaming,
                    &mut feedback_actions,
                );
                for action in feedback_actions {
                    let if_version = self
                        .feedback_items
                        .iter()
                        .find(|item| item.message_id == action.0)
                        .map(|item| item.version.clone());
                    let rating = action.1.clone();
                    let request = MessageFeedbackPutRequest {
                        session_id: self.selected_session.clone().unwrap_or_default(),
                        message_id: action.0,
                        rating: rating.clone(),
                        note: None,
                        category: None,
                        if_version,
                    };
                    let Some(transport) = self.transport.clone() else { continue };
                    let (sender, receiver) = mpsc::channel();
                    std::thread::spawn(move || {
                        let _ = sender.send(
                            MessageFeedbackClient::new(&transport).put(request),
                        );
                    });
                    self.feedback_put_receiver = Some(receiver);
                    self.feedback_status = Some(format!("Rating {}…", rating));
                }
                if let Some(card) = self.pending_questions.first_mut() {
                    let action = card.render(ui);
                    if let Some(action) = action {
                        if let Some(commands) = &self.approval_commands {
                            if let Some(pending) = card.pending().cloned() {
                                let command = match action {
                                    QuestionAction::Answer => ApprovalCommand::AnswerQuestion(
                                        pending,
                                        card.answer(),
                                    ),
                                    QuestionAction::Skip => {
                                        ApprovalCommand::AnswerQuestion(pending, card.skip())
                                    }
                                };
                                let _ = commands.send(command);
                                self.pending_questions.remove(0);
                            }
                        }
                    }
                }
                if let Some(pending) = self.pending_approvals.first().cloned() {
                    ui.separator();
                    ui.heading(format!("Permission: {}", pending.request.tool_name));
                    if let Some(reason) = &pending.request.reason {
                        ui.label(reason);
                    }
                    if let Some(commands) = &self.approval_commands {
                        let mut decision = None;
                        ui.horizontal(|ui| {
                            if ui.button("Allow once").clicked() {
                                decision = Some(ApprovalDecision::AllowedOnce);
                            }
                            if ui.button("Reject").clicked() {
                                decision = Some(ApprovalDecision::Rejected);
                            }
                        });
                        if let Some(decision) = decision {
                            let event_id = pending.invocation.event_id.clone();
                            let _ = commands.send(ApprovalCommand::Answer(pending, decision));
                            self.pending_approvals
                                .retain(|item| item.invocation.event_id != event_id);
                        }
                    }
                }
            }
        });
    }
}
