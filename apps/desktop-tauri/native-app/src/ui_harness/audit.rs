//! Headless UI audit for the shipped egui shell.
//!
//! The audit drives `egui_kittest` over the same `DesktopApp::tick`/`render`
//! pair the window runs. Geometry and text facts come from AccessKit and the
//! painted shape list, never from screenshot diffs or pointer coordinates.
//! Results are written to `target/dsh-kittest/ui-audit.json`; representative
//! frames are exported under `target/dsh-kittest/audit/` for visual review.

use super::*;
use egui::accesskit::{Action, Role};
use egui_kittest::kittest::{NodeT, Queryable};

/// One geometry viewport and raster scale the audit exercises.
#[derive(Clone, Copy)]
struct Viewport {
    width: f32,
    height: f32,
    scale: f32,
}

/// One UI state the audit renders at several viewports.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Scenario {
    Default,
    ManySessions,
    LongTranscript,
    Approval,
    ExplorerTerminal,
    CjkVietnamese,
    HostOffline,
}

impl Scenario {
    fn name(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::ManySessions => "many-sessions",
            Self::LongTranscript => "long-transcript",
            Self::Approval => "approval",
            Self::ExplorerTerminal => "explorer-terminal",
            Self::CjkVietnamese => "cjk-vietnamese",
            Self::HostOffline => "host-offline",
        }
    }

    fn viewports(self) -> &'static [Viewport] {
        match self {
            Self::Default => &[
                Viewport {
                    width: 1024.0,
                    height: 768.0,
                    scale: 1.0,
                },
                Viewport {
                    width: 1280.0,
                    height: 860.0,
                    scale: 1.0,
                },
                Viewport {
                    width: 1920.0,
                    height: 1080.0,
                    scale: 1.5,
                },
            ],
            Self::ManySessions | Self::LongTranscript => &[
                Viewport {
                    width: 1024.0,
                    height: 768.0,
                    scale: 1.0,
                },
                Viewport {
                    width: 1440.0,
                    height: 900.0,
                    scale: 1.25,
                },
                Viewport {
                    width: 1920.0,
                    height: 1080.0,
                    scale: 2.0,
                },
            ],
            Self::Approval => &[
                Viewport {
                    width: 1024.0,
                    height: 768.0,
                    scale: 1.0,
                },
                Viewport {
                    width: 1280.0,
                    height: 860.0,
                    scale: 1.25,
                },
                Viewport {
                    width: 1920.0,
                    height: 1080.0,
                    scale: 2.0,
                },
            ],
            Self::ExplorerTerminal => &[
                Viewport {
                    width: 1024.0,
                    height: 768.0,
                    scale: 1.0,
                },
                Viewport {
                    width: 1440.0,
                    height: 900.0,
                    scale: 1.5,
                },
                Viewport {
                    width: 1920.0,
                    height: 1080.0,
                    scale: 1.5,
                },
            ],
            Self::CjkVietnamese => &[
                Viewport {
                    width: 1024.0,
                    height: 768.0,
                    scale: 1.0,
                },
                Viewport {
                    width: 1280.0,
                    height: 860.0,
                    scale: 1.5,
                },
                Viewport {
                    width: 1920.0,
                    height: 1080.0,
                    scale: 2.0,
                },
            ],
            Self::HostOffline => &[
                Viewport {
                    width: 1024.0,
                    height: 768.0,
                    scale: 1.0,
                },
                Viewport {
                    width: 1280.0,
                    height: 860.0,
                    scale: 1.5,
                },
            ],
        }
    }

    /// Smallest and largest frames are exported for visual review.
    fn exports_review(self, viewport: Viewport) -> bool {
        let area = |candidate: Viewport| candidate.width * candidate.height;
        let smallest = self
            .viewports()
            .iter()
            .map(|candidate| area(*candidate))
            .min_by(f32::total_cmp)
            .expect("scenario viewports");
        let largest = self
            .viewports()
            .iter()
            .map(|candidate| area(*candidate))
            .max_by(f32::total_cmp)
            .expect("scenario viewports");
        area(viewport) == smallest || area(viewport) == largest
    }
}

