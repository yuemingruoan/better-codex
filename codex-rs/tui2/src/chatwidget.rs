//! The main Codex TUI chat surface.
//!
//! `ChatWidget` consumes protocol events, builds and updates history cells, and drives rendering
//! for both the main viewport and overlay UIs.
//!
//! The UI has both committed transcript cells (finalized `HistoryCell`s) and an in-flight active
//! cell (`ChatWidget.active_cell`) that can mutate in place while streaming (often representing a
//! coalesced exec/tool group). The transcript overlay (`Ctrl+T`) renders committed cells plus a
//! cached, render-only live tail derived from the current active cell so in-flight tool calls are
//! visible immediately.
//!
//! The transcript overlay is kept in sync by `App::overlay_forward_event`, which syncs a live tail
//! during draws using `active_cell_transcript_key()` and `active_cell_transcript_lines()`. The
//! cache key is designed to change when the active cell mutates in place or when its transcript
//! output is time-dependent so the overlay can refresh its cached tail without rebuilding it on
//! every draw.
//!
//! The bottom pane exposes a single "task running" indicator that drives the spinner and interrupt
//! hints. This module treats that indicator as derived UI-busy state: it is set while an agent turn
//! is in progress and while MCP server startup is in progress. Those lifecycles are tracked
//! independently (`agent_turn_running` and `mcp_startup_status`) and synchronized via
//! `update_task_running_state`.
use std::collections::HashMap;
use std::collections::HashSet;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

use codex_backend_client::Client as BackendClient;
use codex_core::config::Config;
use codex_core::config::ConstraintResult;
use codex_core::config::types::Notifications;
use codex_core::config::types::SubagentPreset;
use codex_core::features::Feature;
use codex_core::git_info::current_branch_name;
use codex_core::git_info::local_git_branches;
use codex_core::models_manager::manager::ModelsManager;
use codex_core::project_doc::DEFAULT_PROJECT_DOC_FILENAME;
use codex_core::protocol::AgentMessageDeltaEvent;
use codex_core::protocol::AgentMessageEvent;
use codex_core::protocol::AgentReasoningDeltaEvent;
use codex_core::protocol::AgentReasoningEvent;
use codex_core::protocol::AgentReasoningRawContentDeltaEvent;
use codex_core::protocol::AgentReasoningRawContentEvent;
use codex_core::protocol::AgentStatus;
use codex_core::protocol::ApplyPatchApprovalRequestEvent;
use codex_core::protocol::BackgroundEventEvent;
use codex_core::protocol::CollabAgentInteractionBeginEvent;
use codex_core::protocol::CollabAgentInteractionEndEvent;
use codex_core::protocol::CollabAgentSpawnBeginEvent;
use codex_core::protocol::CollabAgentSpawnEndEvent;
use codex_core::protocol::CollabCloseBeginEvent;
use codex_core::protocol::CollabCloseEndEvent;
use codex_core::protocol::CollabWaitingBeginEvent;
use codex_core::protocol::CollabWaitingEndEvent;
use codex_core::protocol::CreditsSnapshot;
use codex_core::protocol::DeprecationNoticeEvent;
use codex_core::protocol::ErrorEvent;
use codex_core::protocol::Event;
use codex_core::protocol::EventMsg;
use codex_core::protocol::ExecApprovalRequestEvent;
use codex_core::protocol::ExecCommandBeginEvent;
use codex_core::protocol::ExecCommandEndEvent;
use codex_core::protocol::ExecCommandOutputDeltaEvent;
use codex_core::protocol::ExecCommandSource;
use codex_core::protocol::ExitedReviewModeEvent;
use codex_core::protocol::ListCustomPromptsResponseEvent;
use codex_core::protocol::ListSkillsResponseEvent;
use codex_core::protocol::McpListToolsResponseEvent;
use codex_core::protocol::McpStartupCompleteEvent;
use codex_core::protocol::McpStartupStatus;
use codex_core::protocol::McpStartupUpdateEvent;
use codex_core::protocol::McpToolCallBeginEvent;
use codex_core::protocol::McpToolCallEndEvent;
use codex_core::protocol::Op;
use codex_core::protocol::PatchApplyBeginEvent;
use codex_core::protocol::RateLimitSnapshot;
use codex_core::protocol::RequestUserInputEvent;
use codex_core::protocol::ReviewRequest;
use codex_core::protocol::ReviewTarget;
use codex_core::protocol::SddGitAction;
use codex_core::protocol::SkillsListEntry;
use codex_core::protocol::StreamErrorEvent;
use codex_core::protocol::TerminalInteractionEvent;
use codex_core::protocol::TokenUsage;
use codex_core::protocol::TokenUsageInfo;
use codex_core::protocol::TurnAbortReason;
use codex_core::protocol::TurnCompleteEvent;
use codex_core::protocol::TurnDiffEvent;
use codex_core::protocol::UndoCompletedEvent;
use codex_core::protocol::UndoStartedEvent;
use codex_core::protocol::UserMessageEvent;
use codex_core::protocol::ViewImageToolCallEvent;
use codex_core::protocol::WarningEvent;
use codex_core::protocol::WebSearchBeginEvent;
use codex_core::protocol::WebSearchEndEvent;
use codex_core::skills::model::SkillDependencies;
use codex_core::skills::model::SkillInterface;
use codex_core::skills::model::SkillMetadata;
use codex_core::skills::model::SkillToolDependency;
#[cfg(target_os = "windows")]
use codex_core::windows_sandbox::WindowsSandboxLevelExt;
use codex_protocol::ThreadId;
use codex_protocol::account::PlanType;
use codex_protocol::approvals::ElicitationRequestEvent;
use codex_protocol::config_types::CollaborationMode;
use codex_protocol::config_types::ModeKind;
use codex_protocol::config_types::SandboxMode;
use codex_protocol::config_types::Settings as CollaborationSettings;
#[cfg(target_os = "windows")]
use codex_protocol::config_types::WindowsSandboxLevel;
use codex_protocol::parse_command::ParsedCommand;
use codex_protocol::user_input::UserInput;
use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyEventKind;
use crossterm::event::KeyModifiers;
use rand::Rng;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::widgets::Paragraph;
use ratatui::widgets::Wrap;
use tokio::sync::mpsc::UnboundedSender;
use tokio::task::JoinHandle;
use tracing::debug;

use crate::app_event::AppEvent;
use crate::app_event::ExitMode;
#[cfg(target_os = "windows")]
use crate::app_event::WindowsSandboxEnableMode;
use crate::app_event::WindowsSandboxFallbackReason;
use crate::app_event_sender::AppEventSender;
use crate::bottom_pane::ApprovalRequest;
use crate::bottom_pane::BottomPane;
use crate::bottom_pane::BottomPaneParams;
use crate::bottom_pane::CancellationEvent;
use crate::bottom_pane::DOUBLE_PRESS_QUIT_SHORTCUT_ENABLED;
use crate::bottom_pane::ImageAttachment;
use crate::bottom_pane::InputResult;
use crate::bottom_pane::QUIT_SHORTCUT_TIMEOUT;
use crate::bottom_pane::SelectionAction;
use crate::bottom_pane::SelectionInteractionMode;
use crate::bottom_pane::SelectionItem;
use crate::bottom_pane::SelectionViewParams;
use crate::bottom_pane::custom_prompt_view::CustomPromptView;
use crate::bottom_pane::popup_consts::standard_popup_hint_line;
use crate::clipboard_paste::clean_clipboard_cache;
use crate::clipboard_paste::paste_image_as_data_url;
use crate::diff_render::display_path_for;
use crate::exec_cell::CommandOutput;
use crate::exec_cell::ExecCell;
use crate::exec_cell::new_active_exec_command;
use crate::get_git_diff::get_git_diff;
use crate::history_cell;
use crate::history_cell::AgentMessageCell;
use crate::history_cell::HistoryCell;
use crate::history_cell::McpToolCallCell;
use crate::history_cell::PlainHistoryCell;
use crate::i18n::language_name;
use crate::i18n::tr;
use crate::i18n::tr_args;
use crate::i18n::tr_list;
use crate::key_hint;
use crate::key_hint::KeyBinding;
use crate::markdown::append_markdown;
use crate::render::Insets;
use crate::render::renderable::ColumnRenderable;
use crate::render::renderable::FlexRenderable;
use crate::render::renderable::Renderable;
use crate::render::renderable::RenderableExt;
use crate::render::renderable::RenderableItem;
use crate::slash_command::SlashCommand;
use crate::status::RateLimitSnapshotDisplay;
use crate::text_formatting::truncate_text;
use crate::tui::FrameRequester;
mod interrupts;
use self::interrupts::InterruptManager;
mod agent;
use self::agent::spawn_agent;
use self::agent::spawn_agent_from_existing;
mod session_header;
use self::session_header::SessionHeader;
use crate::streaming::controller::StreamController;
use crate::version::CODEX_CLI_VERSION;
use std::path::Path;

use chrono::Local;
use codex_common::approval_presets::ApprovalPreset;
use codex_common::approval_presets::builtin_approval_presets;
use codex_common::token_usage::split_total_and_last;
use codex_core::AuthManager;
use codex_core::CodexAuth;
use codex_core::ThreadManager;
use codex_core::git_info::get_git_repo_root;
use codex_core::protocol::AskForApproval;
use codex_core::protocol::SandboxPolicy;
use codex_file_search::FileMatch;
use codex_protocol::config_types::Language;
use codex_protocol::openai_models::ModelPreset;
use codex_protocol::openai_models::ReasoningEffort as ReasoningEffortConfig;
use codex_protocol::plan_tool::UpdatePlanArgs;
use strum::IntoEnumIterator;

fn user_shell_command_help_title(language: Language) -> &'static str {
    tr(language, "chatwidget.shell_help.title")
}

fn user_shell_command_help_hint(language: Language) -> &'static str {
    tr(language, "chatwidget.shell_help.example")
}
const SDD_BRANCH_PREFIX: &str = "sdd/";

fn init_prompt(language: Language) -> &'static str {
    tr(language, "prompt.init")
}

fn sdd_plan_prompt_template(language: Language) -> &'static str {
    tr(language, "prompt.sdd_plan")
}

fn sdd_plan_parallels_prompt_template(language: Language) -> &'static str {
    tr(language, "prompt.sdd_plan_parallels")
}

fn sdd_exec_prompt_template(language: Language) -> &'static str {
    tr(language, "prompt.sdd_execute")
}

fn sdd_exec_parallels_prompt_template(language: Language) -> &'static str {
    tr(language, "prompt.sdd_execute_parallels")
}

fn sdd_merge_prompt_template(language: Language) -> &'static str {
    tr(language, "prompt.sdd_merge")
}

fn sdd_merge_parallels_prompt_template(language: Language) -> &'static str {
    tr(language, "prompt.sdd_merge_parallels")
}
// Track information about an in-flight exec command.
struct RunningCommand {
    command: Vec<String>,
    parsed_cmd: Vec<ParsedCommand>,
    source: ExecCommandSource,
}

struct UnifiedExecWaitState {
    command_display: String,
}

impl UnifiedExecWaitState {
    fn new(command_display: String) -> Self {
        Self { command_display }
    }

    fn is_duplicate(&self, command_display: &str) -> bool {
        self.command_display == command_display
    }
}

const RATE_LIMIT_WARNING_THRESHOLDS: [f64; 3] = [75.0, 90.0, 95.0];
const NUDGE_MODEL_SLUG: &str = "gpt-5.1-codex-mini";
const RATE_LIMIT_SWITCH_PROMPT_THRESHOLD: f64 = 90.0;
const DEFAULT_MODEL_DISPLAY_NAME: &str = "loading";

#[derive(Default)]
struct RateLimitWarningState {
    secondary_index: usize,
    primary_index: usize,
}

impl RateLimitWarningState {
    fn take_warnings(
        &mut self,
        secondary_used_percent: Option<f64>,
        secondary_window_minutes: Option<i64>,
        primary_used_percent: Option<f64>,
        primary_window_minutes: Option<i64>,
        language: Language,
    ) -> Vec<String> {
        let reached_secondary_cap =
            matches!(secondary_used_percent, Some(percent) if percent == 100.0);
        let reached_primary_cap = matches!(primary_used_percent, Some(percent) if percent == 100.0);
        if reached_secondary_cap || reached_primary_cap {
            return Vec::new();
        }

        let mut warnings = Vec::new();

        if let Some(secondary_used_percent) = secondary_used_percent {
            let mut highest_secondary: Option<f64> = None;
            while self.secondary_index < RATE_LIMIT_WARNING_THRESHOLDS.len()
                && secondary_used_percent >= RATE_LIMIT_WARNING_THRESHOLDS[self.secondary_index]
            {
                highest_secondary = Some(RATE_LIMIT_WARNING_THRESHOLDS[self.secondary_index]);
                self.secondary_index += 1;
            }
            if let Some(threshold) = highest_secondary {
                let limit_label = secondary_window_minutes
                    .map(get_limits_duration)
                    .unwrap_or_else(|| "weekly".to_string());
                let limit_label = localize_limit_label(limit_label, language);
                let remaining_percent = 100.0 - threshold;
                let remaining_percent = format!("{remaining_percent:.0}");
                let message = tr_args(
                    language,
                    "chatwidget.rate_limit.warning",
                    &[
                        ("limit_label", &limit_label),
                        ("percent", &remaining_percent),
                    ],
                );
                warnings.push(message);
            }
        }

        if let Some(primary_used_percent) = primary_used_percent {
            let mut highest_primary: Option<f64> = None;
            while self.primary_index < RATE_LIMIT_WARNING_THRESHOLDS.len()
                && primary_used_percent >= RATE_LIMIT_WARNING_THRESHOLDS[self.primary_index]
            {
                highest_primary = Some(RATE_LIMIT_WARNING_THRESHOLDS[self.primary_index]);
                self.primary_index += 1;
            }
            if let Some(threshold) = highest_primary {
                let limit_label = primary_window_minutes
                    .map(get_limits_duration)
                    .unwrap_or_else(|| "5h".to_string());
                let limit_label = localize_limit_label(limit_label, language);
                let remaining_percent = 100.0 - threshold;
                let remaining_percent = format!("{remaining_percent:.0}");
                let message = tr_args(
                    language,
                    "chatwidget.rate_limit.warning",
                    &[
                        ("limit_label", &limit_label),
                        ("percent", &remaining_percent),
                    ],
                );
                warnings.push(message);
            }
        }

        warnings
    }
}

fn localize_limit_label(label: String, language: Language) -> String {
    match label.as_str() {
        "weekly" => tr(language, "chatwidget.rate_limit.label.weekly").to_string(),
        "monthly" => tr(language, "chatwidget.rate_limit.label.monthly").to_string(),
        "annual" => tr(language, "chatwidget.rate_limit.label.annual").to_string(),
        _ => {
            if let Some(number) = label.strip_suffix('h')
                && number.parse::<i64>().is_ok()
            {
                tr_args(
                    language,
                    "chatwidget.rate_limit.label.hours",
                    &[("count", number)],
                )
            } else {
                label
            }
        }
    }
}

pub(crate) fn get_limits_duration(windows_minutes: i64) -> String {
    const MINUTES_PER_HOUR: i64 = 60;
    const MINUTES_PER_DAY: i64 = 24 * MINUTES_PER_HOUR;
    const MINUTES_PER_WEEK: i64 = 7 * MINUTES_PER_DAY;
    const MINUTES_PER_MONTH: i64 = 30 * MINUTES_PER_DAY;
    const ROUNDING_BIAS_MINUTES: i64 = 3;

    let windows_minutes = windows_minutes.max(0);

    if windows_minutes <= MINUTES_PER_DAY.saturating_add(ROUNDING_BIAS_MINUTES) {
        let adjusted = windows_minutes.saturating_add(ROUNDING_BIAS_MINUTES);
        let hours = std::cmp::max(1, adjusted / MINUTES_PER_HOUR);
        format!("{hours}h")
    } else if windows_minutes <= MINUTES_PER_WEEK.saturating_add(ROUNDING_BIAS_MINUTES) {
        "weekly".to_string()
    } else if windows_minutes <= MINUTES_PER_MONTH.saturating_add(ROUNDING_BIAS_MINUTES) {
        "monthly".to_string()
    } else {
        "annual".to_string()
    }
}

/// Common initialization parameters shared by all `ChatWidget` constructors.
pub(crate) struct ChatWidgetInit {
    pub(crate) config: Config,
    pub(crate) frame_requester: FrameRequester,
    pub(crate) app_event_tx: AppEventSender,
    pub(crate) initial_prompt: Option<String>,
    pub(crate) initial_images: Vec<PathBuf>,
    pub(crate) enhanced_keys_supported: bool,
    pub(crate) auth_manager: Arc<AuthManager>,
    pub(crate) models_manager: Arc<ModelsManager>,
    pub(crate) feedback: codex_feedback::CodexFeedback,
    pub(crate) is_first_run: bool,
    pub(crate) model: Option<String>,
}

