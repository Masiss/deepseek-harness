//! Application state for the gpui shell.
//!
//! The Host transport, session projection, and worker channels are the same
//! ones the egui shell used; this module owns their lifecycle and lets the
//! renderer read a single state value.

use crate::command_registry::CommandId;
use crate::dialog;
use crate::document_preview::{ImageDocument, PreviewDocument, TextDocument};
use crate::explorer::WorkspaceExplorer;
use crate::host::{self, Host};
use crate::question_card::QuestionCard;
use crate::restart;
use crate::transcript::{self, RowKind, Transcript};
use crate::ui_preferences::UiPreferences;
use dsh_client_core::{
    approvals::{ApprovalDecision, PendingApproval},
    chat::{AcceptedResponse, ChatClient, PromptContentPart, PromptMode, PromptRequest},
    commands::{
        CommandDescriptor, CommandExecution, CommandResult, CommandSubmitAttachment, CommandsClient,
    },
    follow::{self, FollowFrame, FollowRequest},
    git::{
        GitBranches, GitCheckoutResult, GitClient, GitCommitResult, GitDiff, GitFile,
        GitOperationResult, GitOperationState, GitPathResult, GitRemoteResult, GitStatus,
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
    skills::{SkillListResponse, SkillsClient},
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
        WorkspaceFileWriteResult, WorkspaceFilesClient,
    },
    Error,
};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};

/// Reads the opt-out switch for the initial Host connection.
fn auto_connect_enabled(setting: Option<&str>) -> bool {
    setting != Some("0")
}

/// Progress line shown while the Host sidecar boots.
fn host_start_status(elapsed: std::time::Duration) -> String {
    format!("Starting Host sidecar… {}s", elapsed.as_secs())
}
use super::terminal_state::{append_bounded_output, PreviewScratchDirectory, TerminalPanel};
use super::theme::Theme;
use super::ShellServices;
use gpui::{App, AppContext, Context, Window, WindowAppearance};

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

enum ApprovalChoice {
    Answer(ApprovalDecision),
    Delegate,
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
}