/// One AccessKit node reduced to the facts the audit checks.
struct NodeInfo {
    role: Role,
    label: String,
    rect: egui::Rect,
    disabled: bool,
    click_action: bool,
    text_input: bool,
}

impl NodeInfo {
    fn actionable(&self) -> bool {
        self.click_action || self.text_input
    }
}

/// One control that must exist in a scenario.
#[derive(Clone)]
struct ControlSpec {
    label: &'static str,
    min_size: (f32, f32),
    enabled: Option<bool>,
}

const NAV_LABELS: [&str; 8] = [
    "Sessions",
    "Explorer",
    "Document",
    "Plugins",
    "Skills",
    "Marketplace",
    "Goals",
    "Settings",
];

const NAV_SPECS: [ControlSpec; 8] = [
    nav("Sessions"),
    nav("Explorer"),
    nav("Document"),
    nav("Plugins"),
    nav("Skills"),
    nav("Marketplace"),
    nav("Goals"),
    nav("Settings"),
];

const fn nav(label: &'static str) -> ControlSpec {
    ControlSpec {
        label,
        min_size: (40.0, 40.0),
        enabled: Some(true),
    }
}

fn control(label: &'static str, min_size: (f32, f32), enabled: Option<bool>) -> ControlSpec {
    ControlSpec {
        label,
        min_size,
        enabled,
    }
}

/// Controls every online Session shell exposes, before state-specific ones.
fn base_controls(online: bool, session_selected: bool) -> Vec<ControlSpec> {
    let mut controls: Vec<ControlSpec> = NAV_SPECS.to_vec();
    controls.push(control("New Session", (100.0, 30.0), Some(online)));
    controls.push(control("Search sessions", (120.0, 18.0), Some(online)));
    controls.push(control("Refresh sessions", (20.0, 14.0), Some(true)));
    controls.push(control("File actions", (60.0, 18.0), Some(true)));
    if session_selected {
        controls.push(control("Composer", (200.0, 40.0), Some(online)));
        controls.push(control("Send", (40.0, 20.0), Some(false)));
        controls.push(control("Cancel turn", (50.0, 20.0), Some(false)));
        controls.push(control("Attach file…", (60.0, 18.0), Some(online)));
        controls.push(control("Model", (100.0, 20.0), Some(online)));
        controls.push(control("Terminal", (24.0, 20.0), Some(true)));
        controls.push(control("Jobs", (24.0, 20.0), Some(true)));
        controls.push(control("Schedules", (24.0, 20.0), Some(true)));
        controls.push(control("Deliverables", (24.0, 20.0), Some(true)));
    }
    controls
}

fn scenario_controls(scenario: Scenario) -> Vec<ControlSpec> {
    let mut controls = match scenario {
        Scenario::HostOffline => base_controls(false, false),
        Scenario::Default
        | Scenario::ManySessions
        | Scenario::LongTranscript
        | Scenario::CjkVietnamese => base_controls(true, true),
        Scenario::Approval => base_controls(true, true),
        Scenario::ExplorerTerminal => {
            let mut controls: Vec<ControlSpec> = NAV_SPECS.to_vec();
            controls.push(control("Terminal", (24.0, 20.0), Some(true)));
            controls.push(control("Jobs", (24.0, 20.0), Some(true)));
            controls.push(control("Schedules", (24.0, 20.0), Some(true)));
            controls.push(control("Deliverables", (24.0, 20.0), Some(true)));
            controls
        }
    };
    match scenario {
        Scenario::Approval => {
            controls.push(control("Allow once", (60.0, 20.0), Some(true)));
            controls.push(control("Reject", (45.0, 20.0), Some(true)));
            controls.push(control("Delegate", (60.0, 20.0), Some(true)));
        }
        Scenario::ExplorerTerminal => {
            controls.push(control("Back to Session", (90.0, 20.0), Some(true)));
            controls.push(control("Refresh workspace", (20.0, 14.0), Some(true)));
            controls.push(control("Close Terminal panel", (20.0, 14.0), Some(true)));
            controls.push(control("Terminal command", (200.0, 30.0), Some(true)));
            controls.push(control("Send exact text", (80.0, 20.0), Some(false)));
            controls.push(control("Resize", (50.0, 20.0), Some(false)));
        }
        Scenario::HostOffline => {
            controls.push(control("Retry connection", (100.0, 20.0), Some(true)));
        }
        Scenario::CjkVietnamese => {
            // The composer accepts the text; Send stays disabled until typed.
        }
        _ => {}
    }
    controls
}

