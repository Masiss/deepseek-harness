//! Changes review surface: file grouping, per-side stats, patch parsing, and Git bodies.
//!
//! The panel mirrors the Git review prototype: a Changes header, Diff, Files,
//! and History views, and a commit bar. Every action calls a Host verb and
//! renders only the result the Host reports.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use dsh_client_core::git::{
    GitApplyTarget, GitCommitEntry, GitDiff, GitFile, GitFileSideStats, GitStatus,
};
use gpui::{
    canvas, div, point, prelude::*, px, AnyElement, BorderStyle, Bounds, Context, Corners, Edges,
    Hsla, PathBuilder, Pixels, TextAlign, Window,
};

use super::app::{ChangesTab, DesktopApp, GitCommitAction, GitConfirmation};
use super::render::dialog_shell;
use super::theme::Theme;
use super::widgets;

/// Height of the Changes review header, from the Git prototype (px).
const CHANGES_HEADER_HEIGHT: f32 = 43.0;
/// Height of the Changes view-tab row, from the Git prototype (px).
const CHANGES_TAB_HEIGHT: f32 = 33.0;
/// Height of one changed-file row (px).
const REVIEW_ROW_HEIGHT: f32 = 22.0;
/// Line-number gutter width (px).
const GUTTER_WIDTH: f32 = 30.0;
/// Height of one History row (px).
const HISTORY_ROW_HEIGHT: f32 = 40.0;
/// Height of one row's graph canvas, centred in the row (px).
const HISTORY_GRAPH_HEIGHT: f32 = 34.0;
/// Distance between two lane centres (px).
const HISTORY_LANE_SPACING: f32 = 18.0;
/// Left offset of the first lane centre (px).
const HISTORY_GRAPH_PAD_LEFT: f32 = 8.0;
/// Width the gutter adds after the last lane centre (px).
const HISTORY_GRAPH_TAIL: f32 = 6.0;
/// Graph stroke width (px).
const HISTORY_GRAPH_STROKE: f32 = 1.5;
/// Solid commit-dot radius (px).
const HISTORY_DOT_RADIUS: f32 = 4.1;
/// Outer radius of the HEAD ring (px).
const HISTORY_HEAD_RING_RADIUS: f32 = 5.6;
/// Radius of the filled dot inside the HEAD ring (px).
const HISTORY_HEAD_DOT_RADIUS: f32 = 3.2;
/// Vertical control-point offset of a merge or join curve (px).
const HISTORY_CURVE_CONTROL: f32 = 10.0;
/// Refs shown after a subject before the rest collapse into a count chip.
const HISTORY_VISIBLE_REFS: usize = 4;
/// How many recent commit subjects the commit bar offers.
const RECENT_MESSAGE_LIMIT: usize = 5;

/// Changed paths grouped for the Files tab, in paint order.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct FileGroups<'a> {
    /// Paths with an index change.
    pub(crate) staged: Vec<&'a GitFile>,
    /// Tracked work-tree changes.
    pub(crate) changes: Vec<&'a GitFile>,
    /// Paths Git does not track yet.
    pub(crate) untracked: Vec<&'a GitFile>,
}

impl FileGroups<'_> {
    /// Whether every group is empty.
    pub(crate) fn is_empty(&self) -> bool {
        self.staged.is_empty() && self.changes.is_empty() && self.untracked.is_empty()
    }

    /// Number of distinct changed paths across the groups.
    ///
    /// A path with staged and unstaged content appears in two groups and counts once.
    pub(crate) fn unique_count(&self) -> usize {
        let mut paths: Vec<&str> = self
            .staged
            .iter()
            .chain(&self.changes)
            .chain(&self.untracked)
            .map(|file| file.path.as_str())
            .collect();
        paths.sort_unstable();
        paths.dedup();
        paths.len()
    }
}

/// Whether Git reports the path as untracked.
pub(crate) fn is_untracked(file: &GitFile) -> bool {
    file.status == "untracked"
}

/// Groups changed paths for the Files tab, applying a case-insensitive path filter.
///
/// @param files — paths from the latest `git/status`.
/// @param filter — substring matched against each path; blank keeps every path.
/// @returns the three review groups in paint order.
pub(crate) fn group_files<'a>(files: &'a [GitFile], filter: &str) -> FileGroups<'a> {
    let query = filter.trim().to_lowercase();
    let matches = |file: &GitFile| query.is_empty() || file.path.to_lowercase().contains(&query);
    let mut groups = FileGroups::default();
    for file in files.iter().filter(|file| matches(file)) {
        if is_untracked(file) {
            groups.untracked.push(file);
            continue;
        }
        if file.staged {
            groups.staged.push(file);
        }
        if file.worktree {
            groups.changes.push(file);
        }
    }
    groups
}

/// Line totals the given review side reports for one path.
///
/// The Host omits the side for untracked, ignored, and binary changes, and the
/// two sides are never added together.
/// @param file — one changed path from `git/status`.
/// @param staged — whether the index side is read.
/// @returns that side's totals, or `None` when Git reported no counts.
pub(crate) fn side_stats(file: &GitFile, staged: bool) -> Option<GitFileSideStats> {
    let stats = file.stats.as_ref()?;
    if staged {
        stats.index
    } else {
        stats.worktree
    }
}

/// Totals the commit bar reports for the staged index.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct StagedSummary {
    /// Staged paths, counted once each.
    pub(crate) files: usize,
    /// Staged additions; paths without index counts contribute nothing.
    pub(crate) additions: u64,
    /// Staged deletions; paths without index counts contribute nothing.
    pub(crate) deletions: u64,
}

/// Sums the index side across every staged path.
///
/// @param files — paths from the latest `git/status`.
/// @returns the staged file count and line totals.
pub(crate) fn staged_summary(files: &[GitFile]) -> StagedSummary {
    let mut summary = StagedSummary::default();
    for file in files.iter().filter(|file| file.staged) {
        summary.files += 1;
        if let Some(stats) = side_stats(file, true) {
            summary.additions += stats.additions;
            summary.deletions += stats.deletions;
        }
    }
    summary
}

/// Content signature the Reviewed state compares for one changed path.
///
/// The Host has no per-path content hash verb, so the signature is the status
/// entry: change kind, both side flags, and the per-side counts. A status
/// refresh that changes any of those clears Reviewed.
/// @param file — one changed path from `git/status`.
/// @returns a stable hex digest for that status entry.
pub(crate) fn file_signature(file: &GitFile) -> String {
    let mut hasher = DefaultHasher::new();
    file.path.hash(&mut hasher);
    file.status.hash(&mut hasher);
    file.orig_path.hash(&mut hasher);
    file.staged.hash(&mut hasher);
    file.worktree.hash(&mut hasher);
    for side in [true, false].map(|staged| side_stats(file, staged)) {
        match side {
            Some(stats) => {
                stats.additions.hash(&mut hasher);
                stats.deletions.hash(&mut hasher);
            }
            None => 0u8.hash(&mut hasher),
        }
    }
    format!("{:016x}", hasher.finish())
}

/// Content signature of one loaded patch.
///
/// A destructive hunk action stores this value, so confirmation can refuse when
/// the diff the user saw is no longer the diff on screen.
/// @param path — path the patch belongs to.
/// @param staged — side the patch was read from.
/// @param patch — complete patch text from `git/diff`.
/// @returns a stable hex digest for that patch.
pub(crate) fn patch_signature(path: &str, staged: bool, patch: &str) -> String {
    let mut hasher = DefaultHasher::new();
    path.hash(&mut hasher);
    staged.hash(&mut hasher);
    patch.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// One line of a parsed hunk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DiffLine {
    /// `' '` context, `'+'` addition, `'-'` deletion, or `'\\'` for the
    /// no-newline marker.
    pub(crate) kind: char,
    /// Old-side line number; absent on additions.
    pub(crate) old_line: Option<u32>,
    /// New-side line number; absent on deletions.
    pub(crate) new_line: Option<u32>,
    /// Line content after the mark.
    pub(crate) text: String,
}

/// One `@@` hunk with the lines its header covers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DiffHunk {
    /// The `@@ -a,b +c,d @@` header line.
    pub(crate) header: String,
    /// Hunk lines in patch order.
    pub(crate) lines: Vec<DiffLine>,
}

/// One loaded patch split into its file header and hunks.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ParsedDiff {
    /// Lines before the first hunk; `git/apply` needs them on every hunk patch.
    pub(crate) header: Vec<String>,
    /// Hunks in patch order.
    pub(crate) hunks: Vec<DiffHunk>,
}

impl ParsedDiff {
    /// Builds the patch that applies exactly one hunk.
    ///
    /// @param index — hunk position in {@link ParsedDiff.hunks}.
    /// @returns the file header plus that hunk, or `None` when the index is out of range.
    pub(crate) fn hunk_patch(&self, index: usize) -> Option<String> {
        let hunk = self.hunks.get(index)?;
        let mut patch = String::new();
        for line in &self.header {
            patch.push_str(line);
            patch.push('\n');
        }
        patch.push_str(&hunk.header);
        patch.push('\n');
        for line in &hunk.lines {
            patch.push(line.kind);
            patch.push_str(&line.text);
            patch.push('\n');
        }
        Some(patch)
    }
}

/// Splits a unified patch from `git/diff` into its header and hunks.
///
/// Parsing stops at a second `diff --git` section because the Host returns one
/// path per diff request.
/// @param patch — patch text from `git/diff`.
/// @returns the file header and every hunk with gutter numbers.
pub(crate) fn parse_diff(patch: &str) -> ParsedDiff {
    let text = patch.strip_suffix('\n').unwrap_or(patch);
    let mut parsed = ParsedDiff::default();
    let mut current: Option<DiffHunk> = None;
    let mut in_header = true;
    let mut old_line = 0u32;
    let mut new_line = 0u32;
    for line in text.split('\n') {
        if let Some((old_start, new_start)) = hunk_starts(line) {
            if let Some(hunk) = current.take() {
                parsed.hunks.push(hunk);
            }
            in_header = false;
            old_line = old_start;
            new_line = new_start;
            current = Some(DiffHunk {
                header: line.to_owned(),
                lines: Vec::new(),
            });
            continue;
        }
        if in_header {
            parsed.header.push(line.to_owned());
            continue;
        }
        match current.as_mut() {
            None => {}
            Some(hunk) => match line.chars().next() {
                Some('+') => {
                    hunk.lines.push(DiffLine {
                        kind: '+',
                        old_line: None,
                        new_line: Some(new_line),
                        text: line[1..].to_owned(),
                    });
                    new_line = new_line.saturating_add(1);
                }
                Some('-') => {
                    hunk.lines.push(DiffLine {
                        kind: '-',
                        old_line: Some(old_line),
                        new_line: None,
                        text: line[1..].to_owned(),
                    });
                    old_line = old_line.saturating_add(1);
                }
                Some(' ') => {
                    hunk.lines.push(DiffLine {
                        kind: ' ',
                        old_line: Some(old_line),
                        new_line: Some(new_line),
                        text: line[1..].to_owned(),
                    });
                    old_line = old_line.saturating_add(1);
                    new_line = new_line.saturating_add(1);
                }
                Some('\\') => hunk.lines.push(DiffLine {
                    kind: '\\',
                    old_line: None,
                    new_line: None,
                    text: line[1..].to_owned(),
                }),
                // Any other mark ends the hunk; a second file section is not parsed.
                _ => break,
            },
        }
    }
    if let Some(hunk) = current {
        parsed.hunks.push(hunk);
    }
    parsed
}

/// Reads the old and new start line from one hunk header.
/// @param line — a line of the patch.
/// @returns `(oldStart, newStart)` when the line is an `@@` header.
fn hunk_starts(line: &str) -> Option<(u32, u32)> {
    let rest = line.strip_prefix("@@ -")?;
    let (old, rest) = rest.split_once(' ')?;
    let (new, _) = rest.strip_prefix('+')?.split_once(' ')?;
    Some((start_number(old)?, start_number(new)?))
}

/// Reads a hunk range's start, ignoring its optional count.
/// @param spec — range text such as `12,4`.
/// @returns the start line when it parses as a number.
fn start_number(spec: &str) -> Option<u32> {
    spec.split(',').next()?.parse().ok()
}

/// Whether HEAD is already on its upstream branch.
///
/// @param status — latest `git/status`.
/// @returns whether the branch has an upstream and no unpushed commits.
fn head_is_pushed(status: &GitStatus) -> bool {
    status.upstream.is_some() && status.ahead == 0
}

/// Why Commit is disabled, or `None` when it can run.
/// @param staged_files — staged path count from `git/status`.
/// @param message — commit message draft.
/// @param busy — whether a Git verb is in flight.
/// @param in_operation — whether a merge, rebase, cherry-pick, or revert is open.
/// @returns the first blocking reason.
fn commit_blocker(
    staged_files: usize,
    message: &str,
    busy: bool,
    in_operation: bool,
) -> Option<&'static str> {
    if busy {
        Some("A Git action is running; wait for its result.")
    } else if in_operation {
        Some("Finish the in-progress Git operation before committing.")
    } else if staged_files == 0 {
        Some("Stage at least one change to commit.")
    } else if message.trim().is_empty() {
        Some("Enter a commit message.")
    } else {
        None
    }
}

/// Why Commit & Push is disabled, or `None` when it can run.
/// @param commit — commit blocker from {@link commit_blocker}.
/// @param upstream — upstream ref reported by `git/status`.
/// @param behind — commits the upstream has that the branch lacks.
/// @returns the first blocking reason, naming the remote state.
fn push_blocker(commit: Option<&'static str>, upstream: Option<&str>, behind: u64) -> Option<String> {
    if let Some(reason) = commit {
        return Some(reason.to_owned());
    }
    match upstream {
        None => Some(
            "Publishing a branch needs an upstream; that flow lands in G3.".to_owned(),
        ),
        Some(_) if behind > 0 => Some(format!(
            "The upstream has {behind} newer commit(s); pull before pushing (G3)."
        )),
        Some(_) => None,
    }
}

/// One-line tooltip text for the ahead/behind chip.
/// @param ahead — commits the branch has that the upstream lacks.
/// @param behind — commits the upstream has that the branch lacks.
/// @param upstream — upstream ref, when the branch has one.
/// @returns the chip's description.
fn sync_tooltip(ahead: u64, behind: u64, upstream: Option<&str>) -> String {
    match upstream {
        Some(upstream) => format!("{ahead} ahead · {behind} behind {upstream}"),
        None => format!("{ahead} ahead · {behind} behind · no upstream branch"),
    }
}

/// Right column: review surface for agent changes, backed by Host data only.
pub(super) fn changes_column(
    app: &mut DesktopApp,
    theme: &Theme,
    cx: &mut Context<DesktopApp>,
) -> AnyElement {
    if app.take_git_commit_clear() {
        app.set_input("git-commit-message", "", cx);
        app.git_commit_message.clear();
    }
    let tab = app.changes_tab;
    if tab == ChangesTab::History {
        app.ensure_git_history();
    }
    let commit_panel =
        (tab != ChangesTab::Branches && tab != ChangesTab::History).then(|| commit_bar(app, theme, cx));
    div()
        .flex()
        .flex_col()
        .debug_selector(|| "changes-panel".to_owned())
        .w(theme.sp(
            app.ui_preferences
                .workspace_prefs(app.selected_workspace.as_deref())
                .changes_panel_width
                .unwrap_or(app.ui_preferences.changes_panel_width),
        ))
        .min_w(theme.sp(crate::layout::CHANGES_MIN))
        .max_w(theme.sp(crate::layout::CHANGES_MAX))
        .min_h(px(0.))
        .border_l_1()
        .border_color(theme.border)
        .bg(theme.background)
        .child(review_header(app, theme, cx))
        .child(view_tab_row(app, theme, cx))
        .child(match tab {
            ChangesTab::Branches => branches_body(app, theme, cx),
            ChangesTab::Files => files_body(app, theme, cx),
            ChangesTab::History => history_body(app, theme, cx),
            ChangesTab::Diff => diff_body(app, theme, cx),
        })
        .children(commit_panel)
        .into_any_element()
}