#[derive(Default)]
enum RateLimitSwitchPromptState {
    #[default]
    Idle,
    Pending,
    Shown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum SddDevelopStage {
    AwaitPlanDecision,
    AwaitDevDecision,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SddWorkflow {
    Standard,
    Parallels,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SddDevelopState {
    workflow: SddWorkflow,
    description: String,
    branch_name: String,
    base_branch: Option<String>,
    stage: SddDevelopStage,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum SddGitPendingAction {
    CreateBranchForPlan { description: String },
    FinalizeMerge,
    AbandonBranch,
}
/// Maintains the per-session UI state and interaction state machines for the chat screen.
///
/// `ChatWidget` owns the state derived from the protocol event stream (history cells, streaming
/// buffers, bottom-pane overlays, and transient status text) and turns key presses into user
/// intent (`Op` submissions and `AppEvent` requests).
///
/// It is not responsible for running the agent itself; it reflects progress by updating UI state
/// and by sending requests back to codex-core.
///
/// Quit/interrupt behavior intentionally spans layers: the bottom pane owns local input routing
/// (which view gets Ctrl+C), while `ChatWidget` owns process-level decisions such as interrupting
/// active work, arming the double-press quit shortcut, and requesting shutdown-first exit.
pub(crate) struct ChatWidget {
    app_event_tx: AppEventSender,
    codex_op_tx: UnboundedSender<Op>,
    bottom_pane: BottomPane,
    active_cell: Option<Box<dyn HistoryCell>>,
    /// Monotonic-ish counter used to invalidate transcript overlay caching.
    ///
    /// The transcript overlay appends a cached "live tail" for the current active cell. Most
    /// active-cell updates are mutations of the *existing* cell (not a replacement), so pointer
    /// identity alone is not a good cache key.
    ///
    /// Callers bump this whenever the active cell's transcript output could change without
    /// flushing. It is intentionally allowed to wrap, which implies a rare one-time cache collision
    /// where the overlay may briefly treat new tail content as already cached.
    active_cell_revision: u64,
    config: Config,
    model: Option<String>,
    auth_manager: Arc<AuthManager>,
    models_manager: Arc<ModelsManager>,
    session_header: SessionHeader,
    initial_user_message: Option<UserMessage>,
    token_info: Option<TokenUsageInfo>,
    last_api_token_usage: Option<TokenUsage>,
    rate_limit_snapshot: Option<RateLimitSnapshotDisplay>,
    plan_type: Option<PlanType>,
    rate_limit_warnings: RateLimitWarningState,
    rate_limit_switch_prompt: RateLimitSwitchPromptState,
    rate_limit_poller: Option<JoinHandle<()>>,
    // Stream lifecycle controller
    stream_controller: Option<StreamController>,
    running_commands: HashMap<String, RunningCommand>,
    suppressed_exec_calls: HashSet<String>,
    last_unified_wait: Option<UnifiedExecWaitState>,
    task_complete_pending: bool,
    /// Tracks whether codex-core currently considers an agent turn to be in progress.
    ///
    /// This is kept separate from `mcp_startup_status` so that MCP startup progress (or completion)
    /// can update the status header without accidentally clearing the spinner for an active turn.
    agent_turn_running: bool,
    /// Tracks per-server MCP startup state while startup is in progress.
    ///
    /// The map is `Some(_)` from the first `McpStartupUpdate` until `McpStartupComplete`, and the
    /// bottom pane is treated as "running" while this is populated, even if no agent turn is
    /// currently executing.
    mcp_startup_status: Option<HashMap<String, McpStartupStatus>>,
    // Queue of interruptive UI events deferred during an active write cycle
    interrupts: InterruptManager,
    // Accumulates the current reasoning block text to extract a header
    reasoning_buffer: String,
    // Accumulates full reasoning content for transcript-only recording
    full_reasoning_buffer: String,
    // Current status header shown in the status indicator.
    current_status_header: String,
    // Previous status header to restore after a transient stream retry.
    retry_status_header: Option<String>,
    conversation_id: Option<ThreadId>,
    frame_requester: FrameRequester,
    // Whether to include the initial welcome banner on session configured
    show_welcome_banner: bool,
    // When resuming an existing session (selected via resume picker), avoid an
    // immediate redraw on SessionConfigured to prevent a gratuitous UI flicker.
    suppress_session_configured_redraw: bool,
    // User messages queued while a turn is in progress
    queued_user_messages: VecDeque<UserMessage>,
    // request_user_input prompts that have been surfaced but not finalized.
    pending_request_user_input: VecDeque<RequestUserInputEvent>,
    // Pending notification to show when unfocused on next Draw
    pending_notification: Option<Notification>,
    /// When `Some`, the user has pressed a quit shortcut and the second press
    /// must occur before `quit_shortcut_expires_at`.
    quit_shortcut_expires_at: Option<Instant>,
    /// Tracks which quit shortcut key was pressed first.
    ///
    /// We require the second press to match this key so `Ctrl+C` followed by
    /// `Ctrl+D` (or vice versa) doesn't quit accidentally.
    quit_shortcut_key: Option<KeyBinding>,
    // Simple review mode flag; used to adjust layout and banners.
    is_review_mode: bool,
    // Snapshot of token usage to restore after review mode exits.
    pre_review_token_info: Option<Option<TokenUsageInfo>>,
    pre_review_last_api_token_usage: Option<Option<TokenUsage>>,
    // Whether to add a final message separator after the last message
    needs_final_message_separator: bool,

    last_rendered_width: std::cell::Cell<Option<usize>>,
    // Feedback sink for /feedback
    feedback: codex_feedback::CodexFeedback,
    // Current session rollout path (if known)
    current_rollout_path: Option<PathBuf>,
    // State for the /sdd-develop workflow.
    sdd_state: Option<SddDevelopState>,
    // Pending plan-rework prompt prefix to prepend on the next user submission.
    sdd_pending_plan_rework_prompt: Option<String>,
    // When true, reopen plan options after the next task completes.
    sdd_open_plan_options_after_task: bool,
    // Pending SDD git action awaiting completion.
    sdd_pending_git_action: Option<SddGitPendingAction>,
    // Whether the last SDD git action reported a failure.
    sdd_git_action_failed: bool,
    // When true, start a fresh session after current turn completes (used for SDD abandon).
    sdd_new_session_after_cleanup: bool,
    // Previous spec.sdd_planning value before SDD workflow auto-enables it.
    sdd_spec_sdd_planning_restore: Option<bool>,
}

/// Snapshot of active-cell state that affects transcript overlay rendering.
///
/// The overlay keeps a cached "live tail" for the in-flight cell; this key lets
/// it cheaply decide when to recompute that tail as the active cell evolves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ActiveCellTranscriptKey {
    /// Cache-busting revision for in-place updates.
    ///
    /// Many active cells are updated incrementally while streaming (for example when exec groups
    /// add output or change status), and the transcript overlay caches its live tail, so this
    /// revision gives a cheap way to say "same active cell, but its transcript output is different
    /// now". Callers bump it on any mutation that can affect `HistoryCell::transcript_lines`.
    pub(crate) revision: u64,
    /// Whether the active cell continues the prior stream, which affects
    /// spacing between transcript blocks.
    pub(crate) is_stream_continuation: bool,
    /// Optional animation tick for time-dependent transcript output.
    ///
    /// When this changes, the overlay recomputes the cached tail even if the revision and width
    /// are unchanged, which is how shimmer/spinner visuals can animate in the overlay without any
    /// underlying data change.
    pub(crate) animation_tick: Option<u64>,
}

struct UserMessage {
    text: String,
    image_attachments: Vec<ImageAttachment>,
}

impl From<String> for UserMessage {
    fn from(text: String) -> Self {
        Self {
            text,
            image_attachments: Vec::new(),
        }
    }
}

impl From<&str> for UserMessage {
    fn from(text: &str) -> Self {
        Self {
            text: text.to_string(),
            image_attachments: Vec::new(),
        }
    }
}

fn create_initial_user_message(
    text: String,
    image_attachments: Vec<ImageAttachment>,
) -> Option<UserMessage> {
    if text.is_empty() && image_attachments.is_empty() {
        None
    } else {
        Some(UserMessage {
            text,
            image_attachments,
        })
    }
}

impl ChatWidget {
    /// Synchronize the bottom-pane "task running" indicator with the current lifecycles.
    ///
    /// The bottom pane only has one running flag, but this module treats it as a derived state of
    /// both the agent turn lifecycle and MCP startup lifecycle.
    fn update_task_running_state(&mut self) {
        self.bottom_pane
            .set_task_running(self.agent_turn_running || self.mcp_startup_status.is_some());
    }
    fn flush_answer_stream_with_separator(&mut self) {
        if let Some(mut controller) = self.stream_controller.take()
            && let Some(cell) = controller.finalize()
        {
            self.add_boxed_history(cell);
        }
    }

    /// Update the status indicator header and details.
    ///
    /// Passing `None` clears any existing details.
    fn set_status(&mut self, header: String, details: Option<String>) {
        self.current_status_header = header.clone();
        self.bottom_pane.update_status(header, details);
    }

    /// Convenience wrapper around [`Self::set_status`];
    /// updates the status indicator header and clears any existing details.
    fn set_status_header(&mut self, header: String) {
        self.set_status(header, None);
    }

    fn restore_retry_status_header_if_present(&mut self) {
        if let Some(header) = self.retry_status_header.take() {
            self.set_status_header(header);
        }
    }

    // --- Small event handlers ---
    fn on_session_configured(&mut self, event: codex_core::protocol::SessionConfiguredEvent) {
        self.bottom_pane
            .set_history_metadata(event.history_log_id, event.history_entry_count);
        self.set_skills(None);
        self.conversation_id = Some(event.session_id);
        self.current_rollout_path = event.rollout_path.clone();
        let initial_messages = event.initial_messages.clone();
        let model_for_header = event.model.clone();
        self.model = Some(model_for_header.clone());
        self.session_header.set_model(&model_for_header);
        let session_info_cell = history_cell::new_session_info(
            &self.config,
            &model_for_header,
            event,
            self.show_welcome_banner,
        );
        self.apply_session_info_cell(session_info_cell);

        if let Some(messages) = initial_messages {
            self.replay_initial_messages(messages);
        }
        // Ask codex-core to enumerate custom prompts for this session.
        self.submit_op(Op::ListCustomPrompts);
        self.submit_op(Op::ListSkills {
            cwds: Vec::new(),
            force_reload: false,
        });
        if let Some(user_message) = self.initial_user_message.take() {
            self.submit_user_message(user_message);
        }
        if !self.suppress_session_configured_redraw {
            self.request_redraw();
        }
    }

    fn set_skills(&mut self, skills: Option<Vec<SkillMetadata>>) {
        self.bottom_pane.set_skills(skills);
    }

    fn set_skills_from_response(&mut self, response: &ListSkillsResponseEvent) {
        let skills = skills_for_cwd(&self.config.cwd, &response.skills);
        self.set_skills(Some(skills));
    }

    pub(crate) fn open_feedback_note(
        &mut self,
        category: crate::app_event::FeedbackCategory,
        include_logs: bool,
    ) {
        // Build a fresh snapshot at the time of opening the note overlay.
        let snapshot = self.feedback.snapshot(self.conversation_id);
        let rollout = if include_logs {
            self.current_rollout_path.clone()
        } else {
            None
        };
        let view = crate::bottom_pane::FeedbackNoteView::new(
            category,
            snapshot,
            rollout,
            self.app_event_tx.clone(),
            include_logs,
            self.config.language,
        );
        self.bottom_pane.show_view(Box::new(view));
        self.request_redraw();
    }

    pub(crate) fn open_feedback_consent(&mut self, category: crate::app_event::FeedbackCategory) {
        let params = crate::bottom_pane::feedback_upload_consent_params(
            self.app_event_tx.clone(),
            category,
            self.current_rollout_path.clone(),
            self.config.language,
        );
        self.bottom_pane.show_selection_view(params);
        self.request_redraw();
    }

    fn on_agent_message(&mut self, message: String) {
        // If we have a stream_controller, then the final agent message is redundant and will be a
        // duplicate of what has already been streamed.
        if self.stream_controller.is_none() {
            self.handle_streaming_delta(message);
        }
        self.flush_answer_stream_with_separator();
        self.handle_stream_finished();
        self.request_redraw();
    }

    fn on_agent_message_delta(&mut self, delta: String) {
        self.handle_streaming_delta(delta);
    }

    fn on_agent_reasoning_delta(&mut self, delta: String) {
        // For reasoning deltas, do not stream to history. Accumulate the
        // current reasoning block and extract the first bold element
        // (between **/**) as the chunk header. Show this header as status.
        self.reasoning_buffer.push_str(&delta);

        if let Some(header) = extract_first_bold(&self.reasoning_buffer) {
            // Update the shimmer header to the extracted reasoning chunk header.
            self.set_status_header(header);
        } else {
            // Fallback while we don't yet have a bold header: leave existing header as-is.
        }
        self.request_redraw();
    }

    fn on_agent_reasoning_final(&mut self) {
        // At the end of a reasoning block, record transcript-only content.
        self.full_reasoning_buffer.push_str(&self.reasoning_buffer);
        if !self.full_reasoning_buffer.is_empty() {
            let cell =
                history_cell::new_reasoning_summary_block(self.full_reasoning_buffer.clone());
            self.add_boxed_history(cell);
        }
        self.reasoning_buffer.clear();
        self.full_reasoning_buffer.clear();
        self.request_redraw();
    }

    fn on_reasoning_section_break(&mut self) {
        // Start a new reasoning block for header extraction and accumulate transcript.
        self.full_reasoning_buffer.push_str(&self.reasoning_buffer);
        self.full_reasoning_buffer.push_str("\n\n");
        self.reasoning_buffer.clear();
    }

    // Raw reasoning uses the same flow as summarized reasoning

    fn on_task_started(&mut self) {
        self.pending_request_user_input.clear();
        self.agent_turn_running = true;
        self.bottom_pane.clear_quit_shortcut_hint();
        self.quit_shortcut_expires_at = None;
        self.quit_shortcut_key = None;
        self.update_task_running_state();
        self.retry_status_header = None;
        self.bottom_pane.set_interrupt_hint_visible(true);
        self.set_status_header(tr(self.config.language, "chatwidget.status.running").to_string());
        self.full_reasoning_buffer.clear();
        self.reasoning_buffer.clear();
        self.request_redraw();
    }

    fn on_task_complete(&mut self, last_agent_message: Option<String>) {
        // If a stream is currently active, finalize it.
        self.flush_answer_stream_with_separator();
        // Mark task stopped and request redraw now that all content is in history.
        self.agent_turn_running = false;
        self.update_task_running_state();
        self.running_commands.clear();
        self.pending_request_user_input.clear();
        self.suppressed_exec_calls.clear();
        self.last_unified_wait = None;
        self.request_redraw();

        // If there is a queued user message, send exactly one now to begin the next turn.
        self.maybe_send_next_queued_input();
        // Emit a notification when the turn completes (suppressed if focused).
        self.notify(Notification::AgentTurnComplete {
            response: last_agent_message.unwrap_or_default(),
        });

        self.maybe_show_pending_rate_limit_prompt();

        if let Some(action) = self.sdd_pending_git_action.take() {
            let failed = self.sdd_git_action_failed;
            self.sdd_git_action_failed = false;
            if failed {
                self.add_error_message(
                    tr(self.config.language, "chatwidget.sdd.git_failed").to_string(),
                );
                return;
            }
            match action {
                SddGitPendingAction::CreateBranchForPlan { description } => {
                    let workflow = self
                        .sdd_state
                        .as_ref()
                        .map_or(SddWorkflow::Standard, |state| state.workflow);
                    let prompt = self.build_sdd_plan_prompt(&description, workflow);
                    self.submit_user_message(prompt.into());
                    let plan_request_hint = if workflow == SddWorkflow::Parallels {
                        tr(
                            self.config.language,
                            "chatwidget.sdd.plan_request_hint_parallels",
                        )
                        .to_string()
                    } else {
                        tr(self.config.language, "chatwidget.sdd.plan_request_hint").to_string()
                    };
                    self.add_info_message(
                        tr(self.config.language, "chatwidget.sdd.plan_request_sent").to_string(),
                        Some(plan_request_hint),
                    );
                    self.open_sdd_plan_options();
                }
                SddGitPendingAction::FinalizeMerge => {
                    self.sdd_state = None;
                    self.restore_sdd_planning_after_workflow();
                    self.add_info_message(
                        tr(self.config.language, "chatwidget.sdd.merge_completed").to_string(),
                        None,
                    );
                }
                SddGitPendingAction::AbandonBranch => {
                    self.sdd_state = None;
                    self.restore_sdd_planning_after_workflow();
                    self.sdd_new_session_after_cleanup = true;
                    self.add_info_message(
                        tr(self.config.language, "chatwidget.sdd.branch_deleted").to_string(),
                        None,
                    );
                }
            }
        }

        if self.sdd_open_plan_options_after_task {
            self.sdd_open_plan_options_after_task = false;
            if matches!(
                self.sdd_state,
                Some(SddDevelopState {
                    stage: SddDevelopStage::AwaitPlanDecision,
                    ..
                })
            ) {
                self.open_sdd_plan_options();
            }
        }

        if self.sdd_new_session_after_cleanup {
            self.sdd_new_session_after_cleanup = false;
            self.app_event_tx.send(AppEvent::NewSession);
        }
    }

    pub(crate) fn set_token_info(&mut self, info: Option<TokenUsageInfo>) {
        match info {
            Some(info) => self.apply_token_info(info),
            None => {
                self.bottom_pane.set_context_window(None, None);
                self.bottom_pane.set_token_usage(None);
                self.token_info = None;
                self.last_api_token_usage = None;
            }
        }
    }

    fn apply_token_info(&mut self, info: TokenUsageInfo) {
        let total_usage = info.total_token_usage.clone();
        let last_usage = info.last_token_usage.clone();
        let percent = self.context_used_percent(&info);
        let used_tokens = self.context_used_tokens(&info, percent.is_some());
        self.bottom_pane.set_context_window(percent, used_tokens);
        self.capture_last_api_usage(&last_usage);
        self.refresh_token_usage_display(&total_usage);
        self.token_info = Some(info);
    }

    fn capture_last_api_usage(&mut self, usage: &TokenUsage) {
        if usage.input_tokens != 0
            || usage.cached_input_tokens != 0
            || usage.output_tokens != 0
            || usage.reasoning_output_tokens != 0
        {
            self.last_api_token_usage = Some(usage.clone());
        }
    }

    fn refresh_token_usage_display(&mut self, total_usage: &TokenUsage) {
        let last_usage = self.last_api_token_usage.clone().unwrap_or_default();
        if total_usage.is_zero() && last_usage.is_zero() {
            self.bottom_pane.set_token_usage(None);
            return;
        }

        let split = split_total_and_last(total_usage, &last_usage);
        self.bottom_pane.set_token_usage(Some(split));
    }

    fn context_used_percent(&self, info: &TokenUsageInfo) -> Option<i64> {
        info.model_context_window
            .or(self.config.model_context_window)
            .map(|window| info.last_token_usage.percent_of_context_window_used(window))
    }

    fn context_used_tokens(&self, info: &TokenUsageInfo, percent_known: bool) -> Option<i64> {
        if percent_known {
            return None;
        }

        Some(info.total_token_usage.tokens_in_context_window())
    }

    fn restore_pre_review_token_info(&mut self) {
        if let Some(saved) = self.pre_review_token_info.take() {
            self.last_api_token_usage = self.pre_review_last_api_token_usage.take().unwrap_or(None);
            match saved {
                Some(info) => self.apply_token_info(info),
                None => {
                    self.bottom_pane.set_context_window(None, None);
                    self.bottom_pane.set_token_usage(None);
                    self.token_info = None;
                }
            }
        }
    }

    pub(crate) fn on_rate_limit_snapshot(&mut self, snapshot: Option<RateLimitSnapshot>) {
        if let Some(mut snapshot) = snapshot {
            if snapshot.credits.is_none() {
                snapshot.credits = self
                    .rate_limit_snapshot
                    .as_ref()
                    .and_then(|display| display.credits.as_ref())
                    .map(|credits| CreditsSnapshot {
                        has_credits: credits.has_credits,
                        unlimited: credits.unlimited,
                        balance: credits.balance.clone(),
                    });
            }

            self.plan_type = snapshot.plan_type.or(self.plan_type);

            let warnings = self.rate_limit_warnings.take_warnings(
                snapshot
                    .secondary
                    .as_ref()
                    .map(|window| window.used_percent),
                snapshot
                    .secondary
                    .as_ref()
                    .and_then(|window| window.window_minutes),
                snapshot.primary.as_ref().map(|window| window.used_percent),
                snapshot
                    .primary
                    .as_ref()
                    .and_then(|window| window.window_minutes),
                self.config.language,
            );

            let high_usage = snapshot
                .secondary
                .as_ref()
                .map(|w| w.used_percent >= RATE_LIMIT_SWITCH_PROMPT_THRESHOLD)
                .unwrap_or(false)
                || snapshot
                    .primary
                    .as_ref()
                    .map(|w| w.used_percent >= RATE_LIMIT_SWITCH_PROMPT_THRESHOLD)
                    .unwrap_or(false);

            if high_usage
                && !self.rate_limit_switch_prompt_hidden()
                && self.current_model() != Some(NUDGE_MODEL_SLUG)
                && !matches!(
                    self.rate_limit_switch_prompt,
                    RateLimitSwitchPromptState::Shown
                )
            {
                self.rate_limit_switch_prompt = RateLimitSwitchPromptState::Pending;
            }

            let display = crate::status::rate_limit_snapshot_display(
                &snapshot,
                Local::now(),
                self.config.language,
            );
            self.rate_limit_snapshot = Some(display);

            if !warnings.is_empty() {
                for warning in warnings {
                    self.add_to_history(history_cell::new_warning_event(warning));
                }
                self.request_redraw();
            }
        } else {
            self.rate_limit_snapshot = None;
        }
    }
    /// Finalize any active exec as failed and stop/clear agent-turn UI state.
    ///
    /// This does not clear MCP startup tracking, because MCP startup can overlap with turn cleanup
    /// and should continue to drive the bottom-pane running indicator while it is in progress.
    fn finalize_turn(&mut self) {
        // Ensure any spinner is replaced by a red ✗ and flushed into history.
        self.finalize_active_cell_as_failed();
        // Reset running state and clear streaming buffers.
        self.agent_turn_running = false;
        self.update_task_running_state();
        self.running_commands.clear();
        self.suppressed_exec_calls.clear();
        self.last_unified_wait = None;
        self.stream_controller = None;
        self.maybe_show_pending_rate_limit_prompt();
    }

    fn on_error(&mut self, message: String) {
        if self.sdd_pending_git_action.is_some() {
            self.sdd_git_action_failed = true;
        }
        self.finalize_turn();
        self.pending_request_user_input.clear();
        self.interrupts.take_request_user_input();
        self.add_to_history(history_cell::new_error_event(message));
        self.request_redraw();

        // After an error ends the turn, try sending the next queued input.
        self.maybe_send_next_queued_input();
    }

    fn on_warning(&mut self, message: impl Into<String>) {
        self.add_to_history(history_cell::new_warning_event(message.into()));
        self.request_redraw();
    }

    fn on_mcp_startup_update(&mut self, ev: McpStartupUpdateEvent) {
        let mut status = self.mcp_startup_status.take().unwrap_or_default();
        if let McpStartupStatus::Failed { error } = &ev.status {
            self.on_warning(error);
        }
        status.insert(ev.server, ev.status);
        self.mcp_startup_status = Some(status);
        self.update_task_running_state();
        let language = self.config.language;
        if let Some(current) = &self.mcp_startup_status {
            let total = current.len();
            let mut starting: Vec<_> = current
                .iter()
                .filter_map(|(name, state)| {
                    if matches!(state, McpStartupStatus::Starting) {
                        Some(name)
                    } else {
                        None
                    }
                })
                .collect();
            starting.sort();
            if let Some(first) = starting.first() {
                let completed = total.saturating_sub(starting.len());
                let max_to_show = 3;
                let mut to_show: Vec<String> = starting
                    .iter()
                    .take(max_to_show)
                    .map(ToString::to_string)
                    .collect();
                if starting.len() > max_to_show {
                    to_show.push("…".to_string());
                }
                let header = if total > 1 {
                    let completed = completed.to_string();
                    let total = total.to_string();
                    let servers = to_show.join(", ");
                    tr_args(
                        language,
                        "chatwidget.mcp.starting_servers",
                        &[
                            ("completed", &completed),
                            ("total", &total),
                            ("servers", &servers),
                        ],
                    )
                } else {
                    tr_args(
                        language,
                        "chatwidget.mcp.starting_server_single",
                        &[("server", first)],
                    )
                };
                self.set_status_header(header);
            }
        }
        self.request_redraw();
    }

    fn on_mcp_startup_complete(&mut self, ev: McpStartupCompleteEvent) {
        let mut parts = Vec::new();
        let language = self.config.language;
        if !ev.failed.is_empty() {
            let failed_servers: Vec<_> = ev.failed.iter().map(|f| f.server.clone()).collect();
            let servers = failed_servers.join(", ");
            let failed_message = tr_args(
                language,
                "chatwidget.mcp.startup_failed",
                &[("servers", &servers)],
            );
            parts.push(failed_message);
        }
        if !ev.cancelled.is_empty() {
            let servers = ev.cancelled.join(", ");
            let message = tr_args(
                language,
                "chatwidget.mcp.startup_interrupted",
                &[("servers", &servers)],
            );
            self.on_warning(message);
        }
        if !parts.is_empty() {
            let details = parts.join("; ");
            let message = tr_args(
                language,
                "chatwidget.mcp.startup_incomplete",
                &[("details", &details)],
            );
            self.on_warning(message);
        }

        self.mcp_startup_status = None;
        self.update_task_running_state();
        self.maybe_send_next_queued_input();
        self.request_redraw();
    }

    /// Handle a turn aborted due to user interrupt (Esc).
    /// When there are queued user messages, restore them into the composer
    /// separated by newlines rather than auto‑submitting the next one.
    fn on_interrupted_turn(&mut self, reason: TurnAbortReason) {
        let queued_request_user_input = self.interrupts.take_request_user_input();
        self.pending_request_user_input
            .extend(queued_request_user_input);

        // Finalize, log a gentle prompt, and clear running state.
        self.finalize_turn();

        if reason != TurnAbortReason::ReviewEnded {
            let message = tr(self.config.language, "chatwidget.stream.interrupted").to_string();
            self.add_to_history(history_cell::new_error_event(message));
        }

        // If any messages were queued during the task, restore them into the composer.
        if !self.queued_user_messages.is_empty() {
            let queued_text = self
                .queued_user_messages
                .iter()
                .map(|m| m.text.clone())
                .collect::<Vec<_>>()
                .join("\n");
            let existing_text = self.bottom_pane.composer_text();
            let combined = if existing_text.is_empty() {
                queued_text
            } else if queued_text.is_empty() {
                existing_text
            } else {
                format!("{queued_text}\n{existing_text}")
            };
            self.bottom_pane.set_composer_text(combined);
            // Clear the queue and update the status indicator list.
            self.queued_user_messages.clear();
            self.refresh_queued_user_messages();
        }

        if reason == TurnAbortReason::Interrupted {
            let pending_requests: Vec<_> = self.pending_request_user_input.drain(..).collect();
            for request in pending_requests {
                let lines = self.request_user_input_history_lines(&request, true);
                self.add_boxed_history(Box::new(PlainHistoryCell::new(lines)));
            }
        } else {
            self.pending_request_user_input.clear();
        }

        self.request_redraw();
    }

    fn on_plan_update(&mut self, update: UpdatePlanArgs) {
        self.add_to_history(history_cell::new_plan_update(update, self.config.language));
    }

    fn on_exec_approval_request(&mut self, id: String, ev: ExecApprovalRequestEvent) {
        let id2 = id.clone();
        let ev2 = ev.clone();
        self.defer_or_handle(
            |q| q.push_exec_approval(id, ev),
            |s| s.handle_exec_approval_now(id2, ev2),
        );
    }

    fn on_apply_patch_approval_request(&mut self, id: String, ev: ApplyPatchApprovalRequestEvent) {
        let id2 = id.clone();
        let ev2 = ev.clone();
        self.defer_or_handle(
            |q| q.push_apply_patch_approval(id, ev),
            |s| s.handle_apply_patch_approval_now(id2, ev2),
        );
    }

    fn on_elicitation_request(&mut self, ev: ElicitationRequestEvent) {
        let ev2 = ev.clone();
        self.defer_or_handle(
            |q| q.push_elicitation(ev),
            |s| s.handle_elicitation_request_now(ev2),
        );
    }

    fn on_request_user_input(&mut self, ev: RequestUserInputEvent) {
        let ev2 = ev.clone();
        self.defer_or_handle(
            |q| q.push_user_input(ev),
            |s| s.handle_request_user_input_now(ev2),
        );
    }

    fn on_exec_command_begin(&mut self, ev: ExecCommandBeginEvent) {
        self.flush_answer_stream_with_separator();
        let ev2 = ev.clone();
        self.defer_or_handle(|q| q.push_exec_begin(ev), |s| s.handle_exec_begin_now(ev2));
    }

    fn on_exec_command_output_delta(&mut self, ev: ExecCommandOutputDeltaEvent) {
        let Some(cell) = self
            .active_cell
            .as_mut()
            .and_then(|c| c.as_any_mut().downcast_mut::<ExecCell>())
        else {
            return;
        };

        if cell.append_output(&ev.call_id, std::str::from_utf8(&ev.chunk).unwrap_or("")) {
            self.bump_active_cell_revision();
            self.request_redraw();
        }
    }

    fn on_terminal_interaction(&mut self, _ev: TerminalInteractionEvent) {
        // TODO: Handle once design is ready
    }

    fn on_patch_apply_begin(&mut self, event: PatchApplyBeginEvent) {
        self.add_to_history(history_cell::new_patch_event(
            event.changes,
            &self.config.cwd,
            self.config.language,
        ));
    }

    fn on_view_image_tool_call(&mut self, event: ViewImageToolCallEvent) {
        self.flush_answer_stream_with_separator();
        self.add_to_history(history_cell::new_view_image_tool_call(
            event.path,
            &self.config.cwd,
            self.config.language,
        ));
        self.request_redraw();
    }

    fn on_patch_apply_end(&mut self, event: codex_core::protocol::PatchApplyEndEvent) {
        let ev2 = event.clone();
        self.defer_or_handle(
            |q| q.push_patch_end(event),
            |s| s.handle_patch_apply_end_now(ev2),
        );
    }

    fn on_exec_command_end(&mut self, ev: ExecCommandEndEvent) {
        if self.sdd_pending_git_action.is_some()
            && ev.source == ExecCommandSource::Agent
            && ev.exit_code != 0
        {
            self.sdd_git_action_failed = true;
        }
        let ev2 = ev.clone();
        self.defer_or_handle(|q| q.push_exec_end(ev), |s| s.handle_exec_end_now(ev2));
    }

    fn on_mcp_tool_call_begin(&mut self, ev: McpToolCallBeginEvent) {
        let ev2 = ev.clone();
        self.defer_or_handle(|q| q.push_mcp_begin(ev), |s| s.handle_mcp_begin_now(ev2));
    }

    fn on_mcp_tool_call_end(&mut self, ev: McpToolCallEndEvent) {
        let ev2 = ev.clone();
        self.defer_or_handle(|q| q.push_mcp_end(ev), |s| s.handle_mcp_end_now(ev2));
    }

    fn on_web_search_begin(&mut self, _ev: WebSearchBeginEvent) {
        self.flush_answer_stream_with_separator();
    }

    fn on_web_search_end(&mut self, ev: WebSearchEndEvent) {
        self.flush_answer_stream_with_separator();
        self.add_to_history(history_cell::new_web_search_call(
            ev.query,
            self.config.language,
        ));
    }

    fn on_get_history_entry_response(
        &mut self,
        event: codex_core::protocol::GetHistoryEntryResponseEvent,
    ) {
        let codex_core::protocol::GetHistoryEntryResponseEvent {
            offset,
            log_id,
            entry,
        } = event;
        self.bottom_pane
            .on_history_entry_response(log_id, offset, entry.map(|e| e.text));
    }

    fn on_shutdown_complete(&mut self) {
        self.request_immediate_exit();
    }

    fn on_turn_diff(&mut self, unified_diff: String) {
        debug!("TurnDiffEvent: {unified_diff}");
    }

    fn on_deprecation_notice(&mut self, event: DeprecationNoticeEvent) {
        let DeprecationNoticeEvent { summary, details } = event;
        self.add_to_history(history_cell::new_deprecation_notice(summary, details));
        self.request_redraw();
    }

    fn on_background_event(&mut self, message: String) {
        debug!("BackgroundEvent: {message}");
        self.bottom_pane.ensure_status_indicator();
        self.bottom_pane.set_interrupt_hint_visible(true);
        self.set_status_header(message);
    }

    fn on_collab_spawn_begin(&mut self, ev: CollabAgentSpawnBeginEvent) {
        let message = tr_args(
            self.config.language,
            "chatwidget.collab.spawn_begin",
            &[("call_id", ev.call_id.as_str())],
        );
        self.on_collab_event(message);
    }

    fn on_collab_spawn_end(&mut self, ev: CollabAgentSpawnEndEvent) {
        let status = self.collab_status_label(&ev.status);
        let message = if let Some(receiver_thread_id) = ev.new_thread_id {
            let receiver_id = receiver_thread_id.to_string();
            tr_args(
                self.config.language,
                "chatwidget.collab.spawn_end_with_agent",
                &[
                    ("receiver_id", receiver_id.as_str()),
                    ("status", status.as_str()),
                ],
            )
        } else {
            tr_args(
                self.config.language,
                "chatwidget.collab.spawn_end_without_agent",
                &[("status", status.as_str())],
            )
        };
        self.on_collab_event(message);
    }

    fn on_collab_interaction_begin(&mut self, ev: CollabAgentInteractionBeginEvent) {
        let receiver_id = ev.receiver_thread_id.to_string();
        let message = tr_args(
            self.config.language,
            "chatwidget.collab.interaction_begin",
            &[("receiver_id", receiver_id.as_str())],
        );
        self.on_collab_event(message);
    }

    fn on_collab_interaction_end(&mut self, ev: CollabAgentInteractionEndEvent) {
        let receiver_id = ev.receiver_thread_id.to_string();
        let status = self.collab_status_label(&ev.status);
        let message = tr_args(
            self.config.language,
            "chatwidget.collab.interaction_end",
            &[
                ("receiver_id", receiver_id.as_str()),
                ("status", status.as_str()),
            ],
        );
        self.on_collab_event(message);
    }

    fn on_collab_waiting_begin(&mut self, ev: CollabWaitingBeginEvent) {
        let receiver_id = ev
            .receiver_thread_ids
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        let receiver_id = if receiver_id.is_empty() {
            "-".to_string()
        } else {
            receiver_id
        };
        let message = tr_args(
            self.config.language,
            "chatwidget.collab.waiting_begin",
            &[("receiver_id", receiver_id.as_str())],
        );
        self.on_collab_event(message);
    }

    fn on_collab_waiting_end(&mut self, ev: CollabWaitingEndEvent) {
        let mut statuses: Vec<(String, String)> = ev
            .statuses
            .iter()
            .map(|(thread_id, status)| (thread_id.to_string(), self.collab_status_label(status)))
            .collect();
        statuses.sort_by(|(left, _), (right, _)| left.cmp(right));

        let (receiver_id, status) = match statuses.as_slice() {
            [] => ("-".to_string(), "-".to_string()),
            [(receiver_id, status)] => (receiver_id.clone(), status.clone()),
            _ => {
                let receiver_id = statuses
                    .iter()
                    .map(|(thread_id, _)| thread_id.clone())
                    .collect::<Vec<_>>()
                    .join(", ");
                let status = statuses
                    .iter()
                    .map(|(thread_id, status)| format!("{thread_id}: {status}"))
                    .collect::<Vec<_>>()
                    .join("; ");
                (receiver_id, status)
            }
        };
        let message = tr_args(
            self.config.language,
            "chatwidget.collab.waiting_end",
            &[
                ("receiver_id", receiver_id.as_str()),
                ("status", status.as_str()),
            ],
        );
        self.on_collab_event(message);
    }

    fn on_collab_close_begin(&mut self, ev: CollabCloseBeginEvent) {
        let receiver_id = ev.receiver_thread_id.to_string();
        let message = tr_args(
            self.config.language,
            "chatwidget.collab.close_begin",
            &[("receiver_id", receiver_id.as_str())],
        );
        self.on_collab_event(message);
    }

    fn on_collab_close_end(&mut self, ev: CollabCloseEndEvent) {
        let receiver_id = ev.receiver_thread_id.to_string();
        let status = self.collab_status_label(&ev.status);
        let message = tr_args(
            self.config.language,
            "chatwidget.collab.close_end",
            &[
                ("receiver_id", receiver_id.as_str()),
                ("status", status.as_str()),
            ],
        );
        self.on_collab_event(message);
    }

    fn on_collab_event(&mut self, message: String) {
        self.flush_answer_stream_with_separator();
        self.add_to_history(history_cell::new_info_event(message, None));
        self.request_redraw();
    }

    fn collab_status_label(&self, status: &AgentStatus) -> String {
        let key = match status {
            AgentStatus::PendingInit => "chatwidget.collab.status.pending_init",
            AgentStatus::Running => "chatwidget.collab.status.running",
            AgentStatus::Completed(_) => "chatwidget.collab.status.completed",
            AgentStatus::Errored(error) if is_timeout_error(error) => {
                "chatwidget.collab.status.timed_out"
            }
            AgentStatus::Errored(_) => "chatwidget.collab.status.errored",
            AgentStatus::Shutdown => "chatwidget.collab.status.shutdown",
            AgentStatus::NotFound => "chatwidget.collab.status.not_found",
        };
        tr(self.config.language, key).to_string()
    }

    fn on_undo_started(&mut self, event: UndoStartedEvent) {
        self.bottom_pane.ensure_status_indicator();
        self.bottom_pane.set_interrupt_hint_visible(false);
        let message = event
            .message
            .unwrap_or_else(|| tr(self.config.language, "chatwidget.undo.in_progress").to_string());
        self.set_status_header(message);
    }

    fn on_undo_completed(&mut self, event: UndoCompletedEvent) {
        let UndoCompletedEvent { success, message } = event;
        self.bottom_pane.hide_status_indicator();
        let message = message.unwrap_or_else(|| {
            if success {
                tr(self.config.language, "chatwidget.undo.completed").to_string()
            } else {
                tr(self.config.language, "chatwidget.undo.failed").to_string()
            }
        });
        if success {
            self.add_info_message(message, None);
        } else {
            self.add_error_message(message);
        }
    }

    fn on_stream_error(&mut self, message: String, additional_details: Option<String>) {
        if self.retry_status_header.is_none() {
            self.retry_status_header = Some(self.current_status_header.clone());
        }
        self.set_status(message, additional_details);
    }

    /// Periodic tick to commit at most one queued line to history with a small delay,
    /// animating the output.
    pub(crate) fn on_commit_tick(&mut self) {
        if let Some(controller) = self.stream_controller.as_mut() {
            let (cell, is_idle) = controller.on_commit_tick();
            if let Some(cell) = cell {
                self.bottom_pane.hide_status_indicator();
                self.add_boxed_history(cell);
                self.request_redraw();
            }
            if is_idle {
                self.app_event_tx.send(AppEvent::StopCommitAnimation);
            }
        }
    }

    fn flush_interrupt_queue(&mut self) {
        let mut mgr = std::mem::take(&mut self.interrupts);
        mgr.flush_all(self);
        self.interrupts = mgr;
    }

    #[inline]
    fn defer_or_handle(
        &mut self,
        push: impl FnOnce(&mut InterruptManager),
        handle: impl FnOnce(&mut Self),
    ) {
        // Preserve deterministic FIFO across queued interrupts: once anything
        // is queued due to an active write cycle, continue queueing until the
        // queue is flushed to avoid reordering (e.g., ExecEnd before ExecBegin).
        if self.stream_controller.is_some() || !self.interrupts.is_empty() {
            push(&mut self.interrupts);
        } else {
            handle(self);
        }
    }

    fn handle_stream_finished(&mut self) {
        if self.task_complete_pending {
            self.bottom_pane.hide_status_indicator();
            self.task_complete_pending = false;
        }
        // A completed stream indicates non-exec content was just inserted.
        self.flush_interrupt_queue();
    }

    #[inline]
    fn handle_streaming_delta(&mut self, delta: String) {
        // Before streaming agent content, flush any active exec cell group.
        let mut needs_redraw = self.active_cell.is_some();
        self.flush_active_cell();

        if self.stream_controller.is_none() {
            if self.needs_final_message_separator {
                let elapsed_seconds = self
                    .bottom_pane
                    .status_widget()
                    .map(super::status_indicator_widget::StatusIndicatorWidget::elapsed_seconds);
                self.add_to_history(history_cell::FinalMessageSeparator::new(
                    elapsed_seconds,
                    self.config.language,
                ));
                self.needs_final_message_separator = false;
                needs_redraw = true;
            }
            // Streaming must not capture the current viewport width: width-derived wraps are
            // applied later, at render time, so the transcript can reflow on resize.
            self.stream_controller = Some(StreamController::new());
        }
        if let Some(controller) = self.stream_controller.as_mut()
            && controller.push(&delta)
        {
            self.app_event_tx.send(AppEvent::StartCommitAnimation);
        }
        if needs_redraw {
            self.request_redraw();
        }
    }

    pub(crate) fn handle_exec_end_now(&mut self, ev: ExecCommandEndEvent) {
        let running = self.running_commands.remove(&ev.call_id);
        if self.suppressed_exec_calls.remove(&ev.call_id) {
            return;
        }
        let (command, parsed, source) = match running {
            Some(rc) => (rc.command, rc.parsed_cmd, rc.source),
            None => (ev.command.clone(), ev.parsed_cmd.clone(), ev.source),
        };
        let is_unified_exec_interaction =
            matches!(source, ExecCommandSource::UnifiedExecInteraction);

        let needs_new = self
            .active_cell
            .as_ref()
            .map(|cell| cell.as_any().downcast_ref::<ExecCell>().is_none())
            .unwrap_or(true);
        if needs_new {
            self.flush_active_cell();
            self.active_cell = Some(Box::new(new_active_exec_command(
                ev.call_id.clone(),
                command,
                parsed,
                source,
                ev.interaction_input.clone(),
                self.config.animations,
                self.config.language,
            )));
        }

        if let Some(cell) = self
            .active_cell
            .as_mut()
            .and_then(|c| c.as_any_mut().downcast_mut::<ExecCell>())
        {
            let output = if is_unified_exec_interaction {
                CommandOutput {
                    exit_code: ev.exit_code,
                    formatted_output: String::new(),
                    aggregated_output: String::new(),
                }
            } else {
                CommandOutput {
                    exit_code: ev.exit_code,
                    formatted_output: ev.formatted_output.clone(),
                    aggregated_output: ev.aggregated_output.clone(),
                }
            };
            cell.complete_call(&ev.call_id, output, ev.duration);
            if cell.should_flush() {
                self.flush_active_cell();
            } else {
                self.bump_active_cell_revision();
                self.request_redraw();
            }
        }
    }

    pub(crate) fn handle_patch_apply_end_now(
        &mut self,
        event: codex_core::protocol::PatchApplyEndEvent,
    ) {
        // If the patch was successful, just let the "Edited" block stand.
        // Otherwise, add a failure block.
        if !event.success {
            self.add_to_history(history_cell::new_patch_apply_failure(
                event.stderr,
                self.config.language,
            ));
        }
    }

    pub(crate) fn handle_exec_approval_now(&mut self, id: String, ev: ExecApprovalRequestEvent) {
        self.flush_answer_stream_with_separator();
        let command = shlex::try_join(ev.command.iter().map(String::as_str))
            .unwrap_or_else(|_| ev.command.join(" "));
        self.notify(Notification::ExecApprovalRequested { command });

        let request = ApprovalRequest::Exec {
            id,
            command: ev.command,
            reason: ev.reason,
            proposed_execpolicy_amendment: ev.proposed_execpolicy_amendment,
        };
        self.bottom_pane
            .push_approval_request(request, &self.config.features);
        self.request_redraw();
    }

    pub(crate) fn handle_apply_patch_approval_now(
        &mut self,
        id: String,
        ev: ApplyPatchApprovalRequestEvent,
    ) {
        self.flush_answer_stream_with_separator();

        let request = ApprovalRequest::ApplyPatch {
            id,
            reason: ev.reason,
            changes: ev.changes.clone(),
            cwd: self.config.cwd.clone(),
        };
        self.bottom_pane
            .push_approval_request(request, &self.config.features);
        self.request_redraw();
        self.notify(Notification::EditApprovalRequested {
            cwd: self.config.cwd.clone(),
            changes: ev.changes.keys().cloned().collect(),
        });
    }

    pub(crate) fn handle_elicitation_request_now(&mut self, ev: ElicitationRequestEvent) {
        self.flush_answer_stream_with_separator();

        self.notify(Notification::ElicitationRequested {
            server_name: ev.server_name.clone(),
        });

        let request = ApprovalRequest::McpElicitation {
            server_name: ev.server_name,
            request_id: ev.id,
            message: ev.message,
        };
        self.bottom_pane
            .push_approval_request(request, &self.config.features);
        self.request_redraw();
    }

    fn request_user_input_history_lines(
        &self,
        ev: &RequestUserInputEvent,
        interrupted: bool,
    ) -> Vec<Line<'static>> {
        let language = self.config.language;
        let question_count = ev.questions.len();
        let question_count_text = question_count.to_string();
        let unanswered = tr_args(
            language,
            "request_user_input.progress.unanswered",
            &[("count", &question_count_text)],
        );

        let mut lines: Vec<Line<'static>> = vec![
            vec![
                "• ".dim(),
                "request_user_input".bold(),
                format!(" [{}]", ev.call_id).dim(),
                " ".into(),
                unanswered.dim(),
            ]
            .into(),
        ];

        let detail = if interrupted {
            tr(language, "chatwidget.stream.interrupted")
        } else {
            tr(language, "request_user_input.hint.answer_questions")
        };
        lines.push(vec!["  ".into(), detail.dim()].into());

        if ev.questions.is_empty() {
            lines.push(
                vec![
                    "  ".into(),
                    tr(language, "request_user_input.progress.none").dim(),
                ]
                .into(),
            );
            return lines;
        }

        for (index, question) in ev.questions.iter().enumerate() {
            let question_index = (index + 1).to_string();
            let progress = tr_args(
                language,
                "request_user_input.progress.question",
                &[("index", &question_index), ("total", &question_count_text)],
            );
            lines.push(vec!["  ".into(), progress.dim()].into());

            if !question.header.trim().is_empty() {
                lines.push(Line::from(format!("    {}", question.header)));
            }
            if !question.question.trim().is_empty() {
                lines.push(Line::from(format!("    {}", question.question)));
            }

            if let Some(options) = &question.options {
                if options.is_empty() {
                    lines.push(
                        vec![
                            "    ".into(),
                            tr(language, "request_user_input.empty.options").dim(),
                        ]
                        .into(),
                    );
                } else {
                    for option in options {
                        if option.description.trim().is_empty() {
                            lines.push(Line::from(format!("      - {}", option.label)));
                        } else {
                            lines.push(Line::from(format!(
                                "      - {}: {}",
                                option.label, option.description
                            )));
                        }
                    }
                }
            }
        }

        lines
    }

    pub(crate) fn handle_request_user_input_now(&mut self, ev: RequestUserInputEvent) {
        self.flush_answer_stream_with_separator();
        self.bottom_pane.push_user_input_request(ev);
        self.request_redraw();
    }

    pub(crate) fn handle_exec_begin_now(&mut self, ev: ExecCommandBeginEvent) {
        // Ensure the status indicator is visible while the command runs.
        self.running_commands.insert(
            ev.call_id.clone(),
            RunningCommand {
                command: ev.command.clone(),
                parsed_cmd: ev.parsed_cmd.clone(),
                source: ev.source,
            },
        );
        let is_wait_interaction = matches!(ev.source, ExecCommandSource::UnifiedExecInteraction)
            && ev
                .interaction_input
                .as_deref()
                .map(str::is_empty)
                .unwrap_or(true);
        let command_display = ev.command.join(" ");
        let should_suppress_unified_wait = is_wait_interaction
            && self
                .last_unified_wait
                .as_ref()
                .is_some_and(|wait| wait.is_duplicate(&command_display));
        if is_wait_interaction {
            self.last_unified_wait = Some(UnifiedExecWaitState::new(command_display));
        } else {
            self.last_unified_wait = None;
        }
        if should_suppress_unified_wait {
            self.suppressed_exec_calls.insert(ev.call_id);
            return;
        }
        let interaction_input = ev.interaction_input.clone();
        if let Some(cell) = self
            .active_cell
            .as_mut()
            .and_then(|c| c.as_any_mut().downcast_mut::<ExecCell>())
            && let Some(new_exec) = cell.with_added_call(
                ev.call_id.clone(),
                ev.command.clone(),
                ev.parsed_cmd.clone(),
                ev.source,
                interaction_input.clone(),
            )
        {
            *cell = new_exec;
            self.bump_active_cell_revision();
        } else {
            self.flush_active_cell();

            self.active_cell = Some(Box::new(new_active_exec_command(
                ev.call_id.clone(),
                ev.command.clone(),
                ev.parsed_cmd,
                ev.source,
                interaction_input,
                self.config.animations,
                self.config.language,
            )));
            self.bump_active_cell_revision();
        }

        self.request_redraw();
    }

    pub(crate) fn handle_mcp_begin_now(&mut self, ev: McpToolCallBeginEvent) {
        self.flush_answer_stream_with_separator();
        self.flush_active_cell();
        self.active_cell = Some(Box::new(history_cell::new_active_mcp_tool_call(
            ev.call_id,
            ev.invocation,
            self.config.animations,
            self.config.language,
        )));
        self.bump_active_cell_revision();
        self.request_redraw();
    }
    pub(crate) fn handle_mcp_end_now(&mut self, ev: McpToolCallEndEvent) {
        self.flush_answer_stream_with_separator();

        let McpToolCallEndEvent {
            call_id,
            invocation,
            duration,
            result,
        } = ev;
        let result = result.and_then(|result| {
            serde_json::to_value(result)
                .map_err(|err| format!("failed to serialize MCP tool result: {err}"))
                .and_then(|value| {
                    serde_json::from_value::<mcp_types::CallToolResult>(value)
                        .map_err(|err| format!("failed to parse MCP tool result: {err}"))
                })
        });

        let extra_cell = match self
            .active_cell
            .as_mut()
            .and_then(|cell| cell.as_any_mut().downcast_mut::<McpToolCallCell>())
        {
            Some(cell) if cell.call_id() == call_id => cell.complete(duration, result),
            _ => {
                self.flush_active_cell();
                let mut cell = history_cell::new_active_mcp_tool_call(
                    call_id,
                    invocation,
                    self.config.animations,
                    self.config.language,
                );
                let extra_cell = cell.complete(duration, result);
                self.active_cell = Some(Box::new(cell));
                extra_cell
            }
        };

        self.flush_active_cell();
        if let Some(extra) = extra_cell {
            self.add_boxed_history(extra);
        }
    }

    pub(crate) fn new(common: ChatWidgetInit, thread_manager: Arc<ThreadManager>) -> Self {
        let ChatWidgetInit {
            config,
            frame_requester,
            app_event_tx,
            initial_prompt,
            initial_images,
            enhanced_keys_supported,
            auth_manager,
            models_manager,
            feedback,
            is_first_run,
            model,
        } = common;
        let mut config = config;
        let model = model.filter(|m| !m.trim().is_empty());
        config.model = model.clone();
        let language = config.language;
        let placeholder = example_prompt_placeholder(language);
        let codex_op_tx = spawn_agent(config.clone(), app_event_tx.clone(), thread_manager);

        let model_for_header = config
            .model
            .clone()
            .unwrap_or_else(|| DEFAULT_MODEL_DISPLAY_NAME.to_string());
        let active_cell = if model.is_none() {
            Some(Self::placeholder_session_header_cell(&config))
        } else {
            None
        };

        let initial_attachments = initial_images
            .into_iter()
            .map(ImageAttachment::LocalPath)
            .collect();

        let mut widget = Self {
            app_event_tx: app_event_tx.clone(),
            frame_requester: frame_requester.clone(),
            codex_op_tx,
            bottom_pane: BottomPane::new(BottomPaneParams {
                frame_requester,
                app_event_tx,
                has_input_focus: true,
                enhanced_keys_supported,
                placeholder_text: placeholder,
                disable_paste_burst: config.disable_paste_burst,
                animations_enabled: config.animations,
                skills: None,
                language,
            }),
            active_cell,
            active_cell_revision: 0,
            config,
            model,
            auth_manager,
            models_manager,
            session_header: SessionHeader::new(model_for_header),
            initial_user_message: create_initial_user_message(
                initial_prompt.unwrap_or_default(),
                initial_attachments,
            ),
            token_info: None,
            last_api_token_usage: None,
            rate_limit_snapshot: None,
            plan_type: None,
            rate_limit_warnings: RateLimitWarningState::default(),
            rate_limit_switch_prompt: RateLimitSwitchPromptState::default(),
            rate_limit_poller: None,
            stream_controller: None,
            running_commands: HashMap::new(),
            suppressed_exec_calls: HashSet::new(),
            last_unified_wait: None,
            task_complete_pending: false,
            agent_turn_running: false,
            mcp_startup_status: None,
            interrupts: InterruptManager::new(),
            reasoning_buffer: String::new(),
            full_reasoning_buffer: String::new(),
            current_status_header: tr(language, "chatwidget.status.working").to_string(),
            retry_status_header: None,
            conversation_id: None,
            queued_user_messages: VecDeque::new(),
            pending_request_user_input: VecDeque::new(),
            show_welcome_banner: is_first_run,
            suppress_session_configured_redraw: false,
            pending_notification: None,
            quit_shortcut_expires_at: None,
            quit_shortcut_key: None,
            is_review_mode: false,
            pre_review_token_info: None,
            pre_review_last_api_token_usage: None,
            needs_final_message_separator: false,
            last_rendered_width: std::cell::Cell::new(None),
            feedback,
            current_rollout_path: None,
            sdd_state: None,
            sdd_pending_plan_rework_prompt: None,
            sdd_open_plan_options_after_task: false,
            sdd_pending_git_action: None,
            sdd_git_action_failed: false,
            sdd_new_session_after_cleanup: false,
            sdd_spec_sdd_planning_restore: None,
        };

        widget.prefetch_rate_limits();
        widget
            .bottom_pane
            .set_steer_enabled(widget.config.features.enabled(Feature::Steer));

        widget
    }

    /// Create a ChatWidget attached to an existing conversation (e.g., a fork).
    pub(crate) fn new_from_existing(
        common: ChatWidgetInit,
        conversation: std::sync::Arc<codex_core::CodexThread>,
        session_configured: codex_core::protocol::SessionConfiguredEvent,
    ) -> Self {
        let ChatWidgetInit {
            config,
            frame_requester,
            app_event_tx,
            initial_prompt,
            initial_images,
            enhanced_keys_supported,
            auth_manager,
            models_manager,
            feedback,
            model,
            ..
        } = common;
        let model = model.filter(|m| !m.trim().is_empty());
        let language = config.language;
        let placeholder = example_prompt_placeholder(language);

        let header_model = model.unwrap_or_else(|| session_configured.model.clone());

        let codex_op_tx =
            spawn_agent_from_existing(conversation, session_configured, app_event_tx.clone());
        let status_header = tr(language, "chatwidget.status.working").to_string();

        let initial_attachments = initial_images
            .into_iter()
            .map(ImageAttachment::LocalPath)
            .collect();

        let mut widget = Self {
            app_event_tx: app_event_tx.clone(),
            frame_requester: frame_requester.clone(),
            codex_op_tx,
            bottom_pane: BottomPane::new(BottomPaneParams {
                frame_requester,
                app_event_tx,
                has_input_focus: true,
                enhanced_keys_supported,
                placeholder_text: placeholder,
                disable_paste_burst: config.disable_paste_burst,
                animations_enabled: config.animations,
                skills: None,
                language,
            }),
            active_cell: None,
            active_cell_revision: 0,
            config,
            model: Some(header_model.clone()),
            auth_manager,
            models_manager,
            session_header: SessionHeader::new(header_model),
            initial_user_message: create_initial_user_message(
                initial_prompt.unwrap_or_default(),
                initial_attachments,
            ),
            token_info: None,
            last_api_token_usage: None,
            rate_limit_snapshot: None,
            plan_type: None,
            rate_limit_warnings: RateLimitWarningState::default(),
            rate_limit_switch_prompt: RateLimitSwitchPromptState::default(),
            rate_limit_poller: None,
            stream_controller: None,
            running_commands: HashMap::new(),
            suppressed_exec_calls: HashSet::new(),
            last_unified_wait: None,
            task_complete_pending: false,
            agent_turn_running: false,
            mcp_startup_status: None,
            interrupts: InterruptManager::new(),
            reasoning_buffer: String::new(),
            full_reasoning_buffer: String::new(),
            current_status_header: status_header,
            retry_status_header: None,
            conversation_id: None,
            queued_user_messages: VecDeque::new(),
            pending_request_user_input: VecDeque::new(),
            show_welcome_banner: false,
            suppress_session_configured_redraw: true,
            pending_notification: None,
            quit_shortcut_expires_at: None,
            quit_shortcut_key: None,
            is_review_mode: false,
            pre_review_token_info: None,
            pre_review_last_api_token_usage: None,
            needs_final_message_separator: false,
            last_rendered_width: std::cell::Cell::new(None),
            feedback,
            current_rollout_path: None,
            sdd_state: None,
            sdd_pending_plan_rework_prompt: None,
            sdd_open_plan_options_after_task: false,
            sdd_pending_git_action: None,
            sdd_git_action_failed: false,
            sdd_new_session_after_cleanup: false,
            sdd_spec_sdd_planning_restore: None,
        };

        widget.prefetch_rate_limits();
        widget
            .bottom_pane
            .set_steer_enabled(widget.config.features.enabled(Feature::Steer));

        widget
    }

    pub(crate) fn handle_key_event(&mut self, key_event: KeyEvent) {
        match key_event {
            KeyEvent {
                code: KeyCode::Char(c),
                modifiers,
                kind: KeyEventKind::Press,
                ..
            } if modifiers.contains(KeyModifiers::CONTROL) && c.eq_ignore_ascii_case(&'c') => {
                self.on_ctrl_c();
                return;
            }
            KeyEvent {
                code: KeyCode::Char(c),
                modifiers,
                kind: KeyEventKind::Press,
                ..
            } if modifiers.contains(KeyModifiers::CONTROL) && c.eq_ignore_ascii_case(&'d') => {
                if self.on_ctrl_d() {
                    return;
                }
                self.bottom_pane.clear_quit_shortcut_hint();
                self.quit_shortcut_expires_at = None;
                self.quit_shortcut_key = None;
            }
            KeyEvent {
                code: KeyCode::Char(c),
                modifiers,
                kind: KeyEventKind::Press,
                ..
            } if modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                && c.eq_ignore_ascii_case(&'v') =>
            {
                match paste_image_as_data_url() {
                    Ok(clipboard) => {
                        self.attach_image_data_url(
                            clipboard.placeholder_label,
                            clipboard.data_url,
                            clipboard.info.width,
                            clipboard.info.height,
                            clipboard.info.encoded_format.label(),
                        );
                    }
                    Err(err) => {
                        tracing::warn!("failed to paste image: {err}");
                        let detail = err.to_message(self.config.language);
                        let message = tr_args(
                            self.config.language,
                            "chatwidget.clipboard.image_paste_failed",
                            &[("detail", &detail)],
                        );
                        self.add_to_history(history_cell::new_error_event(message));
                    }
                }
                return;
            }
            other if other.kind == KeyEventKind::Press => {
                self.bottom_pane.clear_quit_shortcut_hint();
                self.quit_shortcut_expires_at = None;
                self.quit_shortcut_key = None;
            }
            _ => {}
        }

        match key_event {
            KeyEvent {
                code: KeyCode::Up,
                modifiers: KeyModifiers::ALT,
                kind: KeyEventKind::Press,
                ..
            } if !self.queued_user_messages.is_empty() => {
                // Prefer the most recently queued item.
                if let Some(user_message) = self.queued_user_messages.pop_back() {
                    self.bottom_pane.set_composer_text(user_message.text);
                    self.refresh_queued_user_messages();
                    self.request_redraw();
                }
            }
            _ => {
                match self.bottom_pane.handle_key_event(key_event) {
                    InputResult::Submitted(text) => {
                        // Enter always sends messages immediately (bypasses queue check)
                        // Clear any reasoning status header when submitting a new message
                        self.reasoning_buffer.clear();
                        self.full_reasoning_buffer.clear();
                        self.set_status_header(
                            tr(self.config.language, "chatwidget.status.working").to_string(),
                        );
                        let user_message = UserMessage {
                            text,
                            image_attachments: self.bottom_pane.take_recent_submission_images(),
                        };
                        if !self.is_session_configured() {
                            self.queue_user_message(user_message);
                        } else {
                            self.submit_user_message(user_message);
                        }
                    }
                    InputResult::Queued(text) => {
                        // Tab queues the message if a task is running, otherwise submits immediately
                        let user_message = UserMessage {
                            text,
                            image_attachments: self.bottom_pane.take_recent_submission_images(),
                        };
                        self.queue_user_message(user_message);
                    }
                    InputResult::Command(cmd) => {
                        self.dispatch_command(cmd);
                    }
                    InputResult::CommandWithArgs(cmd, args) => {
                        self.dispatch_command_with_args(cmd, args);
                    }
                    InputResult::None => {}
                }
            }
        }
    }

    pub(crate) fn attach_image(&mut self, path: PathBuf) {
        tracing::info!("attach_image path={path:?}");
        self.bottom_pane.attach_image(path);
        self.request_redraw();
    }

    pub(crate) fn attach_image_data_url(
        &mut self,
        label: String,
        data_url: String,
        width: u32,
        height: u32,
        format_label: &str,
    ) {
        tracing::info!(
            "attach_image_data_url label={label} width={width} height={height} format={format_label} data_url_len={}",
            data_url.len()
        );
        self.bottom_pane
            .attach_image_data_url(label, data_url, width, height, format_label);
        self.request_redraw();
    }

    fn dispatch_command(&mut self, cmd: SlashCommand) {
        if !cmd.available_during_task() && self.bottom_pane.is_task_running() {
            let message = tr_args(
                self.config.language,
                "chatwidget.slash.disabled_during_task",
                &[("command", cmd.command())],
            );
            self.add_to_history(history_cell::new_error_event(message));
            self.request_redraw();
            return;
        }
        match cmd {
            SlashCommand::Feedback => {
                if !self.config.feedback_enabled {
                    let params = crate::bottom_pane::feedback_disabled_params(self.config.language);
                    self.bottom_pane.show_selection_view(params);
                    self.request_redraw();
                    return;
                }
                // Step 1: pick a category (UI built in feedback_view)
                let params = crate::bottom_pane::feedback_selection_params(
                    self.app_event_tx.clone(),
                    self.config.language,
                );
                self.bottom_pane.show_selection_view(params);
                self.request_redraw();
            }
            SlashCommand::New => {
                self.app_event_tx.send(AppEvent::NewSession);
            }
            SlashCommand::Resume => {
                self.app_event_tx.send(AppEvent::OpenResumePicker);
            }
            SlashCommand::Fork => {
                self.app_event_tx.send(AppEvent::OpenForkPicker);
            }
            SlashCommand::Init => {
                let init_target = self.config.cwd.join(DEFAULT_PROJECT_DOC_FILENAME);
                if init_target.exists() {
                    let filename = DEFAULT_PROJECT_DOC_FILENAME;
                    let message = tr_args(
                        self.config.language,
                        "chatwidget.slash.init_exists",
                        &[("filename", filename)],
                    );
                    self.add_info_message(message, None);
                    return;
                }
                let prompt = init_prompt(self.config.language);
                self.submit_user_message(prompt.to_string().into());
            }
            SlashCommand::Compact => {
                self.app_event_tx.send(AppEvent::CodexOp(Op::Compact));
            }
            SlashCommand::Review => {
                self.open_review_popup();
            }
            SlashCommand::Model => {
                self.open_model_popup();
            }
            SlashCommand::Lang => {
                self.open_language_popup();
            }
            SlashCommand::Agent => {
                self.app_event_tx.send(AppEvent::OpenAgentPopup);
            }
            SlashCommand::Approvals => {
                self.open_approvals_popup();
            }
            SlashCommand::ElevateSandbox => {
                #[cfg(target_os = "windows")]
                {
                    let windows_sandbox_level = WindowsSandboxLevel::from_config(&self.config);
                    let windows_degraded_sandbox_enabled =
                        matches!(windows_sandbox_level, WindowsSandboxLevel::RestrictedToken);
                    if !windows_degraded_sandbox_enabled
                        || !codex_core::windows_sandbox::ELEVATED_SANDBOX_NUX_ENABLED
                    {
                        // This command should not be visible/recognized outside degraded mode,
                        // but guard anyway in case something dispatches it directly.
                        return;
                    }

                    let Some(preset) = builtin_approval_presets()
                        .into_iter()
                        .find(|preset| preset.id == "auto")
                    else {
                        // Avoid panicking in interactive UI; treat this as a recoverable
                        // internal error.
                        self.add_error_message(
                            "Internal error: missing the 'auto' approval preset.".to_string(),
                        );
                        return;
                    };

                    if let Err(err) = self.config.approval_policy.can_set(&preset.approval) {
                        self.add_error_message(err.to_string());
                        return;
                    }

                    self.app_event_tx
                        .send(AppEvent::BeginWindowsSandboxElevatedSetup { preset });
                }
                #[cfg(not(target_os = "windows"))]
                {
                    // Not supported; on non-Windows this command should never be reachable.
                };
            }
            SlashCommand::Quit | SlashCommand::Exit => {
                self.request_quit_without_confirmation();
            }
            SlashCommand::Logout => {
                if let Err(e) = codex_core::auth::logout(
                    &self.config.codex_home,
                    self.config.cli_auth_credentials_store_mode,
                ) {
                    tracing::error!("failed to logout: {e}");
                }
                self.request_quit_without_confirmation();
            }
            // SlashCommand::Undo => {
            //     self.app_event_tx.send(AppEvent::CodexOp(Op::Undo));
            // }
            SlashCommand::Diff => {
                self.add_diff_in_progress();
                let tx = self.app_event_tx.clone();
                let language = self.config.language;
                tokio::spawn(async move {
                    let text = match get_git_diff(language).await {
                        Ok((is_git_repo, diff_text)) => {
                            if is_git_repo {
                                diff_text
                            } else {
                                tr(language, "chatwidget.diff.not_git_repo").to_string()
                            }
                        }
                        Err(e) => tr_args(
                            language,
                            "chatwidget.diff.failed",
                            &[("error", &e.to_string())],
                        ),
                    };
                    tx.send(AppEvent::DiffResult(text));
                });
            }
            SlashCommand::Clean => {
                let language = self.config.language;
                match clean_clipboard_cache(&self.config.cwd) {
                    Ok(result) => {
                        let message = if !result.found_cache_dir {
                            tr(language, "chatwidget.clean.no_codex_dir").to_string()
                        } else if result.deleted == 0 && result.failed == 0 {
                            tr(language, "chatwidget.clean.no_image_cache").to_string()
                        } else if result.failed == 0 {
                            tr_args(
                                language,
                                "chatwidget.clean.deleted",
                                &[("deleted", &result.deleted.to_string())],
                            )
                        } else {
                            tr_args(
                                language,
                                "chatwidget.clean.deleted_with_failures",
                                &[
                                    ("deleted", &result.deleted.to_string()),
                                    ("failed", &result.failed.to_string()),
                                ],
                            )
                        };
                        self.add_info_message(message, None);
                    }
                    Err(err) => {
                        let message = tr_args(
                            language,
                            "chatwidget.clean.failed",
                            &[("error", &err.to_string())],
                        );
                        self.add_to_history(history_cell::new_error_event(message));
                    }
                }
            }
            SlashCommand::Mention => {
                self.insert_str("@");
            }
            SlashCommand::Skills => {
                self.insert_str("$");
            }
            SlashCommand::Status => {
                self.add_status_output();
            }
            SlashCommand::Mcp => {
                self.add_mcp_output();
            }
            SlashCommand::Rollout => {
                if let Some(path) = self.rollout_path() {
                    self.add_info_message(
                        tr_args(
                            self.config.language,
                            "chatwidget.rollout.current_path",
                            &[("path", &path.display().to_string())],
                        ),
                        None,
                    );
                } else {
                    self.add_info_message(
                        tr(self.config.language, "chatwidget.rollout.not_available").to_string(),
                        None,
                    );
                }
            }
            SlashCommand::TestApproval => {
                use codex_core::protocol::EventMsg;
                use std::collections::HashMap;

                use codex_core::protocol::ApplyPatchApprovalRequestEvent;
                use codex_core::protocol::FileChange;

                self.app_event_tx.send(AppEvent::CodexEvent(Event {
                    id: "1".to_string(),
                    // msg: EventMsg::ExecApprovalRequest(ExecApprovalRequestEvent {
                    //     call_id: "1".to_string(),
                    //     command: vec!["git".into(), "apply".into()],
                    //     cwd: self.config.cwd.clone(),
                    //     reason: Some("test".to_string()),
                    // }),
                    msg: EventMsg::ApplyPatchApprovalRequest(ApplyPatchApprovalRequestEvent {
                        call_id: "1".to_string(),
                        turn_id: "turn-1".to_string(),
                        changes: HashMap::from([
                            (
                                PathBuf::from("/tmp/test.txt"),
                                FileChange::Add {
                                    content: "test".to_string(),
                                },
                            ),
                            (
                                PathBuf::from("/tmp/test2.txt"),
                                FileChange::Update {
                                    unified_diff: "+test\n-test2".to_string(),
                                    move_path: None,
                                },
                            ),
                        ]),
                        reason: None,
                        grant_root: Some(PathBuf::from("/tmp")),
                    }),
                }));
            }
        }
    }

    fn is_sdd_workflow_enabled(&mut self, workflow: SddWorkflow) -> bool {
        if workflow == SddWorkflow::Parallels && !self.config.features.enabled(Feature::Collab) {
            let language = self.config.language;
            self.add_info_message(
                tr(language, "chatwidget.sdd.collab_required").to_string(),
                Some(tr(language, "chatwidget.sdd.collab_required_hint").to_string()),
            );
            return false;
        }
        true
    }

    fn handle_sdd_develop_command(&mut self, description: Option<String>, workflow: SddWorkflow) {
        let language = self.config.language;
        if get_git_repo_root(&self.config.cwd).is_none() {
            self.add_info_message(
                tr(language, "chatwidget.sdd.not_git_repo").to_string(),
                None,
            );
            return;
        }
        if !self.is_sdd_workflow_enabled(workflow) {
            return;
        }

        if let Some(desc) = description
            .map(|d| d.trim().to_string())
            .filter(|d| !d.is_empty())
        {
            self.sdd_pending_plan_rework_prompt = None;
            self.sdd_open_plan_options_after_task = false;
            self.sdd_pending_git_action = None;
            self.sdd_git_action_failed = false;
            let branch_name = self.sdd_branch_name(&desc);
            self.sdd_state = Some(SddDevelopState {
                workflow,
                description: desc.clone(),
                branch_name: branch_name.clone(),
                base_branch: None,
                stage: SddDevelopStage::AwaitPlanDecision,
            });
            self.enable_sdd_planning_for_workflow();
            self.set_sdd_collaboration_mode(ModeKind::Plan);
            if workflow == SddWorkflow::Parallels {
                let prompt = self.build_sdd_plan_prompt(&desc, workflow);
                self.submit_user_message(prompt.into());
                self.add_info_message(
                    tr(language, "chatwidget.sdd.plan_request_sent").to_string(),
                    Some(tr(language, "chatwidget.sdd.plan_request_hint_parallels").to_string()),
                );
                self.open_sdd_plan_options();
                return;
            }

            let base_branch = match std::process::Command::new("git")
                .arg("rev-parse")
                .arg("--abbrev-ref")
                .arg("HEAD")
                .current_dir(&self.config.cwd)
                .output()
            {
                Ok(output) if output.status.success() => {
                    let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
                    if branch.is_empty() || branch == "HEAD" {
                        self.add_error_message(
                            tr(language, "chatwidget.sdd.base_branch_unknown").to_string(),
                        );
                        return;
                    }
                    branch
                }
                _ => {
                    self.add_error_message(
                        tr(language, "chatwidget.sdd.base_branch_unknown").to_string(),
                    );
                    return;
                }
            };

            if let Some(state) = self.sdd_state.as_mut() {
                state.base_branch = Some(base_branch.clone());
            }
            self.sdd_pending_git_action =
                Some(SddGitPendingAction::CreateBranchForPlan { description: desc });
            self.sdd_git_action_failed = false;
            self.submit_op(Op::SddGitAction {
                action: SddGitAction::CreateBranch {
                    name: branch_name,
                    base: base_branch,
                },
            });
            self.add_info_message(
                tr(language, "chatwidget.sdd.branch_create_started").to_string(),
                None,
            );
            return;
        }

        match &self.sdd_state {
            Some(SddDevelopState {
                stage: SddDevelopStage::AwaitPlanDecision,
                ..
            }) => {
                self.add_info_message(
                    tr(language, "chatwidget.sdd.plan_stage").to_string(),
                    Some(tr(language, "chatwidget.sdd.use_popup_hint").to_string()),
                );
                self.open_sdd_plan_options();
            }
            Some(SddDevelopState {
                stage: SddDevelopStage::AwaitDevDecision,
                ..
            }) => {
                self.add_info_message(
                    tr(language, "chatwidget.sdd.dev_stage").to_string(),
                    Some(tr(language, "chatwidget.sdd.use_popup_hint").to_string()),
                );
                self.open_sdd_dev_options();
            }
            None => {
                let require_description = if workflow == SddWorkflow::Parallels {
                    tr(language, "chatwidget.sdd.require_description_parallels")
                } else {
                    tr(language, "chatwidget.sdd.require_description")
                };
                self.add_info_message(require_description.to_string(), None);
            }
        }
    }

    pub(crate) fn start_sdd_workflow(&mut self, parallels: bool) {
        let workflow = if parallels {
            SddWorkflow::Parallels
        } else {
            SddWorkflow::Standard
        };
        self.handle_sdd_develop_command(None, workflow);
    }

    pub(crate) fn open_sdd_workflow_popup(&mut self) {
        let language = self.config.language;
        let items = vec![
            SelectionItem {
                name: tr(language, "chatwidget.sdd_workflow_popup.standard").to_string(),
                description: Some(
                    tr(language, "chatwidget.sdd_workflow_popup.standard_desc").to_string(),
                ),
                actions: vec![Box::new(|tx| {
                    tx.send(AppEvent::StartSddWorkflow { parallels: false });
                })],
                dismiss_on_select: true,
                ..Default::default()
            },
            SelectionItem {
                name: tr(language, "chatwidget.sdd_workflow_popup.parallels").to_string(),
                description: Some(
                    tr(language, "chatwidget.sdd_workflow_popup.parallels_desc").to_string(),
                ),
                actions: vec![Box::new(|tx| {
                    tx.send(AppEvent::StartSddWorkflow { parallels: true });
                })],
                dismiss_on_select: true,
                ..Default::default()
            },
        ];
        self.bottom_pane.show_selection_view(SelectionViewParams {
            title: Some(tr(language, "chatwidget.sdd_workflow_popup.title").to_string()),
            subtitle: Some(tr(language, "chatwidget.sdd_workflow_popup.subtitle").to_string()),
            footer_hint: Some(standard_popup_hint_line(language)),
            items,
            header: Box::new(()),
            ..Default::default()
        });
    }

    pub(crate) fn open_sdd_plan_options(&mut self) {
        let language = self.config.language;
        if !matches!(
            self.sdd_state,
            Some(SddDevelopState {
                stage: SddDevelopStage::AwaitPlanDecision,
                ..
            })
        ) {
            self.add_info_message(
                tr(language, "chatwidget.sdd.no_plan_pending").to_string(),
                None,
            );
            return;
        }
        let items = vec![
            SelectionItem {
                name: tr(language, "chatwidget.sdd.option.approve_plan").to_string(),
                description: Some(
                    tr(language, "chatwidget.sdd.option.approve_plan_desc").to_string(),
                ),
                actions: vec![Box::new(|tx| tx.send(AppEvent::SddPlanApproved))],
                dismiss_on_select: true,
                ..Default::default()
            },
            SelectionItem {
                name: tr(language, "chatwidget.sdd.option.request_changes").to_string(),
                description: Some(
                    tr(language, "chatwidget.sdd.option.request_changes_desc").to_string(),
                ),
                actions: vec![Box::new(|tx| tx.send(AppEvent::SddPlanRework))],
                dismiss_on_select: true,
                ..Default::default()
            },
        ];
        self.bottom_pane.show_selection_view(SelectionViewParams {
            title: Some(tr(language, "chatwidget.sdd.plan_options.title").to_string()),
            footer_hint: Some(standard_popup_hint_line(self.config.language)),
            items,
            header: Box::new(()),
            ..Default::default()
        });
        self.request_redraw();
    }

    pub(crate) fn open_sdd_dev_options(&mut self) {
        let language = self.config.language;
        if !matches!(
            self.sdd_state,
            Some(SddDevelopState {
                stage: SddDevelopStage::AwaitDevDecision,
                ..
            })
        ) {
            self.add_info_message(
                tr(language, "chatwidget.sdd.no_dev_branch").to_string(),
                None,
            );
            return;
        }
        let items = vec![
            SelectionItem {
                name: tr(language, "chatwidget.sdd.option.merge_pr").to_string(),
                description: Some(tr(language, "chatwidget.sdd.option.merge_pr_desc").to_string()),
                actions: vec![Box::new(|tx| tx.send(AppEvent::SddDevMergeBranch))],
                dismiss_on_select: true,
                ..Default::default()
            },
            SelectionItem {
                name: tr(language, "chatwidget.sdd.option.continue_changes").to_string(),
                description: Some(
                    tr(language, "chatwidget.sdd.option.continue_changes_desc").to_string(),
                ),
                actions: vec![Box::new(|tx| tx.send(AppEvent::SddDevRequestMoreChanges))],
                dismiss_on_select: true,
                ..Default::default()
            },
            SelectionItem {
                name: tr(language, "chatwidget.sdd.option.abandon").to_string(),
                description: Some(tr(language, "chatwidget.sdd.option.abandon_desc").to_string()),
                actions: vec![Box::new(|tx| tx.send(AppEvent::SddDevAbandonBranch))],
                dismiss_on_select: true,
                ..Default::default()
            },
        ];
        self.bottom_pane.show_selection_view(SelectionViewParams {
            title: Some(tr(language, "chatwidget.sdd.dev_options.title").to_string()),
            footer_hint: Some(standard_popup_hint_line(self.config.language)),
            items,
            header: Box::new(()),
            ..Default::default()
        });
        self.request_redraw();
    }

    pub(crate) async fn on_sdd_plan_approved(&mut self) {
        let language = self.config.language;
        let (description, workflow) = match self.sdd_state.as_ref() {
            Some(state) if state.stage == SddDevelopStage::AwaitPlanDecision => {
                (state.description.clone(), state.workflow)
            }
            Some(_) => {
                self.add_info_message(
                    tr(language, "chatwidget.sdd.plan_stage_required").to_string(),
                    None,
                );
                return;
            }
            None => {
                self.add_info_message(
                    tr(language, "chatwidget.sdd.no_active_plan").to_string(),
                    None,
                );
                return;
            }
        };

        self.sdd_pending_plan_rework_prompt = None;
        self.sdd_open_plan_options_after_task = false;
        self.set_sdd_collaboration_mode(ModeKind::Default);

        if workflow == SddWorkflow::Parallels {
            if let Some(base_branch) = current_branch_name(&self.config.cwd).await
                && let Some(state) = self.sdd_state.as_mut()
            {
                state.base_branch = Some(base_branch);
            }
            if let Some(state) = self.sdd_state.as_mut() {
                state.stage = SddDevelopStage::AwaitDevDecision;
            }
            let prompt = self.build_sdd_exec_prompt(&description, workflow);
            self.submit_user_message(prompt.into());
            self.add_info_message(
                tr(language, "chatwidget.sdd.exec_sent").to_string(),
                Some(tr(language, "chatwidget.sdd.exec_sent_hint_parallels").to_string()),
            );
            self.open_sdd_dev_options();
            return;
        }

        if self
            .sdd_state
            .as_ref()
            .is_some_and(|state| state.base_branch.is_none())
        {
            let base_branch = match current_branch_name(&self.config.cwd).await {
                Some(branch) => branch,
                None => {
                    self.add_error_message(
                        tr(language, "chatwidget.sdd.base_branch_unknown").to_string(),
                    );
                    return;
                }
            };
            if let Some(state) = self.sdd_state.as_mut() {
                state.base_branch = Some(base_branch);
            }
        }

        if let Some(state) = self.sdd_state.as_mut() {
            state.stage = SddDevelopStage::AwaitDevDecision;
        }
        let prompt = self.build_sdd_exec_prompt(&description, workflow);
        self.submit_user_message(prompt.into());
        self.add_info_message(
            tr(language, "chatwidget.sdd.exec_sent").to_string(),
            Some(tr(language, "chatwidget.sdd.exec_sent_hint").to_string()),
        );
        self.open_sdd_dev_options();
    }

    pub(crate) fn on_sdd_plan_rework(&mut self) {
        let language = self.config.language;
        let Some(state) = self.sdd_state.as_ref() else {
            self.add_info_message(
                tr(language, "chatwidget.sdd.no_active_plan_rework").to_string(),
                None,
            );
            return;
        };
        if state.stage != SddDevelopStage::AwaitPlanDecision {
            self.add_info_message(
                tr(language, "chatwidget.sdd.not_in_plan_stage").to_string(),
                None,
            );
            return;
        }
        self.sdd_pending_git_action = None;
        self.sdd_git_action_failed = false;
        let prompt = self.build_sdd_plan_rework_prompt(&state.description);
        self.sdd_pending_plan_rework_prompt = Some(prompt);
        self.set_composer_text(String::new());
        self.add_info_message(
            tr(language, "chatwidget.sdd.plan_rework_ready").to_string(),
            Some(tr(language, "chatwidget.sdd.plan_rework_hint").to_string()),
        );
        self.request_redraw();
    }

    pub(crate) fn on_sdd_request_more_changes(&mut self) {
        let language = self.config.language;
        let Some(state) = self.sdd_state.as_ref() else {
            self.add_info_message(
                tr(language, "chatwidget.sdd.no_active_branch_changes").to_string(),
                None,
            );
            return;
        };
        if state.stage != SddDevelopStage::AwaitDevDecision {
            self.add_info_message(
                tr(language, "chatwidget.sdd.not_in_dev_stage").to_string(),
                None,
            );
            return;
        }
        let prefill = format!(
            "{}\n{}\n\n{}\n",
            tr(language, "chatwidget.sdd.continue_prompt_intro"),
            state.description,
            tr(language, "chatwidget.sdd.continue_prompt_details")
        );
        self.set_composer_text(prefill);
        self.add_info_message(
            tr(language, "chatwidget.sdd.continue_prompt_ready").to_string(),
            Some(tr(language, "chatwidget.sdd.continue_prompt_hint").to_string()),
        );
        self.request_redraw();
    }

    pub(crate) fn on_sdd_merge_branch(&mut self) {
        let language = self.config.language;
        let Some(state) = self.sdd_state.take() else {
            self.add_info_message(
                tr(language, "chatwidget.sdd.no_active_branch_merge").to_string(),
                None,
            );
            return;
        };
        if state.stage != SddDevelopStage::AwaitDevDecision {
            self.sdd_state = Some(state);
            self.add_info_message(
                tr(language, "chatwidget.sdd.not_in_dev_stage_merge").to_string(),
                None,
            );
            return;
        }
        if state.workflow == SddWorkflow::Parallels {
            let prompt =
                self.build_sdd_merge_prompt(&state.description, &state.branch_name, state.workflow);
            self.sdd_pending_git_action = None;
            self.sdd_git_action_failed = false;
            self.submit_user_message(prompt.into());
            self.restore_sdd_planning_after_workflow();
            self.add_info_message(
                tr(language, "chatwidget.sdd.merge_guidance_sent").to_string(),
                Some(tr(language, "chatwidget.sdd.merge_guidance_hint_parallels").to_string()),
            );
            return;
        }
        let commit_message = self.sdd_commit_message(&state.description);
        let branch_name = state.branch_name.clone();
        let base_branch = match state.base_branch.clone() {
            Some(base_branch) => base_branch,
            None => {
                self.sdd_state = Some(state);
                self.add_error_message(
                    tr(language, "chatwidget.sdd.base_branch_unknown").to_string(),
                );
                return;
            }
        };
        self.sdd_state = Some(state);
        self.sdd_pending_git_action = Some(SddGitPendingAction::FinalizeMerge);
        self.sdd_git_action_failed = false;
        self.submit_op(Op::SddGitAction {
            action: SddGitAction::FinalizeMerge {
                name: branch_name,
                base: base_branch,
                commit_message,
            },
        });
        self.add_info_message(
            tr(language, "chatwidget.sdd.merge_started").to_string(),
            None,
        );
    }

    pub(crate) fn on_sdd_abandon_branch(&mut self) {
        let language = self.config.language;
        let Some(state) = self.sdd_state.take() else {
            self.add_info_message(
                tr(language, "chatwidget.sdd.no_active_branch_abandon").to_string(),
                None,
            );
            return;
        };
        if state.stage != SddDevelopStage::AwaitDevDecision {
            self.sdd_state = Some(state);
            self.add_info_message(
                tr(language, "chatwidget.sdd.not_in_dev_stage_abandon").to_string(),
                None,
            );
            return;
        }
        let branch_name = state.branch_name.clone();
        let base_branch = match state.base_branch.clone() {
            Some(base_branch) => base_branch,
            None => {
                self.sdd_state = Some(state);
                self.add_error_message(
                    tr(language, "chatwidget.sdd.base_branch_unknown").to_string(),
                );
                return;
            }
        };
        self.sdd_state = Some(state);
        self.sdd_pending_git_action = Some(SddGitPendingAction::AbandonBranch);
        self.sdd_git_action_failed = false;
        self.submit_op(Op::SddGitAction {
            action: SddGitAction::AbandonBranch {
                name: branch_name,
                base: base_branch,
            },
        });
        self.add_info_message(
            tr(language, "chatwidget.sdd.branch_delete_started").to_string(),
            None,
        );
    }

    fn build_sdd_plan_prompt(&self, description: &str, workflow: SddWorkflow) -> String {
        let template = match workflow {
            SddWorkflow::Standard => sdd_plan_prompt_template(self.config.language),
            SddWorkflow::Parallels => sdd_plan_parallels_prompt_template(self.config.language),
        }
        .trim();
        let description_block = if workflow == SddWorkflow::Parallels {
            match self.sdd_state.as_ref() {
                Some(state) => format!(
                    "{}\n{description}\n{}\n{}",
                    tr(self.config.language, "chatwidget.sdd.requirement_label"),
                    tr(self.config.language, "chatwidget.sdd.branch_label"),
                    state.branch_name
                ),
                None => format!(
                    "{}\n{description}",
                    tr(self.config.language, "chatwidget.sdd.requirement_label")
                ),
            }
        } else {
            format!(
                "{}\n{description}",
                tr(self.config.language, "chatwidget.sdd.requirement_label")
            )
        };
        if template.is_empty() {
            self.inject_sdd_planning_prompt(description_block)
        } else {
            self.inject_sdd_planning_prompt(format!("{template}\n\n{description_block}"))
        }
    }

    fn sdd_branch_name(&self, description: &str) -> String {
        let slug = Self::sdd_slug(description);
        format!("{SDD_BRANCH_PREFIX}{slug}")
    }

    fn sdd_commit_message(&self, description: &str) -> String {
        let slug = Self::sdd_slug(description);
        format!("sdd: {slug}")
    }

    fn sdd_slug(description: &str) -> String {
        let mut slug = String::new();
        let mut prev_dash = false;
        for ch in description.chars() {
            if ch.is_ascii_alphanumeric() {
                slug.push(ch.to_ascii_lowercase());
                prev_dash = false;
            } else if !prev_dash {
                slug.push('-');
                prev_dash = true;
            }
            if slug.len() >= 32 {
                break;
            }
        }
        let slug = slug.trim_matches('-').to_string();
        if slug.is_empty() {
            "task".to_string()
        } else {
            slug
        }
    }

    fn build_sdd_plan_rework_prompt(&self, _description: &str) -> String {
        self.inject_sdd_planning_prompt(
            tr(self.config.language, "chatwidget.sdd.plan_rework_prompt").to_string(),
        )
    }

    fn build_sdd_exec_prompt(&self, description: &str, workflow: SddWorkflow) -> String {
        let template = match workflow {
            SddWorkflow::Standard => sdd_exec_prompt_template(self.config.language),
            SddWorkflow::Parallels => sdd_exec_parallels_prompt_template(self.config.language),
        }
        .trim();
        let description_block = format!(
            "{}\n{description}",
            tr(self.config.language, "chatwidget.sdd.requirement_label")
        );
        if template.is_empty() {
            self.inject_sdd_planning_prompt(description_block)
        } else {
            self.inject_sdd_planning_prompt(format!("{template}\n\n{description_block}"))
        }
    }

    fn build_sdd_merge_prompt(
        &self,
        description: &str,
        branch_name: &str,
        workflow: SddWorkflow,
    ) -> String {
        let template = match workflow {
            SddWorkflow::Standard => sdd_merge_prompt_template(self.config.language),
            SddWorkflow::Parallels => sdd_merge_parallels_prompt_template(self.config.language),
        }
        .trim();
        let context_block = format!(
            "{}\n{description}\n{}\n{branch_name}",
            tr(self.config.language, "chatwidget.sdd.requirement_label"),
            tr(self.config.language, "chatwidget.sdd.branch_label")
        );
        if template.is_empty() {
            self.inject_sdd_planning_prompt(context_block)
        } else {
            self.inject_sdd_planning_prompt(format!("{template}\n\n{context_block}"))
        }
    }

    fn enable_sdd_planning_for_workflow(&mut self) {
        if self.sdd_spec_sdd_planning_restore.is_none() {
            self.sdd_spec_sdd_planning_restore = Some(self.config.spec.sdd_planning);
        }
        if self.config.spec.sdd_planning {
            return;
        }
        self.config.spec.sdd_planning = true;
        self.submit_op(Op::OverrideTurnContext {
            cwd: None,
            approval_policy: None,
            sandbox_policy: None,
            windows_sandbox_level: None,
            model: None,
            effort: None,
            summary: None,
            collaboration_mode: None,
            personality: None,
            spec_parallel_priority: None,
            spec_sdd_planning: Some(true),
        });
    }

    fn restore_sdd_planning_after_workflow(&mut self) {
        let Some(previous) = self.sdd_spec_sdd_planning_restore.take() else {
            return;
        };
        if previous == self.config.spec.sdd_planning {
            return;
        }
        self.config.spec.sdd_planning = previous;
        self.submit_op(Op::OverrideTurnContext {
            cwd: None,
            approval_policy: None,
            sandbox_policy: None,
            windows_sandbox_level: None,
            model: None,
            effort: None,
            summary: None,
            collaboration_mode: None,
            personality: None,
            spec_parallel_priority: None,
            spec_sdd_planning: Some(previous),
        });
    }

    fn inject_sdd_planning_prompt(&self, message: String) -> String {
        if self.config.spec.sdd_planning {
            return message;
        }
        let planning_prompt = tr(self.config.language, "prompt.spec.sdd_planning").trim();
        if planning_prompt.is_empty() {
            return message;
        }
        format!("{planning_prompt}\n\n{message}")
    }
    fn dispatch_command_with_args(&mut self, cmd: SlashCommand, args: String) {
        if !cmd.available_during_task() && self.bottom_pane.is_task_running() {
            let message = format!(
                "'/{}' is disabled while a task is in progress.",
                cmd.command()
            );
            self.add_to_history(history_cell::new_error_event(message));
            self.request_redraw();
            return;
        }

        let trimmed = args.trim();
        match cmd {
            SlashCommand::Review if !trimmed.is_empty() => {
                self.submit_op(Op::Review {
                    review_request: ReviewRequest {
                        target: ReviewTarget::Custom {
                            instructions: trimmed.to_string(),
                        },
                        user_facing_hint: None,
                    },
                });
            }
            _ => self.dispatch_command(cmd),
        }
    }

    pub(crate) fn handle_paste(&mut self, text: String) {
        self.bottom_pane.handle_paste(text);
    }

    // Returns true if caller should skip rendering this frame (a future frame is scheduled).
    pub(crate) fn handle_paste_burst_tick(&mut self, frame_requester: FrameRequester) -> bool {
        if self.bottom_pane.flush_paste_burst_if_due() {
            // A paste just flushed; request an immediate redraw and skip this frame.
            self.request_redraw();
            true
        } else if self.bottom_pane.is_in_paste_burst() {
            // While capturing a burst, schedule a follow-up tick and skip this frame
            // to avoid redundant renders between ticks.
            frame_requester.schedule_frame_in(
                crate::bottom_pane::ChatComposer::recommended_paste_flush_delay(),
            );
            true
        } else {
            false
        }
    }

    fn flush_active_cell(&mut self) {
        if let Some(active) = self.active_cell.take() {
            self.needs_final_message_separator = true;
            self.app_event_tx.send(AppEvent::InsertHistoryCell(active));
        }
    }

    fn add_to_history(&mut self, cell: impl HistoryCell + 'static) {
        self.add_boxed_history(Box::new(cell));
    }

    fn add_boxed_history(&mut self, cell: Box<dyn HistoryCell>) {
        // Keep the placeholder session header as the active cell until real session info arrives,
        // so we can merge headers instead of committing a duplicate box to history.
        let keep_placeholder_header_active = !self.is_session_configured()
            && self
                .active_cell
                .as_ref()
                .is_some_and(|c| c.as_any().is::<history_cell::SessionHeaderHistoryCell>());

        if !keep_placeholder_header_active && !cell.display_lines(u16::MAX).is_empty() {
            // Only break exec grouping if the cell renders visible lines.
            self.flush_active_cell();
            self.needs_final_message_separator = true;
        }
        self.app_event_tx.send(AppEvent::InsertHistoryCell(cell));
    }

    #[allow(dead_code)] // Used in tests
    fn queue_user_message(&mut self, user_message: UserMessage) {
        if !self.is_session_configured() || self.bottom_pane.is_task_running() {
            self.queued_user_messages.push_back(user_message);
            self.refresh_queued_user_messages();
        } else {
            self.submit_user_message(user_message);
        }
    }

    fn submit_user_message(&mut self, user_message: UserMessage) {
        let UserMessage {
            text,
            image_attachments,
        } = user_message;
        if text.is_empty()
            && image_attachments.is_empty()
            && self.sdd_pending_plan_rework_prompt.is_none()
        {
            return;
        }

        // Special-case: "!cmd" executes a local shell command instead of sending to the model.
        if let Some(stripped) = text.strip_prefix('!') {
            let cmd = stripped.trim();
            if cmd.is_empty() {
                self.app_event_tx.send(AppEvent::InsertHistoryCell(Box::new(
                    history_cell::new_info_event(
                        user_shell_command_help_title(self.config.language).to_string(),
                        Some(user_shell_command_help_hint(self.config.language).to_string()),
                    ),
                )));
                return;
            }
            self.submit_op(Op::RunUserShellCommand {
                command: cmd.to_string(),
            });
            return;
        }

        let (send_text, display_text) = self.apply_sdd_plan_rework_prefix(text);

        let mut items: Vec<UserInput> = Vec::new();

        if !send_text.is_empty() {
            // TODO: Thread text element ranges from the composer input. Empty keeps old behavior.
            items.push(UserInput::Text {
                text: send_text,
                text_elements: Vec::new(),
            });
        }

        for attachment in image_attachments {
            match attachment {
                ImageAttachment::LocalPath(path) => {
                    items.push(UserInput::LocalImage { path });
                }
                ImageAttachment::DataUrl(data_url) => {
                    items.push(UserInput::Image {
                        image_url: data_url,
                    });
                }
            }
        }

        if let Some(skills) = self.bottom_pane.skills() {
            let skill_mentions = find_skill_mentions(&display_text, skills);
            for skill in skill_mentions {
                items.push(UserInput::Skill {
                    name: skill.name.clone(),
                    path: skill.path.clone(),
                });
            }
        }

        if items.is_empty() {
            return;
        }

        self.codex_op_tx
            .send(Op::UserInput {
                items,
                final_output_json_schema: None,
            })
            .unwrap_or_else(|e| {
                tracing::error!("failed to send message: {e}");
            });

        // Persist the text to cross-session message history.
        if !display_text.is_empty() {
            self.codex_op_tx
                .send(Op::AddToHistory {
                    text: display_text.clone(),
                })
                .unwrap_or_else(|e| {
                    tracing::error!("failed to send AddHistory op: {e}");
                });
        }

        // Only show the text portion in conversation history.
        if !display_text.is_empty() {
            self.add_to_history(history_cell::new_user_prompt(display_text));
        }
        self.needs_final_message_separator = false;
    }

    fn apply_sdd_plan_rework_prefix(&mut self, text: String) -> (String, String) {
        if let Some(prefix) = self.sdd_pending_plan_rework_prompt.take() {
            self.sdd_open_plan_options_after_task = true;
            let mut combined = prefix;
            combined.push_str(&text);
            (combined, text)
        } else {
            (text.clone(), text)
        }
    }

    /// Replay a subset of initial events into the UI to seed the transcript when
    /// resuming an existing session. This approximates the live event flow and
    /// is intentionally conservative: only safe-to-replay items are rendered to
    /// avoid triggering side effects. Event ids are passed as `None` to
    /// distinguish replayed events from live ones.
    fn replay_initial_messages(&mut self, events: Vec<EventMsg>) {
        for msg in events {
            if matches!(msg, EventMsg::SessionConfigured(_)) {
                continue;
            }
            // `id: None` indicates a synthetic/fake id coming from replay.
            self.dispatch_event_msg(None, msg, true);
        }
    }

    pub(crate) fn handle_codex_event(&mut self, event: Event) {
        let Event { id, msg } = event;
        self.dispatch_event_msg(Some(id), msg, false);
    }

    /// Dispatch a protocol `EventMsg` to the appropriate handler.
    ///
    /// `id` is `Some` for live events and `None` for replayed events from
    /// `replay_initial_messages()`. Callers should treat `None` as a "fake" id
    /// that must not be used to correlate follow-up actions.
    fn dispatch_event_msg(&mut self, id: Option<String>, msg: EventMsg, from_replay: bool) {
        let is_stream_error = matches!(&msg, EventMsg::StreamError(_));
        if !is_stream_error {
            self.restore_retry_status_header_if_present();
        }

        match msg {
            EventMsg::AgentMessageDelta(_)
            | EventMsg::PlanDelta(_)
            | EventMsg::AgentReasoningDelta(_)
            | EventMsg::TerminalInteraction(_)
            | EventMsg::ExecCommandOutputDelta(_) => {}
            _ => {
                tracing::trace!("handle_codex_event: {:?}", msg);
            }
        }

        match msg {
            EventMsg::SessionConfigured(e) => self.on_session_configured(e),
            EventMsg::ThreadNameUpdated(_) => {}
            EventMsg::AgentMessage(AgentMessageEvent { message }) => self.on_agent_message(message),
            EventMsg::AgentMessageDelta(AgentMessageDeltaEvent { delta }) => {
                self.on_agent_message_delta(delta)
            }
            EventMsg::AgentReasoningDelta(AgentReasoningDeltaEvent { delta })
            | EventMsg::AgentReasoningRawContentDelta(AgentReasoningRawContentDeltaEvent {
                delta,
            }) => self.on_agent_reasoning_delta(delta),
            EventMsg::AgentReasoning(AgentReasoningEvent { .. }) => self.on_agent_reasoning_final(),
            EventMsg::AgentReasoningRawContent(AgentReasoningRawContentEvent { text }) => {
                self.on_agent_reasoning_delta(text);
                self.on_agent_reasoning_final();
            }
            EventMsg::AgentReasoningSectionBreak(_) => self.on_reasoning_section_break(),
            EventMsg::TurnStarted(_) => self.on_task_started(),
            EventMsg::TurnComplete(TurnCompleteEvent { last_agent_message }) => {
                self.on_task_complete(last_agent_message)
            }
            EventMsg::TokenCount(ev) => {
                self.set_token_info(ev.info);
                self.on_rate_limit_snapshot(ev.rate_limits);
            }
            EventMsg::Warning(WarningEvent { message }) => self.on_warning(message),
            EventMsg::Error(ErrorEvent { message, .. }) => self.on_error(message),
            EventMsg::McpStartupUpdate(ev) => self.on_mcp_startup_update(ev),
            EventMsg::McpStartupComplete(ev) => self.on_mcp_startup_complete(ev),
            EventMsg::TurnAborted(ev) => match ev.reason {
                TurnAbortReason::Interrupted => {
                    self.on_interrupted_turn(ev.reason);
                }
                TurnAbortReason::Replaced => {
                    let message = tr(self.config.language, "chatwidget.task.replaced").to_string();
                    self.on_error(message)
                }
                TurnAbortReason::ReviewEnded => {
                    self.on_interrupted_turn(ev.reason);
                }
            },
            EventMsg::PlanUpdate(update) => self.on_plan_update(update),
            EventMsg::PlanDelta(_) => {}
            EventMsg::ExecApprovalRequest(ev) => {
                // For replayed events, synthesize an empty id (these should not occur).
                self.on_exec_approval_request(id.unwrap_or_default(), ev)
            }
            EventMsg::ApplyPatchApprovalRequest(ev) => {
                self.on_apply_patch_approval_request(id.unwrap_or_default(), ev)
            }
            EventMsg::ElicitationRequest(ev) => {
                self.on_elicitation_request(ev);
            }
            EventMsg::RequestUserInput(ev) => {
                self.on_request_user_input(ev);
            }
            EventMsg::ExecCommandBegin(ev) => self.on_exec_command_begin(ev),
            EventMsg::TerminalInteraction(delta) => self.on_terminal_interaction(delta),
            EventMsg::ExecCommandOutputDelta(delta) => self.on_exec_command_output_delta(delta),
            EventMsg::PatchApplyBegin(ev) => self.on_patch_apply_begin(ev),
            EventMsg::PatchApplyEnd(ev) => self.on_patch_apply_end(ev),
            EventMsg::ExecCommandEnd(ev) => self.on_exec_command_end(ev),
            EventMsg::ViewImageToolCall(ev) => self.on_view_image_tool_call(ev),
            EventMsg::McpToolCallBegin(ev) => self.on_mcp_tool_call_begin(ev),
            EventMsg::McpToolCallEnd(ev) => self.on_mcp_tool_call_end(ev),
            EventMsg::WebSearchBegin(ev) => self.on_web_search_begin(ev),
            EventMsg::WebSearchEnd(ev) => self.on_web_search_end(ev),
            EventMsg::GetHistoryEntryResponse(ev) => self.on_get_history_entry_response(ev),
            EventMsg::McpListToolsResponse(ev) => self.on_list_mcp_tools(ev),
            EventMsg::ListCustomPromptsResponse(ev) => self.on_list_custom_prompts(ev),
            EventMsg::ListSkillsResponse(ev) => self.on_list_skills(ev),
            EventMsg::ListRemoteSkillsResponse(_) | EventMsg::RemoteSkillDownloaded(_) => {}
            EventMsg::SkillsUpdateAvailable => {
                self.submit_op(Op::ListSkills {
                    cwds: Vec::new(),
                    force_reload: true,
                });
            }
            EventMsg::ShutdownComplete => self.on_shutdown_complete(),
            EventMsg::TurnDiff(TurnDiffEvent { unified_diff }) => self.on_turn_diff(unified_diff),
            EventMsg::DeprecationNotice(ev) => self.on_deprecation_notice(ev),
            EventMsg::BackgroundEvent(BackgroundEventEvent { message }) => {
                self.on_background_event(message)
            }
            EventMsg::UndoStarted(ev) => self.on_undo_started(ev),
            EventMsg::UndoCompleted(ev) => self.on_undo_completed(ev),
            EventMsg::StreamError(StreamErrorEvent {
                message,
                additional_details,
                ..
            }) => self.on_stream_error(message, additional_details),
            EventMsg::UserMessage(ev) => {
                if from_replay {
                    self.on_user_message_event(ev);
                }
            }
            EventMsg::EnteredReviewMode(review_request) => {
                self.on_entered_review_mode(review_request)
            }
            EventMsg::ExitedReviewMode(review) => self.on_exited_review_mode(review),
            EventMsg::ContextCompacted(_) => self.on_agent_message(
                tr(self.config.language, "chatwidget.context_compacted").to_string(),
            ),
            EventMsg::CollabAgentSpawnBegin(ev) => self.on_collab_spawn_begin(ev),
            EventMsg::CollabAgentSpawnEnd(ev) => self.on_collab_spawn_end(ev),
            EventMsg::CollabAgentInteractionBegin(ev) => self.on_collab_interaction_begin(ev),
            EventMsg::CollabAgentInteractionEnd(ev) => self.on_collab_interaction_end(ev),
            EventMsg::CollabWaitingBegin(ev) => self.on_collab_waiting_begin(ev),
            EventMsg::CollabWaitingEnd(ev) => self.on_collab_waiting_end(ev),
            EventMsg::CollabCloseBegin(ev) => self.on_collab_close_begin(ev),
            EventMsg::CollabCloseEnd(ev) => self.on_collab_close_end(ev),
            EventMsg::CollabResumeBegin(_) | EventMsg::CollabResumeEnd(_) => {}
            EventMsg::RawResponseItem(_)
            | EventMsg::ThreadRolledBack(_)
            | EventMsg::ItemStarted(_)
            | EventMsg::ItemCompleted(_)
            | EventMsg::AgentMessageContentDelta(_)
            | EventMsg::ReasoningContentDelta(_)
            | EventMsg::ReasoningRawContentDelta(_)
            | EventMsg::DynamicToolCallRequest(_) => {}
        }
    }

    fn on_entered_review_mode(&mut self, review: ReviewRequest) {
        // Enter review mode and emit a concise banner
        if self.pre_review_token_info.is_none() {
            self.pre_review_token_info = Some(self.token_info.clone());
            self.pre_review_last_api_token_usage = Some(self.last_api_token_usage.clone());
        }
        self.is_review_mode = true;
        let hint = review.user_facing_hint.unwrap_or_else(|| {
            codex_core::review_prompts::user_facing_hint(&review.target, self.config.language)
        });
        let banner = tr_args(
            self.config.language,
            "chatwidget.review.started",
            &[("hint", &hint)],
        );
        self.add_to_history(history_cell::new_review_status_line(banner));
        self.request_redraw();
    }

    fn on_exited_review_mode(&mut self, review: ExitedReviewModeEvent) {
        // Leave review mode; if output is present, flush pending stream + show results.
        if let Some(output) = review.review_output {
            self.flush_answer_stream_with_separator();
            self.flush_interrupt_queue();
            self.flush_active_cell();

            if output.findings.is_empty() {
                let explanation = output.overall_explanation.trim().to_string();
                if explanation.is_empty() {
                    tracing::error!("Reviewer failed to output a response.");
                    self.add_to_history(history_cell::new_error_event(
                        tr(self.config.language, "chatwidget.review.no_response").to_owned(),
                    ));
                } else {
                    // Show explanation when there are no structured findings.
                    let mut rendered: Vec<ratatui::text::Line<'static>> = vec!["".into()];
                    append_markdown(&explanation, None, &mut rendered);
                    let body_cell = AgentMessageCell::new(rendered, false);
                    self.app_event_tx
                        .send(AppEvent::InsertHistoryCell(Box::new(body_cell)));
                }
            }
            // Final message is rendered as part of the AgentMessage.
        }

        self.is_review_mode = false;
        self.restore_pre_review_token_info();
        // Append a finishing banner at the end of this turn.
        self.add_to_history(history_cell::new_review_status_line(
            tr(self.config.language, "chatwidget.review.finished").to_string(),
        ));
        self.request_redraw();
    }

    fn on_user_message_event(&mut self, event: UserMessageEvent) {
        let message = event.message.trim();
        // Only show the text portion in conversation history.
        if !message.is_empty() {
            self.add_to_history(history_cell::new_user_prompt(message.to_string()));
        }

        self.needs_final_message_separator = false;
    }

    /// Exit the UI immediately without waiting for shutdown.
    ///
    /// Prefer [`Self::request_quit_without_confirmation`] for user-initiated exits;
    /// this is mainly a fallback for shutdown completion or emergency exits.
    fn request_immediate_exit(&self) {
        self.app_event_tx.send(AppEvent::Exit(ExitMode::Immediate));
    }

    /// Request a shutdown-first quit.
    ///
    /// This is used for explicit quit commands (`/quit`, `/exit`, `/logout`) and for
    /// the double-press Ctrl+C/Ctrl+D quit shortcut.
    fn request_quit_without_confirmation(&self) {
        self.app_event_tx
            .send(AppEvent::Exit(ExitMode::ShutdownFirst));
    }

    fn request_redraw(&mut self) {
        self.frame_requester.schedule_frame();
    }

    fn bump_active_cell_revision(&mut self) {
        // Wrapping avoids overflow; wraparound would require 2^64 bumps and at
        // worst causes a one-time cache-key collision.
        self.active_cell_revision = self.active_cell_revision.wrapping_add(1);
    }

    fn notify(&mut self, notification: Notification) {
        if !notification.allowed_for(&self.config.tui_notifications) {
            return;
        }
        self.pending_notification = Some(notification);
        self.request_redraw();
    }

    pub(crate) fn maybe_post_pending_notification(&mut self, tui: &mut crate::tui::Tui) {
        if let Some(notif) = self.pending_notification.take() {
            tui.notify(notif.display(self.config.language));
        }
    }

    /// Mark the active cell as failed (✗) and flush it into history.
    fn finalize_active_cell_as_failed(&mut self) {
        if let Some(mut cell) = self.active_cell.take() {
            // Insert finalized cell into history and keep grouping consistent.
            if let Some(exec) = cell.as_any_mut().downcast_mut::<ExecCell>() {
                exec.mark_failed();
            } else if let Some(tool) = cell.as_any_mut().downcast_mut::<McpToolCallCell>() {
                tool.mark_failed();
            }
            self.add_boxed_history(cell);
        }
    }

    // If idle and there are queued inputs, submit exactly one to start the next turn.
    fn maybe_send_next_queued_input(&mut self) {
        if self.bottom_pane.is_task_running() {
            return;
        }
        if let Some(user_message) = self.queued_user_messages.pop_front() {
            self.submit_user_message(user_message);
        }
        // Update the list to reflect the remaining queued messages (if any).
        self.refresh_queued_user_messages();
    }

    /// Rebuild and update the queued user messages from the current queue.
    fn refresh_queued_user_messages(&mut self) {
        let messages: Vec<String> = self
            .queued_user_messages
            .iter()
            .map(|m| m.text.clone())
            .collect();
        self.bottom_pane.set_queued_user_messages(messages);
    }

    pub(crate) fn add_diff_in_progress(&mut self) {
        self.request_redraw();
    }

    pub(crate) fn on_diff_complete(&mut self) {
        self.request_redraw();
    }

    pub(crate) fn add_status_output(&mut self) {
        let default_usage = TokenUsage::default();
        let token_info = self.token_info.as_ref();
        let total_usage = token_info
            .map(|ti| &ti.total_token_usage)
            .unwrap_or(&default_usage);
        self.add_to_history(crate::status::new_status_output(
            &self.config,
            self.auth_manager.as_ref(),
            token_info,
            total_usage,
            &self.conversation_id,
            self.rate_limit_snapshot.as_ref(),
            self.plan_type,
            Local::now(),
            self.model_display_name(),
        ));
    }
    fn stop_rate_limit_poller(&mut self) {
        if let Some(handle) = self.rate_limit_poller.take() {
            handle.abort();
        }
    }

    fn prefetch_rate_limits(&mut self) {
        self.stop_rate_limit_poller();

        if !self
            .auth_manager
            .auth_cached()
            .is_some_and(|auth| auth.is_chatgpt_auth())
        {
            return;
        }

        let base_url = self.config.chatgpt_base_url.clone();
        let app_event_tx = self.app_event_tx.clone();
        let auth_manager = Arc::clone(&self.auth_manager);

        let handle = tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(60));

            loop {
                if let Some(auth) = auth_manager.auth().await
                    && auth.is_chatgpt_auth()
                    && let Some(snapshot) = fetch_rate_limits(base_url.clone(), auth).await
                {
                    app_event_tx.send(AppEvent::RateLimitSnapshotFetched(snapshot));
                }
                interval.tick().await;
            }
        });

        self.rate_limit_poller = Some(handle);
    }

    fn lower_cost_preset(&self) -> Option<ModelPreset> {
        let models = self.models_manager.try_list_models(&self.config).ok()?;
        models
            .iter()
            .find(|preset| preset.show_in_picker && preset.model == NUDGE_MODEL_SLUG)
            .cloned()
    }

    fn rate_limit_switch_prompt_hidden(&self) -> bool {
        self.config
            .notices
            .hide_rate_limit_model_nudge
            .unwrap_or(false)
    }

    fn maybe_show_pending_rate_limit_prompt(&mut self) {
        if self.rate_limit_switch_prompt_hidden() {
            self.rate_limit_switch_prompt = RateLimitSwitchPromptState::Idle;
            return;
        }
        if !matches!(
            self.rate_limit_switch_prompt,
            RateLimitSwitchPromptState::Pending
        ) {
            return;
        }
        if let Some(preset) = self.lower_cost_preset() {
            self.open_rate_limit_switch_prompt(preset);
            self.rate_limit_switch_prompt = RateLimitSwitchPromptState::Shown;
        } else {
            self.rate_limit_switch_prompt = RateLimitSwitchPromptState::Idle;
        }
    }

    fn open_rate_limit_switch_prompt(&mut self, preset: ModelPreset) {
        let switch_model = preset.model.to_string();
        let display_name = preset.display_name.to_string();
        let default_effort: ReasoningEffortConfig = preset.default_reasoning_effort;

        let switch_actions: Vec<SelectionAction> = vec![Box::new(move |tx| {
            tx.send(AppEvent::CodexOp(Op::OverrideTurnContext {
                cwd: None,
                approval_policy: None,
                sandbox_policy: None,
                windows_sandbox_level: None,
                model: Some(switch_model.clone()),
                effort: Some(Some(default_effort)),
                summary: None,
                collaboration_mode: None,
                personality: None,
                spec_parallel_priority: None,
                spec_sdd_planning: None,
            }));
            tx.send(AppEvent::UpdateModel(switch_model.clone()));
            tx.send(AppEvent::UpdateReasoningEffort(Some(default_effort)));
        })];

        let keep_actions: Vec<SelectionAction> = Vec::new();
        let never_actions: Vec<SelectionAction> = vec![Box::new(|tx| {
            tx.send(AppEvent::UpdateRateLimitSwitchPromptHidden(true));
            tx.send(AppEvent::PersistRateLimitSwitchPromptHidden);
        })];
        let description = if preset.description.is_empty() {
            Some(
                tr(
                    self.config.language,
                    "chatwidget.rate_limit_prompt.switch_description",
                )
                .to_string(),
            )
        } else {
            Some(preset.description)
        };

        let items = vec![
            SelectionItem {
                name: tr_args(
                    self.config.language,
                    "chatwidget.rate_limit_prompt.switch_to",
                    &[("display_name", &display_name)],
                ),
                description,
                selected_description: None,
                is_current: false,
                actions: switch_actions,
                dismiss_on_select: true,
                ..Default::default()
            },
            SelectionItem {
                name: tr(
                    self.config.language,
                    "chatwidget.rate_limit_prompt.keep_current",
                )
                .to_string(),
                description: None,
                selected_description: None,
                is_current: false,
                actions: keep_actions,
                dismiss_on_select: true,
                ..Default::default()
            },
            SelectionItem {
                name: tr(
                    self.config.language,
                    "chatwidget.rate_limit_prompt.keep_current_never",
                )
                .to_string(),
                description: Some(
                    tr(
                        self.config.language,
                        "chatwidget.rate_limit_prompt.keep_current_never_desc",
                    )
                    .to_string(),
                ),
                selected_description: None,
                is_current: false,
                actions: never_actions,
                dismiss_on_select: true,
                ..Default::default()
            },
        ];

        self.bottom_pane.show_selection_view(SelectionViewParams {
            title: Some(tr(self.config.language, "chatwidget.rate_limit_prompt.title").to_string()),
            subtitle: Some(tr_args(
                self.config.language,
                "chatwidget.rate_limit_prompt.subtitle",
                &[("display_name", &display_name)],
            )),
            footer_hint: Some(standard_popup_hint_line(self.config.language)),
            items,
            ..Default::default()
        });
    }

    pub(crate) fn open_agent_popup(&mut self) {
        let language = self.config.language;
        let items = vec![
            SelectionItem {
                name: tr(language, "chatwidget.agent_popup.collab").to_string(),
                description: Some(tr(language, "chatwidget.agent_popup.collab_desc").to_string()),
                actions: vec![Box::new(|tx| tx.send(AppEvent::OpenCollabPopup))],
                dismiss_on_select: true,
                ..Default::default()
            },
            SelectionItem {
                name: tr(language, "chatwidget.agent_popup.preset").to_string(),
                description: Some(tr(language, "chatwidget.agent_popup.preset_desc").to_string()),
                actions: vec![Box::new(|tx| tx.send(AppEvent::OpenPresetPopup))],
                dismiss_on_select: true,
                ..Default::default()
            },
            SelectionItem {
                name: tr(language, "chatwidget.agent_popup.spec").to_string(),
                description: Some(tr(language, "chatwidget.agent_popup.spec_desc").to_string()),
                actions: vec![Box::new(|tx| tx.send(AppEvent::OpenSpecPopup))],
                dismiss_on_select: true,
                ..Default::default()
            },
            SelectionItem {
                name: tr(language, "chatwidget.agent_popup.workflow").to_string(),
                description: Some(tr(language, "chatwidget.agent_popup.workflow_desc").to_string()),
                actions: vec![Box::new(|tx| tx.send(AppEvent::OpenSddWorkflowPopup))],
                dismiss_on_select: true,
                ..Default::default()
            },
        ];
        self.bottom_pane.show_selection_view(SelectionViewParams {
            title: Some(tr(language, "chatwidget.agent_popup.title").to_string()),
            subtitle: Some(tr(language, "chatwidget.agent_popup.subtitle").to_string()),
            footer_hint: Some(standard_popup_hint_line(language)),
            items,
            header: Box::new(()),
            ..Default::default()
        });
    }

    /// Open a popup to change UI language.
    pub(crate) fn open_language_popup(&mut self) {
        let ui_language = self.config.language;
        let items = vec![
            SelectionItem {
                name: language_name(ui_language, Language::En).to_string(),
                description: Some(
                    tr(ui_language, "chatwidget.language_popup.english_desc").to_string(),
                ),
                is_current: ui_language == Language::En,
                actions: Self::language_selection_actions(Language::En),
                dismiss_on_select: true,
                ..Default::default()
            },
            SelectionItem {
                name: language_name(ui_language, Language::ZhCn).to_string(),
                description: Some(
                    tr(ui_language, "chatwidget.language_popup.chinese_desc").to_string(),
                ),
                is_current: ui_language == Language::ZhCn,
                actions: Self::language_selection_actions(Language::ZhCn),
                dismiss_on_select: true,
                ..Default::default()
            },
        ];

        self.bottom_pane.show_selection_view(SelectionViewParams {
            title: Some(tr(ui_language, "chatwidget.language_popup.title").to_string()),
            footer_hint: Some(standard_popup_hint_line(ui_language)),
            items,
            header: Box::new(()),
            ..Default::default()
        });
    }

    pub(crate) fn open_spec_popup(&mut self) {
        let language = self.config.language;
        let collab_enabled = self.config.features.enabled(Feature::Collab);
        let parallel_priority_enabled = self.config.spec.parallel_priority;
        let can_toggle_parallel_priority = collab_enabled || parallel_priority_enabled;
        let parallel_priority_description_key = if can_toggle_parallel_priority {
            "chatwidget.spec_popup.parallel_priority_label_desc"
        } else {
            "chatwidget.spec_popup.parallel_priority_requires_collab_desc"
        };
        let items = vec![SelectionItem {
            name: tr(language, "chatwidget.spec_popup.parallel_priority_label").to_string(),
            description: Some(tr(language, parallel_priority_description_key).to_string()),
            is_current: parallel_priority_enabled,
            actions: if can_toggle_parallel_priority {
                Self::spec_parallel_priority_selection_actions(!parallel_priority_enabled)
            } else {
                Vec::new()
            },
            dismiss_on_select: false,
            ..Default::default()
        }];

        self.bottom_pane.show_selection_view(SelectionViewParams {
            title: Some(tr(language, "chatwidget.spec_popup.title").to_string()),
            subtitle: Some(tr(language, "chatwidget.spec_popup.subtitle").to_string()),
            footer_hint: Some(tr(language, "chatwidget.spec_popup.checkbox_hint").into()),
            items,
            interaction_mode: SelectionInteractionMode::MultiSelect,
            header: Box::new(()),
            ..Default::default()
        });
    }

    pub(crate) fn open_collab_popup(&mut self) {
        let language = self.config.language;
        let fallback_model = self
            .current_model()
            .unwrap_or_else(|| self.model_display_name())
            .to_string();
        let fallback_effort = self.config.model_reasoning_effort;
        let items = vec![
            SelectionItem {
                name: tr(language, "chatwidget.collab_popup.plan").to_string(),
                description: Some(tr(language, "chatwidget.collab_popup.plan_desc").to_string()),
                actions: Self::collab_feature_selection_actions(
                    Some(ModeKind::Plan),
                    true,
                    fallback_model.clone(),
                    fallback_effort,
                ),
                dismiss_on_select: true,
                ..Default::default()
            },
            SelectionItem {
                name: tr(language, "chatwidget.collab_popup.proxy").to_string(),
                description: Some(tr(language, "chatwidget.collab_popup.proxy_desc").to_string()),
                actions: Self::collab_feature_selection_actions(
                    Some(ModeKind::Default),
                    true,
                    fallback_model.clone(),
                    fallback_effort,
                ),
                dismiss_on_select: true,
                ..Default::default()
            },
            SelectionItem {
                name: tr(language, "chatwidget.collab_popup.close").to_string(),
                description: Some(tr(language, "chatwidget.collab_popup.close_desc").to_string()),
                actions: Self::collab_feature_selection_actions(
                    None,
                    false,
                    fallback_model,
                    fallback_effort,
                ),
                dismiss_on_select: true,
                ..Default::default()
            },
        ];

        self.bottom_pane.show_selection_view(SelectionViewParams {
            title: Some(tr(language, "chatwidget.collab_popup.title").to_string()),
            subtitle: Some(tr(language, "chatwidget.collab_popup.subtitle").to_string()),
            footer_hint: Some(tr(language, "chatwidget.collab_popup.hint").into()),
            items,
            header: Box::new(()),
            ..Default::default()
        });
    }

    pub(crate) fn open_preset_popup(&mut self) {
        let language = self.config.language;
        let items = [
            SubagentPreset::Edit,
            SubagentPreset::Read,
            SubagentPreset::Grep,
            SubagentPreset::Run,
            SubagentPreset::Websearch,
        ]
        .into_iter()
        .map(|preset| SelectionItem {
            name: Self::subagent_preset_label(language, preset).to_string(),
            description: Some(
                match preset {
                    SubagentPreset::Edit => {
                        tr(language, "chatwidget.preset_popup.preset_edit_desc")
                    }
                    SubagentPreset::Read => {
                        tr(language, "chatwidget.preset_popup.preset_read_desc")
                    }
                    SubagentPreset::Grep => {
                        tr(language, "chatwidget.preset_popup.preset_grep_desc")
                    }
                    SubagentPreset::Run => tr(language, "chatwidget.preset_popup.preset_run_desc"),
                    SubagentPreset::Websearch => {
                        tr(language, "chatwidget.preset_popup.preset_websearch_desc")
                    }
                }
                .to_string(),
            ),
            is_current: false,
            actions: Self::subagent_preset_open_actions(preset),
            dismiss_on_select: true,
            ..Default::default()
        })
        .collect();

        self.bottom_pane.show_selection_view(SelectionViewParams {
            title: Some(tr(language, "chatwidget.preset_popup.title").to_string()),
            subtitle: Some(tr(language, "chatwidget.preset_popup.subtitle").to_string()),
            footer_hint: Some(standard_popup_hint_line(language)),
            items,
            header: Box::new(()),
            ..Default::default()
        });
    }

    pub(crate) fn open_subagent_preset_actions(&mut self, preset: SubagentPreset) {
        let language = self.config.language;
        let preset_label = Self::subagent_preset_label(language, preset);
        let preset_config = self.subagent_preset_config(preset);
        let model_description = tr_args(
            language,
            "chatwidget.preset_popup.current_model_desc",
            &[("model", preset_config.model.as_deref().unwrap_or("-"))],
        );
        let reasoning_description = tr_args(
            language,
            "chatwidget.preset_popup.current_reasoning_desc",
            &[(
                "reasoning",
                preset_config
                    .reasoning_effort
                    .map(|effort| Self::reasoning_effort_label(language, effort))
                    .unwrap_or("-"),
            )],
        );

        let items = vec![
            SelectionItem {
                name: tr(language, "chatwidget.preset_popup.action_set_model").to_string(),
                description: Some(model_description),
                actions: vec![Box::new(move |tx| {
                    tx.send(AppEvent::OpenSubagentPresetModelPicker { preset });
                })],
                dismiss_on_select: true,
                ..Default::default()
            },
            SelectionItem {
                name: tr(language, "chatwidget.preset_popup.action_set_reasoning").to_string(),
                description: Some(reasoning_description),
                actions: vec![Box::new(move |tx| {
                    tx.send(AppEvent::OpenSubagentPresetReasoningPicker { preset });
                })],
                dismiss_on_select: true,
                ..Default::default()
            },
        ];

        self.bottom_pane.show_selection_view(SelectionViewParams {
            title: Some(tr(language, "chatwidget.preset_popup.title").to_string()),
            subtitle: Some(tr_args(
                language,
                "chatwidget.preset_popup.actions_subtitle",
                &[("preset", preset_label)],
            )),
            footer_hint: Some(standard_popup_hint_line(language)),
            items,
            header: Box::new(()),
            ..Default::default()
        });
    }

    pub(crate) fn open_subagent_preset_model_picker(&mut self, preset: SubagentPreset) {
        let language = self.config.language;
        let models = match self.models_manager.try_list_models(&self.config) {
            Ok(models) => models,
            Err(_) => {
                self.add_info_message(
                    tr(language, "chatwidget.model_popup.updating").to_string(),
                    None,
                );
                return;
            }
        };

        let current_model = self.subagent_preset_config(preset).model.as_deref();
        let mut items: Vec<SelectionItem> = models
            .into_iter()
            .filter(|model| model.show_in_picker)
            .map(|model_preset| {
                let model_for_action = model_preset.model.clone();
                SelectionItem {
                    name: model_preset.display_name,
                    description: (!model_preset.description.is_empty())
                        .then_some(model_preset.description.to_string()),
                    is_current: current_model == Some(model_for_action.as_str()),
                    is_default: model_preset.is_default,
                    actions: vec![Box::new(move |tx| {
                        tx.send(AppEvent::UpdateSubagentPresetModel {
                            preset,
                            model: Some(model_for_action.clone()),
                        });
                        tx.send(AppEvent::PersistSubagentPresetModel {
                            preset,
                            model: Some(model_for_action.clone()),
                        });
                    })],
                    dismiss_on_select: true,
                    search_value: Some(model_preset.model),
                    ..Default::default()
                }
            })
            .collect();
        items.sort_by(|left, right| left.name.cmp(&right.name));

        self.bottom_pane.show_selection_view(SelectionViewParams {
            title: Some(tr(language, "chatwidget.preset_popup.title").to_string()),
            subtitle: Some(tr_args(
                language,
                "chatwidget.preset_popup.model_subtitle",
                &[("preset", Self::subagent_preset_label(language, preset))],
            )),
            footer_hint: Some(standard_popup_hint_line(language)),
            items,
            is_searchable: true,
            header: Box::new(()),
            ..Default::default()
        });
    }

    pub(crate) fn open_subagent_preset_reasoning_picker(&mut self, preset: SubagentPreset) {
        let language = self.config.language;
        let current_effort = self.subagent_preset_config(preset).reasoning_effort;
        let fallback_efforts: Vec<ReasoningEffortConfig> = ReasoningEffortConfig::iter()
            .filter(|effort| *effort != ReasoningEffortConfig::None)
            .collect();
        let supported_efforts: Vec<ReasoningEffortConfig> = self
            .models_manager
            .try_list_models(&self.config)
            .ok()
            .and_then(|models| {
                self.subagent_preset_config(preset)
                    .model
                    .as_deref()
                    .or_else(|| self.current_model())
                    .and_then(|model_slug| {
                        models
                            .into_iter()
                            .find(|model_preset| model_preset.model == model_slug)
                            .map(|model_preset| {
                                model_preset
                                    .supported_reasoning_efforts
                                    .into_iter()
                                    .map(|option| option.effort)
                                    .collect::<Vec<_>>()
                            })
                    })
            })
            .filter(|efforts| !efforts.is_empty())
            .unwrap_or(fallback_efforts);
        let items = supported_efforts
            .into_iter()
            .map(|effort| SelectionItem {
                name: Self::reasoning_effort_label(language, effort).to_string(),
                is_current: current_effort == Some(effort),
                actions: vec![Box::new(move |tx| {
                    tx.send(AppEvent::UpdateSubagentPresetReasoningEffort {
                        preset,
                        effort: Some(effort),
                    });
                    tx.send(AppEvent::PersistSubagentPresetReasoningEffort {
                        preset,
                        effort: Some(effort),
                    });
                })],
                dismiss_on_select: true,
                ..Default::default()
            })
            .collect();

        self.bottom_pane.show_selection_view(SelectionViewParams {
            title: Some(tr(language, "chatwidget.preset_popup.title").to_string()),
            subtitle: Some(tr_args(
                language,
                "chatwidget.preset_popup.reasoning_subtitle",
                &[("preset", Self::subagent_preset_label(language, preset))],
            )),
            footer_hint: Some(standard_popup_hint_line(language)),
            items,
            header: Box::new(()),
            ..Default::default()
        });
    }

    pub(crate) fn open_model_popup(&mut self) {
        let language = self.config.language;
        if !self.is_session_configured() {
            let message = tr(language, "chatwidget.model_popup.disabled_until_ready").to_string();
            self.add_info_message(message, None);
            return;
        }

        let presets: Vec<ModelPreset> = match self.models_manager.try_list_models(&self.config) {
            Ok(models) => models,
            Err(_) => {
                self.add_info_message(
                    tr(language, "chatwidget.model_popup.updating").to_string(),
                    None,
                );
                return;
            }
        };
        self.open_model_popup_with_presets(presets);
    }

    pub(crate) fn open_model_popup_with_presets(&mut self, presets: Vec<ModelPreset>) {
        let language = self.config.language;
        let presets: Vec<ModelPreset> = presets
            .into_iter()
            .filter(|preset| preset.show_in_picker)
            .collect();

        let current_model = self.current_model();
        let current_label = presets
            .iter()
            .find(|preset| Some(preset.model.as_str()) == current_model)
            .map(|preset| preset.display_name.to_string())
            .unwrap_or_else(|| self.model_display_name().to_string());

        let (mut auto_presets, other_presets): (Vec<ModelPreset>, Vec<ModelPreset>) = presets
            .into_iter()
            .partition(|preset| Self::is_auto_model(&preset.model));

        if auto_presets.is_empty() {
            self.open_all_models_popup(other_presets);
            return;
        }

        auto_presets.sort_by_key(|preset| Self::auto_model_order(&preset.model));

        let mut items: Vec<SelectionItem> = auto_presets
            .into_iter()
            .map(|preset| {
                let description =
                    (!preset.description.is_empty()).then_some(preset.description.clone());
                let model = preset.model.clone();
                let actions = Self::model_selection_actions(
                    model.clone(),
                    Some(preset.default_reasoning_effort),
                );
                SelectionItem {
                    name: preset.display_name.clone(),
                    description,
                    is_current: Some(model.as_str()) == current_model,
                    is_default: preset.is_default,
                    actions,
                    dismiss_on_select: true,
                    ..Default::default()
                }
            })
            .collect();

        if !other_presets.is_empty() {
            let all_models = other_presets;
            let actions: Vec<SelectionAction> = vec![Box::new(move |tx| {
                tx.send(AppEvent::OpenAllModelsPopup {
                    models: all_models.clone(),
                });
            })];

            let is_current = !items.iter().any(|item| item.is_current);
            let description = Some(tr_args(
                language,
                "chatwidget.model_popup.all_models_desc",
                &[("current_label", &current_label)],
            ));

            items.push(SelectionItem {
                name: tr(language, "chatwidget.model_popup.all_models").to_string(),
                description,
                is_current,
                actions,
                dismiss_on_select: true,
                ..Default::default()
            });
        }

        let header = self.model_menu_header(
            tr(language, "chatwidget.model_popup.quick_title"),
            tr(language, "chatwidget.model_popup.quick_subtitle"),
        );
        self.bottom_pane.show_selection_view(SelectionViewParams {
            footer_hint: Some(standard_popup_hint_line(self.config.language)),
            items,
            header,
            ..Default::default()
        });
    }

    fn model_menu_header(&self, title: &str, subtitle: &str) -> Box<dyn Renderable> {
        let title = title.to_string();
        let subtitle = subtitle.to_string();
        let mut header = ColumnRenderable::new();
        header.push(Line::from(title.bold()));
        header.push(Line::from(subtitle.dim()));
        Box::new(header)
    }

    fn is_auto_model(model: &str) -> bool {
        model.starts_with("codex-auto-")
    }

    fn auto_model_order(model: &str) -> usize {
        match model {
            "codex-auto-fast" => 0,
            "codex-auto-balanced" => 1,
            "codex-auto-thorough" => 2,
            _ => 3,
        }
    }

    pub(crate) fn open_all_models_popup(&mut self, presets: Vec<ModelPreset>) {
        let language = self.config.language;
        if presets.is_empty() {
            self.add_info_message(
                tr(language, "chatwidget.model_popup.no_additional_models").to_string(),
                None,
            );
            return;
        }

        let mut items: Vec<SelectionItem> = Vec::new();
        for preset in presets.into_iter() {
            let description =
                (!preset.description.is_empty()).then_some(preset.description.to_string());
            let is_current = Some(preset.model.as_str()) == self.current_model();
            let single_supported_effort = preset.supported_reasoning_efforts.len() == 1;
            let preset_for_action = preset.clone();
            let actions: Vec<SelectionAction> = vec![Box::new(move |tx| {
                let preset_for_event = preset_for_action.clone();
                tx.send(AppEvent::OpenReasoningPopup {
                    model: preset_for_event,
                });
            })];
            items.push(SelectionItem {
                name: preset.display_name.clone(),
                description,
                is_current,
                is_default: preset.is_default,
                actions,
                dismiss_on_select: single_supported_effort,
                ..Default::default()
            });
        }

        let header = self.model_menu_header(
            tr(language, "chatwidget.model_popup.title"),
            tr(language, "chatwidget.model_popup.subtitle"),
        );
        self.bottom_pane.show_selection_view(SelectionViewParams {
            footer_hint: Some(tr(language, "chatwidget.model_popup.footer_hint").into()),
            items,
            header,
            ..Default::default()
        });
    }

    fn model_selection_actions(
        model_for_action: String,
        effort_for_action: Option<ReasoningEffortConfig>,
    ) -> Vec<SelectionAction> {
        vec![Box::new(move |tx| {
            let effort_label = effort_for_action
                .map(|effort| effort.to_string())
                .unwrap_or_else(|| "default".to_string());
            tx.send(AppEvent::CodexOp(Op::OverrideTurnContext {
                cwd: None,
                approval_policy: None,
                sandbox_policy: None,
                windows_sandbox_level: None,
                model: Some(model_for_action.clone()),
                effort: Some(effort_for_action),
                summary: None,
                collaboration_mode: None,
                personality: None,
                spec_parallel_priority: None,
                spec_sdd_planning: None,
            }));
            tx.send(AppEvent::UpdateModel(model_for_action.clone()));
            tx.send(AppEvent::UpdateReasoningEffort(effort_for_action));
            tx.send(AppEvent::PersistModelSelection {
                model: model_for_action.clone(),
                effort: effort_for_action,
            });
            tracing::info!(
                "Selected model: {}, Selected effort: {}",
                model_for_action,
                effort_label
            );
        })]
    }

    fn language_selection_actions(language: Language) -> Vec<SelectionAction> {
        vec![Box::new(move |tx| {
            tx.send(AppEvent::UpdateLanguage(language));
            tx.send(AppEvent::PersistLanguageSelection { language });
        })]
    }

    fn spec_parallel_priority_selection_actions(enabled: bool) -> Vec<SelectionAction> {
        vec![Box::new(move |tx| {
            tx.send(AppEvent::CodexOp(Op::OverrideTurnContext {
                cwd: None,
                approval_policy: None,
                sandbox_policy: None,
                windows_sandbox_level: None,
                model: None,
                effort: None,
                summary: None,
                collaboration_mode: None,
                personality: None,
                spec_parallel_priority: Some(enabled),
                spec_sdd_planning: None,
            }));
            tx.send(AppEvent::UpdateSpecParallelPriority(enabled));
            tx.send(AppEvent::PersistSpecParallelPriority { enabled });
        })]
    }

    fn collab_feature_selection_actions(
        mode: Option<ModeKind>,
        enabled: bool,
        fallback_model: String,
        fallback_effort: Option<ReasoningEffortConfig>,
    ) -> Vec<SelectionAction> {
        vec![Box::new(move |tx| {
            tx.send(AppEvent::UpdateCollabFeature(enabled));
            tx.send(AppEvent::PersistCollabFeature { enabled });
            if let Some(mode) = mode {
                let settings = CollaborationSettings {
                    model: fallback_model.clone(),
                    reasoning_effort: fallback_effort,
                    developer_instructions: None,
                };
                tx.send(AppEvent::CodexOp(Op::OverrideTurnContext {
                    cwd: None,
                    approval_policy: None,
                    sandbox_policy: None,
                    windows_sandbox_level: None,
                    model: None,
                    effort: None,
                    summary: None,
                    collaboration_mode: Some(CollaborationMode { mode, settings }),
                    personality: None,
                    spec_parallel_priority: None,
                    spec_sdd_planning: None,
                }));
            }
        })]
    }

    fn subagent_preset_label(language: Language, preset: SubagentPreset) -> &'static str {
        let key = match preset {
            SubagentPreset::Edit => "chatwidget.preset_popup.preset_edit",
            SubagentPreset::Read => "chatwidget.preset_popup.preset_read",
            SubagentPreset::Grep => "chatwidget.preset_popup.preset_grep",
            SubagentPreset::Run => "chatwidget.preset_popup.preset_run",
            SubagentPreset::Websearch => "chatwidget.preset_popup.preset_websearch",
        };
        tr(language, key)
    }

    fn subagent_preset_open_actions(preset: SubagentPreset) -> Vec<SelectionAction> {
        vec![Box::new(move |tx| {
            tx.send(AppEvent::OpenSubagentPresetActions { preset });
        })]
    }

    fn subagent_preset_config(
        &self,
        preset: SubagentPreset,
    ) -> &codex_core::config::types::SubagentPresetConfig {
        match preset {
            SubagentPreset::Edit => &self.config.subagent_presets.edit,
            SubagentPreset::Read => &self.config.subagent_presets.read,
            SubagentPreset::Grep => &self.config.subagent_presets.grep,
            SubagentPreset::Run => &self.config.subagent_presets.run,
            SubagentPreset::Websearch => &self.config.subagent_presets.websearch,
        }
    }

    fn set_sdd_collaboration_mode(&self, mode: ModeKind) {
        if !self.config.features.enabled(Feature::Collab) {
            return;
        }
        let settings = CollaborationSettings {
            model: self
                .current_model()
                .map(ToOwned::to_owned)
                .or_else(|| self.config.model.clone())
                .unwrap_or_else(|| self.model_display_name().to_string()),
            reasoning_effort: self.config.model_reasoning_effort,
            developer_instructions: None,
        };
        self.app_event_tx
            .send(AppEvent::CodexOp(Op::OverrideTurnContext {
                cwd: None,
                approval_policy: None,
                sandbox_policy: None,
                windows_sandbox_level: None,
                model: None,
                effort: None,
                summary: None,
                collaboration_mode: Some(CollaborationMode { mode, settings }),
                personality: None,
                spec_parallel_priority: None,
                spec_sdd_planning: None,
            }));
    }

    /// Open a popup to choose the reasoning effort (stage 2) for the given model.
    pub(crate) fn open_reasoning_popup(&mut self, preset: ModelPreset) {
        let default_effort: ReasoningEffortConfig = preset.default_reasoning_effort;
        let supported = preset.supported_reasoning_efforts;

        let warn_effort = if supported
            .iter()
            .any(|option| option.effort == ReasoningEffortConfig::XHigh)
        {
            Some(ReasoningEffortConfig::XHigh)
        } else if supported
            .iter()
            .any(|option| option.effort == ReasoningEffortConfig::High)
        {
            Some(ReasoningEffortConfig::High)
        } else {
            None
        };
        let warning_text = warn_effort.map(|effort| {
            let effort_label = Self::reasoning_effort_label(self.config.language, effort);
            tr_args(
                self.config.language,
                "chatwidget.reasoning.warning",
                &[("effort_label", effort_label)],
            )
        });
        let warn_for_model = preset.model.starts_with("gpt-5.1-codex")
            || preset.model.starts_with("gpt-5.1-codex-max")
            || preset.model.starts_with("gpt-5.2")
            || preset.model.starts_with("gpt-5.3");

        struct EffortChoice {
            stored: Option<ReasoningEffortConfig>,
            display: ReasoningEffortConfig,
        }
        let mut choices: Vec<EffortChoice> = Vec::new();
        for effort in ReasoningEffortConfig::iter() {
            if supported.iter().any(|option| option.effort == effort) {
                choices.push(EffortChoice {
                    stored: Some(effort),
                    display: effort,
                });
            }
        }
        if choices.is_empty() {
            choices.push(EffortChoice {
                stored: Some(default_effort),
                display: default_effort,
            });
        }

        if choices.len() == 1 {
            if let Some(effort) = choices.first().and_then(|c| c.stored) {
                self.apply_model_and_effort(preset.model, Some(effort));
            } else {
                self.apply_model_and_effort(preset.model, None);
            }
            return;
        }

        let default_choice: Option<ReasoningEffortConfig> = choices
            .iter()
            .any(|choice| choice.stored == Some(default_effort))
            .then_some(Some(default_effort))
            .flatten()
            .or_else(|| choices.iter().find_map(|choice| choice.stored))
            .or(Some(default_effort));

        let model_slug = preset.model.to_string();
        let is_current_model = self.current_model() == Some(preset.model.as_str());
        let highlight_choice = if is_current_model {
            self.config.model_reasoning_effort
        } else {
            default_choice
        };
        let selection_choice = highlight_choice.or(default_choice);
        let initial_selected_idx = choices
            .iter()
            .position(|choice| choice.stored == selection_choice)
            .or_else(|| {
                selection_choice
                    .and_then(|effort| choices.iter().position(|choice| choice.display == effort))
            });
        let mut items: Vec<SelectionItem> = Vec::new();
        for choice in choices.iter() {
            let effort = choice.display;
            let mut effort_label =
                Self::reasoning_effort_label(self.config.language, effort).to_string();
            if choice.stored == default_choice {
                effort_label.push_str(tr(
                    self.config.language,
                    "chatwidget.reasoning.default_suffix",
                ));
            }

            let description = choice
                .stored
                .and_then(|effort| {
                    supported
                        .iter()
                        .find(|option| option.effort == effort)
                        .map(|option| option.description.to_string())
                })
                .filter(|text| !text.is_empty());

            let show_warning = warn_for_model && warn_effort == Some(effort);
            let selected_description = if show_warning {
                warning_text.as_ref().map(|warning_message| {
                    description.as_ref().map_or_else(
                        || warning_message.clone(),
                        |d| format!("{d}\n{warning_message}"),
                    )
                })
            } else {
                None
            };

            let model_for_action = model_slug.clone();
            let actions = Self::model_selection_actions(model_for_action, choice.stored);

            items.push(SelectionItem {
                name: effort_label,
                description,
                selected_description,
                is_current: is_current_model && choice.stored == highlight_choice,
                actions,
                dismiss_on_select: true,
                ..Default::default()
            });
        }

        let mut header = ColumnRenderable::new();
        header.push(Line::from(
            tr_args(
                self.config.language,
                "chatwidget.reasoning.select_title",
                &[("model", &model_slug)],
            )
            .bold(),
        ));

        self.bottom_pane.show_selection_view(SelectionViewParams {
            header: Box::new(header),
            footer_hint: Some(standard_popup_hint_line(self.config.language)),
            items,
            initial_selected_idx,
            ..Default::default()
        });
    }

    fn reasoning_effort_label(language: Language, effort: ReasoningEffortConfig) -> &'static str {
        let key = match effort {
            ReasoningEffortConfig::None => "reasoning_effort.none",
            ReasoningEffortConfig::Minimal => "reasoning_effort.minimal",
            ReasoningEffortConfig::Low => "reasoning_effort.low",
            ReasoningEffortConfig::Medium => "reasoning_effort.medium",
            ReasoningEffortConfig::High => "reasoning_effort.high",
            ReasoningEffortConfig::XHigh => "reasoning_effort.xhigh",
        };
        tr(language, key)
    }

    fn apply_model_and_effort(&self, model: String, effort: Option<ReasoningEffortConfig>) {
        self.app_event_tx
            .send(AppEvent::CodexOp(Op::OverrideTurnContext {
                cwd: None,
                approval_policy: None,
                sandbox_policy: None,
                windows_sandbox_level: None,
                model: Some(model.clone()),
                effort: Some(effort),
                summary: None,
                collaboration_mode: None,
                personality: None,
                spec_parallel_priority: None,
                spec_sdd_planning: None,
            }));
        self.app_event_tx.send(AppEvent::UpdateModel(model.clone()));
        self.app_event_tx
            .send(AppEvent::UpdateReasoningEffort(effort));
        self.app_event_tx.send(AppEvent::PersistModelSelection {
            model: model.clone(),
            effort,
        });
        tracing::info!(
            "Selected model: {}, Selected effort: {}",
            model,
            effort
                .map(|e| e.to_string())
                .unwrap_or_else(|| "default".to_string())
        );
    }

    /// Open a popup to choose the approvals mode (ask for approval policy + sandbox policy).
    pub(crate) fn open_approvals_popup(&mut self) {
        let current_approval = self.config.approval_policy.value();
        let current_sandbox = self.config.sandbox_policy.get();
        let mut items: Vec<SelectionItem> = Vec::new();
        let presets: Vec<ApprovalPreset> = builtin_approval_presets();

        #[cfg(target_os = "windows")]
        let windows_sandbox_level = WindowsSandboxLevel::from_config(&self.config);
        #[cfg(target_os = "windows")]
        let windows_degraded_sandbox_enabled =
            matches!(windows_sandbox_level, WindowsSandboxLevel::RestrictedToken);
        #[cfg(not(target_os = "windows"))]
        let windows_degraded_sandbox_enabled = false;

        let show_elevate_sandbox_hint = codex_core::windows_sandbox::ELEVATED_SANDBOX_NUX_ENABLED
            && windows_degraded_sandbox_enabled
            && presets.iter().any(|preset| preset.id == "auto");

        for preset in presets.into_iter() {
            let (label, description_text) = match preset.id {
                "read-only" => (
                    tr(self.config.language, "chatwidget.approvals.read_only.label"),
                    tr(self.config.language, "chatwidget.approvals.read_only.desc"),
                ),
                "auto" => (
                    tr(self.config.language, "chatwidget.approvals.auto.label"),
                    tr(self.config.language, "chatwidget.approvals.auto.desc"),
                ),
                "full-access" => (
                    tr(
                        self.config.language,
                        "chatwidget.approvals.full_access.label",
                    ),
                    tr(
                        self.config.language,
                        "chatwidget.approvals.full_access.desc",
                    ),
                ),
                _ => (preset.label, preset.description),
            };
            let is_current =
                Self::preset_matches_current(current_approval, current_sandbox, &preset);
            let name = if preset.id == "auto" && windows_degraded_sandbox_enabled {
                tr(
                    self.config.language,
                    "chatwidget.approvals.auto_non_elevated_label",
                )
                .to_string()
            } else {
                label.to_string()
            };
            let description = Some(description_text.to_string());
            let requires_confirmation = preset.id == "full-access"
                && !self
                    .config
                    .notices
                    .hide_full_access_warning
                    .unwrap_or(false);
            let actions: Vec<SelectionAction> = if requires_confirmation {
                let preset_clone = preset.clone();
                vec![Box::new(move |tx| {
                    tx.send(AppEvent::OpenFullAccessConfirmation {
                        preset: preset_clone.clone(),
                    });
                })]
            } else if preset.id == "auto" {
                #[cfg(target_os = "windows")]
                {
                    if WindowsSandboxLevel::from_config(&self.config)
                        == WindowsSandboxLevel::Disabled
                    {
                        let preset_clone = preset.clone();
                        if codex_core::windows_sandbox::ELEVATED_SANDBOX_NUX_ENABLED
                            && codex_core::windows_sandbox::sandbox_setup_is_complete(
                                self.config.codex_home.as_path(),
                            )
                        {
                            vec![Box::new(move |tx| {
                                tx.send(AppEvent::EnableWindowsSandboxForAgentMode {
                                    preset: preset_clone.clone(),
                                    mode: WindowsSandboxEnableMode::Elevated,
                                });
                            })]
                        } else {
                            vec![Box::new(move |tx| {
                                tx.send(AppEvent::OpenWindowsSandboxEnablePrompt {
                                    preset: preset_clone.clone(),
                                });
                            })]
                        }
                    } else if let Some((sample_paths, extra_count, failed_scan)) =
                        self.world_writable_warning_details()
                    {
                        let preset_clone = preset.clone();
                        vec![Box::new(move |tx| {
                            tx.send(AppEvent::OpenWorldWritableWarningConfirmation {
                                preset: Some(preset_clone.clone()),
                                sample_paths: sample_paths.clone(),
                                extra_count,
                                failed_scan,
                            });
                        })]
                    } else {
                        Self::approval_preset_actions(preset.approval, preset.sandbox.clone())
                    }
                }
                #[cfg(not(target_os = "windows"))]
                {
                    Self::approval_preset_actions(preset.approval, preset.sandbox.clone())
                }
            } else {
                Self::approval_preset_actions(preset.approval, preset.sandbox.clone())
            };
            items.push(SelectionItem {
                name,
                description,
                is_current,
                actions,
                dismiss_on_select: true,
                ..Default::default()
            });
        }

        let footer_note = show_elevate_sandbox_hint.then(|| {
            let prefix = tr(
                self.config.language,
                "chatwidget.approvals.footer_note.prefix",
            );
            let suffix = tr(
                self.config.language,
                "chatwidget.approvals.footer_note.suffix",
            );
            vec![prefix.dim(), "/setup-elevated-sandbox".cyan(), suffix.dim()].into()
        });

        self.bottom_pane.show_selection_view(SelectionViewParams {
            title: Some(tr(self.config.language, "chatwidget.approvals.title").to_string()),
            footer_note,
            footer_hint: Some(standard_popup_hint_line(self.config.language)),
            items,
            header: Box::new(()),
            ..Default::default()
        });
    }

    fn approval_preset_actions(
        approval: AskForApproval,
        sandbox: SandboxPolicy,
    ) -> Vec<SelectionAction> {
        vec![Box::new(move |tx| {
            let sandbox_clone = sandbox.clone();
            tx.send(AppEvent::CodexOp(Op::OverrideTurnContext {
                cwd: None,
                approval_policy: Some(approval),
                sandbox_policy: Some(sandbox_clone.clone()),
                windows_sandbox_level: None,
                model: None,
                effort: None,
                summary: None,
                collaboration_mode: None,
                personality: None,
                spec_parallel_priority: None,
                spec_sdd_planning: None,
            }));
            tx.send(AppEvent::UpdateAskForApprovalPolicy(approval));
            tx.send(AppEvent::UpdateSandboxPolicy(sandbox_clone));
            if let Some(sandbox_mode) = Self::sandbox_mode_for_policy(&sandbox) {
                tx.send(AppEvent::PersistApprovalSelection {
                    approval_policy: approval,
                    sandbox_mode,
                });
            }
        })]
    }

    fn sandbox_mode_for_policy(policy: &SandboxPolicy) -> Option<SandboxMode> {
        match policy {
            SandboxPolicy::ReadOnly => Some(SandboxMode::ReadOnly),
            SandboxPolicy::WorkspaceWrite { .. } => Some(SandboxMode::WorkspaceWrite),
            SandboxPolicy::DangerFullAccess => Some(SandboxMode::DangerFullAccess),
            SandboxPolicy::ExternalSandbox { .. } => None,
        }
    }

    fn preset_matches_current(
        current_approval: AskForApproval,
        current_sandbox: &SandboxPolicy,
        preset: &ApprovalPreset,
    ) -> bool {
        if current_approval != preset.approval {
            return false;
        }
        matches!(
            (&preset.sandbox, current_sandbox),
            (SandboxPolicy::ReadOnly, SandboxPolicy::ReadOnly)
                | (
                    SandboxPolicy::DangerFullAccess,
                    SandboxPolicy::DangerFullAccess
                )
                | (
                    SandboxPolicy::WorkspaceWrite { .. },
                    SandboxPolicy::WorkspaceWrite { .. }
                )
        )
    }

    #[cfg(target_os = "windows")]
    pub(crate) fn world_writable_warning_details(&self) -> Option<(Vec<String>, usize, bool)> {
        if self
            .config
            .notices
            .hide_world_writable_warning
            .unwrap_or(false)
        {
            return None;
        }
        let cwd = self.config.cwd.clone();
        let env_map: std::collections::HashMap<String, String> = std::env::vars().collect();
        match codex_windows_sandbox::apply_world_writable_scan_and_denies(
            self.config.codex_home.as_path(),
            cwd.as_path(),
            &env_map,
            self.config.sandbox_policy.get(),
            Some(self.config.codex_home.as_path()),
        ) {
            Ok(_) => None,
            Err(_) => Some((Vec::new(), 0, true)),
        }
    }

    #[cfg(not(target_os = "windows"))]
    #[allow(dead_code)]
    pub(crate) fn world_writable_warning_details(&self) -> Option<(Vec<String>, usize, bool)> {
        None
    }

    pub(crate) fn open_full_access_confirmation(&mut self, preset: ApprovalPreset) {
        let approval = preset.approval;
        let sandbox = preset.sandbox;
        let language = self.config.language;
        let mut header_children: Vec<Box<dyn Renderable>> = Vec::new();
        let title_line = Line::from(tr(language, "chatwidget.full_access.title")).bold();
        let intro_text = tr(language, "chatwidget.full_access.intro");
        let warning_text = tr(language, "chatwidget.full_access.warning");
        let info_line = Line::from(vec![intro_text.into(), warning_text.fg(Color::Red)]);
        header_children.push(Box::new(title_line));
        header_children.push(Box::new(
            Paragraph::new(vec![info_line]).wrap(Wrap { trim: false }),
        ));
        let header = ColumnRenderable::with(header_children);

        let mut accept_actions = Self::approval_preset_actions(approval, sandbox.clone());
        accept_actions.push(Box::new(|tx| {
            tx.send(AppEvent::UpdateFullAccessWarningAcknowledged(true));
        }));

        let mut accept_and_remember_actions = Self::approval_preset_actions(approval, sandbox);
        accept_and_remember_actions.push(Box::new(|tx| {
            tx.send(AppEvent::UpdateFullAccessWarningAcknowledged(true));
            tx.send(AppEvent::PersistFullAccessWarningAcknowledged);
        }));

        let deny_actions: Vec<SelectionAction> = vec![Box::new(|tx| {
            tx.send(AppEvent::OpenApprovalsPopup);
        })];

        let items = vec![
            SelectionItem {
                name: tr(language, "chatwidget.full_access.option.continue").to_string(),
                description: Some(
                    tr(language, "chatwidget.full_access.option.continue_desc").to_string(),
                ),
                actions: accept_actions,
                dismiss_on_select: true,
                ..Default::default()
            },
            SelectionItem {
                name: tr(language, "chatwidget.full_access.option.continue_remember").to_string(),
                description: Some(
                    tr(
                        language,
                        "chatwidget.full_access.option.continue_remember_desc",
                    )
                    .to_string(),
                ),
                actions: accept_and_remember_actions,
                dismiss_on_select: true,
                ..Default::default()
            },
            SelectionItem {
                name: tr(language, "chatwidget.full_access.option.cancel").to_string(),
                description: Some(
                    tr(language, "chatwidget.full_access.option.cancel_desc").to_string(),
                ),
                actions: deny_actions,
                dismiss_on_select: true,
                ..Default::default()
            },
        ];

        self.bottom_pane.show_selection_view(SelectionViewParams {
            footer_hint: Some(standard_popup_hint_line(self.config.language)),
            items,
            header: Box::new(header),
            ..Default::default()
        });
    }

    #[cfg(target_os = "windows")]
    pub(crate) fn open_world_writable_warning_confirmation(
        &mut self,
        preset: Option<ApprovalPreset>,
        sample_paths: Vec<String>,
        extra_count: usize,
        failed_scan: bool,
    ) {
        let (approval, sandbox) = match &preset {
            Some(p) => (Some(p.approval), Some(p.sandbox.clone())),
            None => (None, None),
        };
        let language = self.config.language;
        let mut header_children: Vec<Box<dyn Renderable>> = Vec::new();
        let describe_policy = |policy: &SandboxPolicy| {
            let key = match policy {
                SandboxPolicy::ReadOnly => "chatwidget.world_writable.policy.read_only",
                _ => "chatwidget.world_writable.policy.agent",
            };
            tr(language, key)
        };
        let mode_label = preset
            .as_ref()
            .map(|p| describe_policy(&p.sandbox))
            .unwrap_or_else(|| describe_policy(self.config.sandbox_policy.get()));
        let info_line = if failed_scan {
            Line::from(vec![
                tr(language, "chatwidget.world_writable.failed_scan").into(),
                tr_args(
                    language,
                    "chatwidget.world_writable.failed_scan_warning",
                    &[("mode_label", mode_label)],
                )
                .fg(Color::Red),
            ])
        } else {
            Line::from(vec![
                tr(language, "chatwidget.world_writable.unprotected").into(),
                tr(language, "chatwidget.world_writable.remove_everyone").into(),
            ])
        };
        header_children.push(Box::new(
            Paragraph::new(vec![info_line]).wrap(Wrap { trim: false }),
        ));

        if !sample_paths.is_empty() {
            // Show up to three examples and optionally an "and X more" line.
            let mut lines: Vec<Line> = Vec::new();
            lines.push(Line::from(""));
            for p in &sample_paths {
                lines.push(Line::from(format!("  - {p}")));
            }
            if extra_count > 0 {
                let message = tr_args(
                    language,
                    "chatwidget.world_writable.and_more",
                    &[("count", &extra_count.to_string())],
                );
                lines.push(Line::from(message));
            }
            header_children.push(Box::new(Paragraph::new(lines).wrap(Wrap { trim: false })));
        }
        let header = ColumnRenderable::with(header_children);

        // Build actions ensuring acknowledgement happens before applying the new sandbox policy,
        // so downstream policy-change hooks don't re-trigger the warning.
        let mut accept_actions: Vec<SelectionAction> = Vec::new();
        // Suppress the immediate re-scan only when a preset will be applied (i.e., via /approvals),
        // to avoid duplicate warnings from the ensuing policy change.
        if preset.is_some() {
            accept_actions.push(Box::new(|tx| {
                tx.send(AppEvent::SkipNextWorldWritableScan);
            }));
        }
        if let (Some(approval), Some(sandbox)) = (approval, sandbox.clone()) {
            accept_actions.extend(Self::approval_preset_actions(approval, sandbox));
        }

        let mut accept_and_remember_actions: Vec<SelectionAction> = Vec::new();
        accept_and_remember_actions.push(Box::new(|tx| {
            tx.send(AppEvent::UpdateWorldWritableWarningAcknowledged(true));
            tx.send(AppEvent::PersistWorldWritableWarningAcknowledged);
        }));
        if let (Some(approval), Some(sandbox)) = (approval, sandbox) {
            accept_and_remember_actions.extend(Self::approval_preset_actions(approval, sandbox));
        }

        let items = vec![
            SelectionItem {
                name: tr(language, "chatwidget.world_writable.continue").to_string(),
                description: Some(tr_args(
                    language,
                    "chatwidget.world_writable.apply_session",
                    &[("mode_label", mode_label)],
                )),
                actions: accept_actions,
                dismiss_on_select: true,
                ..Default::default()
            },
            SelectionItem {
                name: tr(language, "chatwidget.world_writable.continue_no_warn").to_string(),
                description: Some(tr_args(
                    language,
                    "chatwidget.world_writable.apply_remember",
                    &[("mode_label", mode_label)],
                )),
                actions: accept_and_remember_actions,
                dismiss_on_select: true,
                ..Default::default()
            },
        ];

        self.bottom_pane.show_selection_view(SelectionViewParams {
            footer_hint: Some(standard_popup_hint_line(self.config.language)),
            items,
            header: Box::new(header),
            ..Default::default()
        });
    }

    #[cfg(not(target_os = "windows"))]
    pub(crate) fn open_world_writable_warning_confirmation(
        &mut self,
        _preset: Option<ApprovalPreset>,
        _sample_paths: Vec<String>,
        _extra_count: usize,
        _failed_scan: bool,
    ) {
    }

    #[cfg(target_os = "windows")]
    pub(crate) fn open_windows_sandbox_enable_prompt(&mut self, preset: ApprovalPreset) {
        use ratatui_macros::line;

        let language = self.config.language;
        if !codex_core::windows_sandbox::ELEVATED_SANDBOX_NUX_ENABLED {
            // Legacy flow (pre-NUX): explain the experimental sandbox and let the user enable it
            // directly (no elevation prompts).
            let mut header = ColumnRenderable::new();
            let title_line = tr(language, "chatwidget.windows_sandbox.legacy.title");
            let learn_more_line = tr(language, "chatwidget.windows_sandbox.legacy.learn_more");
            header.push(*Box::new(
                Paragraph::new(vec![line![title_line.bold()], line![learn_more_line]])
                    .wrap(Wrap { trim: false }),
            ));

            let preset_clone = preset;
            let items = vec![
                SelectionItem {
                    name: tr(language, "chatwidget.windows_sandbox.legacy.enable").to_string(),
                    description: None,
                    actions: vec![Box::new(move |tx| {
                        tx.send(AppEvent::EnableWindowsSandboxForAgentMode {
                            preset: preset_clone.clone(),
                            mode: WindowsSandboxEnableMode::Legacy,
                        });
                    })],
                    dismiss_on_select: true,
                    ..Default::default()
                },
                SelectionItem {
                    name: tr(language, "chatwidget.windows_sandbox.legacy.back").to_string(),
                    description: None,
                    actions: vec![Box::new(|tx| {
                        tx.send(AppEvent::OpenApprovalsPopup);
                    })],
                    dismiss_on_select: true,
                    ..Default::default()
                },
            ];

            self.bottom_pane.show_selection_view(SelectionViewParams {
                title: None,
                footer_hint: Some(standard_popup_hint_line(language)),
                items,
                header: Box::new(header),
                ..Default::default()
            });
            return;
        }

        let current_approval = self.config.approval_policy.value();
        let current_sandbox = self.config.sandbox_policy.get();
        let presets = builtin_approval_presets();
        let stay_full_access = presets
            .iter()
            .find(|preset| preset.id == "full-access")
            .is_some_and(|preset| {
                Self::preset_matches_current(current_approval, current_sandbox, preset)
            });
        let stay_actions = if stay_full_access {
            Vec::new()
        } else {
            presets
                .iter()
                .find(|preset| preset.id == "read-only")
                .map(|preset| {
                    Self::approval_preset_actions(preset.approval, preset.sandbox.clone())
                })
                .unwrap_or_default()
        };
        let stay_label = if stay_full_access {
            tr(language, "chatwidget.windows_sandbox.nux.stay_full_access").to_string()
        } else {
            tr(language, "chatwidget.windows_sandbox.nux.stay_read_only").to_string()
        };
        let mut header = ColumnRenderable::new();
        let header_lines = vec![
            line![tr(language, "chatwidget.windows_sandbox.nux.header_title").bold()],
            line![""],
            line![tr(language, "chatwidget.windows_sandbox.nux.header_body")],
            line![tr(language, "chatwidget.windows_sandbox.nux.learn_more")],
        ];
        header.push(*Box::new(
            Paragraph::new(header_lines).wrap(Wrap { trim: false }),
        ));

        let items = vec![
            SelectionItem {
                name: tr(language, "chatwidget.windows_sandbox.nux.setup_elevated").to_string(),
                description: None,
                actions: vec![Box::new(move |tx| {
                    tx.send(AppEvent::BeginWindowsSandboxElevatedSetup {
                        preset: preset.clone(),
                    });
                })],
                dismiss_on_select: true,
                ..Default::default()
            },
            SelectionItem {
                name: stay_label,
                description: None,
                actions: stay_actions,
                dismiss_on_select: true,
                ..Default::default()
            },
        ];

        self.bottom_pane.show_selection_view(SelectionViewParams {
            title: None,
            footer_hint: Some(standard_popup_hint_line(self.config.language)),
            items,
            header: Box::new(header),
            ..Default::default()
        });
    }

    #[cfg(not(target_os = "windows"))]
    pub(crate) fn open_windows_sandbox_enable_prompt(&mut self, _preset: ApprovalPreset) {}

    #[cfg(target_os = "windows")]
    pub(crate) fn open_windows_sandbox_fallback_prompt(
        &mut self,
        preset: ApprovalPreset,
        reason: WindowsSandboxFallbackReason,
    ) {
        use ratatui_macros::line;

        let _ = reason;

        let current_approval = self.config.approval_policy.value();
        let current_sandbox = self.config.sandbox_policy.get();
        let presets = builtin_approval_presets();
        let stay_full_access = presets
            .iter()
            .find(|preset| preset.id == "full-access")
            .is_some_and(|preset| {
                Self::preset_matches_current(current_approval, current_sandbox, preset)
            });
        let stay_actions = if stay_full_access {
            Vec::new()
        } else {
            presets
                .iter()
                .find(|preset| preset.id == "read-only")
                .map(|preset| {
                    Self::approval_preset_actions(preset.approval, preset.sandbox.clone())
                })
                .unwrap_or_default()
        };
        let stay_label = if stay_full_access {
            "Stay in Agent Full Access".to_string()
        } else {
            "Stay in Read-Only".to_string()
        };

        let mut lines = Vec::new();
        lines.push(line!["Use Non-Elevated Sandbox?".bold()]);
        lines.push(line![""]);
        lines.push(line![
            "Elevation failed. You can also use a non-elevated sandbox, which protects your files and prevents network access under most circumstances. However, it carries greater risk if prompt injected."
        ]);
        lines.push(line![
            "Learn more: https://developers.openai.com/codex/windows"
        ]);

        let mut header = ColumnRenderable::new();
        header.push(*Box::new(Paragraph::new(lines).wrap(Wrap { trim: false })));

        let elevated_preset = preset.clone();
        let legacy_preset = preset;
        let items = vec![
            SelectionItem {
                name: "Try elevated agent sandbox setup again".to_string(),
                description: None,
                actions: vec![Box::new(move |tx| {
                    tx.send(AppEvent::BeginWindowsSandboxElevatedSetup {
                        preset: elevated_preset.clone(),
                    });
                })],
                dismiss_on_select: true,
                ..Default::default()
            },
            SelectionItem {
                name: "Use non-elevated agent sandbox".to_string(),
                description: None,
                actions: vec![Box::new(move |tx| {
                    tx.send(AppEvent::EnableWindowsSandboxForAgentMode {
                        preset: legacy_preset.clone(),
                        mode: WindowsSandboxEnableMode::Legacy,
                    });
                })],
                dismiss_on_select: true,
                ..Default::default()
            },
            SelectionItem {
                name: stay_label,
                description: None,
                actions: stay_actions,
                dismiss_on_select: true,
                ..Default::default()
            },
        ];

        self.bottom_pane.show_selection_view(SelectionViewParams {
            title: None,
            footer_hint: Some(standard_popup_hint_line(self.config.language)),
            items,
            header: Box::new(header),
            ..Default::default()
        });
    }

    #[cfg(not(target_os = "windows"))]
    pub(crate) fn open_windows_sandbox_fallback_prompt(
        &mut self,
        _preset: ApprovalPreset,
        _reason: WindowsSandboxFallbackReason,
    ) {
    }

    #[cfg(target_os = "windows")]
    pub(crate) fn maybe_prompt_windows_sandbox_enable(&mut self) {
        if self.config.forced_auto_mode_downgraded_on_windows
            && WindowsSandboxLevel::from_config(&self.config) == WindowsSandboxLevel::Disabled
            && let Some(preset) = builtin_approval_presets()
                .into_iter()
                .find(|preset| preset.id == "auto")
        {
            self.open_windows_sandbox_enable_prompt(preset);
        }
    }

    #[cfg(not(target_os = "windows"))]
    pub(crate) fn maybe_prompt_windows_sandbox_enable(&mut self) {}

    #[cfg(target_os = "windows")]
    pub(crate) fn show_windows_sandbox_setup_status(&mut self) {
        // While elevated sandbox setup runs, prevent typing so the user doesn't
        // accidentally queue messages that will run under an unexpected mode.
        self.bottom_pane.set_composer_input_enabled(
            false,
            Some("Input disabled until setup completes.".to_string()),
        );
        self.bottom_pane.ensure_status_indicator();
        self.bottom_pane.set_interrupt_hint_visible(false);
        self.set_status_header("Setting up agent sandbox. This can take a minute.".to_string());
        self.request_redraw();
    }

    #[cfg(not(target_os = "windows"))]
    #[allow(dead_code)]
    pub(crate) fn show_windows_sandbox_setup_status(&mut self) {}

    #[cfg(target_os = "windows")]
    pub(crate) fn clear_windows_sandbox_setup_status(&mut self) {
        self.bottom_pane.set_composer_input_enabled(true, None);
        self.bottom_pane.hide_status_indicator();
        self.request_redraw();
    }

    #[cfg(not(target_os = "windows"))]
    pub(crate) fn clear_windows_sandbox_setup_status(&mut self) {}

    #[cfg(target_os = "windows")]
    pub(crate) fn clear_forced_auto_mode_downgrade(&mut self) {
        self.config.forced_auto_mode_downgraded_on_windows = false;
    }

    #[cfg(not(target_os = "windows"))]
    #[allow(dead_code)]
    pub(crate) fn clear_forced_auto_mode_downgrade(&mut self) {}

    /// Set the approval policy in the widget's config copy.
    pub(crate) fn set_approval_policy(&mut self, policy: AskForApproval) {
        if let Err(err) = self.config.approval_policy.set(policy) {
            tracing::warn!(%err, "failed to set approval_policy on chat config");
        }
    }

    pub(crate) fn set_language(&mut self, language: Language) {
        self.config.language = language;
        self.bottom_pane.set_language(language);
    }

    /// Set the sandbox policy in the widget's config copy.
    pub(crate) fn set_sandbox_policy(&mut self, policy: SandboxPolicy) -> ConstraintResult<()> {
        #[cfg(target_os = "windows")]
        let should_clear_downgrade = !matches!(&policy, SandboxPolicy::ReadOnly)
            || WindowsSandboxLevel::from_config(&self.config) != WindowsSandboxLevel::Disabled;

        self.config.sandbox_policy.set(policy)?;

        #[cfg(target_os = "windows")]
        if should_clear_downgrade {
            self.config.forced_auto_mode_downgraded_on_windows = false;
        }

        Ok(())
    }

    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    pub(crate) fn set_feature_enabled(&mut self, feature: Feature, enabled: bool) {
        if enabled {
            self.config.features.enable(feature);
        } else {
            self.config.features.disable(feature);
        }
        if feature == Feature::Steer {
            self.bottom_pane.set_steer_enabled(enabled);
        }
    }

    pub(crate) fn set_full_access_warning_acknowledged(&mut self, acknowledged: bool) {
        self.config.notices.hide_full_access_warning = Some(acknowledged);
    }

    pub(crate) fn set_world_writable_warning_acknowledged(&mut self, acknowledged: bool) {
        self.config.notices.hide_world_writable_warning = Some(acknowledged);
    }

    pub(crate) fn set_rate_limit_switch_prompt_hidden(&mut self, hidden: bool) {
        self.config.notices.hide_rate_limit_model_nudge = Some(hidden);
        if hidden {
            self.rate_limit_switch_prompt = RateLimitSwitchPromptState::Idle;
        }
    }

    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    pub(crate) fn world_writable_warning_hidden(&self) -> bool {
        self.config
            .notices
            .hide_world_writable_warning
            .unwrap_or(false)
    }

    /// Set the reasoning effort in the widget's config copy.
    pub(crate) fn set_reasoning_effort(&mut self, effort: Option<ReasoningEffortConfig>) {
        self.config.model_reasoning_effort = effort;
    }

    /// Set the model in the widget's config copy.
    pub(crate) fn set_model(&mut self, model: &str) {
        self.session_header.set_model(model);
        self.model = Some(model.to_string());
    }

    /// Set the parallel-priority spec toggle in the widget's config copy.
    pub(crate) fn set_spec_parallel_priority(&mut self, enabled: bool) {
        self.config.spec.parallel_priority = enabled;
    }

    /// Set one sub-agent preset model override in the widget's config copy.
    pub(crate) fn set_subagent_preset_model(
        &mut self,
        preset: SubagentPreset,
        model: Option<String>,
    ) {
        match preset {
            SubagentPreset::Edit => self.config.subagent_presets.edit.model = model,
            SubagentPreset::Read => self.config.subagent_presets.read.model = model,
            SubagentPreset::Grep => self.config.subagent_presets.grep.model = model,
            SubagentPreset::Run => self.config.subagent_presets.run.model = model,
            SubagentPreset::Websearch => self.config.subagent_presets.websearch.model = model,
        }
    }

    /// Set one sub-agent preset reasoning override in the widget's config copy.
    pub(crate) fn set_subagent_preset_reasoning_effort(
        &mut self,
        preset: SubagentPreset,
        effort: Option<ReasoningEffortConfig>,
    ) {
        match preset {
            SubagentPreset::Edit => self.config.subagent_presets.edit.reasoning_effort = effort,
            SubagentPreset::Read => self.config.subagent_presets.read.reasoning_effort = effort,
            SubagentPreset::Grep => self.config.subagent_presets.grep.reasoning_effort = effort,
            SubagentPreset::Run => self.config.subagent_presets.run.reasoning_effort = effort,
            SubagentPreset::Websearch => {
                self.config.subagent_presets.websearch.reasoning_effort = effort;
            }
        }
    }

    fn current_model(&self) -> Option<&str> {
        self.model.as_deref()
    }

    fn model_display_name(&self) -> &str {
        self.model.as_deref().unwrap_or(DEFAULT_MODEL_DISPLAY_NAME)
    }

    /// Build a placeholder header cell while the session is configuring.
    fn placeholder_session_header_cell(config: &Config) -> Box<dyn HistoryCell> {
        let placeholder_style = Style::default().add_modifier(Modifier::DIM | Modifier::ITALIC);
        Box::new(history_cell::SessionHeaderHistoryCell::new_with_style(
            DEFAULT_MODEL_DISPLAY_NAME.to_string(),
            placeholder_style,
            None,
            config.cwd.clone(),
            CODEX_CLI_VERSION,
            config.language,
        ))
    }

    /// Merge the real session info cell with any placeholder header to avoid double boxes.
    fn apply_session_info_cell(&mut self, cell: history_cell::SessionInfoCell) {
        let mut session_info_cell = Some(Box::new(cell) as Box<dyn HistoryCell>);
        let merged_header = if let Some(active) = self.active_cell.take() {
            if active
                .as_any()
                .is::<history_cell::SessionHeaderHistoryCell>()
            {
                if let Some(cell) = session_info_cell.take() {
                    self.active_cell = Some(cell);
                }
                true
            } else {
                self.active_cell = Some(active);
                false
            }
        } else {
            false
        };

        self.flush_active_cell();

        if !merged_header && let Some(cell) = session_info_cell {
            self.add_boxed_history(cell);
        }
    }

    pub(crate) fn add_info_message(&mut self, message: String, hint: Option<String>) {
        self.add_to_history(history_cell::new_info_event(message, hint));
        self.request_redraw();
    }

    pub(crate) fn add_plain_history_lines(&mut self, lines: Vec<Line<'static>>) {
        self.add_boxed_history(Box::new(PlainHistoryCell::new(lines)));
        self.request_redraw();
    }

    pub(crate) fn add_error_message(&mut self, message: String) {
        self.add_to_history(history_cell::new_error_event(message));
        self.request_redraw();
    }

    pub(crate) fn add_mcp_output(&mut self) {
        if self.config.mcp_servers.is_empty() {
            self.add_to_history(history_cell::empty_mcp_output(self.config.language));
        } else {
            self.submit_op(Op::ListMcpTools);
        }
    }

    /// Forward file-search results to the bottom pane.
    pub(crate) fn apply_file_search_result(&mut self, query: String, matches: Vec<FileMatch>) {
        self.bottom_pane.on_file_search_result(query, matches);
    }

    /// Handles a Ctrl+C press at the chat-widget layer.
    ///
    /// The first press arms a time-bounded quit shortcut and shows a footer hint via the bottom
    /// pane. If cancellable work is active, Ctrl+C also submits `Op::Interrupt` after the shortcut
    /// is armed.
    ///
    /// If the same quit shortcut is pressed again before expiry, this requests a shutdown-first
    /// quit.
    fn on_ctrl_c(&mut self) {
        let key = key_hint::ctrl(KeyCode::Char('c'));
        let modal_or_popup_active = !self.bottom_pane.no_modal_or_popup_active();
        if self.bottom_pane.on_ctrl_c() == CancellationEvent::Handled {
            if DOUBLE_PRESS_QUIT_SHORTCUT_ENABLED {
                if modal_or_popup_active {
                    self.quit_shortcut_expires_at = None;
                    self.quit_shortcut_key = None;
                    self.bottom_pane.clear_quit_shortcut_hint();
                } else {
                    self.arm_quit_shortcut(key);
                }
            }
            return;
        }

        if !DOUBLE_PRESS_QUIT_SHORTCUT_ENABLED {
            if self.is_cancellable_work_active() {
                self.submit_op(Op::Interrupt);
            } else {
                self.request_quit_without_confirmation();
            }
            return;
        }

        if self.quit_shortcut_active_for(key) {
            self.quit_shortcut_expires_at = None;
            self.quit_shortcut_key = None;
            self.request_quit_without_confirmation();
            return;
        }

        self.arm_quit_shortcut(key);

        if self.is_cancellable_work_active() {
            self.submit_op(Op::Interrupt);
        }
    }

    /// Handles a Ctrl+D press at the chat-widget layer.
    ///
    /// Ctrl-D only participates in quit when the composer is empty and no modal/popup is active.
    /// Otherwise it should be routed to the active view and not attempt to quit.
    fn on_ctrl_d(&mut self) -> bool {
        let key = key_hint::ctrl(KeyCode::Char('d'));
        if !DOUBLE_PRESS_QUIT_SHORTCUT_ENABLED {
            if !self.bottom_pane.composer_is_empty() || !self.bottom_pane.no_modal_or_popup_active()
            {
                return false;
            }

            self.request_quit_without_confirmation();
            return true;
        }

        if self.quit_shortcut_active_for(key) {
            self.quit_shortcut_expires_at = None;
            self.quit_shortcut_key = None;
            self.request_quit_without_confirmation();
            return true;
        }

        if !self.bottom_pane.composer_is_empty() || !self.bottom_pane.no_modal_or_popup_active() {
            return false;
        }

        self.arm_quit_shortcut(key);
        true
    }

    /// True if `key` matches the armed quit shortcut and the window has not expired.
    fn quit_shortcut_active_for(&self, key: KeyBinding) -> bool {
        self.quit_shortcut_key == Some(key)
            && self
                .quit_shortcut_expires_at
                .is_some_and(|expires_at| Instant::now() < expires_at)
    }

    /// Arm the double-press quit shortcut and show the footer hint.
    ///
    /// This keeps the state machine (`quit_shortcut_*`) in `ChatWidget`, since
    /// it is the component that interprets Ctrl+C vs Ctrl+D and decides whether
    /// quitting is currently allowed, while delegating rendering to `BottomPane`.
    fn arm_quit_shortcut(&mut self, key: KeyBinding) {
        self.quit_shortcut_expires_at = Instant::now()
            .checked_add(QUIT_SHORTCUT_TIMEOUT)
            .or_else(|| Some(Instant::now()));
        self.quit_shortcut_key = Some(key);
        self.bottom_pane.show_quit_shortcut_hint(key);
    }

    // Review mode counts as cancellable work so Ctrl+C interrupts instead of quitting.
    fn is_cancellable_work_active(&self) -> bool {
        self.bottom_pane.is_task_running() || self.is_review_mode
    }

    pub(crate) fn composer_is_empty(&self) -> bool {
        self.bottom_pane.composer_is_empty()
    }

    /// True when the UI is in the regular composer state with no running task,
    /// no modal overlay (e.g. approvals or status indicator), and no composer popups.
    /// In this state Esc-Esc backtracking is enabled.
    pub(crate) fn is_normal_backtrack_mode(&self) -> bool {
        self.bottom_pane.is_normal_backtrack_mode()
    }

    pub(crate) fn insert_str(&mut self, text: &str) {
        self.bottom_pane.insert_str(text);
    }

    /// Replace the composer content with the provided text and reset cursor.
    pub(crate) fn set_composer_text(&mut self, text: String) {
        self.bottom_pane.set_composer_text(text);
    }

    pub(crate) fn show_esc_backtrack_hint(&mut self) {
        self.bottom_pane.show_esc_backtrack_hint();
    }

    pub(crate) fn clear_esc_backtrack_hint(&mut self) {
        self.bottom_pane.clear_esc_backtrack_hint();
    }

    /// Return true when the bottom pane currently has an active task.
    ///
    /// This is used by the viewport to decide when mouse selections should
    /// disengage auto-follow behavior while responses are streaming.
    pub(crate) fn is_task_running(&self) -> bool {
        self.bottom_pane.is_task_running()
    }

    /// Inform the bottom pane about the current transcript scroll state.
    ///
    /// This is used by the footer to surface when the inline transcript is
    /// scrolled away from the bottom and to display the current
    /// `(visible_top, total)` scroll position alongside other shortcuts.
    pub(crate) fn set_transcript_ui_state(
        &mut self,
        scrolled: bool,
        selection_active: bool,
        scroll_position: Option<(usize, usize)>,
        copy_selection_key: crate::key_hint::KeyBinding,
        copy_feedback: Option<crate::transcript_copy_action::TranscriptCopyFeedback>,
    ) {
        self.bottom_pane.set_transcript_ui_state(
            scrolled,
            selection_active,
            scroll_position,
            copy_selection_key,
            copy_feedback,
        );
    }

    /// Forward an `Op` directly to codex.
    pub(crate) fn submit_op(&self, op: Op) {
        // Record outbound operation for session replay fidelity.
        crate::session_log::log_outbound_op(&op);
        if let Err(e) = self.codex_op_tx.send(op) {
            tracing::error!("failed to submit op: {e}");
        }
    }

    fn on_list_mcp_tools(&mut self, ev: McpListToolsResponseEvent) {
        let McpListToolsResponseEvent {
            tools,
            resources,
            resource_templates,
            auth_statuses,
        } = ev;
        let tools = serde_json::to_value(tools)
            .and_then(serde_json::from_value::<HashMap<String, mcp_types::Tool>>);
        let resources = serde_json::to_value(resources)
            .and_then(serde_json::from_value::<HashMap<String, Vec<mcp_types::Resource>>>);
        let resource_templates = serde_json::to_value(resource_templates)
            .and_then(serde_json::from_value::<HashMap<String, Vec<mcp_types::ResourceTemplate>>>);

        let (Ok(tools), Ok(resources), Ok(resource_templates)) =
            (tools, resources, resource_templates)
        else {
            tracing::warn!("failed to convert MCP tools response payloads");
            return;
        };
        self.add_to_history(history_cell::new_mcp_tools_output(
            &self.config,
            tools,
            resources,
            resource_templates,
            &auth_statuses,
        ));
    }

    fn on_list_custom_prompts(&mut self, ev: ListCustomPromptsResponseEvent) {
        let len = ev.custom_prompts.len();
        debug!("received {len} custom prompts");
        // Forward to bottom pane so the slash popup can show them now.
        self.bottom_pane.set_custom_prompts(ev.custom_prompts);
    }

    fn on_list_skills(&mut self, ev: ListSkillsResponseEvent) {
        self.set_skills_from_response(&ev);
    }

    pub(crate) fn open_review_popup(&mut self) {
        let language = self.config.language;
        let mut items: Vec<SelectionItem> = Vec::new();

        items.push(SelectionItem {
            name: tr(language, "chatwidget.review.base_branch").to_string(),
            description: Some(tr(language, "chatwidget.review.pr_style_desc").to_string()),
            actions: vec![Box::new({
                let cwd = self.config.cwd.clone();
                move |tx| {
                    tx.send(AppEvent::OpenReviewBranchPicker(cwd.clone()));
                }
            })],
            dismiss_on_select: false,
            ..Default::default()
        });

        items.push(SelectionItem {
            name: tr(language, "chatwidget.review.uncommitted").to_string(),
            actions: vec![Box::new(move |tx: &AppEventSender| {
                tx.send(AppEvent::CodexOp(Op::Review {
                    review_request: ReviewRequest {
                        target: ReviewTarget::UncommittedChanges,
                        user_facing_hint: None,
                    },
                }));
            })],
            dismiss_on_select: true,
            ..Default::default()
        });

        // New: Review a specific commit (opens commit picker)
        items.push(SelectionItem {
            name: tr(language, "chatwidget.review.commit").to_string(),
            actions: vec![Box::new({
                let cwd = self.config.cwd.clone();
                move |tx| {
                    tx.send(AppEvent::OpenReviewCommitPicker(cwd.clone()));
                }
            })],
            dismiss_on_select: false,
            ..Default::default()
        });

        items.push(SelectionItem {
            name: tr(language, "chatwidget.review.custom").to_string(),
            actions: vec![Box::new(move |tx| {
                tx.send(AppEvent::OpenReviewCustomPrompt);
            })],
            dismiss_on_select: false,
            ..Default::default()
        });

        self.bottom_pane.show_selection_view(SelectionViewParams {
            title: Some(tr(language, "chatwidget.review.title").into()),
            footer_hint: Some(standard_popup_hint_line(self.config.language)),
            items,
            ..Default::default()
        });
    }

    pub(crate) async fn show_review_branch_picker(&mut self, cwd: &Path) {
        let branches = local_git_branches(cwd).await;
        let current_branch = current_branch_name(cwd).await.unwrap_or_else(|| {
            tr(self.config.language, "chatwidget.review.detached_head").to_string()
        });
        let mut items: Vec<SelectionItem> = Vec::with_capacity(branches.len());

        for option in branches {
            let branch = option.clone();
            items.push(SelectionItem {
                name: format!("{current_branch} -> {branch}"),
                actions: vec![Box::new(move |tx3: &AppEventSender| {
                    tx3.send(AppEvent::CodexOp(Op::Review {
                        review_request: ReviewRequest {
                            target: ReviewTarget::BaseBranch {
                                branch: branch.clone(),
                            },
                            user_facing_hint: None,
                        },
                    }));
                })],
                dismiss_on_select: true,
                search_value: Some(option),
                ..Default::default()
            });
        }

        self.bottom_pane.show_selection_view(SelectionViewParams {
            title: Some(
                tr(self.config.language, "chatwidget.review.base_branch_title").to_string(),
            ),
            footer_hint: Some(standard_popup_hint_line(self.config.language)),
            items,
            is_searchable: true,
            search_placeholder: Some(
                tr(self.config.language, "chatwidget.review.search_branches").to_string(),
            ),
            ..Default::default()
        });
    }

    pub(crate) async fn show_review_commit_picker(&mut self, cwd: &Path) {
        let commits = codex_core::git_info::recent_commits(cwd, 100).await;

        let mut items: Vec<SelectionItem> = Vec::with_capacity(commits.len());
        for entry in commits {
            let subject = entry.subject.clone();
            let sha = entry.sha.clone();
            let search_val = format!("{subject} {sha}");

            items.push(SelectionItem {
                name: subject.clone(),
                actions: vec![Box::new(move |tx3: &AppEventSender| {
                    tx3.send(AppEvent::CodexOp(Op::Review {
                        review_request: ReviewRequest {
                            target: ReviewTarget::Commit {
                                sha: sha.clone(),
                                title: Some(subject.clone()),
                            },
                            user_facing_hint: None,
                        },
                    }));
                })],
                dismiss_on_select: true,
                search_value: Some(search_val),
                ..Default::default()
            });
        }

        self.bottom_pane.show_selection_view(SelectionViewParams {
            title: Some(tr(self.config.language, "chatwidget.review.commit_title").to_string()),
            footer_hint: Some(standard_popup_hint_line(self.config.language)),
            items,
            is_searchable: true,
            search_placeholder: Some(
                tr(self.config.language, "chatwidget.review.search_commits").to_string(),
            ),
            ..Default::default()
        });
    }

    pub(crate) fn show_review_custom_prompt(&mut self) {
        let tx = self.app_event_tx.clone();
        let view = CustomPromptView::new(
            tr(self.config.language, "chatwidget.review.custom").to_string(),
            tr(self.config.language, "chatwidget.review.custom_hint").to_string(),
            None,
            self.config.language,
            Box::new(move |prompt: String| {
                let trimmed = prompt.trim().to_string();
                if trimmed.is_empty() {
                    return;
                }
                tx.send(AppEvent::CodexOp(Op::Review {
                    review_request: ReviewRequest {
                        target: ReviewTarget::Custom {
                            instructions: trimmed,
                        },
                        user_facing_hint: None,
                    },
                }));
            }),
        );
        self.bottom_pane.show_view(Box::new(view));
    }

    pub(crate) fn token_usage(&self) -> TokenUsage {
        self.token_info
            .as_ref()
            .map(|ti| ti.total_token_usage.clone())
            .unwrap_or_default()
    }

    pub(crate) fn conversation_id(&self) -> Option<ThreadId> {
        self.conversation_id
    }

    pub(crate) fn rollout_path(&self) -> Option<PathBuf> {
        self.current_rollout_path.clone()
    }

    fn is_session_configured(&self) -> bool {
        self.conversation_id.is_some()
    }

    /// Returns a cache key describing the current in-flight active cell for the transcript overlay.
    ///
    /// `Ctrl+T` renders committed transcript cells plus a render-only live tail derived from the
    /// current active cell, and the overlay caches that tail; this key is what it uses to decide
    /// whether it must recompute. When there is no active cell, this returns `None` so the overlay
    /// can drop the tail entirely.
    ///
    /// If callers mutate the active cell's transcript output without bumping the revision (or
    /// providing an appropriate animation tick), the overlay will keep showing a stale tail while
    /// the main viewport updates.
    pub(crate) fn active_cell_transcript_key(&self) -> Option<ActiveCellTranscriptKey> {
        let cell = self.active_cell.as_ref()?;
        Some(ActiveCellTranscriptKey {
            revision: self.active_cell_revision,
            is_stream_continuation: cell.is_stream_continuation(),
            animation_tick: cell.transcript_animation_tick(),
        })
    }

    /// Returns the active cell's transcript lines for a given terminal width.
    ///
    /// This is a convenience for the transcript overlay live-tail path, and it intentionally
    /// filters out empty results so the overlay can treat "nothing to render" as "no tail". Callers
    /// should pass the same width the overlay uses; using a different width will cause wrapping
    /// mismatches between the main viewport and the transcript overlay.
    pub(crate) fn active_cell_transcript_lines(&self, width: u16) -> Option<Vec<Line<'static>>> {
        let cell = self.active_cell.as_ref()?;
        let lines = cell.transcript_lines(width);
        (!lines.is_empty()).then_some(lines)
    }

    /// Return a reference to the widget's current config (includes any
    /// runtime overrides applied via TUI, e.g., model or approval policy).
    pub(crate) fn config_ref(&self) -> &Config {
        &self.config
    }

    fn as_renderable(&self) -> RenderableItem<'_> {
        let active_cell_renderable = match &self.active_cell {
            Some(cell) => RenderableItem::Borrowed(cell).inset(Insets::tlbr(1, 0, 0, 0)),
            None => RenderableItem::Owned(Box::new(())),
        };
        let mut flex = FlexRenderable::new();
        flex.push(1, active_cell_renderable);
        flex.push(
            0,
            RenderableItem::Borrowed(&self.bottom_pane).inset(Insets::tlbr(1, 0, 0, 0)),
        );
        RenderableItem::Owned(Box::new(flex))
    }
}