/// One named region assembled from its anchor widgets.
struct RegionSpec {
    name: &'static str,
    anchors: &'static [&'static str],
    min_size: (f32, f32),
}

fn scenario_regions(scenario: Scenario) -> Vec<RegionSpec> {
    let mut regions = vec![RegionSpec {
        name: "nav-rail",
        anchors: &NAV_LABELS,
        min_size: (44.0, 300.0),
    }];
    if scenario != Scenario::ExplorerTerminal {
        regions.push(RegionSpec {
            name: "session-sidebar",
            anchors: &[
                "New Session",
                "Search sessions",
                "File actions",
                "DeepSeek Harness",
            ],
            min_size: (200.0, 300.0),
        });
        regions.push(RegionSpec {
            name: "conversation",
            anchors: &["Composer", "Send", "Model", "Attach file…"],
            min_size: (300.0, 90.0),
        });
        regions.push(RegionSpec {
            name: "status-footer",
            anchors: &["File actions", "DeepSeek Harness"],
            min_size: (100.0, 30.0),
        });
    }
    match scenario {
        Scenario::Approval => {
            regions.push(RegionSpec {
                name: "approval-card",
                anchors: &["Allow once", "Reject", "Delegate"],
                min_size: (170.0, 20.0),
            });
        }
        Scenario::ExplorerTerminal => {
            regions.push(RegionSpec {
                name: "workspace-explorer",
                anchors: &[
                    "Back to Session",
                    "Workspace Explorer",
                    "Refresh workspace",
                    "REPOSITORY TREE",
                ],
                min_size: (200.0, 80.0),
            });
            regions.push(RegionSpec {
                name: "terminal-panel",
                anchors: &["Terminal screen", "Terminal command", "Send exact text"],
                min_size: (300.0, 100.0),
            });
        }
        Scenario::HostOffline => {
            regions.retain(|region| region.name != "conversation");
            regions.push(RegionSpec {
                name: "host-unavailable",
                anchors: &["The Host isn’t reachable", "Retry connection"],
                min_size: (180.0, 60.0),
            });
        }
        _ => {}
    }
    regions
}

/// One run's diagnostics and exported frame path.
#[derive(Default)]
struct RunReport {
    diagnostics: Vec<Value>,
    screenshot: Option<String>,
}

impl RunReport {
    fn error(&mut self, kind: &str, subject: &str, message: String) {
        self.diagnostics.push(serde_json::json!({
            "kind": kind,
            "severity": "error",
            "subject": subject,
            "message": message,
        }));
    }

    fn warning(&mut self, kind: &str, subject: &str, message: String) {
        self.diagnostics.push(serde_json::json!({
            "kind": kind,
            "severity": "warning",
            "subject": subject,
            "message": message,
        }));
    }

    fn has_errors(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|diagnostic| diagnostic["severity"] == "error")
    }
}

fn audit_harness(app: DesktopApp, viewport: Viewport) -> Harness<'static, DesktopApp> {
    Harness::builder()
        .with_size(egui::vec2(viewport.width, viewport.height))
        .with_pixels_per_point(viewport.scale)
        .build_ui_state(
            |ui, app: &mut DesktopApp| {
                app.tick(ui.ctx());
                app.render(ui);
            },
            app,
        )
}

fn collect_nodes<'a>(node: &egui_kittest::Node<'a>, out: &mut Vec<NodeInfo>) {
    let accesskit = node.accesskit_node();
    if accesskit.bounding_box().is_some() {
        out.push(NodeInfo {
            role: accesskit.role(),
            label: accesskit
                .label()
                .or_else(|| accesskit.value())
                .unwrap_or_default(),
            rect: node.rect(),
            disabled: accesskit.is_disabled(),
            click_action: accesskit.data().supports_action(Action::Click),
            text_input: accesskit.is_text_input(),
        });
    }
    for child in node.children() {
        collect_nodes(&child, out);
    }
}

