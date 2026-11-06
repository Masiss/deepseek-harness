//! Session-scoped workspace directory tree and read-only text preview.
//!
//! The tree lists directories lazily and watches the directories it shows plus
//! the previewed file through `workspaceFiles/changes`, so a change on disk
//! refreshes the affected listing and flags the preview instead of leaving
//! stale content on screen. Watch streams belong to the explorer: closing the
//! session, collapsing a directory, or closing the preview drops them.

use dsh_client_core::{
    remote_stream::RemoteStreamMux,
    streams::workspace_file_changes,
    transport::HttpTransport,
    workspace_files::{
        WorkspaceDirectoryListing, WorkspaceFileRange, WorkspaceFileScope, WorkspaceFileText,
        WorkspaceFileWatchFrame, WorkspaceFilesClient,
    },
};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::mpsc;

const FIRST_PAGE_LINES: u64 = 300;

/// Gives a button a stable AccessKit name for the legacy windowless UI harness.

/// One open `workspaceFiles/changes` generation for a directory or the previewed file.
struct PathWatch {
    session_id: String,
    /// Workspace-relative path; empty for the workspace root.
    path: String,
    /// True when the watch reports the previewed file instead of a directory.
    file: bool,
    /// Last version the Host reported, used to ignore duplicate notifications.
    version: Option<String>,
    /// When the last non-duplicate change arrived, for cross-panel refresh.
    changed_at: Option<std::time::Instant>,
    receiver: mpsc::Receiver<Result<WorkspaceFileWatchFrame, String>>,
}

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
    watches: Vec<PathWatch>,
    /// Paths whose watch the Host refused, so a frame loop does not retry them.
    watch_failures: BTreeSet<String>,
    /// Why live refresh is unavailable, when the Host refused a watch.
    watch_error: Option<String>,
    /// The previewed file changed on disk after the visible page was read.
    preview_stale: bool,
    /// The previewed file is gone from disk; its last page stays visible.
    preview_absent: bool,
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
        self.watches.clear();
        self.watch_failures.clear();
        self.watch_error = None;
        self.preview_stale = false;
        self.preview_absent = false;
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
        self.reset_watch_failures();
        self.start_listing(transport, session_id, String::new());
    }

    /// Opens and closes live watches so they match the visible tree.
    ///
    /// The Host watches one target per stream, so the explorer keeps a
    /// generation per visible directory plus the previewed file.
    pub(crate) fn sync_watches(&mut self, streams: Option<&RemoteStreamMux>) {
        let (Some(session_id), Some(streams)) = (self.session_id.clone(), streams) else {
            self.watches.clear();
            return;
        };
        let mut desired: Vec<(String, bool)> = Vec::new();
        for path in self.expanded.iter() {
            desired.push((path.clone(), false));
        }
        if let Some(path) = self.selected_path.clone() {
            desired.push((path, true));
        }
        self.watches.retain(|watch| {
            watch.session_id == session_id
                && desired
                    .iter()
                    .any(|(path, file)| path == &watch.path && *file == watch.file)
        });
        for (path, file) in desired {
            if self.watch_failures.contains(&path) {
                continue;
            }
            if self
                .watches
                .iter()
                .any(|watch| watch.path == path && watch.file == file)
            {
                continue;
            }
            self.watches
                .push(spawn_watch(streams, session_id.clone(), path, file));
        }
    }

    /// Applies completed Host reads and watch frames on the UI thread.
    ///
    /// @param transport - Host transport used to re-list a directory a watch reported.
    /// @returns Whether visible state changed, so the shell repaints.
    pub(crate) fn poll(&mut self, transport: Option<&HttpTransport>) -> bool {
        let mut dirty = self.drain_watch_frames(transport);
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
                            self.directories.insert(path.clone(), listing);
                            self.prune_missing_children(&path);
                            dirty = true;
                        }
                        Err(error) => {
                            self.list_errors.insert(pending.path, error);
                            dirty = true;
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
                        dirty = true;
                    }
                }
                Err(mpsc::TryRecvError::Empty) => index += 1,
            }
        }

        let Some(pending) = &self.pending_read else {
            return dirty;
        };
        let result = match pending.receiver.try_recv() {
            Ok(result) => Some(result),
            Err(mpsc::TryRecvError::Disconnected) => {
                Some(Err("File preview stopped unexpectedly".to_owned()))
            }
            Err(mpsc::TryRecvError::Empty) => None,
        };
        let Some(result) = result else {
            return dirty;
        };
        let Some(pending) = self.pending_read.take() else {
            return dirty;
        };
        if self.session_id.as_deref() != Some(pending.session_id.as_str())
            || self.selected_path.as_deref() != Some(pending.path.as_str())
        {
            return dirty;
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
                    self.preview_stale = false;
                    self.preview_absent = false;
                }
            }
            Ok(page) => {
                self.text = Some(page);
                self.read_error = None;
                self.preview_stale = false;
                self.preview_absent = false;
            }
            Err(error) if error.contains("workspace-file/not-text") => {
                self.read_error = Some(
                    "This file is not UTF-8 text. Use Document preview for supported images and Office files.".to_owned(),
                );
            }
            Err(error) => self.read_error = Some(error),
        }
        true
    }

    /// Consumes live watch frames and refreshes what each one invalidates.
    ///
    /// @returns Whether a tree, preview, or notice changed.
    fn drain_watch_frames(&mut self, transport: Option<&HttpTransport>) -> bool {
        let mut dirty = false;
        let mut index = 0;
        while index < self.watches.len() {
            let received = self.watches[index].receiver.try_recv();
            match received {
                Ok(Ok(WorkspaceFileWatchFrame::Ready)) => index += 1,
                Ok(Ok(WorkspaceFileWatchFrame::Change { change })) => {
                    let watch = &self.watches[index];
                    let path = watch.path.clone();
                    let file = watch.file;
                    let session_id = watch.session_id.clone();
                    let previous = watch.version.clone();
                    let absent = change.absent.unwrap_or(false);
                    let version = if absent { None } else { change.version };
                    self.watches[index].version = version.clone();
                    let unchanged = previous.is_some() && previous == version;
                    if !unchanged || absent {
                        self.watches[index].changed_at = Some(std::time::Instant::now());
                    }
                    if self.session_id.as_deref() != Some(session_id.as_str())
                        || (unchanged && !absent)
                    {
                        index += 1;
                        continue;
                    }
                    if file {
                        self.note_preview_change(version.as_deref(), absent);
                        dirty = true;
                    } else if let Some(transport) = transport {
                        self.start_listing(transport, session_id, path);
                    }
                    index += 1;
                }
                Ok(Err(error)) => {
                    let watch = self.watches.remove(index);
                    self.watch_failures.insert(watch.path);
                    self.watch_error = Some(format!("Live refresh unavailable: {error}"));
                    dirty = true;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    let watch = self.watches.remove(index);
                    self.watch_failures.insert(watch.path);
                    self.watch_error = Some(
                        "Live refresh stopped. Press refresh to watch the workspace again."
                            .to_owned(),
                    );
                    dirty = true;
                }
                Err(mpsc::TryRecvError::Empty) => index += 1,
            }
        }
        dirty
    }

    /// Marks the open preview when its file changes or disappears on disk.
    fn note_preview_change(&mut self, version: Option<&str>, absent: bool) {
        if self.selected_path.is_none() {
            return;
        }
        if absent {
            self.preview_absent = true;
            self.preview_stale = false;
            return;
        }
        let current = self.text.as_ref().map(|text| text.stat.version.as_str());
        if let Some(current) = current {
            self.preview_stale = Some(current) != version;
            self.preview_absent = false;
        }
    }

    /// Drops expanded directories and listings a refreshed directory no longer contains.
    fn prune_missing_children(&mut self, parent: &str) {
        let Some(listing) = self.directories.get(parent) else {
            return;
        };
        // A truncated listing omits Host-limited entries, so absence proves nothing.
        if listing.truncated {
            return;
        }
        let present: BTreeSet<&str> = listing
            .entries
            .iter()
            .filter(|entry| entry.entry_type == "directory")
            .map(|entry| entry.name.as_str())
            .collect();
        let stale: Vec<String> = self
            .expanded
            .iter()
            .chain(self.directories.keys())
            .filter(|path| {
                !path.is_empty() && parent_of(path) == parent && !present.contains(name_of(path))
            })
            .cloned()
            .collect();
        for path in stale {
            let prefix = format!("{path}/");
            self.expanded
                .retain(|key| key != &path && !key.starts_with(&prefix));
            self.directories
                .retain(|key, _| key != &path && !key.starts_with(&prefix));
            self.list_errors
                .retain(|key, _| key != &path && !key.starts_with(&prefix));
            self.watch_failures
                .retain(|key| key != &path && !key.starts_with(&prefix));
            self.watches
                .retain(|watch| watch.path != path && !watch.path.starts_with(&prefix));
        }
    }

    /// Renders the current Session's workspace tree and read-only text pane.





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