impl Drop for ChatWidget {
    fn drop(&mut self) {
        self.stop_rate_limit_poller();
    }
}

impl Renderable for ChatWidget {
    fn render(&self, area: Rect, buf: &mut Buffer) {
        self.as_renderable().render(area, buf);
        self.last_rendered_width.set(Some(area.width as usize));
    }

    fn desired_height(&self, width: u16) -> u16 {
        self.as_renderable().desired_height(width)
    }

    fn cursor_pos(&self, area: Rect) -> Option<(u16, u16)> {
        self.as_renderable().cursor_pos(area)
    }
}

enum Notification {
    AgentTurnComplete { response: String },
    ExecApprovalRequested { command: String },
    EditApprovalRequested { cwd: PathBuf, changes: Vec<PathBuf> },
    ElicitationRequested { server_name: String },
}

impl Notification {
    fn display(&self, language: Language) -> String {
        match self {
            Notification::AgentTurnComplete { response } => {
                Notification::agent_turn_preview(response).unwrap_or_else(|| {
                    tr(language, "chatwidget.notification.agent_turn_complete").to_string()
                })
            }
            Notification::ExecApprovalRequested { command } => tr_args(
                language,
                "chatwidget.notification.exec_approval",
                &[("command", &truncate_text(command, 30))],
            ),
            Notification::EditApprovalRequested { cwd, changes } => {
                let target = if changes.len() == 1 {
                    #[allow(clippy::unwrap_used)]
                    display_path_for(changes.first().unwrap(), cwd)
                } else {
                    tr_args(
                        language,
                        "chatwidget.notification.edit_approval_files",
                        &[("count", &changes.len().to_string())],
                    )
                };
                tr_args(
                    language,
                    "chatwidget.notification.edit_approval",
                    &[("target", &target)],
                )
            }
            Notification::ElicitationRequested { server_name } => tr_args(
                language,
                "chatwidget.notification.elicitation",
                &[("server_name", server_name)],
            ),
        }
    }