fn find_node<'a>(nodes: &'a [NodeInfo], label: &str) -> Option<&'a NodeInfo> {
    nodes
        .iter()
        .filter(|node| node.label == label)
        .find(|node| {
            matches!(
                node.role,
                Role::Button
                    | Role::TextInput
                    | Role::ComboBox
                    | Role::SpinButton
                    | Role::CheckBox
                    | Role::RadioButton
            )
        })
}

fn find_any_node<'a>(nodes: &'a [NodeInfo], label: &str) -> Option<&'a NodeInfo> {
    nodes.iter().find(|node| node.label == label)
}

fn rect_json(rect: egui::Rect) -> Value {
    serde_json::json!([rect.min.x, rect.min.y, rect.width(), rect.height()])
}

/// Painted text that was elided or clipped outside its clip rectangle.
struct PaintedText {
    text: String,
    rect: egui::Rect,
    clip: egui::Rect,
    elided: bool,
    rows: usize,
}

fn collect_text(shape: &egui::Shape, clip: egui::Rect, out: &mut Vec<PaintedText>) {
    match shape {
        egui::Shape::Text(text) => out.push(PaintedText {
            text: text.galley.job.text.clone(),
            rect: text.galley.rect.translate(text.pos.to_vec2()),
            clip,
            elided: text.galley.elided,
            rows: text.galley.rows.len(),
        }),
        egui::Shape::Vec(shapes) => {
            for shape in shapes {
                collect_text(shape, clip, out);
            }
        }
        _ => {}
    }
}

fn painted_texts(harness: &Harness<'_, DesktopApp>) -> Vec<PaintedText> {
    let mut texts = Vec::new();
    for clipped in &harness.output().shapes {
        collect_text(&clipped.shape, clipped.clip_rect, &mut texts);
    }
    texts
}

fn matching_text<'a>(texts: &'a [PaintedText], wanted: &str) -> Option<&'a PaintedText> {
    texts.iter().find(|text| text.text == wanted).or_else(|| {
        texts
            .iter()
            .find(|text| text.text.trim() == wanted.trim() && !wanted.trim().is_empty())
    })
}

fn check_text(report: &mut RunReport, texts: &[PaintedText], wanted: &str, required: bool) {
    let Some(text) = matching_text(texts, wanted) else {
        if required {
            report.error(
                "text-missing",
                wanted,
                "expected text was not painted".to_owned(),
            );
        } else {
            report.warning(
                "text-missing",
                wanted,
                "expected text was not painted; it may be off-screen".to_owned(),
            );
        }
        return;
    };
    if text.elided {
        report.error(
            "text-truncation",
            wanted,
            format!("text was elided to fit: {:?}", text.text),
        );
    }
    let tolerance = 0.5;
    let contained = text.rect.min.x >= text.clip.min.x - tolerance
        && text.rect.min.y >= text.clip.min.y - tolerance
        && text.rect.max.x <= text.clip.max.x + tolerance
        && text.rect.max.y <= text.clip.max.y + tolerance;
    if !contained {
        report.error(
            "text-clipping",
            wanted,
            format!(
                "painted text {:?} escaped its clip rectangle: text={rect:?} clip={clip:?}",
                text.text,
                rect = text.rect,
                clip = text.clip
            ),
        );
    }
}