/// Changes header: title, file count, ahead/behind chip, and Review all.
fn review_header(app: &mut DesktopApp, theme: &Theme, cx: &mut Context<DesktopApp>) -> AnyElement {
    let files = app
        .git_status
        .as_ref()
        .map(|status| status.files.clone())
        .unwrap_or_default();
    let count = group_files(&files, "").unique_count();
    let all_reviewed =
        count > 0 && files.iter().all(|file| app.git_is_reviewed(&file.path));
    let sync = app.git_status.as_ref().and_then(|status| {
        (status.ahead > 0 || status.behind > 0).then(|| {
            (
                status.ahead,
                status.behind,
                status.upstream.clone(),
            )
        })
    });
    let mut left = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(theme.sp(6.))
        .child(
            div()
                .text_size(theme.fs(11.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(theme.text)
                .child("Changes"),
        )
        .child(
            div()
                .text_size(theme.fs(9.5))
                .text_color(theme.muted)
                .child(match count {
                    0 => "No changes".to_owned(),
                    1 => "1 file".to_owned(),
                    count => format!("{count} files"),
                }),
        );
    if let Some((ahead, behind, upstream)) = sync {
        let mut chip = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(theme.sp(3.))
            .px(theme.sp(4.))
            .py(px(1.))
            .rounded(theme.corner())
            .bg(theme.surface_raised)
            .text_size(theme.fs(9.))
            .text_color(theme.muted)
            .id("git-sync-chip")
            .tooltip(widgets::HintView::tooltip(
                theme,
                sync_tooltip(ahead, behind, upstream.as_deref()),
            ));
        if ahead > 0 {
            chip = chip.child(
                div()
                    .text_color(theme.accent)
                    .child(format!("↑{ahead}")),
            );
        }
        if behind > 0 {
            chip = chip.child(div().text_color(theme.warn).child(format!("↓{behind}")));
        }
        left = left.child(chip);
    }
    let review_all_enabled = count > 0 && !all_reviewed;
    let mut review_all = action_text(theme, "Review all", "git-review-all".to_owned());
    if review_all_enabled {
        review_all = review_all.on_click(cx.listener(|app, _, _, cx| {
            app.review_all_git_paths();
            cx.notify();
        }));
    } else {
        review_all = review_all.opacity(0.45).tooltip(widgets::HintView::tooltip(
            theme,
            if count == 0 {
                "No changed path to review."
            } else {
                "Every changed path is already reviewed at its current status."
            },
        ));
    }
    div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .h(theme.sp(CHANGES_HEADER_HEIGHT))
        .px(theme.sp(10.))
        .border_b_1()
        .border_color(theme.border)
        .child(left)
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(theme.sp(4.))
                .child(review_all)
                .child(
                    widgets::Button::new(theme, "×")
                        .build()
                        .id("changes-close")
                        .on_click(cx.listener(|app, _, _, cx| {
                            app.show_changes = false;
                            cx.notify();
                        })),
                ),
        )
        .into_any_element()
}

/// Diff, Files, and History views; Branches joins them while it is open.
fn view_tab_row(app: &mut DesktopApp, theme: &Theme, cx: &mut Context<DesktopApp>) -> AnyElement {
    let tab = app.changes_tab;
    let mut row = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(theme.sp(2.))
        .h(theme.sp(CHANGES_TAB_HEIGHT))
        .px(theme.sp(8.))
        .border_b_1()
        .border_color(theme.border)
        .child(view_tab_button(theme, "Diff", ChangesTab::Diff, tab, cx))
        .child(view_tab_button(theme, "Files", ChangesTab::Files, tab, cx))
        .child(view_tab_button(
            theme,
            "History",
            ChangesTab::History,
            tab,
            cx,
        ));
    if tab == ChangesTab::Branches {
        row = row.child(view_tab_button(
            theme,
            "Branches",
            ChangesTab::Branches,
            tab,
            cx,
        ));
    }
    row.into_any_element()
}

/// One view tab; shows selected while `current` matches.
fn view_tab_button(
    theme: &Theme,
    label: &str,
    value: ChangesTab,
    current: ChangesTab,
    cx: &mut Context<DesktopApp>,
) -> AnyElement {
    let selected = current == value;
    let selector = format!("changes-tab-{label}");
    div()
        .px(theme.sp(8.))
        .py(px(2.))
        .rounded(theme.corner())
        .id(gpui::SharedString::from(format!("changes-tab-{label}")))
        .debug_selector(move || selector.clone())
        .cursor_pointer()
        .when(selected, |element| element.bg(theme.selection))
        .hover(|style| style.bg(theme.surface_raised))
        .text_size(theme.fs(10.))
        .text_color(if selected { theme.accent } else { theme.muted })
        .child(label.to_owned())
        .on_click(cx.listener(move |app, _, window, cx| {
            app.changes_tab = value;
            if value == ChangesTab::Branches {
                app.refresh_git_branches();
            }
            if value == ChangesTab::History {
                app.refresh_git_history();
                window.focus(&app.git_history_focus);
            }
            cx.notify();
        }))
        .into_any_element()
}

/// Branch list with inline checkout, backed by the Host `git/branches` verb.
fn branches_body(app: &mut DesktopApp, theme: &Theme, cx: &mut Context<DesktopApp>) -> AnyElement {
    let current = app
        .git_branches
        .as_ref()
        .and_then(|branches| branches.current.clone())
        .or_else(|| app.git_status.as_ref().and_then(|status| status.branch.clone()));
    let push_confirm = app.git_push_confirm;
    let summary = app.git_remote_summary.clone();
    let mut list = div().flex().flex_col().gap(theme.sp(2.));
    if let Some(branches) = app.git_branches.as_ref() {
        if branches.branches.is_empty() {
            list = list.child(
                div()
                    .text_size(theme.fs(10.5))
                    .text_color(theme.muted)
                    .child("This repository has no local branches yet."),
            );
        }
        for branch in &branches.branches {
            let name = branch.name.clone();
            let selected = branch.current;
            let checkout = name.clone();
            let rebase = name.clone();
            let pick = name.clone();
            let merge = name.clone();
            let revert = branch.commit.clone();
            let mut actions = div().flex().flex_row().items_center().gap(theme.sp(4.));
            if !selected {
                actions = actions
                    .child(
                        action_text(theme, "Checkout", format!("git-checkout-{name}"))
                            .on_click(cx.listener(move |app, _, _, cx| {
                                app.checkout_git_branch(checkout.clone());
                                cx.notify();
                            })),
                    )
                    .child(
                        action_text(theme, "Rebase", format!("git-rebase-{name}"))
                            .on_click(cx.listener(move |app, _, _, cx| {
                                app.rebase_git_branch(rebase.clone());
                                cx.notify();
                            })),
                    )
                    .child(
                        action_text(theme, "Pick", format!("git-pick-{name}"))
                            .on_click(cx.listener(move |app, _, _, cx| {
                                app.cherry_pick_git_commit(pick.clone());
                                cx.notify();
                            })),
                    )
                    .child(
                        action_text(theme, "Merge", format!("git-merge-{name}"))
                            .on_click(cx.listener(move |app, _, _, cx| {
                                app.merge_git_branch(merge.clone());
                                cx.notify();
                            })),
                    );
            } else {
                actions = actions.child(
                    action_text(theme, "Revert tip", format!("git-revert-{name}")).on_click(
                        cx.listener(move |app, _, _, cx| {
                            app.revert_git_commit(revert.clone());
                            cx.notify();
                        }),
                    ),
                );
            }
            list = list.child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .gap(theme.sp(4.))
                    .px(theme.sp(6.))
                    .py(px(2.))
                    .rounded(theme.corner())
                    .when(selected, |element| element.bg(theme.selection))
                    .hover(|style| style.bg(theme.surface_raised))
                    .id(gpui::SharedString::from(format!("git-branch-{name}")))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(theme.sp(4.))
                            .child(
                                div()
                                    .w(theme.sp(10.))
                                    .text_color(if selected { theme.accent } else { theme.muted })
                                    .child(if selected { "●" } else { "" }),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_size(theme.fs(10.5))
                                    .text_color(if selected { theme.accent } else { theme.text })
                                    .child(name),
                            )
                            .child(
                                div()
                                    .text_size(theme.fs(9.))
                                    .text_color(theme.muted)
                                    .child(short_commit(&branch.commit).to_owned()),
                            ),
                    )
                    .child(actions),
            );
        }
    }
    let hint = if app.git_loading {
        "Running a Git action…"
    } else if app.git_branches.is_some() {
        "Checkout switches branches; Rebase replays the current branch onto one; Pick cherry-picks its tip; Merge joins one into the current branch; Revert tip undoes the current branch's newest commit."
    } else {
        "Branch list not loaded."
    };
    div()
        .flex()
        .flex_col()
        .gap(theme.sp(6.))
        .p(theme.sp(12.))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_size(theme.fs(11.5))
                        .text_color(theme.text)
                        .child(match current.as_deref() {
                            Some(branch) => format!("On {branch}"),
                            None => "Detached HEAD".to_owned(),
                        }),
                )
                .child(
                    div()
                        .px(theme.sp(6.))
                        .py(px(2.))
                        .rounded(theme.corner())
                        .cursor_pointer()
                        .hover(|style| style.bg(theme.surface_raised))
                        .id("git-branches-refresh")
                        .text_size(theme.fs(10.))
                        .text_color(theme.muted)
                        .child("Refresh")
                        .on_click(cx.listener(|app, _, _, cx| {
                            app.refresh_git_branches();
                            app.refresh_git_state();
                            cx.notify();
                        })),
                ),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(theme.sp(2.))
                .child(action_text(theme, "Fetch", "git-fetch".to_owned()).on_click(
                    cx.listener(|app, _, _, cx| {
                        app.run_git_remote("fetch");
                        cx.notify();
                    }),
                ))
                .child(action_text(theme, "Pull", "git-pull".to_owned()).on_click(
                    cx.listener(|app, _, _, cx| {
                        app.run_git_remote("pull");
                        cx.notify();
                    }),
                ))
                .child(
                    action_text(
                        theme,
                        if push_confirm { "Confirm push" } else { "Push" },
                        "git-push".to_owned(),
                    )
                    .on_click(cx.listener(move |app, _, _, cx| {
                        if app.git_push_confirm {
                            app.run_git_remote("push");
                        } else {
                            app.git_push_confirm = true;
                        }
                        cx.notify();
                    })),
                ),
        )
        .children(operation_banner(app, theme, cx))
        .child(
            div()
                .text_size(theme.fs(9.5))
                .text_color(theme.muted)
                .child(hint),
        )
        .children(summary.map(|summary| {
            div()
                .text_size(theme.fs(10.))
                .text_color(theme.muted)
                .child(summary)
        }))
        .children(app.git_error.as_ref().map(|error| error_line(theme, error)))
        .child(list)
        .into_any_element()
}

/// Colours one History graph paints with, copied out of the theme snapshot.
#[derive(Clone, Copy)]
struct GraphTheme {
    /// First lane colour.
    accent: Hsla,
    /// Second lane colour.
    warn: Hsla,
    /// Third lane colour.
    error: Hsla,
    /// Interface scale the stroke widths and radii follow.
    scale: f32,
}

impl GraphTheme {
    /// Colour of one lane; the three state tokens cycle.
    /// @param lane — lane index.
    /// @returns the lane's colour.
    fn lane(&self, lane: usize) -> Hsla {
        match lane % 3 {
            0 => self.accent,
            1 => self.warn,
            _ => self.error,
        }
    }
}

/// Lane occupancy and curve endpoints one History row paints.
struct GraphPaint {
    /// Lane of the commit's dot.
    lane: usize,
    /// Occupied lanes before the row.
    before: Vec<bool>,
    /// Lanes whose rail curves into the dot.
    joins: Vec<usize>,
    /// Lanes the merge curves reach.
    extra: Vec<usize>,
    /// Whether the commit has a parent, so the current lane continues below.
    has_parents: bool,
    /// Whether HEAD points here and the dot carries a ring.
    head: bool,
}

/// Width of the fixed graph gutter for a lane count.
/// @param lanes — lanes the layout produced.
/// @returns the gutter width in unscaled pixels.
fn graph_width(lanes: usize) -> f32 {
    lanes.max(1) as f32 * HISTORY_LANE_SPACING + HISTORY_GRAPH_TAIL
}

/// History tab: swimlane graph rows and the selected commit's detail.
fn history_body(app: &mut DesktopApp, theme: &Theme, cx: &mut Context<DesktopApp>) -> AnyElement {
    let commits = app.git_history.clone();
    let layout = layout_graph(&commits);
    let selected = app.git_history_selected.clone();
    let local_branches: Vec<String> = app
        .git_branches
        .as_ref()
        .map(|branches| {
            branches
                .branches
                .iter()
                .map(|branch| branch.name.clone())
                .collect()
        })
        .unwrap_or_default();
    let graph_theme = GraphTheme {
        accent: theme.accent,
        warn: theme.warn,
        error: theme.error,
        scale: theme.scale,
    };
    let busy = app.git_loading;
    let mut list = div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.))
        .id("git-history-scroll")
        .debug_selector(|| "git-history-scroll".to_owned())
        .track_scroll(&app.git_history_scroll)
        .overflow_y_scroll()
        .px(theme.sp(8.))
        .py(theme.sp(4.));
    let row_context = HistoryRowContext {
        theme,
        commits: &commits,
        layout: &layout,
        selected: selected.as_deref(),
        local_branches: &local_branches,
        graph: graph_theme,
    };
    for (row_index, row) in layout.rows.iter().enumerate() {
        list = list.child(history_row(&row_context, row, row_index == 0, cx));
    }
    if commits.is_empty() {
        if app.git_history_loading {
            list = list.child(history_note(
                theme,
                "git-history-loading",
                "Reading history…".to_owned(),
                theme.muted,
            ));
        } else if app.git_history_error.is_none() {
            list = list.child(if app.git_history_loaded {
                history_empty(theme)
            } else {
                history_note(
                    theme,
                    "git-history-loading",
                    "Connect a Session to read its history.".to_owned(),
                    theme.muted,
                )
            });
        }
    } else if app.git_history_loading {
        list = list.child(history_note(
            theme,
            "git-history-loading",
            "Reading history…".to_owned(),
            theme.muted,
        ));
    }
    match app.git_history_footer {
        HistoryFooter::LoadOlder { skip } => {
            list = list.child(load_older_row(theme, skip, busy, cx));
        }
        HistoryFooter::Truncated => {
            list = list.child(history_note(
                theme,
                "git-history-truncated",
                "The Host capped this page at its byte limit; older commits are not listed."
                    .to_owned(),
                theme.warn,
            ));
        }
        HistoryFooter::End => {}
    }
    let detail = history_detail(app, theme, &commits, selected.as_deref(), &local_branches, cx);
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.))
        .debug_selector(|| "git-history-panel".to_owned())
        .track_focus(&app.git_history_focus)
        .on_key_down(cx.listener(|app, event: &gpui::KeyDownEvent, _, cx| {
            let delta = match event.keystroke.key.as_str() {
                "down" => 1,
                "up" => -1,
                _ => return,
            };
            app.move_git_history_selection(delta);
            cx.notify();
        }))
        .children(operation_banner(app, theme, cx))
        .children(app.git_error.as_ref().map(|error| error_line(theme, error)))
        .children(app.git_history_error.as_ref().map(|error| {
            history_note(
                theme,
                "git-history-error",
                format!("{error} Choose History again to retry."),
                theme.error,
            )
        }))
        .child(list)
        .children(detail)
        .into_any_element()
}