    fn type_name(&self) -> &str {
        match self {
            Notification::AgentTurnComplete { .. } => "agent-turn-complete",
            Notification::ExecApprovalRequested { .. }
            | Notification::EditApprovalRequested { .. }
            | Notification::ElicitationRequested { .. } => "approval-requested",
        }
    }

    fn allowed_for(&self, settings: &Notifications) -> bool {
        match settings {
            Notifications::Enabled(enabled) => *enabled,
            Notifications::Custom(allowed) => allowed.iter().any(|a| a == self.type_name()),
        }
    }

    fn agent_turn_preview(response: &str) -> Option<String> {
        let mut normalized = String::new();
        for part in response.split_whitespace() {
            if !normalized.is_empty() {
                normalized.push(' ');
            }
            normalized.push_str(part);
        }
        let trimmed = normalized.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(truncate_text(trimmed, AGENT_NOTIFICATION_PREVIEW_GRAPHEMES))
        }
    }
}

const AGENT_NOTIFICATION_PREVIEW_GRAPHEMES: usize = 200;

fn example_prompts(language: Language) -> &'static [String] {
    tr_list(language, "chatwidget.example_prompts")
}

fn example_prompt_placeholder(language: Language) -> String {
    let prompts = example_prompts(language);
    if prompts.is_empty() {
        return String::new();
    }
    let mut rng = rand::rng();
    prompts[rng.random_range(0..prompts.len())].to_string()
}