fn check_regions(
    report: &mut RunReport,
    nodes: &[NodeInfo],
    viewport: egui::Rect,
    scenario: Scenario,
) -> Value {
    let mut regions = serde_json::Map::new();
    for spec in scenario_regions(scenario) {
        let found: Vec<&NodeInfo> = spec
            .anchors
            .iter()
            .filter_map(|anchor| find_any_node(nodes, anchor))
            .collect();
        if found.is_empty() {
            report.error(
                "region-missing",
                spec.name,
                format!("no anchor from {:?} was present", spec.anchors),
            );
            continue;
        }
        let rect = found
            .iter()
            .fold(egui::Rect::NOTHING, |rect, node| rect.union(node.rect));
        if rect.width() < spec.min_size.0 || rect.height() < spec.min_size.1 {
            report.error(
                "region-min-size",
                spec.name,
                format!(
                    "region is {:.1}x{:.1}, expected at least {}x{}",
                    rect.width(),
                    rect.height(),
                    spec.min_size.0,
                    spec.min_size.1
                ),
            );
        }
        if !viewport.contains_rect(rect) {
            report.error(
                "region-bounds",
                spec.name,
                format!("region {rect:?} escaped viewport {viewport:?}"),
            );
        }
        regions.insert(
            spec.name.to_owned(),
            serde_json::json!({
                "rect": rect_json(rect),
                "anchors": found.iter().map(|node| node.label.clone()).collect::<Vec<_>>(),
                "minSize": [spec.min_size.0, spec.min_size.1],
            }),
        );
    }
    Value::Object(regions)
}

fn check_controls(
    report: &mut RunReport,
    nodes: &[NodeInfo],
    viewport: egui::Rect,
    scenario: Scenario,
) -> Vec<String> {
    let mut labels = Vec::new();
    for spec in scenario_controls(scenario) {
        let Some(node) = find_node(nodes, spec.label) else {
            report.error(
                "control-missing",
                spec.label,
                "expected interactive control was not exposed through AccessKit".to_owned(),
            );
            continue;
        };
        labels.push(spec.label.to_owned());
        if !node.actionable() {
            report.error(
                "interactivity",
                spec.label,
                format!(
                    "control {:?} exposes neither a click action nor a text input",
                    node.role
                ),
            );
        }
        if let Some(enabled) = spec.enabled {
            if node.disabled == enabled {
                report.error(
                    "interactivity",
                    spec.label,
                    format!(
                        "control is {}, expected {}",
                        if node.disabled { "disabled" } else { "enabled" },
                        if enabled { "enabled" } else { "disabled" }
                    ),
                );
            }
        }
        if node.rect.width() < spec.min_size.0 || node.rect.height() < spec.min_size.1 {
            report.error(
                "control-min-size",
                spec.label,
                format!(
                    "control is {:.1}x{:.1}, expected at least {}x{}",
                    node.rect.width(),
                    node.rect.height(),
                    spec.min_size.0,
                    spec.min_size.1
                ),
            );
        }
        if !viewport.contains_rect(node.rect) {
            report.error(
                "control-bounds",
                spec.label,
                format!(
                    "control rect {rect:?} escaped viewport {viewport:?}",
                    rect = node.rect
                ),
            );
        }
    }
    labels
}

fn check_overlaps(report: &mut RunReport, nodes: &[NodeInfo], labels: &[String]) {
    let present: Vec<&NodeInfo> = labels
        .iter()
        .filter_map(|label| find_node(nodes, label))
        .collect();
    for (index, first) in present.iter().enumerate() {
        for second in present.iter().skip(index + 1) {
            let overlap = first.rect.intersect(second.rect);
            if overlap.width() > 2.0 && overlap.height() > 2.0 {
                report.error(
                    "overlap",
                    &format!("{} / {}", first.label, second.label),
                    format!(
                        "interactive controls overlap by {:.1}x{:.1}",
                        overlap.width(),
                        overlap.height()
                    ),
                );
            }
        }
    }
}

