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
    commands::{CommandExecution, CommandResult, CommandsClient},
    follow::{self, FollowFrame, FollowRequest},
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
    streams::{self, JobFollowFrame, JobView},
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
    workspace_files::{
        WorkspaceByteRange, WorkspaceByteReadOptions, WorkspaceFileRange, WorkspaceFileScope,
        WorkspaceFilesClient,
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
    Create,
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

/// Application state owns the transport and the last session-list result so UI rendering stays local.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ActiveView {
    Sessions,
    Explorer,
    Settings,
    Document,
    Plugins,
    Skills,
    Marketplace,
    Goals,
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

pub(crate) struct DesktopApp {
    pub(crate) host: Option<Host>,
    pub(crate) host_receiver: Option<mpsc::Receiver<Host>>,
    /// When the shell was created; startup milestones report their elapsed time
    /// from here so a slow Host launch can be attributed without a profiler.
    pub(crate) started_at: std::time::Instant,
    pub(crate) connect_receiver: Option<mpsc::Receiver<Result<(String, HttpTransport), Error>>>,
    pub(crate) sessions_receiver: Option<mpsc::Receiver<Result<SessionUpdate, Error>>>,
    pub(crate) session_query: String,
    pub(crate) session_search: Option<SessionSearchResponse>,
    pub(crate) session_title: String,
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
    pub(crate) bottom_tool: Option<BottomTool>,
    pub(crate) explorer: WorkspaceExplorer,
    pub(crate) show_explorer: bool,
    pub(crate) explorer_fullscreen: bool,
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
            session_query: String::new(),
            session_search: None,
            session_title: String::new(),
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
            bottom_tool: None,
            explorer: WorkspaceExplorer::default(),
            show_explorer,
            explorer_fullscreen: false,
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
            SessionOperation::Create => "Creating session…",
            SessionOperation::Rename(_) => "Renaming session…",
            SessionOperation::Fork(_) => "Forking session…",
        }
        .to_owned();
        self.sessions_failure_context = match &operation {
            SessionOperation::List => "List refresh failed; previous inventory and selection retained. Refresh to retry",
            SessionOperation::Search(_) => "Search failed; previous results retained. Search again to retry",
            SessionOperation::Create => "Create outcome unknown; refresh/search to confirm remote state before retrying",
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
                SessionOperation::Create => client
                    .create(CreateSessionRequest {
                        workspace_id: None,
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
        self.explorer
            .select_session(Some(&session_id), self.transport.as_ref());
        if retain_loaded_view {
            self.terminal.select_session(None, None, None);
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
        let Some(streams) = self.streams.clone() else {
            self.page_error = Some("Remote streams are not connected".to_owned());
            return;
        };
        let attachment = self.attachment.clone();
        self.chat_text.clear();
        self.attachment = None;
        self.attachment_name.clear();
        self.chat_loading = true;
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
                mode: PromptMode::Queue,
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

    /// Switches the central surface and its side effects, as the navigation rail does.
    pub(crate) fn switch_view(&mut self, view: ActiveView) {
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
    }

    /// Creates a Session and focuses the composer.
    pub(crate) fn new_session(&mut self) {
        self.request_sessions(SessionOperation::Create);
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

    /// Forks the selected Session at its latest sequence.
    pub(crate) fn fork_selected_session(&mut self) {
        let Some(session_id) = self.selected_session.clone() else {
            return;
        };
        self.request_sessions(SessionOperation::Fork(ForkSessionRequest {
            session_id,
            at_seq: None,
            allow_migration: None,
        }));
    }

    /// True while any Session list request is in flight.
    pub(crate) fn sessions_busy(&self) -> bool {
        self.sessions_receiver.is_some()
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
            let super::text_input::TextInputEvent::Submit = event else {
                return;
            };
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
    pub(crate) fn sync_inputs(&mut self, cx: &App) {
        for (key, field) in [
            ("composer", &mut self.chat_text),
            ("session-search", &mut self.session_query),
            ("session-title", &mut self.session_title),
            ("market-query", &mut self.market_query),
            ("skills-query", &mut self.skills_query),
            ("document-path", &mut self.preview_path),
            ("goal-objective", &mut self.goal_objective),
            ("goal-rounds", &mut self.goal_max_rounds),
        ] {
            if let Some(input) = self.inputs.get(key) {
                *field = input.read(cx).text().to_owned();
            }
        }
        if let Some(input) = self.inputs.get("settings-draft") {
            self.host_settings_view.draft = Some(input.read(cx).text().to_owned());
        }
    }

    /// Recomputes the theme after the preferences changed.
    pub(crate) fn refresh_theme(&mut self, window: &Window) {
        self.theme = Theme::new(&self.ui_preferences, appearance_is_dark(window.appearance()));
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

    /// Installs a marketplace entry by its Host-provided spec.
    pub(crate) fn install_entry(&mut self, spec: String) {
        self.install_market_entry(spec, MarketplaceInstallOptions { registry: None });
    }

    /// Applies the client-local shortcut bindings; each chord toggles its view once.
    /// Runs one shared shell command from the menu, shortcut, or palette.
    pub(crate) fn run_command(&mut self, command: CommandId) {        match command {
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
                self.request_sessions(SessionOperation::Create);
                self.focus_composer = true;
            }
            CommandId::ToggleExplorer => {
                self.show_explorer = !self.show_explorer;
            }
            CommandId::ToggleChanges => {}
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
        }
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
        self.poll_dialog();
        self.poll_attachment();
        self.explorer.poll();
        self.poll_jobs();
        self.poll_schedules();
        if self.dialog.is_some()
            || self.jobs_receiver.is_some()
            || self.job_receiver.is_some()
            || self.schedules_receiver.is_some()
            || self.schedule_delete_receiver.is_some()
        {
            self.repaint_soon = true;
        }
        if self.quitting.load(Ordering::Acquire) {
            self.terminal.detach();
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
        if let Some(receiver) = &self.cancel_receiver {
            match receiver.try_recv() {
                Ok(Ok(_)) => {
                    self.status = "Turn cancellation requested".to_owned();
                    self.cancel_receiver = None;
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
                            }
                        }
                    }
                },
                Ok(Err(error)) => {
                    self.page_error = Some(format!("Chat stream failed: {error}"));
                    self.chat_loading = false;
                    self.chat_receiver = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.chat_loading = false;
                    self.chat_receiver = None;
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
    let _ = input.send(TerminalInput::Write {
        sequence: 2,
        data: "exit\n".to_owned(),
    });
    let _ = client.close(TerminalCloseRequest {
        id: "probe-terminal".to_owned(),
    });
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

    #[test]
    fn host_start_status_counts_whole_seconds() {
        use std::time::Duration;
        assert_eq!(host_start_status(Duration::from_millis(0)), "Starting Host sidecar… 0s");
        assert_eq!(host_start_status(Duration::from_millis(2_900)), "Starting Host sidecar… 2s");
        assert_eq!(host_start_status(Duration::from_secs(65)), "Starting Host sidecar… 65s");
    }
}