// Extract the first bold (Markdown) element in the form **...** from `s`.
// Returns the inner text if found; otherwise `None`.
fn extract_first_bold(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut i = 0usize;
    while i + 1 < bytes.len() {
        if bytes[i] == b'*' && bytes[i + 1] == b'*' {
            let start = i + 2;
            let mut j = start;
            while j + 1 < bytes.len() {
                if bytes[j] == b'*' && bytes[j + 1] == b'*' {
                    // Found closing **
                    let inner = &s[start..j];
                    let trimmed = inner.trim();
                    if !trimmed.is_empty() {
                        return Some(trimmed.to_string());
                    } else {
                        return None;
                    }
                }
                j += 1;
            }
            // No closing; stop searching (wait for more deltas)
            return None;
        }
        i += 1;
    }
    None
}

async fn fetch_rate_limits(base_url: String, auth: CodexAuth) -> Option<RateLimitSnapshot> {
    match BackendClient::from_auth(base_url, &auth) {
        Ok(client) => match client.get_rate_limits().await {
            Ok(snapshot) => Some(snapshot),
            Err(err) => {
                debug!(error = ?err, "failed to fetch rate limits from /usage");
                None
            }
        },
        Err(err) => {
            debug!(error = ?err, "failed to construct backend client for rate limits");
            None
        }
    }
}