fn check_expected_texts(
    report: &mut RunReport,
    texts: &[PaintedText],
    scenario: Scenario,
    approval_reason: Option<&str>,
) {
    for label in NAV_LABELS {
        if let Some(text) = texts
            .iter()
            .find(|text| text.text.starts_with("●\n") && text.text.trim_end().ends_with(label))
        {
            if text.rows > 2 {
                report.error(
                    "nav-label-wrap",
                    label,
                    format!(
                        "navigation label wrapped across {} text rows",
                        text.rows - 1
                    ),
                );
            }
        }
    }
    for wanted in [
        "Sessions",
        "＋ New session",
        "Message (Ctrl+Enter to send)",
        "File actions",
        "Ctrl+Enter · Shift+Enter for a new line",
    ] {
        check_text(report, texts, wanted, false);
    }
    if let Some(reason) = approval_reason {
        check_text(report, texts, reason, true);
    }
    match scenario {
        Scenario::ExplorerTerminal => {
            check_text(
                report,
                texts,
                "Exact text to write; include a newline to execute",
                false,
            );
        }
        Scenario::HostOffline => {
            check_text(report, texts, "The Host isn’t reachable", false);
        }
        Scenario::LongTranscript => {
            check_text(
                report,
                texts,
                "row 300: Tiếng Việt có dấu và 中文内容 with a long tail that must wrap rather than be cut",
                true,
            );
        }
        _ => {}
    }
}

fn write_screenshot(
    harness: &mut Harness<'_, DesktopApp>,
    scenario: Scenario,
    viewport: Viewport,
) -> Option<String> {
    let image = harness.render().ok()?;
    let relative = format!(
        "target/dsh-kittest/audit/{}-{}x{}-at-{}.png",
        scenario.name(),
        viewport.width,
        viewport.height,
        viewport.scale
    );
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(&relative);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok()?;
    }
    image.save(&path).ok()?;
    Some(relative)
}

fn functional_check(
    report: &mut RunReport,
    harness: &mut Harness<'_, DesktopApp>,
    scenario: Scenario,
    approval_commands: Option<&mut mpsc::Receiver<ApprovalCommand>>,
) {
    match scenario {
        Scenario::CjkVietnamese => {
            let composer = harness.get_by_label("Composer");
            composer.focus();
            composer.type_text("Tiếng Việt 中文");
            harness.run_steps(3);
            if !harness.state().chat_text.contains("Tiếng Việt 中文") {
                report.error(
                    "interactivity",
                    "Composer",
                    format!(
                        "typed CJK/Vietnamese text was not retained: {:?}",
                        harness.state().chat_text
                    ),
                );
            }
            let send = harness.query_by_label("Send").expect("Send after typing");
            if send.accesskit_node().is_disabled() {
                report.error(
                    "interactivity",
                    "Send",
                    "Send stayed disabled after the composer received text".to_owned(),
                );
            }
            harness.run_steps(2);
            check_text(report, &painted_texts(harness), "Tiếng Việt 中文", true);
        }
        Scenario::Approval => {
            if let Some(receiver) = approval_commands {
                harness.get_by_label("Reject").click_accesskit();
                harness.run_steps(3);
                if !harness.state().pending_approvals.is_empty() {
                    report.error(
                        "interactivity",
                        "Reject",
                        "the approval card remained after Reject".to_owned(),
                    );
                }
                match receiver.try_recv() {
                    Ok(ApprovalCommand::Answer(_, decision)) => {
                        if decision != ApprovalDecision::Rejected {
                            report.error(
                                "interactivity",
                                "Reject",
                                format!("Reject sent {decision:?}"),
                            );
                        }
                    }
                    _ => report.error(
                        "interactivity",
                        "Reject",
                        "Reject did not send an approval decision".to_owned(),
                    ),
                }
            }
        }
        Scenario::ExplorerTerminal => {
            harness
                .get_by_label("Close Terminal panel")
                .click_accesskit();
            harness.run_steps(3);
            if harness.state().bottom_tool.is_some() {
                report.error(
                    "interactivity",
                    "Close Terminal panel",
                    "the terminal panel stayed open".to_owned(),
                );
            }
            harness.get_by_label("Terminal").click_accesskit();
            harness.run_steps(4);
            if !harness.query_by_label("Terminal screen").is_some() {
                report.error(
                    "interactivity",
                    "Terminal",
                    "reopening the terminal panel did not restore its screen".to_owned(),
                );
            }
        }
        _ => {}
    }
}