/// One named note row inside the History list.
/// @param selector — debug selector the harness addresses it by.
/// @param text — message to show.
/// @param color — token the message reads in.
fn history_note(theme: &Theme, selector: &'static str, text: String, color: Hsla) -> AnyElement {
    div()
        .flex_shrink_0()
        .px(theme.sp(6.))
        .py(theme.sp(6.))
        .debug_selector(move || selector.to_owned())
        .text_size(theme.fs(10.))
        .text_color(color)
        .child(text)
        .into_any_element()
}

/// Empty History list for a repository without commits.
fn history_empty(theme: &Theme) -> AnyElement {
    div()
        .debug_selector(|| "git-history-empty".to_owned())
        .child(empty_state(
            theme,
            "No commits",
            "The Host reports no commit in this repository yet.",
        ))
        .into_any_element()
}

/// Footer row that pages to older commits.
/// @param skip — commits to skip from HEAD for the next page.
/// @param busy — whether a Git request already runs for this repository.
fn load_older_row(
    theme: &Theme,
    skip: u32,
    busy: bool,
    cx: &mut Context<DesktopApp>,
) -> AnyElement {
    let mut row = div()
        .flex()
        .flex_row()
        .items_center()
        .justify_center()
        .h(theme.sp(HISTORY_ROW_HEIGHT))
        .flex_shrink_0()
        .rounded(theme.corner())
        .id("git-history-load-older")
        .debug_selector(|| "git-history-load-older".to_owned())
        .text_size(theme.fs(10.))
        .child(if busy {
            "Loading older commits…"
        } else {
            "Load older commits"
        });
    if busy {
        row = row.opacity(0.45).text_color(theme.muted);
    } else {
        row = row
            .cursor_pointer()
            .text_color(theme.accent)
            .hover(|style| style.bg(theme.surface_raised))
            .on_click(cx.listener(move |app, _, _, cx| {
                app.load_git_history_page(skip);
                cx.notify();
            }));
    }
    row.into_any_element()
}

/// Values every History row shares, so one row only adds its own layout.
struct HistoryRowContext<'a> {
    /// Resolved theme snapshot.
    theme: &'a Theme,
    /// Loaded commits the rows index into.
    commits: &'a [GitCommitEntry],
    /// Layout that produced the rows and the gutter width.
    layout: &'a GraphLayout,
    /// oid the list shows as selected.
    selected: Option<&'a str>,
    /// Local branch names used to classify ref chips.
    local_branches: &'a [String],
    /// Colours and scale the graph paints with.
    graph: GraphTheme,
}

/// One 40px History row: graph gutter, subject with refs, author and oid.
/// @param first — whether this is the first row, which tests address directly.
fn history_row(
    context: &HistoryRowContext,
    row: &GraphRow,
    first: bool,
    cx: &mut Context<DesktopApp>,
) -> AnyElement {
    let theme = context.theme;
    let commit = &context.commits[row.index];
    let oid = commit.oid.clone();
    let is_selected = context.selected == Some(commit.oid.as_str());
    let refs = history_refs(&commit.refs, context.local_branches);
    let parents = commit.parents.len();
    let meta = if parents > 1 {
        format!(
            "{} · {} · {parents} parents",
            commit.author_name, commit.author_time
        )
    } else {
        format!("{} · {}", commit.author_name, commit.author_time)
    };
    let mut chips = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(theme.sp(4.))
        .flex_shrink_0();
    for (chip_index, reference) in refs.chips.iter().take(HISTORY_VISIBLE_REFS).enumerate() {
        chips = chips.child(ref_chip(
            theme,
            reference,
            format!("git-history-ref-{}-{chip_index}", row.index),
        ));
    }
    if refs.chips.len() > HISTORY_VISIBLE_REFS {
        let rest = refs
            .chips
            .iter()
            .skip(HISTORY_VISIBLE_REFS)
            .map(|reference| reference.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        chips = chips.child(
            div()
                .px(theme.sp(4.))
                .rounded(px(3.))
                .border_1()
                .border_color(theme.border)
                .id(gpui::SharedString::from(format!(
                    "git-history-refs-{}-more",
                    row.index
                )))
                .text_size(theme.fs(8.5))
                .text_color(theme.muted)
                .tooltip(widgets::HintView::tooltip(theme, rest))
                .child(format!("+{}", refs.chips.len() - HISTORY_VISIBLE_REFS)),
        );
    }
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(theme.sp(6.))
        .h(theme.sp(HISTORY_ROW_HEIGHT))
        .flex_shrink_0()
        .px(theme.sp(2.))
        .rounded(theme.corner())
        .cursor_pointer()
        .id(gpui::SharedString::from(format!("git-history-row-{}", row.index)))
        .hover(|style| style.bg(theme.surface_raised))
        .when(is_selected, |element| element.bg(theme.selection))
        .when(first, |element| {
            element.debug_selector(|| "git-history-row-first".to_owned())
        })
        .when(is_selected, |element| {
            element.debug_selector(|| "git-history-row-selected".to_owned())
        })
        .on_click(cx.listener(move |app, _, window, cx| {
            app.select_git_history(oid.clone());
            window.focus(&app.git_history_focus);
            cx.notify();
        }))
        .child(history_graph(
            theme,
            context.layout.lanes,
            row,
            refs.head,
            !commit.parents.is_empty(),
            context.graph,
        ))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(2.))
                .flex_1()
                .min_w(px(0.))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(theme.sp(5.))
                        .min_w(px(0.))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .truncate()
                                .text_size(theme.fs(12.4))
                                .text_color(theme.text)
                                .when(refs.head, |element| {
                                    element.font_weight(gpui::FontWeight::SEMIBOLD)
                                })
                                .child(commit.subject.clone()),
                        )
                        .child(chips),
                )
                .child(
                    div()
                        .truncate()
                        .text_size(theme.fs(11.))
                        .text_color(theme.muted)
                        .child(meta),
                ),
        )
        .child(
            div()
                .flex_shrink_0()
                .text_size(theme.fs(9.5))
                .text_color(theme.muted)
                .child(short_commit(&commit.oid).to_owned()),
        )
        .into_any_element()
}

/// Ref chip for one decoration.
/// @param id — element id, unique within the loaded page.
fn ref_chip(theme: &Theme, reference: &HistoryRef, id: String) -> AnyElement {
    let (color, border, hint) = match reference.kind {
        RefKind::Branch => (
            theme.accent,
            theme.accent.opacity(0.45),
            "Local branch",
        ),
        RefKind::Remote => (theme.muted, theme.border, "Remote branch"),
        RefKind::Tag => (theme.warn, theme.warn.opacity(0.45), "Tag"),
    };
    div()
        .px(theme.sp(4.))
        .rounded(px(3.))
        .border_1()
        .border_color(border)
        .id(gpui::SharedString::from(id))
        .text_size(theme.fs(9.))
        .text_color(color)
        .tooltip(widgets::HintView::tooltip(theme, hint))
        .child(reference.name.clone())
        .into_any_element()
}

/// The graph gutter of one History row.
fn history_graph(
    theme: &Theme,
    lanes: usize,
    row: &GraphRow,
    head: bool,
    has_parents: bool,
    graph_theme: GraphTheme,
) -> AnyElement {
    let paint = GraphPaint {
        lane: row.lane,
        before: row
            .before
            .iter()
            .map(|waiting| waiting.is_some())
            .collect(),
        joins: row.joins.clone(),
        extra: row.extra.iter().map(|(_, lane)| *lane).collect(),
        has_parents,
        head,
    };
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| paint_history_graph(bounds, &paint, graph_theme, window),
    )
    .w(theme.sp(graph_width(lanes)))
    .h(theme.sp(HISTORY_GRAPH_HEIGHT))
    .flex_shrink_0()
    .into_any_element()
}

/// Paints one row's rails, merge curves, joins, and dot.
fn paint_history_graph(
    bounds: Bounds<Pixels>,
    row: &GraphPaint,
    theme: GraphTheme,
    window: &mut Window,
) {
    let scale = theme.scale;
    let lane_x = |lane: usize| {
        bounds.origin.x
            + px((lane as f32 * HISTORY_LANE_SPACING + HISTORY_GRAPH_PAD_LEFT) * scale)
    };
    let top = bounds.origin.y;
    let bottom = bounds.origin.y + bounds.size.height;
    let center = bounds.origin.y + bounds.size.height / 2.0;
    let control = px(HISTORY_CURVE_CONTROL * scale);
    for (lane, occupied) in row.before.iter().enumerate() {
        if !occupied || lane == row.lane || row.joins.contains(&lane) {
            continue;
        }
        let x = lane_x(lane);
        paint_line(window, point(x, top), point(x, bottom), theme.lane(lane).opacity(0.7), scale);
    }
    if row.before.get(row.lane).copied().unwrap_or(false) {
        let x = lane_x(row.lane);
        paint_line(
            window,
            point(x, top),
            point(x, center),
            theme.lane(row.lane),
            scale,
        );
    }
    for join in &row.joins {
        let from = lane_x(*join);
        let dot = lane_x(row.lane);
        paint_curve(
            window,
            point(from, top),
            point(from, center - control),
            point(dot, center - control),
            point(dot, center),
            theme.lane(*join),
            scale,
        );
    }
    let dot = lane_x(row.lane);
    for lane in &row.extra {
        let target = lane_x(*lane);
        paint_curve(
            window,
            point(dot, center),
            point(dot, center + control),
            point(target, center + control),
            point(target, bottom),
            theme.lane(*lane),
            scale,
        );
    }
    if row.has_parents {
        paint_line(
            window,
            point(dot, center),
            point(dot, bottom),
            theme.lane(row.lane),
            scale,
        );
    }
    let color = theme.lane(row.lane);
    paint_dot(window, dot, center, HISTORY_DOT_RADIUS, color, scale);
    if row.head {
        paint_ring(window, dot, center, HISTORY_HEAD_RING_RADIUS, color, scale);
        paint_dot(window, dot, center, HISTORY_HEAD_DOT_RADIUS, color, scale);
    }
}

/// Paints one straight graph segment.
fn paint_line(
    window: &mut Window,
    from: gpui::Point<Pixels>,
    to: gpui::Point<Pixels>,
    color: Hsla,
    scale: f32,
) {
    let mut builder = PathBuilder::stroke(px(HISTORY_GRAPH_STROKE * scale));
    builder.move_to(from);
    builder.line_to(to);
    paint_stroke(window, builder, color);
}

/// Paints one cubic curve between two lanes.
fn paint_curve(
    window: &mut Window,
    start: gpui::Point<Pixels>,
    control_a: gpui::Point<Pixels>,
    control_b: gpui::Point<Pixels>,
    end: gpui::Point<Pixels>,
    color: Hsla,
    scale: f32,
) {
    let mut builder = PathBuilder::stroke(px(HISTORY_GRAPH_STROKE * scale));
    builder.move_to(start);
    builder.cubic_bezier_to(end, control_a, control_b);
    paint_stroke(window, builder, color);
}

/// Tessellates and paints one stroked path.
///
/// A tessellation failure leaves the segment unpainted rather than panicking in
/// the paint pass; the rows stay usable without it.
fn paint_stroke(window: &mut Window, builder: PathBuilder, color: Hsla) {
    if let Ok(path) = builder.build() {
        window.paint_path(path, color);
    }
}

/// Paints one filled commit dot.
fn paint_dot(window: &mut Window, x: Pixels, y: Pixels, radius: f32, color: Hsla, scale: f32) {
    let diameter = px(radius * 2.0 * scale);
    window.paint_quad(gpui::PaintQuad {
        bounds: Bounds::new(
            point(x - diameter / 2.0, y - diameter / 2.0),
            gpui::size(diameter, diameter),
        ),
        corner_radii: Corners::all(diameter / 2.0),
        background: color.into(),
        border_widths: Edges::all(px(0.)),
        border_color: color,
        border_style: BorderStyle::Solid,
    });
}

/// Paints the HEAD ring around one dot.
fn paint_ring(window: &mut Window, x: Pixels, y: Pixels, radius: f32, color: Hsla, scale: f32) {
    let diameter = px(radius * 2.0 * scale);
    window.paint_quad(gpui::PaintQuad {
        bounds: Bounds::new(
            point(x - diameter / 2.0, y - diameter / 2.0),
            gpui::size(diameter, diameter),
        ),
        corner_radii: Corners::all(diameter / 2.0),
        background: Hsla::transparent_black().into(),
        border_widths: Edges::all(px(1.3 * scale)),
        border_color: color,
        border_style: BorderStyle::Solid,
    });
}