#[cfg(test)]
pub(crate) fn show_review_commit_picker_with_entries(
    chat: &mut ChatWidget,
    entries: Vec<codex_core::git_info::CommitLogEntry>,
) {
    let mut items: Vec<SelectionItem> = Vec::with_capacity(entries.len());
    for entry in entries {
        let subject = entry.subject.clone();
        let sha = entry.sha.clone();
        let search_val = format!("{subject} {sha}");

        items.push(SelectionItem {
            name: subject.clone(),
            actions: vec![Box::new(move |tx3: &AppEventSender| {
                tx3.send(AppEvent::CodexOp(Op::Review {
                    review_request: ReviewRequest {
                        target: ReviewTarget::Commit {
                            sha: sha.clone(),
                            title: Some(subject.clone()),
                        },
                        user_facing_hint: None,
                    },
                }));
            })],
            dismiss_on_select: true,
            search_value: Some(search_val),
            ..Default::default()
        });
    }

    chat.bottom_pane.show_selection_view(SelectionViewParams {
        title: Some(tr(chat.config.language, "chatwidget.review.commit_title").to_string()),
        footer_hint: Some(standard_popup_hint_line(chat.config.language)),
        items,
        is_searchable: true,
        search_placeholder: Some(
            tr(chat.config.language, "chatwidget.review.search_commits").to_string(),
        ),
        ..Default::default()
    });
}