fn build_app(
    host: &FixtureHost,
    scenario: Scenario,
) -> (
    DesktopApp,
    mpsc::Sender<Value>,
    Option<mpsc::Receiver<ApprovalCommand>>,
) {
    match scenario {
        Scenario::ManySessions => {
            for index in 0..40 {
                host.set_page(
                    &format!("session-{index:02}"),
                    vec![user_record(1, "session row")],
                );
            }
            let (mut app, frames) = fixture(host);
            app.selected_session = Some("session-00".to_owned());
            (app, frames, None)
        }
        Scenario::LongTranscript => {
            host.set_page_limit(5000);
            host.set_page(
                "long-session",
                (1..=300)
                    .map(|seq| {
                        user_record(
                            seq,
                            &format!(
                                "row {seq}: Tiếng Việt có dấu và 中文内容 with a long tail that must wrap rather than be cut"
                            ),
                        )
                    })
                    .collect(),
            );
            let (mut app, frames) = fixture(host);
            app.selected_session = Some("long-session".to_owned());
            (app, frames, None)
        }
        Scenario::Approval => {
            host.set_page("fixture-session", vec![user_record(1, "approval")]);
            let (mut app, frames) = fixture(host);
            app.selected_session = Some("fixture-session".to_owned());
            let (events, event_receiver) = mpsc::channel();
            app.approval_events_receiver = Some(event_receiver);
            let mut request = serde_json::Map::new();
            request.insert("toolName".to_owned(), serde_json::json!("shell"));
            request.insert(
                "reason".to_owned(),
                serde_json::json!(
                    "run the fixture command with a long enough explanation to wrap across multiple lines in a narrow panel"
                ),
            );
            events
                .send(Ok(RemoteEvent::Waterfall(RemoteEventInvocation::for_test(
                    "approval/request",
                    request,
                ))))
                .expect("approval event");
            std::mem::forget(events);
            let (commands, receiver) = mpsc::channel();
            app.approval_commands = Some(commands);
            (app, frames, Some(receiver))
        }
        Scenario::ExplorerTerminal => {
            host.set_page("explorer-session", Vec::new());
            let (mut app, frames) = fixture(host);
            app.selected_session = Some("explorer-session".to_owned());
            app.active_view = ActiveView::Explorer;
            app.explorer_fullscreen = true;
            app.bottom_tool = Some(BottomTool::Terminal);
            (app, frames, None)
        }
        Scenario::CjkVietnamese => {
            host.set_page(
                "fixture-session",
                vec![
                    user_record(1, "Xin chào, tiếng Việt có dấu"),
                    assistant_record(2, "中文回复：界面布局正常"),
                ],
            );
            let (mut app, frames) = fixture(host);
            app.selected_session = Some("fixture-session".to_owned());
            app.chat_text.clear();
            (app, frames, None)
        }
        Scenario::HostOffline => {
            let (mut app, frames) = fixture(host);
            app.transport = None;
            app.streams = None;
            app.sessions.clear();
            app.host = None;
            (app, frames, None)
        }
        Scenario::Default => {
            host.set_page("fixture-session", vec![user_record(1, "layout")]);
            let (mut app, frames) = fixture(host);
            app.selected_session = Some("fixture-session".to_owned());
            (app, frames, None)
        }
    }
}