/// Commit detail: identity, parents, refs, patch, and entry actions.
/// @param selected — oid the list selected, or `None` when nothing is selected.
fn history_detail(
    app: &mut DesktopApp,
    theme: &Theme,
    commits: &[GitCommitEntry],
    selected: Option<&str>,
    local_branches: &[String],
    cx: &mut Context<DesktopApp>,
) -> Option<AnyElement> {
    let commit = commits
        .iter()
        .find(|commit| Some(commit.oid.as_str()) == selected)?;
    let refs = history_refs(&commit.refs, local_branches);
    let busy = app.git_loading;
    let mut parents = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(theme.sp(4.))
        .flex_wrap();
    if commit.parents.is_empty() {
        parents = parents.child(
            div()
                .text_size(theme.fs(9.5))
                .text_color(theme.muted)
                .child("Root commit"),
        );
    }
    for parent in &commit.parents {
        let known = commits.iter().any(|commit| &commit.oid == parent);
        let mut chip = div()
            .px(theme.sp(4.))
            .rounded(px(3.))
            .border_1()
            .border_color(theme.border)
            .text_size(theme.fs(9.))
            .text_color(theme.muted)
            .id(gpui::SharedString::from(format!(
                "git-history-parent-{parent}"
            )))
            .child(short_commit(parent).to_owned());
        if known {
            let parent = parent.clone();
            chip = chip
                .cursor_pointer()
                .hover(|style| style.bg(theme.surface_raised))
                .on_click(cx.listener(move |app, _, window, cx| {
                    app.select_git_history(parent.clone());
                    window.focus(&app.git_history_focus);
                    cx.notify();
                }));
        } else {
            chip = chip.opacity(0.55).tooltip(widgets::HintView::tooltip(
                theme,
                "This parent is not in the loaded page; choose Load older to reach it.",
            ));
        }
        parents = parents.child(chip);
    }
    let mut refs_line = Vec::new();
    if refs.head {
        refs_line.push("HEAD".to_owned());
    }
    refs_line.extend(refs.chips.iter().map(|reference| reference.name.clone()));
    let refs_text = if refs_line.is_empty() {
        String::new()
    } else {
        format!("Refs: {}", refs_line.join(", "))
    };
    let patch: AnyElement = match (&app.git_history_show, &app.git_history_show_error) {
        (Some(show), _) => {
            let mut content = div().flex().flex_col().gap(theme.sp(4.)).w_full();
            if show.truncated {
                content = content.child(history_note(
                    theme,
                    "git-history-patch-truncated",
                    "The Host capped this patch at its byte limit.".to_owned(),
                    theme.warn,
                ));
            }
            if show.patch.trim().is_empty() {
                content = content.child(history_note(
                    theme,
                    "git-history-patch-empty",
                    "The Host returned an empty patch for this commit. A merge commit has no default patch, and an empty commit has no change."
                        .to_owned(),
                    theme.muted,
                ));
            } else {
                content = content.child(
                    div()
                        .id("git-history-patch")
                        .debug_selector(|| "git-history-patch".to_owned())
                        .flex_shrink_0()
                        .overflow_x_scroll()
                        .rounded(theme.corner())
                        .border_1()
                        .border_color(theme.border)
                        .bg(theme.code_background)
                        .p(theme.sp(6.))
                        .font(theme.mono_font())
                        .text_size(theme.fs(9.5))
                        .text_color(theme.muted)
                        .whitespace_nowrap()
                        .child(show.patch.clone()),
                );
            }
            content.into_any_element()
        }
        (None, Some(error)) => history_note(
            theme,
            "git-history-patch-error",
            format!("{error} Select the commit again to retry."),
            theme.error,
        ),
        (None, None) if app.git_history_show_loading => history_note(
            theme,
            "git-history-patch-loading",
            "Reading the commit patch…".to_owned(),
            theme.muted,
        ),
        (None, None) => history_note(
            theme,
            "git-history-patch-loading",
            "The patch reads once a Session connects.".to_owned(),
            theme.muted,
        ),
    };
    let meta = format!(
        "{} · {}",
        commit.author_name, commit.author_time
    );
    Some(
        div()
            .flex()
            .flex_col()
            .gap(theme.sp(5.))
            .flex_shrink_0()
            .max_h(gpui::relative(0.46))
            .border_t_1()
            .border_color(theme.border)
            .px(theme.sp(10.))
            .py(theme.sp(8.))
            .id("git-history-detail")
            .debug_selector(|| "git-history-detail".to_owned())
            .overflow_y_scroll()
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_start()
                    .justify_between()
                    .gap(theme.sp(8.))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(theme.sp(3.))
                            .flex_1()
                            .min_w(px(0.))
                            .child(
                                div()
                                    .text_size(theme.fs(12.5))
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .text_color(theme.text)
                                    .child(commit.subject.clone()),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .font(theme.mono_font())
                                    .text_size(theme.fs(9.5))
                                    .text_color(theme.muted)
                                    .child(commit.oid.clone()),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_size(theme.fs(9.5))
                                    .text_color(theme.muted)
                                    .child(meta),
                            )
                            .child(parents)
                            .children((!refs_text.is_empty()).then(|| {
                                div()
                                    .truncate()
                                    .text_size(theme.fs(9.5))
                                    .text_color(theme.muted)
                                    .child(refs_text.clone())
                            })),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(theme.sp(4.))
                            .flex_shrink_0()
                            .child(history_action(
                                theme,
                                "Cherry-pick…",
                                "git-history-cherry",
                                !busy,
                                GitCommitAction::CherryPick,
                                cx,
                            ))
                            .child(history_action(
                                theme,
                                "Revert…",
                                "git-history-revert",
                                !busy,
                                GitCommitAction::Revert,
                                cx,
                            )),
                    ),
            )
            .child(patch)
            .into_any_element(),
    )
}

/// One commit entry action that opens its confirmation dialog.
/// @param enabled — whether no Git request already runs for this repository.
/// @param action — verb the confirmed dialog runs.
fn history_action(
    theme: &Theme,
    label: &str,
    selector: &'static str,
    enabled: bool,
    action: GitCommitAction,
    cx: &mut Context<DesktopApp>,
) -> AnyElement {
    let mut button = div()
        .px(theme.sp(6.))
        .py(px(2.))
        .rounded(theme.corner())
        .border_1()
        .border_color(theme.border)
        .id(gpui::SharedString::from(selector))
        .debug_selector(move || selector.to_owned())
        .text_size(theme.fs(9.5))
        .child(label.to_owned());
    if enabled {
        button = button
            .cursor_pointer()
            .text_color(theme.text)
            .hover(|style| style.bg(theme.surface_raised))
            .on_click(cx.listener(move |app, _, _, cx| {
                app.confirm_git_history_entry(action);
                cx.notify();
            }));
    } else {
        button = button.opacity(0.45).text_color(theme.muted).tooltip(
            widgets::HintView::tooltip(
                theme,
                "A Git request is already running for this repository.",
            ),
        );
    }
    button.into_any_element()
}

/// Small text action used by the Git surfaces.
fn action_text(theme: &Theme, label: &str, id: String) -> gpui::Stateful<gpui::Div> {
    div()
        .px(theme.sp(6.))
        .py(px(2.))
        .rounded(theme.corner())
        .cursor_pointer()
        .hover(|style| style.bg(theme.surface_raised))
        .id(gpui::SharedString::from(id))
        .text_size(theme.fs(10.))
        .text_color(theme.muted)
        .child(label.to_owned())
}

/// Shows the blocking operation, its conflicts, and continue or abort actions.
fn operation_banner(
    app: &mut DesktopApp,
    theme: &Theme,
    cx: &mut Context<DesktopApp>,
) -> Option<AnyElement> {
    let state = app
        .git_state
        .clone()
        .filter(|state| state.operation != "none" || !state.conflicts.is_empty())?;
    let mut conflicts = div().flex().flex_col().gap(theme.sp(1.));
    for path in &state.conflicts {
        let select = path.clone();
        conflicts = conflicts.child(
            div()
                .px(theme.sp(5.))
                .py(px(1.))
                .rounded(theme.corner())
                .cursor_pointer()
                .hover(|style| style.bg(theme.surface_raised))
                .id(gpui::SharedString::from(format!("git-conflict-{path}")))
                .text_size(theme.fs(10.))
                .text_color(theme.error)
                .child(path.clone())
                .on_click(cx.listener(move |app, _, _, cx| {
                    app.changes_tab = ChangesTab::Diff;
                    app.select_git_path(select.clone());
                    cx.notify();
                })),
        );
    }
    Some(
        div()
            .flex()
            .flex_col()
            .gap(theme.sp(3.))
            .p(theme.sp(6.))
            .rounded(theme.corner())
            .bg(theme.surface_raised)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_size(theme.fs(10.5))
                            .text_color(theme.text)
                            .child(format!(
                                "{} in progress · {} conflict(s)",
                                state.operation,
                                state.conflicts.len()
                            )),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .gap(theme.sp(4.))
                            .child(
                                action_text(
                                    theme,
                                    "Continue",
                                    "git-operation-continue".to_owned(),
                                )
                                .on_click(cx.listener(|app, _, _, cx| {
                                    app.continue_git_operation();
                                    cx.notify();
                                })),
                            )
                            .child(
                                action_text(theme, "Abort", "git-operation-abort".to_owned())
                                    .on_click(cx.listener(|app, _, _, cx| {
                                        app.abort_git_operation();
                                        cx.notify();
                                    })),
                            ),
                    ),
            )
            .child(
                div()
                    .text_size(theme.fs(9.5))
                    .text_color(theme.muted)
                    .child("Edit each conflicted file, stage it, then Continue."),
            )
            .child(conflicts)
            .into_any_element(),
    )
}

/// First seven characters of an object id, or the id itself when shorter.
pub(super) fn short_commit(commit: &str) -> &str {
    &commit[..commit.len().min(7)]
}

/// Commits requested per History page.
pub(super) const HISTORY_PAGE_LIMIT: u32 = 50;

/// One commit's placement in the History swimlane graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GraphRow {
    /// Index into the laid-out newest-first commit slice.
    pub(crate) index: usize,
    /// Lane the commit's dot occupies.
    pub(crate) lane: usize,
    /// Lane occupancy before this row; `None` is an empty lane.
    pub(crate) before: Vec<Option<String>>,
    /// Lanes that waited for this commit and now join its lane.
    pub(crate) joins: Vec<usize>,
    /// Additional parents and the lane each merge curve reaches.
    pub(crate) extra: Vec<(String, usize)>,
}

/// Swimlane layout for one newest-first history page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GraphLayout {
    /// One row per laid-out commit, in list order.
    pub(crate) rows: Vec<GraphRow>,
    /// Lane count the fixed gutter must fit.
    pub(crate) lanes: usize,
}

/// Lays out a newest-first commit page as stable swimlanes.
///
/// A lane keeps waiting for the commit it was opened for until that commit is
/// listed, so a branch keeps its column for the whole page. An extra parent of
/// a merge opens its own lane and curves into it; a lane that waited for an
/// already-listed commit joins that commit's lane instead of leaving a rail.
/// @param commits — commits from one `git/log` page, newest first.
/// @returns one row per commit plus the lane count the gutter must fit.
pub(crate) fn layout_graph(commits: &[GitCommitEntry]) -> GraphLayout {
    let mut lanes: Vec<Option<String>> = Vec::new();
    let mut rows = Vec::with_capacity(commits.len());
    let mut max_lanes = 1usize;
    for (index, commit) in commits.iter().enumerate() {
        let matching: Vec<usize> = lanes
            .iter()
            .enumerate()
            .filter(|(_, waiting)| waiting.as_deref() == Some(commit.oid.as_str()))
            .map(|(lane, _)| lane)
            .collect();
        let lane = match matching.first() {
            Some(lane) => *lane,
            None => match lanes.iter().position(Option::is_none) {
                Some(lane) => lane,
                None => {
                    lanes.push(None);
                    lanes.len() - 1
                }
            },
        };
        let before = lanes.clone();
        let joins = matching.get(1..).map(<[usize]>::to_vec).unwrap_or_default();
        for join in &joins {
            lanes[*join] = None;
        }
        lanes[lane] = commit.parents.first().cloned();
        let mut extra = Vec::new();
        for parent in commit.parents.iter().skip(1) {
            let target = match lanes.iter().position(Option::is_none) {
                Some(target) => target,
                None => {
                    lanes.push(None);
                    lanes.len() - 1
                }
            };
            lanes[target] = Some(parent.clone());
            extra.push((parent.clone(), target));
        }
        while lanes.last().is_some_and(Option::is_none) {
            lanes.pop();
        }
        max_lanes = max_lanes.max(lane + 1).max(lanes.len());
        for join in &joins {
            max_lanes = max_lanes.max(join + 1);
        }
        rows.push(GraphRow {
            index,
            lane,
            before,
            joins,
            extra,
        });
    }
    GraphLayout {
        rows,
        lanes: max_lanes,
    }
}

/// Kind of decoration a History ref chip presents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RefKind {
    /// Local branch.
    Branch,
    /// Remote-tracking branch.
    Remote,
    /// Tag.
    Tag,
}

/// One normalized `git/log` decoration ready for a chip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HistoryRef {
    /// Ref as the Host reported it, including the `tag:` prefix.
    pub(crate) name: String,
    /// Kind the chip presents.
    pub(crate) kind: RefKind,
}

/// Refs and HEAD flag a History row presents.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct HistoryRefs {
    /// Whether HEAD points at this commit; the dot carries the ring instead.
    pub(crate) head: bool,
    /// One chip per decoration other than `HEAD`, in Host order.
    pub(crate) chips: Vec<HistoryRef>,
}

/// Splits normalized `git/log` decorations into the HEAD ring flag and chips.
///
/// The Host reports `HEAD`, branch and remote names, and `tag:<name>`; it does
/// not report which remote a name belongs to. A name that is a loaded local
/// branch stays a branch chip, and any other slash-bearing name is presented as
/// a remote branch.
/// @param refs — decorations from one `git/log` commit.
/// @param local_branches — local branch names from `git/branches`, when loaded.
/// @returns the HEAD flag and one chip per non-HEAD decoration.
pub(crate) fn history_refs(refs: &[String], local_branches: &[String]) -> HistoryRefs {
    let mut parsed = HistoryRefs::default();
    for reference in refs {
        if reference == "HEAD" {
            parsed.head = true;
            continue;
        }
        let kind = if reference.starts_with("tag:") {
            RefKind::Tag
        } else if local_branches.iter().any(|branch| branch == reference) {
            RefKind::Branch
        } else if reference.contains('/') {
            RefKind::Remote
        } else {
            RefKind::Branch
        };
        parsed.chips.push(HistoryRef {
            name: reference.clone(),
            kind,
        });
    }
    parsed
}

/// What the History list shows after its loaded rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HistoryFooter {
    /// The last page filled the limit, so older commits may exist.
    LoadOlder {
        /// Commits to skip from HEAD for the next page.
        skip: u32,
    },
    /// The Host capped the page, so older commits are not listed.
    Truncated,
    /// Every commit in the repository is loaded.
    End,
}

/// Decides the History footer from the settled page sizes.
///
/// A full page means the Host may hold older commits. A byte-capped page drops
/// commits from the page it was asked for, so its skip arithmetic no longer
/// lines up with what was listed; paging stops and the cap is shown instead of
/// re-requesting possibly dropped commits.
/// @param received — commits the latest page returned.
/// @param total — commits loaded across every page.
/// @param limit — commits requested per page.
/// @param truncated — whether the Host dropped commits from the latest page.
/// @returns the footer that follows the loaded rows.
pub(crate) fn history_footer(
    received: usize,
    total: usize,
    limit: u32,
    truncated: bool,
) -> HistoryFooter {
    if truncated {
        HistoryFooter::Truncated
    } else if received == limit as usize {
        HistoryFooter::LoadOlder {
            skip: u32::try_from(total).unwrap_or(u32::MAX),
        }
    } else {
        HistoryFooter::End
    }
}

/// Index the History arrow keys move the selection to.
///
/// The first move selects the newest row, whichever arrow started it, so the
/// detail opens on the row the user is looking at before any step is taken.
/// @param current — selected row index, or `None` before the first selection.
/// @param delta — rows to move; a negative value moves towards newer commits.
/// @param len — loaded rows.
/// @returns the next row index, or `None` when the page is empty.
pub(crate) fn stepped_history_selection(
    current: Option<usize>,
    delta: i8,
    len: usize,
) -> Option<usize> {
    if len == 0 {
        return None;
    }
    let Some(current) = current else {
        return Some(0);
    };
    let moved = current as isize + isize::from(delta);
    Some(moved.clamp(0, len as isize - 1) as usize)
}