/// One visible row of the workspace tree, flattened for the gpui Explorer panel.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) struct ExplorerRow {
    /// Workspace-relative path; empty for the root notices.
    pub path: String,
    /// Text painted for the row, without indentation or glyph decoration.
    pub label: String,
    /// Nesting level below the workspace root.
    pub depth: usize,
    pub kind: ExplorerRowKind,
}

/// What an [`ExplorerRow`] represents and how it reacts to a click.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) enum ExplorerRowKind {
    /// A directory; `expanded` selects the disclosure state.
    Directory { expanded: bool },
    /// A file the Session may preview; `selected` marks the open preview.
    File { selected: bool },
    /// A symbolic link the Host reports without following it.
    Symlink,
    /// An entry type this client does not render.
    Unsupported,
    /// A listing failure the user can retry.
    Error,
    /// A listing request still in flight.
    Loading,
    /// Static information such as an omitted-entry notice.
    Notice,
}

/// Read-only accessors and row projection consumed by the gpui Explorer panel.
#[allow(dead_code)]
impl WorkspaceExplorer {
    /// The Session whose workspace this explorer displays.
    pub(crate) fn session(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    /// The workspace-relative path whose text preview is open.
    pub(crate) fn preview_path(&self) -> Option<&str> {
        self.selected_path.as_deref()
    }

    /// The loaded preview page, if any.
    pub(crate) fn preview(&self) -> Option<&WorkspaceFileText> {
        self.text.as_ref()
    }

    /// The most recent preview failure, if any.
    pub(crate) fn preview_error(&self) -> Option<&str> {
        self.read_error.as_deref()
    }

    /// Whether a preview request is still in flight.
    pub(crate) fn preview_pending(&self) -> bool {
        self.pending_read.is_some()
    }

    /// Flattens the tree in draw order, honouring the expanded set.
    pub(crate) fn visible_rows(&self) -> Vec<ExplorerRow> {
        let mut rows = Vec::new();
        let Some(root) = self.directories.get("") else {
            if let Some(error) = self.list_errors.get("") {
                rows.push(ExplorerRow {
                    path: String::new(),
                    label: format!("Could not list workspace: {error}"),
                    depth: 0,
                    kind: ExplorerRowKind::Error,
                });
            } else if self.is_listing_pending("") {
                rows.push(ExplorerRow {
                    path: String::new(),
                    label: "Loading workspace…".to_owned(),
                    depth: 0,
                    kind: ExplorerRowKind::Loading,
                });
            } else {
                rows.push(ExplorerRow {
                    path: String::new(),
                    label: "Workspace tree is not loaded.".to_owned(),
                    depth: 0,
                    kind: ExplorerRowKind::Notice,
                });
            }
            return rows;
        };
        if root.entries.is_empty() {
            rows.push(ExplorerRow {
                path: String::new(),
                label: "This workspace is empty.".to_owned(),
                depth: 0,
                kind: ExplorerRowKind::Notice,
            });
            return rows;
        }
        self.collect_rows("", 0, &mut rows);
        if root.truncated {
            rows.push(ExplorerRow {
                path: String::new(),
                label: "More entries omitted by the Host limit.".to_owned(),
                depth: 0,
                kind: ExplorerRowKind::Notice,
            });
        }
        rows
    }

    fn collect_rows(&self, parent: &str, depth: usize, rows: &mut Vec<ExplorerRow>) {
        let Some(listing) = self.directories.get(parent) else {
            return;
        };
        for entry in &listing.entries {
            let path = join_workspace_path(parent, &entry.name);
            match entry.entry_type.as_str() {
                "directory" => {
                    let expanded = self.expanded.contains(&path);
                    rows.push(ExplorerRow {
                        path: path.clone(),
                        label: entry.name.clone(),
                        depth,
                        kind: ExplorerRowKind::Directory { expanded },
                    });
                    if !expanded {
                        continue;
                    }
                    if let Some(error) = self.list_errors.get(&path) {
                        rows.push(ExplorerRow {
                            path: path.clone(),
                            label: error.clone(),
                            depth: depth + 1,
                            kind: ExplorerRowKind::Error,
                        });
                    } else if self.is_listing_pending(&path) {
                        rows.push(ExplorerRow {
                            path: path.clone(),
                            label: "Loading…".to_owned(),
                            depth: depth + 1,
                            kind: ExplorerRowKind::Loading,
                        });
                    } else if self.directories.contains_key(&path) {
                        self.collect_rows(&path, depth + 1, rows);
                    }
                }
                "file" => rows.push(ExplorerRow {
                    path: path.clone(),
                    label: entry.name.clone(),
                    depth,
                    kind: ExplorerRowKind::File {
                        selected: self.selected_path.as_deref() == Some(path.as_str()),
                    },
                }),
                "symlink" => rows.push(ExplorerRow {
                    path,
                    label: format!("{} · link", entry.name),
                    depth,
                    kind: ExplorerRowKind::Symlink,
                }),
                _ => rows.push(ExplorerRow {
                    path,
                    label: format!("{} · unsupported entry", entry.name),
                    depth,
                    kind: ExplorerRowKind::Unsupported,
                }),
            }
        }
        if listing.truncated {
            rows.push(ExplorerRow {
                path: String::new(),
                label: "More entries were omitted by the Host limit.".to_owned(),
                depth: depth + 1,
                kind: ExplorerRowKind::Notice,
            });
        }
    }

    /// Expands or collapses a directory, requesting its listing on first open.
    pub(crate) fn toggle_directory(
        &mut self,
        transport: Option<&HttpTransport>,
        session_id: &str,
        path: &str,
    ) {
        if self.expanded.remove(path) {
            return;
        }
        self.expanded.insert(path.to_owned());
        if self.directories.contains_key(path) {
            return;
        }
        if let Some(transport) = transport {
            self.start_listing(transport, session_id.to_owned(), path.to_owned());
        }
    }

    /// Opens a file's read-only preview.
    pub(crate) fn open_file(
        &mut self,
        transport: Option<&HttpTransport>,
        session_id: &str,
        path: &str,
    ) {
        if let Some(transport) = transport {
            self.select_file(transport, session_id.to_owned(), path.to_owned());
            self.show_file_pane = self.single_pane;
        }
    }

    /// Discards the preview so a narrow Explorer returns to the tree.
    pub(crate) fn close_preview(&mut self) {
        self.show_file_pane = false;
    }

    /// Whether a narrow Explorer currently shows the preview pane.
    pub(crate) fn preview_pane_visible(&self) -> bool {
        self.single_pane && self.show_file_pane
    }

    /// Applies the pane policy and watch set for the current width and Session.
    pub(crate) fn prepare(
        &mut self,
        transport: Option<&HttpTransport>,
        streams: Option<&RemoteStreamMux>,
        session_id: Option<&str>,
        single_pane: bool,
    ) {
        self.select_session(session_id, transport);
        self.single_pane = single_pane;
        if !single_pane {
            self.show_file_pane = false;
        }
        self.sync_watches(streams);
    }

    /// Retries one failed or stale listing.
    pub(crate) fn retry_listing(
        &mut self,
        transport: Option<&HttpTransport>,
        session_id: &str,
        path: &str,
    ) {
        if let Some(transport) = transport {
            self.start_listing(transport, session_id.to_owned(), path.to_owned());
        }
    }

    /// Reloads the open preview from its first line.
    pub(crate) fn reload_preview(
        &mut self,
        transport: Option<&HttpTransport>,
        session_id: &str,
        path: &str,
    ) {
        if let Some(transport) = transport {
            self.select_file(transport, session_id.to_owned(), path.to_owned());
        }
    }

    /// Appends the next page of the open preview.
    pub(crate) fn load_next_preview_page(
        &mut self,
        transport: Option<&HttpTransport>,
        session_id: &str,
        path: &str,
    ) {
        let (Some(transport), Some(text)) = (transport, self.text.as_ref()) else {
            return;
        };
        let offset = text.offset.saturating_add(text.lines);
        let version = text.stat.version.clone();
        self.read_page(
            transport,
            session_id.to_owned(),
            path.to_owned(),
            offset,
            true,
            Some(version),
        );
    }

    /// Allows refused watches to open again after a user refresh.
    fn reset_watch_failures(&mut self) {
        self.watch_failures.clear();
        self.watch_error = None;
    }

    /// Whether a root listing request is in flight.
    pub(crate) fn root_pending(&self) -> bool {
        self.is_listing_pending("")
    }

    /// Whether the previewed file changed on disk after its page was read.
    pub(crate) fn preview_stale(&self) -> bool {
        self.preview_stale
    }

    /// Whether the previewed file no longer exists on disk.
    pub(crate) fn preview_absent(&self) -> bool {
        self.preview_absent
    }

    /// Why live refresh is unavailable, after the Host refused or closed a watch.
    pub(crate) fn watch_error(&self) -> Option<&str> {
        self.watch_error.as_deref()
    }

    /// Whether the workspace watcher observed any change since the last check.
    ///
    /// The Changes panel uses this as its auto-refresh signal; agents, the file
    /// manager, and the terminal all mutate the same paths and cannot be polled
    /// through a Git status call on every frame.
    pub(crate) fn workspace_changed_since(&mut self, checked_at: Option<std::time::Instant>) -> Option<std::time::Instant> {
        let now = std::time::Instant::now();
        let changed = self
            .watches
            .iter()
            .any(|watch| watch.changed_at.is_some_and(|at| checked_at.is_none_or(|check| at > check)));
        if changed {
            Some(now)
        } else {
            checked_at
        }
    }
}

/// Opens one `workspaceFiles/changes` generation on a worker thread.
fn spawn_watch(
    streams: &RemoteStreamMux,
    session_id: String,
    path: String,
    file: bool,
) -> PathWatch {
    let (sender, receiver) = mpsc::channel();
    let streams = streams.clone();
    let scope = WorkspaceFileScope {
        session_id: session_id.clone(),
    };
    let request_path = if path.is_empty() {
        ".".to_owned()
    } else {
        path.clone()
    };
    std::thread::spawn(
        move || match workspace_file_changes(&streams, &scope, &request_path) {
            Ok(stream) => {
                for frame in stream {
                    if sender
                        .send(frame.map_err(|error| error.to_string()))
                        .is_err()
                    {
                        break;
                    }
                }
            }
            Err(error) => {
                let _ = sender.send(Err(error.to_string()));
            }
        },
    );
    PathWatch {
        session_id,
        path,
        file,
        version: None,
        changed_at: None,
        receiver,
    }
}

fn parent_of(path: &str) -> &str {
    match path.rfind('/') {
        Some(index) => &path[..index],
        None => "",
    }
}

fn name_of(path: &str) -> &str {
    match path.rfind('/') {
        Some(index) => &path[index + 1..],
        None => path,
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

#[cfg(test)]
mod tests {
    use super::*;
    use dsh_client_core::workspace_files::{WorkspaceDirectoryEntry, WorkspaceFileStat};
    use std::sync::mpsc;

    fn listing(path: &str, directories: &[&str]) -> WorkspaceDirectoryListing {
        WorkspaceDirectoryListing {
            path: path.to_owned(),
            entries: directories
                .iter()
                .map(|name| WorkspaceDirectoryEntry {
                    name: (*name).to_owned(),
                    entry_type: "directory".to_owned(),
                    size: None,
                })
                .collect(),
            truncated: false,
        }
    }

    fn watched(
        path: &str,
        file: bool,
    ) -> (
        PathWatch,
        mpsc::Sender<Result<WorkspaceFileWatchFrame, String>>,
    ) {
        let (sender, receiver) = mpsc::channel();
        (
            PathWatch {
                session_id: "s1".to_owned(),
                path: path.to_owned(),
                file,
                version: None,
                changed_at: None,
                receiver,
            },
            sender,
        )
    }

    fn explorer_with_session() -> WorkspaceExplorer {
        let mut explorer = WorkspaceExplorer::default();
        explorer.select_session(Some("s1"), None);
        explorer
    }

    fn change(path: &str, version: Option<&str>) -> WorkspaceFileWatchFrame {
        WorkspaceFileWatchFrame::Change {
            change: dsh_client_core::workspace_files::WorkspaceFileChange {
                absolute_path: format!("/w/{path}"),
                version: version.map(str::to_owned),
                absent: None,
            },
        }
    }

    fn absent(path: &str) -> WorkspaceFileWatchFrame {
        WorkspaceFileWatchFrame::Change {
            change: dsh_client_core::workspace_files::WorkspaceFileChange {
                absolute_path: format!("/w/{path}"),
                version: None,
                absent: Some(true),
            },
        }
    }

    #[test]
    fn repeated_directory_version_does_not_relist() {
        let mut explorer = explorer_with_session();
        let (watch, sender) = watched("", false);
        explorer.watches.push(watch);
        sender.send(Ok(change("", Some("v1")))).unwrap();
        explorer.drain_watch_frames(None);
        assert_eq!(explorer.watches[0].version.as_deref(), Some("v1"));
        sender.send(Ok(change("", Some("v1")))).unwrap();
        explorer.drain_watch_frames(None);
        assert_eq!(explorer.watches.len(), 1);
        sender.send(Ok(change("", Some("v2")))).unwrap();
        explorer.drain_watch_frames(None);
        assert_eq!(explorer.watches[0].version.as_deref(), Some("v2"));
    }

    #[test]
    fn only_a_real_watch_change_reports_external_mutation() {
        let mut explorer = explorer_with_session();
        let (watch, sender) = watched("", false);
        explorer.watches.push(watch);
        sender.send(Ok(change("", Some("v1")))).unwrap();
        explorer.drain_watch_frames(None);
        let checked = explorer.workspace_changed_since(None);
        assert!(checked.is_some());
        sender.send(Ok(change("", Some("v1")))).unwrap();
        explorer.drain_watch_frames(None);
        assert_eq!(explorer.workspace_changed_since(checked), checked);
        sender.send(Ok(change("", Some("v2")))).unwrap();
        explorer.drain_watch_frames(None);
        assert_ne!(explorer.workspace_changed_since(checked), checked);
    }

    #[test]
    fn preview_change_marks_the_open_page_stale() {
        let mut explorer = explorer_with_session();
        explorer.selected_path = Some("a.txt".to_owned());
        explorer.text = Some(WorkspaceFileText {
            stat: WorkspaceFileStat {
                absolute_path: "/w/a.txt".to_owned(),
                version: "v1".to_owned(),
                bytes: None,
            },
            offset: 1,
            text: "old".to_owned(),
            lines: 1,
            eof: true,
        });
        let (watch, sender) = watched("a.txt", true);
        explorer.watches.push(watch);
        sender.send(Ok(change("a.txt", Some("v2")))).unwrap();
        explorer.drain_watch_frames(None);
        assert!(explorer.preview_stale());
        assert!(!explorer.preview_absent());
        assert_eq!(explorer.preview().unwrap().text, "old");

        sender.send(Ok(change("a.txt", Some("v1")))).unwrap();
        explorer.drain_watch_frames(None);
        assert!(!explorer.preview_stale());
    }

    #[test]
    fn deleting_the_previewed_file_keeps_the_last_read() {
        let mut explorer = explorer_with_session();
        explorer.selected_path = Some("a.txt".to_owned());
        explorer.text = Some(WorkspaceFileText {
            stat: WorkspaceFileStat {
                absolute_path: "/w/a.txt".to_owned(),
                version: "v1".to_owned(),
                bytes: None,
            },
            offset: 1,
            text: "kept".to_owned(),
            lines: 1,
            eof: true,
        });
        let (watch, sender) = watched("a.txt", true);
        explorer.watches.push(watch);
        sender.send(Ok(absent("a.txt"))).unwrap();
        explorer.drain_watch_frames(None);
        assert!(explorer.preview_absent());
        assert_eq!(explorer.preview().unwrap().text, "kept");
        assert_eq!(explorer.preview_path(), Some("a.txt"));
    }

    #[test]
    fn refused_watch_reports_once_and_is_not_retried() {
        let mut explorer = explorer_with_session();
        let (watch, sender) = watched("src", false);
        explorer.watches.push(watch);
        sender
            .send(Err("workspace-file/watch-unsupported".to_owned()))
            .unwrap();
        explorer.drain_watch_frames(None);
        assert!(explorer.watches.is_empty());
        assert!(explorer
            .watch_error()
            .is_some_and(|error| error.contains("watch-unsupported")));
        explorer.sync_watches(None);
        assert!(explorer.watches.is_empty());
        explorer.reset_watch_failures();
        assert!(explorer.watch_error().is_none());
    }

    #[test]
    fn truncated_listing_keeps_expanded_directories() {
        let mut explorer = explorer_with_session();
        explorer.expanded.insert(String::new());
        explorer.expanded.insert("src".to_owned());
        explorer
            .directories
            .insert("src".to_owned(), listing("src", &[]));
        let mut truncated = listing("", &[]);
        truncated.truncated = true;
        explorer.directories.insert(String::new(), truncated);

        explorer.prune_missing_children("");

        assert!(explorer.directories.contains_key("src"));
        assert!(explorer.expanded.contains("src"));
    }

    #[test]
    fn refreshing_a_directory_drops_listings_it_no_longer_contains() {
        let mut explorer = explorer_with_session();
        explorer.expanded.insert(String::new());
        explorer.expanded.insert("src".to_owned());
        explorer.expanded.insert("src/nested".to_owned());
        explorer
            .directories
            .insert(String::new(), listing("", &["src"]));
        explorer
            .directories
            .insert("src".to_owned(), listing("src", &["nested"]));
        explorer
            .directories
            .insert("src/nested".to_owned(), listing("src/nested", &[]));
        explorer
            .list_errors
            .insert("src/nested".to_owned(), "boom".to_owned());

        explorer.directories.insert(String::new(), listing("", &[]));
        explorer.prune_missing_children("");

        assert!(explorer.directories.keys().all(|path| path.is_empty()));
        assert_eq!(
            explorer.expanded.iter().cloned().collect::<Vec<_>>(),
            vec![String::new()]
        );
        assert!(explorer.list_errors.is_empty());
    }
}