#[test]
fn headless_ui_audit_matrix() {
    let audit_directory =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/dsh-kittest/audit");
    let _ = std::fs::remove_dir_all(&audit_directory);
    std::fs::create_dir_all(&audit_directory).expect("audit image directory");
    let mut runs = Vec::new();
    let mut layout_failures = 0_usize;
    let mut interaction_failures = 0_usize;
    let mut review_images = Vec::new();

    for scenario in [
        Scenario::Default,
        Scenario::ManySessions,
        Scenario::LongTranscript,
        Scenario::Approval,
        Scenario::ExplorerTerminal,
        Scenario::CjkVietnamese,
        Scenario::HostOffline,
    ] {
        for viewport in scenario.viewports() {
            let host = FixtureHost::start();
            let (app, frames, mut approval_receiver) = build_app(&host, scenario);
            let mut harness = audit_harness(app, *viewport);
            match scenario {
                Scenario::LongTranscript => {
                    open_session(&mut harness, &frames, "long-session", Vec::new(), 200);
                }
                Scenario::CjkVietnamese => {
                    open_session(&mut harness, &frames, "fixture-session", Vec::new(), 2);
                }
                _ => {}
            }
            harness.run_steps(6);
            let viewport_rect = egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(viewport.width, viewport.height),
            );
            let mut nodes = Vec::new();
            collect_nodes(&harness.root(), &mut nodes);
            let mut report = RunReport::default();

            let regions = check_regions(&mut report, &nodes, viewport_rect, scenario);
            let controls = check_controls(&mut report, &nodes, viewport_rect, scenario);
            check_overlaps(&mut report, &nodes, &controls);
            let texts = painted_texts(&harness);
            let approval_reason = harness
                .state()
                .pending_approvals
                .first()
                .and_then(|pending| pending.request.reason.as_deref())
                .map(str::to_owned);
            check_expected_texts(&mut report, &texts, scenario, approval_reason.as_deref());

            if scenario.exports_review(*viewport) {
                report.screenshot = write_screenshot(&mut harness, scenario, *viewport);
                if let Some(path) = &report.screenshot {
                    review_images.push(path.clone());
                }
            }

            functional_check(
                &mut report,
                &mut harness,
                scenario,
                approval_receiver.as_mut(),
            );

            let layout_error = report.diagnostics.iter().any(|diagnostic| {
                diagnostic["severity"] == "error"
                    && matches!(
                        diagnostic["kind"].as_str(),
                        Some(
                            "region-missing"
                                | "region-min-size"
                                | "region-bounds"
                                | "control-min-size"
                                | "control-bounds"
                                | "overlap"
                                | "text-clipping"
                                | "text-truncation"
                        )
                    )
            });
            let interaction_error = report.diagnostics.iter().any(|diagnostic| {
                diagnostic["severity"] == "error"
                    && matches!(
                        diagnostic["kind"].as_str(),
                        Some("control-missing" | "interactivity" | "text-missing")
                    )
            });
            if layout_error {
                layout_failures += 1;
            }
            if interaction_error {
                interaction_failures += 1;
            }
            runs.push(serde_json::json!({
                "scenario": scenario.name(),
                "viewport": [viewport.width, viewport.height],
                "scale": viewport.scale,
                "status": if report.has_errors() { "fail" } else { "pass" },
                "regions": regions,
                "diagnostics": report.diagnostics,
                "screenshot": report.screenshot,
                "visual": "pending",
            }));
        }
    }

    let report = serde_json::json!({
        "schemaVersion": 1,
        "layout": if layout_failures == 0 { "pass" } else { "fail" },
        "interaction": if interaction_failures == 0 { "pass" } else { "fail" },
        "visual": "pending",
        "summary": {
            "runs": runs.len(),
            "layoutFailures": layout_failures,
            "interactionFailures": interaction_failures,
            "reviewImages": review_images,
        },
        "runs": runs,
    });
    let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/dsh-kittest");
    std::fs::create_dir_all(&directory).expect("audit report directory");
    let path = directory.join("ui-audit.json");
    std::fs::write(
        &path,
        serde_json::to_vec_pretty(&report).expect("audit report"),
    )
    .expect("audit report write");

    let failures: Vec<String> = report["runs"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|run| run["status"] == "fail")
        .map(|run| {
            format!(
                "{} {}x{}@{}: {}",
                run["scenario"].as_str().unwrap_or_default(),
                run["viewport"][0],
                run["viewport"][1],
                run["scale"],
                run["diagnostics"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|diagnostic| diagnostic["severity"] == "error")
                    .map(|diagnostic| format!(
                        "{} {}: {}",
                        diagnostic["kind"].as_str().unwrap_or_default(),
                        diagnostic["subject"].as_str().unwrap_or_default(),
                        diagnostic["message"].as_str().unwrap_or_default()
                    ))
                    .collect::<Vec<_>>()
                    .join("; ")
            )
        })
        .collect();
    assert!(
        failures.is_empty(),
        "headless UI audit failed; report: {}\n{}",
        path.display(),
        failures.join("\n")
    );
}