enum SessionUpdate {
    List(Vec<SessionSummary>),
    Search(SessionSearchResponse),
    Created(String),
    Renamed(String),
    Forked(String),
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
    Branches,
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
    Checkout {
        session_id: String,
        current: Option<String>,
        target: String,
        head: Option<String>,
        changed_files: Vec<GitFile>,
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

enum GitUpdate {
    Status(Result<GitStatus, Error>),
    Branches(Result<GitBranches, Error>),
    Checkout(Result<GitCheckoutResult, Error>),
    Diff(Result<GitDiff, Error>),
    Path(Result<GitPathResult, Error>),
    Commit(Result<GitCommitResult, Error>),
    State(Result<GitOperationState, Error>),
    StatusAndState(
        Result<GitStatus, Error>,
        Result<GitOperationState, Error>,
    ),
    Remote(&'static str, Result<GitRemoteResult, Error>),
    Operation(Result<GitOperationResult, Error>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BottomTool {
    Terminal,
    Jobs,
    Schedules,
    Deliverables,
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
    Save,
}

/// Snapshot, bundles, and managed plugins returned by the inventory workers.
type PluginInventoryResult =
    Result<(PluginInventorySnapshot, Vec<BundleInfo>, Vec<ManagedPlugin>), Error>;

/// Terminal panes one window may show at once, including the primary pane.
pub(crate) const MAX_TERMINAL_PANES: usize = 4;

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

/// One palette row: a shell command or a Host-registered human command.
#[derive(Clone)]
pub(crate) enum PaletteEntry {
    Shell(CommandId),
    Host(CommandDescriptor),
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
    pub(crate) cancel_receiver: Option<mpsc::Receiver<Result<AcceptedResponse, Error>>>,
    pub(crate) dialog: Option<(DialogPurpose, mpsc::Receiver<Option<PathBuf>>)>,
    pub(crate) attachment: Option<FileUploadValue>,
    pub(crate) attachment_name: String,
    pub(crate) attachment_receiver: Option<mpsc::Receiver<Result<FileUploadValue, String>>>,
    pub(crate) attachment_error: Option<String>,
    pub(crate) chat_loading: bool,
    pub(crate) chat_receiver: Option<mpsc::Receiver<Result<FollowFrame, Error>>>,
    pub(crate) transcript: Transcript,
    pub(crate) assistant_streaming: bool,
    pub(crate) assistant_draft: String,
    pub(crate) approval_events_receiver: Option<mpsc::Receiver<Result<RemoteEvent, RemoteEventError>>>,
    pub(crate) approval_commands: Option<mpsc::Sender<ApprovalCommand>>,
    pub(crate) pending_approvals: Vec<PendingApproval>,
    pub(crate) pending_questions: Vec<QuestionCard>,
    /// Height the question/approval stack used last frame, kept out of the transcript.
    pub(crate) attention_reserved_height: f32,
    pub(crate) deliverables: Vec<DeliverableFile>,
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
    pub(crate) ui_preferences: UiPreferences,
    pub(crate) settings_description: Option<SettingsDescribeValue>,
    pub(crate) settings_loading: bool,
    pub(crate) settings_error: Option<String>,
    pub(crate) settings_receiver: Option<mpsc::Receiver<Result<SettingsDescribeValue, Error>>>,
    pub(crate) settings_update_receiver: Option<mpsc::Receiver<Result<SettingsNamespaceView, Error>>>,
    pub(crate) show_settings: bool,
    pub(crate) preview_document: Option<TextDocument>,
    pub(crate) preview_image: Option<ImageDocument>,
    pub(crate) preview_path: String,
    pub(crate) preview_loading: bool,
    pub(crate) preview_error: Option<String>,
    pub(crate) preview_receiver: Option<mpsc::Receiver<Result<PreviewDocument, String>>>,
    /// Editable copy of the loaded text document; saved with its version guard.
    pub(crate) preview_draft: String,
    /// Draft text to install into the document editor on the next render.
    pub(crate) pending_preview_text: Option<String>,
    pub(crate) preview_save_receiver: Option<mpsc::Receiver<Result<WorkspaceFileWriteResult, Error>>>,
    pub(crate) preview_save_status: Option<String>,
    pub(crate) show_document: bool,
    pub(crate) show_plugins: bool,
    pub(crate) show_skills: bool,
    pub(crate) show_market: bool,
    pub(crate) show_goals: bool,
    pub(crate) top_menu_open: bool,
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
    /// Latest diff for {@link git_selected_path}.
    pub(crate) git_diff: Option<GitDiff>,
    /// In-progress rebase, cherry-pick, merge, or revert and its conflicts.
    pub(crate) git_state: Option<GitOperationState>,
    /// Git's own text for the last remote or multi-step action.
    pub(crate) git_remote_summary: Option<String>,
    /// Two-step confirmation for push.
    pub(crate) git_push_confirm: bool,
    /// Set when a state refresh waits for the current Git worker.
    pub(crate) git_state_pending: bool,
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
    /// Whether the Session switcher popover is open.
    pub(crate) show_session_menu: bool,
    /// The panel edge being dragged, while the pointer is down.
    pub(crate) panel_drag: Option<PanelDrag>,
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
        let theme = Theme::new(&ui_preferences, appearance_is_dark(window.appearance()));
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
        Self {

            poll_task: Some(task),
            repaint_soon: false,
            close_requested: false,
            preferences_draft: None,
            preferences_message: None,
            theme,
            host_settings_view: HostSettingsView {
                namespace: None,
                draft: None,
                message: None,
            },
            inputs: std::collections::HashMap::new(),
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
            cancel_receiver: None,
            dialog: None,
            attachment: None,
            attachment_name: String::new(),
            attachment_receiver: None,
            attachment_error: None,
            chat_loading: false,
            chat_receiver: None,
            transcript: Transcript::default(),
            assistant_streaming: false,
            assistant_draft: String::new(),
            approval_events_receiver: None,
            approval_commands: None,
            pending_approvals: Vec::new(),
            pending_questions: Vec::new(),
            attention_reserved_height: 0.0,
            deliverables: Vec::new(),
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
            preview_document: None,
            preview_image: None,
            preview_path: String::new(),
            preview_loading: false,
            preview_error: None,
            preview_receiver: None,
            preview_draft: String::new(),
            pending_preview_text: None,
            preview_save_receiver: None,
            preview_save_status: None,
            show_document: false,
            show_plugins: false,
            show_skills: false,
            show_market: false,
            show_goals: false,
            top_menu_open: false,
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
            git_diff: None,
            git_state: None,
            git_remote_summary: None,
            git_push_confirm: false,
            git_confirmation: None,
            git_refresh_pending: false,
            git_refresh_probe_started_at: None,
            git_state_pending: false,
            git_error: None,
            git_loading: false,
            git_receiver: None,
            show_session_menu: false,
            panel_drag: None,
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
        }
        .to_owned();
        self.sessions_failure_context = match &operation {
            SessionOperation::List => "List refresh failed; previous inventory and selection retained. Refresh to retry",
            SessionOperation::Search(_) => "Search failed; previous results retained. Search again to retry",
            SessionOperation::Create(_) => "Create outcome unknown; refresh/search to confirm remote state before retrying",
            SessionOperation::Rename(_) => "Rename outcome unknown; reload the selected session to confirm its title before retrying",
            SessionOperation::Fork(_) => "Fork outcome unknown; refresh/search to confirm remote state before retrying",
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
            };
            let _ = sender.send(result);
        });
        self.sessions_receiver = Some(receiver);
    }

    pub(crate) fn open_settings(&mut self) {
        self.show_settings = true;
        if self.settings_description.is_some() || self.settings_loading {
            return;
        }
        self.refresh_settings();
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
        self.git_remote_summary = None;
        self.git_push_confirm = false;
        self.git_confirmation = None;
        self.git_refresh_pending = false;
        self.git_refresh_probe_started_at = None;
        self.git_state_pending = false;
        self.git_error = None;
        self.git_loading = false;
        self.git_receiver = None;
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
    pub(crate) fn run_git_remote(&mut self, operation: &'static str) {
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
            let result = match operation {
                "fetch" => client.fetch(session_id),
                "pull" => client.pull(session_id),
                _ => client.push(session_id),
            };
            let _ = sender.send(GitUpdate::Remote(operation, result));
        });
        self.git_receiver = Some(receiver);
        self.git_loading = true;
        self.git_remote_summary = None;
        self.git_error = None;
    }

    /// Rebases the current branch onto one local branch.
    pub(crate) fn rebase_git_branch(&mut self, branch: String) {
        self.run_git_operation(move |client, session_id| client.rebase(session_id, branch));
    }

    /// Cherry-picks one commit onto the current branch.
    pub(crate) fn cherry_pick_git_commit(&mut self, commit: String) {
        self.run_git_operation(move |client, session_id| client.cherry_pick(session_id, commit));
    }

    /// Continues the in-progress rebase or cherry-pick.
    pub(crate) fn continue_git_operation(&mut self) {
        self.run_git_operation(|client, session_id| client.continue_operation(session_id));
    }

    /// Aborts the in-progress rebase or cherry-pick.
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

    /// Selects one changed path and loads its diff for the current side.
    pub(crate) fn select_git_path(&mut self, path: String) {
        self.git_selected_path = Some(path);
        self.refresh_git_diff();
    }

    /// Toggles the diff side for the selected path.
    pub(crate) fn set_git_diff_staged(&mut self, staged: bool) {
        self.git_diff_staged = staged;
        self.refresh_git_diff();
    }

    /// Loads the selected path's diff from the Host.
    pub(crate) fn refresh_git_diff(&mut self) {
        let (Some(session_id), Some(transport), Some(path)) = (
            self.selected_session.clone(),
            self.transport.clone(),
            self.git_selected_path.clone(),
        ) else {
            self.git_diff = None;
            return;
        };
        if self.git_receiver.is_some() {
            return;
        }
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
        self.mutate_git_path(path, GitPathVerb::Stage);
    }

    /// Unstages one changed path.
    pub(crate) fn unstage_git_path(&mut self, path: String) {
        self.mutate_git_path(path, GitPathVerb::Unstage);
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
                self.mutate_git_path(path, GitPathVerb::Discard);
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
        }
    }

