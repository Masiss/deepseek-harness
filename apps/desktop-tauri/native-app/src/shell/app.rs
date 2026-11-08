//! Application state for the gpui shell.
//!
//! The Host transport, session projection, and worker channels live here; this
//! module owns their lifecycle and lets the renderer read a single state value.

use crate::command_registry::CommandId;
use crate::dialog;
use crate::document_preview::{ImageDocument, PreviewDocument, TextDocument};
use crate::explorer::WorkspaceExplorer;
use crate::host::{self, Host};
use crate::question_card::QuestionCard;
use crate::restart;
use crate::session_list;
use crate::theme::remote::{self, CheckedTheme, RefreshOutcome};
use crate::theme::{ThemeCatalog, ThemeCatalogError, ThemeSource};
use crate::transcript::{self, RowKind, Transcript};
use crate::ui_preferences::UiPreferences;
use dsh_client_core::{
    approvals::{ApprovalDecision, PendingApproval},
    chat::{AcceptedResponse, ChatClient, PromptContentPart, PromptMode, PromptRequest},
    commands::{
        CommandDescriptor, CommandExecution, CommandResult, CommandSubmitAttachment, CommandsClient,
    },
    diagnostics::{DiagnoseResponse, DiagnosticItem, DiagnosticsClient, DiagnosticsResponse},
    follow::{self, FollowFrame, FollowRequest},
    git::{
        GitApplyRequest, GitApplyResult, GitApplyTarget, GitBranches, GitCheckoutResult, GitClient,
        GitCommitEntry, GitCommitOptions, GitCommitResult, GitConflict, GitDiff,
        GitDiscardUntrackedResult, GitFile, GitLog, GitOperationResult, GitOperationState,
        GitRemoteResult, GitResolveRequest, GitResolveResult, GitResolveSide, GitShow, GitStash,
        GitStashAction, GitStashEntry, GitStashRequest, GitStatus,
    },
    goals::{CreateGoalRequest, EditGoalRequest, GoalRef, GoalView, GoalsClient},
    marketplace::{MarketplaceClient, MarketplaceInstallOptions},
    message_feedback::{MessageFeedbackClient, MessageFeedbackItem},
    models::{ModelCatalog, ModelCatalogModel, ModelClient, ModelSelection, SelectModelRequest, SelectModelResponse},
    office_to_pdf::OfficeToPdfClient,
    plugins::{BundleInfo, ChangeResult, ManagedPlugin, PluginInventorySnapshot, PluginsClient},
    remote_events::{RemoteEvent, RemoteEventError, RemoteEventOutcome, RemoteEvents},
    remote_stream::RemoteStreamMux,
    schedules::{ScheduleDeleteRequest, ScheduleListRequest, SchedulesClient},
    session::{
        SessionAddress, SessionAddressKind, SessionPage, SessionPageRequest, SessionSummary,
        SessionTurnWindow, SessionViewer,
    },
    sessions::{
        CreateSessionRequest, ForkSessionRequest, RenameSessionRequest, SessionSearchResponse,
        SessionsClient,
    },
    settings::{SettingsClient, SettingsDescribeValue, SettingsNamespaceView},
    skills::{SkillListResponse, SkillSetDisabledResponse, SkillsClient},
    streams::{self, JobFollowFrame, JobView, WorkspaceFollowFrame, WorkspaceView},
    terminal::TerminalFrame,
    transport::HttpTransport,
    upload::{FileUploadClient, FileUploadValue},
    user_questions::{
        AskUserQuestionAnswer, AskUserQuestionAnswerItem, PendingQuestion, UserQuestionsClient,
    },
    stream::FrameStream,
    terminal::{
        self, TerminalClient, TerminalCloseRequest, TerminalCreateRequest, TerminalInput,
    },
    workspace::{WorkspaceArchiveValue, WorkspaceClient, WorkspacePinValue},
    workspace_files::{
        WorkspaceByteRange, WorkspaceByteReadOptions, WorkspaceFileRange, WorkspaceFileScope,
        WorkspaceFileWriteResult, WorkspaceFilesClient, WorkspacePathSearchValue,
        WorkspaceSearchMatch, WorkspaceSearchValue,
    },
    Error,
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

/// Reads the opt-out switch for the initial Host connection.
fn auto_connect_enabled(setting: Option<&str>) -> bool {
    setting != Some("0")
}

/// Progress line shown while the Host sidecar boots.
fn host_start_status(elapsed: std::time::Duration) -> String {
    format!("Starting Host sidecar… {}s", elapsed.as_secs())
}
use super::git_review::HistoryFooter;
use super::terminal_state::{append_bounded_output, PreviewScratchDirectory, TerminalPanel};
use super::theme::Theme;
use super::ShellServices;
use gpui::{point, px, App, AppContext, Context, FocusHandle, ScrollHandle, Window, WindowAppearance};

enum PluginAction {
    SetPlugin { id: String, enabled: bool },
    SetBundle { name: String, enabled: bool },
    RemoveBundle { name: String },
}

enum ApprovalCommand {
    Answer(PendingApproval, ApprovalDecision),
    Delegate(dsh_client_core::remote_events::RemoteEventInvocation),
    AnswerQuestion(PendingQuestion, AskUserQuestionAnswer),
}

enum GoalMutation {
    Create(CreateGoalRequest),
    Edit(GoalRef, EditGoalRequest),
    Pause(GoalRef),
    Resume(GoalRef),
    Complete(GoalRef),
    Clear(GoalRef),
}

/// One file the model declared as a final deliverable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DeliverableFile {
    pub(crate) path: String,
    pub(crate) description: Option<String>,
}

/// Follow backlog plus history page a Session load resolves to.
type PageLoad = (i64, SessionPage, Vec<dsh_client_core::follow::EventFrame>);

enum SessionOperation {
    List,
    Search(String),
    Create(Option<String>),
    Rename(RenameSessionRequest),
    Fork(ForkSessionRequest),
    Delete(String),
}

enum SessionUpdate {
    List(Vec<SessionSummary>),
    Search(SessionSearchResponse),
    Created(String),
    Renamed(String),
    Forked(String),
    Deleted(String),
}

/// What startup does with the last opened Session.
#[derive(Debug, PartialEq, Eq)]
enum RestoreSession {
    /// Restore is disabled or a Session is already open.
    None,
    /// The remembered Session is present in the list.
    Load(String),
    /// The remembered Session is gone; open `fallback` when one exists.
    Missing {
        last: String,
        fallback: Option<String>,
    },
}

/// Chooses the startup Session from the remembered id and the live list.
fn restore_session_target(
    restore_enabled: bool,
    last_session_id: Option<&str>,
    sessions: &[SessionSummary],
    selected_session: Option<&str>,
) -> RestoreSession {
    if !restore_enabled || selected_session.is_some() {
        return RestoreSession::None;
    }
    let Some(last) = last_session_id else {
        return RestoreSession::None;
    };
    if sessions.iter().any(|session| session.session_id == last) {
        return RestoreSession::Load(last.to_owned());
    }
    let fallback = sessions
        .iter()
        .max_by_key(|session| session.updated_at)
        .map(|session| session.session_id.clone());
    RestoreSession::Missing {
        last: last.to_owned(),
        fallback,
    }
}

/// Stores or removes one Session draft, returning whether it changed.
fn remember_draft(
    drafts: &mut std::collections::BTreeMap<String, String>,
    session_id: &str,
    text: &str,
) -> bool {
    if text.is_empty() {
        drafts.remove(session_id).is_some()
    } else if drafts.get(session_id).is_some_and(|draft| draft == text) {
        false
    } else {
        drafts.insert(session_id.to_owned(), text.to_owned());
        true
    }
}

/// Whether `selected_session` has a Host-reported run in flight.
fn session_is_running(sessions: &[SessionSummary], selected_session: Option<&str>) -> bool {
    let Some(selected_session) = selected_session else {
        return false;
    };
    sessions
        .iter()
        .find(|session| session.session_id == selected_session)
        .is_some_and(|session| session.running)
}

/// Classify a Git failure for the surfaces that display it, keeping the Host's
/// own wording for anything a caller can act on.
/// Host commands whose name or description contain the lowercased query.
fn matching_host_commands(
    commands: &[CommandDescriptor],
    query: &str,
) -> Vec<CommandDescriptor> {
    let query = query.trim().to_lowercase();
    commands
        .iter()
        .filter(|command| {
            query.is_empty()
                || command.name.to_lowercase().contains(&query)
                || command.description.to_lowercase().contains(&query)
        })
        .cloned()
        .collect()
}

/// One Changes-surface keyboard action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ChangesShortcut {
    /// Select the hunk above the focused one.
    HunkPrevious,
    /// Select the hunk below the focused one.
    HunkNext,
    /// Stage the selected file, the ticked set, or the focused hunk.
    Stage,
    /// Unstage the selected file, the ticked set, or the focused hunk.
    Unstage,
    /// Discard the selected file or focused hunk after confirmation.
    Discard,
}

/// Registers one global handler for a Changes shortcut action.
///
/// The handler is global so the chord works while nothing owns the keyboard; the
/// panel, overlay, and typing guards live in the shell method it calls. Action
/// handlers stop propagation by default, so a keystroke the shortcut declines is
/// released to the focused field, and typing a letter still types it.
///
/// @param cx — the app context the shell is built in.
/// @param shell — the shell entity the handler updates.
/// @param shortcut — the action the handler runs.
fn on_changes_shortcut<A: gpui::Action>(
    cx: &mut Context<DesktopApp>,
    shell: gpui::WeakEntity<DesktopApp>,
    shortcut: ChangesShortcut,
) {
    App::on_action(cx, move |_: &A, cx: &mut App| {
        let Some(shell) = shell.upgrade() else {
            return;
        };
        shell.update(cx, |app, cx| {
            if app.run_changes_shortcut(shortcut) {
                cx.notify();
            } else {
                cx.propagate();
            }
        });
    });
}

/// One palette row step, clamped to the available rows.
fn stepped_selection(current: usize, delta: i8, len: usize) -> usize {
    if len == 0 {
        return 0;
    }
    let moved = current as isize + isize::from(delta);
    moved.clamp(0, len as isize - 1) as usize
}

/// What one raw-mode edit added after the shared prefix, and how many characters it deleted.
fn raw_keystroke(previous: &str, next: &str) -> (String, usize) {
    let shared = previous
        .chars()
        .zip(next.chars())
        .take_while(|(left, right)| left == right)
        .map(|(left, _)| left.len_utf8())
        .sum::<usize>();
    (
        next[shared..].to_owned(),
        previous[shared..].chars().count(),
    )
}

fn failure_text(error: &Error) -> String {
    match error {
        Error::Remote { code, .. } if code == "git/not-repository" => "No repository".to_owned(),
        Error::Remote { code, .. } if code == "git/unavailable" => {
            "git is not installed on the Host".to_owned()
        }
        Error::Remote { code, message, .. } if code.starts_with("git/") => message.clone(),
        other => other.to_string(),
    }
}

/// First seven characters of a commit id, or the id itself when shorter.
fn short_object_id(commit: &str) -> &str {
    &commit[..commit.len().min(7)]
}

/// Whether the Host-side discard guard refuses this change kind.
fn git_status_cannot_discard(status: &str) -> bool {
    !matches!(status, "modified" | "deleted" | "typechange")
}

/// Builds the `git/stash` request one panel action sends.
///
/// @param action - Save, apply, pop, or drop.
/// @param index - Entry number for apply, pop, and drop.
/// @param message - Optional save message; blank text is sent as absent because
///   the Host refuses a blank message.
/// @param include_untracked - Save only: also stash untracked files.
/// @returns The request the Host receives.
fn stash_request(
    action: GitStashAction,
    index: Option<u32>,
    message: Option<String>,
    include_untracked: bool,
) -> GitStashRequest {
    let saved = action == GitStashAction::Save;
    let saved_message = if saved {
        message.filter(|text| !text.trim().is_empty())
    } else {
        None
    };
    GitStashRequest {
        action,
        message: saved_message,
        include_untracked: saved && include_untracked,
        index: match action {
            GitStashAction::Apply | GitStashAction::Pop | GitStashAction::Drop => index,
            GitStashAction::List | GitStashAction::Save => None,
        },
    }
}

/// Why the shell refuses to delete this path, or `None` when the Host may.
///
/// `git clean -f` never removes ignored content and a path Git collapsed into
/// `dir/` would take the directory with it, so both are refused before the
/// confirmation opens.
///
/// @param file - Changed path the Files tab row shows.
/// @returns The refusal text, or `None` for an untracked file.
fn untracked_delete_refusal(file: &GitFile) -> Option<String> {
    if file.path.ends_with('/') {
        return Some(format!(
            "\"{}\" is an untracked directory; the shell deletes files only.",
            file.path
        ));
    }
    if !super::git_review::is_untracked(file) {
        return Some(format!(
            "\"{}\" is not an untracked file; reload the Git status before retrying.",
            file.path
        ));
    }
    None
}

/// Status `git/discardUntracked` uses as its stale-review guard.
fn untracked_delete_expected_status() -> String {
    super::git_review::UNTRACKED_STATUS.to_owned()
}

/// Side name the settled `git/resolve` status line shows.
///
/// @param side - Side the Host kept.
/// @returns `ours` or `theirs`.
fn resolve_side_label(side: GitResolveSide) -> &'static str {
    match side {
        GitResolveSide::Ours => "ours",
        GitResolveSide::Theirs => "theirs",
    }
}

/// Whether a discard confirmation still describes the latest Git status.
fn discard_confirmation_is_fresh(
    status: Option<&GitStatus>,
    session_id: &str,
    selected_session_id: Option<&str>,
    path: &str,
    expected_status: &str,
    staged: bool,
    worktree: bool,
) -> bool {
    selected_session_id == Some(session_id)
        && status.is_some_and(|status| {
            status.files.iter().any(|file| {
                file.path == path
                    && file.status == expected_status
                    && file.staged == staged
                    && file.worktree == worktree
                    && !git_status_cannot_discard(&file.status)
            })
        })
}

/// Whether a bulk discard confirmation still describes every ticked path.
fn discard_many_is_fresh(
    status: Option<&GitStatus>,
    session_id: &str,
    selected_session_id: Option<&str>,
    files: &[GitFile],
) -> bool {
    !files.is_empty()
        && files.iter().all(|file| {
            discard_confirmation_is_fresh(
                status,
                session_id,
                selected_session_id,
                &file.path,
                &file.status,
                file.staged,
                file.worktree,
            )
        })
}

/// Whether a hunk confirmation still describes the status and patch the user reviewed.
fn hunk_confirmation_is_fresh(
    status: Option<&GitStatus>,
    diff: Option<&GitDiff>,
    session_id: &str,
    selected_session_id: Option<&str>,
    path: &str,
    expected_status: &str,
    staged: bool,
    worktree: bool,
    signature: &str,
) -> bool {
    discard_confirmation_is_fresh(
        status,
        session_id,
        selected_session_id,
        path,
        expected_status,
        staged,
        worktree,
    ) && diff.is_some_and(|diff| {
        super::git_review::patch_signature(&diff.path, diff.staged, &diff.patch) == signature
    })
}

/// Whether a checkout confirmation still describes the latest repository state.
fn checkout_confirmation_is_fresh(
    status: Option<&GitStatus>,
    branches: Option<&GitBranches>,
    session_id: &str,
    selected_session_id: Option<&str>,
    current: &Option<String>,
    target: &str,
    head: &Option<String>,
    changed_files: &[GitFile],
) -> bool {
    selected_session_id == Some(session_id)
        && status.is_some_and(|status| {
            status.branch == *current
                && status.commit == *head
                && status.files.as_slice() == changed_files
        })
        && branches.is_some_and(|branches| {
            branches
                .branches
                .iter()
                .any(|branch| branch.name == target)
        })
}

/// Application state owns the transport and the last session-list result so UI rendering stays local.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ActiveView {    Sessions,
    Explorer,
    Settings,
    Document,
    Plugins,
    Skills,
    Marketplace,
    Goals,
}

/// Sections of the Preferences overlay, in navigation order.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PreferencesSection {
    General,
    Appearance,
    Keymap,
    Extensions,
    Skills,
    Marketplace,
    About,
}

/// Panel edge the user is dragging.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResizeTarget {
    Explorer,
    Changes,
    /// Boundary between one terminal pane and the next, by left pane index.
    TerminalPane(usize),
}

/// One in-flight drag of a docked panel's edge.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PanelDrag {
    pub(crate) target: ResizeTarget,
    /// Window x where the drag began.
    pub(crate) start_x: f32,
    /// Panel width when the drag began, or the left pane's weight for a splitter.
    pub(crate) start_width: f32,
    /// Right pane's weight for a terminal splitter.
    pub(crate) start_secondary: f32,
}

/// Which Changes list the right column shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChangesTab {
    Diff,
    Files,
    History,
}

/// Status-bar Git menu that is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GitMenu {
    /// Local branches, checkout, and branch operations.
    Branch,
    /// Fetch, pull, push, and publish.
    Sync,
}

/// One remote mutation the sync menu starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GitRemoteAction {
    Fetch,
    Pull,
    Push,
    /// Push the current branch and record `origin` as its upstream.
    PushSetUpstream,
}

impl GitRemoteAction {
    /// Label the sync menu and the in-flight reason show.
    ///
    /// @returns The action name as the user reads it.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Fetch => "Fetch",
            Self::Pull => "Pull",
            Self::Push => "Push",
            Self::PushSetUpstream => "Push and set upstream",
        }
    }

    /// Verb name the Host returns for this action.
    ///
    /// @returns `fetch`, `pull`, or `push`.
    pub(crate) fn verb(self) -> &'static str {
        match self {
            Self::Fetch => "fetch",
            Self::Pull => "pull",
            Self::Push | Self::PushSetUpstream => "push",
        }
    }
}

/// Branch or branch-tip action the branch menu confirms first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GitBranchAction {
    Merge,
    Rebase,
    CherryPick,
    Revert,
}

impl GitBranchAction {
    /// Label the confirmation dialog's action button.
    ///
    /// @returns The action name.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Merge => "Merge",
            Self::Rebase => "Rebase",
            Self::CherryPick => "Cherry-pick",
            Self::Revert => "Revert",
        }
    }
}

/// History entry action a confirmation dialog asks about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GitCommitAction {
    Revert,
    CherryPick,
}

/// One stash entry mutation the panel confirms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StashEntryAction {
    Apply,
    Pop,
    Drop,
}

impl StashEntryAction {
    /// Label the confirmation and the settled status line show.
    ///
    /// @returns The action name as the user reads it.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Apply => "Apply",
            Self::Pop => "Pop",
            Self::Drop => "Drop",
        }
    }

    /// `git/stash` action field this panel action sends.
    ///
    /// @returns The wire action.
    fn request(self) -> GitStashAction {
        match self {
            Self::Apply => GitStashAction::Apply,
            Self::Pop => GitStashAction::Pop,
            Self::Drop => GitStashAction::Drop,
        }
    }
}

/// Stash list and controls opened from the branch or sync menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StashPanel {
    /// Entries newest first, as the last `git/stash list` reported them.
    pub(crate) entries: Vec<GitStashEntry>,
    /// Whether the Host byte cap dropped entries from the list.
    pub(crate) truncated: bool,
    /// Whether a `git/stash` request is in flight for this panel.
    pub(crate) loading: bool,
    /// Host or client failure text, verbatim.
    pub(crate) error: Option<String>,
    /// Git's own output for the last mutation, verbatim.
    pub(crate) summary: Option<String>,
    /// Whether the next save also stashes untracked files.
    pub(crate) include_untracked: bool,
    /// Status-bar trigger the panel returns focus to when it closes.
    pub(crate) trigger: ChromeTrigger,
}

/// One unmerged path's conflict stages, as the operation banner loads them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GitConflictView {
    /// Unmerged path the stages belong to.
    pub(crate) path: String,
    /// Stages the Host returned; `None` while the read runs or after it failed.
    pub(crate) conflict: Option<GitConflict>,
    /// Whether a `git/conflict` read is in flight.
    pub(crate) loading: bool,
    /// Failure of the last stage read, verbatim.
    pub(crate) error: Option<String>,
}

/// Destructive Git action whose confirmation dialog is open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum GitConfirmation {
    Discard {
        session_id: String,
        path: String,
        status: String,
        staged: bool,
        worktree: bool,
    },
    /// Discard several ticked work-tree changes.
    DiscardMany {
        session_id: String,
        files: Vec<GitFile>,
    },
    /// Discard one hunk through `git/apply`.
    DiscardHunk {
        session_id: String,
        path: String,
        status: String,
        staged: bool,
        worktree: bool,
        /// File header plus the one hunk, as `git/apply` expects it.
        patch: String,
        /// Content signature of the diff the user reviewed.
        signature: String,
    },
    Checkout {
        session_id: String,
        current: Option<String>,
        target: String,
        head: Option<String>,
        changed_files: Vec<GitFile>,
    },
    /// Revert or cherry-pick one commit listed in History.
    CommitEntry {
        session_id: String,
        commit: String,
        subject: String,
        action: GitCommitAction,
    },
    /// Push the current branch, optionally publishing it upstream.
    Push {
        session_id: String,
        set_upstream: bool,
    },
    /// Merge, rebase, cherry-pick, or revert through one local branch.
    BranchEntry {
        session_id: String,
        branch: String,
        commit: String,
        action: GitBranchAction,
    },
    /// Keep one conflict side through `git/resolve`.
    ResolveConflict {
        session_id: String,
        path: String,
        side: GitResolveSide,
    },
    /// Apply, pop, or drop one stash entry.
    Stash {
        session_id: String,
        action: StashEntryAction,
        index: u32,
        message: String,
    },
    /// Delete one untracked file through `git/discardUntracked`.
    DiscardUntracked {
        session_id: String,
        path: String,
    },
}

/// One settled Git worker result.
/// Index or work-tree verb one path mutation runs.
#[derive(Clone, Copy)]
enum GitPathVerb {
    Stage,
    Unstage,
    Discard,
}

impl GitPathVerb {
    /// The operation name the Host reports for this verb.
    fn operation(self) -> &'static str {
        match self {
            Self::Stage => "stage",
            Self::Unstage => "unstage",
            Self::Discard => "discard",
        }
    }
}

enum GitUpdate {
    Status(Result<GitStatus, Error>),
    Branches(Result<GitBranches, Error>),
    Checkout(Result<GitCheckoutResult, Error>),
    Diff(Result<GitDiff, Error>),
    /// One verb over several paths; `paths` lists the calls that landed before
    /// the first failure.
    Paths {
        operation: &'static str,
        paths: Vec<String>,
        failure: Option<Error>,
    },
    Apply(Result<GitApplyResult, Error>),
    Commit(Result<GitCommitResult, Error>),
    Log(Result<GitLog, Error>),
    /// One `git/log` page; `skip` is the commit count the page asked to skip.
    History {
        skip: u32,
        result: Result<GitLog, Error>,
    },
    /// One `git/show` patch; `oid` is the commit it was requested for.
    Show {
        oid: String,
        result: Result<GitShow, Error>,
    },
    /// One revert or cherry-pick; `commit` is the oid the user confirmed.
    CommitEntry {
        action: GitCommitAction,
        commit: String,
        result: Result<GitOperationResult, Error>,
    },
    State(Result<GitOperationState, Error>),
    StatusAndState(
        Result<GitStatus, Error>,
        Result<GitOperationState, Error>,
    ),
    Remote(&'static str, Result<GitRemoteResult, Error>),
    Operation(Result<GitOperationResult, Error>),
    /// One `git/conflict` read; `path` is the view the stages belong to.
    Conflict {
        path: String,
        result: Result<GitConflict, Error>,
    },
    /// One `git/resolve` call.
    Resolve(Result<GitResolveResult, Error>),
    /// One `git/stash` action.
    Stash(Result<GitStash, Error>),
    /// One `git/discardUntracked` call.
    DiscardUntracked(Result<GitDiscardUntrackedResult, Error>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BottomTool {
    Terminal,
    Jobs,
    Schedules,
    Deliverables,
    Diagnostics,
}

/// Goal lifecycle change requested by the Goals surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GoalAction {
    Pause,
    Resume,
    Complete,
    Clear,
}


/// What the running file picker is collecting.
#[derive(Clone, Copy)]
enum DialogPurpose {
    Attach,
    Open,
    Folder,
    ThemeAdd,
    ThemeExport,
}

/// Snapshot, bundles, and managed plugins returned by the inventory workers.
type PluginInventoryResult =
    Result<(PluginInventorySnapshot, Vec<BundleInfo>, Vec<ManagedPlugin>), Error>;

/// Terminal panes one window may show at once, including the primary pane.
pub(crate) const MAX_TERMINAL_PANES: usize = 4;

/// Most Agent buffers kept open at once. Older non-active buffers are evicted.
pub(crate) const MAX_AGENT_TABS: usize = 6;

/// Replace one document's diagnostics with a fresh on-demand result. An empty
/// `uri` means the Host could not resolve a file, so nothing is replaced.
/// @param items - merged snapshot to update.
/// @param uri - canonical file URI the result describes.
/// @param incoming - diagnostics just published for that file.
pub(crate) fn merge_diagnostics(
    items: &mut Vec<DiagnosticItem>,
    uri: &str,
    incoming: Vec<DiagnosticItem>,
) {
    if !uri.is_empty() {
        items.retain(|item| item.uri != uri);
    }
    items.extend(incoming);
}
/// Input field keys, one per pane index.
pub(crate) const TERMINAL_INPUT_KEYS: [&str; MAX_TERMINAL_PANES] = [
    "terminal-input",
    "terminal-input-1",
    "terminal-input-2",
    "terminal-input-3",
];

/// Title field keys, one per pane index.
pub(crate) const TERMINAL_TITLE_KEYS: [&str; MAX_TERMINAL_PANES] = [
    "terminal-title",
    "terminal-title-1",
    "terminal-title-2",
    "terminal-title-3",
];

/// Pane index one terminal input field key addresses.
fn terminal_input_index(key: &str) -> Option<usize> {
    TERMINAL_INPUT_KEYS
        .iter()
        .position(|candidate| *candidate == key)
}

/// One palette row: a shell command, a Host-registered human command,
/// or a loaded workspace file for quick-open.
#[derive(Clone)]
pub(crate) enum PaletteEntry {
    Shell(CommandId),
    Host(CommandDescriptor),
    File(String),
}

/// Chrome control that owns the overlay Escape closes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChromeTrigger {
    Menu,
    Search,
    Workspace,
    StatusTools,
    GitBranch,
    GitSync,
    /// Conflict paths in the in-progress operation banner.
    GitConflict,
}

/// Keyboard focus handles of the chrome controls that open an overlay.
///
/// Escape returns focus to the control that opened the overlay, so each one
/// keeps a handle that survives across frames.
pub(crate) struct ChromeFocus {
    pub(crate) menu: FocusHandle,
    pub(crate) search: FocusHandle,
    pub(crate) workspace: FocusHandle,
    pub(crate) status_tools: FocusHandle,
    pub(crate) git_branch: FocusHandle,
    pub(crate) git_sync: FocusHandle,
    pub(crate) git_conflict: FocusHandle,
}

impl ChromeFocus {
    /// The handle that takes focus back when its overlay closes.
    ///
    /// @param trigger - Control whose overlay just closed.
    /// @returns The control's focus handle.
    pub(crate) fn for_trigger(&self, trigger: ChromeTrigger) -> &FocusHandle {
        match trigger {
            ChromeTrigger::Menu => &self.menu,
            ChromeTrigger::Search => &self.search,
            ChromeTrigger::Workspace => &self.workspace,
            ChromeTrigger::StatusTools => &self.status_tools,
            ChromeTrigger::GitBranch => &self.git_branch,
            ChromeTrigger::GitSync => &self.git_sync,
            ChromeTrigger::GitConflict => &self.git_conflict,
        }
    }
}

/// One workspace the top bar's picker can switch to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SessionWorkspace {
    /// Working directory the entry stands for.
    pub(crate) cwd: String,
    /// Newest Session of that directory, which selecting the entry opens.
    pub(crate) session_id: String,
    /// Sessions the Session list reports for that directory.
    pub(crate) session_count: usize,
    /// Whether the selected Session belongs to this workspace.
    pub(crate) active: bool,
}

pub(crate) struct DesktopApp {
    pub(crate) host: Option<Host>,
    pub(crate) host_receiver: Option<mpsc::Receiver<Host>>,
    /// When the shell was created; startup milestones report their elapsed time
    /// from here so a slow Host launch can be attributed without a profiler.
    pub(crate) started_at: std::time::Instant,
    pub(crate) connect_receiver: Option<mpsc::Receiver<Result<(String, HttpTransport), Error>>>,
    pub(crate) sessions_receiver: Option<mpsc::Receiver<Result<SessionUpdate, Error>>>,
    /// Archived Session ids as last reported by an archive mutation.
    pub(crate) archived_session_ids: Vec<String>,
    /// Pinned Session ids as last reported by a pin mutation.
    pub(crate) pinned_session_ids: Vec<String>,
    pub(crate) archive_receiver: Option<mpsc::Receiver<Result<WorkspaceArchiveValue, Error>>>,
    pub(crate) pin_receiver: Option<mpsc::Receiver<Result<WorkspacePinValue, Error>>>,
    pub(crate) workspace_status: Option<String>,
    /// Workspaces and grouping sets from `workspace/follow`.
    pub(crate) workspaces: Vec<WorkspaceView>,
    pub(crate) selected_workspace: Option<String>,
    pub(crate) workspace_receiver: Option<mpsc::Receiver<Result<WorkspaceFollowFrame, String>>>,
    pub(crate) workspace_error: Option<String>,
    pub(crate) workspace_create_receiver: Option<mpsc::Receiver<Result<dsh_client_core::workspace::WorkspaceCreateValue, Error>>>,
    pub(crate) file_search_query: String,
    pub(crate) file_search_items: Vec<WorkspaceSearchMatch>,
    pub(crate) file_search_scanned: u64,
    pub(crate) file_search_truncated: bool,
    pub(crate) file_search_loading: bool,
    pub(crate) file_search_error: Option<String>,
    pub(crate) file_search_receiver: Option<mpsc::Receiver<Result<WorkspaceSearchValue, Error>>>,
    /// Workspace paths matching the palette query, and the query they answer.
    pub(crate) palette_paths: Vec<String>,
    pub(crate) palette_paths_for: String,
    pub(crate) palette_paths_receiver:
        Option<mpsc::Receiver<Result<WorkspacePathSearchValue, Error>>>,
    /// Workspace-relative path the handoff summary is written to.
    pub(crate) handoff_path: String,
    /// Last rendered handoff summary, kept for preview and writing.
    pub(crate) handoff_preview: Option<String>,
    pub(crate) handoff_status: Option<String>,
    pub(crate) handoff_receiver: Option<mpsc::Receiver<Result<String, Error>>>,
    pub(crate) session_query: String,
    pub(crate) session_search: Option<SessionSearchResponse>,
    pub(crate) session_title: String,
    /// Checkpoint sequence entered for the next fork; blank means latest.
    pub(crate) fork_seq_text: String,
    pub(crate) sessions_status: String,
    pub(crate) sessions_error: Option<String>,
    pub(crate) sessions_failure_context: &'static str,
    pub(crate) host_url: Option<String>,
    pub(crate) transport: Option<HttpTransport>,
    pub(crate) streams: Option<RemoteStreamMux>,
    pub(crate) sessions: Vec<SessionSummary>,
    pub(crate) selected_session: Option<String>,
    /// Session ids open as Agent buffers, in display order; the active buffer
    /// is {@link selected_session}.
    pub(crate) agent_tabs: Vec<String>,
    /// Agent buffer under the pointer; only that buffer shows a close affordance.
    pub(crate) hovered_buffer: Option<String>,
    pub(crate) page_has_more: bool,
    pub(crate) page_loading: bool,
    pub(crate) page_before_seq: Option<i64>,
    pub(crate) page_error: Option<String>,
    pub(crate) page_receiver: Option<mpsc::Receiver<Result<PageLoad, Error>>>,
    pub(crate) page_through_seq: Option<i64>,
    pub(crate) chat_text: String,
    /// Delivery mode selected for the next prompt.
    pub(crate) prompt_mode: PromptMode,
    /// Recently sent prompts for recall.
    pub(crate) prompt_history: Vec<String>,
    pub(crate) prompt_history_index: Option<usize>,
    /// Whether the prompt history popover is open.
    pub(crate) prompt_history_open: bool,
    /// Whether the @ file picker is open.
    pub(crate) file_picker_open: bool,
    pub(crate) file_picker_query: String,
    /// Composer text to install into the text field on the next render.
    pub(crate) pending_composer_text: Option<String>,
    /// Whether the last Session id or a draft changed on disk.
    pub(crate) session_state_dirty: bool,
    /// When the Session state last changed, used to debounce its save.
    pub(crate) session_state_changed_at: Option<std::time::Instant>,
    /// Last time a run-state poll refreshed the Session list.
    pub(crate) last_session_poll: std::time::Instant,
    pub(crate) focus_composer: bool,
    /// Focus handles of the controls that open an overlay.
    pub(crate) chrome_focus: ChromeFocus,
    /// Control that takes keyboard focus back on the next frame.
    pub(crate) pending_chrome_focus: Option<ChromeTrigger>,
    pub(crate) cancel_receiver: Option<mpsc::Receiver<Result<AcceptedResponse, Error>>>,
    pub(crate) dialog: Option<(DialogPurpose, mpsc::Receiver<Option<PathBuf>>)>,
    pub(crate) attachment: Option<FileUploadValue>,
    pub(crate) attachment_name: String,
    pub(crate) attachment_receiver: Option<mpsc::Receiver<Result<FileUploadValue, String>>>,
    pub(crate) attachment_error: Option<String>,
    pub(crate) chat_loading: bool,
    pub(crate) chat_receiver: Option<mpsc::Receiver<Result<FollowFrame, Error>>>,
    pub(crate) transcript: Transcript,
    /// Retain few restored transcripts so returning to a recent Session
    /// resumes instead of re-downloading its snapshot.
    pub(crate) session_transcripts: std::collections::VecDeque<(String, Transcript)>,
    pub(crate) assistant_streaming: bool,
    pub(crate) assistant_draft: String,
    pub(crate) approval_events_receiver: Option<mpsc::Receiver<Result<RemoteEvent, RemoteEventError>>>,
    pub(crate) approval_commands: Option<mpsc::Sender<ApprovalCommand>>,
    pub(crate) pending_approvals: Vec<PendingApproval>,
    pub(crate) pending_questions: Vec<QuestionCard>,
    /// Height the question/approval stack used last frame, kept out of the transcript.
    pub(crate) attention_reserved_height: f32,
    pub(crate) deliverables: Vec<DeliverableFile>,
    /// Diagnostics published by configured language servers for the selected Session.
    pub(crate) diagnostics: Vec<DiagnosticItem>,
    /// Workspace root the current diagnostics snapshot describes.
    pub(crate) diagnostics_workspace: Option<String>,
    pub(crate) diagnostics_loading: bool,
    pub(crate) diagnostics_error: Option<String>,
    pub(crate) diagnostics_receiver:
        Option<mpsc::Receiver<Result<DiagnosticsResponse, Error>>>,
    /// On-demand diagnose worker for the document currently being opened.
    pub(crate) diagnose_receiver: Option<mpsc::Receiver<Result<DiagnoseResponse, Error>>>,
    pub(crate) market_indexes: Vec<serde_json::Value>,
    pub(crate) market_results: Vec<serde_json::Value>,
    pub(crate) market_url: Option<String>,
    pub(crate) market_query: String,
    pub(crate) market_receiver: Option<mpsc::Receiver<Result<serde_json::Value, Error>>>,
    pub(crate) market_status: Option<String>,
    pub(crate) market_loading: bool,
    pub(crate) feedback_items: Vec<MessageFeedbackItem>,
    pub(crate) feedback_receiver: Option<mpsc::Receiver<Result<Vec<MessageFeedbackItem>, Error>>>,
    pub(crate) feedback_put_receiver: Option<mpsc::Receiver<Result<MessageFeedbackItem, Error>>>,
    pub(crate) feedback_status: Option<String>,
    pub(crate) ui_preferences_path: PathBuf,
    /// Directory that holds installed user theme packs.
    pub(crate) ui_themes_path: PathBuf,
    pub(crate) ui_preferences: UiPreferences,
    /// Built-in and user theme packs discovered for this client.
    pub(crate) theme_catalog: ThemeCatalog,
    /// Result or warning from the last theme file action.
    pub(crate) theme_message: Option<String>,
    /// User pack open in the JSON editor.
    pub(crate) theme_editor: Option<ThemeEditor>,
    /// Imported file waiting for the user to confirm replacing its id.
    pub(crate) theme_pending_import: Option<PathBuf>,
    /// Pack the open save dialog will export.
    pub(crate) theme_export_id: Option<String>,
    /// User pack whose Remove button is waiting for confirmation.
    pub(crate) theme_remove_confirm: Option<String>,
    /// GitHub import dialog state, when open.
    pub(crate) github_dialog: Option<GithubThemeDialog>,
    /// Running explicit GitHub check, import, or update.
    pub(crate) github_receiver: Option<mpsc::Receiver<GithubTaskResult>>,
    pub(crate) settings_description: Option<SettingsDescribeValue>,
    pub(crate) settings_loading: bool,
    pub(crate) settings_error: Option<String>,
    pub(crate) settings_receiver: Option<mpsc::Receiver<Result<SettingsDescribeValue, Error>>>,
    pub(crate) settings_update_receiver: Option<mpsc::Receiver<Result<SettingsNamespaceView, Error>>>,
    pub(crate) show_settings: bool,
    /// Section selected in the Preferences overlay's navigation column.
    pub(crate) preferences_section: PreferencesSection,
    /// Handle the Preferences window focuses while it is open.
    pub(crate) preferences_focus: FocusHandle,
    /// Input key focused when the overlay opened, restored when it closes.
    pub(crate) preferences_focus_return: Option<String>,
    /// Whether the next frame moves focus into the Preferences window.
    pub(crate) focus_preferences: bool,
    /// Whether the next frame returns focus to the owner captured at open.
    pub(crate) restore_preferences_focus: bool,
    pub(crate) preview_document: Option<TextDocument>,
    pub(crate) preview_image: Option<ImageDocument>,
    pub(crate) preview_path: String,
    pub(crate) preview_loading: bool,
    pub(crate) preview_error: Option<String>,
    pub(crate) preview_receiver: Option<mpsc::Receiver<Result<PreviewDocument, String>>>,
    /// Editable copy of the loaded text document; saved with its version guard.
    pub(crate) preview_draft: String,
    /// Path and 1-based line the document editor should reveal on the next render.
    pub(crate) preview_target_line: Option<(String, u64)>,
    /// Draft text to install into the document editor on the next render.
    pub(crate) pending_preview_text: Option<String>,
    pub(crate) preview_save_receiver: Option<mpsc::Receiver<Result<WorkspaceFileWriteResult, Error>>>,
    pub(crate) preview_save_status: Option<String>,
    /// Draft destination for a rename of the loaded document.
    pub(crate) document_new_path: String,
    /// Whether the Delete button already asked for a second click.
    pub(crate) document_delete_confirm: bool,
    /// Session id whose permanent deletion is waiting for a second click.
    pub(crate) session_delete_confirm: Option<String>,
    pub(crate) document_op_status: Option<String>,
    pub(crate) document_op_receiver:
        Option<mpsc::Receiver<Result<DocumentMutation, Error>>>,
    pub(crate) show_document: bool,
    pub(crate) show_plugins: bool,
    pub(crate) show_skills: bool,
    pub(crate) show_market: bool,
    pub(crate) show_goals: bool,
    pub(crate) top_menu_open: bool,
    /// Whether the top bar's workspace picker is open.
    pub(crate) workspace_picker_open: bool,
    pub(crate) goal: Option<GoalView>,
    pub(crate) goal_loading: bool,
    pub(crate) goal_mutation_loading: bool,
    pub(crate) goal_receiver: Option<mpsc::Receiver<Result<Option<GoalView>, Error>>>,
    pub(crate) goal_mutation_receiver: Option<mpsc::Receiver<Result<Option<GoalView>, Error>>>,
    pub(crate) goal_objective: String,
    pub(crate) goal_max_rounds: String,
    pub(crate) clear_goal_confirmation: Option<GoalRef>,
    pub(crate) goal_error: Option<String>,
    pub(crate) goal_status: Option<String>,
    pub(crate) plan_command_receiver: Option<mpsc::Receiver<Result<Option<CommandExecution>, Error>>>,
    pub(crate) plan_command_loading: bool,
    pub(crate) plan_command_status: Option<String>,
    pub(crate) active_view: ActiveView,
    pub(crate) skills: Option<SkillListResponse>,
    pub(crate) skills_query: String,
    pub(crate) selected_skill: Option<String>,
    pub(crate) skills_receiver: Option<mpsc::Receiver<Result<SkillListResponse, Error>>>,
    pub(crate) skills_write_receiver: Option<mpsc::Receiver<Result<SkillSetDisabledResponse, Error>>>,
    pub(crate) skills_status: Option<String>,
    pub(crate) skills_loading: bool,
    pub(crate) skills_error: Option<String>,
    pub(crate) model_catalog: Option<ModelCatalog>,
    pub(crate) model_selection: Option<ModelSelection>,
    pub(crate) model_error: Option<String>,
    pub(crate) model_receiver: Option<mpsc::Receiver<Result<ModelCatalog, Error>>>,
    pub(crate) model_requested_at: Option<std::time::Instant>,
    pub(crate) model_select_receiver: Option<mpsc::Receiver<Result<SelectModelResponse, Error>>>,
    pub(crate) model_init_receiver: Option<mpsc::Receiver<Result<(), Error>>>,
    pub(crate) plugins_snapshot: Option<PluginInventorySnapshot>,
    pub(crate) plugins_loading: bool,
    pub(crate) plugins_error: Option<String>,
    pub(crate) plugins_receiver: Option<mpsc::Receiver<PluginInventoryResult>>,
    pub(crate) plugin_bundles: Vec<BundleInfo>,
    pub(crate) managed_plugins: Vec<ManagedPlugin>,
    pub(crate) plugin_write_receiver: Option<mpsc::Receiver<Result<ChangeResult, Error>>>,
    pub(crate) plugin_write_status: Option<String>,
    pub(crate) terminal: TerminalPanel,
    /// Additional terminal panes beside the primary one.
    pub(crate) terminal_extra: Vec<Box<TerminalPanel>>,
    /// Relative width of each pane; missing entries mean an even share.
    pub(crate) terminal_pane_widths: Vec<f32>,
    /// Focused terminal pane; `0` is {@link terminal}.
    pub(crate) terminal_focus: usize,
    pub(crate) bottom_tool: Option<BottomTool>,
    pub(crate) explorer: WorkspaceExplorer,
    pub(crate) show_explorer: bool,
    pub(crate) explorer_fullscreen: bool,
    /// Whether the right Changes column is docked.
    pub(crate) show_changes: bool,
    pub(crate) changes_tab: ChangesTab,
    /// Latest Host-reported Git status for the selected Session.
    pub(crate) git_status: Option<GitStatus>,
    /// Latest Host-reported branch list for the selected Session.
    pub(crate) git_branches: Option<GitBranches>,
    /// Path whose diff the Changes panel shows.
    pub(crate) git_selected_path: Option<String>,
    /// Whether the selected path's diff reads the staged side.
    pub(crate) git_diff_staged: bool,
    /// Hunk the Diff tab's keyboard actions target, as a hunk index.
    pub(crate) git_focused_hunk: Option<usize>,
    /// Latest diff for {@link git_selected_path}.
    pub(crate) git_diff: Option<GitDiff>,
    /// In-progress rebase, cherry-pick, merge, or revert and its conflicts.
    pub(crate) git_state: Option<GitOperationState>,
    /// Conflict stages the operation banner opened for one unmerged path.
    pub(crate) git_conflict_view: Option<GitConflictView>,
    /// Focus the conflict chips take back when the conflict view closes.
    pub(crate) git_conflict_focus: FocusHandle,
    /// Path whose stages must be read once the worker is free.
    git_conflict_pending: Option<String>,
    /// Stash list and controls opened from a status-bar menu.
    pub(crate) git_stash: Option<StashPanel>,
    /// Set when the stash list must be read once the worker is free.
    git_stash_pending: bool,
    /// Set when the stash message must be cleared on the next frame.
    git_stash_message_clear: bool,
    /// Git's own text for the last remote or multi-step action.
    pub(crate) git_remote_summary: Option<String>,
    /// Status-bar menu the user has open, if any.
    pub(crate) git_menu: Option<GitMenu>,
    /// Remote mutation in flight; the sync menu names it while it runs.
    pub(crate) git_remote_pending: Option<GitRemoteAction>,
    /// Set when a state refresh waits for the current Git worker.
    pub(crate) git_state_pending: bool,
    /// Set when the selected path's diff must be read once the worker is free.
    git_diff_pending: bool,
    /// Failure of the last Git verb, already classified for display.
    pub(crate) git_error: Option<String>,
    /// Whether a Git verb is in flight.
    pub(crate) git_loading: bool,
    /// At most one Git verb runs at a time, so responses stay ordered.
    pub(crate) git_receiver: Option<mpsc::Receiver<GitUpdate>>,
    /// Destructive Git action waiting for the user's explicit confirmation.
    pub(crate) git_confirmation: Option<GitConfirmation>,
    /// Whether the workspace watcher changed since the last Git refresh check.
    pub(crate) git_refresh_pending: bool,
    /// Last watcher timestamp the Git auto-refresh check consumed.
    pub(crate) git_refresh_probe_started_at: Option<std::time::Instant>,
    /// Paths ticked in the Files tab bulk selection.
    pub(crate) git_selection: Vec<String>,
    /// Files-tab filter text, mirrored from its text field.
    pub(crate) git_filter: String,
    /// Reviewed signature per changed path.
    pub(crate) git_reviewed: HashMap<String, String>,
    /// Commit draft, mirrored from `git-commit-message`.
    pub(crate) git_commit_message: String,
    /// Whether the next commit replaces HEAD.
    pub(crate) git_amend: bool,
    /// Whether the next commit appends a `Signed-off-by` trailer.
    pub(crate) git_sign_off: bool,
    /// Whether the recent-message list is expanded.
    pub(crate) git_recent_open: bool,
    /// Recent commit subjects from `git/log`, newest first.
    pub(crate) git_recent_messages: Vec<String>,
    /// Failure of the last recent-message read, kept apart from `git_error`.
    pub(crate) git_log_error: Option<String>,
    /// Whether a recent-message read is in flight.
    pub(crate) git_log_loading: bool,
    /// Set when the recent-message list must be read on the next settle.
    git_log_pending: bool,
    /// Commits of the History tab, newest first.
    pub(crate) git_history: Vec<GitCommitEntry>,
    /// Whether a `git/log` page has settled, including for an empty repository.
    pub(crate) git_history_loaded: bool,
    /// Paging footer for the loaded History rows.
    pub(crate) git_history_footer: HistoryFooter,
    /// Selected commit oid; the detail pane and the arrow keys read it.
    pub(crate) git_history_selected: Option<String>,
    /// Patch of the selected commit from `git/show`.
    pub(crate) git_history_show: Option<GitShow>,
    /// Failure of the last history page read.
    pub(crate) git_history_error: Option<String>,
    /// Failure of the last commit patch read.
    pub(crate) git_history_show_error: Option<String>,
    /// Whether a history page read is in flight.
    pub(crate) git_history_loading: bool,
    /// Whether the selected commit's patch is in flight.
    pub(crate) git_history_show_loading: bool,
    /// Page the next free worker must read, as the commits to skip.
    git_history_pending: Option<u32>,
    /// Commit whose patch must be read once the worker is free.
    git_history_show_pending: Option<String>,
    /// Newest commit the loaded page starts from; a moved HEAD reloads it.
    git_history_head: Option<String>,
    /// Focus the arrow keys need while History is the active tab.
    pub(crate) git_history_focus: FocusHandle,
    /// Scroll position the arrow keys keep the selection visible in.
    pub(crate) git_history_scroll: ScrollHandle,
    /// Set when a successful commit continues with a push.
    git_push_pending: bool,
    /// Set when the commit input must be cleared on the next frame.
    pub(crate) git_commit_input_clear: bool,
    /// Whether the Session switcher popover is open.
    pub(crate) show_session_menu: bool,
    /// Whether the status bar's tool popover is open.
    pub(crate) status_menu_open: bool,
    /// The panel edge being dragged, while the pointer is down.
    pub(crate) panel_drag: Option<PanelDrag>,
    /// Last divider press, used to recognize the double-click reset gesture.
    last_divider_click: Option<(ResizeTarget, Instant)>,
    /// Whether the About dialog is open.
    pub(crate) show_about: bool,
    /// Whether the command palette is open.
    pub(crate) palette_open: bool,
    /// Current palette query, mirrored from its text field.
    pub(crate) palette_query: String,
    /// Set when the palette's search field must take focus on the next frame.
    pub(crate) focus_palette: bool,
    /// Selected row in the palette's current match list.
    pub(crate) palette_selected: usize,
    /// Host-registered human commands the palette offers for the selected Session.
    pub(crate) host_commands: Vec<CommandDescriptor>,
    pub(crate) host_commands_receiver: Option<mpsc::Receiver<Result<Vec<CommandDescriptor>, Error>>>,
    pub(crate) host_commands_loading: bool,
    pub(crate) host_commands_error: Option<String>,
    pub(crate) host_command_receiver: Option<mpsc::Receiver<Result<Option<CommandExecution>, Error>>>,
    pub(crate) host_command_loading: bool,
    /// Composer line returned to the user when the Host does not resolve it.
    pub(crate) pending_slash_line: Option<String>,
    pub(crate) schedules: Vec<serde_json::Value>,
    pub(crate) schedules_session: Option<String>,
    pub(crate) schedules_receiver: Option<mpsc::Receiver<Result<Vec<serde_json::Value>, String>>>,
    pub(crate) schedules_error: Option<String>,
    pub(crate) schedule_delete_receiver: Option<mpsc::Receiver<Result<(), String>>>,
    pub(crate) jobs: Vec<JobView>,
    pub(crate) jobs_session: Option<String>,
    pub(crate) jobs_receiver: Option<mpsc::Receiver<Result<Vec<JobView>, String>>>,
    pub(crate) jobs_error: Option<String>,
    pub(crate) job_selected: Option<String>,
    pub(crate) job_output: String,
    pub(crate) job_status: String,
    pub(crate) job_receiver: Option<mpsc::Receiver<Result<JobFollowFrame, String>>>,
    pub(crate) status: String,
    pub(crate) update_prompt: Option<host::UpdatePrompt>,
    pub(crate) update_error: Option<String>,
    pub(crate) update_open_receiver: Option<mpsc::Receiver<Result<(), Error>>>,
    pub(crate) update_status: Option<String>,
    pub(crate) retry_after_host_start: bool,
    pub(crate) quitting: Arc<AtomicBool>,
    pub(crate) show_requested: Arc<AtomicBool>,
    pub(crate) hide_requested: Arc<AtomicBool>,
    pub(crate) hidden: Arc<AtomicBool>,
    pub(crate) tray_available: bool,
    pub(crate) auto_connect: bool,
    /// Keeps the 40 ms poll loop alive while the window exists.
    pub(crate) poll_task: Option<gpui::Task<()>>,
    /// Set when worker state changed outside a paint.
    pub(crate) repaint_soon: bool,
    /// Set when the window must close.
    pub(crate) close_requested: bool,
    /// Local edits for the Settings surface.
    pub(crate) preferences_draft: Option<UiPreferences>,
    pub(crate) preferences_message: Option<String>,
    pub(crate) host_settings_view: HostSettingsView,
    pub(crate) theme: Theme,
    /// Text fields owned by the shell, keyed by a stable name.
    pub(crate) inputs: std::collections::HashMap<&'static str, gpui::Entity<super::text_input::TextInput>>,
    /// Text field that owned the keyboard when the frame was drawn.
    ///
    /// Changes shortcuts read it to stand down while the user is typing; the
    /// focus change itself repaints, so the value is current when a key lands.
    pub(crate) typing_focus: Option<&'static str>,
    /// Scroll position of the transcript pane.
    pub(crate) transcript_scroll: gpui::ScrollHandle,
    /// Set when new transcript rows should scroll into view.
    pub(crate) scroll_transcript_to_bottom: bool,
    /// Whether the composer's model picker is open.
    pub(crate) model_menu_open: bool,
    /// Shortcut commands waiting for the poll loop.
    pub(crate) commands: Arc<Mutex<Vec<CommandId>>>,
}

/// Parses the fork checkpoint field: blank means the latest sequence.
fn parse_fork_checkpoint(text: &str) -> Result<Option<i64>, String> {
    let checkpoint = text.trim();
    if checkpoint.is_empty() {
        return Ok(None);
    }
    checkpoint
        .parse::<i64>()
        .ok()
        .filter(|seq| *seq >= 0)
        .map(Some)
        .ok_or_else(|| {
            format!(
                "Fork checkpoint must be a non-negative integer sequence, got {checkpoint:?}. Blank means latest."
            )
        })
}

/// Applies one `workspace/follow` frame to the grouping state.
fn apply_workspace_frame(
    workspaces: &mut Vec<WorkspaceView>,
    selected_workspace: &mut Option<String>,
    archived_session_ids: &mut Vec<String>,
    pinned_session_ids: &mut Vec<String>,
    frame: WorkspaceFollowFrame,
) {
    match frame {
        WorkspaceFollowFrame::Baseline { value } | WorkspaceFollowFrame::Replace { value } => {
            *workspaces = value.items;
            *archived_session_ids = value.archived_session_ids;
            *pinned_session_ids = value.pinned_session_ids;
        }
        WorkspaceFollowFrame::Upsert { workspace } => {
            workspaces.retain(|item| item.workspace_id != workspace.workspace_id);
            workspaces.push(workspace);
        }
        WorkspaceFollowFrame::Remove { workspace_id } => {
            workspaces.retain(|item| item.workspace_id != workspace_id);
            if selected_workspace.as_deref() == Some(workspace_id.as_str()) {
                *selected_workspace = None;
            }
        }
        WorkspaceFollowFrame::Order { workspace_ids } => {
            workspaces.sort_by_key(|item| {
                workspace_ids
                    .iter()
                    .position(|id| id == &item.workspace_id)
                    .unwrap_or(usize::MAX)
            });
        }
        WorkspaceFollowFrame::Archived { archived_session_ids: archived } => {
            *archived_session_ids = archived;
        }
        WorkspaceFollowFrame::Pinned { pinned_session_ids: pinned } => {
            *pinned_session_ids = pinned;
        }
    }
}

/// Editable user theme pack open in Preferences → Appearance.
#[derive(Clone)]
pub(crate) struct ThemeEditor {
    /// Id of the installed user pack being edited.
    pub(crate) id: String,
    /// Text to install into the editor input on the next render.
    pub(crate) pending_text: Option<String>,
}

/// In-app GitHub import dialog.
#[derive(Clone)]
pub(crate) struct GithubThemeDialog {
    /// Fetch or validation status shown below the fields.
    pub(crate) status: Option<String>,
    /// Last named rejection.
    pub(crate) error: Option<String>,
    /// Whether a Check or Import worker is running.
    pub(crate) busy: bool,
}

/// One completed GitHub theme worker result.
pub(crate) enum GithubTaskResult {
    /// Check result without a cache write.
    Checked(Result<CheckedTheme, String>),
    /// Import result with the cached pack id.
    Imported(Result<String, String>),
    /// Explicit update result for one cached pack.
    Updated {
        id: String,
        result: Result<RefreshOutcome, String>,
    },
}

/// Host settings editor state for the gpui Settings surface.
pub(crate) struct HostSettingsView {
    pub(crate) namespace: Option<String>,
    pub(crate) draft: Option<String>,
    pub(crate) message: Option<String>,
}


impl DesktopApp {
    pub(crate) fn new(
        services: ShellServices,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let ShellServices {
            quitting,
            show_requested,
            hide_requested,
            hidden,
            tray_available,
            ui_preferences_path,
            ui_themes_path,
            spawn_host,
            commands,
        } = services;
        let (ui_preferences, ui_preferences_load_error) =
            match UiPreferences::load(&ui_preferences_path) {
                Ok(preferences) => (preferences, None),
                Err(error) => {
                    eprintln!("dsh: UI preferences unavailable; defaults are active: {error}");
                    (UiPreferences::default(), Some(error.to_string()))
                }
            };
        let _preferences_load_error = ui_preferences_load_error;
        let show_explorer = ui_preferences.show_workspace_explorer;
        let host_receiver = if spawn_host {
            let (host_sender, host_receiver) = mpsc::channel();
            std::thread::spawn(move || {
                let _ = host_sender.send(Host::start());
            });
            Some(host_receiver)
        } else {
            None
        };
        let theme_catalog = ThemeCatalog::load(&ui_themes_path);
        let theme = Theme::new_with_catalog(
            &ui_preferences,
            appearance_is_dark(window.appearance()),
            &theme_catalog,
        );
        // Escape resolves to `Dismiss`; the listener is global so overlays close
        // whichever node holds focus, including a palette search field.
        let shell = cx.entity().downgrade();
        let dismiss_shell = shell.clone();
        App::on_action(cx, move |_: &super::Dismiss, cx: &mut App| {
            let Some(shell) = dismiss_shell.upgrade() else {
                return;
            };
            shell.update(cx, |app, cx| {
                if app.dismiss_overlay() {
                    cx.notify();
                }
            });
        });
        // Changes-surface keys are window-wide chords; each handler reads the
        // frame's typing focus, so a keystroke that belongs to a text field never
        // runs a Git verb.
        on_changes_shortcut::<super::ChangesHunkPrevious>(
            cx,
            shell.clone(),
            ChangesShortcut::HunkPrevious,
        );
        on_changes_shortcut::<super::ChangesHunkNext>(cx, shell.clone(), ChangesShortcut::HunkNext);
        on_changes_shortcut::<super::ChangesStage>(cx, shell.clone(), ChangesShortcut::Stage);
        on_changes_shortcut::<super::ChangesUnstage>(cx, shell.clone(), ChangesShortcut::Unstage);
        on_changes_shortcut::<super::ChangesDiscard>(cx, shell.clone(), ChangesShortcut::Discard);
        App::on_action(cx, move |_: &super::CommitStaged, cx: &mut App| {
            let Some(shell) = shell.upgrade() else {
                return;
            };
            shell.update(cx, |app, cx| {
                if app.commit_from_keyboard(cx) {
                    cx.notify();
                } else {
                    cx.propagate();
                }
            });
        });
        let task = cx.spawn_in(window, async move |weak, cx| {
            loop {
                gpui::Timer::after(std::time::Duration::from_millis(40)).await;
                let alive = weak
                    .update_in(cx, |app, window, cx| app.tick(window, cx))
                    .unwrap_or(false);
                if !alive {
                    break;
                }
            }
        });
        let git_conflict_focus = cx.focus_handle();
        Self {

            poll_task: Some(task),
            repaint_soon: false,
            close_requested: false,
            preferences_draft: None,
            preferences_message: None,
            ui_themes_path,
            theme_catalog,
            theme_message: None,
            theme_editor: None,
            theme_pending_import: None,
            theme_export_id: None,
            theme_remove_confirm: None,
            github_dialog: None,
            github_receiver: None,
            theme,
            host_settings_view: HostSettingsView {
                namespace: None,
                draft: None,
                message: None,
            },
            inputs: std::collections::HashMap::new(),
            typing_focus: None,
            transcript_scroll: gpui::ScrollHandle::new(),
            scroll_transcript_to_bottom: true,
            model_menu_open: false,
            commands,
            host: None,
            host_receiver,
            started_at: std::time::Instant::now(),
            connect_receiver: None,
            sessions_receiver: None,
            archived_session_ids: Vec::new(),
            pinned_session_ids: Vec::new(),
            archive_receiver: None,
            pin_receiver: None,
            workspace_status: None,
            workspaces: Vec::new(),
            selected_workspace: None,
            workspace_receiver: None,
            workspace_error: None,
            workspace_create_receiver: None,
            file_search_query: String::new(),
            file_search_items: Vec::new(),
            file_search_scanned: 0,
            file_search_truncated: false,
            file_search_loading: false,
            file_search_error: None,
            file_search_receiver: None,
            palette_paths: Vec::new(),
            palette_paths_for: String::new(),
            palette_paths_receiver: None,
            handoff_path: "HANDOFF.md".to_owned(),
            handoff_preview: None,
            handoff_status: None,
            handoff_receiver: None,
            session_query: String::new(),
            session_search: None,
            session_title: String::new(),
            fork_seq_text: String::new(),
            sessions_status: String::new(),
            sessions_error: None,
            sessions_failure_context: "",
            host_url: None,
            transport: None,
            streams: None,
            sessions: Vec::new(),
            selected_session: None,
            agent_tabs: Vec::new(),
            hovered_buffer: None,
            page_has_more: false,
            page_loading: false,
            page_before_seq: None,
            page_error: None,
            page_receiver: None,
            page_through_seq: None,
            chat_text: String::new(),
            prompt_mode: PromptMode::Queue,
            prompt_history: Vec::new(),
            prompt_history_index: None,
            prompt_history_open: false,
            file_picker_open: false,
            file_picker_query: String::new(),
            pending_composer_text: None,
            session_state_dirty: false,
            session_state_changed_at: None,
            last_session_poll: std::time::Instant::now(),
            focus_composer: false,
            chrome_focus: ChromeFocus {
                menu: cx.focus_handle(),
                search: cx.focus_handle(),
                workspace: cx.focus_handle(),
                status_tools: cx.focus_handle(),
                git_branch: cx.focus_handle(),
                git_sync: cx.focus_handle(),
                git_conflict: git_conflict_focus.clone(),
            },
            pending_chrome_focus: None,
            cancel_receiver: None,
            dialog: None,
            attachment: None,
            attachment_name: String::new(),
            attachment_receiver: None,
            attachment_error: None,
            chat_loading: false,
            chat_receiver: None,
            transcript: Transcript::default(),
            session_transcripts: std::collections::VecDeque::new(),
            assistant_streaming: false,
            assistant_draft: String::new(),
            approval_events_receiver: None,
            approval_commands: None,
            pending_approvals: Vec::new(),
            pending_questions: Vec::new(),
            attention_reserved_height: 0.0,
            deliverables: Vec::new(),
            diagnostics: Vec::new(),
            diagnostics_workspace: None,
            diagnostics_loading: false,
            diagnostics_error: None,
            diagnostics_receiver: None,
            diagnose_receiver: None,
            market_indexes: Vec::new(),
            market_results: Vec::new(),
            market_url: None,
            market_query: String::new(),
            market_receiver: None,
            market_status: None,
            market_loading: false,
            feedback_items: Vec::new(),
            feedback_receiver: None,
            feedback_put_receiver: None,
            feedback_status: None,
            ui_preferences_path,
            ui_preferences,
            settings_description: None,
            settings_loading: false,
            settings_error: None,
            settings_receiver: None,
            settings_update_receiver: None,
            show_settings: false,
            preferences_section: PreferencesSection::General,
            preferences_focus: cx.focus_handle(),
            preferences_focus_return: None,
            focus_preferences: false,
            restore_preferences_focus: false,
            preview_document: None,
            preview_image: None,
            preview_path: String::new(),
            preview_loading: false,
            preview_error: None,
            preview_receiver: None,
            preview_draft: String::new(),
            preview_target_line: None,
            pending_preview_text: None,
            preview_save_receiver: None,
            preview_save_status: None,
            document_new_path: String::new(),
            document_delete_confirm: false,
            session_delete_confirm: None,
            document_op_status: None,
            document_op_receiver: None,
            show_document: false,
            show_plugins: false,
            show_skills: false,
            show_market: false,
            show_goals: false,
            top_menu_open: false,
            workspace_picker_open: false,
            goal: None,
            goal_loading: false,
            goal_mutation_loading: false,
            goal_receiver: None,
            goal_mutation_receiver: None,
            goal_objective: String::new(),
            goal_max_rounds: String::new(),
            clear_goal_confirmation: None,
            goal_error: None,
            goal_status: None,
            plan_command_receiver: None,
            plan_command_loading: false,
            plan_command_status: None,
            active_view: ActiveView::Sessions,
            skills: None,
            skills_query: String::new(),
            selected_skill: None,
            skills_receiver: None,
            skills_write_receiver: None,
            skills_status: None,
            skills_loading: false,
            skills_error: None,
            model_catalog: None,
            model_selection: None,
            model_error: None,
            model_receiver: None,
            model_requested_at: None,
            model_select_receiver: None,
            model_init_receiver: None,
            plugins_snapshot: None,
            plugins_loading: false,
            plugins_error: None,
            plugins_receiver: None,
            plugin_bundles: Vec::new(),
            managed_plugins: Vec::new(),
            plugin_write_receiver: None,
            plugin_write_status: None,
            terminal: TerminalPanel::default(),
            terminal_extra: Vec::new(),
            terminal_pane_widths: Vec::new(),
            terminal_focus: 0,
            bottom_tool: None,
            explorer: WorkspaceExplorer::default(),
            show_explorer,
            explorer_fullscreen: false,
            show_changes: true,
            changes_tab: ChangesTab::Diff,
            git_status: None,
            git_branches: None,
            git_selected_path: None,
            git_diff_staged: false,
            git_focused_hunk: None,
            git_diff: None,
            git_state: None,
            git_conflict_view: None,
            git_conflict_focus,
            git_conflict_pending: None,
            git_stash: None,
            git_stash_pending: false,
            git_stash_message_clear: false,
            git_remote_summary: None,
            git_menu: None,
            git_remote_pending: None,
            git_confirmation: None,
            git_refresh_pending: false,
            git_refresh_probe_started_at: None,
            git_selection: Vec::new(),
            git_filter: String::new(),
            git_reviewed: HashMap::new(),
            git_commit_message: String::new(),
            git_amend: false,
            git_sign_off: false,
            git_recent_open: false,
            git_recent_messages: Vec::new(),
            git_log_error: None,
            git_log_loading: false,
            git_log_pending: false,
            git_history: Vec::new(),
            git_history_loaded: false,
            git_history_footer: HistoryFooter::End,
            git_history_selected: None,
            git_history_show: None,
            git_history_error: None,
            git_history_show_error: None,
            git_history_loading: false,
            git_history_show_loading: false,
            git_history_pending: None,
            git_history_show_pending: None,
            git_history_head: None,
            git_history_focus: cx.focus_handle(),
            git_history_scroll: ScrollHandle::default(),
            git_push_pending: false,
            git_commit_input_clear: false,
            git_state_pending: false,
            git_diff_pending: false,
            git_error: None,
            git_loading: false,
            git_receiver: None,
            show_session_menu: false,
            status_menu_open: false,
            panel_drag: None,
            last_divider_click: None,
            show_about: false,
            palette_open: false,
            palette_query: String::new(),
            focus_palette: false,
            palette_selected: 0,
            host_commands: Vec::new(),
            host_commands_receiver: None,
            host_commands_loading: false,
            host_commands_error: None,
            host_command_receiver: None,
            host_command_loading: false,
            pending_slash_line: None,
            schedules: Vec::new(),
            schedules_session: None,
            schedules_receiver: None,
            schedules_error: None,
            schedule_delete_receiver: None,
            jobs: Vec::new(),
            jobs_session: None,
            jobs_receiver: None,
            jobs_error: None,
            job_selected: None,
            job_output: String::new(),
            job_status: String::new(),
            job_receiver: None,
            status: "Starting Host sidecar…".to_owned(),
            update_prompt: None,
            update_error: None,
            update_open_receiver: None,
            update_status: None,
            retry_after_host_start: false,
            quitting,
            show_requested,
            hide_requested,
            hidden,
            tray_available,
            auto_connect: auto_connect_enabled(
                std::env::var("DSH_DESKTOP_AUTO_CONNECT").ok().as_deref(),
            ),
        }
    }

    pub(crate) fn connect(&mut self) {
        if self.host_receiver.is_some() || self.connect_receiver.is_some() {
            return;
        }
        let launch_url = self
            .host
            .as_ref()
            .and_then(Host::launch_url)
            .map(str::to_owned);
        let Some(launch_url) = launch_url else {
            if self
                .host
                .as_ref()
                .is_some_and(|host| host.error().is_some())
            {
                self.restart_host();
            }
            return;
        };
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(HttpTransport::authenticate(&launch_url));
        });
        self.connect_receiver = Some(receiver);
        self.status = "Authenticating with Host…".to_owned();
    }

    /// Replaces a failed Host before exchanging a new launch token.
    fn restart_host(&mut self) {
        if self.host_receiver.is_some() || self.connect_receiver.is_some() {
            return;
        }
        let previous = self.host.take();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            drop(previous);
            let _ = sender.send(Host::start());
        });
        self.host_receiver = Some(receiver);
        self.retry_after_host_start = true;
        self.host_url = None;
        self.transport = None;
        self.streams = None;
        self.status = "Restarting Host sidecar…".to_owned();
    }

    fn start_remote_events(&mut self, transport: HttpTransport, streams: RemoteStreamMux) {
        let (event_sender, event_receiver) = mpsc::channel();
        let (command_sender, command_receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let events = match RemoteEvents::connect(transport, &streams) {
                Ok(events) => events,
                Err(error) => {
                    let _ = event_sender.send(Err(error));
                    return;
                }
            };
            loop {
                while let Ok(command) = command_receiver.try_recv() {
                    let result = match command {
                        ApprovalCommand::Answer(pending, decision) => {
                            pending.answer(&events, decision)
                        }
                        ApprovalCommand::Delegate(invocation) => events.submit::<ApprovalDecision>(
                            &invocation,
                            dsh_client_core::remote_events::RemoteEventOutcome::Next,
                        ),
                        ApprovalCommand::AnswerQuestion(pending, answer) => {
                            pending.answer(&events, answer)
                        }
                    };
                    if let Err(error) = result {
                        let _ = event_sender.send(Err(error));
                    }
                }
                match events.recv_timeout(std::time::Duration::from_millis(25)) {
                    Ok(Some(event)) => {
                        if matches!(event, RemoteEvent::Ready { .. }) {
                            eprintln!("dsh: native events=ready");
                        }
                        if event_sender.send(Ok(event)).is_err() {
                            return;
                        }
                    }
                    Ok(None) => {}
                    Err(error) => {
                        let _ = event_sender.send(Err(error));
                        return;
                    }
                }
            }
        });
        self.approval_events_receiver = Some(event_receiver);
        self.approval_commands = Some(command_sender);
        self.pending_approvals.clear();
        self.pending_questions.clear();
    }

    pub(crate) fn refresh_sessions(&mut self) {
        self.request_sessions(SessionOperation::List);
    }

    pub(crate) fn request_sessions(&mut self, operation: SessionOperation) {
        if self.sessions_receiver.is_some() {
            return;
        }
        let Some(transport) = self.transport.clone() else {
            return;
        };
        self.sessions_status = match &operation {
            SessionOperation::List => "Loading sessions…",
            SessionOperation::Search(_) => "Searching sessions…",
            SessionOperation::Create(_) => "Creating session…",
            SessionOperation::Rename(_) => "Renaming session…",
            SessionOperation::Fork(_) => "Forking session…",
            SessionOperation::Delete(_) => "Deleting session…",
        }
        .to_owned();
        self.sessions_failure_context = match &operation {
            SessionOperation::List => "List refresh failed; previous inventory and selection retained. Refresh to retry",
            SessionOperation::Search(_) => "Search failed; previous results retained. Search again to retry",
            SessionOperation::Create(_) => "Create outcome unknown; refresh/search to confirm remote state before retrying",
            SessionOperation::Rename(_) => "Rename outcome unknown; reload the selected session to confirm its title before retrying",
            SessionOperation::Fork(_) => "Fork outcome unknown; refresh/search to confirm remote state before retrying",
            SessionOperation::Delete(_) => "Delete outcome unknown; refresh the list before retrying",
        };
        self.sessions_error = None;
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let client = SessionsClient::new(&transport);
            let result = match operation {
                SessionOperation::List => SessionViewer::new(&transport)
                    .list(None)
                    .map(SessionUpdate::List),
                SessionOperation::Search(query) => client.search(query).map(SessionUpdate::Search),
                SessionOperation::Create(workspace_id) => client
                    .create(CreateSessionRequest {
                        workspace_id,
                        cwd: None,
                        session_id: None,
                        agent_preset: None,
                    })
                    .map(|session| SessionUpdate::Created(session.session_id)),
                SessionOperation::Rename(request) => client
                    .rename(request)
                    .map(|response| SessionUpdate::Renamed(response.title)),
                SessionOperation::Fork(request) => client
                    .fork(request)
                    .map(|response| SessionUpdate::Forked(response.session_id)),
                SessionOperation::Delete(session_id) => client
                    .delete(session_id)
                    .map(|response| SessionUpdate::Deleted(response.session_id)),
            };
            let _ = sender.send(result);
        });
        self.sessions_receiver = Some(receiver);
    }

    /// Opens the Preferences overlay without replacing the shell behind it.
    pub(crate) fn open_settings(&mut self) {
        if !self.show_settings {
            self.preferences_section = PreferencesSection::General;
            self.focus_preferences = true;
        }
        self.show_settings = true;
        if self.settings_description.is_some() || self.settings_loading {
            return;
        }
        self.refresh_settings();
    }

    /// Closes the Preferences overlay and schedules the focus return.
    pub(crate) fn close_preferences(&mut self) {
        if !self.show_settings {
            return;
        }
        self.show_settings = false;
        self.restore_preferences_focus = true;
        if self.active_view == ActiveView::Settings {
            self.active_view = ActiveView::Sessions;
        }
    }

    /// Starts the Host-owned signed update flow from the native Settings page.
    pub(crate) fn open_updates(&mut self) {
        if self.update_open_receiver.is_some() {
            return;
        }
        let Some(transport) = self.transport.clone() else {
            self.update_status = Some("Connect to the Host before checking for updates.".into());
            return;
        };
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let result = transport
                .post_json_with_timeout(
                    "/__dsh-desktop/updates-open",
                    "application/json",
                    b"null".to_vec(),
                    std::time::Duration::from_secs(20 * 60),
                    &AtomicBool::new(false),
                )
                .map(|_| ());
            let _ = sender.send(result);
        });
        self.update_open_receiver = Some(receiver);
        self.update_status = Some("Checking for desktop updates…".into());
    }


    fn model_catalog_empty(&self) -> bool {
        self.model_receiver.is_none()
            && self
                .model_catalog
                .as_ref()
                .is_none_or(|catalog| catalog.groups.iter().all(|group| group.models.is_empty()))
    }

    /// Labels the model picker with the chosen model or the reason there is none.
    fn model_picker_label(&self) -> String {
        model_picker_label(
            self.model_selection.as_ref(),
            self.model_error.as_deref(),
            self.model_catalog.as_ref(),
            self.model_receiver.is_some(),
        )
    }

    /// Whether the interface currently needs a CJK fallback face.
    fn cjk_required(&self) -> bool {
        self.transcript.has_cjk()
            || crate::fonts::contains_cjk(&self.chat_text)
            || crate::fonts::contains_cjk(&self.session_title)
            || self.sessions.iter().any(|session| {
                session
                    .projections
                    .as_ref()
                    .and_then(|projections| projections.get("title"))
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(crate::fonts::contains_cjk)
            })
    }


    pub(crate) fn refresh_models(&mut self) {
        if self.model_receiver.is_some() {
            return;
        }
        let Some(transport) = self.transport.clone() else {
            return;
        };
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(ModelClient::new(&transport).catalog());
        });
        self.model_receiver = Some(receiver);
        self.model_requested_at = Some(std::time::Instant::now());
        self.model_error = None;
    }

    pub(crate) fn select_model(&mut self, selection: ModelSelection) {
        let (Some(session), Some(transport)) =
            (self.selected_session.clone(), self.transport.clone())
        else {
            self.model_error = Some("Select a Session before choosing a model".to_owned());
            return;
        };
        let request = SelectModelRequest {
            session_id: session,
            selection,
        };
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(ModelClient::new(&transport).select(request));
        });
        self.model_select_receiver = Some(receiver);
        self.model_error = None;
    }

    pub(crate) fn initialize_default_model(&mut self) {
        if self.model_init_receiver.is_some() {
            return;
        }
        let Some(transport) = self.transport.clone() else {
            self.model_error = Some("Connect before selecting the default model".to_owned());
            return;
        };
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(ModelClient::new(&transport).initialize_default_model());
        });
        self.model_init_receiver = Some(receiver);
        self.model_error = None;
    }

    pub(crate) fn refresh_plugins(&mut self) {
        if self.plugins_loading {
            return;
        }
        let Some(transport) = self.transport.clone() else {
            self.plugins_error = Some("Connect to load plugins".to_owned());
            return;
        };
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let client = PluginsClient::new(&transport);
            let result = client
                .list()
                .map(|snapshot| (snapshot, client.list_bundles(), client.list_plugins()))
                .map(|(snapshot, bundles, plugins)| {
                    (
                        snapshot,
                        bundles.unwrap_or_default(),
                        plugins.unwrap_or_default(),
                    )
                });
            let _ = sender.send(result);
        });
        self.plugins_receiver = Some(receiver);
        self.plugins_loading = true;
        self.plugins_error = None;
    }

    fn record_deliverables(&mut self, event: &dsh_client_core::follow::SessionWireEvent) {
        record_deliverables(&mut self.deliverables, event);
    }

    pub(crate) fn write_plugin(&mut self, action: PluginAction) {
        if self.plugin_write_receiver.is_some() {
            return;
        }
        let Some(transport) = self.transport.clone() else {
            self.plugins_error = Some("Connect to change plugins".to_owned());
            return;
        };
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let client = PluginsClient::new(&transport);
            let result = match action {
                PluginAction::SetPlugin { id, enabled } => client.set_plugin_enabled(&id, enabled),
                PluginAction::SetBundle { name, enabled } => {
                    client.set_bundle_enabled(&name, enabled)
                }
                PluginAction::RemoveBundle { name } => client.remove_bundle(&name),
            };
            let _ = sender.send(result);
        });
        self.plugin_write_receiver = Some(receiver);
        self.plugin_write_status = Some("Applying plugin change…".to_owned());
    }

    pub(crate) fn refresh_feedback(&mut self) {
        if self.feedback_receiver.is_some() {
            return;
        }
        let Some(transport) = self.transport.clone() else {
            self.feedback_status = Some("Connect to load feedback".to_owned());
            return;
        };
        let Some(session) = self.selected_session.clone() else {
            self.feedback_items.clear();
            return;
        };
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(MessageFeedbackClient::new(&transport).list(session));
        });
        self.feedback_receiver = Some(receiver);
        self.feedback_status = None;
    }

    pub(crate) fn refresh_market_indexes(&mut self) {
        if self.market_loading {
            return;
        }
        let Some(transport) = self.transport.clone() else {
            self.market_status = Some("Connect to load marketplace".to_owned());
            return;
        };
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(MarketplaceClient::new(&transport).list_indexes());
        });
        self.market_receiver = Some(receiver);
        self.market_loading = true;
        self.market_status = None;
    }

    pub(crate) fn search_market(&mut self) {
        if self.market_loading {
            return;
        }
        let Some(transport) = self.transport.clone() else {
            self.market_status = Some("Connect to search".to_owned());
            return;
        };
        let Some(url) = self.market_url.clone() else {
            self.market_status = Some("No market index configured".to_owned());
            return;
        };
        let query = self.market_query.clone();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(MarketplaceClient::new(&transport).search(&url, &query));
        });
        self.market_receiver = Some(receiver);
        self.market_loading = true;
        self.market_status = None;
    }

    pub(crate) fn install_market_entry(&mut self, spec: String, options: MarketplaceInstallOptions) {
        if self.market_loading {
            return;
        }
        let Some(transport) = self.transport.clone() else {
            self.market_status = Some("Connect to install".to_owned());
            return;
        };
        let status_spec = spec.clone();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let client = MarketplaceClient::new(&transport);
            let result = (|| -> Result<String, Error> {
                let inspect = client.inspect(&spec, &options)?;
                let status = inspect
                    .get("status")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("");
                if status != "accepted" {
                    let problem = inspect
                        .get("problem")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("unknown");
                    let reason = inspect
                        .get("reason")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("the Host refused this install target");
                    return Err(Error::Remote {
                        code: "inspect-rejected".into(),
                        message: format!("inspect refused {spec}: {problem}: {reason}"),
                        details: serde_json::Map::new(),
                    });
                }
                let install = client.install_bundle(&spec, &options)?;
                let application = install
                    .get("application")
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(|| Error::InvalidEnvelope("missing application result"))?;
                Ok(format!("{spec} -> {application}"))
            })();
            let _ = sender.send(result.map(serde_json::Value::String));
        });
        self.market_receiver = Some(receiver);
        self.market_loading = true;
        self.market_status = Some(format!("Installing {status_spec}…"));
    }

    pub(crate) fn refresh_skills(&mut self) {
        if self.skills_loading {
            return;
        }
        let Some(transport) = self.transport.clone() else {
            self.skills_error = Some("Connect to load skills".to_owned());
            return;
        };
        let Some(session) = self.selected_session.clone() else {
            self.skills_error = Some("Select a session to list its skills".to_owned());
            return;
        };
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(SkillsClient::new(&transport).list(session));
        });
        self.skills_receiver = Some(receiver);
        self.skills_loading = true;
        self.skills_error = None;
    }

    /// Enables or disables one skill through the Host, which enforces the set
    /// on every catalog read (model catalogs and `/skill` alike).
    pub(crate) fn set_skill_enabled(&mut self, name: String, enabled: bool) {
        if self.skills_write_receiver.is_some() {
            return;
        }
        let Some(transport) = self.transport.clone() else {
            self.skills_status = Some("Connect before changing skill enablement".to_owned());
            return;
        };
        let mut names = self
            .skills
            .as_ref()
            .map(|skills| skills.disabled_skills.clone())
            .unwrap_or_default();
        names.retain(|existing| existing != &name);
        if !enabled {
            names.push(name.clone());
        }
        names.sort();
        names.dedup();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(SkillsClient::new(&transport).set_disabled(names));
        });
        self.skills_write_receiver = Some(receiver);
        self.skills_status = Some(format!(
            "{} {name}…",
            if enabled { "Enabling" } else { "Disabling" }
        ));
    }

    pub(crate) fn refresh_settings(&mut self) {
        if self.settings_loading || self.settings_update_receiver.is_some() {
            return;
        }
        let Some(transport) = self.transport.clone() else {
            self.settings_error = Some("Connect to load settings".to_owned());
            return;
        };
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(SettingsClient::new(&transport).describe());
        });
        self.settings_receiver = Some(receiver);
        self.settings_loading = true;
        self.settings_error = None;
    }

    pub(crate) fn mutate_settings(&mut self, request: dsh_client_core::settings::SettingsMutateRequest) {
        if self.settings_loading || self.settings_update_receiver.is_some() {
            return;
        }
        let Some(transport) = self.transport.clone() else {
            self.host_settings_view.message =
                Some("Not connected. Refresh settings before editing again.".into());
            return;
        };
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(SettingsClient::new(&transport).mutate(request));
        });
        self.settings_update_receiver = Some(receiver);
    }


    pub(crate) fn read_document(&mut self) {
        let (Some(session_id), Some(transport)) =
            (self.selected_session.clone(), self.transport.clone())
        else {
            self.preview_error = Some("Select a connected session before reading a file.".into());
            return;
        };
        let path = self.preview_path.trim().to_owned();
        if path.is_empty() {
            self.preview_error = Some("Enter a Host file path.".into());
            return;
        }
        let (sender, receiver) = mpsc::channel();
        let image_path = is_image_path(&path);
        if !image_path && !Self::is_office_path_now(&path) {
            self.diagnose_file(path.clone());
        }
        std::thread::spawn(move || {
            let client = WorkspaceFilesClient::new(&transport);
            let scope = WorkspaceFileScope { session_id };
            let result = if image_path {
                read_image(&client, scope, &path)
            } else {
                client
                    .read(
                        scope,
                        path.clone(),
                        WorkspaceFileRange {
                            offset: None,
                            limit: Some(500),
                        },
                    )
                    .map(|page| {
                        let name = page
                            .stat
                            .absolute_path
                            .rsplit('/')
                            .next()
                            .unwrap_or(&path)
                            .to_owned();
                        PreviewDocument::Text(TextDocument {
                            name,
                            host_path: page.stat.absolute_path,
                            text: page.text,
                            version: Some(page.stat.version),
                        })
                    })
                    .map_err(|error| error.to_string())
            };
            let _ = sender.send(result);
        });
        self.preview_receiver = Some(receiver);
        self.preview_loading = true;
        self.preview_error = None;
    }

    /// Saves the edited preview draft back to the Host file, guarded by the
    /// version from the last read. A stale version is reported, never forced.
    pub(crate) fn save_preview_document(&mut self) {
        if self.preview_save_receiver.is_some() {
            return;
        }
        let (Some(session_id), Some(transport)) =
            (self.selected_session.clone(), self.transport.clone())
        else {
            self.preview_error = Some("Select a connected session before saving.".into());
            return;
        };
        let Some(document) = self.preview_document.clone() else {
            self.preview_error = Some("No text document is loaded to save.".into());
            return;
        };
        let content = self.preview_draft.clone();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let client = WorkspaceFilesClient::new(&transport);
            let result = client.write(
                WorkspaceFileScope { session_id },
                document.host_path,
                content,
                document.version,
            );
            let _ = sender.send(result);
        });
        self.preview_save_receiver = Some(receiver);
        self.preview_save_status = Some("Saving to Host…".to_owned());
    }

    /// Renames the loaded document to the drafted path, guarded by its version.
    pub(crate) fn rename_preview_document(&mut self) {
        if self.document_op_receiver.is_some() {
            return;
        }
        let (Some(session_id), Some(transport)) =
            (self.selected_session.clone(), self.transport.clone())
        else {
            self.document_op_status = Some("Select a connected session first.".into());
            return;
        };
        let Some(document) = self.preview_document.clone() else {
            self.document_op_status = Some("No text document is loaded to rename.".into());
            return;
        };
        let destination = self.document_new_path.trim().to_owned();
        if destination.is_empty() {
            self.document_op_status = Some("Enter a destination path.".into());
            return;
        }
        let source = document.host_path.clone();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let client = WorkspaceFilesClient::new(&transport);
            let result = client
                .rename(
                    WorkspaceFileScope { session_id },
                    source,
                    destination,
                    document.version,
                )
                .map(DocumentMutation::Renamed);
            let _ = sender.send(result);
        });
        self.document_op_receiver = Some(receiver);
        self.document_op_status = Some("Renaming on the Host…".to_owned());
    }

    /// Deletes the loaded document after a second click confirms the choice.
    pub(crate) fn delete_preview_document(&mut self) {
        if self.document_op_receiver.is_some() {
            return;
        }
        if !self.document_delete_confirm {
            self.document_delete_confirm = true;
            self.document_op_status =
                Some("Click Delete again to remove the file from the Host.".to_owned());
            return;
        }
        let (Some(session_id), Some(transport)) =
            (self.selected_session.clone(), self.transport.clone())
        else {
            self.document_op_status = Some("Select a connected session first.".into());
            return;
        };
        let Some(document) = self.preview_document.clone() else {
            self.document_op_status = Some("No text document is loaded to delete.".into());
            return;
        };
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let client = WorkspaceFilesClient::new(&transport);
            let result = client
                .delete(
                    WorkspaceFileScope { session_id },
                    document.host_path,
                    document.version,
                )
                .map(|_| DocumentMutation::Deleted);
            let _ = sender.send(result);
        });
        self.document_op_receiver = Some(receiver);
        self.document_delete_confirm = false;
        self.document_op_status = Some("Deleting on the Host…".to_owned());
    }

    pub(crate) fn read_office_document(&mut self) {
        let (Some(session_id), Some(transport)) =
            (self.selected_session.clone(), self.transport.clone())
        else {
            self.preview_error = Some("Select a connected session before reading a file.".into());
            return;
        };
        let path = self.preview_path.trim().to_owned();
        if path.is_empty() {
            self.preview_error = Some("Enter a Host file path.".into());
            return;
        }
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let scope = WorkspaceFileScope { session_id };
            let result = OfficeToPdfClient::new(&transport)
                .render(scope, path.clone(), "foreground")
                .map_err(|error| error.to_string())
                .and_then(|preview| {
                    // Keep parallel previews from overwriting the same temporary PDF.
                    let fingerprint = path.bytes().fold(0_u64, |hash, byte| {
                        hash.wrapping_mul(31).wrapping_add(u64::from(byte))
                    });
                    let directory = std::env::temp_dir().join(format!(
                        "dsh-office-preview-{}-{fingerprint:x}",
                        std::process::id()
                    ));
                    let _scratch = PreviewScratchDirectory(directory.clone());
                    std::fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
                    let pdf_path = directory.join("document.pdf");
                    std::fs::write(&pdf_path, &preview.pdf).map_err(|error| error.to_string())?;
                    let prefix = directory.join("page");
                    let output = std::process::Command::new("pdftoppm")
                        .args([
                            "-png",
                            "-r",
                            "110",
                            "-f",
                            "1",
                            "-l",
                            "1",
                            pdf_path.to_str().unwrap_or_default(),
                            prefix.to_str().unwrap_or_default(),
                        ])
                        .output()
                        .map_err(|error| error.to_string())?;
                    if !output.status.success() {
                        return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned());
                    }
                    let png_path = format!("{}-1.png", prefix.display());
                    let bytes = std::fs::read(&png_path).map_err(|error| error.to_string())?;
                    let image =
                        image::load_from_memory(&bytes).map_err(|error| error.to_string())?;
                    let rgba = image.to_rgba8();
                    let (width, height) = rgba.dimensions();
                    let name = path.rsplit('/').next().unwrap_or(&path).to_owned();
                    let mut document_name = name;
                    if !preview.missing_fonts.is_empty() {
                        document_name = format!(
                            "{document_name} (missing fonts: {})",
                            preview.missing_fonts.join(", ")
                        );
                    }
                    Ok(PreviewDocument::Image(ImageDocument {
                        name: document_name,
                        host_path: path,
                        size: [width as usize, height as usize],
                        rgba: rgba.into_raw(),
                    }))
                });
            let _ = sender.send(result);
        });
        self.preview_receiver = Some(receiver);
        self.preview_loading = true;
        self.preview_error = None;
    }

    fn is_office_path(path: &str) -> bool {
        matches!(
            path.rsplit('.').next().map(str::to_lowercase).as_deref(),
            Some("doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx")
        )
    }

    /// Copies the composer text into the selected Session's draft.
    pub(crate) fn remember_active_draft(&mut self) {
        let Some(session_id) = self.selected_session.clone() else {
            return;
        };
        let changed = remember_draft(
            &mut self.ui_preferences.session_drafts,
            &session_id,
            &self.chat_text,
        );
        if changed {
            self.mark_session_state_dirty();
        }
    }

    /// Restores the selected Session's draft into the composer.
    fn restore_active_draft(&mut self, session_id: &str) {
        let draft = self
            .ui_preferences
            .session_drafts
            .get(session_id)
            .cloned()
            .unwrap_or_default();
        self.chat_text = draft.clone();
        self.pending_composer_text = Some(draft);
    }

    /// Marks the last-Session state and drafts for a debounced save.
    fn mark_session_state_dirty(&mut self) {
        self.session_state_dirty = true;
        self.session_state_changed_at = Some(std::time::Instant::now());
    }

    /// Writes the last-Session state and drafts.
    fn flush_session_state(&mut self) {
        if !self.session_state_dirty {
            return;
        }
        self.session_state_dirty = false;
        if let Err(error) = self.ui_preferences.save(&self.ui_preferences_path) {
            self.status = format!("Could not save Session drafts: {error}");
        }
    }

    /// Writes Session state after edits have been quiet for the debounce window.
    fn flush_session_state_if_quiet(&mut self) {
        if !self.session_state_dirty {
            return;
        }
        let quiet = self
            .session_state_changed_at
            .is_none_or(|changed| changed.elapsed() >= std::time::Duration::from_millis(750));
        if quiet {
            self.flush_session_state();
        }
    }

    /// Whether the selected Session has a Host-reported run in flight.
    pub(crate) fn selected_session_running(&self) -> bool {
        session_is_running(&self.sessions, self.selected_session.as_deref())
    }

    /// Drops Git state when no Session is selected or the Host is gone.
    fn reset_git(&mut self) {
        self.git_status = None;
        self.git_branches = None;
        self.git_selected_path = None;
        self.git_diff = None;
        self.git_state = None;
        self.git_conflict_view = None;
        self.git_conflict_pending = None;
        self.git_stash = None;
        self.git_stash_pending = false;
        self.git_stash_message_clear = false;
        self.git_remote_summary = None;
        self.git_remote_pending = None;
        self.git_confirmation = None;
        self.git_refresh_pending = false;
        self.git_refresh_probe_started_at = None;
        self.git_selection.clear();
        self.git_reviewed.clear();
        self.git_commit_message.clear();
        self.git_amend = false;
        self.git_recent_messages.clear();
        self.git_log_error = None;
        self.git_log_loading = false;
        self.git_log_pending = false;
        self.reset_git_history();
        self.git_push_pending = false;
        self.git_commit_input_clear = false;
        self.git_state_pending = false;
        self.git_diff_pending = false;
        self.git_error = None;
        self.git_loading = false;
        self.git_receiver = None;
    }

    /// Drops the History page, its selection, and its scroll position.
    fn reset_git_history(&mut self) {
        self.git_history.clear();
        self.git_history_loaded = false;
        self.git_history_footer = HistoryFooter::End;
        self.git_history_selected = None;
        self.git_history_show = None;
        self.git_history_error = None;
        self.git_history_show_error = None;
        self.git_history_loading = false;
        self.git_history_show_loading = false;
        self.git_history_pending = None;
        self.git_history_show_pending = None;
        self.git_history_head = None;
        self.git_history_scroll.set_offset(point(px(0.), px(0.)));
    }

    /// Refreshes the selected Session's repository status from the Host.
    pub(crate) fn refresh_git(&mut self) {
        let (Some(session_id), Some(transport)) =
            (self.selected_session.clone(), self.transport.clone())
        else {
            self.reset_git();
            return;
        };
        if self.git_receiver.is_some() {
            return;
        }
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let result = GitClient::new(&transport).status(session_id);
            let _ = sender.send(GitUpdate::Status(result));
        });
        self.git_receiver = Some(receiver);
        self.git_loading = true;
        self.git_error = None;
    }

    /// Loads the local branch list for the selected Session.
    pub(crate) fn refresh_git_branches(&mut self) {
        let (Some(session_id), Some(transport)) =
            (self.selected_session.clone(), self.transport.clone())
        else {
            self.reset_git();
            return;
        };
        if self.git_receiver.is_some() {
            return;
        }
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let result = GitClient::new(&transport).branches(session_id);
            let _ = sender.send(GitUpdate::Branches(result));
        });
        self.git_receiver = Some(receiver);
        self.git_loading = true;
        self.git_error = None;
    }

    /// Opens or closes one status-bar Git menu.
    ///
    /// Opening a menu closes the other chrome overlays so only one is painted,
    /// and a branch menu asks the Host for the local branch list when it opens.
    ///
    /// @param menu - Branch or sync menu to toggle.
    pub(crate) fn toggle_git_menu(&mut self, menu: GitMenu) {
        if self.git_menu == Some(menu) {
            self.git_menu = None;
            return;
        }
        self.git_menu = Some(menu);
        self.top_menu_open = false;
        self.workspace_picker_open = false;
        self.show_session_menu = false;
        self.status_menu_open = false;
        if menu == GitMenu::Branch {
            self.refresh_git_branches();
        }
    }

    /// Why branch and remote actions are disabled, or `None` when one may start.
    ///
    /// The shell runs one Git verb at a time, so a remote mutation in flight
    /// names itself rather than letting a second click queue silently.
    ///
    /// @returns The reason the menus show, or `None` when an action may start.
    pub(crate) fn git_action_blocker(&self) -> Option<String> {
        if let Some(action) = self.git_remote_pending {
            return Some(format!("{} is running; wait for its result.", action.label()));
        }
        if self.git_loading {
            return Some("A Git request is running; wait for its result.".to_owned());
        }
        if self.transport.is_none() {
            return Some("Host is offline.".to_owned());
        }
        if self.selected_session.is_none() {
            return Some("Select a Session to use its repository.".to_owned());
        }
        None
    }

    /// Confirms one merge, rebase, cherry-pick, or revert through a local branch.
    ///
    /// @param branch - Local branch the menu row names.
    /// @param commit - That branch's tip, which the cherry-pick and revert verbs take.
    /// @param action - Which branch operation to run after confirmation.
    pub(crate) fn request_git_branch_action(
        &mut self,
        branch: String,
        commit: String,
        action: GitBranchAction,
    ) {
        let Some(session_id) = self.selected_session.clone() else {
            return;
        };
        if let Some(reason) = self.git_action_blocker() {
            self.git_error = Some(reason);
            return;
        }
        let listed = self
            .git_branches
            .as_ref()
            .is_some_and(|branches| branches.branches.iter().any(|item| item.name == branch));
        if !listed {
            self.git_error = Some(format!(
                "{} unavailable: \"{branch}\" is no longer in the branch list.",
                action.label()
            ));
            self.refresh_git_branches();
            return;
        }
        self.git_confirmation = Some(GitConfirmation::BranchEntry {
            session_id,
            branch,
            commit,
            action,
        });
    }

    /// Confirms a push of the current branch before the Host runs it.
    ///
    /// @param set_upstream - Publish the branch to `origin` and record its upstream.
    pub(crate) fn request_git_push(&mut self, set_upstream: bool) {
        let Some(session_id) = self.selected_session.clone() else {
            return;
        };
        if let Some(reason) = self.git_action_blocker() {
            self.git_error = Some(reason);
            return;
        }
        self.git_confirmation = Some(GitConfirmation::Push {
            session_id,
            set_upstream,
        });
    }

    /// Loads the in-progress operation and conflict paths.
    pub(crate) fn refresh_git_state(&mut self) {
        let (Some(session_id), Some(transport)) =
            (self.selected_session.clone(), self.transport.clone())
        else {
            self.git_state = None;
            return;
        };
        if self.git_receiver.is_some() {
            self.git_state_pending = true;
            return;
        }
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let result = GitClient::new(&transport).state(session_id);
            let _ = sender.send(GitUpdate::State(result));
        });
        self.git_receiver = Some(receiver);
        self.git_loading = true;
    }

    /// Runs one remote action and reloads the repository after it settles.
    ///
    /// @param action - Fetch, pull, push, or push with an upstream.
    pub(crate) fn run_git_remote(&mut self, action: GitRemoteAction) {
        let (Some(session_id), Some(transport)) =
            (self.selected_session.clone(), self.transport.clone())
        else {
            self.reset_git();
            return;
        };
        if self.git_receiver.is_some() {
            return;
        }
        let operation = action.verb();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let client = GitClient::new(&transport);
            let result = match action {
                GitRemoteAction::Fetch => client.fetch(session_id),
                GitRemoteAction::Pull => client.pull(session_id),
                GitRemoteAction::Push => client.push(session_id),
                GitRemoteAction::PushSetUpstream => client.push_with_options(session_id, true),
            };
            let _ = sender.send(GitUpdate::Remote(operation, result));
        });
        self.git_receiver = Some(receiver);
        self.git_loading = true;
        self.git_remote_pending = Some(action);
        self.git_remote_summary = None;
        self.git_error = None;
    }

    /// Continues the in-progress merge, rebase, cherry-pick, or revert.
    pub(crate) fn continue_git_operation(&mut self) {
        self.run_git_operation(|client, session_id| client.continue_operation(session_id));
    }

    /// Aborts the in-progress merge, rebase, cherry-pick, or revert.
    pub(crate) fn abort_git_operation(&mut self) {
        self.run_git_operation(|client, session_id| client.abort_operation(session_id));
    }

    fn run_git_operation(
        &mut self,
        operation: impl FnOnce(&GitClient<'_>, String) -> Result<GitOperationResult, Error> + Send + 'static,
    ) {
        let (Some(session_id), Some(transport)) =
            (self.selected_session.clone(), self.transport.clone())
        else {
            self.reset_git();
            return;
        };
        if self.git_receiver.is_some() {
            return;
        }
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let client = GitClient::new(&transport);
            let result = operation(&client, session_id);
            let _ = sender.send(GitUpdate::Operation(result));
        });
        self.git_receiver = Some(receiver);
        self.git_loading = true;
        self.git_remote_summary = None;
        self.git_error = None;
    }

    /// Opens one unmerged path's stages in the Changes surface.
    ///
    /// Clicking a conflict path only loads the Host's stages; the view states no
    /// resolution until `git/resolve` reports one.
    ///
    /// @param path - Unmerged path the operation banner lists.
    pub(crate) fn open_git_conflict(&mut self, path: String) {
        let (Some(session_id), Some(transport)) =
            (self.selected_session.clone(), self.transport.clone())
        else {
            self.git_error = Some("Reading conflict stages needs an online Host.".to_owned());
            return;
        };
        self.show_changes = true;
        self.git_menu = None;
        self.git_conflict_view = Some(GitConflictView {
            path: path.clone(),
            conflict: None,
            loading: true,
            error: None,
        });
        if self.git_receiver.is_some() {
            self.git_conflict_pending = Some(path);
            return;
        }
        self.request_git_conflict(session_id, transport, path);
    }

    /// Closes the conflict view and returns focus to the conflict chips.
    pub(crate) fn close_git_conflict(&mut self) {
        self.git_conflict_pending = None;
        if self.git_conflict_view.take().is_some() {
            self.pending_chrome_focus = Some(ChromeTrigger::GitConflict);
        }
    }

    /// Reads one unmerged path's stages from the Host.
    fn request_git_conflict(
        &mut self,
        session_id: String,
        transport: HttpTransport,
        path: String,
    ) {
        self.git_conflict_pending = None;
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let result = GitClient::new(&transport).conflict(session_id, path.clone());
            let _ = sender.send(GitUpdate::Conflict { path, result });
        });
        self.git_receiver = Some(receiver);
        self.git_loading = true;
        self.git_error = None;
    }

    /// Opens the confirmation for keeping one conflict side.
    ///
    /// @param path - Unmerged path the view shows.
    /// @param side - `ours` or `theirs`, the side `git/resolve` keeps.
    pub(crate) fn request_resolve_conflict(&mut self, path: String, side: GitResolveSide) {
        let Some(session_id) = self.selected_session.clone() else {
            return;
        };
        if let Some(reason) = self.git_action_blocker() {
            self.git_error = Some(reason);
            return;
        }
        let listed = self
            .git_state
            .as_ref()
            .is_some_and(|state| state.conflicts.iter().any(|item| item == &path));
        if !listed {
            self.git_error = Some(format!(
                "Resolve cancelled: \"{path}\" is no longer an unmerged path."
            ));
            self.refresh_git_state();
            return;
        }
        self.git_confirmation = Some(GitConfirmation::ResolveConflict {
            session_id,
            path,
            side,
        });
    }

    /// Runs one `git/resolve` call once the user confirmed the side.
    fn run_git_resolve(&mut self, session_id: String, path: String, side: GitResolveSide) {
        let Some(transport) = self.transport.clone() else {
            self.reset_git();
            return;
        };
        if self.git_receiver.is_some() {
            return;
        }
        let request = GitResolveRequest { path, side };
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let result = GitClient::new(&transport).resolve(session_id, &request);
            let _ = sender.send(GitUpdate::Resolve(result));
        });
        self.git_receiver = Some(receiver);
        self.git_loading = true;
        self.git_error = None;
    }

    /// Opens the Stash panel from one status-bar menu trigger.
    ///
    /// @param trigger - Branch or sync control that keeps focus when the panel closes.
    pub(crate) fn open_git_stash(&mut self, trigger: ChromeTrigger) {
        self.git_menu = None;
        self.git_stash = Some(StashPanel {
            entries: Vec::new(),
            truncated: false,
            loading: true,
            error: None,
            summary: None,
            include_untracked: false,
            trigger,
        });
        self.refresh_git_stash();
    }

    /// Closes the Stash panel and returns focus to the menu that opened it.
    pub(crate) fn close_git_stash(&mut self) {
        if let Some(panel) = self.git_stash.take() {
            self.pending_chrome_focus = Some(panel.trigger);
        }
    }

    /// Loads the stash list into the open panel.
    pub(crate) fn refresh_git_stash(&mut self) {
        if self.git_stash.is_none() {
            return;
        }
        let (Some(session_id), Some(transport)) =
            (self.selected_session.clone(), self.transport.clone())
        else {
            if let Some(panel) = self.git_stash.as_mut() {
                panel.loading = false;
                panel.error = Some("Reading the stash list needs an online Host.".to_owned());
            }
            return;
        };
        if self.git_receiver.is_some() {
            self.git_stash_pending = true;
            return;
        }
        self.git_stash_pending = false;
        if let Some(panel) = self.git_stash.as_mut() {
            panel.loading = true;
            panel.error = None;
        }
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let result = GitClient::new(&transport).stash_list(session_id);
            let _ = sender.send(GitUpdate::Stash(result));
        });
        self.git_receiver = Some(receiver);
        self.git_loading = true;
    }

    /// Saves the work tree as one stash entry.
    ///
    /// @param message - Optional message; blank is sent as absent.
    /// @param include_untracked - Also stash untracked files, never ignored ones.
    pub(crate) fn save_git_stash(&mut self, message: String, include_untracked: bool) {
        if let Some(panel) = self.git_stash.as_mut() {
            panel.include_untracked = include_untracked;
        }
        self.run_git_stash(
            GitStashAction::Save,
            None,
            Some(message),
            include_untracked,
        );
    }

    /// Opens the confirmation for applying, popping, or dropping one stash entry.
    ///
    /// @param action - Apply, pop, or drop.
    /// @param index - Entry number the panel row shows.
    /// @param message - Entry message the confirmation names.
    pub(crate) fn request_stash_action(
        &mut self,
        action: StashEntryAction,
        index: u32,
        message: String,
    ) {
        let Some(session_id) = self.selected_session.clone() else {
            return;
        };
        if let Some(reason) = self.git_action_blocker() {
            if let Some(panel) = self.git_stash.as_mut() {
                panel.error = Some(reason);
            }
            return;
        }
        let listed = self
            .git_stash
            .as_ref()
            .is_some_and(|panel| panel.entries.iter().any(|entry| entry.index == index));
        if !listed {
            if let Some(panel) = self.git_stash.as_mut() {
                panel.error = Some(format!(
                    "{} cancelled: stash@{{{index}}} is no longer listed.",
                    action.label()
                ));
            }
            self.refresh_git_stash();
            return;
        }
        self.git_confirmation = Some(GitConfirmation::Stash {
            session_id,
            action,
            index,
            message,
        });
    }

    /// Runs one stash mutation after its confirmation.
    ///
    /// @param action - Wire action the Host runs.
    /// @param index - Entry number for apply, pop, and drop.
    /// @param message - Optional save message.
    /// @param include_untracked - Save only: also stash untracked files.
    fn run_git_stash(
        &mut self,
        action: GitStashAction,
        index: Option<u32>,
        message: Option<String>,
        include_untracked: bool,
    ) {
        let (Some(session_id), Some(transport)) =
            (self.selected_session.clone(), self.transport.clone())
        else {
            self.reset_git();
            return;
        };
        if self.git_receiver.is_some() {
            return;
        }
        if let Some(panel) = self.git_stash.as_mut() {
            panel.loading = true;
            panel.error = None;
        }
        let request = stash_request(action, index, message, include_untracked);
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let result = GitClient::new(&transport).stash(session_id, &request);
            let _ = sender.send(GitUpdate::Stash(result));
        });
        self.git_receiver = Some(receiver);
        self.git_loading = true;
    }

    /// Opens the stricter confirmation for deleting one untracked file.
    ///
    /// Directories and paths Git does not report as untracked are refused before
    /// a dialog opens, so the Host never receives a deletion they could match.
    ///
    /// @param path - Untracked row the Files tab names.
    pub(crate) fn request_discard_untracked(&mut self, path: String) {
        let Some(session_id) = self.selected_session.clone() else {
            return;
        };
        let Some(file) = self.git_file(&path) else {
            self.git_error = Some(format!(
                "Delete cancelled: \"{path}\" is no longer in the changed-file list."
            ));
            return;
        };
        if let Some(refusal) = untracked_delete_refusal(&file) {
            self.git_error = Some(refusal);
            return;
        }
        self.git_confirmation = Some(GitConfirmation::DiscardUntracked { session_id, path });
    }

    /// Runs one `git/discardUntracked` call once the user confirmed the path.
    fn run_git_discard_untracked(&mut self, session_id: String, path: String) {
        let Some(transport) = self.transport.clone() else {
            self.reset_git();
            return;
        };
        if self.git_receiver.is_some() {
            return;
        }
        let expected_status = Some(untracked_delete_expected_status());
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let result = GitClient::new(&transport).discard_untracked(
                session_id,
                path,
                expected_status,
            );
            let _ = sender.send(GitUpdate::DiscardUntracked(result));
        });
        self.git_receiver = Some(receiver);
        self.git_loading = true;
        self.git_error = None;
    }

    /// Picks up a stash message clear requested by a settled save.
    ///
    /// @returns Whether the Stash panel must clear its message field this frame.
    pub(crate) fn take_git_stash_clear(&mut self) -> bool {
        std::mem::take(&mut self.git_stash_message_clear)
    }

    /// Picks the path and side the Diff tab shows after a status refresh.
    ///
    /// Keeps the selected path while it is still changed and the shown side still
    /// has content; otherwise it selects the first work-tree change.
    fn ensure_git_selection(&mut self) {
        let files = self
            .git_status
            .as_ref()
            .map(|status| status.files.clone())
            .unwrap_or_default();
        if let Some(path) = self.git_selected_path.clone() {
            if let Some(file) = files.iter().find(|file| file.path == path) {
                let shown_side_has_content = if self.git_diff_staged {
                    file.staged
                } else {
                    file.worktree
                };
                if !shown_side_has_content {
                    self.git_diff_staged = file.staged;
                }
                return;
            }
        }
        match files
            .iter()
            .find(|file| file.worktree)
            .or_else(|| files.first())
        {
            Some(file) => {
                self.git_selected_path = Some(file.path.clone());
                self.git_diff_staged = !file.worktree;
            }
            None => {
                self.git_selected_path = None;
                self.git_diff = None;
            }
        }
    }

    /// Selects one changed path and loads its diff for the current side.
    pub(crate) fn select_git_path(&mut self, path: String) {
        self.git_selected_path = Some(path);
        self.request_git_diff();
    }

    /// Drops the loaded patch and schedules its read once the worker is free.
    ///
    /// The patch is dropped immediately so the pane never pairs one path's
    /// content with another path's header.
    fn request_git_diff(&mut self) {
        self.git_diff = None;
        self.git_focused_hunk = None;
        self.refresh_git_diff();
    }

    /// Records which shell text field owns the keyboard for the coming frame.
    ///
    /// @param window — window whose focus is read.
    /// @param cx — app context the input entities are read through.
    pub(crate) fn sync_typing_focus(&mut self, window: &Window, cx: &App) {
        let typing = self
            .inputs
            .iter()
            .find(|(_, input)| input.read(cx).is_focused(window))
            .map(|(key, _)| *key);
        self.typing_focus = typing;
    }

    /// Whether a Changes shortcut may run now.
    ///
    /// The column must be open, no overlay or Git confirmation may own the
    /// screen, and no text field may be typing.
    ///
    /// @returns whether the shortcuts act on the Changes surface.
    fn changes_shortcuts_available(&self) -> bool {
        self.show_changes
            && self.typing_focus.is_none()
            && self.git_confirmation.is_none()
            && self.git_stash.is_none()
            && self.git_menu.is_none()
            && !self.status_menu_open
            && !self.show_session_menu
            && !self.workspace_picker_open
            && !self.top_menu_open
            && !self.palette_open
            && !self.show_about
            && !self.show_settings
            && self.git_conflict_view.is_none()
            && self.update_prompt.is_none()
    }

    /// Runs one Changes shortcut, or nothing when its guards refuse it.
    ///
    /// @param shortcut — the action the pressed chord maps to.
    /// @returns whether the shell state changed.
    fn run_changes_shortcut(&mut self, shortcut: ChangesShortcut) -> bool {
        if !self.changes_shortcuts_available() {
            return false;
        }
        match shortcut {
            ChangesShortcut::HunkPrevious => self.move_git_hunk_focus(-1),
            ChangesShortcut::HunkNext => self.move_git_hunk_focus(1),
            ChangesShortcut::Stage => self.stage_from_keyboard(),
            ChangesShortcut::Unstage => self.unstage_from_keyboard(),
            ChangesShortcut::Discard => self.discard_from_keyboard(),
        }
    }

    /// Moves the Diff tab's hunk selection by `delta`.
    ///
    /// The first move selects the first hunk going down and the last going up, so
    /// the arrows work from a fresh diff.
    ///
    /// @param delta — hunks to move; negative selects an earlier hunk.
    /// @returns whether the selection moved.
    fn move_git_hunk_focus(&mut self, delta: isize) -> bool {
        if self.changes_tab != ChangesTab::Diff {
            return false;
        }
        let count = self.git_hunk_count();
        if count == 0 {
            return false;
        }
        let next = match self.git_focused_hunk {
            None if delta > 0 => 0,
            None => count - 1,
            Some(current) => {
                let moved = current as isize + delta;
                if moved < 0 || moved >= count as isize {
                    return false;
                }
                moved as usize
            }
        };
        if self.git_focused_hunk == Some(next) {
            return false;
        }
        self.git_focused_hunk = Some(next);
        true
    }

    /// Hunks in the loaded diff.
    fn git_hunk_count(&self) -> usize {
        self.git_diff
            .as_ref()
            .map(|diff| super::git_review::parse_diff(&diff.patch).hunks.len())
            .unwrap_or(0)
    }

    /// Path, one-hunk patch, and status signature of the focused hunk.
    ///
    /// @returns the values `git/apply` needs, or `None` when no hunk is selected.
    fn focused_git_hunk(&self) -> Option<(String, String, String)> {
        let path = self.git_selected_path.clone()?;
        let diff = self.git_diff.as_ref()?;
        let index = self.git_focused_hunk?;
        let patch = super::git_review::parse_diff(&diff.patch).hunk_patch(index)?;
        let status = self
            .git_file(&path)
            .map(|file| file.status.clone())
            .unwrap_or_default();
        Some((path, patch, status))
    }

    /// `s`: stages the focused hunk, the ticked set, or the selected file.
    fn stage_from_keyboard(&mut self) -> bool {
        match self.changes_tab {
            ChangesTab::Files => {
                if !self.git_selection.is_empty() {
                    self.stage_git_selection();
                    return true;
                }
                let Some(path) = self.git_selected_path.clone() else {
                    return false;
                };
                if !self.git_file(&path).is_some_and(|file| file.worktree) {
                    return false;
                }
                self.stage_git_path(path);
                true
            }
            ChangesTab::Diff => {
                if self.git_diff_staged {
                    return false;
                }
                let Some((path, patch, status)) = self.focused_git_hunk() else {
                    return false;
                };
                self.apply_git_hunk(path, patch, GitApplyTarget::Index, false, Some(status));
                true
            }
            ChangesTab::History => false,
        }
    }

    /// `u`: unstages the focused hunk, the ticked set, or the selected file.
    fn unstage_from_keyboard(&mut self) -> bool {
        match self.changes_tab {
            ChangesTab::Files => {
                if !self.git_selection.is_empty() {
                    self.unstage_git_selection();
                    return true;
                }
                let Some(path) = self.git_selected_path.clone() else {
                    return false;
                };
                if !self.git_file(&path).is_some_and(|file| file.staged) {
                    return false;
                }
                self.unstage_git_path(path);
                true
            }
            ChangesTab::Diff => {
                if !self.git_diff_staged {
                    return false;
                }
                let Some((path, patch, status)) = self.focused_git_hunk() else {
                    return false;
                };
                self.apply_git_hunk(path, patch, GitApplyTarget::Index, true, Some(status));
                true
            }
            ChangesTab::History => false,
        }
    }

    /// `d`: discards the focused hunk, the ticked set, or the selected file.
    ///
    /// Every path ends in the same confirmation the panel's buttons open.
    fn discard_from_keyboard(&mut self) -> bool {
        match self.changes_tab {
            ChangesTab::Files => {
                if !self.git_selection.is_empty() {
                    self.discard_git_selection();
                    return true;
                }
                let Some(path) = self.git_selected_path.clone() else {
                    return false;
                };
                let Some(file) = self.git_file(&path) else {
                    return false;
                };
                if !file.worktree || git_status_cannot_discard(&file.status) {
                    return false;
                }
                self.discard_git_path(path);
                true
            }
            ChangesTab::Diff => {
                if self.git_diff_staged {
                    return false;
                }
                let Some((path, patch, _)) = self.focused_git_hunk() else {
                    return false;
                };
                self.discard_git_hunk(path, patch);
                true
            }
            ChangesTab::History => false,
        }
    }

    /// `Ctrl+Enter` in the commit message field commits the staged set.
    ///
    /// The chord is the one Changes key that fires while a field is typing, so it
    /// checks the field it belongs to instead of refusing every focused input.
    ///
    /// @param cx — app context the commit message field is read through.
    /// @returns whether a commit request started.
    pub(crate) fn commit_from_keyboard(&mut self, cx: &App) -> bool {
        if self.typing_focus != Some("git-commit-message") {
            return false;
        }
        let Some(status) = self.git_status.clone() else {
            return false;
        };
        let message = self.input_text("git-commit-message", cx);
        let summary = super::git_review::staged_summary(&status.files);
        let in_operation = self
            .git_state
            .as_ref()
            .is_some_and(|state| state.operation != "none" || !state.conflicts.is_empty());
        if super::git_review::commit_blocker(
            summary.files,
            &message,
            self.git_loading,
            in_operation,
        )
        .is_some()
        {
            return false;
        }
        let amend = self.git_amend && !super::git_review::head_is_pushed(&status);
        let sign_off = self.git_sign_off;
        self.commit_git(message, amend, sign_off, false);
        true
    }

    /// Selects one path from the Files tab and reads the group's diff side.
    ///
    /// @param path — path to show in the Diff tab.
    /// @param staged — whether the group reads the index side.
    pub(crate) fn select_git_file(&mut self, path: String, staged: bool) {
        self.git_selected_path = Some(path);
        self.git_diff_staged = staged;
        self.request_git_diff();
    }

    /// Toggles one path in the Files-tab bulk selection.
    /// @param path — changed path to add or remove.
    pub(crate) fn toggle_git_selection(&mut self, path: String) {
        if let Some(index) = self.git_selection.iter().position(|item| item == &path) {
            self.git_selection.remove(index);
        } else {
            self.git_selection.push(path);
        }
    }

    /// Clears the Files-tab bulk selection.
    pub(crate) fn clear_git_selection(&mut self) {
        self.git_selection.clear();
    }

    /// Whether the Files-tab bulk selection contains one path.
    /// @param path — changed path to test.
    /// @returns whether the path is ticked.
    pub(crate) fn git_selection_contains(&self, path: &str) -> bool {
        self.git_selection.iter().any(|item| item == path)
    }

    /// The ticked paths, read from the latest status so actions use Host data.
    pub(crate) fn selected_git_files(&self) -> Vec<GitFile> {
        let Some(status) = self.git_status.as_ref() else {
            return Vec::new();
        };
        status
            .files
            .iter()
            .filter(|file| self.git_selection_contains(&file.path))
            .cloned()
            .collect()
    }

    /// Stages every ticked path that has a work-tree change.
    pub(crate) fn stage_git_selection(&mut self) {
        let targets = self
            .selected_git_files()
            .into_iter()
            .filter(|file| file.worktree)
            .map(|file| (file.path, None))
            .collect::<Vec<_>>();
        if targets.is_empty() {
            self.git_error = Some("No ticked path has an unstaged change to stage.".to_owned());
            return;
        }
        self.run_git_paths(GitPathVerb::Stage, targets);
    }

    /// Unstages every ticked path that has an index change.
    pub(crate) fn unstage_git_selection(&mut self) {
        let targets = self
            .selected_git_files()
            .into_iter()
            .filter(|file| file.staged)
            .map(|file| (file.path, None))
            .collect::<Vec<_>>();
        if targets.is_empty() {
            self.git_error = Some("No ticked path has a staged change to unstage.".to_owned());
            return;
        }
        self.run_git_paths(GitPathVerb::Unstage, targets);
    }

    /// Opens a confirmation before discarding every ticked work-tree change.
    pub(crate) fn discard_git_selection(&mut self) {
        let Some(session_id) = self.selected_session.clone() else {
            return;
        };
        let files = self
            .selected_git_files()
            .into_iter()
            .filter(|file| file.worktree && !git_status_cannot_discard(&file.status))
            .collect::<Vec<_>>();
        if files.is_empty() {
            self.git_error =
                Some("No ticked path has a discarding work-tree change.".to_owned());
            return;
        }
        self.git_confirmation = Some(GitConfirmation::DiscardMany { session_id, files });
    }

    /// Toggles Reviewed for the selected path at its current status signature.
    pub(crate) fn toggle_git_reviewed(&mut self) {
        let Some(path) = self.git_selected_path.clone() else {
            return;
        };
        let Some(file) = self.git_file(&path) else {
            return;
        };
        let signature = super::git_review::file_signature(&file);
        if self.git_reviewed.get(&path) == Some(&signature) {
            self.git_reviewed.remove(&path);
        } else {
            self.git_reviewed.insert(path, signature);
        }
    }

    /// Whether the selected path is marked reviewed at its current signature.
    /// @param path — changed path to test.
    /// @returns whether the stored signature matches the latest status entry.
    pub(crate) fn git_is_reviewed(&self, path: &str) -> bool {
        let Some(file) = self.git_file(path) else {
            return false;
        };
        self.git_reviewed.get(path) == Some(&super::git_review::file_signature(&file))
    }

    /// Marks every changed path reviewed at its current status signature.
    pub(crate) fn review_all_git_paths(&mut self) {
        let Some(status) = self.git_status.as_ref() else {
            return;
        };
        for file in &status.files {
            self.git_reviewed
                .insert(file.path.clone(), super::git_review::file_signature(file));
        }
    }

    /// One changed path from the latest status.
    /// @param path — changed path to find.
    /// @returns a copy of its status entry, or `None` when it is gone.
    pub(crate) fn git_file(&self, path: &str) -> Option<GitFile> {
        self.git_status
            .as_ref()
            .and_then(|status| status.files.iter().find(|file| file.path == path))
            .cloned()
    }

    /// Toggles the diff side for the selected path.
    pub(crate) fn set_git_diff_staged(&mut self, staged: bool) {
        self.git_diff_staged = staged;
        self.request_git_diff();
    }

    /// Loads the selected path's diff from the Host.
    pub(crate) fn refresh_git_diff(&mut self) {
        let (Some(session_id), Some(transport), Some(path)) = (
            self.selected_session.clone(),
            self.transport.clone(),
            self.git_selected_path.clone(),
        ) else {
            self.git_diff = None;
            self.git_diff_pending = false;
            return;
        };
        if self.git_receiver.is_some() {
            self.git_diff_pending = true;
            return;
        }
        self.git_diff_pending = false;
        self.git_diff = None;
        let staged = self.git_diff_staged;
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let result = GitClient::new(&transport).diff(session_id, path, staged);
            let _ = sender.send(GitUpdate::Diff(result));
        });
        self.git_receiver = Some(receiver);
        self.git_loading = true;
        self.git_error = None;
    }

    /// Stages one changed path.
    pub(crate) fn stage_git_path(&mut self, path: String) {
        self.run_git_paths(GitPathVerb::Stage, vec![(path, None)]);
    }

    /// Unstages one changed path.
    pub(crate) fn unstage_git_path(&mut self, path: String) {
        self.run_git_paths(GitPathVerb::Unstage, vec![(path, None)]);
    }

    /// Applies one hunk patch to the index or the work tree.
    ///
    /// @param path — path the patch belongs to.
    /// @param patch — file header plus one hunk, as `git/apply` expects it.
    /// @param target — index or work tree.
    /// @param reverse — apply the patch backwards.
    /// @param expected_status — status the diff was read at; the Host refuses another.
    pub(crate) fn apply_git_hunk(
        &mut self,
        path: String,
        patch: String,
        target: GitApplyTarget,
        reverse: bool,
        expected_status: Option<String>,
    ) {
        let (Some(session_id), Some(transport)) =
            (self.selected_session.clone(), self.transport.clone())
        else {
            self.reset_git();
            return;
        };
        if self.git_receiver.is_some() {
            return;
        }
        let request = GitApplyRequest {
            path,
            patch,
            target,
            reverse,
            expected_status,
        };
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let result = GitClient::new(&transport).apply(session_id, &request);
            let _ = sender.send(GitUpdate::Apply(result));
        });
        self.git_receiver = Some(receiver);
        self.git_loading = true;
        self.git_error = None;
    }

    /// Opens a confirmation before discarding one hunk through `git/apply`.
    ///
    /// @param path — path the hunk belongs to.
    /// @param patch — file header plus the hunk the user reviewed.
    pub(crate) fn discard_git_hunk(&mut self, path: String, patch: String) {
        let Some(session_id) = self.selected_session.clone() else {
            return;
        };
        let Some(file) = self.git_file(&path) else {
            self.git_error =
                Some("Discard cancelled: the file list changed before confirmation.".to_owned());
            return;
        };
        if !file.worktree || git_status_cannot_discard(&file.status) {
            self.git_error =
                Some("Discard unavailable for this file's current Git state.".to_owned());
            return;
        }
        let signature = super::git_review::patch_signature(&path, false, &patch);
        self.git_confirmation = Some(GitConfirmation::DiscardHunk {
            session_id,
            path,
            status: file.status,
            staged: file.staged,
            worktree: file.worktree,
            patch,
            signature,
        });
    }

    /// Opens a confirmation before discarding one path's unstaged change.
    pub(crate) fn discard_git_path(&mut self, path: String) {
        let Some(session_id) = self.selected_session.clone() else {
            return;
        };
        let Some(file) = self
            .git_status
            .as_ref()
            .and_then(|status| status.files.iter().find(|file| file.path == path))
            .cloned()
        else {
            self.git_error =
                Some("Discard cancelled: the file list changed before confirmation.".to_owned());
            return;
        };
        if !file.worktree || git_status_cannot_discard(&file.status) {
            self.git_error =
                Some("Discard unavailable for this file's current Git state.".to_owned());
            return;
        }
        self.git_confirmation = Some(GitConfirmation::Discard {
            session_id,
            path,
            status: file.status.clone(),
            staged: file.staged,
            worktree: file.worktree,
        });
    }

    /// Confirms a pending discard after rechecking the last Git status.
    pub(crate) fn confirm_git_confirmation(&mut self) {
        let Some(confirmation) = self.git_confirmation.take() else {
            return;
        };
        match confirmation {
            GitConfirmation::Discard {
                session_id,
                path,
                status,
                staged,
                worktree,
            } => {
                let fresh = discard_confirmation_is_fresh(
                    self.git_status.as_ref(),
                    &session_id,
                    self.selected_session.as_deref(),
                    &path,
                    &status,
                    staged,
                    worktree,
                );
                if !fresh {
                    self.git_error = Some(
                        "Discard cancelled: the file changed after confirmation opened.".to_owned(),
                    );
                    self.refresh_git();
                    return;
                }
                self.run_git_paths(GitPathVerb::Discard, vec![(path, Some(status))]);
            }
            GitConfirmation::DiscardMany { session_id, files } => {
                let fresh = discard_many_is_fresh(
                    self.git_status.as_ref(),
                    &session_id,
                    self.selected_session.as_deref(),
                    &files,
                );
                if !fresh {
                    self.git_error = Some(
                        "Discard cancelled: a ticked file changed after confirmation opened."
                            .to_owned(),
                    );
                    self.refresh_git();
                    return;
                }
                let targets = files
                    .into_iter()
                    .map(|file| (file.path, Some(file.status)))
                    .collect();
                self.run_git_paths(GitPathVerb::Discard, targets);
            }
            GitConfirmation::DiscardHunk {
                session_id,
                path,
                status,
                staged,
                worktree,
                patch,
                signature,
            } => {
                let fresh = hunk_confirmation_is_fresh(
                    self.git_status.as_ref(),
                    self.git_diff.as_ref(),
                    &session_id,
                    self.selected_session.as_deref(),
                    &path,
                    &status,
                    staged,
                    worktree,
                    &signature,
                );
                if !fresh {
                    self.git_error = Some(
                        "Discard cancelled: the diff changed after confirmation opened."
                            .to_owned(),
                    );
                    self.refresh_git();
                    return;
                }
                self.apply_git_hunk(
                    path,
                    patch,
                    GitApplyTarget::Worktree,
                    true,
                    Some(status),
                );
            }
            GitConfirmation::Checkout {
                session_id,
                current,
                target,
                head,
                changed_files,
            } => {
                let fresh = checkout_confirmation_is_fresh(
                    self.git_status.as_ref(),
                    self.git_branches.as_ref(),
                    &session_id,
                    self.selected_session.as_deref(),
                    &current,
                    &target,
                    &head,
                    &changed_files,
                );
                if !fresh {
                    self.git_error = Some(
                        "Checkout cancelled: the repository changed after confirmation opened."
                            .to_owned(),
                    );
                    self.refresh_git();
                    self.refresh_git_branches();
                    return;
                }
                self.checkout_git_branch_after_confirmation(target);
            }
            GitConfirmation::CommitEntry {
                session_id,
                commit,
                action,
                ..
            } => {
                if self.selected_session.as_deref() != Some(session_id.as_str()) {
                    self.git_error = Some(
                        "The confirmed commit belongs to another Session; nothing was run."
                            .to_owned(),
                    );
                    return;
                }
                self.run_git_commit_entry(action, session_id, commit);
            }
            GitConfirmation::Push {
                session_id,
                set_upstream,
            } => {
                if self.selected_session.as_deref() != Some(session_id.as_str()) {
                    self.git_error = Some(
                        "The confirmed push belongs to another Session; nothing was run."
                            .to_owned(),
                    );
                    return;
                }
                if let Some(reason) = self.git_action_blocker() {
                    self.git_error = Some(reason);
                    return;
                }
                self.run_git_remote(if set_upstream {
                    GitRemoteAction::PushSetUpstream
                } else {
                    GitRemoteAction::Push
                });
            }
            GitConfirmation::BranchEntry {
                session_id,
                branch,
                commit,
                action,
            } => {
                if self.selected_session.as_deref() != Some(session_id.as_str()) {
                    self.git_error = Some(
                        "The confirmed branch action belongs to another Session; nothing was run."
                            .to_owned(),
                    );
                    return;
                }
                if let Some(reason) = self.git_action_blocker() {
                    self.git_error = Some(reason);
                    return;
                }
                match action {
                    GitBranchAction::Merge => self.run_git_operation(move |client, session_id| {
                        client.merge(session_id, branch)
                    }),
                    GitBranchAction::Rebase => self.run_git_operation(move |client, session_id| {
                        client.rebase(session_id, branch)
                    }),
                    GitBranchAction::CherryPick => self.run_git_operation(move |client, session_id| {
                        client.cherry_pick(session_id, commit)
                    }),
                    GitBranchAction::Revert => self.run_git_operation(move |client, session_id| {
                        client.revert(session_id, commit)
                    }),
                }
            }
            GitConfirmation::ResolveConflict {
                session_id,
                path,
                side,
            } => {
                if self.selected_session.as_deref() != Some(session_id.as_str()) {
                    self.git_error = Some(
                        "The confirmed conflict side belongs to another Session; nothing was run."
                            .to_owned(),
                    );
                    return;
                }
                if let Some(reason) = self.git_action_blocker() {
                    self.git_error = Some(reason);
                    return;
                }
                let listed = self
                    .git_state
                    .as_ref()
                    .is_some_and(|state| state.conflicts.iter().any(|item| item == &path));
                if !listed {
                    self.git_error = Some(format!(
                        "Resolve cancelled: \"{path}\" is no longer an unmerged path."
                    ));
                    self.refresh_git_state();
                    return;
                }
                self.run_git_resolve(session_id, path, side);
            }
            GitConfirmation::Stash {
                session_id,
                action,
                index,
                message,
            } => {
                if self.selected_session.as_deref() != Some(session_id.as_str()) {
                    if let Some(panel) = self.git_stash.as_mut() {
                        panel.error = Some(
                            "The confirmed stash entry belongs to another Session; nothing was run."
                                .to_owned(),
                        );
                    }
                    return;
                }
                if let Some(reason) = self.git_action_blocker() {
                    if let Some(panel) = self.git_stash.as_mut() {
                        panel.error = Some(reason);
                    }
                    return;
                }
                self.run_git_stash(action.request(), Some(index), Some(message), false);
            }
            GitConfirmation::DiscardUntracked { session_id, path } => {
                if self.selected_session.as_deref() != Some(session_id.as_str()) {
                    self.git_error = Some(
                        "The confirmed deletion belongs to another Session; nothing was run."
                            .to_owned(),
                    );
                    return;
                }
                let fresh = self
                    .git_status
                    .as_ref()
                    .and_then(|status| status.files.iter().find(|file| file.path == path))
                    .is_some_and(|file| untracked_delete_refusal(file).is_none());
                if !fresh {
                    self.git_error = Some(
                        "Delete cancelled: the file changed after confirmation opened.".to_owned(),
                    );
                    self.refresh_git();
                    return;
                }
                self.run_git_discard_untracked(session_id, path);
            }
        }
    }

    /// Runs one path verb over the given paths, stopping at the first failure.
    ///
    /// The Host applies one verb per call. Keeping the calls in one worker means
    /// the panel can never report a partially applied batch as confirmed.
    ///
    /// @param verb — index or work-tree verb to run.
    /// @param targets — path and the status the caller displayed, when the verb uses one.
    fn run_git_paths(&mut self, verb: GitPathVerb, targets: Vec<(String, Option<String>)>) {
        let (Some(session_id), Some(transport)) =
            (self.selected_session.clone(), self.transport.clone())
        else {
            self.reset_git();
            return;
        };
        if self.git_receiver.is_some() || targets.is_empty() {
            return;
        }
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let client = GitClient::new(&transport);
            let mut paths = Vec::new();
            let mut failure = None;
            for (path, expected_status) in targets {
                let result = match verb {
                    GitPathVerb::Stage => client.stage(session_id.clone(), path.clone()),
                    GitPathVerb::Unstage => client.unstage(session_id.clone(), path.clone()),
                    GitPathVerb::Discard => {
                        client.discard(session_id.clone(), path.clone(), expected_status)
                    }
                };
                match result {
                    Ok(result) => paths.push(result.path),
                    Err(error) => {
                        failure = Some(error);
                        break;
                    }
                }
            }
            let _ = sender.send(GitUpdate::Paths {
                operation: verb.operation(),
                paths,
                failure,
            });
        });
        self.git_receiver = Some(receiver);
        self.git_loading = true;
        self.git_error = None;
    }

    /// Commits the staged index with the commit bar's options.
    ///
    /// @param message — commit message; a blank message is refused before a worker starts.
    /// @param amend — replace HEAD instead of adding a commit.
    /// @param sign_off — append the committer's `Signed-off-by` trailer.
    /// @param push — push the new commit once it lands.
    pub(crate) fn commit_git(&mut self, message: String, amend: bool, sign_off: bool, push: bool) {
        let (Some(session_id), Some(transport)) =
            (self.selected_session.clone(), self.transport.clone())
        else {
            self.reset_git();
            return;
        };
        if message.trim().is_empty() || self.git_receiver.is_some() {
            return;
        }
        let options = GitCommitOptions { amend, sign_off };
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let result =
                GitClient::new(&transport).commit_with_options(session_id, message, &options);
            let _ = sender.send(GitUpdate::Commit(result));
        });
        self.git_receiver = Some(receiver);
        self.git_loading = true;
        self.git_push_pending = push;
        self.git_error = None;
    }

    /// Whether a settled commit must clear the draft on the next frame.
    /// @returns `true` once per committed message.
    pub(crate) fn take_git_commit_clear(&mut self) -> bool {
        std::mem::take(&mut self.git_commit_input_clear)
    }

    /// Loads the selected Session's recent commit subjects for the commit bar.
    fn refresh_git_log(&mut self) {
        let (Some(session_id), Some(transport)) =
            (self.selected_session.clone(), self.transport.clone())
        else {
            return;
        };
        if self.git_receiver.is_some() {
            self.git_log_pending = true;
            return;
        }
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let result = GitClient::new(&transport).log(session_id, Some(5), None);
            let _ = sender.send(GitUpdate::Log(result));
        });
        self.git_receiver = Some(receiver);
        self.git_log_loading = true;
        self.git_log_error = None;
    }

    /// Reads the first History page while the History tab renders.
    ///
    /// The loaded flag keeps a settled page from being read again on every
    /// frame; opening the tab or a moved HEAD asks for a page explicitly.
    pub(crate) fn ensure_git_history(&mut self) {
        if self.git_history_loaded
            || self.git_history_loading
            || self.git_history_pending.is_some()
            || self.git_history_error.is_some()
        {
            return;
        }
        if self.selected_session.is_none() || self.transport.is_none() {
            return;
        }
        self.refresh_git_history();
    }

    /// Loads the newest History page for the selected Session.
    pub(crate) fn refresh_git_history(&mut self) {
        self.request_git_history(0);
    }

    /// Loads the page the History footer points at.
    /// @param skip — commits to skip from HEAD; the footer reports the loaded count.
    pub(crate) fn load_git_history_page(&mut self, skip: u32) {
        self.request_git_history(skip);
    }

    /// Reads one History page once the repository worker is free.
    ///
    /// A page that cannot start now is remembered as the pending page, so the
    /// repository never runs two Git requests at once.
    /// @param skip — commits to skip from HEAD; `0` reloads the newest page.
    fn request_git_history(&mut self, skip: u32) {
        let (Some(session_id), Some(transport)) =
            (self.selected_session.clone(), self.transport.clone())
        else {
            self.reset_git();
            return;
        };
        if self.git_receiver.is_some() {
            self.git_history_pending = Some(skip);
            return;
        }
        self.git_history_pending = None;
        self.git_history_loading = true;
        self.git_history_error = None;
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let result = GitClient::new(&transport).log(
                session_id,
                Some(super::git_review::HISTORY_PAGE_LIMIT),
                (skip > 0).then_some(skip),
            );
            let _ = sender.send(GitUpdate::History { skip, result });
        });
        self.git_receiver = Some(receiver);
        self.git_loading = true;
    }

    /// Applies one settled `git/log` page to the History tab.
    /// @param skip — commits the page skipped from HEAD.
    /// @param log — commits in newest-first order plus the Host byte-cap flag.
    fn apply_git_history_page(&mut self, skip: u32, log: GitLog) {
        let received = log.commits.len();
        let truncated = log.truncated;
        if skip == 0 {
            self.git_history = log.commits;
            self.git_history_head = self
                .git_history
                .first()
                .map(|commit| commit.oid.clone());
        } else {
            self.git_history.extend(log.commits);
        }
        self.git_history_loaded = true;
        self.git_history_footer = super::git_review::history_footer(
            received,
            self.git_history.len(),
            super::git_review::HISTORY_PAGE_LIMIT,
            truncated,
        );
        let keep = self
            .git_history_selected
            .as_deref()
            .is_some_and(|oid| self.git_history.iter().any(|commit| commit.oid == oid));
        if !keep {
            self.git_history_selected = None;
            self.git_history_show = None;
            self.git_history_show_error = None;
            self.git_history_show_loading = false;
            self.git_history_show_pending = None;
        }
    }

    /// Selects one History commit and reads its patch.
    /// @param oid — full object id of a commit in the loaded page.
    pub(crate) fn select_git_history(&mut self, oid: String) {
        if self.git_history_selected.as_deref() == Some(oid.as_str()) {
            if self.git_history_show.is_none() && !self.git_history_show_loading {
                self.git_history_show_error = None;
                self.request_git_history_show();
            }
            return;
        }
        self.git_history_selected = Some(oid);
        self.git_history_show = None;
        self.git_history_show_error = None;
        self.request_git_history_show();
    }

    /// Moves the History selection by one row, clamped to the loaded page.
    /// @param delta — rows to move; a negative value moves towards newer commits.
    pub(crate) fn move_git_history_selection(&mut self, delta: i8) {
        let current = self
            .git_history_selected
            .as_deref()
            .and_then(|oid| self.git_history.iter().position(|commit| commit.oid == oid));
        let Some(next) = super::git_review::stepped_history_selection(
            current,
            delta,
            self.git_history.len(),
        ) else {
            return;
        };
        let oid = self.git_history[next].oid.clone();
        self.git_history_scroll.scroll_to_item(next);
        self.select_git_history(oid);
    }

    /// Reads the selected commit's patch once the repository worker is free.
    fn request_git_history_show(&mut self) {
        let (Some(session_id), Some(transport), Some(oid)) = (
            self.selected_session.clone(),
            self.transport.clone(),
            self.git_history_selected.clone(),
        ) else {
            return;
        };
        if self
            .git_history_show
            .as_ref()
            .is_some_and(|show| show.commit == oid)
        {
            return;
        }
        if self.git_receiver.is_some() {
            self.git_history_show_pending = Some(oid);
            return;
        }
        self.git_history_show_pending = None;
        self.git_history_show_loading = true;
        self.git_history_show_error = None;
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let result = GitClient::new(&transport).show(session_id, oid.clone(), None);
            let _ = sender.send(GitUpdate::Show { oid, result });
        });
        self.git_receiver = Some(receiver);
        self.git_loading = true;
    }

    /// Opens the confirmation for reverting or cherry-picking the selected commit.
    /// @param action — revert or cherry-pick.
    pub(crate) fn confirm_git_history_entry(&mut self, action: GitCommitAction) {
        let (Some(session_id), Some(oid)) =
            (self.selected_session.clone(), self.git_history_selected.clone())
        else {
            return;
        };
        let Some(commit) = self
            .git_history
            .iter()
            .find(|commit| commit.oid == oid)
        else {
            return;
        };
        self.git_confirmation = Some(GitConfirmation::CommitEntry {
            session_id,
            commit: oid,
            subject: commit.subject.clone(),
            action,
        });
    }

    /// Runs one confirmed revert or cherry-pick through the Host.
    /// @param action — verb to run.
    /// @param session_id — Session the confirmation named.
    /// @param commit — full object id the user confirmed.
    fn run_git_commit_entry(
        &mut self,
        action: GitCommitAction,
        session_id: String,
        commit: String,
    ) {
        let Some(transport) = self.transport.clone() else {
            return;
        };
        if self.git_receiver.is_some() {
            return;
        }
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let client = GitClient::new(&transport);
            let result = match action {
                GitCommitAction::Revert => client.revert(session_id, commit.clone()),
                GitCommitAction::CherryPick => client.cherry_pick(session_id, commit.clone()),
            };
            let _ = sender.send(GitUpdate::CommitEntry {
                action,
                commit,
                result,
            });
        });
        self.git_receiver = Some(receiver);
        self.git_loading = true;
        self.git_error = None;
    }

    /// Opens a confirmation before checking out one local branch.
    pub(crate) fn checkout_git_branch(&mut self, branch: String) {
        let Some(session_id) = self.selected_session.clone() else {
            return;
        };
        let Some(status) = self.git_status.clone() else {
            self.git_error =
                Some("Checkout unavailable until the repository status is refreshed.".to_owned());
            return;
        };
        if status.branch.as_deref() == Some(branch.as_str()) {
            return;
        }
        let listed = self
            .git_branches
            .as_ref()
            .is_some_and(|branches| branches.branches.iter().any(|item| item.name == branch));
        if !listed {
            self.git_error = Some(format!(
                "Checkout unavailable: \"{branch}\" is no longer in the branch list."
            ));
            self.refresh_git_branches();
            return;
        }
        self.git_confirmation = Some(GitConfirmation::Checkout {
            session_id,
            current: status.branch.clone(),
            target: branch,
            head: status.commit.clone(),
            changed_files: status.files.clone(),
        });
    }

    /// Checks out one local branch and reloads the repository state after it lands.
    fn checkout_git_branch_after_confirmation(&mut self, branch: String) {
        let (Some(session_id), Some(transport)) =
            (self.selected_session.clone(), self.transport.clone())
        else {
            self.reset_git();
            return;
        };
        if self.git_receiver.is_some() {
            return;
        }
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let result = GitClient::new(&transport).checkout(session_id, branch);
            let _ = sender.send(GitUpdate::Checkout(result));
        });
        self.git_receiver = Some(receiver);
        self.git_loading = true;
        self.git_error = None;
    }

    /// Applies one settled Git worker result; returns whether the frame changed.
    fn poll_git(&mut self) -> bool {
        let Some(receiver) = &self.git_receiver else {
            return false;
        };
        match receiver.try_recv() {
            Ok(update) => {
                self.git_receiver = None;
                self.git_loading = false;
                self.git_remote_pending = None;
                match update {
                    GitUpdate::Status(Ok(status)) => {
                        let files = status.files.clone();
                        let head = status.commit.clone();
                        self.git_selection
                            .retain(|path| files.iter().any(|file| &file.path == path));
                        self.git_reviewed
                            .retain(|path, _| files.iter().any(|file| &file.path == path));
                        self.git_status = Some(status);
                        self.git_error = None;
                        self.ensure_git_selection();
                        if self.git_selected_path.is_some() && self.git_diff.is_none() {
                            self.git_diff_pending = true;
                        }
                        if self.git_recent_messages.is_empty() && self.git_log_error.is_none() {
                            self.git_log_pending = true;
                        }
                        if self.show_changes
                            && self.changes_tab == ChangesTab::History
                            && self.git_history_loaded
                            && head.is_some()
                            && head != self.git_history_head
                        {
                            self.refresh_git_history();
                        }
                    }
                    GitUpdate::Status(Err(error)) => {
                        self.git_status = None;
                        self.git_error = Some(failure_text(&error));
                    }
                    GitUpdate::Branches(Ok(branches)) => {
                        self.git_branches = Some(branches);
                        self.git_error = None;
                    }
                    GitUpdate::Branches(Err(error)) => {
                        self.git_branches = None;
                        self.git_error = Some(failure_text(&error));
                    }
                    GitUpdate::Checkout(Ok(result)) => {
                        self.git_branches = None;
                        self.git_error = None;
                        self.status = format!("Checked out {}", result.branch);
                        self.refresh_git();
                        self.refresh_git_state();
                    }
                    GitUpdate::Checkout(Err(error)) => {
                        self.git_branches = None;
                        self.git_error = Some(failure_text(&error));
                    }
                    GitUpdate::Diff(Ok(diff)) => {
                        self.git_diff = Some(diff);
                        self.git_error = None;
                    }
                    GitUpdate::Diff(Err(error)) => {
                        self.git_diff = None;
                        self.git_error = Some(failure_text(&error));
                    }
                    GitUpdate::Paths {
                        operation,
                        paths,
                        failure,
                    } => {
                        self.git_diff = None;
                        match failure {
                            Some(error) => {
                                self.git_error = Some(failure_text(&error));
                            }
                            None => {
                                if operation == "discard" {
                                    self.git_selection.clear();
                                }
                                self.status = match paths.as_slice() {
                                    [] => format!("{operation}: nothing to change"),
                                    [path] => format!("{operation}: {path}"),
                                    paths => format!("{operation}: {} paths", paths.len()),
                                };
                            }
                        }
                        self.refresh_git();
                    }
                    GitUpdate::Apply(Ok(result)) => {
                        self.git_diff = None;
                        self.status = format!("{}: {}", result.operation, result.path);
                        self.refresh_git();
                    }
                    GitUpdate::Apply(Err(error)) => {
                        // A refused patch leaves both sides untouched; reload what is on disk.
                        self.git_error = Some(failure_text(&error));
                        self.git_diff = None;
                        self.refresh_git();
                    }
                    GitUpdate::Commit(Ok(result)) => {
                        self.git_diff = None;
                        self.git_selected_path = None;
                        self.git_reviewed.clear();
                        self.git_recent_messages.clear();
                        self.git_log_error = None;
                        self.git_amend = false;
                        self.git_commit_input_clear = true;
                        self.status = format!("Committed {}", short_object_id(&result.commit));
                        self.refresh_git();
                        self.refresh_git_state();
                    }
                    GitUpdate::Commit(Err(error)) => {
                        self.git_push_pending = false;
                        self.git_error = Some(failure_text(&error));
                    }
                    GitUpdate::Log(Ok(log)) => {
                        self.git_log_loading = false;
                        self.git_recent_messages = log
                            .commits
                            .into_iter()
                            .map(|commit| commit.subject)
                            .collect();
                        self.git_log_error = None;
                    }
                    GitUpdate::Log(Err(error)) => {
                        self.git_log_loading = false;
                        self.git_log_error = Some(failure_text(&error));
                    }
                    GitUpdate::History {
                        skip,
                        result: Ok(log),
                    } => {
                        self.git_history_loading = false;
                        self.git_history_error = None;
                        self.apply_git_history_page(skip, log);
                    }
                    GitUpdate::History {
                        result: Err(error),
                        ..
                    } => {
                        self.git_history_loading = false;
                        self.git_history_error = Some(failure_text(&error));
                    }
                    GitUpdate::Show { oid, result } => {
                        if self.git_history_selected.as_deref() == Some(oid.as_str()) {
                            self.git_history_show_loading = false;
                            match result {
                                Ok(show) => {
                                    self.git_history_show = Some(show);
                                    self.git_history_show_error = None;
                                }
                                Err(error) => {
                                    self.git_history_show = None;
                                    self.git_history_show_error = Some(failure_text(&error));
                                }
                            }
                        }
                    }
                    GitUpdate::CommitEntry {
                        action,
                        commit,
                        result: Ok(result),
                    } => {
                        self.git_state = Some(result.state);
                        if !result.output.is_empty() {
                            self.git_remote_summary = Some(result.output);
                        }
                        self.status = format!(
                            "{} {}",
                            match action {
                                GitCommitAction::Revert => "Reverted",
                                GitCommitAction::CherryPick => "Cherry-picked",
                            },
                            short_object_id(&commit)
                        );
                        self.refresh_git();
                        self.refresh_git_state();
                    }
                    GitUpdate::CommitEntry {
                        result: Err(error),
                        ..
                    } => {
                        self.git_error = Some(failure_text(&error));
                    }
                    GitUpdate::State(Ok(state)) => {
                        self.git_state = Some(state);
                    }
                    GitUpdate::StatusAndState(Ok(status), Ok(state)) => {
                        self.git_status = Some(status);
                        self.git_state = Some(state);
                        self.git_error = None;
                    }
                    GitUpdate::StatusAndState(Ok(status), Err(error)) => {
                        self.git_status = Some(status);
                        self.git_state = None;
                        self.git_error = Some(failure_text(&error));
                    }
                    GitUpdate::StatusAndState(Err(error), _) => {
                        self.git_status = None;
                        self.git_error = Some(failure_text(&error));
                    }
                    GitUpdate::State(Err(error)) => {
                        self.git_state = None;
                        self.git_error = Some(failure_text(&error));
                    }
                    GitUpdate::Remote(operation, Ok(result)) => {
                        self.git_remote_summary = Some(if result.summary.is_empty() {
                            format!("{} complete", result.operation)
                        } else {
                            result.summary.clone()
                        });
                        self.status = format!("{operation} complete");
                        self.refresh_git();
                        self.refresh_git_state();
                    }
                    GitUpdate::Remote(_, Err(error)) => {
                        self.git_error = Some(failure_text(&error));
                    }
                    GitUpdate::Operation(Ok(result)) => {
                        self.git_state = Some(result.state);
                        if !result.output.is_empty() {
                            self.git_remote_summary = Some(result.output);
                        }
                        self.refresh_git();
                    }
                    GitUpdate::Operation(Err(error)) => {
                        self.git_error = Some(failure_text(&error));
                    }
                    GitUpdate::Conflict { path, result } => {
                        if let Some(view) = self.git_conflict_view.as_mut() {
                            if view.path == path {
                                view.loading = false;
                                match result {
                                    Ok(conflict) => {
                                        view.conflict = Some(conflict);
                                        view.error = None;
                                    }
                                    Err(error) => {
                                        view.conflict = None;
                                        view.error = Some(failure_text(&error));
                                    }
                                }
                            }
                        }
                    }
                    GitUpdate::Resolve(Ok(result)) => {
                        if self
                            .git_conflict_view
                            .as_ref()
                            .is_some_and(|view| view.path == result.path)
                        {
                            self.git_conflict_view = None;
                        }
                        if let Some(state) = self.git_state.as_mut() {
                            state.conflicts.retain(|path| path != &result.path);
                        }
                        self.git_error = None;
                        self.status = format!(
                            "Accepted {} for {}",
                            resolve_side_label(result.side),
                            result.path
                        );
                        self.refresh_git();
                        self.refresh_git_state();
                    }
                    GitUpdate::Resolve(Err(error)) => {
                        let text = failure_text(&error);
                        self.git_error = Some(text.clone());
                        if let Some(view) = self.git_conflict_view.as_mut() {
                            view.loading = false;
                            view.error = Some(text);
                        }
                        self.refresh_git_state();
                    }
                    GitUpdate::Stash(Ok(stash)) => {
                        let action = stash.action;
                        if let Some(panel) = self.git_stash.as_mut() {
                            panel.loading = false;
                            panel.error = None;
                            if action == GitStashAction::List {
                                panel.entries = stash.entries;
                                panel.truncated = stash.truncated;
                            } else if !stash.summary.is_empty() {
                                panel.summary = Some(stash.summary);
                            }
                        }
                        let name = match action {
                            GitStashAction::List => "list",
                            GitStashAction::Save => "save",
                            GitStashAction::Apply => "apply",
                            GitStashAction::Pop => "pop",
                            GitStashAction::Drop => "drop",
                        };
                        if action != GitStashAction::List {
                            self.status = format!("stash {name} complete");
                        }
                        if action == GitStashAction::Save {
                            self.git_stash_message_clear = true;
                        }
                        if matches!(
                            action,
                            GitStashAction::Save | GitStashAction::Apply | GitStashAction::Pop
                        ) {
                            self.refresh_git();
                        }
                        if self.git_stash.is_some() {
                            self.refresh_git_stash();
                        }
                    }
                    GitUpdate::Stash(Err(error)) => {
                        let text = failure_text(&error);
                        if let Some(panel) = self.git_stash.as_mut() {
                            panel.loading = false;
                            panel.error = Some(text);
                        } else {
                            self.git_error = Some(text);
                        }
                    }
                    GitUpdate::DiscardUntracked(result) => {
                        match result {
                            Ok(result) => {
                                self.git_selection.retain(|item| item != &result.path);
                                self.git_reviewed.remove(&result.path);
                                if self.git_selected_path.as_deref() == Some(result.path.as_str()) {
                                    self.git_selected_path = None;
                                    self.git_diff = None;
                                }
                                self.status = format!("Deleted {}", result.path);
                                self.refresh_git();
                            }
                            Err(error) => {
                                let text = failure_text(&error);
                                // Reloading status clears `git_error`, so the refusal is written after it.
                                self.refresh_git();
                                self.git_error = Some(text);
                            }
                        }
                    }
                }
                if self.git_state_pending {
                    self.git_state_pending = false;
                    self.refresh_git_state();
                }
                if self.git_push_pending && self.git_receiver.is_none() {
                    self.git_push_pending = false;
                    self.run_git_remote(GitRemoteAction::Push);
                }
                if self.git_diff_pending && self.git_receiver.is_none() {
                    self.refresh_git_diff();
                }
                if self.git_log_pending && self.git_receiver.is_none() {
                    self.git_log_pending = false;
                    self.refresh_git_log();
                }
                if let Some(skip) = self.git_history_pending.take() {
                    if self.git_receiver.is_none() {
                        self.request_git_history(skip);
                    } else {
                        self.git_history_pending = Some(skip);
                    }
                }
                if self.git_history_show_pending.is_some() && self.git_receiver.is_none() {
                    self.request_git_history_show();
                }
                if let Some(path) = self.git_conflict_pending.take() {
                    if self.git_receiver.is_none() {
                        if let (Some(session_id), Some(transport)) =
                            (self.selected_session.clone(), self.transport.clone())
                        {
                            self.request_git_conflict(session_id, transport, path);
                        } else if let Some(view) = self.git_conflict_view.as_mut() {
                            view.loading = false;
                            view.error =
                                Some("Reading conflict stages needs an online Host.".to_owned());
                        }
                    } else {
                        self.git_conflict_pending = Some(path);
                    }
                }
                if self.git_stash_pending && self.git_receiver.is_none() {
                    self.git_stash_pending = false;
                    self.refresh_git_stash();
                }
                true
            }
            Err(mpsc::TryRecvError::Empty) => false,
            Err(mpsc::TryRecvError::Disconnected) => {
                self.git_receiver = None;
                self.git_loading = false;
                self.git_state_pending = false;
                self.git_log_loading = false;
                self.git_log_pending = false;
                self.git_history_loading = false;
                self.git_history_show_loading = false;
                self.git_history_pending = None;
                self.git_history_show_pending = None;
                self.git_push_pending = false;
                self.git_conflict_pending = None;
                self.git_stash_pending = false;
                if let Some(view) = self.git_conflict_view.as_mut() {
                    view.loading = false;
                    view.error = Some("Git worker stopped unexpectedly".to_owned());
                }
                if let Some(panel) = self.git_stash.as_mut() {
                    panel.loading = false;
                    panel.error = Some("Git worker stopped unexpectedly".to_owned());
                }
                self.git_error = Some("Git worker stopped unexpectedly".to_owned());
                true
            }
        }
    }

    /// Refreshes the Session list for the live run badge without status noise.
    fn refresh_sessions_quietly(&mut self) {
        if self.sessions_receiver.is_some() {
            return;
        }
        let status = self.sessions_status.clone();
        self.refresh_sessions();
        self.sessions_status = status;
    }

    /// Polls the canonical run state while a turn may still be active.
    fn poll_run_state(&mut self) {
        if !self.chat_loading && !self.sessions.iter().any(|session| session.running) {
            return;
        }
        if self.last_session_poll.elapsed() < std::time::Duration::from_secs(3) {
            return;
        }
        self.last_session_poll = std::time::Instant::now();
        self.refresh_sessions_quietly();
    }

    /// Refreshes Git once a workspace watch reported a change.
    ///
    /// Status and operation state are one combined request so the panel cannot
    /// show a diff/edit from one repository generation next to state from
    /// another.
    fn poll_git_auto_refresh(&mut self) {
        if !self.git_refresh_pending || self.selected_session.is_none() {
            return;
        }
        if self.git_receiver.is_some() {
            return;
        }
        let (Some(session_id), Some(transport)) =
            (self.selected_session.clone(), self.transport.clone())
        else {
            return;
        };
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let client = GitClient::new(&transport);
            let status = client.status(session_id.clone());
            let state = client.state(session_id);
            let _ = sender.send(GitUpdate::StatusAndState(status, state));
        });
        self.git_receiver = Some(receiver);
        self.git_loading = true;
        self.git_error = None;
        self.git_refresh_pending = false;
        if self.git_selected_path.is_some() {
            let _ = self.git_diff.take();
        }
    }

    pub(crate) fn load_session(&mut self, session_id: String) {
        self.load_session_with_policy(session_id, false);
    }

    /// Adds one Session to the Agent tab strip, keeping the active buffer.
    ///
    /// Opening a buffer that is already present only makes it active through
    /// the caller's Session switch; the existing order is preserved.
    /// @param session_id - Session to show as an Agent buffer.
    pub(crate) fn open_agent_tab(&mut self, session_id: &str) {
        if self.agent_tabs.iter().any(|id| id == session_id) {
            return;
        }
        self.agent_tabs.push(session_id.to_owned());
        while self.agent_tabs.len() > MAX_AGENT_TABS {
            let active = self.selected_session.clone();
            let Some(index) = self
                .agent_tabs
                .iter()
                .position(|id| Some(id) != active.as_ref())
            else {
                break;
            };
            self.agent_tabs.remove(index);
        }
    }

    /// Closes one Agent buffer. Closing the active buffer activates its right
    /// neighbour, then its left one, or clears the conversation when none
    /// remain.
    /// @param session_id - Buffer to close.
    pub(crate) fn close_agent_tab(&mut self, session_id: &str) {
        let Some(index) = self.agent_tabs.iter().position(|id| id == session_id) else {
            return;
        };
        self.agent_tabs.remove(index);
        if self.selected_session.as_deref() != Some(session_id) {
            return;
        }
        let next = self
            .agent_tabs
            .get(index)
            .or_else(|| index.checked_sub(1).and_then(|left| self.agent_tabs.get(left)))
            .cloned();
        match next {
            Some(next) => self.load_session(next),
            None => {
                self.selected_session = None;
                self.session_title.clear();
                self.chat_text.clear();
                self.transcript.reset();
            }
        }
    }

    /// The buffer tab's label: the live title for the active buffer, otherwise
    /// the Session list's projected label or a short identity.
    /// @param session_id - Buffer whose label is requested.
    /// @returns Display text for the tab.
    pub(crate) fn agent_tab_label(&self, session_id: &str) -> String {
        if self.selected_session.as_deref() == Some(session_id) && !self.session_title.is_empty() {
            return self.session_title.clone();
        }
        self.sessions
            .iter()
            .find(|summary| summary.session_id == session_id)
            .map(session_list::session_label)
            .unwrap_or_else(|| session_list::truncate(session_id, 18))
    }

    /// Workspaces the top bar's picker offers, most recently used first.
    ///
    /// Each entry stands for one working directory the Session list reports and
    /// carries its newest Session, because picking a workspace switches the
    /// selected Session. Directories stay separate even when they nest.
    ///
    /// @returns One entry per distinct Session working directory.
    pub(crate) fn session_workspaces(&self) -> Vec<SessionWorkspace> {
        let active_cwd = self
            .selected_session
            .as_ref()
            .and_then(|id| {
                self.sessions
                    .iter()
                    .find(|summary| &summary.session_id == id)
            })
            .and_then(|summary| summary.cwd.clone());
        let mut entries: Vec<(i64, SessionWorkspace)> = Vec::new();
        for summary in &self.sessions {
            let Some(cwd) = summary
                .cwd
                .as_deref()
                .map(str::trim)
                .filter(|cwd| !cwd.is_empty())
            else {
                continue;
            };
            match entries.iter_mut().find(|(_, entry)| entry.cwd == cwd) {
                Some((updated_at, entry)) => {
                    entry.session_count += 1;
                    if summary.updated_at > *updated_at {
                        *updated_at = summary.updated_at;
                        entry.session_id = summary.session_id.clone();
                    }
                }
                None => entries.push((
                    summary.updated_at,
                    SessionWorkspace {
                        cwd: cwd.to_owned(),
                        session_id: summary.session_id.clone(),
                        session_count: 1,
                        active: active_cwd.as_deref() == Some(cwd),
                    },
                )),
            }
        }
        entries.sort_by(|left, right| {
            right
                .0
                .cmp(&left.0)
                .then_with(|| left.1.cwd.cmp(&right.1.cwd))
        });
        entries.into_iter().map(|(_, entry)| entry).collect()
    }

    /// Switches to the newest Session of one workspace.
    ///
    /// @param cwd - Working directory the picker row stands for.
    pub(crate) fn select_session_workspace(&mut self, cwd: &str) {
        let session_id = self
            .session_workspaces()
            .into_iter()
            .find(|entry| entry.cwd == cwd)
            .map(|entry| entry.session_id);
        if let Some(session_id) = session_id {
            self.load_session(session_id);
        }
    }

    fn reload_session_after_reconnect(&mut self, session_id: String) {
        self.load_session_with_policy(session_id, true);
    }

    fn load_session_with_policy(&mut self, session_id: String, retain_loaded_view: bool) {
        let retain_loaded_view =
            retain_loaded_view && self.selected_session.as_deref() == Some(session_id.as_str());
        if !retain_loaded_view {
            if let Some(previous) = self.selected_session.clone() {
                if previous != session_id {
                    self.session_transcripts.retain(|(id, _)| id != &previous);
                    self.session_transcripts.push_back((previous, self.transcript.clone()));
                    while self.session_transcripts.len() > 6 {
                        self.session_transcripts.pop_front();
                    }
                }
            }
            self.remember_active_draft();
        }
        if !retain_loaded_view {
            self.session_title.clear();
            self.active_view = ActiveView::Sessions;
            self.show_settings = false;
            self.show_plugins = false;
            self.show_skills = false;
            self.show_market = false;
            self.show_goals = false;
            self.show_document = false;
            self.skills = None;
            self.skills_receiver = None;
            self.skills_loading = false;
            self.skills_error = None;
            self.skills_query.clear();
            self.selected_skill = None;
        }
        self.selected_session = Some(session_id.clone());
        self.open_agent_tab(&session_id);
        self.ui_preferences.last_session_id = Some(session_id.clone());
        self.mark_session_state_dirty();
        if !retain_loaded_view {
            self.restore_active_draft(&session_id);
        }
        self.flush_session_state();
        self.explorer
            .select_session(Some(&session_id), self.transport.as_ref());
        if retain_loaded_view {
            self.terminal.select_session(None, None, None);
            self.terminal_extra.clear();
            self.terminal_pane_widths.clear();
            self.terminal_focus = 0;
        }
        self.goal = None;
        self.goal_receiver = None;
        self.goal_mutation_receiver = None;
        self.goal_loading = false;
        self.goal_mutation_loading = false;
        self.goal_error = None;
        self.goal_status = None;
        self.clear_goal_confirmation = None;
        self.terminal.select_session(
            Some(session_id.clone()),
            self.transport.clone(),
            self.streams.clone(),
        );
        self.jobs.clear();
        self.jobs_session = None;
        self.jobs_error = None;
        self.job_selected = None;
        self.job_output.clear();
        self.job_status.clear();
        self.job_receiver = None;
        self.schedules.clear();
        self.schedules_session = None;
        self.schedules_error = None;
        self.schedule_delete_receiver = None;
        self.start_job_list();
        self.start_schedules();
        self.page_has_more = false;
        self.page_receiver = None;
        self.page_through_seq = None;
        self.page_error = None;
        self.page_before_seq = None;
        self.chat_receiver = None;
        self.chat_loading = false;
        self.assistant_streaming = false;
        self.assistant_draft.clear();
        if !retain_loaded_view {
            self.transcript.reset();
            if let Some(index) = self
                .session_transcripts
                .iter()
                .position(|(id, _)| id == &session_id)
            {
                self.transcript = self
                    .session_transcripts
                    .remove(index)
                    .map(|(_, transcript)| transcript)
                    .unwrap_or_default();
            }
            self.scroll_transcript_to_bottom = true;
            self.deliverables.clear();
            self.feedback_items.clear();
            self.preview_document = None;
            self.preview_image = None;
            self.preview_target_line = None;
            self.preview_error = None;
        }
        self.refresh_feedback();
        self.reset_git_history();
        self.refresh_git();
        self.refresh_git_state();
        self.page_loading = true;
        let Some(transport) = self.transport.clone() else {
            self.page_loading = false;
            self.page_error = Some("Connect to load this session".to_owned());
            return;
        };
        if retain_loaded_view {
            self.explorer.refresh_root(&transport);
        }
        let Some(streams) = self.streams.clone() else {
            self.page_loading = false;
            self.page_error = Some("Remote streams are not connected".to_owned());
            return;
        };
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let result = (|| {
                let mut stream = follow::follow(
                    &streams,
                    FollowRequest {
                        address: SessionAddress {
                            kind: SessionAddressKind::Session,
                            session_id: session_id.clone(),
                        },
                        max_messages: Some(1),
                        turn_window: None,
                        assistant_stream: false,
                    },
                )?;
                let frame = stream.next().ok_or(Error::InvalidEnvelope(
                    "session follow ended before snapshot",
                ))??;
                let FollowFrame::Snapshot(snapshot) = frame else {
                    return Err(Error::InvalidEnvelope(
                        "session follow did not start with snapshot",
                    ));
                };
                let through_seq = snapshot.cursor;
                // The snapshot is the newest backlog; the page holds older
                // history. Both project into the same transcript.
                let live = snapshot.records;
                drop(stream);
                let page = SessionViewer::new(&transport).page(&session_id, through_seq)?;
                Ok((through_seq, page, live))
            })();
            let _ = sender.send(result);
        });
        self.page_receiver = Some(receiver);
        if self.show_goals {
            self.refresh_goal();
        }
    }

    pub(crate) fn load_older_session_page(&mut self) {
        if self.page_loading {
            return;
        }
        let Some(session_id) = self.selected_session.clone() else {
            self.page_error = Some("Select a Session before loading older events".to_owned());
            return;
        };
        let Some(before_seq) = self.transcript.oldest_seq() else {
            self.page_error = Some("No earlier session events are available".to_owned());
            return;
        };
        let Some(through_seq) = self.page_through_seq else {
            self.page_error = Some("Session history cursor is unavailable".to_owned());
            return;
        };
        let Some(transport) = self.transport.clone() else {
            self.page_error = Some("Connect before loading older events".to_owned());
            return;
        };
        let request = SessionPageRequest {
            address: SessionAddress {
                kind: SessionAddressKind::Session,
                session_id,
            },
            through_seq,
            before_seq: Some(before_seq),
            max_messages: Some(200),
            turn_window: Some(SessionTurnWindow {
                min_messages: 100,
                min_turns: 2,
            }),
        };
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let result = SessionViewer::new(&transport)
                .page_request(request)
                .map(|page| (through_seq, page, Vec::new()));
            let _ = sender.send(result);
        });
        self.page_receiver = Some(receiver);
        self.page_loading = true;
        self.page_before_seq = Some(before_seq);
        self.page_error = None;
    }

    pub(crate) fn refresh_goal(&mut self) {
        if self.goal_loading || self.goal_mutation_loading {
            return;
        }
        let Some(session_id) = self.selected_session.clone() else {
            self.goal_error = Some("Select a Session to view its goal".to_owned());
            return;
        };
        let Some(transport) = self.transport.clone() else {
            self.goal_error = Some("Connect before loading the goal".to_owned());
            return;
        };
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(GoalsClient::new(&transport).get(&session_id));
        });
        self.goal_receiver = Some(receiver);
        self.goal_loading = true;
        self.goal_error = None;
    }

    pub(crate) fn mutate_goal(&mut self, mutation: GoalMutation) {
        if self.goal_mutation_loading || self.goal_loading {
            return;
        }
        let Some(session_id) = self.selected_session.clone() else {
            self.goal_error = Some("Select a Session before changing its goal".to_owned());
            return;
        };
        let Some(transport) = self.transport.clone() else {
            self.goal_error = Some("Connect before changing the goal".to_owned());
            return;
        };
        let status = match &mutation {
            GoalMutation::Create(_) => "Creating goal…",
            GoalMutation::Edit(_, _) => "Saving goal…",
            GoalMutation::Pause(_) => "Pausing goal…",
            GoalMutation::Resume(_) => "Resuming goal…",
            GoalMutation::Complete(_) => "Completing goal…",
            GoalMutation::Clear(_) => "Clearing goal…",
        }
        .to_owned();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let client = GoalsClient::new(&transport);
            let result = match mutation {
                GoalMutation::Create(request) => {
                    client.create(&session_id, request).and_then(|created| {
                        let goal = client
                            .get(&session_id)?
                            .ok_or(Error::InvalidEnvelope("created goal is not readable"))?;
                        if goal.id != created.r#ref.id || goal.revision != created.r#ref.revision {
                            return Err(Error::InvalidEnvelope("created goal identity changed"));
                        }
                        Ok(Some(goal))
                    })
                }
                GoalMutation::Edit(reference, request) => {
                    client.edit(&session_id, &reference, request).map(Some)
                }
                GoalMutation::Pause(reference) => client.pause(&session_id, &reference).map(Some),
                GoalMutation::Resume(reference) => client.resume(&session_id, &reference).map(Some),
                GoalMutation::Complete(reference) => {
                    client.complete(&session_id, &reference).map(Some)
                }
                GoalMutation::Clear(reference) => {
                    client.clear(&session_id, &reference).map(|_| None)
                }
            };
            let _ = sender.send(result);
        });
        self.goal_mutation_receiver = Some(receiver);
        self.goal_mutation_loading = true;
        self.goal_status = Some(status);
        self.goal_error = None;
    }

    pub(crate) fn run_plan_command(&mut self, active: bool) {
        if self.plan_command_loading || self.chat_loading || self.assistant_streaming {
            return;
        }
        let Some(agent_id) = self.selected_session.clone() else {
            self.plan_command_status =
                Some("Select a Session before changing plan mode".to_owned());
            return;
        };
        let Some(transport) = self.transport.clone() else {
            self.plan_command_status = Some("Connect before changing plan mode".to_owned());
            return;
        };
        let line = if active { "/plan" } else { "/plan off" };
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let result = CommandsClient::new(&transport).execute(agent_id.as_str(), line, vec![]);
            let _ = sender.send(result);
        });
        self.plan_command_receiver = Some(receiver);
        self.plan_command_loading = true;
        self.plan_command_status = Some(if active {
            "Entering plan mode…".to_owned()
        } else {
            "Leaving plan mode…".to_owned()
        });
    }


    pub(crate) fn pick_attachment(&mut self) {
        if self.transport.is_none() {
            self.attachment_error = Some("Connect before attaching a file".to_owned());
            return;
        }
        if self.selected_session.is_none() {
            self.attachment_error = Some("Select a Session before attaching a file".to_owned());
            return;
        }
        self.dialog = Some((DialogPurpose::Attach, dialog::open_file_async()));
        self.attachment_error = None;
        self.status = "Choose a file to attach…".to_owned();
    }

    /// Attaches the first file dropped onto the window.
    ///
    /// The composer holds one attachment, so a multi-file drop names its limit
    /// instead of silently keeping only one of the dropped paths.
    pub(crate) fn attach_dropped_paths(&mut self, paths: &[PathBuf]) {
        let Some(path) = paths.first() else {
            return;
        };
        if self.attachment.is_some() || self.attachment_receiver.is_some() {
            self.attachment_error =
                Some("Detach the current file before dropping another.".to_owned());
            return;
        }
        if paths.len() > 1 {
            self.attachment_error =
                Some(format!("Attached {}; the composer holds one file at a time.", path.display()));
        }
        self.attach_path(path.clone());
    }

    /// Uploads the file the picker returned; the picker itself never runs here.
    fn attach_path(&mut self, path: PathBuf) {
        let Some(transport) = self.transport.clone() else {
            self.attachment_error = Some("Connect before attaching a file".to_owned());
            return;
        };
        let Some(session_id) = self.selected_session.clone() else {
            self.attachment_error = Some("Select a Session before attaching a file".to_owned());
            return;
        };
        let name = path
            .file_name()
            .map_or_else(String::new, |name| name.to_string_lossy().into_owned());
        let (sender, receiver) = mpsc::channel();
        self.attachment_receiver = Some(receiver);
        self.attachment_error = None;
        self.attachment = None;
        self.attachment_name = name.clone();
        std::thread::spawn(move || {
            let result = std::fs::read(&path)
                .map_err(|error| format!("reading {name} failed: {error}"))
                .and_then(|bytes| {
                    FileUploadClient::new(&transport)
                        .upload(&session_id, Some(&name), bytes)
                        .map_err(|error| error.to_string())
                });
            let _ = sender.send(result);
        });
    }

    /// Applies a picked path to the document preview, which reads Host paths.
    fn open_preview_path(&mut self, path: &Path) {
        self.preview_path = path.display().to_string();
        if self.selected_session.is_some() && self.transport.is_some() {
            self.read_document();
        } else {
            self.preview_error =
                Some("Select a connected Session before reading a file.".to_owned());
        }
    }

    fn poll_dialog(&mut self, window: &Window) {
        let Some((purpose, receiver)) = &self.dialog else {
            return;
        };
        let purpose = *purpose;
        match receiver.try_recv() {
            Ok(path) => {
                self.dialog = None;
                match (purpose, path) {
                    (DialogPurpose::Attach, Some(path)) => self.attach_path(path),
                    (DialogPurpose::Open, Some(path)) => self.open_preview_path(&path),
                    (DialogPurpose::Folder, Some(path)) => self.open_folder_path(path),
                    (DialogPurpose::ThemeAdd, Some(path)) => {
                        self.import_theme_file(path, window)
                    }
                    (DialogPurpose::ThemeExport, Some(path)) => self.export_theme_to(&path),
                    (_, None) => self.status = "File picker cancelled".to_owned(),
                }
            }
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => {
                self.dialog = None;
                self.status = "File picker stopped unexpectedly".to_owned();
            }
        }
    }

    fn poll_attachment(&mut self) {
        let Some(receiver) = &self.attachment_receiver else {
            return;
        };
        match receiver.try_recv() {
            Ok(Ok(value)) => {
                self.attachment = Some(value);
                self.attachment_receiver = None;
            }
            Ok(Err(error)) => {
                self.attachment_receiver = None;
                self.attachment_error = Some(format!("Upload failed: {error}"));
                self.attachment_name.clear();
            }
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => {
                self.attachment_receiver = None;
                self.attachment_error = Some("Upload worker stopped unexpectedly".to_owned());
            }
        }
    }

    pub(crate) fn start_schedules(&mut self) {
        let (Some(session), Some(transport)) =
            (self.selected_session.clone(), self.transport.clone())
        else {
            return;
        };
        if self.schedules_session.as_deref() == Some(session.as_str()) {
            return;
        }
        self.schedules_session = Some(session.clone());
        let (sender, receiver) = mpsc::channel();
        self.schedules_receiver = Some(receiver);
        self.schedules_error = None;
        std::thread::spawn(move || {
            let request = ScheduleListRequest {
                session_id: session,
            };
            let result = SchedulesClient::new(&transport)
                .list(request)
                .map_err(|error| error.to_string());
            let _ = sender.send(result);
        });
    }

    pub(crate) fn delete_schedule(&mut self, id: String) {
        let (Some(session), Some(transport)) =
            (self.selected_session.clone(), self.transport.clone())
        else {
            return;
        };
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let request = ScheduleDeleteRequest {
                session_id: session,
                id,
            };
            let result = SchedulesClient::new(&transport)
                .delete(request)
                .map(|_| ())
                .map_err(|error| error.to_string());
            let _ = sender.send(result);
        });
        self.schedule_delete_receiver = Some(receiver);
    }

    fn poll_schedules(&mut self) {
        loop {
            let update = self
                .schedules_receiver
                .as_ref()
                .map(|receiver| receiver.try_recv());
            match update {
                Some(Ok(Ok(records))) => self.schedules = records,
                Some(Ok(Err(error))) => {
                    self.schedules_error = Some(format!("Schedule list failed: {error}"));
                    self.schedules_receiver = None;
                }
                Some(Err(mpsc::TryRecvError::Disconnected)) => {
                    self.schedules_receiver = None;
                }
                Some(Err(mpsc::TryRecvError::Empty)) | None => break,
            }
        }
        loop {
            let update = self
                .schedule_delete_receiver
                .as_ref()
                .map(|receiver| receiver.try_recv());
            match update {
                Some(Ok(Ok(()))) => {
                    self.schedule_delete_receiver = None;
                    self.schedules_session = None;
                    self.start_schedules();
                    self.status = "Schedule deleted".to_owned();
                }
                Some(Ok(Err(error))) => {
                    self.schedules_error = Some(format!("Delete failed: {error}"));
                    self.schedule_delete_receiver = None;
                }
                Some(Err(mpsc::TryRecvError::Disconnected)) => {
                    self.schedule_delete_receiver = None;
                }
                Some(Err(mpsc::TryRecvError::Empty)) | None => break,
            }
        }
    }

    /// Requests diagnostics for the selected Session's workspace from the
    /// configured language servers. Servers that have not run yet publish
    /// nothing, so an empty snapshot is a normal result.
    pub(crate) fn refresh_diagnostics(&mut self) {
        let (Some(session), Some(transport)) = (self.selected_session.clone(), self.transport.clone())
        else {
            return;
        };
        if self.diagnostics_receiver.is_some() {
            return;
        }
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(DiagnosticsClient::new(&transport).list(session));
        });
        self.diagnostics_receiver = Some(receiver);
        self.diagnostics_loading = true;
        self.diagnostics_error = None;
    }

    /// Reads the diagnostics worker's result into shell state.
    /// @returns whether the shell should repaint.
    fn poll_diagnostics(&mut self) -> bool {
        let Some(receiver) = &self.diagnostics_receiver else {
            return false;
        };
        match receiver.try_recv() {
            Ok(Ok(response)) => {
                self.diagnostics = response.items;
                self.diagnostics_workspace = Some(response.workspace_root);
                self.diagnostics_loading = false;
                self.diagnostics_receiver = None;
                true
            }
            Ok(Err(error)) => {
                self.diagnostics_error = Some(error.to_string());
                self.diagnostics_loading = false;
                self.diagnostics_receiver = None;
                true
            }
            Err(mpsc::TryRecvError::Empty) => true,
            Err(mpsc::TryRecvError::Disconnected) => {
                self.diagnostics_error = Some("Diagnostics request ended unexpectedly".to_owned());
                self.diagnostics_loading = false;
                self.diagnostics_receiver = None;
                true
            }
        }
    }

    /// Opens one file in its configured language server so the Diagnostics
    /// panel has data without waiting for an agent LSP query.
    /// @param path - workspace-relative or absolute file to diagnose.
    pub(crate) fn diagnose_file(&mut self, path: String) {
        let (Some(session), Some(transport)) = (self.selected_session.clone(), self.transport.clone())
        else {
            return;
        };
        // A newer document supersedes an older diagnose result.
        self.diagnose_receiver = None;
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(DiagnosticsClient::new(&transport).diagnose(session, path));
        });
        self.diagnose_receiver = Some(receiver);
    }

    /// Merges one on-demand diagnose result into the diagnostics snapshot.
    /// @returns whether the shell should repaint.
    fn poll_diagnose(&mut self) -> bool {
        let Some(receiver) = &self.diagnose_receiver else {
            return false;
        };
        match receiver.try_recv() {
            Ok(Ok(response)) => {
                merge_diagnostics(&mut self.diagnostics, &response.uri, response.items);
                self.diagnostics_workspace = Some(response.workspace_root);
                self.diagnostics_error = None;
                self.diagnose_receiver = None;
                true
            }
            Ok(Err(error)) => {
                self.diagnostics_error = Some(error.to_string());
                self.diagnose_receiver = None;
                true
            }
            Err(mpsc::TryRecvError::Empty) => true,
            Err(mpsc::TryRecvError::Disconnected) => {
                self.diagnose_receiver = None;
                true
            }
        }
    }

    pub(crate) fn start_job_list(&mut self) {
        let (Some(session), Some(mux)) = (self.selected_session.clone(), self.streams.clone())
        else {
            return;
        };
        if self.jobs_session.as_deref() == Some(session.as_str()) {
            return;
        }
        self.jobs_session = Some(session.clone());
        let (sender, receiver) = mpsc::channel();
        self.jobs_receiver = Some(receiver);
        self.jobs_error = None;
        std::thread::spawn(move || match streams::job_list(&mux, &session) {
            Ok(stream) => {
                for frame in stream {
                    let update = match frame {
                        Ok(rows) => Ok(rows.jobs),
                        Err(error) => Err(error.to_string()),
                    };
                    let failed = update.is_err();
                    if sender.send(update).is_err() || failed {
                        break;
                    }
                }
            }
            Err(error) => {
                let _ = sender.send(Err(error.to_string()));
            }
        });
    }

    pub(crate) fn start_job_follow(&mut self, job_id: String) {
        let Some(mux) = self.streams.clone() else {
            return;
        };
        self.job_selected = Some(job_id.clone());
        self.job_output.clear();
        self.job_status = "Opening job…".to_owned();
        let (sender, receiver) = mpsc::channel();
        self.job_receiver = Some(receiver);
        std::thread::spawn(move || {
            let request = streams::JobFollowRequest {
                session_id: None,
                job_id,
                from: None,
            };
            match streams::job_follow(&mux, request) {
                Ok(stream) => {
                    for frame in stream {
                        let update = frame.map_err(|error| error.to_string());
                        let failed = update.is_err();
                        if sender.send(update).is_err() || failed {
                            break;
                        }
                    }
                }
                Err(error) => {
                    let _ = sender.send(Err(error.to_string()));
                }
            }
        });
    }

    /// Opens the Workspace projection stream; baseline replaces local state.
    pub(crate) fn start_workspace_follow(&mut self) {
        if self.workspace_receiver.is_some() {
            return;
        }
        let Some(mux) = self.streams.clone() else {
            self.workspace_error = Some("Remote streams are not connected".to_owned());
            return;
        };
        let (sender, receiver) = mpsc::channel();
        self.workspace_receiver = Some(receiver);
        std::thread::spawn(move || {
            match streams::workspace_follow(&mux) {
                Ok(stream) => {
                    for frame in stream {
                        let update = frame.map_err(|error| error.to_string());
                        let failed = update.is_err();
                        if sender.send(update).is_err() || failed {
                            break;
                        }
                    }
                }
                Err(error) => {
                    let _ = sender.send(Err(error.to_string()));
                }
            }
        });
    }

    fn apply_workspace_frame(&mut self, frame: WorkspaceFollowFrame) {
        apply_workspace_frame(
            &mut self.workspaces,
            &mut self.selected_workspace,
            &mut self.archived_session_ids,
            &mut self.pinned_session_ids,
            frame,
        );
        self.workspace_error = None;
    }

    fn poll_jobs(&mut self) {
        loop {
            let update = self
                .jobs_receiver
                .as_ref()
                .map(|receiver| receiver.try_recv());
            match update {
                Some(Ok(Ok(rows))) => self.jobs = rows,
                Some(Ok(Err(error))) => {
                    self.jobs_error = Some(format!("Job list failed: {error}"));
                    self.jobs_receiver = None;
                }
                Some(Err(mpsc::TryRecvError::Disconnected)) => {
                    self.jobs_receiver = None;
                }
                Some(Err(mpsc::TryRecvError::Empty)) | None => break,
            }
        }
        loop {
            let update = self
                .job_receiver
                .as_ref()
                .map(|receiver| receiver.try_recv());
            match update {
                Some(Ok(Ok(frame))) => match frame {
                    JobFollowFrame::Opened { job, from } => {
                        self.job_status = format!("{} · from byte {from}", job.status);
                    }
                    JobFollowFrame::Output { chunks, next, .. } => {
                        for chunk in chunks {
                            append_bounded_output(&mut self.job_output, &chunk.text);
                        }
                        self.job_status = format!("output to byte {next}");
                    }
                    JobFollowFrame::Status { job } => {
                        self.job_status = job.status;
                    }
                },
                Some(Ok(Err(error))) => {
                    self.job_status = format!("Job output failed: {error}");
                    self.job_receiver = None;
                }
                Some(Err(mpsc::TryRecvError::Disconnected)) => {
                    self.job_receiver = None;
                }
                Some(Err(mpsc::TryRecvError::Empty)) | None => break,
            }
        }
    }

    pub(crate) fn cancel_turn(&mut self) {
        if self.cancel_receiver.is_some() {
            return;
        }
        let (Some(session), Some(transport)) =
            (self.selected_session.clone(), self.transport.clone())
        else {
            self.page_error = Some("Select a connected Session before cancelling".to_owned());
            return;
        };
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(ChatClient::new(&transport).cancel(session));
        });
        self.cancel_receiver = Some(receiver);
    }

    pub(crate) fn send_prompt(&mut self) {
        let Some(session_id) = self.selected_session.clone() else {
            return;
        };
        let text = self.chat_text.trim().to_owned();
        if text.is_empty() || self.chat_loading {
            return;
        }
        let Some(transport) = self.transport.clone() else {
            self.page_error = Some("Connect before sending a prompt".to_owned());
            return;
        };
        if text.starts_with('/') {
            self.submit_slash_command(text, transport);
            return;
        }
        let Some(streams) = self.streams.clone() else {
            self.page_error = Some("Remote streams are not connected".to_owned());
            return;
        };
        let mode = self.prompt_mode;
        let attachment = self.attachment.clone();
        self.chat_text.clear();
        self.attachment = None;
        self.attachment_name.clear();
        self.chat_loading = true;
        self.remember_active_draft();
        self.record_prompt_history(&text);
        self.refresh_sessions_quietly();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let chat = ChatClient::new(&transport);
            let mut content = vec![PromptContentPart::Text { text }];
            if let Some(attachment) = attachment {
                content.push(PromptContentPart::File {
                    receipt_id: attachment.receipt_id,
                });
            }
            let request = PromptRequest {
                request_id: chat.next_request_id(),
                session_id: session_id.clone(),
                mode,
                content,
                client_time_zone: None,
            };
            if let Err(error) = chat.prompt(request) {
                let _ = sender.send(Err(error));
                return;
            }
            let request = FollowRequest {
                address: SessionAddress {
                    kind: SessionAddressKind::Session,
                    session_id,
                },
                max_messages: Some(200),
                turn_window: None,
                assistant_stream: true,
            };
            match follow::follow(&streams, request) {
                Ok(stream) => {
                    for frame in stream {
                        if sender.send(frame).is_err() {
                            break;
                        }
                    }
                }
                Err(error) => {
                    let _ = sender.send(Err(error));
                }
            }
        });
        self.chat_receiver = Some(receiver);
    }

    /// Records one sent prompt for later recall.
    pub(crate) fn record_prompt_history(&mut self, text: &str) {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return;
        }
        if self.prompt_history.last().map(String::as_str) != Some(trimmed) {
            self.prompt_history.push(trimmed.to_owned());
        }
        if self.prompt_history.len() > 20 {
            let excess = self.prompt_history.len() - 20;
            self.prompt_history.drain(0..excess);
        }
        self.prompt_history_index = None;
    }

    /// Recalls a sent prompt into the composer.
    pub(crate) fn recall_prompt_history(&mut self, direction: i8) {
        if self.prompt_history.is_empty() {
            return;
        }
        let index = match self.prompt_history_index {
            None => {
                if direction > 0 {
                    self.prompt_history.len() - 1
                } else {
                    return;
                }
            }
            Some(index) => {
                if direction > 0 {
                    index.saturating_sub(1)
                } else {
                    (index + 1).min(self.prompt_history.len() - 1)
                }
            }
        };
        self.prompt_history_index = Some(index);
        if let Some(text) = self.prompt_history.get(index).cloned() {
            self.chat_text = text.clone();
            self.pending_composer_text = Some(text);
        }
    }

    /// Inserts a workspace file reference into the composer.
    pub(crate) fn insert_file_reference(&mut self, path: &str) {
        if !self.chat_text.ends_with(' ') && !self.chat_text.is_empty() {
            self.chat_text.push(' ');
        }
        self.chat_text.push_str(&format!("@{path} "));
        self.pending_composer_text = Some(self.chat_text.clone());
        self.file_picker_query.clear();
        self.file_picker_open = false;
        self.focus_composer = true;
        self.remember_active_draft();
    }

    /// Sends a prompt while a turn is already running, without opening a second follow stream.
    pub(crate) fn queue_prompt(&mut self) {
        let text = self.chat_text.trim().to_owned();
        if text.is_empty() || !self.chat_loading {
            return;
        }
        let Some(session_id) = self.selected_session.clone() else {
            return;
        };
        let Some(transport) = self.transport.clone() else {
            self.page_error = Some("Connect before sending a prompt".to_owned());
            return;
        };
        let mode = self.prompt_mode;
        let attachment = self.attachment.clone();
        self.chat_text.clear();
        self.attachment = None;
        self.attachment_name.clear();
        self.remember_active_draft();
        self.record_prompt_history(&text);
        std::thread::spawn(move || {
            let chat = ChatClient::new(&transport);
            let mut content = vec![PromptContentPart::Text { text }];
            if let Some(attachment) = attachment {
                content.push(PromptContentPart::File {
                    receipt_id: attachment.receipt_id,
                });
            }
            let request = PromptRequest {
                request_id: chat.next_request_id(),
                session_id,
                mode,
                content,
                client_time_zone: None,
            };
            let _ = chat.prompt(request);
        });
    }

    pub(crate) fn switch_view(&mut self, view: ActiveView) {
        self.show_session_menu = false;
        self.show_about = false;
        if view == ActiveView::Settings {
            self.open_settings();
            return;
        }
        self.close_preferences();
        self.active_view = view;
        self.explorer_fullscreen = view == ActiveView::Explorer;
        self.show_settings = false;
        self.show_document = view == ActiveView::Document;
        self.show_plugins = view == ActiveView::Plugins;
        self.show_skills = view == ActiveView::Skills;
        self.show_market = view == ActiveView::Marketplace;
        self.show_goals = view == ActiveView::Goals;
        match view {
            ActiveView::Settings => self.open_settings(),
            ActiveView::Plugins if self.plugins_snapshot.is_none() => self.refresh_plugins(),
            ActiveView::Skills if self.skills.is_none() => self.refresh_skills(),
            ActiveView::Marketplace if self.market_indexes.is_empty() => {
                self.refresh_market_indexes();
            }
            ActiveView::Goals => self.refresh_goal(),
            _ => {}
        }
    }

    /// Opens a Session chosen in the list.
    pub(crate) fn choose_session(&mut self, session_id: String) {
        self.load_session(session_id);
        self.refresh_sessions_quietly();
    }

    /// Creates a Session and focuses the composer.
    pub(crate) fn new_session(&mut self) {
        self.request_sessions(SessionOperation::Create(self.selected_workspace.clone()));
        self.focus_composer = true;
    }

    /// Reloads the session list without a search query.
    pub(crate) fn reload_sessions(&mut self) {
        self.session_query.clear();
        self.refresh_sessions();
    }

    /// Runs the session search for the current query, or reloads when it is blank.
    pub(crate) fn search_sessions(&mut self) {
        if self.session_query.trim().is_empty() {
            self.reload_sessions();
        } else {
            self.request_sessions(SessionOperation::Search(self.session_query.clone()));
        }
    }

    /// Renames the selected Session from the title field.
    pub(crate) fn rename_selected_session(&mut self) {
        let Some(session_id) = self.selected_session.clone() else {
            return;
        };
        let title = self.session_title.trim().to_owned();
        if title.is_empty() {
            return;
        }
        self.request_sessions(SessionOperation::Rename(RenameSessionRequest { session_id, title }));
    }

    /// Forks the selected Session at the entered checkpoint, or at its latest
    /// sequence when the checkpoint field is blank.
    pub(crate) fn fork_selected_session(&mut self) {
        let Some(session_id) = self.selected_session.clone() else {
            return;
        };
        let at_seq = match parse_fork_checkpoint(&self.fork_seq_text) {
            Ok(at_seq) => at_seq,
            Err(error) => {
                self.sessions_error = Some(error);
                return;
            }
        };
        self.request_sessions(SessionOperation::Fork(ForkSessionRequest {
            session_id,
            at_seq,
            allow_migration: None,
        }));
    }

    /// True while any Session list request is in flight.
    pub(crate) fn sessions_busy(&self) -> bool {
        self.sessions_receiver.is_some()
    }

    /// Whether a Session request is in flight, without naming its private type.
    pub(crate) fn sessions_request_pending(&self) -> bool {
        self.sessions_receiver.is_some()
    }

    /// Archives the selected Session; `stop_activity` also stops its running
    /// work instead of being refused as `workspace/session-active`.
    pub(crate) fn archive_selected_session(&mut self, stop_activity: bool) {
        let Some(session_id) = self.selected_session.clone() else {
            return;
        };
        self.write_archive(session_id, stop_activity, true);
    }

    /// Restores one archived Session by id.
    pub(crate) fn unarchive_session(&mut self, session_id: String) {
        self.write_archive(session_id, false, false);
    }

    /// Permanently deletes the selected Session after a second click on the
    /// same selection; deletion is irreversible and the Host refuses a
    /// running or still-attached Session.
    pub(crate) fn delete_selected_session(&mut self) {
        let Some(session_id) = self.selected_session.clone() else {
            return;
        };
        if self.session_delete_confirm.as_deref() != Some(session_id.as_str()) {
            self.session_delete_confirm = Some(session_id);
            self.sessions_status = "Delete is irreversible; click Delete again to confirm".to_owned();
            return;
        }
        self.session_delete_confirm = None;
        self.request_sessions(SessionOperation::Delete(session_id));
    }

    /// Whether the selected Session's Delete button is waiting for confirmation.
    pub(crate) fn session_delete_armed(&self) -> bool {
        self.session_delete_confirm.is_some()
            && self.session_delete_confirm == self.selected_session
    }

    /// Runs the bounded workspace content search for the current query.
    pub(crate) fn run_file_search(&mut self) {
        if self.file_search_receiver.is_some() {
            return;
        }
        let query = self.file_search_query.trim().to_owned();
        if query.is_empty() {
            return;
        }
        let (Some(session_id), Some(transport)) = (self.selected_session.clone(), self.transport.clone())
        else {
            self.file_search_error = Some("Select a connected session to search files.".into());
            return;
        };
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let client = WorkspaceFilesClient::new(&transport);
            let result = client.search(WorkspaceFileScope { session_id }, query);
            let _ = sender.send(result);
        });
        self.file_search_receiver = Some(receiver);
        self.file_search_loading = true;
        self.file_search_error = None;
    }

    /// Renders the handoff summary from the current Session transcript.
    pub(crate) fn generate_handoff(&mut self) {
        let Some(session_id) = self.selected_session.clone() else {
            self.handoff_status = Some("Select a Session before generating a handoff".to_owned());
            return;
        };
        let markdown = crate::handoff::render_from(
            &self.transcript,
            Some(&session_id),
            &self.session_title,
            self.goal.as_ref(),
            self.transcript.run_state(self.assistant_streaming),
        );
        self.handoff_preview = Some(markdown);
        self.handoff_status = Some("Handoff preview generated from the Session log".to_owned());
    }

    /// Writes the rendered handoff into the workspace through the version-guarded
    /// file RPC. An existing file is refused unless `overwrite` is set.
    pub(crate) fn write_handoff(&mut self, overwrite: bool) {
        if self.handoff_receiver.is_some() {
            return;
        }
        let Some(markdown) = self.handoff_preview.clone() else {
            self.handoff_status = Some("Generate the handoff preview first".to_owned());
            return;
        };
        let Some(session_id) = self.selected_session.clone() else {
            self.handoff_status = Some("Select a Session before writing a handoff".to_owned());
            return;
        };
        let Some(transport) = self.transport.clone() else {
            self.handoff_status = Some("Connect before writing a handoff".to_owned());
            return;
        };
        let path = self.handoff_path.trim().to_owned();
        if path.is_empty() {
            self.handoff_status = Some("Enter the workspace path for the handoff".to_owned());
            return;
        }
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let client = WorkspaceFilesClient::new(&transport);
            let scope = WorkspaceFileScope {
                session_id: session_id.clone(),
            };
            let result = (|| -> Result<String, Error> {
                let existing = client.stat(scope.clone(), path.clone());
                match existing {
                    Ok(stat) if !overwrite => Err(Error::Remote {
                        code: "handoff-exists".to_owned(),
                        message: format!(
                            "{path} already exists (version {}); confirm overwrite to replace it",
                            stat.version
                        ),
                        details: serde_json::Map::new(),
                    }),
                    Ok(stat) => client
                        .write(scope, path.clone(), markdown, Some(stat.version))
                        .map(|written| format!("Wrote {path} ({} bytes)", written.after.len())),
                    Err(_) => client
                        .write(scope, path.clone(), markdown, None)
                        .map(|written| format!("Created {path} ({} bytes)", written.after.len())),                }
            })();
            let _ = sender.send(result);
        });
        self.handoff_receiver = Some(receiver);
        self.handoff_status = Some("Writing handoff to the workspace…".to_owned());
    }

    fn write_archive(&mut self, session_id: String, stop_activity: bool, archive: bool) {
        if self.archive_receiver.is_some() {
            return;
        }
        let Some(transport) = self.transport.clone() else {
            self.sessions_error = Some("Connect to change session grouping".to_owned());
            return;
        };
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let client = WorkspaceClient::new(&transport);
            let result = if archive {
                client.archive_session(&session_id, stop_activity)
            } else {
                client.unarchive_session(&session_id)
            };
            let _ = sender.send(result);
        });
        self.archive_receiver = Some(receiver);
        self.workspace_status = Some("Updating session grouping…".to_owned());
    }

    /// Pins or unpins the selected Session.
    pub(crate) fn set_selected_session_pinned(&mut self, pinned: bool) {
        let Some(session_id) = self.selected_session.clone() else {
            return;
        };
        if self.pin_receiver.is_some() {
            return;
        }
        let Some(transport) = self.transport.clone() else {
            self.sessions_error = Some("Connect to change session grouping".to_owned());
            return;
        };
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let client = WorkspaceClient::new(&transport);
            let result = if pinned {
                client.pin_session(&session_id)
            } else {
                client.unpin_session(&session_id)
            };
            let _ = sender.send(result);
        });
        self.pin_receiver = Some(receiver);
        self.workspace_status = Some("Updating session grouping…".to_owned());
    }

    /// Answers the first pending approval.
    pub(crate) fn decide_approval(&mut self, allow: bool) {
        let Some(pending) = self.pending_approvals.first().cloned() else {
            return;
        };
        let Some(commands) = &self.approval_commands else {
            return;
        };
        let decision = if allow {
            ApprovalDecision::AllowedOnce
        } else {
            ApprovalDecision::Rejected
        };
        let _ = commands.send(ApprovalCommand::Answer(pending.clone(), decision));
        self.pending_approvals
            .retain(|item| item.invocation.event_id != pending.invocation.event_id);
    }

    /// Hands the first pending approval to the user-driven delegate path.
    pub(crate) fn delegate_approval(&mut self) {
        let Some(pending) = self.pending_approvals.first().cloned() else {
            return;
        };
        let Some(commands) = &self.approval_commands else {
            return;
        };
        let _ = commands.send(ApprovalCommand::Delegate(pending.invocation.clone()));
        self.pending_approvals
            .retain(|item| item.invocation.event_id != pending.invocation.event_id);
    }

    /// Submits the first question card, either as answered or as skipped.
    pub(crate) fn submit_question(&mut self, skip: bool) {
        let Some(card) = self.pending_questions.first_mut() else {
            return;
        };
        let answer = if skip { card.skip() } else { card.answer() };
        let Some(pending) = card.pending().cloned() else {
            return;
        };
        let Some(commands) = &self.approval_commands else {
            return;
        };
        let _ = commands.send(ApprovalCommand::AnswerQuestion(pending, answer));
        self.pending_questions.remove(0);
    }

    /// Opens a deliverable in the Document surface.
    pub(crate) fn open_deliverable(&mut self, path: String) {
        self.preview_path = path.clone();
        self.switch_view(ActiveView::Document);
        if Self::is_office_path(&path) {
            self.read_office_document();
        } else {
            self.read_document();
        }
    }

    /// Creates the goal described by the objective field.
    pub(crate) fn create_goal(&mut self) {
        let objective = self.goal_objective.trim().to_owned();
        if objective.is_empty() {
            self.goal_error = Some("Describe what the agent should accomplish".to_owned());
            return;
        }
        let max_goal_rounds = match self.parse_goal_rounds() {
            Ok(rounds) => rounds,
            Err(message) => {
                self.goal_error = Some(message.to_owned());
                return;
            }
        };
        self.mutate_goal(GoalMutation::Create(CreateGoalRequest {
            objective,
            max_goal_rounds,
        }));
    }

    /// Saves objective and round edits against the current goal revision.
    pub(crate) fn save_goal_edits(&mut self) {
        let Some(goal) = self.goal.clone() else {
            return;
        };
        let objective = self.goal_objective.trim();
        if objective.is_empty() {
            self.goal_error = Some("Describe what the agent should accomplish".to_owned());
            return;
        }
        let rounds_changed = self.goal_max_rounds != goal.max_goal_rounds.to_string();
        let max_goal_rounds = if rounds_changed {
            match self.parse_goal_rounds() {
                Ok(rounds) => rounds,
                Err(message) => {
                    self.goal_error = Some(message.to_owned());
                    return;
                }
            }
        } else {
            None
        };
        let objective_changed = objective != goal.objective;
        if !objective_changed && max_goal_rounds.is_none() {
            return;
        }
        self.mutate_goal(GoalMutation::Edit(
            GoalRef {
                id: goal.id.clone(),
                revision: goal.revision,
            },
            EditGoalRequest {
                objective: objective_changed.then(|| objective.to_owned()),
                max_goal_rounds,
            },
        ));
    }

    /// Applies a lifecycle change to the current goal.
    pub(crate) fn change_goal(&mut self, action: GoalAction) {
        let Some(goal) = self.goal.clone() else {
            return;
        };
        let reference = GoalRef {
            id: goal.id.clone(),
            revision: goal.revision,
        };
        let mutation = match action {
            GoalAction::Pause => GoalMutation::Pause(reference),
            GoalAction::Resume => GoalMutation::Resume(reference),
            GoalAction::Complete => GoalMutation::Complete(reference),
            GoalAction::Clear => {
                self.clear_goal_confirmation = Some(reference);
                return;
            }
        };
        self.mutate_goal(mutation);
    }

    /// Applies the confirmed goal removal.
    pub(crate) fn confirm_clear_goal(&mut self) {
        let Some(reference) = self.clear_goal_confirmation.take() else {
            return;
        };
        self.mutate_goal(GoalMutation::Clear(reference));
    }

    fn parse_goal_rounds(&self) -> Result<Option<u64>, &'static str> {
        if self.goal_max_rounds.trim().is_empty() {
            return Ok(None);
        }
        match self.goal_max_rounds.trim().parse::<u64>() {
            Ok(rounds) if rounds > 0 && rounds <= 9_007_199_254_740_991 => Ok(Some(rounds)),
            _ => Err("Maximum rounds must be a positive whole number"),
        }
    }

    /// The pane at `index`, where `0` is the primary pane.
    pub(crate) fn terminal_pane_at(&self, index: usize) -> Option<&TerminalPanel> {
        if index == 0 {
            Some(&self.terminal)
        } else {
            self.terminal_extra
                .get(index - 1)
                .map(|pane| pane.as_ref())
        }
    }

    /// The pane at `index`, mutable.
    pub(crate) fn terminal_pane_mut(&mut self, index: usize) -> Option<&mut TerminalPanel> {
        if index == 0 {
            Some(&mut self.terminal)
        } else {
            self.terminal_extra
                .get_mut(index - 1)
                .map(|pane| pane.as_mut())
        }
    }

    /// Number of terminal panes currently shown.
    pub(crate) fn terminal_pane_count(&self) -> usize {
        1 + self.terminal_extra.len()
    }

    /// Terminal ids already shown in another pane, so a new pane can pick a free one.
    fn terminal_pane_selected(&self) -> Vec<String> {
        std::iter::once(&self.terminal)
            .chain(self.terminal_extra.iter().map(|pane| pane.as_ref()))
            .filter_map(|pane| pane.selected.clone())
            .collect()
    }

    /// Relative width of one terminal pane.
    pub(crate) fn terminal_pane_weight(&self, index: usize) -> f32 {
        self.terminal_pane_widths
            .get(index)
            .copied()
            .unwrap_or(1.0)
    }

    /// One pane's share of the pane row, in `0..=1`.
    pub(crate) fn terminal_pane_fraction(&self, index: usize) -> f32 {
        let total: f32 = (0..self.terminal_pane_count())
            .map(|slot| self.terminal_pane_weight(slot))
            .sum();
        self.terminal_pane_weight(index) / total.max(f32::EPSILON)
    }

    /// Mirrors a terminal field's text, sending each key immediately in raw mode.
    fn terminal_typed(&mut self, index: usize, text: String, cx: &mut Context<Self>) {
        self.focus_terminal_pane(index);
        let Some(panel) = self.terminal_pane_mut(index) else {
            return;
        };
        if !panel.raw_keys {
            panel.text = text;
            return;
        }
        let previous = std::mem::take(&mut panel.text);
        if text.is_empty() {
            return;
        }
        let (added, removed) = raw_keystroke(&previous, &text);
        if removed > 0 && !panel.send_encoded(&"\u{7f}".repeat(removed)) {
            panel.error = Some("Terminal is not writable; attach first".to_owned());
        }
        if !added.is_empty() && !panel.send_encoded(&added) {
            panel.error = Some("Terminal is not writable; attach first".to_owned());
        }
        panel.text.clear();
        if let Some(input) = self.inputs.get(TERMINAL_INPUT_KEYS[index]).cloned() {
            input.update(cx, |input, cx| input.set_text(String::new(), cx));
        }
        cx.notify();
    }

    /// Sends a cursor key in raw mode; a submit-line field leaves navigation alone.
    fn terminal_navigate(&mut self, index: usize, delta: i8) {
        let Some(panel) = self.terminal_pane_mut(index) else {
            return;
        };
        if !panel.raw_keys {
            return;
        }
        if let Some(bytes) = crate::terminal_input::arrow(delta) {
            if !panel.send_encoded(&bytes) {
                panel.error = Some("Terminal is not writable; attach first".to_owned());
            }
        }
    }

    /// Sends one named cursor or function key to a raw-mode pane.
    fn terminal_named_key(&mut self, index: usize, name: &'static str) {
        let Some(panel) = self.terminal_pane_mut(index) else {
            return;
        };
        if !panel.raw_keys {
            return;
        }
        if let Some(bytes) = crate::terminal_input::key(name) {
            if !panel.send_encoded(&bytes) {
                panel.error = Some("Terminal is not writable; attach first".to_owned());
            }
        }
    }

    /// Sends the addressed pane's field as one line, or Enter alone in raw mode.
    pub(crate) fn submit_terminal_input(&mut self, index: usize) {
        self.focus_terminal_pane(index);
        let Some(panel) = self.terminal_pane_mut(index) else {
            return;
        };
        let text = std::mem::take(&mut panel.text);
        if panel.raw_keys {
            if !panel.send_encoded("\r") {
                panel.error = Some("Terminal is not writable; attach first".to_owned());
            }
            return;
        }
        if text.is_empty() {
            return;
        }
        if !panel.send_encoded(&format!("{text}\r")) {
            panel.error = Some("Terminal is not writable; attach first".to_owned());
        }
    }

    /// Sets which terminal pane owns typed input.
    pub(crate) fn focus_terminal_pane(&mut self, index: usize) {
        if index < self.terminal_pane_count() {
            self.terminal_focus = index;
        }
    }

    /// Flips the focused pane between submit-line and raw keystroke input.
    pub(crate) fn toggle_terminal_raw_keys(&mut self, cx: &mut Context<Self>) {
        let index = self.terminal_focus;
        if let Some(panel) = self.terminal_pane_mut(index) {
            panel.raw_keys = !panel.raw_keys;
            panel.text.clear();
        }
        if let Some(input) = self.inputs.get(TERMINAL_INPUT_KEYS[index]).cloned() {
            input.update(cx, |input, cx| input.set_text(String::new(), cx));
        }
    }

    /// Adds one pane over a terminal the other panes do not show yet.
    pub(crate) fn add_terminal_pane(&mut self) {
        if self.terminal_pane_count() >= MAX_TERMINAL_PANES {
            self.status =
                format!("At most {MAX_TERMINAL_PANES} terminal panes are shown at once");
            return;
        }
        let Some(session) = self.selected_session.clone() else {
            self.status = "Select a connected Session before adding a terminal pane".to_owned();
            return;
        };
        let used = self.terminal_pane_selected();
        let other = self
            .terminal
            .items
            .iter()
            .find(|item| !used.contains(&item.id))
            .map(|item| item.id.clone());
        let mut panel = TerminalPanel::default();
        panel.session = Some(session);
        panel.cols = self.terminal.cols;
        panel.rows = self.terminal.rows;
        panel.items = self.terminal.items.clone();
        if let Some(id) = other {
            panel.select_terminal(id);
            if let Some(streams) = self.streams.clone() {
                panel.attach_selected(streams);
            }
        }
        panel.dirty = true;
        self.terminal_extra.push(Box::new(panel));
        self.terminal_pane_widths
            .resize(self.terminal_pane_count(), 1.0);
        self.terminal_focus = self.terminal_pane_count() - 1;
    }

    /// Removes one extra pane; the primary pane stays.
    pub(crate) fn remove_terminal_pane(&mut self, index: usize) {
        if index == 0 || index >= self.terminal_pane_count() {
            return;
        }
        self.terminal_extra.remove(index - 1);
        if self.terminal_pane_widths.len() > index {
            self.terminal_pane_widths.remove(index);
        }
        self.terminal_focus = self.terminal_focus.min(self.terminal_pane_count() - 1);
    }

    /// Returns the shell-owned input entity for `key`, creating it on first use.
    pub(crate) fn text_field(
        &mut self,
        key: &'static str,
        placeholder: &str,
        multiline: bool,
        cx: &mut Context<Self>,
    ) -> gpui::Entity<super::text_input::TextInput> {
        if let Some(input) = self.inputs.get(key) {
            return input.clone();
        }
        let placeholder: gpui::SharedString = placeholder.to_owned().into();
        let input = cx.new(|cx| super::text_input::TextInput::state(placeholder, multiline, cx));
        cx.subscribe(&input, move |app, entity, event, cx| {
            match event {
                super::text_input::TextInputEvent::Changed(text)
                    if terminal_input_index(key).is_some() =>
                {
                    if let Some(index) = terminal_input_index(key) {
                        app.terminal_typed(index, text.clone(), cx);
                    }
                    return;
                }
                super::text_input::TextInputEvent::Navigate(delta)
                    if terminal_input_index(key).is_some() =>
                {
                    if let Some(index) = terminal_input_index(key) {
                        app.terminal_navigate(index, *delta);
                    }
                    cx.notify();
                    return;
                }
                super::text_input::TextInputEvent::Key(name)
                    if terminal_input_index(key).is_some() =>
                {
                    if let Some(index) = terminal_input_index(key) {
                        app.terminal_named_key(index, name);
                    }
                    cx.notify();
                    return;
                }
                super::text_input::TextInputEvent::Changed(text) if key == "palette-query" => {
                    app.palette_query = text.clone();
                    app.palette_selected = 0;
                    app.request_palette_paths();
                    cx.notify();
                    return;
                }
                super::text_input::TextInputEvent::Changed(text) if key == "git-file-filter" => {
                    app.git_filter = text.clone();
                    cx.notify();
                    return;
                }
                super::text_input::TextInputEvent::Changed(text) if key == "git-commit-message" => {
                    app.git_commit_message = text.clone();
                    cx.notify();
                    return;
                }
                super::text_input::TextInputEvent::Navigate(delta) if key == "palette-query" => {
                    app.move_palette_selection(*delta);
                    cx.notify();
                    return;
                }
                super::text_input::TextInputEvent::Submit => {}
                _ => return,
            }
            let text = entity.read(cx).text().to_owned();
            match key {
                "composer" => {
                    app.chat_text = text;
                    if !app.chat_text.trim().is_empty()
                        && !app.chat_loading
                        && app.transport.is_some()
                        && app.attachment_receiver.is_none()
                    {
                        app.send_prompt();
                    }
                    entity.update(cx, |input, cx| input.set_text("", cx));
                }
                "session-search" => {
                    app.session_query = text;
                    app.search_sessions();
                }
                "session-title" => app.session_title = text,
                "session-fork-seq" => app.fork_seq_text = text,
                "file-picker-query" => app.file_picker_query = text,
                "file-search-query" => app.file_search_query = text,
                "handoff-path" => app.handoff_path = text,
                "document-body" => app.preview_draft = text,
                "market-query" => {
                    app.market_query = text;
                    app.search_market();
                }
                "skills-query" => app.skills_query = text,
                "document-path" => {
                    app.preview_path = text;
                    app.document_delete_confirm = false;
                    app.read_document();
                }
                "document-new-path" => app.document_new_path = text,
                "goal-objective" => app.goal_objective = text,
                "goal-rounds" => app.goal_max_rounds = text,
                "settings-draft" => app.host_settings_view.draft = Some(text),
                "palette-query" => {
                    app.palette_query = text;
                    app.run_selected_palette_match();
                }
                _ if terminal_input_index(key).is_some() => {
                    if let Some(index) = terminal_input_index(key) {
                        app.submit_terminal_input(index);
                    }
                    entity.update(cx, |input, cx| input.set_text(String::new(), cx));
                }
                _ => {}
            }
            cx.notify();
        })
        .detach();
        self.inputs.insert(key, input.clone());
        input
    }

    /// Reads the current value of a shell-owned input.
    pub(crate) fn input_text(&self, key: &str, cx: &App) -> String {
        self.inputs
            .get(key)
            .map(|input| input.read(cx).text().to_owned())
            .unwrap_or_default()
    }

    /// Replaces a shell-owned input's value.
    pub(crate) fn set_input(&mut self, key: &str, value: &str, cx: &mut Context<Self>) {
        if let Some(input) = self.inputs.get(key).cloned() {
            input.update(cx, |input, cx| input.set_text(value.to_owned(), cx));
        }
    }

    /// Focuses a shell-owned input when it exists.
    pub(crate) fn focus_input(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(input) = self.inputs.get(key).cloned() {
            input.update(cx, |input, cx| input.focus(window, cx));
        }
    }

    /// Copies edited text inputs into the plain state fields the workers use.
    pub(crate) fn sync_inputs(&mut self, cx: &mut Context<Self>) {
        if let Some(input) = self.inputs.get("composer").cloned() {
            if let Some(text) = self.pending_composer_text.take() {
                input.update(cx, |input, cx| input.set_text(text, cx));
            }
        }
        for (key, field) in [
            ("composer", &mut self.chat_text),
            ("session-search", &mut self.session_query),
            ("session-title", &mut self.session_title),
            ("market-query", &mut self.market_query),
            ("skills-query", &mut self.skills_query),
            ("document-path", &mut self.preview_path),
            ("goal-objective", &mut self.goal_objective),
            ("goal-rounds", &mut self.goal_max_rounds),
            ("palette-query", &mut self.palette_query),
            ("session-fork-seq", &mut self.fork_seq_text),
            ("file-picker-query", &mut self.file_picker_query),
            ("file-search-query", &mut self.file_search_query),
            ("handoff-path", &mut self.handoff_path),
            ("document-body", &mut self.preview_draft),
        ] {
            if let Some(input) = self.inputs.get(key) {
                *field = input.read(&*cx).text().to_owned();
            }
        }
        if let Some(input) = self.inputs.get("settings-draft") {
            self.host_settings_view.draft = Some(input.read(&*cx).text().to_owned());
        }
        self.remember_active_draft();
    }

    /// Recomputes the theme after the preferences changed.
    pub(crate) fn refresh_theme(&mut self, window: &Window) {
        let effective =
            effective_preferences(&self.ui_preferences, self.selected_workspace.as_deref());
        self.theme = Theme::new_with_catalog(
            &effective,
            appearance_is_dark(window.appearance()),
            &self.theme_catalog,
        );
    }

    /// Starts resizing one docked panel from a pointer position.
    pub(crate) fn begin_panel_drag(&mut self, target: ResizeTarget, start_x: f32) {
        // gpui 0.2.2 has no double-click event, so the divider times its own
        // presses: two on the same divider inside 400 ms reset that width.
        let now = Instant::now();
        let repeated = self
            .last_divider_click
            .is_some_and(|(last, at)| last == target && now.duration_since(at) <= Duration::from_millis(400));
        self.last_divider_click = Some((target, now));
        if repeated {
            self.last_divider_click = None;
            self.reset_panel_width(target);
            return;
        }
        let (start_width, start_secondary) = match target {
            ResizeTarget::Explorer => (
                self.ui_preferences
                    .workspace_prefs(self.selected_workspace.as_deref())
                    .workspace_panel_width
                    .unwrap_or(self.ui_preferences.workspace_panel_width),
                0.0,
            ),
            ResizeTarget::Changes => (
                self.ui_preferences
                    .workspace_prefs(self.selected_workspace.as_deref())
                    .changes_panel_width
                    .unwrap_or(self.ui_preferences.changes_panel_width),
                0.0,
            ),
            ResizeTarget::TerminalPane(boundary) => {
                self.terminal_pane_widths
                    .resize(self.terminal_pane_count(), 1.0);
                (
                    self.terminal_pane_weight(boundary),
                    self.terminal_pane_weight(boundary + 1),
                )
            }
        };
        self.panel_drag = Some(PanelDrag {
            target,
            start_x,
            start_width,
            start_secondary,
        });
    }

    /// Applies a pointer position to the panel being resized, within its bounds.
    pub(crate) fn drag_panel(&mut self, x: f32) {
        let Some(drag) = self.panel_drag else {
            return;
        };
        let delta = x - drag.start_x;
        match drag.target {
            ResizeTarget::Explorer => {
                let width = (drag.start_width + delta).clamp(
                    crate::layout::EXPLORER_MIN,
                    crate::layout::EXPLORER_MAX,
                );
                self.ui_preferences
                    .set_workspace_panel_width(self.selected_workspace.as_deref(), width);
            }
            ResizeTarget::Changes => {
                let width =
                    (drag.start_width - delta).clamp(crate::layout::CHANGES_MIN, crate::layout::CHANGES_MAX);
                self.ui_preferences
                    .set_changes_panel_width(self.selected_workspace.as_deref(), width);
            }
            ResizeTarget::TerminalPane(boundary) => {
                let total = drag.start_width + drag.start_secondary;
                let minimum = 0.25_f32.min(total / 2.0);
                let first = (drag.start_width + delta / 400.0).clamp(minimum, total - minimum);
                if self.terminal_pane_widths.len() <= boundary + 1 {
                    self.terminal_pane_widths
                        .resize(self.terminal_pane_count(), 1.0);
                }
                if let Some(pair) = self
                    .terminal_pane_widths
                    .get_mut(boundary..boundary + 2)
                {
                    pair[0] = first;
                    pair[1] = total - first;
                }
            }
        }
    }

    /// Restores one panel's default width after the divider's reset gesture.
    fn reset_panel_width(&mut self, target: ResizeTarget) {
        let workspace = self.selected_workspace.as_deref();
        match target {
            ResizeTarget::Explorer => self.ui_preferences.reset_workspace_panel_width(workspace),
            ResizeTarget::Changes => self.ui_preferences.reset_changes_panel_width(workspace),
            ResizeTarget::TerminalPane(_) => {
                self.terminal_pane_widths.clear();
                return;
            }
        }
        self.persist_preferences();
    }

    /// Ends a resize and stores the resulting width.
    pub(crate) fn finish_panel_drag(&mut self) {
        if let Some(drag) = self.panel_drag.take() {
            if !matches!(drag.target, ResizeTarget::TerminalPane(_)) {
                self.persist_preferences();
            }
        }
    }

    /// Persists the current interface preferences.
    pub(crate) fn persist_preferences(&mut self) {
        match self.ui_preferences.save(&self.ui_preferences_path) {
            Ok(()) => self.preferences_message = Some("Interface preferences saved.".to_owned()),
            Err(error) => {
                self.preferences_message = Some(format!("Could not save preferences: {error}"));
            }
        }
    }

    /// Applies a local preference change and records it.
    pub(crate) fn update_preferences(
        &mut self,
        window: &Window,
        change: impl FnOnce(&mut UiPreferences),
    ) {
        change(&mut self.ui_preferences);
        self.refresh_theme(window);
        self.persist_preferences();
    }

    /// Opens the picker for a theme pack file to install.
    pub(crate) fn add_theme_file(&mut self) {
        self.dialog = Some((DialogPurpose::ThemeAdd, dialog::open_theme_file_async()));
        self.theme_message = Some("Choose a theme pack JSON file…".to_owned());
    }

    /// Replaces the pending user theme after the duplicate-id warning.
    pub(crate) fn replace_theme_file(&mut self, window: &Window) {
        let Some(path) = self.theme_pending_import.take() else {
            self.theme_message =
                Some("Choose a theme file before confirming a replacement.".to_owned());
            return;
        };
        match self.theme_catalog.import_file(&path, true) {
            Ok(id) => {
                self.reload_theme_catalog(window);
                self.theme_message = Some(format!("Replaced user theme `{id}`."));
            }
            Err(error) => self.theme_message = Some(format!("Could not add theme: {error}")),
        }
    }

    /// Installs a picked theme file, warning before it replaces a user file.
    ///
    /// @param path — file chosen by the user.
    /// @param window — window whose appearance the refreshed theme follows.
    pub(crate) fn import_theme_file(&mut self, path: PathBuf, window: &Window) {
        match self.theme_catalog.import_file(&path, false) {
            Ok(id) => {
                self.theme_pending_import = None;
                self.reload_theme_catalog(window);
                self.theme_message = Some(format!("Added user theme `{id}`."));
            }
            Err(ThemeCatalogError::AlreadyInstalled { id }) => {
                self.theme_pending_import = Some(path);
                self.theme_message = Some(format!(
                    "A user theme with id `{id}` already exists. Choose Replace theme file to overwrite it."
                ));
            }
            Err(error) => self.theme_message = Some(format!("Could not add theme: {error}")),
        }
    }

    /// Writes a complete copy of one installed pack.
    ///
    /// @param id — pack to copy.
    /// @param window — window whose appearance the refreshed catalogue follows.
    pub(crate) fn duplicate_theme(&mut self, id: &str, window: &Window) {
        match self.theme_catalog.duplicate(id) {
            Ok(copy) => {
                self.reload_theme_catalog(window);
                self.theme_message =
                    Some(format!("Created `{copy}.json` from the `{id}` theme."));
            }
            Err(error) => self.theme_message = Some(format!("Could not duplicate theme: {error}")),
        }
    }

    /// Opens one user pack in the JSON editor.
    ///
    /// @param id — installed user pack id.
    pub(crate) fn edit_theme(&mut self, id: &str) {
        match self.theme_catalog.edit_text(id) {
            Ok(text) => {
                self.theme_editor = Some(ThemeEditor {
                    id: id.to_owned(),
                    pending_text: Some(text),
                });
                self.theme_message =
                    Some(format!("Editing `{id}`. Save validates the document before writing."));
            }
            Err(error) => self.theme_message = Some(format!("Could not edit theme: {error}")),
        }
    }

    /// Validates and saves the JSON editor's current text.
    ///
    /// @param text — complete pack document typed by the user.
    /// @param window — window whose appearance the refreshed theme follows.
    pub(crate) fn save_theme_editor(&mut self, text: &str, window: &Window) {
        let Some(editor) = self.theme_editor.clone() else {
            return;
        };
        match self.theme_catalog.save_edit(&editor.id, text) {
            Ok(id) => {
                if self.ui_preferences.theme_id == editor.id {
                    self.ui_preferences.theme_id = id.clone();
                }
                self.theme_editor = None;
                self.reload_theme_catalog(window);
                self.persist_preferences();
                self.theme_message = Some(format!("Saved user theme `{id}`."));
            }
            Err(error) => {
                self.theme_message = Some(format!("Could not save theme: {error}"));
            }
        }
    }

    /// Closes the JSON editor without writing the file.
    pub(crate) fn cancel_theme_editor(&mut self) {
        self.theme_editor = None;
        self.theme_message = Some("Theme edits discarded; the file is unchanged.".to_owned());
    }

    /// Opens the export picker for one installed pack.
    ///
    /// @param id — pack to export.
    pub(crate) fn export_theme(&mut self, id: &str) {
        if self.theme_catalog.find(id).is_none() {
            self.theme_message = Some(format!("Theme `{id}` is not installed."));
            return;
        }
        self.theme_export_id = Some(id.to_owned());
        self.dialog = Some((
            DialogPurpose::ThemeExport,
            dialog::save_theme_file_async(&format!("{id}.json")),
        ));
        self.theme_message = Some(format!("Choose where to save `{id}.json`…"));
    }

    /// Writes the pack selected for export to a picked destination.
    ///
    /// @param path — destination returned by the save dialog.
    pub(crate) fn export_theme_to(&mut self, path: &Path) {
        let Some(id) = self.theme_export_id.take() else {
            self.theme_message = Some("Choose a theme before exporting.".to_owned());
            return;
        };
        match self.theme_catalog.export_to(&id, path) {
            Ok(()) => self.theme_message = Some(format!("Exported `{id}` to {}.", path.display())),
            Err(error) => self.theme_message = Some(format!("Could not export theme: {error}")),
        }
    }

    /// Removes a user pack after a second confirmation click.
    ///
    /// @param id — installed user pack id.
    /// @param window — window whose appearance the refreshed theme follows.
    pub(crate) fn remove_theme(&mut self, id: &str, window: &Window) {
        if self.theme_remove_confirm.as_deref() != Some(id) {
            self.theme_remove_confirm = Some(id.to_owned());
            self.theme_message =
                Some(format!("Remove user theme `{id}`? Choose Confirm remove to delete its file."));
            return;
        }
        self.theme_remove_confirm = None;
        match self.theme_catalog.remove(id) {
            Ok(()) => {
                self.reload_theme_catalog(window);
                self.theme_message = Some(format!("Removed user theme `{id}`."));
            }
            Err(error) => self.theme_message = Some(format!("Could not remove theme: {error}")),
        }
    }

    fn reload_theme_catalog(&mut self, window: &Window) {
        self.theme_catalog = ThemeCatalog::load(&self.ui_themes_path);
        self.refresh_theme(window);
    }

    /// Opens the in-app GitHub import dialog.
    pub(crate) fn open_github_dialog(&mut self) {
        self.github_dialog = Some(GithubThemeDialog {
            status: None,
            error: None,
            busy: false,
        });
    }

    /// Closes the GitHub import dialog without fetching.
    pub(crate) fn close_github_dialog(&mut self) {
        self.github_dialog = None;
        if self.github_receiver.is_none() {
            self.theme_message = Some("GitHub theme import cancelled.".to_owned());
        }
    }

    /// Fetches and validates a source without caching it.
    ///
    /// @param source — URL or `owner/repo[@ref]`.
    /// @param ref_name — optional repository ref.
    /// @param path — optional repository document path.
    pub(crate) fn check_github_source(&mut self, source: &str, ref_name: &str, path: &str) {
        if self.github_receiver.is_some() {
            if let Some(dialog) = self.github_dialog.as_mut() {
                dialog.error = Some("A GitHub theme request is already running.".to_owned());
            }
            return;
        }
        let spec = match remote::resolve_source(source, ref_name, path) {
            Ok(spec) => spec,
            Err(error) => {
                if let Some(dialog) = self.github_dialog.as_mut() {
                    dialog.error = Some(error.to_string());
                    dialog.status = None;
                }
                return;
            }
        };
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let result = remote::check_source(&spec, &remote::ReqwestFetcher)
                .map_err(|error| error.to_string());
            let _ = sender.send(GithubTaskResult::Checked(result));
        });
        self.github_receiver = Some(receiver);
        if let Some(dialog) = self.github_dialog.as_mut() {
            dialog.busy = true;
            dialog.error = None;
            dialog.status = Some("Checking the GitHub theme source…".to_owned());
        }
    }

    /// Fetches, validates, and caches a GitHub theme.
    ///
    /// @param source — URL or `owner/repo[@ref]`.
    /// @param ref_name — optional repository ref.
    /// @param path — optional repository document path.
    pub(crate) fn import_github_source(&mut self, source: &str, ref_name: &str, path: &str) {
        if self.github_receiver.is_some() {
            if let Some(dialog) = self.github_dialog.as_mut() {
                dialog.error = Some("A GitHub theme request is already running.".to_owned());
            }
            return;
        }
        let spec = match remote::resolve_source(source, ref_name, path) {
            Ok(spec) => spec,
            Err(error) => {
                if let Some(dialog) = self.github_dialog.as_mut() {
                    dialog.error = Some(error.to_string());
                    dialog.status = None;
                }
                return;
            }
        };
        let themes = self.ui_themes_path.clone();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let result = remote::import_source(&themes, &spec, &remote::ReqwestFetcher)
                .map(|cached| cached.pack.id)
                .map_err(|error| error.to_string());
            let _ = sender.send(GithubTaskResult::Imported(result));
        });
        self.github_receiver = Some(receiver);
        if let Some(dialog) = self.github_dialog.as_mut() {
            dialog.busy = true;
            dialog.error = None;
            dialog.status = Some("Importing the GitHub theme…".to_owned());
        }
    }

    /// Refreshes one cached GitHub pack with its stored ETag.
    ///
    /// @param id — cached pack id.
    pub(crate) fn update_github_theme(&mut self, id: &str) {
        if self.github_receiver.is_some() {
            self.theme_message = Some("A GitHub theme request is already running.".to_owned());
            return;
        }
        let cached = self
            .theme_catalog
            .find(id)
            .is_some_and(|theme| theme.source == ThemeSource::Github);
        if !cached {
            self.theme_message = Some(format!("Theme `{id}` is not a cached GitHub theme."));
            return;
        }
        let themes = self.ui_themes_path.clone();
        let id_owned = id.to_owned();
        let worker_id = id_owned.clone();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let result = remote::refresh_cached(&themes, &worker_id, &remote::ReqwestFetcher)
                .map_err(|error| error.to_string());
            let _ = sender.send(GithubTaskResult::Updated {
                id: worker_id,
                result,
            });
        });
        self.github_receiver = Some(receiver);
        self.theme_message = Some(format!("Checking `{id_owned}` for updates…"));
    }

    /// Applies one completed GitHub worker result.
    fn poll_github_theme(&mut self, window: &Window) -> bool {
        let Some(receiver) = &self.github_receiver else {
            return false;
        };
        match receiver.try_recv() {
            Ok(result) => {
                self.github_receiver = None;
                match result {
                    GithubTaskResult::Checked(Ok(checked)) => {
                        if let Some(dialog) = self.github_dialog.as_mut() {
                            dialog.busy = false;
                            dialog.error = None;
                            let ref_name = if checked.ref_name.is_empty() {
                                "the default branch".to_owned()
                            } else {
                                format!("ref `{}`", checked.ref_name)
                            };
                            dialog.status = Some(format!(
                                "Found `{}` (`{}`) from {} at {ref_name}.",
                                checked.name, checked.id, checked.source_url
                            ));
                        }
                    }
                    GithubTaskResult::Checked(Err(error)) => {
                        if let Some(dialog) = self.github_dialog.as_mut() {
                            dialog.busy = false;
                            dialog.status = None;
                            dialog.error = Some(error);
                        }
                    }
                    GithubTaskResult::Imported(Ok(id)) => {
                        self.github_dialog = None;
                        self.reload_theme_catalog(window);
                        let shadowed = self
                            .theme_catalog
                            .find(&id)
                            .is_some_and(|theme| theme.source != ThemeSource::Github);
                        self.theme_message = Some(if shadowed {
                            format!(
                                "Cached GitHub theme `{id}`; a local user file with the same id takes precedence."
                            )
                        } else {
                            format!("Imported cached GitHub theme `{id}`.")
                        });
                    }
                    GithubTaskResult::Imported(Err(error)) => {
                        if let Some(dialog) = self.github_dialog.as_mut() {
                            dialog.busy = false;
                            dialog.status = None;
                            dialog.error = Some(error);
                        }
                    }
                    GithubTaskResult::Updated { id, result } => match result {
                        Ok(RefreshOutcome::Unchanged) => {
                            self.theme_message =
                                Some(format!("Theme `{id}` is up to date; the cache is unchanged."));
                        }
                        Ok(RefreshOutcome::Updated(meta)) => {
                            self.reload_theme_catalog(window);
                            self.theme_message = Some(format!(
                                "Updated cached theme `{id}` from {}.",
                                meta.source_url
                            ));
                        }
                        Err(error) => {
                            self.theme_message = Some(format!(
                                "Could not update theme `{id}`: {error}. The cached copy is unchanged."
                            ));
                        }
                    },
                }
                true
            }
            Err(mpsc::TryRecvError::Empty) => false,
            Err(mpsc::TryRecvError::Disconnected) => {
                self.github_receiver = None;
                if let Some(dialog) = self.github_dialog.as_mut() {
                    dialog.busy = false;
                    dialog.error = Some("The GitHub theme request stopped unexpectedly.".to_owned());
                }
                true
            }
        }
    }

    /// The composer's model label, naming the selected model or the blocker.
    pub(crate) fn model_label(&self) -> String {
        self.model_picker_label()
    }

    /// Whether the Host reported no usable model provider.
    pub(crate) fn model_catalog_empty_now(&self) -> bool {
        self.model_catalog_empty()
    }

    /// Applies a model chosen in the composer picker.
    pub(crate) fn select_model_ids(
        &mut self,
        provider: &str,
        model: &str,
        reasoning_effort: Option<String>,
    ) {
        self.select_model(ModelSelection {
            provider: provider.to_owned(),
            model: model.to_owned(),
            reasoning_effort,
        });
    }

    /// Opens the native file picker for the attachment flow.
    pub(crate) fn open_file_dialog(&mut self) {
        self.dialog = Some((DialogPurpose::Open, dialog::open_file_async()));
    }

    /// Opens the native folder picker for the workspace flow.
    pub(crate) fn open_folder_dialog(&mut self) {
        self.dialog = Some((DialogPurpose::Folder, dialog::open_folder_async()));
    }

    /// Registers the picked folder on the Host and starts a session in it.
    fn open_folder_path(&mut self, path: PathBuf) {
        let Some(transport) = self.transport.clone() else {
            self.sessions_error = Some("Connect before opening a folder".to_owned());
            return;
        };
        if self.workspace_create_receiver.is_some() {
            return;
        }
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let result = WorkspaceClient::new(&transport)
                .create_workspace(&path.display().to_string());
            let _ = sender.send(result);
        });
        self.workspace_create_receiver = Some(receiver);
        self.workspace_status = Some("Opening folder…".to_owned());
    }

    /// Whether the Host path is an Office document this client renders.
    pub(crate) fn is_office_path_now(path: &str) -> bool {
        Self::is_office_path(path)
    }

    /// Enables or disables one profile plugin entry.
    pub(crate) fn set_plugin_enabled(&mut self, id: String, enabled: bool) {
        self.write_plugin(PluginAction::SetPlugin { id, enabled });
    }

    /// Enables or disables one plugin bundle.
    pub(crate) fn set_bundle_enabled(&mut self, name: String, enabled: bool) {
        self.write_plugin(PluginAction::SetBundle { name, enabled });
    }

    /// Removes one installed plugin bundle.
    pub(crate) fn remove_bundle(&mut self, name: String) {
        self.write_plugin(PluginAction::RemoveBundle { name });
    }

    /// Installs a marketplace entry by its Host-provided spec.
    pub(crate) fn install_entry(&mut self, spec: String) {
        self.install_market_entry(spec, MarketplaceInstallOptions { registry: None });
    }

    /// Updates one installed package by removing it and installing the index
    /// spec, then restoring its previous enablement when known.
    pub(crate) fn update_market_entry(&mut self, spec: String, name: String, version: String) {
        if self.market_loading {
            return;
        }
        let Some(transport) = self.transport.clone() else {
            self.market_status = Some("Connect to install".to_owned());
            return;
        };
        let was_enabled = self
            .plugin_bundles
            .iter()
            .find(|bundle| bundle.name == name)
            .map(|bundle| bundle.enabled);
        let status_text = format!("Updating {name} to {version}…");
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let market = MarketplaceClient::new(&transport);
            let plugins = PluginsClient::new(&transport);
            let result = (|| -> Result<String, Error> {
                plugins.remove_bundle(&name)?;
                let inspect = market.inspect(&spec, &MarketplaceInstallOptions { registry: None })?;
                let status = inspect
                    .get("status")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("");
                if status != "accepted" {
                    let problem = inspect
                        .get("problem")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("unknown");
                    let reason = inspect
                        .get("reason")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("the Host refused this install target");
                    return Err(Error::Remote {
                        code: "inspect-rejected".into(),
                        message: format!("removed {name}, then inspect refused {spec}: {problem}: {reason}; reinstall to restore"),
                        details: serde_json::Map::new(),
                    });
                }
                let install = market.install_bundle(&spec, &MarketplaceInstallOptions { registry: None })?;
                let application = install
                    .get("application")
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(|| Error::InvalidEnvelope("missing application result"))?;
                if was_enabled == Some(true) {
                    plugins.set_bundle_enabled(&name, true)?;
                }
                Ok(format!("{name} -> {version} ({application})"))
            })();
            let _ = sender.send(result.map(serde_json::Value::String));
        });
        self.market_receiver = Some(receiver);
        self.market_loading = true;
        self.market_status = Some(status_text);
    }

    /// Runs one shared shell command from the menu, shortcut, or palette.
    pub(crate) fn run_command(&mut self, command: CommandId) {
        self.show_session_menu = false;
        self.top_menu_open = false;
        self.workspace_picker_open = false;
        self.status_menu_open = false;
        self.show_about = false;
        match command {
            CommandId::ToggleSettings => {
                if self.show_settings {
                    self.close_preferences();
                } else {
                    self.open_settings();
                }
            }
            CommandId::ToggleMarketplace => {
                self.show_market = !self.show_market;
                self.active_view = ActiveView::Marketplace;
            }
            CommandId::ToggleGoals => {
                self.show_goals = !self.show_goals;
                self.active_view = ActiveView::Goals;
            }
            CommandId::NewSession => {
                self.request_sessions(SessionOperation::Create(self.selected_workspace.clone()));
                self.focus_composer = true;
            }
            CommandId::ToggleExplorer => {
                self.show_explorer = !self.show_explorer;
            }
            CommandId::ToggleChanges => {
                self.show_changes = !self.show_changes;
            }
            CommandId::ShowChangesDiff => {
                self.show_changes = true;
                self.changes_tab = ChangesTab::Diff;
            }
            CommandId::ToggleTerminal => {
                self.bottom_tool = if self.bottom_tool == Some(BottomTool::Terminal) {
                    None
                } else {
                    Some(BottomTool::Terminal)
                };
            }
            CommandId::TogglePlugins => {
                self.show_plugins = !self.show_plugins;
                self.active_view = ActiveView::Plugins;
            }
            CommandId::ToggleSkills => {
                self.show_skills = !self.show_skills;
                self.active_view = ActiveView::Skills;
            }
            CommandId::OpenFile => {
                self.switch_view(ActiveView::Document);
                self.open_file_dialog();
            }
            CommandId::About => {
                self.show_about = true;
            }
            CommandId::ShowPalette => {
                self.palette_open = true;
                self.palette_query.clear();
                self.palette_selected = 0;
                self.focus_palette = true;
                self.refresh_host_commands();
            }
            CommandId::Dismiss => {
                self.dismiss_overlay();
            }
            CommandId::Quit => {
                self.flush_session_state();
                self.quitting
                    .store(true, std::sync::atomic::Ordering::Release);
                self.close_requested = true;
            }
        }
    }

    /// Closes the command palette and clears its query.
    pub(crate) fn close_palette(&mut self) {
        self.palette_open = false;
        self.palette_query.clear();
        self.palette_selected = 0;
        self.focus_palette = false;
    }

    /// Closes the topmost overlay and reports whether one was open.
    ///
    /// Overlays stack in paint order, so a single Escape peels exactly the one
    /// the user is looking at and leaves the rest untouched.
    ///
    /// @returns `true` when an overlay was closed.
    pub(crate) fn dismiss_overlay(&mut self) -> bool {
        // Reverse paint order, so Escape always peels the overlay on top.
        if self.git_confirmation.is_some() {
            self.git_confirmation = None;
            return true;
        }
        if self.clear_goal_confirmation.is_some() {
            self.clear_goal_confirmation = None;
            return true;
        }
        if self.update_prompt.is_some() {
            self.update_prompt = None;
            return true;
        }
        if self.git_stash.is_some() {
            self.close_git_stash();
            return true;
        }
        if let Some(menu) = self.git_menu.take() {
            self.pending_chrome_focus = Some(match menu {
                GitMenu::Branch => ChromeTrigger::GitBranch,
                GitMenu::Sync => ChromeTrigger::GitSync,
            });
            return true;
        }
        if self.status_menu_open {
            self.status_menu_open = false;
            self.pending_chrome_focus = Some(ChromeTrigger::StatusTools);
            return true;
        }
        if self.show_session_menu {
            self.show_session_menu = false;
            return true;
        }
        if self.workspace_picker_open {
            self.workspace_picker_open = false;
            self.pending_chrome_focus = Some(ChromeTrigger::Workspace);
            return true;
        }
        if self.top_menu_open {
            self.top_menu_open = false;
            self.pending_chrome_focus = Some(ChromeTrigger::Menu);
            return true;
        }
        if self.palette_open {
            self.close_palette();
            self.pending_chrome_focus = Some(ChromeTrigger::Search);
            return true;
        }
        if self.show_about {
            self.show_about = false;
            return true;
        }
        if self.show_settings {
            self.close_preferences();
            return true;
        }
        if self.git_conflict_view.is_some() {
            self.close_git_conflict();
            return true;
        }
        false
    }

    /// One row the palette can run.
    /// Keeps the palette's workspace path matches in step with its query.
    ///
    /// One request is in flight at a time; a result that answers an older query
    /// is dropped and the current query is asked again.
    pub(crate) fn request_palette_paths(&mut self) {
        let query = self.palette_query.trim().to_lowercase();
        if query.is_empty() || self.transport.is_none() || self.selected_session.is_none() {
            self.palette_paths.clear();
            self.palette_paths_for.clear();
            return;
        }
        if self.palette_paths_receiver.is_some() || self.palette_paths_for == query {
            return;
        }
        let (Some(session_id), Some(transport)) =
            (self.selected_session.clone(), self.transport.clone())
        else {
            return;
        };
        let (sender, receiver) = mpsc::channel();
        let requested = query.clone();
        std::thread::spawn(move || {
            let client = WorkspaceFilesClient::new(&transport);
            let result = client.search_paths(
                WorkspaceFileScope { session_id },
                requested,
                PALETTE_PATH_RESULTS,
            );
            let _ = sender.send(result);
        });
        self.palette_paths_for = query;
        self.palette_paths_receiver = Some(receiver);
    }

    pub(crate) fn palette_entries(&self) -> Vec<PaletteEntry> {
        let query = self.palette_query.trim().to_lowercase();
        let mut entries: Vec<PaletteEntry> = CommandId::matching(&self.palette_query)
            .into_iter()
            .map(PaletteEntry::Shell)
            .collect();
        entries.extend(
            matching_host_commands(&self.host_commands, &query)
                .into_iter()
                .map(PaletteEntry::Host),
        );
        if !query.is_empty() {
            let mut files: Vec<String> = self
                .explorer
                .visible_rows()
                .into_iter()
                .filter(|row| matches!(row.kind, crate::explorer::ExplorerRowKind::File { .. }))
                .filter(|row| {
                    row.label.to_ascii_lowercase().contains(&query)
                        || row.path.to_ascii_lowercase().contains(&query)
                })
                .map(|row| row.path)
                .collect();
            // A loaded directory tree only covers the folders the user opened, so
            // the Host's bounded path search widens it without reading content.
            if self.palette_paths_for == query {
                for path in &self.palette_paths {
                    if !files.contains(path) {
                        files.push(path.clone());
                    }
                }
            }
            entries.extend(files.into_iter().take(8).map(PaletteEntry::File));
        }
        entries
    }

    /// Moves the palette selection by one step, clamped to the match list.
    pub(crate) fn move_palette_selection(&mut self, delta: i8) {
        self.palette_selected = stepped_selection(self.palette_selected, delta, self.palette_entries().len());
    }

    /// Runs the row the palette currently selects.
    pub(crate) fn run_selected_palette_match(&mut self) {
        let entries = self.palette_entries();
        let Some(entry) = entries.get(self.palette_selected.min(entries.len().saturating_sub(1))) else {
            return;
        };
        let entry = entry.clone();
        self.run_palette_entry(entry);
    }

    /// Runs one palette row after closing the palette.
    pub(crate) fn run_palette_entry(&mut self, entry: PaletteEntry) {
        self.close_palette();
        match entry {
            PaletteEntry::Shell(command) => self.run_command(command),
            PaletteEntry::Host(command) => self.run_host_command(command),
            PaletteEntry::File(path) => {
                self.preview_path = path;
                self.show_document = true;
                self.active_view = ActiveView::Document;
                self.read_document();
            }
        }
    }

    /// Loads the Host's human commands for the selected Session.
    pub(crate) fn refresh_host_commands(&mut self) {
        let (Some(session_id), Some(transport)) =
            (self.selected_session.clone(), self.transport.clone())
        else {
            self.host_commands.clear();
            self.host_commands_error = None;
            return;
        };
        if self.host_commands_receiver.is_some() {
            return;
        }
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let result = CommandsClient::new(&transport).list(&session_id);
            let _ = sender.send(result);
        });
        self.host_commands_receiver = Some(receiver);
        self.host_commands_loading = true;
        self.host_commands_error = None;
    }

    /// Sends a composer line that starts with `/` to the Host command registry.
    fn submit_slash_command(&mut self, line: String, transport: HttpTransport) {
        let Some(session_id) = self.selected_session.clone() else {
            return;
        };
        if self.host_command_receiver.is_some() {
            self.page_error = Some("A Host command is already running".to_owned());
            return;
        }
        let attachments = self
            .attachment
            .take()
            .map(|attachment| {
                vec![CommandSubmitAttachment::File {
                    receipt_id: attachment.receipt_id,
                }]
            })
            .unwrap_or_default();
        self.attachment_name.clear();
        self.chat_text.clear();
        self.remember_active_draft();
        self.record_prompt_history(&line);
        self.pending_slash_line = Some(line.clone());
        self.status = format!("Running {line}…");
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let result = CommandsClient::new(&transport).execute(&session_id, &line, attachments);
            let _ = sender.send(result);
        });
        self.host_command_receiver = Some(receiver);
        self.host_command_loading = true;
    }

    /// Whether the connected Host registered the `plan` command.
    pub(crate) fn plan_command_available(&self) -> bool {
        self.host_commands.iter().any(|command| command.name == "plan")
    }

    /// Toggles plan mode through the runtime's own `/plan` command.
    pub(crate) fn toggle_plan_mode(&mut self) {
        let Some(transport) = self.transport.clone() else {
            self.status = "Connect before changing plan mode".to_owned();
            return;
        };
        let line = if self.transcript.plan_active() {
            "/plan off"
        } else {
            "/plan"
        };
        self.submit_slash_command(line.to_owned(), transport);
    }

    /// Executes one Host-registered command line and reports its real outcome.
    pub(crate) fn run_host_command(&mut self, command: CommandDescriptor) {
        let (Some(session_id), Some(transport)) =
            (self.selected_session.clone(), self.transport.clone())
        else {
            self.status = "Select a connected Session before running a Host command".to_owned();
            return;
        };
        if self.host_command_receiver.is_some() {
            return;
        }
        let line = format!("/{}", command.name);
        self.status = format!("Running {line}…");
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let result = CommandsClient::new(&transport).execute(&session_id, &line, Vec::new());
            let _ = sender.send(result);
        });
        self.host_command_receiver = Some(receiver);
        self.host_command_loading = true;
    }

    /// Settles command-list and command-execution workers.
    fn poll_host_commands(&mut self) -> bool {
        let mut changed = false;
        if let Some(receiver) = &self.host_commands_receiver {
            match receiver.try_recv() {
                Ok(Ok(commands)) => {
                    self.host_commands = commands;
                    self.host_commands_error = None;
                    self.host_commands_loading = false;
                    self.host_commands_receiver = None;
                    changed = true;
                }
                Ok(Err(error)) => {
                    self.host_commands_error = Some(failure_text(&error));
                    self.host_commands_loading = false;
                    self.host_commands_receiver = None;
                    changed = true;
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.host_commands_error = Some("Command list worker stopped unexpectedly".to_owned());
                    self.host_commands_loading = false;
                    self.host_commands_receiver = None;
                    changed = true;
                }
            }
        }
        if let Some(receiver) = &self.host_command_receiver {
            match receiver.try_recv() {
                Ok(Ok(execution)) => {
                    if execution.is_some() {
                        self.pending_slash_line = None;
                    }
                    self.status = match execution {
                        Some(execution) => match execution.result {
                            CommandResult::Success { text, .. } => text.unwrap_or_else(|| "Command completed".to_owned()),
                            CommandResult::Error { text } => text,
                        },
                        None => {
                            match self.pending_slash_line.take() {
                                Some(line) => {
                                    self.chat_text = line;
                                    self.page_error = Some(
                                        "The Host did not resolve that command; it is back in the composer."
                                            .to_owned(),
                                    );
                                    "Unknown command".to_owned()
                                }
                                None => "The Host did not accept that command line".to_owned(),
                            }
                        }
                    };
                    self.host_command_loading = false;
                    self.host_command_receiver = None;
                    changed = true;
                }
                Ok(Err(error)) => {
                    self.status = failure_text(&error);
                    self.host_command_loading = false;
                    self.host_command_receiver = None;
                    self.pending_slash_line = None;
                    changed = true;
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.status = "Command worker stopped unexpectedly".to_owned();
                    self.host_command_loading = false;
                    self.host_command_receiver = None;
                    changed = true;
                }
            }
        }
        changed
    }

    pub(crate) fn tick(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.repaint_soon = false;
        let commands = std::mem::take(
            &mut *self
                .commands
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        );
        for command in commands {
            self.run_command(command);
            self.repaint_soon = true;
        }
        self.flush_session_state_if_quiet();
        self.poll_run_state();
        self.poll_dialog(window);
        if self.poll_github_theme(window) {
            self.repaint_soon = true;
        }
        self.poll_attachment();
        self.poll_git_auto_refresh();
        if self.explorer.poll(self.transport.as_ref()) {
            self.repaint_soon = true;
        }
        let previous_probe = self.git_refresh_probe_started_at;
        self.git_refresh_probe_started_at = self.explorer.workspace_changed_since(previous_probe);
        if self.git_refresh_probe_started_at != previous_probe {
            self.git_refresh_pending = true;
        }
        self.poll_jobs();
        self.poll_schedules();
        if self.poll_diagnostics() {
            self.repaint_soon = true;
        }
        if self.poll_diagnose() {
            self.repaint_soon = true;
        }
        if self.poll_git() {
            self.repaint_soon = true;
        }
        if self.poll_host_commands() {
            self.repaint_soon = true;
        }
        if self.dialog.is_some()
            || self.jobs_receiver.is_some()
            || self.job_receiver.is_some()
            || self.schedules_receiver.is_some()
            || self.schedule_delete_receiver.is_some()
            || self.diagnostics_receiver.is_some()
            || self.diagnose_receiver.is_some()
            || self.github_receiver.is_some()
        {
            self.repaint_soon = true;
        }
        if self.quitting.load(Ordering::Acquire) {
            self.flush_session_state();
            self.terminal.detach();
            for pane in &mut self.terminal_extra {
                pane.detach();
            }
            self.close_requested = true;
            return false;
        }
        if let Some(host) = self.host.as_mut() {
            while let Some(event) = host.event() {
                match event {
                    host::HostEvent::Failed(message) => {
                        eprintln!("dsh: native Host failed: {message}");
                        self.status = message;
                        self.update_prompt = None;
                        self.update_error = None;
                        self.host_url = None;
                        self.transport = None;
                        self.streams = None;
                        self.connect_receiver = None;
                        self.git_status = None;
                        self.git_branches = None;
                        self.git_error = None;
                        self.git_loading = false;
                        self.git_receiver = None;
                        if self.chat_loading {
                            self.chat_loading = false;
                            self.chat_receiver = None;
                            self.assistant_streaming = false;
                            self.assistant_draft.clear();
                            self.page_error = Some(
                                "The Host connection was lost during this turn. Reload the Session to confirm its state before retrying.".to_owned(),
                            );
                        }
                        self.terminal.detach();
                        for pane in &mut self.terminal_extra {
                            pane.detach();
                        }
                    }
                    host::HostEvent::UpdatePrompt(prompt) => {
                        self.update_prompt = Some(prompt);
                        self.update_error = None;
                        self.update_status = None;
                        self.repaint_soon = true;
                    }
                    host::HostEvent::Restart => {
                        match restart::executable(std::env::var_os("APPIMAGE").map(Into::into))
                            .and_then(|path| restart::schedule(&path, std::process::id())) {
                            Ok(_) => {
                                self.quitting.store(true, Ordering::Release);
                                self.close_requested = true;
                                return false;
                            }
                            Err(error) => eprintln!("dsh: native restart failed; restart the application manually: {error}"),
                        }
                    }
                }
            }
        }
        self.terminal.select_session(
            self.selected_session.clone(),
            self.transport.clone(),
            self.streams.clone(),
        );
        self.terminal.poll();
        let session = self.selected_session.clone();
        let transport = self.transport.clone();
        let streams = self.streams.clone();
        for pane in &mut self.terminal_extra {
            pane.select_session(session.clone(), transport.clone(), streams.clone());
            pane.poll();
        }
        if let Some(receiver) = &self.host_receiver {
            match receiver.try_recv() {
                Ok(host) => {
                    let retry_after_start = self.retry_after_host_start;
                    self.retry_after_host_start = false;
                    self.host_url = host.host_url().map(str::to_owned);
                    self.status = host.error().map_or_else(
                        || {
                            self.host_url.as_ref().map_or_else(
                                || "Disconnected — Host launch configuration is not set".to_owned(),
                                |_| "Host ready — click Connect".to_owned(),
                            )
                        },
                        str::to_owned,
                    );
                    self.host = Some(host);
                    self.host_receiver = None;
                    if self.host_url.is_some() {
                        eprintln!(
                            "dsh: native host=ready in {}ms",
                            self.started_at.elapsed().as_millis()
                        );
                    }
                    if (self.auto_connect || retry_after_start) && self.host_url.is_some() {
                        self.connect();
                    }
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.status = "Host startup worker stopped unexpectedly".to_owned();
                    self.host_receiver = None;
                    self.retry_after_host_start = false;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    self.status = host_start_status(self.started_at.elapsed());
                    self.repaint_soon = true;
                }
            }
        }
        if let Some(receiver) = &self.connect_receiver {
            match receiver.try_recv() {
                Ok(Ok((base, transport))) => {
                    eprintln!(
                        "dsh: native auth=ok in {}ms",
                        self.started_at.elapsed().as_millis()
                    );
                    let streams = match transport.credential().and_then(|credential| {
                        RemoteStreamMux::connect(transport.base_url().as_str(), credential)
                    }) {
                        Ok(streams) => streams,
                        Err(error) => {
                            eprintln!("dsh: native stream mux failed: {error}");
                            self.connect_receiver = None;
                            self.status = format!("Remote streams unavailable: {error}");
                            return false;
                        }
                    };
                    self.host_url = Some(base);
                    self.refresh_models();
                    let approval_probe = matches!(
                        std::env::var("DSH_DESKTOP_PROBE_APPROVAL").as_deref(),
                        Ok("1" | "allow" | "reject" | "delegate")
                    );
                    let terminal_probe =
                        std::env::var("DSH_DESKTOP_PROBE_TERMINAL").as_deref() == Ok("1");
                    let timed_question_probe =
                        std::env::var("DSH_DESKTOP_PROBE_TIMED_QUESTION").as_deref() == Ok("1");
                    if approval_probe {
                        // The probe owns the only `$events` reader so one waterfall
                        // answer cannot race a second connected client.
                        let probe_transport = transport.clone();
                        let probe_mux = streams.clone();
                        std::thread::spawn(move || probe_approval(probe_transport, probe_mux));
                    } else if timed_question_probe {
                        // Same exclusivity: the timed question is answered by the
                        // probe, not by a card the operator would have to click.
                        let probe_transport = transport.clone();
                        let probe_mux = streams.clone();
                        std::thread::spawn(move || {
                            probe_timed_question(probe_transport, probe_mux)
                        });
                    } else {
                        self.start_remote_events(transport.clone(), streams.clone());
                    }
                    if terminal_probe {
                        let probe_transport = transport.clone();
                        let probe_mux = streams.clone();
                        std::thread::spawn(move || probe_terminal(probe_transport, probe_mux));
                    }
                    if std::env::var("DSH_DESKTOP_PROBE_STREAMS").as_deref() == Ok("1") {
                        let probe_transport = transport.clone();
                        let probe_mux = streams.clone();
                        std::thread::spawn(move || probe_streams(probe_transport, probe_mux));
                    }
                    self.streams = Some(streams);
                    self.transport = Some(transport);
                    self.start_workspace_follow();
                    self.connect_receiver = None;
                    self.status = "Connected — loading sessions…".to_owned();
                    self.refresh_sessions();
                    self.refresh_models();
                    if let Some(session_id) = self.selected_session.clone() {
                        self.reload_session_after_reconnect(session_id);
                    }
                }
                Ok(Err(error)) => {
                    self.connect_receiver = None;
                    eprintln!("dsh: native auth=failed: {error}");
                    self.status = format!("Authentication failed: {error}");
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.connect_receiver = None;
                    self.status = "Authentication worker stopped unexpectedly".to_owned();
                }
                Err(mpsc::TryRecvError::Empty) => {
                    self.repaint_soon = true;
                }
            }
        }
        if let Some(receiver) = &self.sessions_receiver {
            match receiver.try_recv() {
                Ok(Ok(update)) => {
                    self.sessions_receiver = None;
                    match update {
                        SessionUpdate::List(sessions) => {
                            self.sessions = sessions;
                            self.session_search = None;
                            self.sessions_status = format!("{} session(s)", self.sessions.len());
                            match restore_session_target(
                                self.ui_preferences.restore_last_session,
                                self.ui_preferences.last_session_id.as_deref(),
                                &self.sessions,
                                self.selected_session.as_deref(),
                            ) {
                                RestoreSession::None => {}
                                RestoreSession::Load(session_id) => {
                                    self.load_session(session_id);
                                }
                                RestoreSession::Missing { last, fallback } => {
                                    if let Some(session_id) = fallback {
                                        self.sessions_status = format!(
                                            "Last Session {last} is unavailable; opened the most recent Session instead."
                                        );
                                        self.load_session(session_id);
                                    } else {
                                        self.sessions_status = format!(
                                            "Last Session {last} is unavailable and no other Sessions exist."
                                        );
                                    }
                                }
                            }
                        }
                        SessionUpdate::Search(results) => {
                            self.sessions_status = format!("{} match(es)", results.items.len());
                            self.session_search = Some(results);
                        }
                        SessionUpdate::Created(id) | SessionUpdate::Forked(id) => {
                            self.load_session(id.clone());
                            self.session_query.clear();
                            self.session_search = None;
                            self.refresh_sessions();
                            self.sessions_status = format!("Session {id} ready; refreshing list…");
                        }
                        SessionUpdate::Renamed(title) => {
                            self.session_title = title.clone();
                            self.session_query.clear();
                            self.session_search = None;
                            self.refresh_sessions();
                            self.sessions_status = format!("Renamed to {title}; refreshing list…");
                        }
                        SessionUpdate::Deleted(id) => {
                            self.agent_tabs.retain(|tab| tab != &id);
                            if self.selected_session.as_deref() == Some(id.as_str()) {
                                self.selected_session = None;
                            }
                            self.session_query.clear();
                            self.session_search = None;
                            self.refresh_sessions();
                            self.sessions_status = format!("Deleted Session {id}; refreshing list…");
                        }
                    }
                }
                Ok(Err(error)) => {
                    self.sessions_receiver = None;
                    self.sessions_status = "Session request failed".to_owned();
                    self.sessions_error =
                        Some(format!("{}: {error}", self.sessions_failure_context));
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.sessions_receiver = None;
                    self.sessions_status = "Session worker stopped unexpectedly".to_owned();
                    self.sessions_error = Some(self.sessions_failure_context.to_owned());
                }
                Err(mpsc::TryRecvError::Empty) => {
                    self.repaint_soon = true;
                }
            }
        }
        if let Some(receiver) = &self.archive_receiver {
            match receiver.try_recv() {
                Ok(Ok(value)) => {
                    self.archived_session_ids = value.archived_session_ids;
                    self.workspace_status = Some(format!(
                        "{} archived session(s)",
                        self.archived_session_ids.len()
                    ));
                    self.archive_receiver = None;
                    self.refresh_sessions();
                }
                Ok(Err(error)) => {
                    self.sessions_error = Some(format!("Archive change failed: {error}"));
                    self.archive_receiver = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.sessions_error = Some("Archive worker stopped unexpectedly".into());
                    self.archive_receiver = None;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    self.repaint_soon = true;
                }
            }
        }
        if let Some(receiver) = &self.pin_receiver {
            match receiver.try_recv() {
                Ok(Ok(value)) => {
                    self.pinned_session_ids = value.pinned_session_ids;
                    self.workspace_status = Some(format!(
                        "{} pinned session(s)",
                        self.pinned_session_ids.len()
                    ));
                    self.pin_receiver = None;
                }
                Ok(Err(error)) => {
                    self.sessions_error = Some(format!("Pin change failed: {error}"));
                    self.pin_receiver = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.sessions_error = Some("Pin worker stopped unexpectedly".into());
                    self.pin_receiver = None;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    self.repaint_soon = true;
                }
            }
        }
        if let Some(receiver) = &self.workspace_receiver {
            match receiver.try_recv() {
                Ok(Ok(frame)) => self.apply_workspace_frame(frame),
                Ok(Err(error)) => {
                    self.workspace_error = Some(format!("Workspace sync failed: {error}"));
                    self.workspace_receiver = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.workspace_error = Some("Workspace sync stopped unexpectedly".into());
                    self.workspace_receiver = None;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    self.repaint_soon = true;
                }
            }
        }
        if let Some(receiver) = &self.handoff_receiver {
            match receiver.try_recv() {
                Ok(Ok(message)) => {
                    self.handoff_status = Some(message);
                    self.handoff_receiver = None;
                    if let Some(transport) = self.transport.clone() {
                        self.explorer.refresh_root(&transport);
                    }
                }
                Ok(Err(error)) => {
                    self.handoff_status = Some(format!("Handoff write refused: {error}"));
                    self.handoff_receiver = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.handoff_status = Some("Handoff worker stopped unexpectedly".into());
                    self.handoff_receiver = None;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    self.repaint_soon = true;
                }
            }
        }
        if let Some(receiver) = &self.workspace_create_receiver {            match receiver.try_recv() {
                Ok(Ok(created)) => {
                    self.workspace_create_receiver = None;
                    self.selected_workspace = Some(created.workspace.workspace_id.clone());
                    self.workspace_status = Some(format!(
                        "Workspace {} open; creating a session…",
                        created.workspace.path
                    ));
                    self.request_sessions(SessionOperation::Create(Some(
                        created.workspace.workspace_id,
                    )));
                    self.focus_composer = true;
                }
                Ok(Err(error)) => {
                    self.sessions_error = Some(format!("Open folder failed: {error}"));
                    self.workspace_create_receiver = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.sessions_error = Some("Open folder worker stopped unexpectedly".into());
                    self.workspace_create_receiver = None;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    self.repaint_soon = true;
                }
            }
        }
        if let Some(receiver) = &self.palette_paths_receiver {
            match receiver.try_recv() {
                Ok(Ok(value)) => {
                    self.palette_paths_receiver = None;
                    if self.palette_paths_for == self.palette_query.trim().to_lowercase() {
                        self.palette_paths = value.paths;
                    }
                    // Typing during the request leaves the answer stale; ask again.
                    self.request_palette_paths();
                }
                Ok(Err(_)) => {
                    self.palette_paths_receiver = None;
                    self.palette_paths.clear();
                    self.palette_paths_for.clear();
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.palette_paths_receiver = None;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    self.repaint_soon = true;
                }
            }
        }
        if let Some(receiver) = &self.file_search_receiver {
            match receiver.try_recv() {
                Ok(Ok(value)) => {
                    self.file_search_items = value.matches;
                    self.file_search_scanned = value.scanned_files;
                    self.file_search_truncated = value.truncated;
                    self.file_search_loading = false;
                    self.file_search_receiver = None;
                }
                Ok(Err(error)) => {
                    self.file_search_loading = false;
                    self.file_search_receiver = None;
                    self.file_search_error = Some(format!("File search failed: {error}"));
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.file_search_loading = false;
                    self.file_search_receiver = None;
                    self.file_search_error = Some("File search worker stopped unexpectedly".into());
                }
                Err(mpsc::TryRecvError::Empty) => {
                    self.repaint_soon = true;
                }
            }
        }
        if let Some(receiver) = &self.page_receiver {
            match receiver.try_recv() {
                Ok(Ok((through_seq, page, live))) => {
                    self.page_through_seq = Some(through_seq);
                    let cached_last = self.transcript.last_seq();
                    self.scroll_transcript_to_bottom = true;
                    let before_seq = self.page_before_seq.take();
                    let oldest_incoming = page.records.first().map(|record| record.event.seq);
                    let advanced = match (before_seq, oldest_incoming) {
                        (Some(before_seq), Some(seq)) => seq < before_seq,
                        (None, _) => true,
                        (Some(_), None) => false,
                    };
                    // History and live events share one projection; the page
                    // is no longer kept as a second copy of the same events.
                    let history: Vec<_> = page
                        .records
                        .iter()
                        .map(|record| {
                            dsh_client_core::follow::SessionWireEvent::from(&record.event)
                        })
                        .collect();
                    // A Host 0.2.2 follow opening always starts at the log end,
                    // so a cached transcript can leave a gap when the Session
                    // advanced while inactive. Reset instead of merging across it.
                    if cached_last >= 0 {
                        let oldest_history = history.iter().map(|event| event.seq).min();
                        let oldest_live = live.iter().map(|frame| frame.event.seq).min();
                        let oldest = match (oldest_history, oldest_live) {
                            (Some(left), Some(right)) => Some(left.min(right)),
                            (Some(value), None) | (None, Some(value)) => Some(value),
                            (None, None) => None,
                        };
                        if oldest.is_some_and(|seq| seq > cached_last + 1) {
                            self.transcript.reset();
                        }
                    }
                    self.transcript.apply_snapshot(&live);
                    for event in &history {
                        self.record_deliverables(event);
                    }
                    self.transcript.merge(history.iter());
                    self.scroll_transcript_to_bottom = true;
                    self.page_has_more = page.has_more && advanced;
                    self.page_loading = false;
                    self.page_error = None;
                    self.page_receiver = None;
                }
                Ok(Err(error)) => {
                    self.page_loading = false;
                    self.page_before_seq = None;
                    self.page_error = Some(format!("Could not load session: {error}"));
                    self.page_receiver = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.page_loading = false;
                    self.page_before_seq = None;
                    self.page_error = Some("Session request stopped unexpectedly".to_owned());
                    self.page_receiver = None;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    self.repaint_soon = true;
                }
            }
        }
        if let Some(receiver) = &self.goal_receiver {
            match receiver.try_recv() {
                Ok(Ok(goal)) => {
                    self.goal = goal;
                    if let Some(goal) = &self.goal {
                        self.goal_objective = goal.objective.clone();
                        self.goal_max_rounds = goal.max_goal_rounds.to_string();
                    } else {
                        self.goal_objective.clear();
                        self.goal_max_rounds.clear();
                    }
                    self.goal_loading = false;
                    self.goal_error = None;
                    self.goal_receiver = None;
                }
                Ok(Err(error)) => {
                    self.goal_loading = false;
                    self.goal_error = Some(format!("Could not load goal: {error}"));
                    self.goal_receiver = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.goal_loading = false;
                    self.goal_error = Some("Goal request stopped unexpectedly".to_owned());
                    self.goal_receiver = None;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    self.repaint_soon = true;
                }
            }
        }
        if let Some(receiver) = &self.goal_mutation_receiver {
            match receiver.try_recv() {
                Ok(Ok(goal)) => {
                    self.goal = goal;
                    if let Some(goal) = &self.goal {
                        self.goal_objective = goal.objective.clone();
                        self.goal_max_rounds = goal.max_goal_rounds.to_string();
                        self.goal_status = Some("Goal updated".to_owned());
                    } else {
                        self.goal_objective.clear();
                        self.goal_status = Some("Goal cleared".to_owned());
                    }
                    self.goal_mutation_loading = false;
                    self.goal_error = None;
                    self.goal_mutation_receiver = None;
                }
                Ok(Err(error)) => {
                    self.goal_mutation_loading = false;
                    self.goal_error = Some(format!("Could not change goal: {error}"));
                    self.goal_mutation_receiver = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.goal_mutation_loading = false;
                    self.goal_error = Some("Goal update stopped unexpectedly".to_owned());
                    self.goal_mutation_receiver = None;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    self.repaint_soon = true;
                }
            }
        }
        if let Some(receiver) = &self.plan_command_receiver {
            match receiver.try_recv() {
                Ok(Ok(Some(execution))) => {
                    self.plan_command_loading = false;
                    self.plan_command_receiver = None;
                    match execution.result {
                        CommandResult::Success { text, .. } => {
                            self.plan_command_status =
                                text.or_else(|| Some("Plan command completed".to_owned()));
                            if let Some(session_id) = self.selected_session.clone() {
                                self.load_session(session_id);
                            }
                        }
                        CommandResult::Error { text } => {
                            self.plan_command_status = Some(text);
                        }
                    }
                }
                Ok(Ok(None)) => {
                    self.plan_command_loading = false;
                    self.plan_command_receiver = None;
                    self.plan_command_status =
                        Some("The /plan command is not available in this Session".to_owned());
                }
                Ok(Err(error)) => {
                    self.plan_command_loading = false;
                    self.plan_command_receiver = None;
                    self.plan_command_status = Some(format!("Plan command failed: {error}"));
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.plan_command_loading = false;
                    self.plan_command_receiver = None;
                    self.plan_command_status = Some("Plan command stopped unexpectedly".to_owned());
                }
                Err(mpsc::TryRecvError::Empty) => {
                    self.repaint_soon = true;
                }
            }
        }
        if let Some(receiver) = &self.settings_update_receiver {
            let result = match receiver.try_recv() {
                Ok(result) => Some(result.map(|_| ())),
                Err(mpsc::TryRecvError::Disconnected) => {
                    Some(Err(Error::InvalidEnvelope("settings worker stopped")))
                }
                Err(mpsc::TryRecvError::Empty) => {
                    self.repaint_soon = true;
                    None
                }
            };
            if let Some(result) = result {
                self.settings_update_receiver = None;
                match result {
                    Ok(()) => {
                        self.host_settings_view.draft = None;
                        self.host_settings_view.message = None;
                        self.refresh_settings();
                    }
                    Err(error) => {
                        self.host_settings_view.message = Some(settings_write_error(&error))
                    }
                }
            }
        }
        if let Some(receiver) = &self.settings_receiver {
            match receiver.try_recv() {
                Ok(Ok(description)) => {
                    self.host_settings_view.draft = None;
                    self.settings_description = Some(description);
                    self.settings_error = None;
                    self.settings_loading = false;
                    self.settings_receiver = None;
                }
                Ok(Err(error)) => {
                    let _ = error; // Raw server messages may include submitted settings or credentials.
                    self.settings_error =
                        Some("Could not load settings. Refresh to try again.".into());
                    self.settings_loading = false;
                    self.settings_receiver = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.settings_error = Some("Settings request stopped unexpectedly".to_owned());
                    self.settings_loading = false;
                    self.settings_receiver = None;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    self.repaint_soon = true;
                }
            }
        }
        if let Some(receiver) = &self.update_open_receiver {
            match receiver.try_recv() {
                Ok(Ok(())) => {
                    self.update_open_receiver = None;
                    self.update_status = Some("Update check completed.".into());
                }
                Ok(Err(_)) => {
                    self.update_open_receiver = None;
                    self.update_status = Some(
                        "Could not check for updates. Check the Host connection and retry.".into(),
                    );
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.update_open_receiver = None;
                    self.update_status = Some("The update request stopped unexpectedly.".into());
                }
                Err(mpsc::TryRecvError::Empty) => {
                    self.repaint_soon = true;
                }
            }
        }
        if let Some(receiver) = &self.document_op_receiver {
            match receiver.try_recv() {
                Ok(Ok(DocumentMutation::Renamed(result))) => {
                    self.document_op_receiver = None;
                    self.preview_path = result.path.clone();
                    self.document_new_path.clear();
                    self.document_op_status = Some(format!("Renamed to {}", result.path));
                    if let Some(transport) = self.transport.as_ref() {
                        self.explorer.refresh_root(transport);
                    }
                    self.read_document();
                }
                Ok(Ok(DocumentMutation::Deleted)) => {
                    self.document_op_receiver = None;
                    self.preview_document = None;
                    self.preview_image = None;
                    self.pending_preview_text = None;
                    self.preview_draft.clear();
                    self.document_op_status = Some("Deleted from the Host.".to_owned());
                    if let Some(transport) = self.transport.as_ref() {
                        self.explorer.refresh_root(transport);
                    }
                }
                Ok(Err(error)) => {
                    self.document_op_receiver = None;
                    self.document_op_status = Some(format!("File change failed: {error}"));
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.document_op_receiver = None;
                    self.document_op_status = Some("File worker stopped unexpectedly".into());
                }
                Err(mpsc::TryRecvError::Empty) => {
                    self.repaint_soon = true;
                }
            }
        }
        if let Some(receiver) = &self.preview_receiver {
            match receiver.try_recv() {
                Ok(Ok(document)) => {
                    match document {
                        PreviewDocument::Text(document) => {
                            self.preview_draft = document.text.clone();
                            self.pending_preview_text = Some(document.text.clone());
                            self.preview_save_status = None;
                            self.preview_document = Some(document);
                            self.preview_image = None;
                        }
                        PreviewDocument::Image(image) => {
                            self.preview_image = Some(image);
                            self.preview_document = None;
                        }
                    }
                    self.preview_loading = false;
                    self.preview_error = None;
                    self.preview_receiver = None;
                }
                Ok(Err(error)) => {
                    self.preview_loading = false;
                    self.preview_error = Some(format!("Could not read Host file: {error}"));
                    self.preview_receiver = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.preview_loading = false;
                    self.preview_error = Some("Document read worker stopped unexpectedly".into());
                    self.preview_receiver = None;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    self.repaint_soon = true;
                }
            }
        }
        if let Some(receiver) = &self.preview_save_receiver {
            match receiver.try_recv() {
                Ok(Ok(result)) => {
                    self.preview_save_status = Some(format!(
                        "Saved {} ({})",
                        result.stat.absolute_path, result.operation
                    ));
                    if let Some(document) = self.preview_document.as_mut() {
                        document.version = Some(result.stat.version);
                        document.text = self.preview_draft.clone();
                    }
                    self.preview_save_receiver = None;
                    self.preview_error = None;
                }
                Ok(Err(error)) => {
                    self.preview_save_status = None;
                    self.preview_error = Some(format!("Host save failed: {error}"));
                    self.preview_save_receiver = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.preview_save_status = None;
                    self.preview_error = Some("Document save worker stopped unexpectedly".into());
                    self.preview_save_receiver = None;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    self.repaint_soon = true;
                }
            }
        }
        if let Some(receiver) = &self.cancel_receiver {
            match receiver.try_recv() {
                Ok(Ok(_)) => {
                    self.status = "Turn cancellation requested".to_owned();
                    self.cancel_receiver = None;
                    self.refresh_sessions_quietly();
                }
                Ok(Err(error)) => {
                    self.page_error = Some(format!("Cancel failed: {error}"));
                    self.cancel_receiver = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.page_error = Some("Cancel worker stopped unexpectedly".into());
                    self.cancel_receiver = None;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    self.repaint_soon = true;
                }
            }
        }
        if let Some(receiver) = &self.model_receiver {
            match receiver.try_recv() {
                Ok(Ok(catalog)) => {
                    if self.model_selection.is_none() {
                        self.model_selection = Some(catalog.default.clone());
                    }
                    self.model_catalog = Some(catalog);
                    self.model_error = None;
                    self.model_receiver = None;
                    self.model_requested_at = None;
                }
                Ok(Err(error)) => {
                    self.model_error = Some(format!("Could not load models: {error}"));
                    self.model_receiver = None;
                    self.model_requested_at = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.model_error = Some("Model catalog worker stopped unexpectedly".into());
                    self.model_receiver = None;
                    self.model_requested_at = None;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    // A Host without a configured provider can leave this call
                    // unanswered; stop waiting so the picker can say why.
                    if self.model_requested_at.is_some_and(|started| {
                        started.elapsed() > std::time::Duration::from_secs(10)
                    }) {
                        self.model_receiver = None;
                        self.model_requested_at = None;
                        self.model_error = Some(
                            "The Host did not answer the model catalog; check provider settings"
                                .to_owned(),
                        );
                    }
                    self.repaint_soon = true;
                }
            }
        }
        if let Some(receiver) = &self.model_select_receiver {
            match receiver.try_recv() {
                Ok(Ok(response)) => {
                    self.model_selection = Some(response.selected);
                    self.model_error = None;
                    self.model_select_receiver = None;
                }
                Ok(Err(error)) => {
                    self.model_error = Some(format!("Model selection failed: {error}"));
                    self.model_select_receiver = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.model_error = Some("Model selection worker stopped unexpectedly".into());
                    self.model_select_receiver = None;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    self.repaint_soon = true;
                }
            }
        }
        if let Some(receiver) = &self.model_init_receiver {
            match receiver.try_recv() {
                Ok(Ok(())) => {
                    self.model_init_receiver = None;
                    self.model_selection = None;
                    self.model_error = None;
                    self.refresh_models();
                }
                Ok(Err(error)) => {
                    self.model_error = Some(format!("Default model selection failed: {error}"));
                    self.model_init_receiver = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.model_error = Some("Default model worker stopped unexpectedly".into());
                    self.model_init_receiver = None;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    self.repaint_soon = true;
                }
            }
        }
        if let Some(receiver) = &self.plugins_receiver {
            match receiver.try_recv() {
                Ok(Ok((snapshot, bundles, plugins))) => {
                    self.plugins_snapshot = Some(snapshot);
                    self.plugin_bundles = bundles;
                    self.managed_plugins = plugins;
                    self.plugins_loading = false;
                    self.plugins_error = None;
                    self.plugins_receiver = None;
                }
                Ok(Err(error)) => {
                    self.plugins_loading = false;
                    self.plugins_error = Some(format!("Could not load plugins: {error}"));
                    self.plugins_receiver = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.plugins_loading = false;
                    self.plugins_error =
                        Some("Plugin inventory worker stopped unexpectedly".into());
                    self.plugins_receiver = None;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    self.repaint_soon = true;
                }
            }
        }
        if let Some(receiver) = &self.plugin_write_receiver {
            match receiver.try_recv() {
                Ok(Ok(result)) => {
                    let warning = result
                        .warnings
                        .first()
                        .map(|warning| format!(" ({warning})"))
                        .unwrap_or_default();
                    self.plugin_write_status = Some(format!(
                        "{} · {}{warning}",
                        result.target, result.application
                    ));
                    self.plugin_write_receiver = None;
                    if result.changed {
                        self.refresh_plugins();
                    }
                }
                Ok(Err(error)) => {
                    self.plugins_error = Some(format!("Plugin change failed: {error}"));
                    self.plugin_write_receiver = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.plugins_error = Some("Plugin change worker stopped unexpectedly".into());
                    self.plugin_write_receiver = None;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    self.repaint_soon = true;
                }
            }
        }
        if let Some(receiver) = &self.feedback_put_receiver {
            match receiver.try_recv() {
                Ok(Ok(item)) => {
                    if let Some(existing) = self
                        .feedback_items
                        .iter_mut()
                        .find(|x| x.message_id == item.message_id)
                    {
                        *existing = item;
                    } else {
                        self.feedback_items.push(item);
                    }
                    self.feedback_put_receiver = None;
                    self.feedback_status = Some("Feedback saved.".to_owned());
                }
                Ok(Err(error)) => {
                    self.feedback_status = Some(format!("Feedback failed: {error}"));
                    self.feedback_put_receiver = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.feedback_status = Some("Feedback worker stopped unexpectedly".into());
                    self.feedback_put_receiver = None;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    self.repaint_soon = true;
                }
            }
        }
        if let Some(receiver) = &self.feedback_receiver {
            match receiver.try_recv() {
                Ok(Ok(items)) => {
                    self.feedback_items = items;
                    self.feedback_receiver = None;
                }
                Ok(Err(error)) => {
                    self.feedback_status = Some(format!("Feedback load failed: {error}"));
                    self.feedback_receiver = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.feedback_status = Some("Feedback worker stopped unexpectedly".into());
                    self.feedback_receiver = None;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    self.repaint_soon = true;
                }
            }
        }
        if let Some(receiver) = &self.market_receiver {
            match receiver.try_recv() {
                Ok(Ok(value)) => {
                    if let Some(text) = value.as_str() {
                        self.market_status = Some(text.to_owned());
                        self.refresh_plugins();
                    } else if value.get("entries").is_some() {
                        self.market_results = value
                            .get("entries")
                            .and_then(serde_json::Value::as_array)
                            .cloned()
                            .unwrap_or_default();
                        self.market_status =
                            Some(format!("{} result(s)", self.market_results.len()));
                    } else if value.get("url").is_some() || value.is_array() {
                        let indexes = if value.is_array() {
                            value.as_array().cloned().unwrap_or_default()
                        } else {
                            vec![value.clone()]
                        };
                        self.market_indexes = indexes;
                        self.market_url = self
                            .market_indexes
                            .first()
                            .and_then(|index| index.get("url"))
                            .and_then(serde_json::Value::as_str)
                            .map(str::to_owned);
                        self.market_status =
                            Some(format!("{} index(es)", self.market_indexes.len()));
                    }
                    self.market_loading = false;
                    self.market_receiver = None;
                }
                Ok(Err(error)) => {
                    self.market_status = Some(format!("Marketplace failed: {error}"));
                    self.market_loading = false;
                    self.market_receiver = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.market_status = Some("Marketplace worker stopped unexpectedly".into());
                    self.market_loading = false;
                    self.market_receiver = None;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    self.repaint_soon = true;
                }
            }
        }
        if let Some(receiver) = &self.skills_receiver {
            match receiver.try_recv() {
                Ok(Ok(skills)) => {
                    if !self.selected_skill.as_ref().is_some_and(|selected| {
                        skills.skills.iter().any(|skill| &skill.name == selected)
                    }) {
                        self.selected_skill = skills.skills.first().map(|skill| skill.name.clone());
                    }
                    self.skills = Some(skills);
                    self.skills_loading = false;
                    self.skills_error = None;
                    self.skills_receiver = None;
                }
                Ok(Err(error)) => {
                    self.skills_loading = false;
                    self.skills_error = Some(format!("Could not load skills: {error}"));
                    self.skills_receiver = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.skills_loading = false;
                    self.skills_error = Some("Skill list worker stopped unexpectedly".into());
                    self.skills_receiver = None;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    self.repaint_soon = true;
                }
            }
        }
        if let Some(receiver) = &self.skills_write_receiver {
            match receiver.try_recv() {
                Ok(Ok(applied)) => {
                    self.skills_status = Some(if applied.persisted {
                        format!("Disabled skills: {}", applied.disabled_skills.join(", "))
                    } else {
                        "Applied for this Host process only; no settings entry owns the set".to_owned()
                    });
                    self.skills_write_receiver = None;
                    self.refresh_skills();
                }
                Ok(Err(error)) => {
                    self.skills_status = Some(format!("Skill change refused: {error}"));
                    self.skills_write_receiver = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.skills_status = Some("Skill change worker stopped unexpectedly".into());
                    self.skills_write_receiver = None;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    self.repaint_soon = true;
                }
            }
        }
        if let Some(receiver) = &self.chat_receiver {
            match receiver.try_recv() {
                Ok(Ok(frame)) => match frame {
                    FollowFrame::Snapshot(snapshot) => {
                        self.transcript.apply_snapshot(&snapshot.records);
                        self.deliverables.clear();
                        for record in &snapshot.records {
                            self.record_deliverables(&record.event);
                        }
                    }
                    FollowFrame::Event(frame) => {
                        if frame.event.event_type == "assistant/message" {
                            self.assistant_draft.clear();
                        }
                        self.record_deliverables(&frame.event);
                        self.transcript.apply(&frame.event);
                    }
                    FollowFrame::AssistantStream { frame } => {
                        use dsh_client_core::follow::AssistantStreamFrame;
                        match &frame {
                            AssistantStreamFrame::Start { .. } => {
                                self.assistant_streaming = true;
                                self.assistant_draft.clear();
                            }
                            AssistantStreamFrame::Chunk { chunk, .. } => {
                                append_stream_text(&mut self.assistant_draft, chunk);
                            }
                            AssistantStreamFrame::End { .. } => {
                                self.assistant_streaming = false;
                                self.refresh_sessions_quietly();
                            }
                        }
                    }
                },
                Ok(Err(error)) => {
                    self.page_error = Some(format!("Chat stream failed: {error}"));
                    self.chat_loading = false;
                    self.chat_receiver = None;
                    self.refresh_sessions_quietly();
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.chat_loading = false;
                    self.chat_receiver = None;
                    self.refresh_sessions_quietly();
                }
                Err(mpsc::TryRecvError::Empty) => {
                    self.repaint_soon = true;
                }
            }
        }
        if let Some(receiver) = &self.approval_events_receiver {
            match receiver.try_recv() {
                Ok(Ok(RemoteEvent::Ready { .. })) => {
                    self.status = "Connected — permissions are listening".to_owned();
                }
                Ok(Ok(RemoteEvent::Waterfall(invocation))) => {
                    if invocation.event == "approval/request" {
                        match PendingApproval::from_invocation(invocation) {
                            Ok(pending) => {
                                // A reconnect can redeliver the same ask; one card per
                                // audit id keeps a second Allow from answering nothing.
                                let duplicate = pending.request.request_id.as_deref().is_some_and(
                                    |id| {
                                        self.pending_approvals.iter().any(|queued| {
                                            queued.request.request_id.as_deref() == Some(id)
                                        })
                                    },
                                );
                                if !duplicate {
                                    self.pending_approvals.push(pending);
                                }
                            }
                            Err(error) => {
                                self.page_error = Some(format!("Invalid approval request: {error}"))
                            }
                        }
                    } else if invocation.event == "user-questions/request" {
                        match PendingQuestion::from_invocation(invocation) {
                            Ok(pending) => self.pending_questions.push(QuestionCard::new(pending)),
                            Err(error) => {
                                self.page_error = Some(format!("Invalid question request: {error}"))
                            }
                        }
                    } else if let Some(commands) = &self.approval_commands {
                        let _ = commands.send(ApprovalCommand::Delegate(invocation));
                    }
                }
                Ok(Ok(RemoteEvent::Cancel { event_id })) => {
                    self.pending_approvals
                        .retain(|pending| pending.invocation.event_id != event_id);
                    self.pending_questions.retain(|card| {
                        card.pending()
                            .is_some_and(|pending| pending.invocation.event_id != event_id)
                    });
                }
                Ok(Ok(RemoteEvent::Emit { .. })) => {}
                Ok(Err(error)) => {
                    self.page_error = Some(format!("Permission event stream failed: {error}"));
                    self.approval_events_receiver = None;
                    self.approval_commands = None;
                    self.pending_approvals.clear();
                    self.pending_questions.clear();
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.status = "Permission event stream closed".to_owned();
                    self.approval_events_receiver = None;
                    self.approval_commands = None;
                    self.pending_approvals.clear();
                    self.pending_questions.clear();
                }
                Err(mpsc::TryRecvError::Empty) => {
                    self.repaint_soon = true;
                }
            }
        }
        if self
            .show_requested
            .swap(false, std::sync::atomic::Ordering::AcqRel)
        {
            window.activate_window();
            self.hidden.store(false, Ordering::Release);
        }
        if self
            .hide_requested
            .swap(false, std::sync::atomic::Ordering::AcqRel)
        {
            window.minimize_window();
            self.hidden.store(true, Ordering::Release);
        }
            if self.repaint_soon || has_pending_workers(self) {
            cx.notify();
        }
        !self.close_requested
    }

}

fn catalog_model<'a>(
    catalog: &'a ModelCatalog,
    selection: &ModelSelection,
) -> Option<&'a ModelCatalogModel> {
    catalog
        .groups
        .iter()
        .filter(|group| group.id == selection.provider)
        .flat_map(|group| group.models.iter())
        .find(|model| model.id == selection.model)
}

/// Current Unix time in milliseconds.
fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| {
            i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
        })
}

/// Appends or refreshes the files a `deliverables/presented` event declared.
fn record_deliverables(
    deliverables: &mut Vec<DeliverableFile>,
    event: &dsh_client_core::follow::SessionWireEvent,
) {
    if event.event_type != "deliverables/presented" {
        return;
    }
    let Some(files) = event
        .data
        .get("files")
        .and_then(serde_json::Value::as_array)
    else {
        return;
    };
    for file in files {
        let Some(path) = file.get("path").and_then(serde_json::Value::as_str) else {
            continue;
        };
        let description = file
            .get("description")
            .and_then(serde_json::Value::as_str)
            .filter(|text| !text.is_empty())
            .map(str::to_owned);
        match deliverables.iter_mut().find(|entry| entry.path == path) {
            Some(entry) => entry.description = description,
            None => deliverables.push(DeliverableFile {
                path: path.to_owned(),
                description,
            }),
        }
    }
}

/// Appends one assistant-stream text delta, bounded to the transcript budget.
fn append_stream_text(draft: &mut String, chunk: &serde_json::Value) {
    if chunk.get("type").and_then(serde_json::Value::as_str) != Some("text-delta") {
        return;
    }
    let Some(text) = chunk.get("text").and_then(serde_json::Value::as_str) else {
        return;
    };
    if draft.len() + text.len() > transcript::MAX_BYTES {
        return;
    }
    draft.push_str(text);
}

/// Largest image preview read; larger files are rejected instead of fetched.
/// Path matches one palette query asks the Host for; the palette shows eight.
const PALETTE_PATH_RESULTS: u64 = 20;

/** Result of the document page's rename or delete action. */
pub(crate) enum DocumentMutation {
    Renamed(dsh_client_core::workspace_files::WorkspaceFileRenameResult),
    Deleted,
}

const PREVIEW_IMAGE_LIMIT: usize = 16 * 1024 * 1024;
/// Largest decoded image side accepted by the preview.
const PREVIEW_IMAGE_MAX_DIMENSION: u32 = 8192;

/// True for extensions the preview decodes as an image.
fn is_image_path(path: &str) -> bool {
    let Some((_, extension)) = path.rsplit_once('.') else {
        return false;
    };
    let extension = extension.to_ascii_lowercase();
    matches!(
        extension.as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "ico"
    )
}

/// Reads and decodes one workspace image for display.
fn read_image(
    client: &WorkspaceFilesClient<'_>,
    scope: WorkspaceFileScope,
    path: &str,
) -> Result<PreviewDocument, String> {
    let bytes = client
        .read_bytes(
            scope,
            path,
            WorkspaceByteReadOptions {
                range: Some(WorkspaceByteRange {
                    offset: None,
                    length: Some(PREVIEW_IMAGE_LIMIT as u64),
                }),
                base_file: None,
            },
        )
        .map_err(|error| error.to_string())?;
    if !bytes.eof {
        return Err(format!(
            "image is larger than {} MiB",
            PREVIEW_IMAGE_LIMIT / (1024 * 1024)
        ));
    }
    let name = bytes
        .stat
        .absolute_path
        .rsplit('/')
        .next()
        .unwrap_or(path)
        .to_owned();
    decode_preview_image(name, bytes.stat.absolute_path, &bytes.data)
}

/// Decodes exact image bytes into a preview document.
fn decode_preview_image(
    name: String,
    host_path: String,
    data: &[u8],
) -> Result<PreviewDocument, String> {
    let decoded =
        image::load_from_memory(data).map_err(|error| format!("image decoding failed: {error}"))?;
    if decoded.width() > PREVIEW_IMAGE_MAX_DIMENSION
        || decoded.height() > PREVIEW_IMAGE_MAX_DIMENSION
    {
        return Err(format!(
            "image exceeds {PREVIEW_IMAGE_MAX_DIMENSION} pixels on a side"
        ));
    }
    let rgba = decoded.to_rgba8();
    Ok(PreviewDocument::Image(ImageDocument {
        name,
        host_path,
        size: [rgba.width() as usize, rgba.height() as usize],
        rgba: rgba.into_raw(),
    }))
}

fn model_picker_label(
    selection: Option<&ModelSelection>,
    error: Option<&str>,
    catalog: Option<&ModelCatalog>,
    pending: bool,
) -> String {
    if let Some(selection) = selection {
        return format!("{} / {}", selection.provider, selection.model);
    }
    if error.is_some() {
        return "Model unavailable".to_owned();
    }
    if pending {
        return "Model: loading…".to_owned();
    }
    match catalog {
        Some(catalog) if catalog.groups.iter().all(|group| group.models.is_empty()) => {
            "No provider configured".to_owned()
        }
        Some(_) => "Choose a model".to_owned(),
        None => "No provider configured".to_owned(),
    }
}

/// Model the probe requests: the DeepSeek API-key route when it exists, else the catalog default.
fn probe_model_selection(catalog: &ModelCatalog) -> Option<ModelSelection> {
    catalog
        .groups
        .iter()
        .find(|group| group.id == "deepseek-official")
        .and_then(|group| group.models.first())
        .map(|model| ModelSelection {
            provider: "deepseek-official".to_owned(),
            model: model.id.clone(),
            reasoning_effort: model
                .reasoning
                .as_ref()
                .and_then(|reasoning| reasoning.default_effort.clone()),
        })
        .or_else(|| {
            catalog.groups.iter().find_map(|group| {
                group.models.first().map(|model| ModelSelection {
                    provider: group.id.clone(),
                    model: model.id.clone(),
                    reasoning_effort: model
                        .reasoning
                        .as_ref()
                        .and_then(|reasoning| reasoning.default_effort.clone()),
                })
            })
        })
}

fn settings_write_error(error: &Error) -> String {
    match error {
        Error::Remote { code, .. } if code == "SETTINGS_CONFLICT" =>
            "Settings conflict: another edit changed this namespace. Draft retained; refresh settings, discard the draft, and review current values before editing again.".into(),
        Error::Remote { .. } =>
            "Save failed. Draft retained; refresh settings to confirm remote values and review the schema before editing again. No automatic retry was made.".into(),
        _ => "Save outcome unknown. Draft retained; refresh settings to confirm remote values before discarding the draft and editing again. No automatic retry was made.".into(),
    }
}

/// True while any worker channel can still deliver state to the window.
fn has_pending_workers(app: &DesktopApp) -> bool {
    app.host_receiver.is_some()
        || app.connect_receiver.is_some()
        || app.sessions_receiver.is_some()
        || app.archive_receiver.is_some()
        || app.pin_receiver.is_some()
        || app.workspace_receiver.is_some()
        || app.page_receiver.is_some()
        || app.chat_receiver.is_some()
        || app.cancel_receiver.is_some()
        || app.approval_events_receiver.is_some()
        || app.dialog.is_some()
        || app.attachment_receiver.is_some()
        || app.model_receiver.is_some()
        || app.model_select_receiver.is_some()
        || app.model_init_receiver.is_some()
        || app.plugins_receiver.is_some()
        || app.plugin_write_receiver.is_some()
        || app.feedback_receiver.is_some()
        || app.feedback_put_receiver.is_some()
        || app.market_receiver.is_some()
        || app.skills_receiver.is_some()
        || app.skills_write_receiver.is_some()
        || app.settings_receiver.is_some()
        || app.settings_update_receiver.is_some()
        || app.update_open_receiver.is_some()
        || app.preview_receiver.is_some()
        || app.goal_receiver.is_some()
        || app.goal_mutation_receiver.is_some()
        || app.plan_command_receiver.is_some()
        || app.jobs_receiver.is_some()
        || app.job_receiver.is_some()
        || app.schedules_receiver.is_some()
        || app.schedule_delete_receiver.is_some()
        || app.handoff_receiver.is_some()
        || app.file_search_receiver.is_some()
        || app.document_op_receiver.is_some()
        || app.palette_paths_receiver.is_some()
        || app.workspace_create_receiver.is_some()
        || app.terminal.inventory.is_some()
        || app.terminal.updates.is_some()
        || app.terminal.dirty
        || app
            .terminal_extra
            .iter()
            .any(|pane| pane.inventory.is_some() || pane.updates.is_some() || pane.dirty)

}


/// Global preferences with the selected workspace's appearance overrides applied.
///
/// An unset override inherits the global value, so resolution reads one plain
/// preferences value for the frame's scheme, density, and pack.
fn effective_preferences(preferences: &UiPreferences, workspace: Option<&str>) -> UiPreferences {
    let mut effective = preferences.clone();
    let overrides = preferences.workspace_prefs(workspace);
    if let Some(theme) = overrides.theme {
        effective.theme = theme;
    }
    if let Some(density) = overrides.density {
        effective.density = density;
    }
    effective
}

fn appearance_is_dark(appearance: WindowAppearance) -> bool {
    matches!(appearance, WindowAppearance::Dark | WindowAppearance::VibrantDark)
}


fn probe_approval(transport: HttpTransport, mux: RemoteStreamMux) {
    use std::time::{Duration, Instant};
    let marker = match std::env::var("DSH_APPROVAL_PROBE_MARKER") {
        Ok(value) if !value.is_empty() => value,
        _ => {
            eprintln!("dsh: native approval=failed: DSH_APPROVAL_PROBE_MARKER is required");
            return;
        }
    };
    let mode = std::env::var("DSH_DESKTOP_PROBE_APPROVAL").unwrap_or_default();
    let decision = match mode.as_str() {
        "reject" => Some(ApprovalDecision::Rejected),
        "delegate" => None,
        _ => Some(ApprovalDecision::AllowedOnce),
    };
    let session = match SessionsClient::new(&transport).create(CreateSessionRequest {
        workspace_id: None,
        cwd: None,
        session_id: None,
        agent_preset: None,
    }) {
        Ok(session) => session.session_id,
        Err(error) => {
            eprintln!("dsh: native approval=failed: session create: {error}");
            return;
        }
    };
    let model = match ModelClient::new(&transport).catalog() {
        Ok(catalog) => match probe_model_selection(&catalog) {
            Some(selection) => selection,
            None => {
                eprintln!("dsh: native approval=failed: empty model catalog");
                return;
            }
        },
        Err(error) => {
            eprintln!("dsh: native approval=failed: model catalog: {error}");
            return;
        }
    };
    eprintln!(
        "dsh: native approval=model {} / {}",
        model.provider, model.model
    );
    if let Err(error) = ModelClient::new(&transport).select(SelectModelRequest {
        session_id: session.clone(),
        selection: model,
    }) {
        eprintln!("dsh: native approval=failed: select model: {error}");
        return;
    }
    let events = match RemoteEvents::connect(transport.clone(), &mux) {
        Ok(events) => events,
        Err(error) => {
            eprintln!("dsh: native approval=failed: events: {error}");
            return;
        }
    };
    let chat = ChatClient::new(&transport);
    let request = PromptRequest {
        request_id: chat.next_request_id(),
        session_id: session.clone(),
        mode: PromptMode::Queue,
        content: vec![PromptContentPart::Text {
            text: "Run the escalated approval probe command.".to_owned(),
        }],
        client_time_zone: None,
    };
    if let Err(error) = chat.prompt(request) {
        eprintln!("dsh: native approval=failed: prompt: {error}");
        return;
    }
    let deadline = Instant::now() + Duration::from_secs(120);
    let mut asked = false;
    let mut answered = false;
    let mut roundtrip_reported = false;
    while Instant::now() < deadline {
        if !roundtrip_reported && std::path::Path::new(&marker).exists() {
            roundtrip_reported = true;
            eprintln!("dsh: native approval=roundtrip-ok");
            probe_job_follow(&mux, &session);
        }
        match events.recv_timeout(Duration::from_millis(250)) {
            Ok(Some(RemoteEvent::Waterfall(invocation))) => {
                if invocation.event == "user-questions/request" {
                    let answer = user_question_answer(&invocation.request);
                    match events.submit(&invocation, RemoteEventOutcome::Result { value: answer }) {
                        Ok(()) => {
                            eprintln!("dsh: native question=answered");
                            if !roundtrip_reported && std::path::Path::new(&marker).exists() {
                                eprintln!("dsh: native approval=roundtrip-ok");
                                probe_job_follow(&mux, &session);
                            }
                            if probe_question_result(&mux, &session) {
                                eprintln!("dsh: native question=answer-ok");
                                if probe_transcript(&mux, &session) {
                                    eprintln!("dsh: native transcript=ok");
                                } else {
                                    eprintln!("dsh: native transcript=failed");
                                }
                            } else {
                                eprintln!("dsh: native question=failed: no answered result");
                            }
                        }
                        Err(error) => {
                            eprintln!("dsh: native question=failed: {error}");
                        }
                    }
                    return;
                }
                if invocation.event != "approval/request" {
                    continue;
                }
                let pending = match PendingApproval::from_invocation(invocation) {
                    Ok(pending) => pending,
                    Err(error) => {
                        eprintln!("dsh: native approval=failed: decode request: {error}");
                        continue;
                    }
                };
                if !asked {
                    asked = true;
                    eprintln!(
                        "dsh: native approval=asked tool={}",
                        pending.request.tool_name
                    );
                }
                let submitted = match decision {
                    Some(decision) => pending.answer(&events, decision),
                    None => pending.delegate(&events),
                };
                match submitted {
                    Ok(()) => {
                        if !answered {
                            answered = true;
                            match decision {
                                Some(ApprovalDecision::AllowedOnce) => {
                                    eprintln!("dsh: native approval=allowed-once")
                                }
                                Some(ApprovalDecision::Rejected) => {
                                    eprintln!("dsh: native approval=rejected")
                                }
                                None => eprintln!("dsh: native approval=delegated"),
                            }
                        }
                        if decision != Some(ApprovalDecision::AllowedOnce) {
                            let (expected, label) = match decision {
                                Some(ApprovalDecision::Rejected) => ("rejected", "rejected-ok"),
                                _ => ("unavailable", "delegated-ok"),
                            };
                            if probe_decided(&mux, &session, expected) {
                                eprintln!("dsh: native approval={label}");
                            } else {
                                eprintln!(
                                    "dsh: native approval=failed: no {expected} decision logged"
                                );
                            }
                            if std::path::Path::new(&marker).exists() {
                                eprintln!(
                                    "dsh: native approval=failed: command ran despite {expected}"
                                );
                            }
                            return;
                        }
                    }
                    Err(error) => eprintln!("dsh: native approval=answer-failed: {error}"),
                }
            }
            Ok(Some(_)) | Ok(None) => {}
            Err(error) => {
                eprintln!("dsh: native approval=failed: events closed: {error}");
                return;
            }
        }
    }
    eprintln!(
        "dsh: native approval=failed: timeout asked={asked} answered={answered} marker={}",
        std::path::Path::new(&marker).exists()
    );
}

/// Runs only when `DSH_DESKTOP_PROBE_TERMINAL=1`.
///
/// Exercises the terminal path the panel uses against a live Host: create a
/// Session, create a terminal through the RPCs, attach over the shared mux,
/// write one command, and read the streamed output back. The lane's probe reads
/// the `dsh: native terminal=` marker.
fn probe_terminal(transport: HttpTransport, mux: RemoteStreamMux) {
    use std::time::Duration;
    let session = match SessionsClient::new(&transport).create(CreateSessionRequest {
        workspace_id: None,
        cwd: None,
        session_id: None,
        agent_preset: None,
    }) {
        Ok(session) => session.session_id,
        Err(error) => {
            eprintln!("dsh: native terminal=failed: session create: {error}");
            return;
        }
    };
    let client = TerminalClient::new(&transport, session.as_str());
    let environment = match client.environment() {
        Ok(environment) => environment,
        Err(error) => {
            eprintln!("dsh: native terminal=failed: environment: {error}");
            return;
        }
    };
    if let Err(error) = client.create(TerminalCreateRequest {
        shell_path: None,
        id: "probe-terminal".to_owned(),
        cols: 80,
        rows: 24,
    }) {
        eprintln!("dsh: native terminal=failed: create: {error}");
        return;
    }
    let (mut frames, input) = match terminal::attach(&mux, session.as_str(), "probe-terminal") {
        Ok(attachment) => attachment,
        Err(error) => {
            eprintln!("dsh: native terminal=failed: attach: {error}");
            return;
        }
    };
    let command = "printf 'terminal-probe-ok\\n'\n";
    if input
        .send(TerminalInput::Write {
            sequence: 1,
            data: command.to_owned(),
        })
        .is_err()
    {
        eprintln!("dsh: native terminal=failed: input closed");
        return;
    }
    let mut output = String::new();
    let mut exit_code = None;
    while let Some(frame) = frames.recv_timeout(Duration::from_secs(20)) {
        match frame {
            Ok(TerminalFrame::Output(chunk)) => output.push_str(&chunk.data),
            Ok(TerminalFrame::State(state)) => exit_code = state.info.exit_code,
            Ok(_) => {}
            Err(error) => {
                eprintln!("dsh: native terminal=failed: frame: {error}");
                return;
            }
        }
        if output.contains("terminal-probe-ok") {
            break;
        }
    }
    if output.contains("terminal-probe-ok") {
        eprintln!(
            "dsh: native terminal=ok max_input={} max_cols={} exit={}",
            environment.max_input_bytes,
            environment.max_cols,
            exit_code.map(|code| code.to_string()).unwrap_or_default()
        );
    } else {
        eprintln!(
            "dsh: native terminal=failed: no marker in output bytes={}",
            output.len()
        );
    }

    // Detach the first attachment, then confirm the Host retained the PTY and its screen.
    drop(frames);
    drop(input);
    let (mut frames, input) = match terminal::attach(&mux, session.as_str(), "probe-terminal") {
        Ok(attachment) => attachment,
        Err(error) => {
            eprintln!("dsh: native terminal-reconnect=failed: attach: {error}");
            let _ = client.close(TerminalCloseRequest {
                id: "probe-terminal".to_owned(),
            });
            return;
        }
    };
    let mut reattached = String::new();
    let mut retained = false;
    while let Some(frame) = frames.recv_timeout(Duration::from_secs(10)) {
        match frame {
            Ok(TerminalFrame::Snapshot(frame)) => reattached.push_str(&frame.screen),
            Ok(TerminalFrame::Output(chunk)) => reattached.push_str(&chunk.data),
            Ok(_) => {}
            Err(error) => {
                eprintln!("dsh: native terminal-reconnect=failed: frame: {error}");
                break;
            }
        }
        if reattached.contains("terminal-probe-ok") {
            retained = true;
            break;
        }
    }
    let _ = input.send(TerminalInput::Write {
        sequence: 2,
        data: "exit\n".to_owned(),
    });
    let _ = client.close(TerminalCloseRequest {
        id: "probe-terminal".to_owned(),
    });
    if retained {
        eprintln!(
            "dsh: native terminal-reconnect=ok bytes={}",
            reattached.len()
        );
    } else {
        eprintln!(
            "dsh: native terminal-reconnect=failed: marker missing after reattach bytes={}",
            reattached.len()
        );
    }
}

/// Runs only when `DSH_DESKTOP_PROBE_TIMED_QUESTION=1`.
///
/// Answers a timed `ask_user_question` the way the app does, but also proves the
/// foreground wait stream the question card subscribes to. The lane's probe
/// reads the `dsh: native timed-question=` markers.
fn probe_timed_question(transport: HttpTransport, mux: RemoteStreamMux) {
    use std::time::{Duration, Instant};
    let session = match SessionsClient::new(&transport).create(CreateSessionRequest {
        workspace_id: None,
        cwd: None,
        session_id: None,
        agent_preset: None,
    }) {
        Ok(session) => session.session_id,
        Err(error) => {
            eprintln!("dsh: native timed-question=failed: session create: {error}");
            return;
        }
    };
    let model = match ModelClient::new(&transport).catalog() {
        Ok(catalog) => match probe_model_selection(&catalog) {
            Some(selection) => selection,
            None => {
                eprintln!("dsh: native timed-question=failed: empty model catalog");
                return;
            }
        },
        Err(error) => {
            eprintln!("dsh: native timed-question=failed: model catalog: {error}");
            return;
        }
    };
    if let Err(error) = ModelClient::new(&transport).select(SelectModelRequest {
        session_id: session.clone(),
        selection: model,
    }) {
        eprintln!("dsh: native timed-question=failed: select model: {error}");
        return;
    }
    let events = match RemoteEvents::connect(transport.clone(), &mux) {
        Ok(events) => events,
        Err(error) => {
            eprintln!("dsh: native timed-question=failed: events: {error}");
            return;
        }
    };
    let chat = ChatClient::new(&transport);
    let request = PromptRequest {
        request_id: chat.next_request_id(),
        session_id: session.clone(),
        mode: PromptMode::Queue,
        content: vec![PromptContentPart::Text {
            text: "Ask the timed probe question.".to_owned(),
        }],
        client_time_zone: None,
    };
    if let Err(error) = chat.prompt(request) {
        eprintln!("dsh: native timed-question=failed: prompt: {error}");
        return;
    }
    let deadline = Instant::now() + Duration::from_secs(120);
    while Instant::now() < deadline {
        match events.recv_timeout(Duration::from_millis(250)) {
            Ok(Some(RemoteEvent::Waterfall(invocation))) => {
                if invocation.event != "user-questions/request" {
                    continue;
                }
                let pending = match PendingQuestion::from_invocation(invocation) {
                    Ok(pending) => pending,
                    Err(error) => {
                        eprintln!("dsh: native timed-question=failed: decode: {error}");
                        return;
                    }
                };
                let Some(wait) = pending.request.wait.clone() else {
                    eprintln!("dsh: native timed-question=failed: blocking question");
                    return;
                };
                if !wait.timed {
                    eprintln!("dsh: native timed-question=failed: not timed");
                    return;
                }
                let Some(call_id) = wait.call_id.clone() else {
                    eprintln!("dsh: native timed-question=failed: no call id");
                    return;
                };
                let mut frames = 0_u32;
                let mut remaining = 0_u64;
                match streams::user_question_wait(&mux, session.as_str(), &call_id) {
                    Ok(mut stream) => {
                        while let Some(frame) = stream.recv_timeout(Duration::from_secs(5)) {
                            match frame {
                                Ok(frame) => {
                                    frames += 1;
                                    remaining = remaining.max(frame.remaining_ms);
                                }
                                Err(error) => {
                                    eprintln!(
                                        "dsh: native timed-question=failed: wait frame: {error}"
                                    );
                                    break;
                                }
                            }
                        }
                    }
                    Err(error) => {
                        eprintln!("dsh: native timed-question=failed: attach wait: {error}");
                        return;
                    }
                }
                eprintln!("dsh: native timed-question=attached frames={frames}");
                if frames == 0 {
                    eprintln!("dsh: native timed-question=failed: no wait frames");
                    return;
                }
                // The foreground wait ends at the model's timeout; the answer then
                // arrives as a continued-question reply.
                std::thread::sleep(Duration::from_secs(3));
                let answer = AskUserQuestionAnswer {
                    answers: pending
                        .request
                        .questions
                        .iter()
                        .map(|question| AskUserQuestionAnswerItem {
                            id: question.id.clone(),
                            selected: Vec::new(),
                            custom: Some("timed probe answer".to_owned()),
                        })
                        .collect(),
                };
                match UserQuestionsClient::new(&transport).answer(&session, &call_id, answer) {
                    Ok(true) => eprintln!(
                        "dsh: native timed-question=ok frames={frames} remaining_ms={remaining}"
                    ),
                    Ok(false) => eprintln!("dsh: native timed-question=failed: not answerable"),
                    Err(error) => {
                        eprintln!("dsh: native timed-question=failed: answer: {error}");
                    }
                }
                return;
            }
            Ok(Some(_)) | Ok(None) => {}
            Err(error) => {
                eprintln!("dsh: native timed-question=failed: events closed: {error}");
                return;
            }
        }
    }
    eprintln!("dsh: native timed-question=failed: timeout");
}

/// Reads the current session snapshot, retrying after a failed open.
fn probe_snapshot(mux: &RemoteStreamMux, session: &str) -> Option<follow::SnapshotFrame> {
    let request = FollowRequest {
        address: SessionAddress {
            kind: SessionAddressKind::Session,
            session_id: session.to_owned(),
        },
        max_messages: Some(200),
        turn_window: None,
        assistant_stream: false,
    };
    let stream = follow::follow(mux, request).ok()?;
    for frame in stream {
        match frame {
            Ok(FollowFrame::Snapshot(snapshot)) => return Some(snapshot),
            Ok(_) => {}
            Err(_) => return None,
        }
    }
    None
}

/// Waits for the audit event the Host appends after a decision.
fn probe_decided(mux: &RemoteStreamMux, session: &str, expected: &str) -> bool {
    use std::time::{Duration, Instant};
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        if let Some(snapshot) = probe_snapshot(mux, session) {
            for entry in &snapshot.records {
                let event = &entry.event;
                if event.event_type == "approval/decided"
                    && event
                        .data
                        .get("outcome")
                        .and_then(serde_json::Value::as_str)
                        == Some(expected)
                {
                    return true;
                }
            }
        }
        std::thread::sleep(Duration::from_millis(300));
    }
    false
}

/// Builds an answer that selects the first option of each asked question.
fn user_question_answer(
    request: &serde_json::Map<String, serde_json::Value>,
) -> AskUserQuestionAnswer {
    let answers = request
        .get("questions")
        .and_then(serde_json::Value::as_array)
        .map(|questions| {
            questions
                .iter()
                .filter_map(|question| {
                    let id = question
                        .get("id")
                        .and_then(serde_json::Value::as_str)?
                        .to_owned();
                    let selected = question
                        .get("options")
                        .and_then(serde_json::Value::as_array)
                        .and_then(|options| options.first())
                        .and_then(|option| option.get("label"))
                        .and_then(serde_json::Value::as_str)
                        .map(|label| vec![label.to_owned()])
                        .unwrap_or_default();
                    Some(AskUserQuestionAnswerItem {
                        id,
                        selected,
                        custom: None,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    AskUserQuestionAnswer { answers }
}

/// Waits for the answered tool result in the durable log.
fn probe_question_result(mux: &RemoteStreamMux, session: &str) -> bool {
    use std::time::{Duration, Instant};
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        if let Some(snapshot) = probe_snapshot(mux, session) {
            for entry in &snapshot.records {
                let event = &entry.event;
                if event.event_type == "tool/result" && event.data.to_string().contains("Yes") {
                    return true;
                }
            }
        }
        std::thread::sleep(Duration::from_millis(300));
    }
    false
}

/// Projects the live session log into transcript rows and checks the kinds.
fn probe_transcript(mux: &RemoteStreamMux, session: &str) -> bool {
    let Some(snapshot) = probe_snapshot(mux, session) else {
        eprintln!("dsh: native transcript=failed: no snapshot");
        return false;
    };
    let mut transcript = Transcript::default();
    transcript.apply_snapshot(&snapshot.records);
    let rows = transcript.rows();
    let has = |kind| rows.iter().any(|row| row.kind == kind);
    let ok = has(RowKind::User)
        && has(RowKind::Assistant)
        && has(RowKind::ToolResult)
        && has(RowKind::Approval)
        && has(RowKind::Job)
        && rows.len() >= 5;
    eprintln!(
        "dsh: native transcript rows={} user={} assistant={} tool={} approval={} job={}",
        rows.len(),
        has(RowKind::User),
        has(RowKind::Assistant),
        has(RowKind::ToolResult),
        has(RowKind::Approval),
        has(RowKind::Job)
    );
    ok
}

/// Follows the background job the mock model starts after the approved turn./// Follows the background job the mock model starts after the approved turn.
fn probe_job_follow(mux: &RemoteStreamMux, session: &str) {
    use std::time::{Duration, Instant};
    let mut list = match streams::job_list(mux, session) {
        Ok(list) => list,
        Err(error) => {
            eprintln!("dsh: native job=failed: list open: {error}");
            return;
        }
    };
    let deadline = Instant::now() + Duration::from_secs(45);
    let mut job_id = None;
    while Instant::now() < deadline {
        match list.recv_timeout(Duration::from_millis(250)) {
            Some(Ok(frame)) => {
                if let Some(job) = frame.jobs.first() {
                    job_id = Some(job.id.clone());
                    break;
                }
            }
            Some(Err(error)) => {
                eprintln!("dsh: native job=failed: list: {error}");
                return;
            }
            None => {}
        }
    }
    let Some(job_id) = job_id else {
        eprintln!("dsh: native job=failed: no job listed");
        return;
    };
    let request = streams::JobFollowRequest {
        session_id: Some(session.to_owned()),
        job_id,
        from: Some(0),
    };
    let mut follow = match streams::job_follow(mux, request) {
        Ok(follow) => follow,
        Err(error) => {
            eprintln!("dsh: native job=failed: follow open: {error}");
            return;
        }
    };
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut output = String::new();
    while Instant::now() < deadline {
        match follow.recv_timeout(Duration::from_millis(250)) {
            Some(Ok(JobFollowFrame::Output { chunks, .. })) => {
                for chunk in chunks {
                    output.push_str(&chunk.text);
                }
                if output.contains("job-output-ok") {
                    eprintln!("dsh: native job=follow-ok");
                    return;
                }
            }
            Some(Ok(_)) => {}
            Some(Err(error)) => {
                eprintln!("dsh: native job=failed: follow: {error}");
                return;
            }
            None => {}
        }
    }
    eprintln!("dsh: native job=failed: output marker missing");
}

/// Reports one probed stream: its opening frame, its failure, or a timeout.
fn probe_first<T: serde::de::DeserializeOwned>(
    endpoint: &str,
    opened: Result<FrameStream<T>, Error>,
    timeout: std::time::Duration,
) {
    match opened {
        Ok(mut stream) => match stream.recv_timeout(timeout) {
            Some(Ok(_)) => eprintln!("dsh: native stream {endpoint}=ok"),
            Some(Err(error)) => eprintln!("dsh: native stream {endpoint}=failed: {error}"),
            None => {
                eprintln!("dsh: native stream {endpoint}=failed: no frame before probe timeout")
            }
        },
        Err(error) => eprintln!("dsh: native stream {endpoint}=failed: {error}"),
    }
}

/// Opens the stream endpoints that need no live id and reports their first frame.
///
/// Runs only when `DSH_DESKTOP_PROBE_STREAMS=1`; the lane's GUI probe reads the
/// `dsh: native stream` lines to prove the endpoints on a real Host.
fn probe_streams(transport: HttpTransport, mux: RemoteStreamMux) {
    use std::time::Duration;
    let timeout = Duration::from_secs(20);
    probe_first("workspace/follow", streams::workspace_follow(&mux), timeout);
    probe_first("session/control", streams::session_control(&mux), timeout);
    probe_first("account/watch", streams::account_watch(&mux), timeout);
    probe_first(
        "remoteAccess/follow",
        streams::remote_access_follow(&mux),
        timeout,
    );
    // The expiry stream carries no opening frame; opening it is the whole check.
    match streams::account_watch_expiry(&mux) {
        Ok(_) => eprintln!("dsh: native stream account/watchExpiry=ok"),
        Err(error) => eprintln!("dsh: native stream account/watchExpiry=failed: {error}"),
    }
    let session_id = match SessionViewer::new(&transport).list(None) {
        Ok(items) if !items.is_empty() => Some(items[0].session_id.clone()),
        Ok(_) => match SessionsClient::new(&transport).create(CreateSessionRequest {
            workspace_id: None,
            cwd: None,
            session_id: None,
            agent_preset: None,
        }) {
            Ok(session) => Some(session.session_id),
            Err(error) => {
                eprintln!("dsh: native stream job/list=failed: session create: {error}");
                None
            }
        },
        Err(error) => {
            eprintln!("dsh: native stream job/list=failed: {error}");
            None
        }
    };
    match &session_id {
        Some(id) => {
            probe_first("job/list", streams::job_list(&mux, id), timeout);
            let scope = WorkspaceFileScope {
                session_id: id.clone(),
            };
            probe_first(
                "workspaceFiles/changes",
                streams::workspace_file_changes(&mux, &scope, "cordis.yml"),
                timeout,
            );
            let request = ScheduleListRequest {
                session_id: id.clone(),
            };
            match SchedulesClient::new(&transport).list(request) {
                Ok(records) => eprintln!("dsh: native schedules={}", records.len()),
                Err(error) => eprintln!("dsh: native schedules=failed: {error}"),
            }
        }
        None => {
            eprintln!("dsh: native stream job/list=skipped: no sessions");
            eprintln!("dsh: native stream workspaceFiles/changes=skipped: no sessions");
        }
    }
    for endpoint in ["job/follow", "userQuestions/attachWait"] {
        eprintln!("dsh: native stream {endpoint}=skipped: needs a live id");
    }
    match PluginsClient::new(&transport).list() {
        Ok(snapshot) => eprintln!("dsh: native plugins={}", snapshot.entries.len()),
        Err(error) => eprintln!("dsh: native plugins=failed: {error}"),
    }
    match ModelClient::new(&transport).catalog() {
        Ok(catalog) => eprintln!(
            "dsh: native models={} failures={}",
            catalog
                .groups
                .iter()
                .map(|group| group.models.len())
                .sum::<usize>(),
            catalog.failures.len()
        ),
        Err(error) => eprintln!("dsh: native models=failed: {error}"),
    }
}

/// Maps a screen position inside the terminal grid to a one-based cell.
///
/// The raster uses `FontId::monospace(12.0)`; pointer rows therefore count
/// visual rows, which match program rows for a full-screen alternate buffer.

impl gpui::Render for DesktopApp {
    fn render(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl gpui::IntoElement {
        super::render::render(self, window, cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dsh_client_core::models::ModelProviderGroup;

    #[test]
    fn on_demand_diagnostics_replace_only_the_same_file() {
        use dsh_client_core::diagnostics::{DiagnosticItem, DiagnosticPosition, DiagnosticRange};
        let item = |uri: &str, message: &str| DiagnosticItem {
            uri: uri.to_owned(),
            range: DiagnosticRange {
                start: DiagnosticPosition { line: 0, character: 0 },
                end: DiagnosticPosition { line: 0, character: 1 },
            },
            severity: "error".to_owned(),
            message: message.to_owned(),
            source: None,
            code: None,
        };
        let mut items = vec![item("file:///w/a.ts", "stale-a"), item("file:///w/b.ts", "keep-b")];
        merge_diagnostics(&mut items, "file:///w/a.ts", vec![item("file:///w/a.ts", "fresh-a")]);
        assert_eq!(items.len(), 2);
        assert!(items.iter().any(|entry| entry.message == "fresh-a"));
        assert!(!items.iter().any(|entry| entry.message == "stale-a"));
        assert!(items.iter().any(|entry| entry.message == "keep-b"));

        // An unresolved URI must not clear unrelated diagnostics.
        merge_diagnostics(&mut items, "", Vec::new());
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn workspace_frames_upsert_remove_order_and_clear_selection() {
        use dsh_client_core::streams::{WorkspaceBaseline, WorkspaceView};
        fn view(id: &str) -> WorkspaceView {
            WorkspaceView {
                workspace_id: id.to_owned(),
                path: format!("/ws/{id}"),
                title: id.to_owned(),
                session_ids: vec![id.to_owned()],
                created_at: String::new(),
                updated_at: String::new(),
            }
        }
        let mut workspaces = Vec::new();
        let mut selected = Some("w2".to_owned());
        let mut archived = Vec::new();
        let mut pinned = Vec::new();
        apply_workspace_frame(
            &mut workspaces,
            &mut selected,
            &mut archived,
            &mut pinned,
            WorkspaceFollowFrame::Baseline {
                value: WorkspaceBaseline {
                    items: vec![view("w1"), view("w2")],
                    archived_session_ids: vec!["a1".to_owned()],
                    pinned_session_ids: Vec::new(),
                },
            },
        );
        assert_eq!(workspaces.iter().map(|item| item.workspace_id.as_str()).collect::<Vec<_>>(), ["w1", "w2"]);
        assert_eq!(archived, ["a1"]);
        apply_workspace_frame(
            &mut workspaces,
            &mut selected,
            &mut archived,
            &mut pinned,
            WorkspaceFollowFrame::Remove { workspace_id: "w2".to_owned() },
        );
        assert_eq!(workspaces.iter().map(|item| item.workspace_id.as_str()).collect::<Vec<_>>(), ["w1"]);
        assert_eq!(selected, None);
    }

    #[test]
    fn workspace_preferences_apply_theme_and_density_overrides() {
        use crate::ui_preferences::{ThemeMode, UiDensity, WorkspacePrefs};

        let mut preferences = UiPreferences::default();
        preferences.theme = ThemeMode::Light;
        preferences.workspace_overrides.insert(
            "w1".to_owned(),
            WorkspacePrefs {
                theme: Some(ThemeMode::Dark),
                density: Some(UiDensity::Compact),
                ..WorkspacePrefs::default()
            },
        );
        let effective = effective_preferences(&preferences, Some("w1"));
        assert_eq!(effective.theme, ThemeMode::Dark);
        assert_eq!(effective.density, UiDensity::Compact);

        let inherited = effective_preferences(&preferences, Some("w2"));
        assert_eq!(inherited.theme, ThemeMode::Light);
        assert_eq!(inherited.density, UiDensity::default());
    }

    #[test]
    fn fork_checkpoint_blank_means_latest_and_only_non_negative_integers_pass() {
        assert_eq!(parse_fork_checkpoint(""), Ok(None));
        assert_eq!(parse_fork_checkpoint("  "), Ok(None));
        assert_eq!(parse_fork_checkpoint("42"), Ok(Some(42)));
        assert_eq!(parse_fork_checkpoint(" 7 "), Ok(Some(7)));
        assert!(parse_fork_checkpoint("-1").is_err());
        assert!(parse_fork_checkpoint("latest").is_err());
        assert!(parse_fork_checkpoint("3.5").is_err());
    }

    #[test]
    fn git_failures_pass_through_actionable_git_messages() {
        let refused = Error::Remote {
            code: "git/path-refused".to_owned(),
            message: "\"a.txt\" has no staged change".to_owned(),
            details: serde_json::Map::new(),
        };
        assert_eq!(failure_text(&refused), "\"a.txt\" has no staged change");
        assert_eq!(short_object_id("0123456789abcdef"), "0123456");
        assert_eq!(short_object_id("abc"), "abc");
    }

    #[test]
    fn remote_git_failures_keep_the_host_message() {
        for code in [
            "git/remote-auth",
            "git/remote-diverged",
            "git/remote-no-upstream",
            "git/remote-failed",
        ] {
            let error = Error::Remote {
                code: code.to_owned(),
                message: format!("{code} detail from git"),
                details: serde_json::Map::new(),
            };
            assert_eq!(failure_text(&error), format!("{code} detail from git"));
        }
    }

    #[test]
    fn checkout_refusals_keep_the_git_message() {
        let message = "checking out \"feature\" failed: error: Your local changes to the following files would be overwritten by checkout: src/a.ts";
        let error = Error::Remote {
            code: "git/checkout-refused".to_owned(),
            message: message.to_owned(),
            details: serde_json::Map::new(),
        };
        assert_eq!(failure_text(&error), message);
    }

    #[test]
    fn git_failures_keep_actionable_host_wording() {
        let not_repository = Error::Remote {
            code: "git/not-repository".to_owned(),
            message: "not a git repository".to_owned(),
            details: serde_json::Map::new(),
        };
        assert_eq!(failure_text(&not_repository), "No repository");
        let unavailable = Error::Remote {
            code: "git/unavailable".to_owned(),
            message: "missing".to_owned(),
            details: serde_json::Map::new(),
        };
        assert_eq!(failure_text(&unavailable), "git is not installed on the Host");
    }

    #[test]
    fn auto_connect_defaults_to_enabled() {
        assert!(auto_connect_enabled(None));
    }

    #[test]
    fn auto_connect_can_be_opted_out() {
        assert!(!auto_connect_enabled(Some("0")));
    }

    #[test]
    fn auto_connect_accepts_explicit_on() {
        assert!(auto_connect_enabled(Some("1")));
    }

    #[test]
    fn office_path_detection_covers_word_extensions() {
        assert!(DesktopApp::is_office_path("docs/report.DOCX"));
        assert!(DesktopApp::is_office_path("docs/sheet.xlsx"));
        assert!(DesktopApp::is_office_path("deck.pptx"));
        assert!(!DesktopApp::is_office_path("docs/readme.txt"));
        assert!(!DesktopApp::is_office_path("docs/noext"));
    }

    fn summary(session_id: &str, updated_at: i64, running: bool) -> SessionSummary {
        SessionSummary {
            agent_available: true,
            format_status: None,
            session_id: session_id.to_owned(),
            updated_at,
            running,
            blank: false,
            parent_session_id: None,
            origin: None,
            cwd: None,
            projections: None,
        }
    }

    #[test]
    fn restore_session_target_opens_the_remembered_session() {
        let sessions = vec![summary("a", 1, false), summary("b", 2, false)];
        assert_eq!(
            restore_session_target(true, Some("a"), &sessions, None),
            RestoreSession::Load("a".to_owned())
        );
    }

    #[test]
    fn restore_session_target_respects_the_setting_and_an_open_session() {
        let sessions = vec![summary("a", 1, false)];
        assert_eq!(
            restore_session_target(false, Some("a"), &sessions, None),
            RestoreSession::None
        );
        assert_eq!(
            restore_session_target(true, Some("a"), &sessions, Some("a")),
            RestoreSession::None
        );
        assert_eq!(
            restore_session_target(true, None, &sessions, None),
            RestoreSession::None
        );
    }

    #[test]
    fn restore_session_target_falls_back_to_the_most_recent_session() {
        let sessions = vec![summary("a", 10, false), summary("b", 30, false)];
        assert_eq!(
            restore_session_target(true, Some("gone"), &sessions, None),
            RestoreSession::Missing {
                last: "gone".to_owned(),
                fallback: Some("b".to_owned()),
            }
        );
        assert_eq!(
            restore_session_target(true, Some("gone"), &[], None),
            RestoreSession::Missing {
                last: "gone".to_owned(),
                fallback: None,
            }
        );
    }

    #[test]
    fn session_drafts_are_kept_per_session_and_cleared_when_empty() {
        let mut drafts = std::collections::BTreeMap::new();
        assert!(remember_draft(&mut drafts, "a", "first"));
        assert!(!remember_draft(&mut drafts, "a", "first"));
        assert!(remember_draft(&mut drafts, "b", "second"));
        assert_eq!(drafts.get("a").map(String::as_str), Some("first"));
        assert_eq!(drafts.get("b").map(String::as_str), Some("second"));
        assert!(remember_draft(&mut drafts, "a", ""));
        assert!(!drafts.contains_key("a"));
        assert!(!remember_draft(&mut drafts, "missing", ""));
    }

    #[test]
    fn run_badge_reads_only_the_selected_host_summary() {
        let sessions = vec![summary("a", 1, false), summary("b", 2, true)];
        assert!(session_is_running(&sessions, Some("b")));
        assert!(!session_is_running(&sessions, Some("a")));
        assert!(!session_is_running(&sessions, None));
        assert!(!session_is_running(&sessions, Some("missing")));
    }

    #[test]
    fn image_path_detection_covers_raster_extensions() {
        assert!(is_image_path("assets/logo.png"));
        assert!(is_image_path("assets/photo.JPG"));
        assert!(!is_image_path("docs/report.docx"));
        assert!(!is_image_path("archive.tar.gz"));
        assert!(!is_image_path("noext"));
    }

    #[test]
    fn stream_text_delta_appends_only_text_deltas() {
        let mut draft = String::new();
        append_stream_text(&mut draft, &serde_json::json!({ "type": "text-delta", "text": "Hel" }));
        assert_eq!(draft, "Hel");
        append_stream_text(&mut draft, &serde_json::json!({ "type": "other", "text": "lo" }));
        assert_eq!(draft, "Hel");
        append_stream_text(&mut draft, &serde_json::json!({ "type": "text-delta" }));
        assert_eq!(draft, "Hel");
        append_stream_text(&mut draft, &serde_json::json!({ "type": "text-delta", "text": "lo" }));
        assert_eq!(draft, "Hello");
    }

    #[test]
    fn stream_text_delta_respects_transcript_budget() {
        let mut draft = String::new();
        let big_text = "x".repeat(crate::transcript::MAX_BYTES + 1);
        let big_delta = serde_json::Value::Object(
            [
                ("type".to_owned(), serde_json::Value::String("text-delta".to_owned())),
                ("text".to_owned(), serde_json::Value::String(big_text)),
            ]
            .into_iter()
            .collect(),
        );
        append_stream_text(&mut draft, &big_delta);
        assert!(draft.is_empty());
    }

    #[test]
    fn now_millis_is_a_real_timestamp() {
        assert!(now_millis() > 0);
    }

    #[test]
    fn model_picker_label_names_the_selected_model() {
        let selection = ModelSelection {
            provider: "deepseek-official".to_owned(),
            model: "gpt-5.6".to_owned(),
            reasoning_effort: None,
        };
        assert_eq!(
            model_picker_label(Some(&selection), None, None, false),
            "deepseek-official / gpt-5.6"
        );
    }

    #[test]
    fn model_picker_label_reports_each_blocker_in_order() {
        assert_eq!(model_picker_label(None, Some("boom"), None, false), "Model unavailable");
        assert_eq!(model_picker_label(None, None, None, true), "Model: loading…");
        assert_eq!(model_picker_label(None, None, None, false), "No provider configured");
        let catalog = ModelCatalog {
            default: ModelSelection {
                provider: "deepseek-official".to_owned(),
                model: "gpt-5.6".to_owned(),
                reasoning_effort: None,
            },
            routable_providers: vec!["deepseek-official".to_owned()],
            groups: vec![ModelProviderGroup {
                id: "deepseek-official".to_owned(),
                name: "DeepSeek".to_owned(),
                models: vec![ModelCatalogModel {
                    id: "gpt-5.6".to_owned(),
                    name: "GPT-5.6".to_owned(),
                    description: None,
                    reasoning: None,
                }],
            }],
            failures: vec![],
        };
        assert_eq!(model_picker_label(None, None, Some(&catalog), false), "Choose a model");
    }

    #[test]
    fn probe_model_selection_prefers_deepseek_official() {
        let catalog = ModelCatalog {
            default: ModelSelection {
                provider: "deepseek-official".to_owned(),
                model: "gpt-5.6".to_owned(),
                reasoning_effort: None,
            },
            routable_providers: vec![],
            groups: vec![ModelProviderGroup {
                id: "deepseek-official".to_owned(),
                name: "DeepSeek".to_owned(),
                models: vec![ModelCatalogModel {
                    id: "gpt-5.6".to_owned(),
                    name: "GPT-5.6".to_owned(),
                    description: None,
                    reasoning: None,
                }],
            }],
            failures: vec![],
        };
        let selection = probe_model_selection(&catalog);
        assert_eq!(selection.as_ref().map(|selection| selection.model.clone()), Some("gpt-5.6".to_owned()));
    }

    #[test]
    fn catalog_model_finds_the_requested_model() {
        let catalog = ModelCatalog {
            default: ModelSelection {
                provider: "deepseek-official".to_owned(),
                model: "gpt-5.6".to_owned(),
                reasoning_effort: None,
            },
            routable_providers: vec![],
            groups: vec![ModelProviderGroup {
                id: "deepseek-official".to_owned(),
                name: "DeepSeek".to_owned(),
                models: vec![ModelCatalogModel {
                    id: "gpt-5.6".to_owned(),
                    name: "GPT-5.6".to_owned(),
                    description: None,
                    reasoning: None,
                }],
            }],
            failures: vec![],
        };
        let found = catalog_model(&catalog, &ModelSelection {
            provider: "deepseek-official".to_owned(),
            model: "gpt-5.6".to_owned(),
            reasoning_effort: None,
        });
        assert!(found.is_some());
        let missing = catalog_model(&catalog, &ModelSelection {
            provider: "other-provider".to_owned(),
            model: "missing".to_owned(),
            reasoning_effort: None,
        });
        assert!(missing.is_none());
    }

    #[test]
    fn settings_write_error_reports_the_conflict_branch() {
        let error = Error::Remote {
            code: "SETTINGS_CONFLICT".to_owned(),
            message: "conflict".to_owned(),
            details: Default::default(),
        };
        assert!(settings_write_error(&error).contains("Settings conflict"));
    }

    #[test]
    fn settings_write_error_reports_save_failure() {
        let error = Error::Remote {
            code: "NETWORK".to_owned(),
            message: "offline".to_owned(),
            details: Default::default(),
        };
        assert!(settings_write_error(&error).contains("Save failed"));
    }

    #[test]
    fn settings_write_error_reports_unknown_outcome() {
        let error = Error::Cancelled;
        assert!(settings_write_error(&error).contains("Save outcome unknown"));
    }

    fn text_delta(text: &str) -> serde_json::Value {
        serde_json::Value::Object(
            [
                ("type".to_owned(), serde_json::Value::String("text-delta".to_owned())),
                ("text".to_owned(), serde_json::Value::String(text.to_owned())),
            ]
            .into_iter()
            .collect(),
        )
    }

    #[test]
    fn auto_connect_treats_other_values_as_enabled() {
        assert!(auto_connect_enabled(Some("false")));
        assert!(auto_connect_enabled(Some("")));
    }

    #[test]
    fn stream_text_delta_accepts_exactly_the_budget() {
        let mut draft = String::new();
        append_stream_text(&mut draft, &text_delta(&"x".repeat(crate::transcript::MAX_BYTES)));
        assert_eq!(draft.len(), crate::transcript::MAX_BYTES);
    }

    #[test]
    fn stream_text_delta_rejects_one_byte_over_budget() {
        let mut draft = String::new();
        append_stream_text(
            &mut draft,
            &text_delta(&"x".repeat(crate::transcript::MAX_BYTES + 1)),
        );
        assert!(draft.is_empty());
    }

    #[test]
    fn office_path_detection_requires_a_known_final_extension() {
        assert!(!DesktopApp::is_office_path("report.docx.txt"));
        assert!(DesktopApp::is_office_path("report.txt.docx"));
    }

    fn host_command(name: &str, description: &str) -> CommandDescriptor {
        CommandDescriptor {
            definition_id: None,
            name: name.to_owned(),
            description: description.to_owned(),
            input: None,
        }
    }

    #[test]
    fn palette_host_commands_match_name_or_description() {
        let commands = vec![
            host_command("review", "Review the current diff"),
            host_command("test", "Run the test suite"),
        ];
        assert_eq!(matching_host_commands(&commands, "").len(), 2);
        assert_eq!(
            matching_host_commands(&commands, "diff")
                .iter()
                .map(|command| command.name.as_str())
                .collect::<Vec<_>>(),
            ["review"]
        );
        assert_eq!(matching_host_commands(&commands, "TEST").len(), 1);
        assert!(matching_host_commands(&commands, "deploy").is_empty());
    }

    #[test]
    fn palette_selection_clamps_to_available_rows() {
        assert_eq!(stepped_selection(0, 1, 3), 1);
        assert_eq!(stepped_selection(0, -1, 3), 0);
        assert_eq!(stepped_selection(2, 1, 3), 2);
        assert_eq!(stepped_selection(4, 0, 3), 2);
        assert_eq!(stepped_selection(2, -1, 0), 0);
    }

    #[test]
    fn terminal_input_fields_map_to_pane_indices() {
        assert_eq!(terminal_input_index("terminal-input"), Some(0));
        assert_eq!(terminal_input_index("terminal-input-3"), Some(3));
        assert_eq!(terminal_input_index("terminal-input-4"), None);
        assert_eq!(terminal_input_index("composer"), None);
    }

    #[test]
    fn raw_keystroke_reports_appends_and_deletions() {
        assert_eq!(raw_keystroke("", "a"), ("a".to_owned(), 0));
        assert_eq!(raw_keystroke("ab", "abc"), ("c".to_owned(), 0));
        assert_eq!(raw_keystroke("abc", "ab"), (String::new(), 1));
        assert_eq!(raw_keystroke("abc", "x"), ("x".to_owned(), 3));
        assert_eq!(raw_keystroke("abc", "abd"), ("d".to_owned(), 1));
    }

    #[test]
    fn host_start_status_counts_whole_seconds() {
        use std::time::Duration;
        assert_eq!(host_start_status(Duration::from_millis(0)), "Starting Host sidecar… 0s");
        assert_eq!(host_start_status(Duration::from_millis(2_900)), "Starting Host sidecar… 2s");
        assert_eq!(host_start_status(Duration::from_secs(65)), "Starting Host sidecar… 65s");
    }

    #[test]
    fn git_confirmations_refuse_stale_file_and_branch_state() {
        use dsh_client_core::git::{GitBranches, GitBranch, GitFile, GitStatus};

        let status = GitStatus {
            branch: Some("main".to_owned()),
            commit: Some("abc1234".to_owned()),
            detached: false,
            upstream: None,
            ahead: 0,
            behind: 0,
            files: vec![GitFile {
                path: "a.txt".to_owned(),
                status: "modified".to_owned(),
                staged: false,
                worktree: true,
                orig_path: None,
                stats: None,
            }],
        };
        assert!(discard_confirmation_is_fresh(
            Some(&status),
            "session",
            Some("session"),
            "a.txt",
            "modified",
            false,
            true,
        ));
        assert!(!discard_confirmation_is_fresh(
            Some(&status),
            "session",
            Some("session"),
            "a.txt",
            "modified",
            true,
            true,
        ));

        let branches = GitBranches {
            branches: vec![GitBranch {
                name: "feature".to_owned(),
                commit: "def5678".to_owned(),
                current: false,
            }],
            current: Some("main".to_owned()),
        };
        let expected = status.files.clone();
        assert!(checkout_confirmation_is_fresh(
            Some(&status),
            Some(&branches),
            "session",
            Some("session"),
            &Some("main".to_owned()),
            "feature",
            &Some("abc1234".to_owned()),
            &expected,
        ));

        let mut changed = status.clone();
        changed.files[0].status = "deleted".to_owned();
        assert!(!checkout_confirmation_is_fresh(
            Some(&changed),
            Some(&branches),
            "session",
            Some("session"),
            &Some("main".to_owned()),
            "feature",
            &Some("abc1234".to_owned()),
            &expected,
        ));
    }

    fn untracked_file(path: &str) -> GitFile {
        GitFile {
            path: path.to_owned(),
            status: "untracked".to_owned(),
            staged: false,
            worktree: true,
            orig_path: None,
            stats: None,
        }
    }

    #[test]
    fn stash_requests_name_the_action_index_and_save_options() {
        let save = stash_request(
            GitStashAction::Save,
            None,
            Some("work in progress".to_owned()),
            true,
        );
        assert_eq!(save.action, GitStashAction::Save);
        assert_eq!(save.message.as_deref(), Some("work in progress"));
        assert!(save.include_untracked);
        assert_eq!(save.index, None);

        let blank = stash_request(
            GitStashAction::Save,
            None,
            Some("   ".to_owned()),
            false,
        );
        assert_eq!(blank.message, None);
        assert!(!blank.include_untracked);

        for (action, expected) in [
            (GitStashAction::Apply, "apply"),
            (GitStashAction::Pop, "pop"),
            (GitStashAction::Drop, "drop"),
        ] {
            let request = stash_request(action, Some(3), Some("ignored".to_owned()), true);
            assert_eq!(request.action, action);
            assert_eq!(request.index, Some(3));
            assert_eq!(request.message, None);
            assert!(!request.include_untracked);
            assert_eq!(
                serde_json::to_value(&request).unwrap()["action"],
                serde_json::Value::String(expected.to_owned())
            );
        }

        let listed = stash_request(GitStashAction::List, Some(1), None, false);
        assert_eq!(listed.index, None);
        assert_eq!(listed.message, None);
    }

    #[test]
    fn untracked_delete_refuses_directories_and_non_untracked_paths() {
        assert!(untracked_delete_refusal(&untracked_file("docs/")).is_some());
        assert!(untracked_delete_refusal(&untracked_file("docs/new.md")).is_none());
        let mut modified = untracked_file("docs/new.md");
        modified.status = "modified".to_owned();
        assert!(untracked_delete_refusal(&modified).is_some());
        let mut ignored = untracked_file("docs/new.md");
        ignored.status = "ignored".to_owned();
        assert!(untracked_delete_refusal(&ignored).is_some());
        assert_eq!(untracked_delete_expected_status(), "untracked");
    }

    /// Shell with one Session's repository and a free Git worker.
    fn git_outcome_harness(setup: impl FnOnce(&mut DesktopApp)) -> crate::harness::Harness {
        crate::harness::Harness::open_with(1366., 768., move |app| {
            app.selected_session = Some("git-session".to_owned());
            app.transport = HttpTransport::new("http://127.0.0.1:9").ok();
            app.git_state = Some(GitOperationState {
                operation: "merge".to_owned(),
                conflicts: vec!["a.txt".to_owned(), "b.txt".to_owned()],
            });
            app.git_conflict_view = Some(GitConflictView {
                path: "a.txt".to_owned(),
                conflict: None,
                loading: true,
                error: None,
            });
            app.git_stash = Some(StashPanel {
                entries: Vec::new(),
                truncated: false,
                loading: true,
                error: None,
                summary: None,
                include_untracked: false,
                trigger: ChromeTrigger::GitBranch,
            });
            setup(app);
        })
    }

    #[test]
    fn accepted_conflict_side_closes_the_view_and_clears_the_conflict() {
        let mut harness = git_outcome_harness(|_| {});
        harness.with(|app, _| {
            let (sender, receiver) = mpsc::channel();
            sender
                .send(GitUpdate::Resolve(Ok(GitResolveResult {
                    path: "a.txt".to_owned(),
                    side: GitResolveSide::Ours,
                })))
                .unwrap();
            app.git_receiver = Some(receiver);
            assert!(app.poll_git());
            assert!(app.git_conflict_view.is_none());
            assert_eq!(
                app.git_state.as_ref().map(|state| state.conflicts.clone()),
                Some(vec!["b.txt".to_owned()])
            );
            assert!(app.status.contains("Accepted ours for a.txt"));
        });
    }

    #[test]
    fn refused_conflict_resolution_keeps_the_view_and_the_host_message() {
        let mut harness = git_outcome_harness(|_| {});
        harness.with(|app, _| {
            let (sender, receiver) = mpsc::channel();
            sender
                .send(GitUpdate::Resolve(Err(Error::Remote {
                    code: "git/path-refused".to_owned(),
                    message: "\"a.txt\" has no theirs side".to_owned(),
                    details: Default::default(),
                })))
                .unwrap();
            app.git_receiver = Some(receiver);
            assert!(app.poll_git());
            assert_eq!(
                app.git_conflict_view
                    .as_ref()
                    .and_then(|view| view.error.clone()),
                Some("\"a.txt\" has no theirs side".to_owned())
            );
            assert_eq!(
                app.git_state
                    .as_ref()
                    .map(|state| state.conflicts.len()),
                Some(2)
            );
        });
    }

    #[test]
    fn settled_stash_list_and_save_update_the_panel() {
        let mut harness = git_outcome_harness(|_| {});
        harness.with(|app, _| {
            let (sender, receiver) = mpsc::channel();
            sender
                .send(GitUpdate::Stash(Ok(GitStash {
                    action: GitStashAction::List,
                    entries: vec![GitStashEntry {
                        index: 0,
                        oid: "deadbeef".to_owned(),
                        message: "wip".to_owned(),
                        time: "2026-11-08T10:00:00+07:00".to_owned(),
                    }],
                    summary: String::new(),
                    truncated: true,
                })))
                .unwrap();
            app.git_receiver = Some(receiver);
            assert!(app.poll_git());
            let panel = app.git_stash.as_ref().unwrap();
            assert_eq!(panel.entries.len(), 1);
            assert!(panel.truncated);
        });

        let mut saved = git_outcome_harness(|_| {});
        saved.with(|app, _| {
            let (sender, receiver) = mpsc::channel();
            sender
                .send(GitUpdate::Stash(Ok(GitStash {
                    action: GitStashAction::Save,
                    entries: Vec::new(),
                    summary: "Saved working directory".to_owned(),
                    truncated: false,
                })))
                .unwrap();
            app.git_receiver = Some(receiver);
            assert!(app.poll_git());
            let panel = app.git_stash.as_ref().unwrap();
            assert_eq!(panel.summary.as_deref(), Some("Saved working directory"));
            assert!(panel.entries.is_empty());
            assert!(app.take_git_stash_clear());
        });
    }

    #[test]
    fn refused_untracked_delete_keeps_the_host_message_and_reloads_status() {
        let mut harness = git_outcome_harness(|app| app.git_status = None);
        harness.with(|app, _| {
            let (sender, receiver) = mpsc::channel();
            sender
                .send(GitUpdate::DiscardUntracked(Err(Error::Remote {
                    code: "git/path-refused".to_owned(),
                    message: "\"docs/\" is an untracked directory, not a file".to_owned(),
                    details: Default::default(),
                })))
                .unwrap();
            app.git_receiver = Some(receiver);
            assert!(app.poll_git());
            assert_eq!(
                app.git_error.as_deref(),
                Some("\"docs/\" is an untracked directory, not a file")
            );
            assert!(app.git_loading);
        });
    }
}