/// Changed-file list grouped into Staged, Changes, and Untracked.
fn files_body(app: &mut DesktopApp, theme: &Theme, cx: &mut Context<DesktopApp>) -> AnyElement {
    let files = app
        .git_status
        .as_ref()
        .map(|status| status.files.clone())
        .unwrap_or_default();
    let groups = group_files(&files, &app.git_filter);
    let filter_input = app.text_field("git-file-filter", "Filter changed files", false, cx);
    let toolbar = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(theme.sp(6.))
        .px(theme.sp(10.))
        .py(theme.sp(6.))
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .h(theme.control_height())
                .child(filter_input),
        )
        .child(
            action_text(theme, "Refresh", "git-files-refresh".to_owned()).on_click(
                cx.listener(|app, _, _, cx| {
                    app.refresh_git();
                    cx.notify();
                }),
            ),
        );
    let mut list = div().flex().flex_col().gap(theme.sp(6.)).px(theme.sp(8.)).py(theme.sp(4.));
    if !groups.staged.is_empty() {
        list = list.child(group_heading(theme, "Staged", groups.staged.len()));
        for file in &groups.staged {
            list = list.child(file_row(app, theme, file, true, cx));
        }
    }
    if !groups.changes.is_empty() {
        list = list.child(group_heading(theme, "Changes", groups.changes.len()));
        for file in &groups.changes {
            list = list.child(file_row(app, theme, file, false, cx));
        }
    }
    if !groups.untracked.is_empty() {
        list = list.child(group_heading(theme, "Untracked", groups.untracked.len()));
        for file in &groups.untracked {
            list = list.child(file_row(app, theme, file, false, cx));
        }
    }
    if groups.is_empty() {
        list = list.child(if files.is_empty() {
            empty_state(
                theme,
                "Working tree clean",
                "The Host reports no changes in this workspace.",
            )
        } else {
            empty_state(
                theme,
                "No matching changes",
                "Clear the filter to see every changed path.",
            )
        });
    }
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.))
        .child(toolbar)
        .children(selection_bar(app, theme, cx))
        .children(operation_banner(app, theme, cx))
        .children(app.git_error.as_ref().map(|error| error_line(theme, error)))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_h(px(0.))
                .id("git-files-scroll")
                .overflow_y_scroll()
                .child(list),
        )
        .into_any_element()
}

/// Bulk actions for the ticked paths.
fn selection_bar(
    app: &mut DesktopApp,
    theme: &Theme,
    cx: &mut Context<DesktopApp>,
) -> Option<AnyElement> {
    let count = app.git_selection.len();
    if count == 0 {
        return None;
    }
    let selected = app.selected_git_files();
    let busy = app.git_loading;
    let stage_ready = !busy && selected.iter().any(|file| file.worktree);
    let unstage_ready = !busy && selected.iter().any(|file| file.staged);
    let discard_ready = !busy
        && selected
            .iter()
            .any(|file| file.worktree && can_discard(&file.status));
    let mut stage = action_text(theme, "Stage", "git-bulk-stage".to_owned());
    stage = if stage_ready {
        stage.on_click(cx.listener(|app, _, _, cx| {
            app.stage_git_selection();
            cx.notify();
        }))
    } else {
        stage.opacity(0.45).tooltip(widgets::HintView::tooltip(
            theme,
            if busy {
                "A Git action is running; wait for its result."
            } else {
                "No ticked path has an unstaged change."
            },
        ))
    };
    let mut unstage = action_text(theme, "Unstage", "git-bulk-unstage".to_owned());
    unstage = if unstage_ready {
        unstage.on_click(cx.listener(|app, _, _, cx| {
            app.unstage_git_selection();
            cx.notify();
        }))
    } else {
        unstage.opacity(0.45).tooltip(widgets::HintView::tooltip(
            theme,
            if busy {
                "A Git action is running; wait for its result."
            } else {
                "No ticked path has a staged change."
            },
        ))
    };
    let mut discard = action_text(theme, "Discard…", "git-bulk-discard".to_owned());
    discard = if discard_ready {
        discard.on_click(cx.listener(|app, _, _, cx| {
            app.discard_git_selection();
            cx.notify();
        }))
    } else {
        discard.opacity(0.45).tooltip(widgets::HintView::tooltip(
            theme,
            if busy {
                "A Git action is running; wait for its result."
            } else {
                "No ticked path has a discarding work-tree change."
            },
        ))
    };
    Some(
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(theme.sp(4.))
            .px(theme.sp(10.))
            .py(theme.sp(3.))
            .bg(theme.surface_raised)
            .child(
                div()
                    .text_size(theme.fs(9.5))
                    .text_color(theme.text)
                    .child(format!("{count} selected")),
            )
            .child(stage)
            .child(unstage)
            .child(discard)
            .child(div().flex_1())
            .child(
                action_text(theme, "Clear", "git-bulk-clear".to_owned()).on_click(cx.listener(
                    |app, _, _, cx| {
                        app.clear_git_selection();
                        cx.notify();
                    },
                )),
            )
            .into_any_element(),
    )
}

/// One group heading with its path count.
fn group_heading(theme: &Theme, label: &str, count: usize) -> AnyElement {
    div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .px(theme.sp(4.))
        .py(px(2.))
        .child(
            div()
                .text_size(theme.fs(9.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(theme.muted)
                .child(label.to_uppercase()),
        )
        .child(
            div()
                .text_size(theme.fs(9.))
                .text_color(theme.muted)
                .child(count.to_string()),
        )
        .into_any_element()
}

/// One changed path in a group, with its tick box and hover actions.
fn file_row(
    app: &mut DesktopApp,
    theme: &Theme,
    file: &GitFile,
    staged: bool,
    cx: &mut Context<DesktopApp>,
) -> AnyElement {
    let untracked = is_untracked(file);
    let path = file.path.clone();
    let highlight = app.git_selected_path.as_deref() == Some(path.as_str());
    let ticked = app.git_selection_contains(&path);
    let reviewed = app.git_is_reviewed(&path);
    let side = if staged {
        "staged"
    } else if untracked {
        "untracked"
    } else {
        "changes"
    };
    let group = format!("git-file-hover-{side}-{path}");
    let select = path.clone();
    let row = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(theme.sp(5.))
        .h(theme.sp(REVIEW_ROW_HEIGHT))
        .px(theme.sp(4.))
        .rounded(theme.corner())
        .group(group.clone())
        .when(highlight, |element| element.bg(theme.selection))
        .hover(|style| style.bg(theme.surface_raised))
        .id(gpui::SharedString::from(format!("git-file-{side}-{path}")))
        .cursor_pointer()
        .on_click(cx.listener(move |app, _, _, cx| {
            app.select_git_file(select.clone(), staged);
            cx.notify();
        }))
        .child(select_box(theme, &path, side, ticked, cx))
        .child(kind_chip(theme, &file.status))
        .child(path_label(theme, &path))
        .child(file_stats_label(theme, file, staged))
        .when(reviewed, |element| element.child(reviewed_pill(theme)));
    row.child(file_actions(app, theme, file, staged, group, cx))
        .into_any_element()
}

/// Tick box that toggles one path in the bulk selection.
fn select_box(
    theme: &Theme,
    path: &str,
    side: &str,
    ticked: bool,
    cx: &mut Context<DesktopApp>,
) -> AnyElement {
    let path = path.to_owned();
    div()
        .w(theme.sp(11.))
        .text_size(theme.fs(10.))
        .text_color(if ticked { theme.accent } else { theme.muted })
        .child(if ticked { "☑" } else { "☐" })
        .id(gpui::SharedString::from(format!("git-select-{side}-{path}")))
        .cursor_pointer()
        .on_click(cx.listener(move |app, _, _, cx| {
            app.toggle_git_selection(path.clone());
            cx.stop_propagation();
            cx.notify();
        }))
        .into_any_element()
}

/// Short badge for one change kind.
fn change_badge(status: &str) -> &str {
    match status {
        "added" => "+",
        "deleted" => "-",
        "renamed" => "R",
        "copied" => "C",
        "typechange" => "T",
        "untracked" => "?",
        "ignored" => "!",
        "conflict" => "U",
        _ => "M",
    }
}

/// Kind chip for one change.
fn kind_chip(theme: &Theme, status: &str) -> AnyElement {
    let color = match status {
        "added" | "untracked" => theme.accent,
        "deleted" | "conflict" => theme.error,
        _ => theme.muted,
    };
    div()
        .w(theme.sp(13.))
        .flex()
        .justify_center()
        .rounded(theme.corner())
        .bg(theme.surface_raised)
        .text_size(theme.fs(8.5))
        .text_color(color)
        .child(change_badge(status).to_owned())
        .into_any_element()
}

/// Repository path with a dimmed directory prefix.
fn path_label(theme: &Theme, path: &str) -> AnyElement {
    let (dir, base) = match path.rsplit_once('/') {
        Some((dir, base)) => (format!("{dir}/"), base),
        None => (String::new(), path),
    };
    let mut label = div().flex().flex_row().items_center().flex_1().min_w(px(0.));
    if !dir.is_empty() {
        label = label.child(
            div()
                .flex_none()
                .max_w(theme.sp(110.))
                .truncate()
                .text_size(theme.fs(9.5))
                .text_color(theme.muted)
                .child(dir),
        );
    }
    label
        .child(
            div()
                .truncate()
                .text_size(theme.fs(10.5))
                .text_color(theme.text)
                .child(base.to_owned()),
        )
        .into_any_element()
}

/// One file's totals for the shown side, never a sum of both sides.
fn file_stats_label(theme: &Theme, file: &GitFile, staged: bool) -> AnyElement {
    if is_untracked(file) {
        return div()
            .text_size(theme.fs(9.))
            .text_color(theme.muted)
            .child("untracked")
            .into_any_element();
    }
    match side_stats(file, staged) {
        Some(stats) => div()
            .flex()
            .flex_row()
            .items_center()
            .gap(theme.sp(3.))
            .text_size(theme.fs(9.))
            .child(
                div()
                    .text_color(theme.accent)
                    .child(format!("+{}", stats.additions)),
            )
            .child(
                div()
                    .text_color(theme.error)
                    .child(format!("−{}", stats.deletions)),
            )
            .into_any_element(),
        None => div()
            .text_size(theme.fs(9.))
            .text_color(theme.muted)
            .child("binary")
            .into_any_element(),
    }
}

/// Reviewed marker shown after a path.
fn reviewed_pill(theme: &Theme) -> AnyElement {
    div()
        .text_size(theme.fs(8.5))
        .text_color(theme.accent)
        .child("reviewed")
        .into_any_element()
}

/// Per-row hover actions for one review group.
fn file_actions(
    app: &mut DesktopApp,
    theme: &Theme,
    file: &GitFile,
    staged: bool,
    group: String,
    cx: &mut Context<DesktopApp>,
) -> AnyElement {
    let busy = app.git_loading;
    let mut actions = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(theme.sp(2.))
        .opacity(0.)
        .group_hover(group.clone(), |style| style.opacity(1.));
    let busy_reason = "A Git action is running; wait for its result.";
    if staged {
        let mut unstage = row_action(
            theme,
            "Unstage",
            format!("git-row-unstage-{group}"),
            group.clone(),
            false,
        );
        unstage = if busy {
            unstage.opacity(0.45).tooltip(widgets::HintView::tooltip(theme, busy_reason))
        } else {
            let path = file.path.clone();
            unstage.on_click(cx.listener(move |app, _, _, cx| {
                app.unstage_git_path(path.clone());
                cx.stop_propagation();
                cx.notify();
            }))
        };
        actions = actions.child(unstage);
    } else if is_untracked(file) {
        let mut stage = row_action(
            theme,
            "Stage",
            format!("git-row-stage-{group}"),
            group.clone(),
            false,
        );
        stage = if busy {
            stage.opacity(0.45).tooltip(widgets::HintView::tooltip(theme, busy_reason))
        } else {
            let path = file.path.clone();
            stage.on_click(cx.listener(move |app, _, _, cx| {
                app.stage_git_path(path.clone());
                cx.stop_propagation();
                cx.notify();
            }))
        };
        actions = actions.child(stage).child(delete_untracked_action(theme, &group));
    } else {
        let mut stage = row_action(
            theme,
            "Stage",
            format!("git-row-stage-{group}"),
            group.clone(),
            false,
        );
        stage = if file.status == "conflict" {
            stage.opacity(0.45).tooltip(widgets::HintView::tooltip(
                theme,
                "Conflict choices and resolution land in G4; the Host refuses a patch on a conflicted path.",
            ))
        } else if busy {
            stage.opacity(0.45).tooltip(widgets::HintView::tooltip(theme, busy_reason))
        } else {
            let path = file.path.clone();
            stage.on_click(cx.listener(move |app, _, _, cx| {
                app.stage_git_path(path.clone());
                cx.stop_propagation();
                cx.notify();
            }))
        };
        actions = actions.child(stage);
        if can_discard(&file.status) {
            let mut discard = row_action(
                theme,
                "Discard",
                format!("git-row-discard-{group}"),
                group.clone(),
                true,
            );
            discard = if busy {
                discard.opacity(0.45).tooltip(widgets::HintView::tooltip(theme, busy_reason))
            } else {
                let path = file.path.clone();
                discard.on_click(cx.listener(move |app, _, _, cx| {
                    app.discard_git_path(path.clone());
                    cx.stop_propagation();
                    cx.notify();
                }))
            };
            actions = actions.child(discard);
        }
    }
    actions.into_any_element()
}

/// One hover-revealed row action.
fn row_action(
    theme: &Theme,
    label: &str,
    id: String,
    group: String,
    danger: bool,
) -> gpui::Stateful<gpui::Div> {
    div()
        .px(theme.sp(3.))
        .py(px(1.))
        .rounded(theme.corner())
        .text_size(theme.fs(9.))
        .text_color(if danger { theme.error } else { theme.muted })
        .opacity(0.)
        .group_hover(group, |style| style.opacity(1.))
        .hover(|style| style.bg(theme.surface_raised))
        .id(gpui::SharedString::from(id))
        .cursor_pointer()
        .child(label.to_owned())
}

/// Untracked delete stays disabled until the Host gains the verb in G4.
fn delete_untracked_action(theme: &Theme, group: &str) -> AnyElement {
    div()
        .px(theme.sp(3.))
        .py(px(1.))
        .rounded(theme.corner())
        .text_size(theme.fs(9.))
        .text_color(theme.muted)
        .opacity(0.)
        .group_hover(group.to_owned(), |style| style.opacity(0.5))
        .id(gpui::SharedString::from(format!("git-row-delete-{group}")))
        .tooltip(widgets::HintView::tooltip(
            theme,
            "Deleting an untracked path needs a Host verb; it lands in G4.",
        ))
        .child("Delete")
        .into_any_element()
}

/// Whether the Host accepts a discard for this change kind.
fn can_discard(status: &str) -> bool {
    matches!(status, "modified" | "deleted" | "typechange")
}

/// Whether Git reported one changed path as binary rather than a text patch.
///
/// The Host returns the patch as text, so magic text and NUL bytes are the only
/// client-safe signal available until the RPC exposes a binary flag.
fn git_diff_is_binary(diff: &GitDiff) -> bool {
    diff.patch.contains('\0')
        || diff.patch
            .lines()
            .any(|line| line.starts_with("Binary files ") && line.ends_with(" differ"))
}

/// Diff pane for the selected path with an Unstaged/Staged switch.
fn diff_body(app: &mut DesktopApp, theme: &Theme, cx: &mut Context<DesktopApp>) -> AnyElement {
    let staged = app.git_diff_staged;
    let busy = app.git_loading;
    let path = app.git_selected_path.clone();
    let file = path.as_deref().and_then(|path| app.git_file(path));
    let diff = app.git_diff.clone();
    let parsed = diff.as_ref().map(|diff| parse_diff(&diff.patch));
    let binary = diff.as_ref().is_some_and(git_diff_is_binary);
    let file_label = match file.as_ref() {
        Some(file) => div()
            .flex()
            .flex_row()
            .items_center()
            .gap(theme.sp(5.))
            .flex_1()
            .min_w(px(0.))
            .child(
                div()
                    .truncate()
                    .text_size(theme.fs(10.5))
                    .text_color(theme.text)
                    .child(file.path.clone()),
            )
            .child(file_stats_label(theme, file, staged))
            .into_any_element(),
        None => div()
            .text_size(theme.fs(10.5))
            .text_color(theme.muted)
            .child("No file selected")
            .into_any_element(),
    };
    let file_bar = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(theme.sp(6.))
        .px(theme.sp(10.))
        .py(theme.sp(6.))
        .border_b_1()
        .border_color(theme.border)
        .child(file_label)
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(theme.sp(2.))
                .child(diff_side_toggle(
                    theme, "Unstaged", false, staged, busy, cx,
                ))
                .child(diff_side_toggle(theme, "Staged", true, staged, busy, cx)),
        );
    let mut content = div().flex().flex_col().gap(theme.sp(6.)).p(theme.sp(8.));
    match (path.as_ref(), diff.as_ref()) {
        (None, _) => {
            content = content.child(empty_state(
                theme,
                "Select a changed file",
                "Pick a path in Files to read its diff.",
            ));
        }
        (Some(_), None) => {
            content = content.child(
                div()
                    .text_size(theme.fs(10.5))
                    .text_color(theme.muted)
                    .child(if app.git_loading {
                        "Reading the diff…"
                    } else {
                        "Diff unavailable. Refresh the status and select the path again."
                    }),
            );
        }
        (Some(path), Some(diff)) => {
            if diff.truncated {
                content = content.child(state_note(
                    theme,
                    "The Host capped this patch at its byte limit. Hunk actions are disabled for a partial patch.",
                    theme.warn,
                ));
            }
            if binary {
                content = content.child(state_note(
                    theme,
                    "Git reports this path as binary: there is no text diff to review.",
                    theme.warn,
                ));
            } else if diff.untracked {
                content = content.child(state_note(
                    theme,
                    "Untracked path: this is the whole file, not a change against the index. Stage it before committing; hunk actions need a tracked change.",
                    theme.muted,
                ));
            }
            if let Some(parsed) = parsed.as_ref() {
                if parsed.hunks.is_empty() && !binary {
                    content = content.child(state_note(
                        theme,
                        "No text hunks in this patch.",
                        theme.muted,
                    ));
                }
                let status = file.as_ref().map(|file| file.status.as_str()).unwrap_or("");
                let disabled = if binary {
                    Some("A binary patch has no hunk to apply.")
                } else if diff.truncated {
                    Some("A partial patch cannot be applied safely.")
                } else if diff.untracked {
                    Some("The Host refuses a patch on an untracked path; stage the file first.")
                } else if status == "conflict" {
                    Some("Conflict choices and resolution land in G4; the Host refuses a patch on a conflicted path.")
                } else {
                    None
                };
                for (index, hunk) in parsed.hunks.iter().enumerate() {
                    let patch = parsed.hunk_patch(index).unwrap_or_default();
                    content = content.child(hunk_card(
                        app, theme, path, status, hunk, &patch, staged, disabled, cx,
                    ));
                }
            }
        }
    }
    let review_bar = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(theme.sp(6.))
        .px(theme.sp(10.))
        .py(theme.sp(5.))
        .border_t_1()
        .border_color(theme.border)
        .child(reviewed_toggle(
            theme,
            path.as_deref().is_some_and(|path| app.git_is_reviewed(path)),
            path.is_some() && !app.git_loading,
            cx,
        ))
        .child(
            div()
                .text_size(theme.fs(9.))
                .text_color(theme.muted)
                .child(
                    "Reviewed is keyed by path and content signature; it clears when the status entry changes.",
                ),
        );
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.))
        .children(operation_banner(app, theme, cx))
        .children(app.git_error.as_ref().map(|error| error_line(theme, error)))
        .child(file_bar)
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_h(px(0.))
                .id("git-diff-scroll")
                .overflow_scroll()
                .child(content),
        )
        .child(review_bar)
        .into_any_element()
}