fn find_skill_mentions(text: &str, skills: &[SkillMetadata]) -> Vec<SkillMetadata> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut matches: Vec<SkillMetadata> = Vec::new();
    for skill in skills {
        if seen.contains(&skill.name) {
            continue;
        }
        let needle = format!("${}", skill.name);
        if text.contains(&needle) {
            seen.insert(skill.name.clone());
            matches.push(skill.clone());
        }
    }
    matches
}

fn skills_for_cwd(cwd: &Path, skills_entries: &[SkillsListEntry]) -> Vec<SkillMetadata> {
    skills_entries
        .iter()
        .find(|entry| entry.cwd.as_path() == cwd)
        .map(|entry| {
            entry
                .skills
                .iter()
                .map(|skill| SkillMetadata {
                    name: skill.name.clone(),
                    description: skill.description.clone(),
                    short_description: skill.short_description.clone(),
                    interface: skill.interface.clone().map(|interface| SkillInterface {
                        display_name: interface.display_name,
                        short_description: interface.short_description,
                        icon_small: interface.icon_small,
                        icon_large: interface.icon_large,
                        brand_color: interface.brand_color,
                        default_prompt: interface.default_prompt,
                    }),
                    dependencies: skill.dependencies.clone().map(|dependencies| {
                        SkillDependencies {
                            tools: dependencies
                                .tools
                                .into_iter()
                                .map(|tool| SkillToolDependency {
                                    r#type: tool.r#type,
                                    value: tool.value,
                                    description: tool.description,
                                    transport: tool.transport,
                                    command: tool.command,
                                    url: tool.url,
                                })
                                .collect(),
                        }
                    }),
                    policy: None,
                    path: skill.path.clone(),
                    scope: skill.scope,
                })
                .collect()
        })
        .unwrap_or_default()
}

fn is_timeout_error(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("timed out") || lower.contains("timeout")
}

#[cfg(test)]
pub(crate) mod tests;