    fn mutate_git_path(&mut self, path: String, verb: GitPathVerb) {
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
            let result = match verb {
                GitPathVerb::Stage => client.stage(session_id, path),
                GitPathVerb::Unstage => client.unstage(session_id, path),
                GitPathVerb::Discard => client.discard(session_id, path),
            };
            let _ = sender.send(GitUpdate::Path(result));
        });
        self.git_receiver = Some(receiver);
        self.git_loading = true;
        self.git_error = None;
    }

    /// Commits the staged index with the composer's message.
    pub(crate) fn commit_git(&mut self, message: String) {
        let (Some(session_id), Some(transport)) =
            (self.selected_session.clone(), self.transport.clone())
        else {
            self.reset_git();
            return;
        };
        if message.trim().is_empty() || self.git_receiver.is_some() {
            return;
        }
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let result = GitClient::new(&transport).commit(session_id, message);
            let _ = sender.send(GitUpdate::Commit(result));
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
                match update {
                    GitUpdate::Status(Ok(status)) => {
                        self.git_status = Some(status);
                        self.git_error = None;
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
                    GitUpdate::Path(Ok(result)) => {
                        self.git_diff = None;
                        self.status = format!("{}: {}", result.operation, result.path);
                        self.refresh_git();
                        if self.changes_tab == ChangesTab::Branches {
                            self.refresh_git_branches();
                        }
                    }
                    GitUpdate::Path(Err(error)) => {
                        self.git_error = Some(failure_text(&error));
                    }
                    GitUpdate::Commit(Ok(result)) => {
                        self.git_diff = None;
                        self.git_selected_path = None;
                        self.status = format!("Committed {}", short_object_id(&result.commit));
                        self.refresh_git();
                        self.refresh_git_state();
                    }
                    GitUpdate::Commit(Err(error)) => {
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
                        self.git_push_confirm = false;
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
                        self.git_push_confirm = false;
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
                }
                if self.git_state_pending {
                    self.git_state_pending = false;
                    self.refresh_git_state();
                }
                true
            }
            Err(mpsc::TryRecvError::Empty) => false,
            Err(mpsc::TryRecvError::Disconnected) => {
                self.git_receiver = None;
                self.git_loading = false;
                self.git_state_pending = false;
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

    fn reload_session_after_reconnect(&mut self, session_id: String) {
        self.load_session_with_policy(session_id, true);
    }

    fn load_session_with_policy(&mut self, session_id: String, retain_loaded_view: bool) {
        let retain_loaded_view =
            retain_loaded_view && self.selected_session.as_deref() == Some(session_id.as_str());
        if !retain_loaded_view {
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
            self.scroll_transcript_to_bottom = true;
            self.deliverables.clear();
            self.feedback_items.clear();
            self.preview_document = None;
            self.preview_image = None;
            self.preview_error = None;
        }
        self.refresh_feedback();
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
        let from_seq = (self.transcript.last_seq() >= 0).then_some(self.transcript.last_seq());
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
                        from_seq,
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

    /// Writes the previewed text to a local path chosen through the save picker.
    fn save_preview_path(&mut self, path: PathBuf) {
        let Some(document) = self.preview_document.clone() else {
            self.status = "Save a document after previewing one".to_owned();
            return;
        };
        let text = document.text;
        let name = path.display().to_string();
        match std::fs::write(&path, text) {
            Ok(()) => self.status = format!("Saved preview to {name}"),
            Err(error) => self.status = format!("Could not save {name}: {error}"),
        }
    }

    fn poll_dialog(&mut self) {
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
                    (DialogPurpose::Save, Some(path)) => self.save_preview_path(path),
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
        let from_seq = (self.transcript.last_seq() >= 0).then_some(self.transcript.last_seq());
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
                from_seq,
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
        self.active_view = view;
        self.explorer_fullscreen = view == ActiveView::Explorer;
        self.show_settings = view == ActiveView::Settings;
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
                "document-body" => app.preview_draft = text,
                "market-query" => {
                    app.market_query = text;
                    app.search_market();
                }
                "skills-query" => app.skills_query = text,
                "document-path" => {
                    app.preview_path = text;
                    app.read_document();
                }
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
        self.theme = Theme::new(&self.ui_preferences, appearance_is_dark(window.appearance()));
    }

    /// Starts resizing one docked panel from a pointer position.
    pub(crate) fn begin_panel_drag(&mut self, target: ResizeTarget, start_x: f32) {
        let (start_width, start_secondary) = match target {
            ResizeTarget::Explorer => (self.ui_preferences.workspace_panel_width, 0.0),
            ResizeTarget::Changes => (self.ui_preferences.changes_panel_width, 0.0),
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
                self.ui_preferences.workspace_panel_width = (drag.start_width + delta).clamp(
                    crate::layout::EXPLORER_MIN,
                    crate::layout::EXPLORER_MAX,
                );
            }
            ResizeTarget::Changes => {
                self.ui_preferences.changes_panel_width = (drag.start_width - delta)
                    .clamp(crate::layout::CHANGES_MIN, crate::layout::CHANGES_MAX);
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

    /// Opens the native save picker for the Document surface.
    pub(crate) fn save_file_dialog(&mut self) {
        self.dialog = Some((DialogPurpose::Save, dialog::save_file_async()));
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

    /// Runs one shared shell command from the menu, shortcut, or palette.
    pub(crate) fn run_command(&mut self, command: CommandId) {
        self.show_session_menu = false;
        self.top_menu_open = false;
        self.show_about = false;
        match command {
            CommandId::ToggleSettings => {
                self.show_settings = !self.show_settings;
                self.active_view = ActiveView::Settings;
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
        if self.show_session_menu {
            self.show_session_menu = false;
            return true;
        }
        if self.top_menu_open {
            self.top_menu_open = false;
            return true;
        }
        if self.palette_open {
            self.close_palette();
            self.focus_composer = true;
            return true;
        }
        if self.show_about {
            self.show_about = false;
            return true;
        }
        false
    }

    /// One row the palette can run.
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
        self.poll_dialog();
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
        if let Some(receiver) = &self.page_receiver {
            match receiver.try_recv() {
                Ok(Ok((through_seq, page, live))) => {
                    self.page_through_seq = Some(through_seq);
                    self.transcript.apply_snapshot(&live);
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
                            Ok(pending) => self.pending_approvals.push(pending),
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
        || app.terminal.inventory.is_some()
        || app.terminal.updates.is_some()
        || app.terminal.dirty
        || app
            .terminal_extra
            .iter()
            .any(|pane| pane.inventory.is_some() || pane.updates.is_some() || pane.dirty)

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
        from_seq: None,
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
            files: vec![GitFile {
                path: "a.txt".to_owned(),
                status: "modified".to_owned(),
                staged: false,
                worktree: true,
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
}