/// Unstaged/Staged switch for the diff.
fn diff_side_toggle(
    theme: &Theme,
    label: &str,
    value: bool,
    current: bool,
    busy: bool,
    cx: &mut Context<DesktopApp>,
) -> AnyElement {
    let selected = current == value;
    let mut toggle = div()
        .px(theme.sp(6.))
        .py(px(2.))
        .rounded(theme.corner())
        .when(selected, |element| element.bg(theme.selection))
        .hover(|style| style.bg(theme.surface_raised))
        .text_size(theme.fs(9.5))
        .text_color(if selected { theme.accent } else { theme.muted })
        .id(gpui::SharedString::from(format!("git-diff-{label}")))
        .child(label.to_owned());
    toggle = if selected {
        toggle.cursor_pointer()
    } else if busy {
        toggle.opacity(0.45).tooltip(widgets::HintView::tooltip(
            theme,
            "A Git action is running; wait for its result.",
        ))
    } else {
        toggle.cursor_pointer().on_click(cx.listener(
            move |app, _, _, cx| {
                app.set_git_diff_staged(value);
                cx.notify();
            },
        ))
    };
    toggle.into_any_element()
}

/// Reviewed checkbox for the selected path.
fn reviewed_toggle(
    theme: &Theme,
    reviewed: bool,
    enabled: bool,
    cx: &mut Context<DesktopApp>,
) -> AnyElement {
    let mut toggle = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(theme.sp(4.))
        .px(theme.sp(4.))
        .py(px(1.))
        .rounded(theme.corner())
        .text_size(theme.fs(9.5))
        .text_color(if reviewed { theme.accent } else { theme.muted })
        .id("git-reviewed-toggle")
        .child(if reviewed { "☑ Reviewed" } else { "☐ Reviewed" });
    if enabled {
        toggle = toggle.cursor_pointer().on_click(cx.listener(|app, _, _, cx| {
            app.toggle_git_reviewed();
            cx.notify();
        }));
    } else {
        toggle = toggle
            .opacity(0.45)
            .tooltip(widgets::HintView::tooltip(
                theme,
                "Select a changed path to mark it reviewed.",
            ));
    }
    toggle.into_any_element()
}

/// One hunk card with old/new gutters and its stage, unstage, or discard action.
fn hunk_card(
    app: &mut DesktopApp,
    theme: &Theme,
    path: &str,
    status: &str,
    hunk: &DiffHunk,
    patch: &str,
    staged: bool,
    disabled: Option<&str>,
    cx: &mut Context<DesktopApp>,
) -> AnyElement {
    let mut actions = div().flex().flex_row().items_center().gap(theme.sp(2.));
    let busy_reason = "A Git action is running; wait for its result.";
    let blocked = if app.git_loading {
        Some(busy_reason)
    } else {
        disabled
    };
    if staged {
        let mut unstage = action_text(
            theme,
            "Unstage hunk",
            format!("git-hunk-unstage-{path}-{}", hunk.header),
        );
        unstage = match blocked {
            Some(reason) => unstage
                .opacity(0.45)
                .tooltip(widgets::HintView::tooltip(theme, reason.to_owned())),
            None => {
                let path = path.to_owned();
                let patch = patch.to_owned();
                let expected = status.to_owned();
                unstage.on_click(cx.listener(move |app, _, _, cx| {
                    app.apply_git_hunk(
                        path.clone(),
                        patch.clone(),
                        GitApplyTarget::Index,
                        true,
                        Some(expected.clone()),
                    );
                    cx.notify();
                }))
            }
        };
        actions = actions.child(unstage);
    } else {
        let mut stage = action_text(
            theme,
            "Stage hunk",
            format!("git-hunk-stage-{path}-{}", hunk.header),
        );
        stage = match blocked {
            Some(reason) => stage
                .opacity(0.45)
                .tooltip(widgets::HintView::tooltip(theme, reason.to_owned())),
            None => {
                let path = path.to_owned();
                let patch = patch.to_owned();
                let expected = status.to_owned();
                stage.on_click(cx.listener(move |app, _, _, cx| {
                    app.apply_git_hunk(
                        path.clone(),
                        patch.clone(),
                        GitApplyTarget::Index,
                        false,
                        Some(expected.clone()),
                    );
                    cx.notify();
                }))
            }
        };
        let mut discard = action_text(
            theme,
            "Discard hunk…",
            format!("git-hunk-discard-{path}-{}", hunk.header),
        );
        discard = match blocked {
            Some(reason) => discard
                .opacity(0.45)
                .tooltip(widgets::HintView::tooltip(theme, reason.to_owned())),
            None => {
                let path = path.to_owned();
                let patch = patch.to_owned();
                discard.on_click(cx.listener(move |app, _, _, cx| {
                    app.discard_git_hunk(path.clone(), patch.clone());
                    cx.notify();
                }))
            }
        };
        actions = actions.child(stage).child(discard);
    }
    let mut lines = div().flex().flex_col().py(px(2.));
    for line in &hunk.lines {
        lines = lines.child(hunk_line(theme, line));
    }
    div()
        .flex()
        .flex_col()
        .rounded(theme.corner())
        .border_1()
        .border_color(theme.border)
        .overflow_hidden()
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .gap(theme.sp(6.))
                .px(theme.sp(6.))
                .py(px(2.))
                .bg(theme.surface_raised)
                .child(
                    div()
                        .truncate()
                        .text_size(theme.fs(9.))
                        .font(theme.mono_font())
                        .text_color(theme.muted)
                        .child(hunk.header.clone()),
                )
                .child(actions),
        )
        .child(lines)
        .into_any_element()
}

/// One hunk line with both gutters.
fn hunk_line(theme: &Theme, line: &DiffLine) -> AnyElement {
    let color = match line.kind {
        '+' => theme.accent,
        '-' => theme.error,
        _ => theme.text,
    };
    div()
        .flex()
        .flex_row()
        .whitespace_nowrap()
        .font(theme.mono_font())
        .text_size(theme.fs(9.5))
        .child(gutter(theme, line.old_line))
        .child(gutter(theme, line.new_line))
        .child(
            div()
                .w(theme.sp(10.))
                .flex_shrink_0()
                .text_color(color)
                .child(line.kind.to_string()),
        )
        .child(
            div()
                .text_color(color)
                .child(if line.text.is_empty() {
                    " ".to_owned()
                } else {
                    line.text.clone()
                }),
        )
        .into_any_element()
}

/// Right-aligned line number for one side.
fn gutter(theme: &Theme, number: Option<u32>) -> AnyElement {
    div()
        .w(theme.sp(GUTTER_WIDTH))
        .flex_shrink_0()
        .text_align(TextAlign::Right)
        .text_color(theme.muted)
        .child(number.map(|number| number.to_string()).unwrap_or_default())
        .into_any_element()
}

/// Inline note for a state that blocks review actions.
fn state_note(theme: &Theme, text: &str, color: gpui::Hsla) -> AnyElement {
    div()
        .px(theme.sp(6.))
        .py(theme.sp(4.))
        .rounded(theme.corner())
        .bg(theme.surface_raised)
        .text_size(theme.fs(9.5))
        .text_color(color)
        .child(text.to_owned())
        .into_any_element()
}

/// Empty-state block with a title and detail line.
fn empty_state(theme: &Theme, title: &str, detail: &str) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap(theme.sp(3.))
        .px(theme.sp(8.))
        .py(theme.sp(12.))
        .child(
            div()
                .text_size(theme.fs(11.))
                .text_color(theme.text)
                .child(title.to_owned()),
        )
        .child(
            div()
                .text_size(theme.fs(10.))
                .text_color(theme.muted)
                .child(detail.to_owned()),
        )
        .into_any_element()
}

/// One error line for the last failed Git verb.
fn error_line(theme: &Theme, error: &str) -> AnyElement {
    div()
        .px(theme.sp(10.))
        .py(theme.sp(3.))
        .text_size(theme.fs(10.))
        .text_color(theme.error)
        .child(error.to_owned())
        .into_any_element()
}

/// Commit bar: staged summary, message, recent messages, amend, sign-off, commit.
fn commit_bar(app: &mut DesktopApp, theme: &Theme, cx: &mut Context<DesktopApp>) -> AnyElement {
    let files = app
        .git_status
        .as_ref()
        .map(|status| status.files.clone())
        .unwrap_or_default();
    let summary = staged_summary(&files);
    let status = app.git_status.clone();
    let busy = app.git_loading;
    let in_operation = app
        .git_state
        .as_ref()
        .is_some_and(|state| state.operation != "none" || !state.conflicts.is_empty());
    let pushed = status.as_ref().is_some_and(head_is_pushed);
    let upstream = status.as_ref().and_then(|status| status.upstream.clone());
    let behind = status.as_ref().map(|status| status.behind).unwrap_or(0);
    let commit = commit_blocker(summary.files, &app.git_commit_message, busy, in_operation);
    let push = push_blocker(commit, upstream.as_deref(), behind);
    let amend_checked = app.git_amend && !pushed;
    let sign_off = app.git_sign_off;
    let mut summary_row = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(theme.sp(4.))
        .text_size(theme.fs(9.5))
        .text_color(theme.muted);
    summary_row = if summary.files == 0 {
        summary_row.child("No staged changes")
    } else {
        summary_row
            .child(format!("{} staged", summary.files))
            .child(
                div()
                    .text_color(theme.accent)
                    .child(format!("+{}", summary.additions)),
            )
            .child(
                div()
                    .text_color(theme.error)
                    .child(format!("−{}", summary.deletions)),
            )
    };
    let recent_toggle = action_text(
        theme,
        if app.git_recent_open { "Recent ▴" } else { "Recent ▾" },
        "git-recent-toggle".to_owned(),
    )
    .on_click(cx.listener(|app, _, _, cx| {
        app.git_recent_open = !app.git_recent_open;
        cx.notify();
    }));
    let message_input = app.text_field("git-commit-message", "Commit message", false, cx);
    let mut recent = None;
    if app.git_recent_open {
        let mut list = div().flex().flex_col().gap(px(1.));
        for (index, subject) in app
            .git_recent_messages
            .iter()
            .take(RECENT_MESSAGE_LIMIT)
            .enumerate()
        {
            let fill = subject.clone();
            list = list.child(
                div()
                    .truncate()
                    .px(theme.sp(4.))
                    .py(px(1.))
                    .rounded(theme.corner())
                    .text_size(theme.fs(9.5))
                    .text_color(theme.text)
                    .hover(|style| style.bg(theme.surface_raised))
                    .id(gpui::SharedString::from(format!(
                        "git-recent-{index}"
                    )))
                    .cursor_pointer()
                    .child(subject.clone())
                    .on_click(cx.listener(move |app, _, _, cx| {
                        app.set_input("git-commit-message", &fill, cx);
                        app.git_commit_message = fill.clone();
                        cx.notify();
                    })),
            );
        }
        if app.git_recent_messages.is_empty() {
            list = list.child(
                div()
                    .text_size(theme.fs(9.))
                    .text_color(theme.muted)
                    .child(if app.git_log_loading {
                        "Reading recent commits…"
                    } else {
                        "No recent commit messages."
                    }),
            );
        }
        if let Some(error) = app.git_log_error.as_ref() {
            list = list.child(
                div()
                    .text_size(theme.fs(9.))
                    .text_color(theme.error)
                    .child(error.clone()),
            );
        }
        recent = Some(list);
    }
    let mut amend = option_toggle(theme, "Amend", amend_checked);
    amend = if commit.is_none() && !pushed && status.as_ref().is_some_and(|status| status.commit.is_some())
    {
        amend.on_click(cx.listener(|app, _, _, cx| {
            app.git_amend = !app.git_amend;
            cx.notify();
        }))
    } else {
        amend.opacity(0.45).tooltip(widgets::HintView::tooltip(
            theme,
            if pushed {
                "HEAD is already on the upstream; amending would need a force-push, which lands in G3."
            } else if status.as_ref().and_then(|status| status.commit.as_deref()).is_none() {
                "There is no commit to amend yet."
            } else {
                "Stage a change and enter a message before amending."
            },
        ))
    };
    let mut sign_off_toggle = option_toggle(theme, "Sign-off", sign_off);
    sign_off_toggle = sign_off_toggle.on_click(cx.listener(|app, _, _, cx| {
        app.git_sign_off = !app.git_sign_off;
        cx.notify();
    }));
    let mut commit_button = widgets::Button::new(theme, "Commit")
        .primary()
        .enabled(commit.is_none())
        .build()
        .id("git-commit");
    commit_button = match commit {
        Some(reason) => commit_button.tooltip(widgets::HintView::tooltip(theme, reason)),
        None => commit_button.on_click(cx.listener(move |app, _, _, cx| {
            let message = app.input_text("git-commit-message", cx);
            app.commit_git(message, amend_checked, sign_off, false);
            cx.notify();
        })),
    };
    let mut push_button = widgets::Button::new(theme, "Commit & Push")
        .enabled(push.is_none())
        .build()
        .id("git-commit-push");
    push_button = match &push {
        Some(reason) => push_button.tooltip(widgets::HintView::tooltip(theme, reason.clone())),
        None => push_button.on_click(cx.listener(move |app, _, _, cx| {
            let message = app.input_text("git-commit-message", cx);
            app.commit_git(message, amend_checked, sign_off, true);
            cx.notify();
        })),
    };
    let hint = commit
        .map(str::to_owned)
        .or_else(|| push.clone())
        .or_else(|| {
            pushed.then(|| {
                "HEAD is already on the upstream; Amend stays disabled until a local commit exists."
                    .to_owned()
            })
        });
    div()
        .flex()
        .flex_col()
        .gap(theme.sp(5.))
        .px(theme.sp(10.))
        .py(theme.sp(8.))
        .border_t_1()
        .border_color(theme.border)
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .gap(theme.sp(6.))
                .child(summary_row)
                .child(recent_toggle),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(theme.sp(4.))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .h(theme.control_height())
                        .child(message_input),
                ),
        )
        .children(recent)
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(theme.sp(6.))
                .child(amend)
                .child(sign_off_toggle)
                .child(div().flex_1())
                .child(push_button)
                .child(commit_button),
        )
        .children(hint.map(|hint| {
            div()
                .text_size(theme.fs(9.))
                .text_color(theme.muted)
                .child(hint)
        }))
        .into_any_element()
}

/// Checkbox-style option that the caller attaches a click handler to.
fn option_toggle(theme: &Theme, label: &str, checked: bool) -> gpui::Stateful<gpui::Div> {
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(theme.sp(3.))
        .rounded(theme.corner())
        .text_size(theme.fs(9.5))
        .text_color(if checked { theme.accent } else { theme.muted })
        .id(gpui::SharedString::from(format!("git-option-{label}")))
        .cursor_pointer()
        .child(format!("{} {label}", if checked { "☑" } else { "☐" }))
}

/// Confirmation dialog for a destructive Git action.
pub(super) fn confirmation_dialog(
    _app: &mut DesktopApp,
    theme: &Theme,
    confirmation: GitConfirmation,
    cx: &mut Context<DesktopApp>,
) -> AnyElement {
    let (title, body, action_label) = match &confirmation {
        GitConfirmation::Discard { path, status, .. } => (
            "Discard unstaged changes?".to_owned(),
            format!(
                "This restores \"{path}\" ({status}) from the index. Any staged changes remain. This cannot be undone."
            ),
            "Discard changes",
        ),
        GitConfirmation::DiscardMany { files, .. } => (
            format!("Discard {} unstaged changes?", files.len()),
            bulk_discard_body(files),
            "Discard changes",
        ),
        GitConfirmation::DiscardHunk {
            path, patch, status, ..
        } => {
            let header = parse_diff(patch)
                .hunks
                .first()
                .map(|hunk| hunk.header.clone())
                .unwrap_or_default();
            (
                "Discard this hunk?".to_owned(),
                format!(
                    "This restores the hunk {header} of \"{path}\" ({status}) from the index. Other hunks keep their work-tree content. This cannot be undone."
                ),
                "Discard hunk",
            )
        }
        GitConfirmation::Checkout {
            current,
            target,
            head,
            changed_files,
            ..
        } => (
            "Switch branch?".to_owned(),
            format!(
                "Switch from {} to \"{target}\" on {}. {}",
                current.as_deref().unwrap_or("detached HEAD"),
                head.as_deref().unwrap_or("unborn HEAD"),
                if changed_files.is_empty() {
                    "The work tree is clean.".to_owned()
                } else {
                    format!(
                        "{} changed path(s) will be carried only when Git can switch without overwriting them.",
                        changed_files.len()
                    )
                }
            ),
            "Switch branch",
        ),
        GitConfirmation::CommitEntry {
            commit,
            subject,
            action,
            ..
        } => {
            let short = short_commit(commit);
            match action {
                GitCommitAction::Revert => (
                    "Revert commit?".to_owned(),
                    format!(
                        "This asks the Host to revert {short} \"{subject}\" on the current branch. The surface reports only the result the Host returns."
                    ),
                    "Revert",
                ),
                GitCommitAction::CherryPick => (
                    "Cherry-pick commit?".to_owned(),
                    format!(
                        "This asks the Host to cherry-pick {short} \"{subject}\" onto the current branch. The surface reports only the result the Host returns."
                    ),
                    "Cherry-pick",
                ),
            }
        }
    };
    let details = body.to_owned();
    dialog_shell(theme, &title, |content| {
        content.child(
            div()
                .flex()
                .flex_col()
                .gap(theme.gap())
                .child(
                    div()
                        .text_size(theme.fs(11.5))
                        .text_color(theme.muted)
                        .child(details),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .justify_end()
                        .gap(theme.sp(6.))
                        .child(
                            widgets::Button::new(theme, "Cancel")
                                .build()
                                .id("git-confirm-cancel")
                                .cursor_pointer()
                                .on_click(cx.listener(|app, _, _, cx| {
                                    app.git_confirmation = None;
                                    cx.notify();
                                })),
                        )
                        .child(
                            widgets::Button::new(theme, action_label)
                                .danger()
                                .build()
                                .id("git-confirm-action")
                                .cursor_pointer()
                                .on_click(cx.listener(|app, _, _, cx| {
                                    app.confirm_git_confirmation();
                                    cx.notify();
                                })),
                        ),
                ),
        )
    })
}

/// Lists the paths a bulk discard will restore, capped for the dialog.
fn bulk_discard_body(files: &[GitFile]) -> String {
    const SHOWN: usize = 8;
    let mut body = String::from(
        "This restores these paths from the index. Any staged changes remain. This cannot be undone.\n",
    );
    for file in files.iter().take(SHOWN) {
        body.push_str(&format!("\n· {} ({})", file.path, file.status));
    }
    if files.len() > SHOWN {
        body.push_str(&format!("\n…and {} more", files.len() - SHOWN));
    }
    body
}

#[cfg(test)]
mod tests {
    use super::*;
    use dsh_client_core::git::{GitFileSideStats, GitFileStats, GitShow};

    fn file(path: &str, status: &str, staged: bool, worktree: bool) -> GitFile {
        GitFile {
            path: path.to_owned(),
            status: status.to_owned(),
            staged,
            worktree,
            orig_path: None,
            stats: None,
        }
    }

    fn with_stats(
        mut file: GitFile,
        index: Option<(u64, u64)>,
        worktree: Option<(u64, u64)>,
    ) -> GitFile {
        let side = |pair: Option<(u64, u64)>| {
            pair.map(|(additions, deletions)| GitFileSideStats {
                additions,
                deletions,
            })
        };
        file.stats = Some(GitFileStats {
            index: side(index),
            worktree: side(worktree),
        });
        file
    }

    fn status(ahead: u64, behind: u64, upstream: Option<&str>) -> GitStatus {
        GitStatus {
            branch: Some("main".to_owned()),
            commit: Some("aaaaaaa".to_owned()),
            detached: false,
            upstream: upstream.map(str::to_owned),
            ahead,
            behind,
            files: Vec::new(),
        }
    }

    #[test]
    fn groups_separate_staged_changes_and_untracked_paths() {
        let files = vec![
            file("src/a.ts", "modified", true, true),
            file("src/b.ts", "modified", false, true),
            file("docs/new.md", "untracked", false, true),
            file("src/c.ts", "added", true, false),
        ];
        let groups = group_files(&files, "");
        assert_eq!(
            groups
                .staged
                .iter()
                .map(|file| file.path.as_str())
                .collect::<Vec<_>>(),
            ["src/a.ts", "src/c.ts"]
        );
        assert_eq!(
            groups
                .changes
                .iter()
                .map(|file| file.path.as_str())
                .collect::<Vec<_>>(),
            ["src/a.ts", "src/b.ts"]
        );
        assert_eq!(
            groups
                .untracked
                .iter()
                .map(|file| file.path.as_str())
                .collect::<Vec<_>>(),
            ["docs/new.md"]
        );
        assert_eq!(groups.unique_count(), 4);
        assert!(!groups.is_empty());
    }

    #[test]
    fn filter_matches_paths_case_insensitively_and_keeps_groups_empty() {
        let files = vec![
            file("src/Runtime/Checkpoint.ts", "modified", false, true),
            file("docs/plan.md", "modified", false, true),
        ];
        let filtered = group_files(&files, "runtime");
        assert_eq!(filtered.changes.len(), 1);
        assert_eq!(filtered.changes[0].path, "src/Runtime/Checkpoint.ts");
        assert!(group_files(&files, "missing").is_empty());
        assert_eq!(group_files(&files, "  ").changes.len(), 2);
    }

    #[test]
    fn side_stats_read_one_side_and_never_sum_them() {
        let tracked = with_stats(
            file("src/a.ts", "modified", true, true),
            Some((3, 1)),
            Some((9, 4)),
        );
        assert_eq!(
            side_stats(&tracked, true),
            Some(GitFileSideStats {
                additions: 3,
                deletions: 1
            })
        );
        assert_eq!(
            side_stats(&tracked, false),
            Some(GitFileSideStats {
                additions: 9,
                deletions: 4
            })
        );
        let untracked = file("docs/new.md", "untracked", false, true);
        assert_eq!(side_stats(&untracked, false), None);
    }

    #[test]
    fn staged_summary_counts_only_the_index_side() {
        let files = vec![
            with_stats(
                file("src/a.ts", "modified", true, true),
                Some((3, 1)),
                Some((9, 4)),
            ),
            with_stats(file("src/b.ts", "added", true, false), Some((5, 0)), None),
            file("src/c.ts", "modified", false, true),
        ];
        assert_eq!(
            staged_summary(&files),
            StagedSummary {
                files: 2,
                additions: 8,
                deletions: 1
            }
        );
    }

    #[test]
    fn reviewed_signature_changes_with_the_status_entry() {
        let before = with_stats(
            file("src/a.ts", "modified", false, true),
            None,
            Some((2, 0)),
        );
        let same = before.clone();
        let after = with_stats(
            file("src/a.ts", "modified", false, true),
            None,
            Some((3, 0)),
        );
        assert_eq!(file_signature(&before), file_signature(&same));
        assert_ne!(file_signature(&before), file_signature(&after));
    }

    #[test]
    fn patch_signature_changes_with_the_patch() {
        let first = patch_signature("src/a.ts", false, "@@ -1 +1 @@\n-a\n+b\n");
        assert_eq!(
            first,
            patch_signature("src/a.ts", false, "@@ -1 +1 @@\n-a\n+b\n")
        );
        assert_ne!(
            first,
            patch_signature("src/a.ts", false, "@@ -1 +1 @@\n-a\n+c\n")
        );
        assert_ne!(
            first,
            patch_signature("src/a.ts", true, "@@ -1 +1 @@\n-a\n+b\n")
        );
    }

    const PATCH: &str = concat!(
        "diff --git a/src/a.ts b/src/a.ts\n",
        "index 1111111..2222222 100644\n",
        "--- a/src/a.ts\n",
        "+++ b/src/a.ts\n",
        "@@ -12,3 +12,4 @@ function run() {\n",
        " const first = 1;\n",
        "-const second = 2;\n",
        "+const second = 3;\n",
        "+const third = 4;\n",
        " return first;\n",
        "@@ -40,2 +41,2 @@ tail\n",
        " one\n",
        "-two\n",
        "+TWO\n",
    );

    #[test]
    fn parse_diff_keeps_header_and_numbers_both_gutters() {
        let parsed = parse_diff(PATCH);
        assert_eq!(parsed.header.len(), 4);
        assert_eq!(parsed.header[0], "diff --git a/src/a.ts b/src/a.ts");
        assert_eq!(parsed.hunks.len(), 2);

        let first = &parsed.hunks[0];
        assert_eq!(first.header, "@@ -12,3 +12,4 @@ function run() {");
        assert_eq!(first.lines.len(), 5);
        assert_eq!(first.lines[0].kind, ' ');
        assert_eq!(
            (first.lines[0].old_line, first.lines[0].new_line),
            (Some(12), Some(12))
        );
        assert_eq!(first.lines[1].kind, '-');
        assert_eq!(
            (first.lines[1].old_line, first.lines[1].new_line),
            (Some(13), None)
        );
        assert_eq!(first.lines[2].kind, '+');
        assert_eq!(
            (first.lines[2].old_line, first.lines[2].new_line),
            (None, Some(13))
        );
        assert_eq!(
            (first.lines[3].old_line, first.lines[3].new_line),
            (None, Some(14))
        );
        assert_eq!(
            (first.lines[4].old_line, first.lines[4].new_line),
            (Some(14), Some(15))
        );

        let second = &parsed.hunks[1];
        assert_eq!(
            (second.lines[1].old_line, second.lines[1].new_line),
            (Some(41), None)
        );
        assert_eq!(
            (second.lines[2].old_line, second.lines[2].new_line),
            (None, Some(42))
        );
    }

    #[test]
    fn hunk_patch_rebuilds_one_hunk_behind_the_file_header() {
        let parsed = parse_diff(PATCH);
        let patch = parsed.hunk_patch(0).expect("first hunk");
        assert!(patch.starts_with("diff --git a/src/a.ts b/src/a.ts\n"));
        assert!(patch.contains("--- a/src/a.ts\n+++ b/src/a.ts\n@@ -12,3 +12,4 @@"));
        assert!(!patch.contains("@@ -40,3 +41,3 @@"));
        assert_eq!(parse_diff(&patch).hunks.len(), 1);
        assert!(parsed.hunk_patch(9).is_none());
    }

    #[test]
    fn parse_diff_reports_binary_and_headerless_patches_as_hunks_only() {
        let binary = parse_diff("Binary files a/logo.png and b/logo.png differ\n");
        assert_eq!(
            binary.header,
            vec!["Binary files a/logo.png and b/logo.png differ".to_owned()]
        );
        assert!(binary.hunks.is_empty());
        assert!(binary.hunk_patch(0).is_none());
    }

    #[test]
    fn parse_diff_marks_a_missing_final_newline_without_advancing_gutters() {
        let patch = "diff --git a/a.txt b/a.txt\n--- a/a.txt\n+++ b/a.txt\n@@ -1 +1 @@\n-old\n\\ No newline at end of file\n+new\n";
        let parsed = parse_diff(patch);
        let hunk = &parsed.hunks[0];
        assert_eq!(hunk.lines[1].kind, '\\');
        assert_eq!(
            (hunk.lines[1].old_line, hunk.lines[1].new_line),
            (None, None)
        );
        assert!(parsed
            .hunk_patch(0)
            .expect("hunk")
            .contains("\\ No newline at end of file\n"));
    }

    #[test]
    fn change_badges_name_each_change_kind() {
        assert_eq!(change_badge("added"), "+");
        assert_eq!(change_badge("deleted"), "-");
        assert_eq!(change_badge("untracked"), "?");
        assert_eq!(change_badge("modified"), "M");
    }

    #[test]
    fn binary_patches_are_reported_as_binary() {
        let text = GitDiff {
            path: "a.txt".to_owned(),
            staged: false,
            patch: "@@\n+hello".to_owned(),
            truncated: false,
            untracked: false,
        };
        assert!(!git_diff_is_binary(&text));
        let binary = GitDiff {
            path: "image.png".to_owned(),
            staged: false,
            patch: "Binary files a/image.png and b/image.png differ\n".to_owned(),
            truncated: false,
            untracked: false,
        };
        assert!(git_diff_is_binary(&binary));
    }

    #[test]
    fn discard_is_limited_to_worktree_changes_of_tracked_files() {
        assert!(can_discard("modified"));
        assert!(can_discard("deleted"));
        assert!(can_discard("typechange"));
        assert!(!can_discard("untracked"));
        assert!(!can_discard("renamed"));
        assert!(!can_discard("conflict"));
    }

    #[test]
    fn short_commit_keeps_at_most_seven_characters() {
        assert_eq!(short_commit("0123456789abcdef"), "0123456");
        assert_eq!(short_commit("abc"), "abc");
        assert_eq!(short_commit(""), "");
    }

    #[test]
    fn head_is_pushed_only_with_an_upstream_and_no_local_commits() {
        assert!(head_is_pushed(&status(0, 3, Some("origin/main"))));
        assert!(!head_is_pushed(&status(2, 0, Some("origin/main"))));
        assert!(!head_is_pushed(&status(0, 0, None)));
    }

    #[test]
    fn commit_blocker_names_the_first_missing_precondition() {
        assert_eq!(
            commit_blocker(0, "", true, true),
            Some("A Git action is running; wait for its result.")
        );
        assert_eq!(
            commit_blocker(1, "msg", false, true),
            Some("Finish the in-progress Git operation before committing.")
        );
        assert_eq!(
            commit_blocker(0, "msg", false, false),
            Some("Stage at least one change to commit.")
        );
        assert_eq!(
            commit_blocker(1, "   ", false, false),
            Some("Enter a commit message.")
        );
        assert_eq!(commit_blocker(1, "msg", false, false), None);
    }

    #[test]
    fn push_blocker_requires_an_upstream_and_a_non_behind_branch() {
        assert_eq!(
            push_blocker(Some("Enter a commit message."), Some("origin/main"), 0),
            Some("Enter a commit message.".to_owned())
        );
        assert_eq!(
            push_blocker(None, None, 0),
            Some("Publishing a branch needs an upstream; that flow lands in G3.".to_owned())
        );
        assert!(push_blocker(None, Some("origin/main"), 2)
            .expect("blocked")
            .contains("pull before pushing"));
        assert_eq!(push_blocker(None, Some("origin/main"), 0), None);
    }

    #[test]
    fn sync_tooltip_names_the_upstream_and_both_counts() {
        assert_eq!(sync_tooltip(2, 1, Some("origin/main")), "2 ahead · 1 behind origin/main");
        assert_eq!(sync_tooltip(0, 1, None), "0 ahead · 1 behind · no upstream branch");
    }

    /// One `git/log` entry for the History tests.
    fn history_entry(oid: &str, parents: &[&str], refs: &[&str]) -> GitCommitEntry {
        GitCommitEntry {
            oid: oid.to_owned(),
            parents: parents.iter().map(|parent| (*parent).to_owned()).collect(),
            refs: refs.iter().map(|reference| (*reference).to_owned()).collect(),
            subject: format!("subject {oid}"),
            author_name: "Ada".to_owned(),
            author_email: "ada@example.com".to_owned(),
            author_time: "2026-11-08T10:00:00+07:00".to_owned(),
        }
    }

    /// The merge history every join and merge case lays out.
    fn merge_history() -> Vec<GitCommitEntry> {
        vec![
            history_entry("m", &["a", "b"], &["HEAD", "main"]),
            history_entry("a", &["base"], &[]),
            history_entry("b", &["base"], &[]),
            history_entry("base", &[], &[]),
        ]
    }

    #[test]
    fn graph_keeps_a_linear_history_in_one_lane() {
        let commits = vec![
            history_entry("c3", &["c2"], &["HEAD", "main"]),
            history_entry("c2", &["c1"], &[]),
            history_entry("c1", &[], &[]),
        ];
        let layout = layout_graph(&commits);
        assert_eq!(layout.lanes, 1);
        assert_eq!(layout.rows.len(), 3);
        assert!(layout.rows.iter().all(|row| row.lane == 0));
        assert!(layout
            .rows
            .iter()
            .all(|row| row.joins.is_empty() && row.extra.is_empty()));
        assert_eq!(layout.rows[0].before, vec![None]);
        assert_eq!(layout.rows[1].before, vec![Some("c2".to_owned())]);
    }

    #[test]
    fn graph_curves_an_extra_parent_into_its_own_lane() {
        let layout = layout_graph(&merge_history());
        assert_eq!(layout.lanes, 2);
        assert_eq!(layout.rows[0].lane, 0);
        assert_eq!(layout.rows[0].extra, vec![("b".to_owned(), 1)]);
        assert_eq!(layout.rows[1].lane, 0);
        assert_eq!(layout.rows[1].before, vec![Some("a".to_owned()), Some("b".to_owned())]);
        assert_eq!(layout.rows[2].lane, 1);
    }

    #[test]
    fn graph_joins_a_lane_that_waits_for_an_already_listed_commit() {
        let layout = layout_graph(&merge_history());
        let base = &layout.rows[3];
        assert_eq!(base.lane, 0);
        assert_eq!(base.joins, vec![1]);
        assert_eq!(
            base.before,
            vec![Some("base".to_owned()), Some("base".to_owned())]
        );
        assert!(base.extra.is_empty());
        assert_eq!(layout.lanes, 2);
    }

    #[test]
    fn graph_gives_an_octopus_three_stable_lanes() {
        let commits = vec![
            history_entry("h", &["a", "b", "c"], &["HEAD"]),
            history_entry("a", &["root"], &[]),
            history_entry("b", &["root"], &[]),
            history_entry("c", &["root"], &[]),
            history_entry("root", &[], &[]),
        ];
        let layout = layout_graph(&commits);
        assert_eq!(layout.lanes, 3);
        assert_eq!(layout.rows[0].lane, 0);
        assert_eq!(
            layout.rows[0].extra,
            vec![("b".to_owned(), 1), ("c".to_owned(), 2)]
        );
        assert_eq!(layout.rows[1].lane, 0);
        assert_eq!(layout.rows[2].lane, 1);
        assert_eq!(layout.rows[3].lane, 2);
        assert_eq!(layout.rows[4].joins, vec![1, 2]);
    }

    #[test]
    fn refs_ring_head_and_name_each_chip() {
        let refs = vec![
            "HEAD".to_owned(),
            "main".to_owned(),
            "origin/main".to_owned(),
            "tag:v1.0".to_owned(),
            "feat/history".to_owned(),
        ];
        let parsed = history_refs(&refs, &["feat/history".to_owned()]);
        assert!(parsed.head);
        assert_eq!(
            parsed.chips,
            vec![
                HistoryRef {
                    name: "main".to_owned(),
                    kind: RefKind::Branch,
                },
                HistoryRef {
                    name: "origin/main".to_owned(),
                    kind: RefKind::Remote,
                },
                HistoryRef {
                    name: "tag:v1.0".to_owned(),
                    kind: RefKind::Tag,
                },
                HistoryRef {
                    name: "feat/history".to_owned(),
                    kind: RefKind::Branch,
                },
            ]
        );
    }

    #[test]
    fn refs_keep_head_out_of_the_chips() {
        let parsed = history_refs(&["HEAD".to_owned()], &[]);
        assert!(parsed.head);
        assert!(parsed.chips.is_empty());
    }

    #[test]
    fn refs_treat_an_unloaded_slash_name_as_a_remote_branch() {
        let parsed = history_refs(&["feat/history".to_owned(), "v2".to_owned()], &[]);
        assert!(!parsed.head);
        assert_eq!(parsed.chips[0].kind, RefKind::Remote);
        assert_eq!(parsed.chips[1].kind, RefKind::Branch);
    }

    #[test]
    fn history_footer_pages_only_after_a_full_page() {
        assert_eq!(
            history_footer(50, 50, 50, false),
            HistoryFooter::LoadOlder { skip: 50 }
        );
        assert_eq!(
            history_footer(50, 100, 50, false),
            HistoryFooter::LoadOlder { skip: 100 }
        );
        assert_eq!(history_footer(12, 62, 50, false), HistoryFooter::End);
        assert_eq!(history_footer(0, 50, 50, false), HistoryFooter::End);
    }

    #[test]
    fn history_footer_shows_the_host_cap_instead_of_paging() {
        assert_eq!(history_footer(50, 50, 50, true), HistoryFooter::Truncated);
        assert_eq!(history_footer(8, 58, 50, true), HistoryFooter::Truncated);
    }

    #[test]
    fn history_selection_clamps_and_starts_at_the_newest_row() {
        assert_eq!(stepped_history_selection(None, 1, 3), Some(0));
        assert_eq!(stepped_history_selection(None, -1, 3), Some(0));
        assert_eq!(stepped_history_selection(Some(0), 1, 3), Some(1));
        assert_eq!(stepped_history_selection(Some(2), 1, 3), Some(2));
        assert_eq!(stepped_history_selection(Some(1), -1, 3), Some(0));
        assert_eq!(stepped_history_selection(Some(0), -1, 3), Some(0));
        assert_eq!(stepped_history_selection(Some(0), 1, 0), None);
    }

    /// Opens the shell on the History tab with a seeded page.
    fn history_harness(setup: impl FnOnce(&mut DesktopApp)) -> crate::harness::Harness {
        crate::harness::Harness::open_with(1366., 768., move |app| {
            app.changes_tab = ChangesTab::History;
            app.git_history = vec![
                history_entry("c3", &["c2"], &["HEAD", "main"]),
                history_entry("c2", &["c1"], &[]),
                history_entry("c1", &[], &[]),
            ];
            app.git_history_loaded = true;
            setup(app);
        })
    }

    #[test]
    fn history_tab_renders_rows_and_moves_the_selection_with_arrows() {
        let mut harness = history_harness(|_| {});
        assert!(harness.bounds("git-history-panel").is_some());
        assert!(harness.bounds("git-history-row-first").is_some());
        assert!(harness.bounds("git-history-detail").is_none());
        assert!(harness.click("git-history-row-first"));
        assert_eq!(
            harness.with(|app, _| app.git_history_selected.clone()),
            Some("c3".to_owned())
        );
        assert!(harness.bounds("git-history-row-selected").is_some());
        harness.keys("down");
        assert_eq!(
            harness.with(|app, _| app.git_history_selected.clone()),
            Some("c2".to_owned())
        );
        harness.keys("up");
        assert_eq!(
            harness.with(|app, _| app.git_history_selected.clone()),
            Some("c3".to_owned())
        );
    }

    #[test]
    fn history_detail_renders_the_selected_commit_patch() {
        let mut harness = history_harness(|app| {
            app.git_history_selected = Some("c2".to_owned());
            app.git_history_show = Some(GitShow {
                commit: "c2".to_owned(),
                patch: "diff --git a/a.ts b/a.ts\n+one\n".to_owned(),
                truncated: false,
            });
        });
        assert!(harness.bounds("git-history-detail").is_some());
        assert!(harness.bounds("git-history-patch").is_some());
        assert!(harness.bounds("git-history-patch-empty").is_none());
        assert!(harness.bounds("git-history-cherry").is_some());
        assert!(harness.bounds("git-history-revert").is_some());
    }

    #[test]
    fn history_detail_names_an_empty_and_a_truncated_patch() {
        let mut empty = history_harness(|app| {
            app.git_history_selected = Some("c1".to_owned());
            app.git_history_show = Some(GitShow {
                commit: "c1".to_owned(),
                patch: String::new(),
                truncated: false,
            });
        });
        assert!(empty.bounds("git-history-patch-empty").is_some());
        let mut truncated = history_harness(|app| {
            app.git_history_selected = Some("c1".to_owned());
            app.git_history_show = Some(GitShow {
                commit: "c1".to_owned(),
                patch: "diff --git a/a.ts b/a.ts\n+one\n".to_owned(),
                truncated: true,
            });
        });
        assert!(truncated.bounds("git-history-patch-truncated").is_some());
        assert!(truncated.bounds("git-history-patch").is_some());
    }

    #[test]
    fn history_detail_names_a_failed_or_pending_patch() {
        let mut failed = history_harness(|app| {
            app.git_history_selected = Some("c2".to_owned());
            app.git_history_show_error = Some("git show failed".to_owned());
        });
        assert!(failed.bounds("git-history-patch-error").is_some());
        assert!(failed.bounds("git-history-patch").is_none());
        let mut pending = history_harness(|app| {
            app.git_history_selected = Some("c2".to_owned());
            app.git_history_show_loading = true;
        });
        assert!(pending.bounds("git-history-patch-loading").is_some());
    }

    #[test]
    fn history_list_names_an_empty_repository() {
        let mut harness = history_harness(|app| app.git_history.clear());
        assert!(harness.bounds("git-history-empty").is_some());
        assert!(harness.bounds("git-history-row-first").is_none());
    }

    #[test]
    fn history_list_names_a_failed_page() {
        let mut harness = history_harness(|app| {
            app.git_history.clear();
            app.git_history_error = Some("git log failed".to_owned());
        });
        assert!(harness.bounds("git-history-error").is_some());
        assert!(harness.bounds("git-history-empty").is_none());
    }

    #[test]
    fn history_list_offers_load_older_only_for_an_uncapped_page() {
        let mut paged = history_harness(|app| {
            app.git_history_footer = HistoryFooter::LoadOlder { skip: 50 };
        });
        assert!(paged.bounds("git-history-load-older").is_some());
        assert!(paged.bounds("git-history-truncated").is_none());
        let mut capped = history_harness(|app| {
            app.git_history_footer = HistoryFooter::Truncated;
        });
        assert!(capped.bounds("git-history-truncated").is_some());
        assert!(capped.bounds("git-history-load-older").is_none());
    }

    #[test]
    fn bulk_discard_body_lists_paths_and_caps_the_list() {
        let files = (0..10)
            .map(|index| file(&format!("src/{index}.ts"), "modified", false, true))
            .collect::<Vec<_>>();
        let body = bulk_discard_body(&files);
        assert!(body.contains("· src/0.ts (modified)"));
        assert!(body.contains("…and 2 more"));
        assert!(!body.contains("· src/9.ts"));
    }
}
