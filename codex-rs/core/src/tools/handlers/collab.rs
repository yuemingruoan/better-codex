use crate::agent::AgentStatus;
use crate::codex::Session;
use crate::codex::TurnContext;
use crate::config::Config;
use crate::config::types::SubagentPreset;
use crate::error::CodexErr;
use crate::function_tool::FunctionCallError;
use crate::models_manager::manager::RefreshStrategy;
use crate::protocol::AskForApproval;
use crate::protocol::SandboxPolicy;
use crate::thread_manager::AgentSpawnMetadata;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolOutput;
use crate::tools::context::ToolPayload;
use crate::tools::handlers::parse_arguments;
use crate::tools::registry::ToolHandler;
use crate::tools::registry::ToolKind;
use async_trait::async_trait;
use codex_protocol::ThreadId;
use codex_protocol::config_types::ReasoningSummary as ReasoningSummaryConfig;
use codex_protocol::config_types::SandboxMode;
use codex_protocol::models::FunctionCallOutputBody;
use codex_protocol::openai_models::ReasoningEffort;
use codex_protocol::protocol::CollabAgentInteractionBeginEvent;
use codex_protocol::protocol::CollabAgentInteractionEndEvent;
use codex_protocol::protocol::CollabAgentSpawnBeginEvent;
use codex_protocol::protocol::CollabAgentSpawnEndEvent;
use codex_protocol::protocol::CollabCloseBeginEvent;
use codex_protocol::protocol::CollabCloseEndEvent;
use codex_protocol::protocol::CollabResumeBeginEvent;
use codex_protocol::protocol::CollabResumeEndEvent;
use codex_protocol::protocol::CollabWaitingBeginEvent;
use codex_protocol::protocol::CollabWaitingEndEvent;
use codex_protocol::protocol::SessionSource;
use codex_protocol::protocol::SubAgentSource;
use codex_protocol::user_input::UserInput;
use serde::Deserialize;
use serde::Serialize;

pub struct CollabHandler;

#[allow(dead_code)] // Referenced by tests.
pub(crate) const MIN_WAIT_TIMEOUT_MS: i64 = 100;
pub(crate) const DEFAULT_WAIT_TIMEOUT_MS: i64 = crate::config::DEFAULT_COLLAB_WAIT_TIMEOUT_MS;
pub(crate) const MAX_WAIT_TIMEOUT_MS: i64 = 300_000;
const ALLOWED_SPAWN_PRESETS: [&str; 5] = ["edit", "read", "grep", "run", "websearch"];

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum WaitWakeupReason {
    AnyCompleted,
    AllCompleted,
    Timeout,
    NoTargets,
}

#[derive(Debug, Deserialize)]
struct CloseAgentArgs {
    agent_id: String,
}

#[derive(Debug, Default)]
struct SpawnConfigOverrides {
    preset: Option<SubagentPreset>,
    model: Option<String>,
    reasoning_effort: Option<ReasoningEffort>,
    reasoning_summary: Option<ReasoningSummaryConfig>,
    approval_policy: Option<AskForApproval>,
    sandbox_mode: Option<SandboxMode>,
}

#[async_trait]
impl ToolHandler for CollabHandler {
    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    fn matches_kind(&self, payload: &ToolPayload) -> bool {
        matches!(payload, ToolPayload::Function { .. })
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<ToolOutput, FunctionCallError> {
        let ToolInvocation {
            session,
            turn,
            tool_name,
            payload,
            call_id,
            ..
        } = invocation;

        let arguments = match payload {
            ToolPayload::Function { arguments } => arguments,
            _ => {
                return Err(FunctionCallError::RespondToModel(
                    "collab handler received unsupported payload".to_string(),
                ));
            }
        };

        match tool_name.as_str() {
            "spawn_agent" => spawn::handle(session, turn, call_id, arguments).await,
            "send_input" => send_input::handle(session, turn, call_id, arguments).await,
            "resume_agent" => resume_agent::handle(session, turn, call_id, arguments).await,
            "wait" => wait::handle(session, turn, call_id, arguments).await,
            "wait_agents" => wait_agents::handle(session, turn, call_id, arguments).await,
            "list_agents" => list_agents::handle(session, call_id, arguments).await,
            "rename_agent" => rename_agent::handle(session, call_id, arguments).await,
            "close_agent" => close_agent::handle(session, turn, call_id, arguments).await,
            "close_agents" => close_agents::handle(session, turn, call_id, arguments).await,
            other => Err(FunctionCallError::RespondToModel(format!(
                "unsupported collab tool {other}"
            ))),
        }
    }
}

mod spawn {
    use super::*;
    use crate::agent::AgentRole;

    use crate::agent::exceeds_thread_spawn_depth_limit;
    use crate::agent::next_thread_spawn_depth;
    use std::sync::Arc;

    #[derive(Debug, Deserialize)]
    struct SpawnAgentArgs {
        items: Option<Vec<UserInput>>,
        agent_type: Option<AgentRole>,
        name: Option<String>,
        #[serde(rename = "label")]
        label: Option<String>,
        acceptance_criteria: Option<Vec<String>>,
        test_commands: Option<Vec<String>>,
        allow_nested_agents: Option<bool>,
        preset: Option<String>,
        model: Option<String>,
        reasoning_effort: Option<ReasoningEffort>,
        reasoning_summary: Option<ReasoningSummaryConfig>,
        approval_policy: Option<AskForApproval>,
        sandbox_mode: Option<SandboxMode>,
    }

    #[derive(Debug, Serialize)]
    struct SpawnAgentResult {
        agent_id: String,
    }

    pub async fn handle(
        session: Arc<Session>,
        turn: Arc<TurnContext>,
        call_id: String,
        arguments: String,
    ) -> Result<ToolOutput, FunctionCallError> {
        let args: SpawnAgentArgs = parse_arguments(&arguments)?;
        if args.label.is_some() {
            return Err(FunctionCallError::RespondToModel(
                "label is no longer supported; use name instead / label 参数已废弃，请改用 name"
                    .to_string(),
            ));
        }
        let preset = parse_spawn_preset(args.preset.as_deref())?;
        let agent_role = args.agent_type.unwrap_or(AgentRole::Default);
        let input_items = parse_collab_input(args.items)?;
        let prompt = input_preview(&input_items);
        let session_source = turn.session_source.clone();
        let child_depth = next_thread_spawn_depth(&session_source);
        if exceeds_thread_spawn_depth_limit(child_depth) {
            return Err(FunctionCallError::RespondToModel(
                "Agent depth limit reached. Solve the task yourself.".to_string(),
            ));
        }
        validate_spawn_limits(
            session.as_ref(),
            session.conversation_id,
            turn.config.as_ref(),
        )
        .await?;
        session
            .send_event(
                &turn,
                CollabAgentSpawnBeginEvent {
                    call_id: call_id.clone(),
                    sender_thread_id: session.conversation_id,
                    prompt: prompt.clone(),
                }
                .into(),
            )
            .await;
        let overrides = SpawnConfigOverrides {
            preset,
            model: args.model,
            reasoning_effort: args.reasoning_effort,
            reasoning_summary: args.reasoning_summary,
            approval_policy: args.approval_policy,
            sandbox_mode: args.sandbox_mode,
        };
        let mut config =
            build_agent_spawn_config(session.as_ref(), turn.as_ref(), &overrides).await?;
        let metadata = AgentSpawnMetadata {
            creator_thread_id: Some(session.conversation_id),
            label: args.name,
            goal: prompt.clone(),
            acceptance_criteria: args.acceptance_criteria.unwrap_or_default(),
            test_commands: args.test_commands.unwrap_or_default(),
            allow_nested_agents: args.allow_nested_agents.unwrap_or(false),
        };
        agent_role
            .apply_to_config(&mut config)
            .map_err(FunctionCallError::RespondToModel)?;
        let result = session
            .services
            .agent_control
            .spawn_agent_with_metadata_and_source(
                config,
                metadata,
                input_items,
                Some(thread_spawn_source(session.conversation_id, child_depth)),
            )
            .await
            .map_err(collab_spawn_error);
        let (new_thread_id, status) = match &result {
            Ok(thread_id) => (
                Some(*thread_id),
                session.services.agent_control.get_status(*thread_id).await,
            ),
            Err(_) => (None, AgentStatus::NotFound),
        };
        session
            .send_event(
                &turn,
                CollabAgentSpawnEndEvent {
                    call_id,
                    sender_thread_id: session.conversation_id,
                    new_thread_id,
                    prompt,
                    status,
                }
                .into(),
            )
            .await;
        let new_thread_id = result?;

        let content = serde_json::to_string(&SpawnAgentResult {
            agent_id: new_thread_id.to_string(),
        })
        .map_err(|err| {
            FunctionCallError::Fatal(format!("failed to serialize spawn_agent result: {err}"))
        })?;

        Ok(ToolOutput::Function {
            body: FunctionCallOutputBody::Text(content),
            success: Some(true),
        })
    }
}

mod send_input {
    use super::*;
    use std::sync::Arc;

    #[derive(Debug, Deserialize)]
    struct SendInputArgs {
        agent_id: String,
        items: Option<Vec<UserInput>>,
        #[serde(default)]
        interrupt: bool,
    }

    #[derive(Debug, Serialize)]
    struct SendInputResult {
        submission_id: String,
    }

    pub async fn handle(
        session: Arc<Session>,
        turn: Arc<TurnContext>,
        call_id: String,
        arguments: String,
    ) -> Result<ToolOutput, FunctionCallError> {
        let args: SendInputArgs = parse_arguments(&arguments)?;
        let receiver_thread_id = agent_id(&args.agent_id)?;
        let input_items = parse_collab_input(args.items)?;
        let prompt = input_preview(&input_items);
        if args.interrupt {
            session
                .services
                .agent_control
                .interrupt_agent(receiver_thread_id)
                .await
                .map_err(|err| collab_agent_error(receiver_thread_id, err))?;
        }
        session
            .send_event(
                &turn,
                CollabAgentInteractionBeginEvent {
                    call_id: call_id.clone(),
                    sender_thread_id: session.conversation_id,
                    receiver_thread_id,
                    prompt: prompt.clone(),
                }
                .into(),
            )
            .await;
        let result = session
            .services
            .agent_control
            .send_input(receiver_thread_id, input_items)
            .await
            .map_err(|err| collab_agent_error(receiver_thread_id, err));
        let status = session
            .services
            .agent_control
            .get_status(receiver_thread_id)
            .await;
        session
            .send_event(
                &turn,
                CollabAgentInteractionEndEvent {
                    call_id,
                    sender_thread_id: session.conversation_id,
                    receiver_thread_id,
                    prompt,
                    status,
                }
                .into(),
            )
            .await;
        let submission_id = result?;

        let content = serde_json::to_string(&SendInputResult { submission_id }).map_err(|err| {
            FunctionCallError::Fatal(format!("failed to serialize send_input result: {err}"))
        })?;

        Ok(ToolOutput::Function {
            body: FunctionCallOutputBody::Text(content),
            success: Some(true),
        })
    }
}

mod resume_agent {
    use super::*;
    use crate::agent::exceeds_thread_spawn_depth_limit;
    use crate::agent::next_thread_spawn_depth;
    use crate::rollout::find_thread_path_by_id_str;
    use std::sync::Arc;

    #[derive(Debug, Deserialize)]
    struct ResumeAgentArgs {
        agent_id: String,
    }

    #[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
    pub(super) struct ResumeAgentResult {
        pub(super) status: AgentStatus,
    }

    pub async fn handle(
        session: Arc<Session>,
        turn: Arc<TurnContext>,
        call_id: String,
        arguments: String,
    ) -> Result<ToolOutput, FunctionCallError> {
        let args: ResumeAgentArgs = parse_arguments(&arguments)?;
        let receiver_thread_id = agent_id(&args.agent_id)?;
        let child_depth = next_thread_spawn_depth(&turn.session_source);
        if exceeds_thread_spawn_depth_limit(child_depth) {
            return Err(FunctionCallError::RespondToModel(
                "Agent depth limit reached. Solve the task yourself.".to_string(),
            ));
        }

        session
            .send_event(
                &turn,
                CollabResumeBeginEvent {
                    call_id: call_id.clone(),
                    sender_thread_id: session.conversation_id,
                    receiver_thread_id,
                }
                .into(),
            )
            .await;

        let mut status = session
            .services
            .agent_control
            .get_status(receiver_thread_id)
            .await;
        let error = if matches!(status, AgentStatus::NotFound) {
            // If the thread is no longer active, attempt to restore it from rollout.
            match try_resume_closed_agent(
                &session,
                &turn,
                receiver_thread_id,
                &args.agent_id,
                child_depth,
            )
            .await
            {
                Ok(resumed_status) => {
                    status = resumed_status;
                    None
                }
                Err(err) => {
                    status = session
                        .services
                        .agent_control
                        .get_status(receiver_thread_id)
                        .await;
                    Some(err)
                }
            }
        } else {
            None
        };

        session
            .send_event(
                &turn,
                CollabResumeEndEvent {
                    call_id,
                    sender_thread_id: session.conversation_id,
                    receiver_thread_id,
                    status: status.clone(),
                }
                .into(),
            )
            .await;

        if let Some(err) = error {
            return Err(err);
        }

        let content = serde_json::to_string(&ResumeAgentResult { status }).map_err(|err| {
            FunctionCallError::Fatal(format!("failed to serialize resume_agent result: {err}"))
        })?;

        Ok(ToolOutput::Function {
            body: FunctionCallOutputBody::Text(content),
            success: Some(true),
        })
    }

    async fn try_resume_closed_agent(
        session: &Arc<Session>,
        turn: &Arc<TurnContext>,
        receiver_thread_id: ThreadId,
        receiver_id: &str,
        child_depth: i32,
    ) -> Result<AgentStatus, FunctionCallError> {
        let rollout_path = find_thread_path_by_id_str(
            turn.config.codex_home.as_path(),
            receiver_id,
        )
        .await
        .map_err(|err| {
            FunctionCallError::RespondToModel(format!(
                "tool failed: failed to locate rollout for agent {receiver_thread_id}: {err}"
            ))
        })?
        .ok_or_else(|| {
            FunctionCallError::RespondToModel(format!(
                "agent with id {receiver_thread_id} not found"
            ))
        })?;

        let config = build_agent_resume_config(turn.as_ref(), child_depth)?;
        let resumed_thread_id = session
            .services
            .agent_control
            .resume_agent_from_rollout(
                config,
                rollout_path,
                thread_spawn_source(session.conversation_id, child_depth),
            )
            .await
            .map_err(|err| collab_agent_error(receiver_thread_id, err))?;

        Ok(session
            .services
            .agent_control
            .get_status(resumed_thread_id)
            .await)
    }
}

mod wait {
    use super::*;
    use crate::agent::status::is_final;
    use futures::FutureExt;
    use futures::StreamExt;
    use futures::stream::FuturesUnordered;
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::sync::watch::Receiver;
    use tokio::time::Instant;

    use tokio::time::timeout_at;

    #[derive(Debug, Deserialize)]
    struct WaitArgs {
        agent_id: Option<String>,
        agent_ids: Option<Vec<String>>,
        timeout_ms: Option<i64>,
    }

    #[derive(Debug, Serialize)]
    struct WaitResult {
        status: HashMap<ThreadId, AgentStatus>,
        timed_out: bool,
        wakeup_reason: WaitWakeupReason,
    }

    pub async fn handle(
        session: Arc<Session>,
        turn: Arc<TurnContext>,
        call_id: String,
        arguments: String,
    ) -> Result<ToolOutput, FunctionCallError> {
        let args: WaitArgs = parse_arguments(&arguments)?;
        let mut agent_ids = args.agent_ids.unwrap_or_default();
        if agent_ids.is_empty()
            && let Some(agent_id) = args.agent_id
        {
            agent_ids.push(agent_id);
        }
        if agent_ids.is_empty() {
            return Err(FunctionCallError::RespondToModel(
                "agent_ids must be non-empty".to_owned(),
            ));
        }
        let receiver_thread_ids = agent_ids
            .iter()
            .map(|id| agent_id(id))
            .collect::<Result<Vec<_>, _>>()?;

        // Validate timeout.
        let timeout_ms =
            resolve_wait_timeout_ms(args.timeout_ms, turn.config.default_wait_timeout_ms)?;

        session
            .send_event(
                &turn,
                CollabWaitingBeginEvent {
                    sender_thread_id: session.conversation_id,
                    receiver_thread_ids: receiver_thread_ids.clone(),
                    call_id: call_id.clone(),
                }
                .into(),
            )
            .await;

        let mut status_rxs = Vec::with_capacity(receiver_thread_ids.len());
        let mut initial_final_statuses = Vec::new();
        for id in &receiver_thread_ids {
            match session.services.agent_control.subscribe_status(*id).await {
                Ok(rx) => {
                    let status = rx.borrow().clone();
                    if is_final(&status) {
                        initial_final_statuses.push((*id, status));
                    }
                    status_rxs.push((*id, rx));
                }
                Err(CodexErr::ThreadNotFound(_)) => {
                    initial_final_statuses.push((*id, AgentStatus::NotFound));
                }
                Err(err) => {
                    let mut statuses = HashMap::with_capacity(1);
                    statuses.insert(*id, session.services.agent_control.get_status(*id).await);
                    session
                        .send_event(
                            &turn,
                            CollabWaitingEndEvent {
                                sender_thread_id: session.conversation_id,
                                call_id: call_id.clone(),
                                statuses,
                            }
                            .into(),
                        )
                        .await;
                    return Err(collab_agent_error(*id, err));
                }
            }
        }

        let statuses = if !initial_final_statuses.is_empty() {
            initial_final_statuses
        } else if timeout_ms == 0 {
            Vec::new()
        } else {
            // Wait for the first agent to reach a final status.
            let mut futures = FuturesUnordered::new();
            for (id, rx) in status_rxs.into_iter() {
                let session = session.clone();
                futures.push(wait_for_final_status(session, id, rx));
            }
            let mut results = Vec::new();
            let deadline = Instant::now() + Duration::from_millis(timeout_ms as u64);
            loop {
                match timeout_at(deadline, futures.next()).await {
                    Ok(Some(Some(result))) => {
                        results.push(result);
                        break;
                    }
                    Ok(Some(None)) => continue,
                    Ok(None) | Err(_) => break,
                }
            }
            if !results.is_empty() {
                // Drain the unlikely last elements to prevent race.
                loop {
                    match futures.next().now_or_never() {
                        Some(Some(Some(result))) => results.push(result),
                        Some(Some(None)) => continue,
                        Some(None) | None => break,
                    }
                }
            }
            results
        };

        // Convert payload.
        let statuses_map = statuses.clone().into_iter().collect::<HashMap<_, _>>();
        let timed_out = statuses.is_empty();
        let result = WaitResult {
            status: statuses_map.clone(),
            timed_out,
            wakeup_reason: if timed_out {
                WaitWakeupReason::Timeout
            } else {
                WaitWakeupReason::AnyCompleted
            },
        };

        // Final event emission.
        session
            .send_event(
                &turn,
                CollabWaitingEndEvent {
                    sender_thread_id: session.conversation_id,
                    call_id,
                    statuses: statuses_map,
                }
                .into(),
            )
            .await;

        let content = serde_json::to_string(&result).map_err(|err| {
            FunctionCallError::Fatal(format!("failed to serialize wait result: {err}"))
        })?;

        Ok(ToolOutput::Function {
            body: FunctionCallOutputBody::Text(content),
            success: None,
        })
    }

    async fn wait_for_final_status(
        session: Arc<Session>,
        thread_id: ThreadId,
        mut status_rx: Receiver<AgentStatus>,
    ) -> Option<(ThreadId, AgentStatus)> {
        let mut status = status_rx.borrow().clone();
        if is_final(&status) {
            return Some((thread_id, status));
        }

        loop {
            if status_rx.changed().await.is_err() {
                let latest = session.services.agent_control.get_status(thread_id).await;
                return is_final(&latest).then_some((thread_id, latest));
            }
            status = status_rx.borrow().clone();
            if is_final(&status) {
                return Some((thread_id, status));
            }
        }
    }
}

mod wait_agents {
    use super::*;
    use crate::agent::status::is_final;
    use std::collections::HashMap;
    use std::collections::HashSet;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::time::Instant;
    use tokio::time::sleep_until;

    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "snake_case")]
    enum WaitAgentsMode {
        Any,
        All,
    }

    #[derive(Debug, Deserialize)]
    struct WaitAgentsArgs {
        agent_ids: Option<Vec<String>>,
        mode: Option<WaitAgentsMode>,
        timeout_ms: Option<i64>,
    }

    #[derive(Debug, Serialize)]
    struct AgentStatusSnapshot {
        agent_id: String,
        status: AgentStatus,
    }

    #[derive(Debug, Serialize)]
    struct WaitAgentsResult {
        statuses: Vec<AgentStatusSnapshot>,
        completed_agent_ids: Vec<String>,
        timed_out: bool,
        wakeup_reason: WaitWakeupReason,
    }

    pub async fn handle(
        session: Arc<Session>,
        turn: Arc<TurnContext>,
        call_id: String,
        arguments: String,
    ) -> Result<ToolOutput, FunctionCallError> {
        let args: WaitAgentsArgs = parse_arguments(&arguments)?;
        let timeout_ms =
            resolve_wait_timeout_ms(args.timeout_ms, turn.config.default_wait_timeout_ms)?;
        let mode = args.mode.unwrap_or(WaitAgentsMode::Any);
        let target_ids =
            resolve_target_ids(session.as_ref(), args.agent_ids, session.conversation_id).await?;

        session
            .send_event(
                &turn,
                CollabWaitingBeginEvent {
                    sender_thread_id: session.conversation_id,
                    receiver_thread_ids: target_ids.clone(),
                    call_id: call_id.clone(),
                }
                .into(),
            )
            .await;

        let result = wait_for_agents(session.as_ref(), &target_ids, mode, timeout_ms).await?;

        let statuses = result
            .statuses
            .iter()
            .map(|snapshot| {
                let receiver_thread_id =
                    ThreadId::from_string(&snapshot.agent_id).map_err(|err| {
                        FunctionCallError::Fatal(format!(
                            "failed to deserialize wait_agents snapshot id {}: {err:?}",
                            snapshot.agent_id
                        ))
                    })?;
                Ok((receiver_thread_id, snapshot.status.clone()))
            })
            .collect::<Result<HashMap<_, _>, _>>()?;

        session
            .send_event(
                &turn,
                CollabWaitingEndEvent {
                    sender_thread_id: session.conversation_id,
                    call_id: call_id.clone(),
                    statuses,
                }
                .into(),
            )
            .await;

        let content = serde_json::to_string(&result).map_err(|err| {
            FunctionCallError::Fatal(format!("failed to serialize wait_agents result: {err}"))
        })?;
        let success = !result.timed_out
            && result
                .statuses
                .iter()
                .all(|snapshot| !matches!(snapshot.status, AgentStatus::Errored(_)));

        Ok(ToolOutput::Function {
            body: FunctionCallOutputBody::Text(content),
            success: Some(success),
        })
    }

    async fn resolve_target_ids(
        session: &Session,
        agent_ids: Option<Vec<String>>,
        current_thread_id: ThreadId,
    ) -> Result<Vec<ThreadId>, FunctionCallError> {
        let use_explicit_ids = agent_ids
            .as_ref()
            .is_some_and(|agent_ids| !agent_ids.is_empty());
        let mut seen = HashSet::new();
        let parsed_ids = if let Some(agent_ids) = agent_ids
            && !agent_ids.is_empty()
        {
            agent_ids
                .into_iter()
                .map(|id| agent_id(&id))
                .collect::<Result<Vec<_>, _>>()?
        } else {
            let agents = session
                .services
                .agent_control
                .list_agents()
                .await
                .map_err(collab_spawn_error)?;
            agents
                .into_iter()
                .filter_map(|agent| {
                    if agent.creator_thread_id == Some(current_thread_id) {
                        Some(agent.agent_id)
                    } else {
                        None
                    }
                })
                .collect()
        };

        let mut unique_ids = Vec::new();
        for id in parsed_ids {
            if id == current_thread_id {
                continue;
            }
            if seen.insert(id) {
                unique_ids.push(id);
            }
        }

        if use_explicit_ids {
            return Ok(unique_ids);
        }

        let mut active_ids = Vec::new();
        for id in unique_ids {
            let status = session.services.agent_control.get_status(id).await;
            if !is_final(&status) {
                active_ids.push(id);
            }
        }
        Ok(active_ids)
    }

    async fn wait_for_agents(
        session: &Session,
        ids: &[ThreadId],
        mode: WaitAgentsMode,
        timeout_ms: i64,
    ) -> Result<WaitAgentsResult, FunctionCallError> {
        if ids.is_empty() {
            return Ok(WaitAgentsResult {
                statuses: Vec::new(),
                completed_agent_ids: Vec::new(),
                timed_out: false,
                wakeup_reason: WaitWakeupReason::NoTargets,
            });
        }

        let deadline = Instant::now() + Duration::from_millis(timeout_ms as u64);
        loop {
            let statuses = collect_statuses(session, ids).await;
            let completed_agent_ids = statuses
                .iter()
                .filter(|snapshot| is_final(&snapshot.status))
                .map(|snapshot| snapshot.agent_id.clone())
                .collect::<Vec<_>>();
            let all_done = completed_agent_ids.len() == statuses.len();
            let any_done = !completed_agent_ids.is_empty();
            let wakeup_reason = match mode {
                WaitAgentsMode::Any if any_done => Some(WaitWakeupReason::AnyCompleted),
                WaitAgentsMode::All if all_done => Some(WaitWakeupReason::AllCompleted),
                _ => None,
            };
            if let Some(wakeup_reason) = wakeup_reason {
                return Ok(WaitAgentsResult {
                    statuses,
                    completed_agent_ids,
                    timed_out: false,
                    wakeup_reason,
                });
            }

            let now = Instant::now();
            if now >= deadline {
                return Ok(WaitAgentsResult {
                    statuses,
                    completed_agent_ids,
                    timed_out: true,
                    wakeup_reason: WaitWakeupReason::Timeout,
                });
            }

            let next_tick = std::cmp::min(now + Duration::from_millis(50), deadline);
            sleep_until(next_tick).await;
        }
    }

    async fn collect_statuses(session: &Session, ids: &[ThreadId]) -> Vec<AgentStatusSnapshot> {
        let mut statuses = Vec::with_capacity(ids.len());
        for id in ids {
            statuses.push(AgentStatusSnapshot {
                agent_id: id.to_string(),
                status: session.services.agent_control.get_status(*id).await,
            });
        }
        statuses
    }
}

mod list_agents {
    use super::*;
    use std::collections::HashSet;
    use std::sync::Arc;

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
    #[serde(rename_all = "snake_case")]
    enum AgentStatusKind {
        PendingInit,
        Running,
        Completed,
        Errored,
        Shutdown,
        NotFound,
    }

    #[derive(Debug, Deserialize)]
    struct ListAgentsArgs {
        agent_id: Option<String>,
        statuses: Option<Vec<AgentStatusKind>>,
        include_closed: Option<bool>,
    }

    #[derive(Debug, Serialize)]
    struct ListAgentItem {
        agent_id: String,
        creator_agent_id: Option<String>,
        name: Option<String>,
        goal: String,
        acceptance_criteria: Vec<String>,
        test_commands: Vec<String>,
        allow_nested_agents: bool,
        created_at_ms: i64,
        updated_at_ms: i64,
        status: AgentStatus,
        closed: bool,
    }

    #[derive(Debug, Serialize)]
    struct ListAgentsResult {
        agents: Vec<ListAgentItem>,
    }

    pub async fn handle(
        session: Arc<Session>,
        _call_id: String,
        arguments: String,
    ) -> Result<ToolOutput, FunctionCallError> {
        let args: ListAgentsArgs = parse_arguments(&arguments)?;
        let creator_agent_id = args.agent_id.as_deref().map(agent_id).transpose()?;
        let status_filters = args
            .statuses
            .unwrap_or_default()
            .into_iter()
            .collect::<HashSet<_>>();
        let include_closed = args.include_closed.unwrap_or(false);

        let agents = session
            .services
            .agent_control
            .list_agents()
            .await
            .map_err(collab_spawn_error)?;
        let mut items = Vec::new();
        for agent in agents {
            if !include_closed && agent.closed {
                continue;
            }
            if let Some(creator_agent_id) = creator_agent_id
                && agent.creator_thread_id != Some(creator_agent_id)
            {
                continue;
            }
            if !status_filters.is_empty() && !status_filters.contains(&status_kind(&agent.status)) {
                continue;
            }
            items.push(ListAgentItem {
                agent_id: agent.agent_id.to_string(),
                creator_agent_id: agent.creator_thread_id.map(|id| id.to_string()),
                name: agent.label,
                goal: agent.goal,
                acceptance_criteria: agent.acceptance_criteria,
                test_commands: agent.test_commands,
                allow_nested_agents: agent.allow_nested_agents,
                created_at_ms: agent.created_at_ms,
                updated_at_ms: agent.updated_at_ms,
                status: agent.status,
                closed: agent.closed,
            });
        }
        let content =
            serde_json::to_string(&ListAgentsResult { agents: items }).map_err(|err| {
                FunctionCallError::Fatal(format!("failed to serialize list_agents result: {err}"))
            })?;
        Ok(ToolOutput::Function {
            body: FunctionCallOutputBody::Text(content),
            success: Some(true),
        })
    }

    fn status_kind(status: &AgentStatus) -> AgentStatusKind {
        match status {
            AgentStatus::PendingInit => AgentStatusKind::PendingInit,
            AgentStatus::Running => AgentStatusKind::Running,
            AgentStatus::Completed(_) => AgentStatusKind::Completed,
            AgentStatus::Errored(_) => AgentStatusKind::Errored,
            AgentStatus::Shutdown => AgentStatusKind::Shutdown,
            AgentStatus::NotFound => AgentStatusKind::NotFound,
        }
    }
}

mod rename_agent {
    use super::*;
    use std::sync::Arc;

    #[derive(Debug, Deserialize)]
    struct RenameAgentArgs {
        agent_id: String,
        name: String,
    }

    #[derive(Debug, Serialize)]
    struct RenameAgentResult {
        agent_id: String,
        name: String,
    }

    pub async fn handle(
        session: Arc<Session>,
        _call_id: String,
        arguments: String,
    ) -> Result<ToolOutput, FunctionCallError> {
        let args: RenameAgentArgs = parse_arguments(&arguments)?;
        let agent_id = agent_id(&args.agent_id)?;
        let record = session
            .services
            .agent_control
            .rename_agent(agent_id, args.name)
            .await
            .map_err(collab_rename_error)?;
        let content = serde_json::to_string(&RenameAgentResult {
            agent_id: record.agent_id.to_string(),
            name: record.label.unwrap_or_default(),
        })
        .map_err(|err| {
            FunctionCallError::Fatal(format!("failed to serialize rename_agent result: {err}"))
        })?;
        Ok(ToolOutput::Function {
            body: FunctionCallOutputBody::Text(content),
            success: Some(true),
        })
    }
}

pub mod close_agent {
    use super::*;
    use std::sync::Arc;

    #[derive(Debug, Deserialize, Serialize)]
    pub(super) struct CloseAgentResult {
        pub(super) status: AgentStatus,
    }

    pub async fn handle(
        session: Arc<Session>,
        turn: Arc<TurnContext>,
        call_id: String,
        arguments: String,
    ) -> Result<ToolOutput, FunctionCallError> {
        let args: CloseAgentArgs = parse_arguments(&arguments)?;
        let agent_id = agent_id(&args.agent_id)?;
        session
            .send_event(
                &turn,
                CollabCloseBeginEvent {
                    call_id: call_id.clone(),
                    sender_thread_id: session.conversation_id,
                    receiver_thread_id: agent_id,
                }
                .into(),
            )
            .await;
        let status = match session
            .services
            .agent_control
            .subscribe_status(agent_id)
            .await
        {
            Ok(mut status_rx) => status_rx.borrow_and_update().clone(),
            Err(err) => {
                let status = session.services.agent_control.get_status(agent_id).await;
                session
                    .send_event(
                        &turn,
                        CollabCloseEndEvent {
                            call_id: call_id.clone(),
                            sender_thread_id: session.conversation_id,
                            receiver_thread_id: agent_id,
                            status,
                        }
                        .into(),
                    )
                    .await;
                return Err(collab_agent_error(agent_id, err));
            }
        };
        let close_result = if !matches!(status, AgentStatus::Shutdown) {
            session
                .services
                .agent_control
                .shutdown_agent_with_descendants(
                    agent_id,
                    turn.config.auto_close_on_parent_shutdown,
                )
                .await
                .map(|_| ())
        } else {
            Ok(())
        };
        let status = match &close_result {
            Ok(()) => AgentStatus::Shutdown,
            Err(_) => resolve_closed_agent_status(session.as_ref(), agent_id).await,
        };
        session
            .send_event(
                &turn,
                CollabCloseEndEvent {
                    call_id,
                    sender_thread_id: session.conversation_id,
                    receiver_thread_id: agent_id,
                    status: status.clone(),
                }
                .into(),
            )
            .await;
        close_result.map_err(|err| collab_agent_error(agent_id, err))?;

        let content = serde_json::to_string(&CloseAgentResult { status }).map_err(|err| {
            FunctionCallError::Fatal(format!("failed to serialize close_agent result: {err}"))
        })?;

        Ok(ToolOutput::Function {
            body: FunctionCallOutputBody::Text(content),
            success: Some(true),
        })
    }

    async fn resolve_closed_agent_status(session: &Session, agent_id: ThreadId) -> AgentStatus {
        if let Ok(Some(record)) = session
            .services
            .agent_control
            .get_agent_record(agent_id)
            .await
            && record.closed
        {
            return record.status;
        }
        session.services.agent_control.get_status(agent_id).await
    }
}

mod close_agents {
    use super::*;
    use std::collections::HashSet;
    use std::sync::Arc;

    #[derive(Debug, Deserialize)]
    struct CloseAgentsArgs {
        agent_ids: Vec<String>,
        ignore_missing: Option<bool>,
    }

    #[derive(Debug, Serialize)]
    struct CloseAgentItemResult {
        agent_id: String,
        status: AgentStatus,
        closed: bool,
        error: Option<String>,
    }

    #[derive(Debug, Serialize)]
    struct CloseAgentsResult {
        results: Vec<CloseAgentItemResult>,
    }

    pub async fn handle(
        session: Arc<Session>,
        turn: Arc<TurnContext>,
        call_id: String,
        arguments: String,
    ) -> Result<ToolOutput, FunctionCallError> {
        let args: CloseAgentsArgs = parse_arguments(&arguments)?;
        if args.agent_ids.is_empty() {
            return Err(FunctionCallError::RespondToModel(
                "agent_ids must not be empty".to_string(),
            ));
        }
        let ignore_missing = args.ignore_missing.unwrap_or(false);
        let mut seen = HashSet::new();
        let mut agent_ids = Vec::new();
        for id in args.agent_ids {
            let agent_id = agent_id(&id)?;
            if seen.insert(agent_id) {
                agent_ids.push(agent_id);
            }
        }

        let mut results = Vec::with_capacity(agent_ids.len());
        for agent_id in agent_ids {
            session
                .send_event(
                    &turn,
                    CollabCloseBeginEvent {
                        call_id: call_id.clone(),
                        sender_thread_id: session.conversation_id,
                        receiver_thread_id: agent_id,
                    }
                    .into(),
                )
                .await;

            let status_before = session.services.agent_control.get_status(agent_id).await;
            if matches!(status_before, AgentStatus::NotFound) {
                session
                    .send_event(
                        &turn,
                        CollabCloseEndEvent {
                            call_id: call_id.clone(),
                            sender_thread_id: session.conversation_id,
                            receiver_thread_id: agent_id,
                            status: AgentStatus::NotFound,
                        }
                        .into(),
                    )
                    .await;
                let error = if ignore_missing {
                    None
                } else {
                    Some(format!("agent with id {agent_id} not found"))
                };
                results.push(CloseAgentItemResult {
                    agent_id: agent_id.to_string(),
                    status: AgentStatus::NotFound,
                    closed: false,
                    error,
                });
                continue;
            }

            let close_result = session
                .services
                .agent_control
                .shutdown_agent_with_descendants(
                    agent_id,
                    turn.config.auto_close_on_parent_shutdown,
                )
                .await;
            let status = if close_result.is_ok() {
                AgentStatus::Shutdown
            } else {
                session.services.agent_control.get_status(agent_id).await
            };
            session
                .send_event(
                    &turn,
                    CollabCloseEndEvent {
                        call_id: call_id.clone(),
                        sender_thread_id: session.conversation_id,
                        receiver_thread_id: agent_id,
                        status: status.clone(),
                    }
                    .into(),
                )
                .await;
            results.push(CloseAgentItemResult {
                agent_id: agent_id.to_string(),
                status,
                closed: close_result.is_ok(),
                error: close_result
                    .err()
                    .map(|err| format!("collab close failed: {err}")),
            });
        }

        let success = results.iter().all(|result| result.error.is_none());
        let content = serde_json::to_string(&CloseAgentsResult { results }).map_err(|err| {
            FunctionCallError::Fatal(format!("failed to serialize close_agents result: {err}"))
        })?;
        Ok(ToolOutput::Function {
            body: FunctionCallOutputBody::Text(content),
            success: Some(success),
        })
    }
}

fn agent_id(id: &str) -> Result<ThreadId, FunctionCallError> {
    ThreadId::from_string(id)
        .map_err(|e| FunctionCallError::RespondToModel(format!("invalid agent id {id}: {e:?}")))
}

fn collab_spawn_error(err: CodexErr) -> FunctionCallError {
    match err {
        CodexErr::UnsupportedOperation(_) => {
            FunctionCallError::RespondToModel("collab manager unavailable".to_string())
        }
        err => FunctionCallError::RespondToModel(format!("collab spawn failed: {err}")),
    }
}

fn collab_agent_error(agent_id: ThreadId, err: CodexErr) -> FunctionCallError {
    match err {
        CodexErr::ThreadNotFound(id) => {
            FunctionCallError::RespondToModel(format!("agent with id {id} not found"))
        }
        CodexErr::InternalAgentDied => {
            FunctionCallError::RespondToModel(format!("agent with id {agent_id} is closed"))
        }
        CodexErr::UnsupportedOperation(_) => {
            FunctionCallError::RespondToModel("collab manager unavailable".to_string())
        }
        err => FunctionCallError::RespondToModel(format!("collab tool failed: {err}")),
    }
}

fn collab_rename_error(err: CodexErr) -> FunctionCallError {
    match err {
        CodexErr::ThreadNotFound(id) => FunctionCallError::RespondToModel(format!(
            "agent with id {id} not found; use list_agents to verify the id"
        )),
        CodexErr::UnsupportedOperation(_) => {
            FunctionCallError::RespondToModel("collab manager unavailable".to_string())
        }
        err => FunctionCallError::RespondToModel(format!("rename_agent failed: {err}")),
    }
}

fn resolve_wait_timeout_ms(
    timeout_ms: Option<i64>,
    default_timeout_ms: i64,
) -> Result<i64, FunctionCallError> {
    let timeout_ms = timeout_ms.unwrap_or(default_timeout_ms);
    match timeout_ms {
        ms if ms < 0 => Err(FunctionCallError::RespondToModel(
            "timeout_ms must be greater than or equal to zero".to_owned(),
        )),
        0 => Ok(0),
        ms => Ok(ms.clamp(MIN_WAIT_TIMEOUT_MS, MAX_WAIT_TIMEOUT_MS)),
    }
}

async fn validate_spawn_limits(
    session: &Session,
    creator_thread_id: ThreadId,
    config: &Config,
) -> Result<(), FunctionCallError> {
    if let Some(parent) = session
        .services
        .agent_control
        .get_agent_record(creator_thread_id)
        .await
        .map_err(collab_spawn_error)?
        && !parent.allow_nested_agents
    {
        return Err(FunctionCallError::RespondToModel(
            "nested agent spawning is disabled for this agent".to_string(),
        ));
    }

    let depth = spawn_depth(session, creator_thread_id).await?;
    if depth >= config.max_spawn_depth {
        return Err(FunctionCallError::RespondToModel(format!(
            "maximum spawn depth ({}) reached",
            config.max_spawn_depth
        )));
    }

    let active_subagent_count = session
        .services
        .agent_control
        .list_agents()
        .await
        .map_err(collab_spawn_error)?
        .into_iter()
        .filter(|agent| {
            agent.creator_thread_id == Some(creator_thread_id)
                && !agent.closed
                && matches!(
                    agent.status,
                    AgentStatus::PendingInit | AgentStatus::Running
                )
        })
        .count();
    if active_subagent_count >= config.max_active_subagents_per_thread {
        return Err(FunctionCallError::RespondToModel(format!(
            "maximum active sub-agents per thread ({}) reached",
            config.max_active_subagents_per_thread
        )));
    }

    Ok(())
}

async fn spawn_depth(
    session: &Session,
    mut thread_id: ThreadId,
) -> Result<usize, FunctionCallError> {
    let mut depth = 0;
    while let Some(record) = session
        .services
        .agent_control
        .get_agent_record(thread_id)
        .await
        .map_err(collab_spawn_error)?
    {
        let Some(parent_thread_id) = record.creator_thread_id else {
            break;
        };
        depth += 1;
        thread_id = parent_thread_id;
    }
    Ok(depth)
}

async fn build_agent_spawn_config(
    session: &Session,
    turn: &TurnContext,
    overrides: &SpawnConfigOverrides,
) -> Result<Config, FunctionCallError> {
    let mut config = build_agent_shared_config(turn)?;
    config.base_instructions = Some(session.get_base_instructions().await.text);
    config.model = Some(turn.model_info.slug.clone());
    config.model_provider = turn.provider.clone();
    config.model_reasoning_effort = turn.reasoning_effort;
    config.model_reasoning_summary = turn.reasoning_summary;
    config.developer_instructions = turn.developer_instructions.clone();
    config.compact_prompt = turn.compact_prompt.clone();
    config.shell_environment_policy = turn.shell_environment_policy.clone();
    config.codex_linux_sandbox_exe = turn.codex_linux_sandbox_exe.clone();
    config.cwd = turn.cwd.clone();
    config
        .approval_policy
        .set(turn.approval_policy)
        .map_err(|err| {
            FunctionCallError::RespondToModel(format!("approval_policy is invalid: {err}"))
        })?;
    config
        .sandbox_policy
        .set(turn.sandbox_policy.clone())
        .map_err(|err| {
            FunctionCallError::RespondToModel(format!("sandbox_policy is invalid: {err}"))
        })?;

    apply_spawn_model_overrides(session, turn, &mut config, overrides).await?;
    let allow_subagent_permission_escalation = config.allow_subagent_permission_escalation;
    apply_spawn_permission_overrides(
        turn,
        &mut config,
        overrides,
        allow_subagent_permission_escalation,
    )?;

    Ok(config)
}

fn build_agent_resume_config(
    turn: &TurnContext,
    _child_depth: i32,
) -> Result<Config, FunctionCallError> {
    let mut config = build_agent_shared_config(turn)?;
    // For resume, keep base instructions sourced from rollout/session metadata.
    config.base_instructions = None;
    Ok(config)
}

fn build_agent_shared_config(turn: &TurnContext) -> Result<Config, FunctionCallError> {
    let mut config = (*turn.config).clone();
    config.model = Some(turn.model_info.slug.clone());
    config.model_provider = turn.provider.clone();
    config.model_reasoning_effort = turn.reasoning_effort;
    config.model_reasoning_summary = turn.reasoning_summary;
    config.developer_instructions = turn.developer_instructions.clone();
    config.compact_prompt = turn.compact_prompt.clone();
    config.shell_environment_policy = turn.shell_environment_policy.clone();
    config.codex_linux_sandbox_exe = turn.codex_linux_sandbox_exe.clone();
    config.cwd = turn.cwd.clone();
    config
        .approval_policy
        .set(turn.approval_policy)
        .map_err(|err| {
            FunctionCallError::RespondToModel(format!("approval_policy is invalid: {err}"))
        })?;
    config
        .sandbox_policy
        .set(turn.sandbox_policy.clone())
        .map_err(|err| {
            FunctionCallError::RespondToModel(format!("sandbox_policy is invalid: {err}"))
        })?;
    Ok(config)
}

fn thread_spawn_source(parent_thread_id: ThreadId, depth: i32) -> SessionSource {
    SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
        parent_thread_id,
        depth,
    })
}

fn parse_collab_input(items: Option<Vec<UserInput>>) -> Result<Vec<UserInput>, FunctionCallError> {
    match items {
        None => Err(FunctionCallError::RespondToModel(
            "Provide required field: items".to_string(),
        )),
        Some(items) if items.is_empty() => Err(FunctionCallError::RespondToModel(
            "Items can't be empty".to_string(),
        )),
        Some(items) => Ok(items),
    }
}

fn input_preview(items: &[UserInput]) -> String {
    let parts: Vec<String> = items
        .iter()
        .map(|item| match item {
            UserInput::Text { text, .. } => text.clone(),
            UserInput::Image { .. } => "[image]".to_string(),
            UserInput::LocalImage { path } => format!("[local_image:{}]", path.display()),
            UserInput::Skill { name, path } => {
                format!("[skill:${name}]({})", path.display())
            }
            UserInput::Mention { name, path } => format!("[mention:${name}]({path})"),
            _ => "[input]".to_string(),
        })
        .collect();

    parts.join("\n")
}

fn parse_spawn_preset(preset: Option<&str>) -> Result<Option<SubagentPreset>, FunctionCallError> {
    let Some(preset) = preset.map(str::trim).filter(|preset| !preset.is_empty()) else {
        return Ok(None);
    };
    let parsed = match preset {
        "edit" => SubagentPreset::Edit,
        "read" => SubagentPreset::Read,
        "grep" => SubagentPreset::Grep,
        "run" => SubagentPreset::Run,
        "websearch" => SubagentPreset::Websearch,
        _ => {
            let allowed = ALLOWED_SPAWN_PRESETS.join("|");
            return Err(FunctionCallError::RespondToModel(format!(
                "unsupported preset `{preset}`; expected one of {allowed} / 不支持的 preset `{preset}`，可选值：{allowed}"
            )));
        }
    };
    Ok(Some(parsed))
}

async fn apply_spawn_model_overrides(
    session: &Session,
    turn: &TurnContext,
    config: &mut Config,
    overrides: &SpawnConfigOverrides,
) -> Result<(), FunctionCallError> {
    let preset_config = overrides.preset.map(|preset| match preset {
        SubagentPreset::Edit => &turn.config.subagent_presets.edit,
        SubagentPreset::Read => &turn.config.subagent_presets.read,
        SubagentPreset::Grep => &turn.config.subagent_presets.grep,
        SubagentPreset::Run => &turn.config.subagent_presets.run,
        SubagentPreset::Websearch => &turn.config.subagent_presets.websearch,
    });
    let preset_reasoning_effort = preset_config.and_then(|preset| preset.reasoning_effort);
    if let Some(model) = overrides
        .model
        .as_deref()
        .or_else(|| preset_config.and_then(|preset| preset.model.as_deref()))
    {
        let model = model.trim();
        if model.is_empty() {
            return Err(FunctionCallError::RespondToModel(
                "model must not be empty".to_string(),
            ));
        }
        validate_spawn_model_override(session, config, &turn.model_info.slug, model).await?;
        config.model = Some(model.to_string());
    }

    if let Some(reasoning_effort) = overrides.reasoning_effort.or(preset_reasoning_effort) {
        config.model_reasoning_effort = Some(reasoning_effort);
    }
    if let Some(reasoning_summary) = overrides.reasoning_summary {
        config.model_reasoning_summary = reasoning_summary;
    }

    let model = config
        .model
        .clone()
        .unwrap_or_else(|| turn.model_info.slug.clone());
    let model_info = session
        .services
        .models_manager
        .get_model_info(&model, config)
        .await;

    if config.model_reasoning_effort.is_some()
        && overrides.reasoning_effort.is_none()
        && preset_reasoning_effort.is_none()
        && model_info.supported_reasoning_levels.is_empty()
    {
        config.model_reasoning_effort = None;
    }

    if let Some(reasoning_effort) = config.model_reasoning_effort {
        if !model_info
            .supported_reasoning_levels
            .iter()
            .any(|preset| preset.effort == reasoning_effort)
        {
            let supported = model_info
                .supported_reasoning_levels
                .iter()
                .map(|preset| preset.effort.to_string())
                .collect::<Vec<_>>();
            let supported = if supported.is_empty() {
                "none".to_string()
            } else {
                supported.join(", ")
            };
            return Err(FunctionCallError::RespondToModel(format!(
                "reasoning_effort {reasoning_effort} is not supported by model {model}; supported values: {supported}"
            )));
        }
    }

    Ok(())
}

async fn validate_spawn_model_override(
    session: &Session,
    config: &Config,
    current_model: &str,
    requested_model: &str,
) -> Result<(), FunctionCallError> {
    if requested_model == current_model {
        return Ok(());
    }

    let available_models = session
        .services
        .models_manager
        .list_models(config, RefreshStrategy::Offline)
        .await;
    if available_models
        .iter()
        .any(|preset| preset.model == requested_model)
    {
        return Ok(());
    }

    Err(FunctionCallError::RespondToModel(format!(
        "model {requested_model} is not available"
    )))
}

fn apply_spawn_permission_overrides(
    turn: &TurnContext,
    config: &mut Config,
    overrides: &SpawnConfigOverrides,
    allow_subagent_permission_escalation: bool,
) -> Result<(), FunctionCallError> {
    if let Some(approval_policy) = overrides.approval_policy {
        if !allow_subagent_permission_escalation
            && approval_policy_level(approval_policy) > approval_policy_level(turn.approval_policy)
        {
            return Err(FunctionCallError::RespondToModel(format!(
                "approval_policy {approval_policy} is not allowed because it is more permissive than parent policy {}",
                turn.approval_policy
            )));
        }
        config.approval_policy.set(approval_policy).map_err(|err| {
            FunctionCallError::RespondToModel(format!("approval_policy is invalid: {err}"))
        })?;
    }

    if let Some(sandbox_mode) = overrides.sandbox_mode {
        let sandbox_policy = sandbox_policy_from_mode(sandbox_mode);
        if !allow_subagent_permission_escalation
            && sandbox_policy_level(&sandbox_policy) > sandbox_policy_level(&turn.sandbox_policy)
        {
            return Err(FunctionCallError::RespondToModel(format!(
                "sandbox_mode {sandbox_mode} is not allowed because it is more permissive than parent mode {}",
                sandbox_mode_from_policy(&turn.sandbox_policy)
            )));
        }
        config.sandbox_policy.set(sandbox_policy).map_err(|err| {
            FunctionCallError::RespondToModel(format!("sandbox_policy is invalid: {err}"))
        })?;
    }

    Ok(())
}

fn approval_policy_level(policy: AskForApproval) -> u8 {
    match policy {
        AskForApproval::Never => 0,
        AskForApproval::UnlessTrusted => 1,
        AskForApproval::OnRequest => 2,
        AskForApproval::OnFailure => 3,
    }
}

fn sandbox_policy_level(policy: &SandboxPolicy) -> u8 {
    match policy {
        SandboxPolicy::ReadOnly => 0,
        SandboxPolicy::WorkspaceWrite { .. } => 1,
        SandboxPolicy::DangerFullAccess | SandboxPolicy::ExternalSandbox { .. } => 2,
    }
}

fn sandbox_policy_from_mode(mode: SandboxMode) -> SandboxPolicy {
    match mode {
        SandboxMode::ReadOnly => SandboxPolicy::new_read_only_policy(),
        SandboxMode::WorkspaceWrite => SandboxPolicy::new_workspace_write_policy(),
        SandboxMode::DangerFullAccess => SandboxPolicy::DangerFullAccess,
    }
}

fn sandbox_mode_from_policy(policy: &SandboxPolicy) -> SandboxMode {
    match policy {
        SandboxPolicy::ReadOnly => SandboxMode::ReadOnly,
        SandboxPolicy::WorkspaceWrite { .. } => SandboxMode::WorkspaceWrite,
        SandboxPolicy::DangerFullAccess | SandboxPolicy::ExternalSandbox { .. } => {
            SandboxMode::DangerFullAccess
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AuthManager;
    use crate::CodexAuth;
    use crate::ThreadManager;
    use crate::agent::MAX_THREAD_SPAWN_DEPTH;
    use crate::built_in_model_providers;
    use crate::codex::make_session_and_context;
    use crate::codex::make_session_and_context_with_rx;
    use crate::config::Config;
    use crate::config::types::ShellEnvironmentPolicy;
    use crate::function_tool::FunctionCallError;
    use crate::protocol::AskForApproval;
    use crate::protocol::Op;
    use crate::protocol::SandboxPolicy;
    use crate::turn_diff_tracker::TurnDiffTracker;
    use codex_protocol::ThreadId;
    use codex_protocol::models::ContentItem;
    use codex_protocol::models::ResponseItem;
    use codex_protocol::protocol::Event;
    use codex_protocol::protocol::EventMsg;
    use codex_protocol::protocol::InitialHistory;
    use codex_protocol::protocol::RolloutItem;
    use pretty_assertions::assert_eq;
    use serde::Deserialize;
    use serde_json::json;
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::sync::Mutex;
    use tokio::time::timeout;

    fn invocation(
        session: Arc<crate::codex::Session>,
        turn: Arc<TurnContext>,
        tool_name: &str,
        payload: ToolPayload,
    ) -> ToolInvocation {
        ToolInvocation {
            session,
            turn,
            tracker: Arc::new(Mutex::new(TurnDiffTracker::default())),
            call_id: "call-1".to_string(),
            tool_name: tool_name.to_string(),
            payload,
        }
    }

    fn function_payload(args: serde_json::Value) -> ToolPayload {
        ToolPayload::Function {
            arguments: args.to_string(),
        }
    }

    fn thread_manager() -> ThreadManager {
        ThreadManager::with_models_provider(
            CodexAuth::from_api_key("dummy"),
            built_in_model_providers()["openai"].clone(),
        )
    }

    fn overwrite_turn_config(turn: &mut TurnContext, mutate: impl FnOnce(&mut Config)) {
        let mut config = (*turn.config).clone();
        mutate(&mut config);
        turn.config = Arc::new(config);
    }

    async fn recv_close_end_status(rx: &async_channel::Receiver<Event>) -> AgentStatus {
        loop {
            let event = timeout(Duration::from_secs(1), rx.recv())
                .await
                .expect("collab close end event should arrive")
                .expect("event channel should stay open");
            if let EventMsg::CollabCloseEnd(event) = event.msg {
                return event.status;
            }
        }
    }

    #[tokio::test]
    async fn handler_rejects_non_function_payloads() {
        let (session, turn) = make_session_and_context().await;
        let invocation = invocation(
            Arc::new(session),
            Arc::new(turn),
            "spawn_agent",
            ToolPayload::Custom {
                input: "hello".to_string(),
            },
        );
        let Err(err) = CollabHandler.handle(invocation).await else {
            panic!("payload should be rejected");
        };
        assert_eq!(
            err,
            FunctionCallError::RespondToModel(
                "collab handler received unsupported payload".to_string()
            )
        );
    }

    #[tokio::test]
    async fn handler_rejects_unknown_tool() {
        let (session, turn) = make_session_and_context().await;
        let invocation = invocation(
            Arc::new(session),
            Arc::new(turn),
            "unknown_tool",
            function_payload(json!({})),
        );
        let Err(err) = CollabHandler.handle(invocation).await else {
            panic!("tool should be rejected");
        };
        assert_eq!(
            err,
            FunctionCallError::RespondToModel("unsupported collab tool unknown_tool".to_string())
        );
    }

    #[tokio::test]
    async fn spawn_agent_requires_items() {
        let (session, turn) = make_session_and_context().await;
        let invocation = invocation(
            Arc::new(session),
            Arc::new(turn),
            "spawn_agent",
            function_payload(json!({})),
        );
        let Err(err) = CollabHandler.handle(invocation).await else {
            panic!("missing items should be rejected");
        };
        assert_eq!(
            err,
            FunctionCallError::RespondToModel("Provide required field: items".to_string())
        );
    }

    #[tokio::test]
    async fn spawn_agent_rejects_legacy_label_parameter() {
        let (session, turn) = make_session_and_context().await;
        let invocation = invocation(
            Arc::new(session),
            Arc::new(turn),
            "spawn_agent",
            function_payload(json!({
                "items": [{"type": "text", "text": "do work"}],
                "label": "legacy-name"
            })),
        );
        let Err(err) = CollabHandler.handle(invocation).await else {
            panic!("legacy label should be rejected");
        };
        assert_eq!(
            err,
            FunctionCallError::RespondToModel(
                "label is no longer supported; use name instead / label 参数已废弃，请改用 name"
                    .to_string()
            )
        );
    }

    #[tokio::test]
    async fn spawn_agent_rejects_empty_items() {
        let (session, turn) = make_session_and_context().await;
        let invocation = invocation(
            Arc::new(session),
            Arc::new(turn),
            "spawn_agent",
            function_payload(json!({"items": []})),
        );
        let Err(err) = CollabHandler.handle(invocation).await else {
            panic!("empty items should be rejected");
        };
        assert_eq!(
            err,
            FunctionCallError::RespondToModel("Items can't be empty".to_string())
        );
    }

    #[tokio::test]
    async fn spawn_agent_errors_when_manager_dropped() {
        let (session, turn) = make_session_and_context().await;
        let invocation = invocation(
            Arc::new(session),
            Arc::new(turn),
            "spawn_agent",
            function_payload(json!({"items": [{"type": "text", "text": "hello"}]})),
        );
        let Err(err) = CollabHandler.handle(invocation).await else {
            panic!("spawn should fail without a manager");
        };
        assert_eq!(
            err,
            FunctionCallError::RespondToModel("collab manager unavailable".to_string())
        );
    }

    #[tokio::test]
    async fn spawn_agent_rejects_unknown_model_override() {
        let (mut session, turn) = make_session_and_context().await;
        let manager = thread_manager();
        session.services.agent_control = manager.agent_control();
        let invocation = invocation(
            Arc::new(session),
            Arc::new(turn),
            "spawn_agent",
            function_payload(
                json!({"items": [{"type": "text", "text": "hello"}], "model": "not-a-real-model"}),
            ),
        );
        let Err(err) = CollabHandler.handle(invocation).await else {
            panic!("unknown model should be rejected");
        };
        assert_eq!(
            err,
            FunctionCallError::RespondToModel(
                "model not-a-real-model is not available".to_string()
            )
        );
    }

    #[tokio::test]
    async fn spawn_agent_rejects_unknown_preset() {
        let (session, turn) = make_session_and_context().await;
        let invocation = invocation(
            Arc::new(session),
            Arc::new(turn),
            "spawn_agent",
            function_payload(json!({
                "items": [{"type": "text", "text": "hello"}],
                "preset": "not-a-real-preset"
            })),
        );
        let Err(err) = CollabHandler.handle(invocation).await else {
            panic!("unknown preset should be rejected");
        };
        assert_eq!(
            err,
            FunctionCallError::RespondToModel(
                "unsupported preset `not-a-real-preset`; expected one of edit|read|grep|run|websearch / 不支持的 preset `not-a-real-preset`，可选值：edit|read|grep|run|websearch".to_string()
            )
        );
    }

    #[tokio::test]
    async fn spawn_agent_rejects_invalid_reasoning_effort_value() {
        let (session, turn) = make_session_and_context().await;
        let invocation = invocation(
            Arc::new(session),
            Arc::new(turn),
            "spawn_agent",
            function_payload(
                json!({"items": [{"type": "text", "text": "hello"}], "reasoning_effort": "invalid"}),
            ),
        );
        let Err(err) = CollabHandler.handle(invocation).await else {
            panic!("invalid reasoning_effort should be rejected");
        };
        let FunctionCallError::RespondToModel(msg) = err else {
            panic!("expected respond-to-model error");
        };
        assert!(msg.starts_with("failed to parse function arguments:"));
    }

    #[tokio::test]
    async fn spawn_agent_rejects_unsupported_reasoning_effort_for_model() {
        let (mut session, turn) = make_session_and_context().await;
        let manager = thread_manager();
        session.services.agent_control = manager.agent_control();
        let config = turn.config.clone();
        let model = turn.model_info.slug.clone();
        let model_info = session
            .services
            .models_manager
            .get_model_info(&model, config.as_ref())
            .await;
        let unsupported_effort = [
            ReasoningEffort::None,
            ReasoningEffort::Minimal,
            ReasoningEffort::Low,
            ReasoningEffort::Medium,
            ReasoningEffort::High,
            ReasoningEffort::XHigh,
        ]
        .into_iter()
        .find(|effort| {
            !model_info
                .supported_reasoning_levels
                .iter()
                .any(|preset| preset.effort == *effort)
        })
        .expect("expected at least one unsupported effort for the default model");
        let invocation = invocation(
            Arc::new(session),
            Arc::new(turn),
            "spawn_agent",
            function_payload(json!({
                "items": [{"type": "text", "text": "hello"}],
                "reasoning_effort": unsupported_effort.to_string(),
            })),
        );
        let Err(err) = CollabHandler.handle(invocation).await else {
            panic!("unsupported effort should be rejected");
        };
        let FunctionCallError::RespondToModel(msg) = err else {
            panic!("expected respond-to-model error");
        };
        assert!(msg.contains("reasoning_effort"));
        assert!(msg.contains("is not supported by model"));
    }

    #[tokio::test]
    async fn spawn_agent_rejects_nested_spawn_when_parent_disallows_it() {
        let (mut session, turn) = make_session_and_context().await;
        let manager = thread_manager();
        let control = manager.agent_control();
        session.services.agent_control = control.clone();
        let root_thread_id = session.conversation_id;
        let config = turn.config.as_ref().clone();
        let child_id = control
            .spawn_agent_with_metadata(
                config,
                "child".to_string(),
                AgentSpawnMetadata {
                    creator_thread_id: Some(root_thread_id),
                    allow_nested_agents: false,
                    ..AgentSpawnMetadata::default()
                },
            )
            .await
            .expect("spawn child");
        session.conversation_id = child_id;
        let invocation = invocation(
            Arc::new(session),
            Arc::new(turn),
            "spawn_agent",
            function_payload(json!({"items": [{"type": "text", "text": "grandchild"}]})),
        );
        let Err(err) = CollabHandler.handle(invocation).await else {
            panic!("nested spawn should be rejected");
        };
        assert_eq!(
            err,
            FunctionCallError::RespondToModel(
                "nested agent spawning is disabled for this agent".to_string()
            )
        );
    }

    #[tokio::test]
    async fn spawn_agent_rejects_when_spawn_depth_limit_is_reached() {
        let (mut session, turn) = make_session_and_context().await;
        let manager = thread_manager();
        let control = manager.agent_control();
        session.services.agent_control = control.clone();
        let root_thread_id = session.conversation_id;
        let config = turn.config.as_ref().clone();
        let child_id = control
            .spawn_agent_with_metadata(
                config,
                "child".to_string(),
                AgentSpawnMetadata {
                    creator_thread_id: Some(root_thread_id),
                    allow_nested_agents: true,
                    ..AgentSpawnMetadata::default()
                },
            )
            .await
            .expect("spawn child");
        session.conversation_id = child_id;
        let invocation = invocation(
            Arc::new(session),
            Arc::new(turn),
            "spawn_agent",
            function_payload(json!({"items": [{"type": "text", "text": "grandchild"}]})),
        );
        let Err(err) = CollabHandler.handle(invocation).await else {
            panic!("spawn depth limit should reject");
        };
        assert_eq!(
            err,
            FunctionCallError::RespondToModel(format!(
                "maximum spawn depth ({}) reached",
                crate::config::DEFAULT_COLLAB_MAX_SPAWN_DEPTH
            ))
        );
    }

    #[tokio::test]
    async fn spawn_agent_allows_when_spawn_depth_limit_is_raised() {
        let (mut session, mut turn) = make_session_and_context().await;
        overwrite_turn_config(&mut turn, |config| {
            config.max_spawn_depth = 2;
        });
        let manager = thread_manager();
        let control = manager.agent_control();
        session.services.agent_control = control.clone();
        let root_thread_id = session.conversation_id;
        let config = turn.config.as_ref().clone();
        let child_id = control
            .spawn_agent_with_metadata(
                config,
                "child".to_string(),
                AgentSpawnMetadata {
                    creator_thread_id: Some(root_thread_id),
                    allow_nested_agents: true,
                    ..AgentSpawnMetadata::default()
                },
            )
            .await
            .expect("spawn child");
        session.conversation_id = child_id;
        overwrite_turn_config(&mut turn, |config| {
            config.max_spawn_depth = 2;
        });

        let invocation = invocation(
            Arc::new(session),
            Arc::new(turn),
            "spawn_agent",
            function_payload(json!({"items": [{"type": "text", "text": "grandchild"}]})),
        );
        let output = CollabHandler
            .handle(invocation)
            .await
            .expect("spawn should succeed");
        let ToolOutput::Function {
            body: FunctionCallOutputBody::Text(content),
            success,
            ..
        } = output
        else {
            panic!("expected function output");
        };
        let value: serde_json::Value = serde_json::from_str(&content).expect("json result");
        let grandchild_id = value
            .get("agent_id")
            .and_then(serde_json::Value::as_str)
            .and_then(|id| ThreadId::from_string(id).ok())
            .expect("grandchild id should be present");
        assert_eq!(success, Some(true));

        let _ = control.shutdown_agent(grandchild_id).await;
        let _ = control.shutdown_agent(child_id).await;
    }

    #[tokio::test]
    async fn spawn_agent_rejects_when_active_subagent_limit_is_reached() {
        let (mut session, mut turn) = make_session_and_context().await;
        overwrite_turn_config(&mut turn, |config| {
            config.agent_max_threads =
                Some(crate::config::DEFAULT_COLLAB_MAX_ACTIVE_SUBAGENTS_PER_THREAD + 2);
        });
        let manager = thread_manager();
        let control = manager.agent_control();
        session.services.agent_control = control.clone();
        let root_thread_id = session.conversation_id;
        let config = turn.config.as_ref().clone();
        for i in 0..crate::config::DEFAULT_COLLAB_MAX_ACTIVE_SUBAGENTS_PER_THREAD {
            let _ = control
                .spawn_agent_with_metadata_and_source(
                    config.clone(),
                    AgentSpawnMetadata {
                        creator_thread_id: Some(root_thread_id),
                        label: Some(format!("child-{i}")),
                        ..AgentSpawnMetadata::default()
                    },
                    Vec::new(),
                    None,
                )
                .await
                .expect("spawn child");
        }
        let invocation = invocation(
            Arc::new(session),
            Arc::new(turn),
            "spawn_agent",
            function_payload(json!({"items": [{"type": "text", "text": "one more child"}]})),
        );
        let Err(err) = CollabHandler.handle(invocation).await else {
            panic!("active subagent limit should reject");
        };
        assert_eq!(
            err,
            FunctionCallError::RespondToModel(format!(
                "maximum active sub-agents per thread ({}) reached",
                crate::config::DEFAULT_COLLAB_MAX_ACTIVE_SUBAGENTS_PER_THREAD
            ))
        );
    }

    #[tokio::test]
    async fn spawn_agent_rejects_when_configured_active_subagent_limit_is_reached() {
        let (mut session, mut turn) = make_session_and_context().await;
        overwrite_turn_config(&mut turn, |config| {
            config.max_active_subagents_per_thread = 1;
        });
        let manager = thread_manager();
        let control = manager.agent_control();
        session.services.agent_control = control.clone();
        let root_thread_id = session.conversation_id;
        let config = turn.config.as_ref().clone();
        let _ = control
            .spawn_agent_with_metadata(
                config,
                "child-0".to_string(),
                AgentSpawnMetadata {
                    creator_thread_id: Some(root_thread_id),
                    ..AgentSpawnMetadata::default()
                },
            )
            .await
            .expect("spawn child");
        let invocation = invocation(
            Arc::new(session),
            Arc::new(turn),
            "spawn_agent",
            function_payload(json!({"items": [{"type": "text", "text": "one more child"}]})),
        );
        let Err(err) = CollabHandler.handle(invocation).await else {
            panic!("active subagent limit should reject");
        };
        assert_eq!(
            err,
            FunctionCallError::RespondToModel(
                "maximum active sub-agents per thread (1) reached".to_string()
            )
        );
    }

    #[tokio::test]
    async fn list_agents_filters_by_creator_status_and_closed() {
        let (mut session, turn) = make_session_and_context().await;
        let manager = thread_manager();
        let control = manager.agent_control();
        session.services.agent_control = control.clone();
        let config = turn.config.as_ref().clone();
        let creator_id = ThreadId::new();
        let metadata = AgentSpawnMetadata {
            creator_thread_id: Some(creator_id),
            label: Some("worker-a".to_string()),
            goal: "run checks".to_string(),
            acceptance_criteria: vec!["all tests pass".to_string()],
            test_commands: vec!["cargo test -p codex-core".to_string()],
            allow_nested_agents: false,
        };
        let agent_id = control
            .spawn_agent_with_metadata(config, "do work".to_string(), metadata)
            .await
            .expect("spawn agent");
        control
            .shutdown_agent(agent_id)
            .await
            .expect("shutdown should succeed");

        let invocation = invocation(
            Arc::new(session),
            Arc::new(turn),
            "list_agents",
            function_payload(json!({
                "agent_id": creator_id.to_string(),
                "statuses": ["shutdown"],
                "include_closed": true
            })),
        );
        let output = CollabHandler
            .handle(invocation)
            .await
            .expect("list_agents should succeed");
        let ToolOutput::Function {
            body: FunctionCallOutputBody::Text(content),
            success,
            ..
        } = output
        else {
            panic!("expected function output");
        };
        let value: serde_json::Value = serde_json::from_str(&content).expect("json result");
        let agents = value
            .get("agents")
            .and_then(serde_json::Value::as_array)
            .expect("agents array");
        assert_eq!(agents.len(), 1);
        assert_eq!(
            agents[0].get("agent_id"),
            Some(&json!(agent_id.to_string()))
        );
        assert_eq!(
            agents[0].get("creator_agent_id"),
            Some(&json!(creator_id.to_string()))
        );
        assert_eq!(agents[0].get("name"), Some(&json!("worker-a")));
        assert_eq!(agents[0].get("status"), Some(&json!("shutdown")));
        assert_eq!(agents[0].get("closed"), Some(&json!(true)));
        assert_eq!(success, Some(true));
    }

    #[tokio::test]
    async fn spawn_agent_name_is_visible_in_list_agents() {
        let (mut session, turn) = make_session_and_context().await;
        let manager = thread_manager();
        let control = manager.agent_control();
        session.services.agent_control = control.clone();
        let creator_id = session.conversation_id;
        let session = Arc::new(session);
        let turn = Arc::new(turn);

        let spawn_invocation = invocation(
            session.clone(),
            turn.clone(),
            "spawn_agent",
            function_payload(json!({
                "items": [{"type": "text", "text": "do work"}],
                "name": "worker-primary"
            })),
        );
        let spawn_output = CollabHandler
            .handle(spawn_invocation)
            .await
            .expect("spawn should succeed");
        let ToolOutput::Function {
            body: FunctionCallOutputBody::Text(spawn_content),
            success: spawn_success,
            ..
        } = spawn_output
        else {
            panic!("expected function output");
        };
        assert_eq!(spawn_success, Some(true));
        let spawn_value: serde_json::Value =
            serde_json::from_str(&spawn_content).expect("spawn result json");
        let agent_id = spawn_value
            .get("agent_id")
            .and_then(serde_json::Value::as_str)
            .and_then(|id| ThreadId::from_string(id).ok())
            .expect("agent id should be present");

        let list_invocation = invocation(
            session.clone(),
            turn.clone(),
            "list_agents",
            function_payload(json!({"agent_id": creator_id.to_string()})),
        );
        let list_output = CollabHandler
            .handle(list_invocation)
            .await
            .expect("list_agents should succeed");
        let ToolOutput::Function {
            body: FunctionCallOutputBody::Text(content),
            success,
            ..
        } = list_output
        else {
            panic!("expected function output");
        };
        assert_eq!(success, Some(true));
        let value: serde_json::Value = serde_json::from_str(&content).expect("json result");
        let agents = value
            .get("agents")
            .and_then(serde_json::Value::as_array)
            .expect("agents array");
        let agent = agents
            .iter()
            .find(|agent| agent.get("agent_id") == Some(&json!(agent_id.to_string())))
            .expect("spawned agent should be listed");
        assert_eq!(agent.get("name"), Some(&json!("worker-primary")));
        assert_eq!(agent.get("label"), None);

        let _ = control.shutdown_agent(agent_id).await;
    }

    #[tokio::test]
    async fn rename_agent_updates_name() {
        let (mut session, turn) = make_session_and_context().await;
        let manager = thread_manager();
        let control = manager.agent_control();
        session.services.agent_control = control.clone();
        let creator_id = session.conversation_id;
        let session = Arc::new(session);
        let turn = Arc::new(turn);

        let spawn_invocation = invocation(
            session.clone(),
            turn.clone(),
            "spawn_agent",
            function_payload(json!({
                "items": [{"type": "text", "text": "do work"}],
                "name": "worker-before"
            })),
        );
        let spawn_output = CollabHandler
            .handle(spawn_invocation)
            .await
            .expect("spawn should succeed");
        let ToolOutput::Function {
            body: FunctionCallOutputBody::Text(spawn_content),
            ..
        } = spawn_output
        else {
            panic!("expected function output");
        };
        let spawn_value: serde_json::Value =
            serde_json::from_str(&spawn_content).expect("spawn result json");
        let agent_id = spawn_value
            .get("agent_id")
            .and_then(serde_json::Value::as_str)
            .and_then(|id| ThreadId::from_string(id).ok())
            .expect("agent id should be present");

        let rename_invocation = invocation(
            session.clone(),
            turn.clone(),
            "rename_agent",
            function_payload(json!({
                "agent_id": agent_id.to_string(),
                "name": "worker-after"
            })),
        );
        let rename_output = CollabHandler
            .handle(rename_invocation)
            .await
            .expect("rename_agent should succeed");
        let ToolOutput::Function {
            body: FunctionCallOutputBody::Text(rename_content),
            success: rename_success,
            ..
        } = rename_output
        else {
            panic!("expected function output");
        };
        assert_eq!(rename_success, Some(true));
        let rename_value: serde_json::Value =
            serde_json::from_str(&rename_content).expect("rename result json");
        assert_eq!(
            rename_value.get("agent_id"),
            Some(&json!(agent_id.to_string()))
        );
        assert_eq!(rename_value.get("name"), Some(&json!("worker-after")));

        let list_invocation = invocation(
            session.clone(),
            turn.clone(),
            "list_agents",
            function_payload(json!({"agent_id": creator_id.to_string()})),
        );
        let list_output = CollabHandler
            .handle(list_invocation)
            .await
            .expect("list_agents should succeed");
        let ToolOutput::Function {
            body: FunctionCallOutputBody::Text(content),
            success,
            ..
        } = list_output
        else {
            panic!("expected function output");
        };
        assert_eq!(success, Some(true));
        let value: serde_json::Value = serde_json::from_str(&content).expect("json result");
        let agents = value
            .get("agents")
            .and_then(serde_json::Value::as_array)
            .expect("agents array");
        let agent = agents
            .iter()
            .find(|agent| agent.get("agent_id") == Some(&json!(agent_id.to_string())))
            .expect("renamed agent should be listed");
        assert_eq!(agent.get("name"), Some(&json!("worker-after")));
        assert_eq!(agent.get("label"), None);

        let _ = control.shutdown_agent(agent_id).await;
    }

    #[tokio::test]
    async fn list_agents_excludes_closed_by_default() {
        let (mut session, turn) = make_session_and_context().await;
        let manager = thread_manager();
        let control = manager.agent_control();
        session.services.agent_control = control.clone();
        let config = turn.config.as_ref().clone();
        let agent_id = control
            .spawn_agent_with_metadata(config, "do work".to_string(), AgentSpawnMetadata::default())
            .await
            .expect("spawn agent");
        control
            .shutdown_agent(agent_id)
            .await
            .expect("shutdown should succeed");
        let invocation = invocation(
            Arc::new(session),
            Arc::new(turn),
            "list_agents",
            function_payload(json!({})),
        );
        let output = CollabHandler
            .handle(invocation)
            .await
            .expect("list_agents should succeed");
        let ToolOutput::Function {
            body: FunctionCallOutputBody::Text(content),
            ..
        } = output
        else {
            panic!("expected function output");
        };
        let value: serde_json::Value = serde_json::from_str(&content).expect("json result");
        let agents = value
            .get("agents")
            .and_then(serde_json::Value::as_array)
            .expect("agents array");
        assert_eq!(agents.len(), 0);
    }

    #[tokio::test]
    async fn send_input_requires_items() {
        let (session, turn) = make_session_and_context().await;
        let invocation = invocation(
            Arc::new(session),
            Arc::new(turn),
            "send_input",
            function_payload(json!({"agent_id": ThreadId::new().to_string()})),
        );
        let Err(err) = CollabHandler.handle(invocation).await else {
            panic!("missing items should be rejected");
        };
        assert_eq!(
            err,
            FunctionCallError::RespondToModel("Provide required field: items".to_string())
        );
    }

    #[tokio::test]
    async fn send_input_rejects_empty_items() {
        let (session, turn) = make_session_and_context().await;
        let invocation = invocation(
            Arc::new(session),
            Arc::new(turn),
            "send_input",
            function_payload(json!({
                "agent_id": ThreadId::new().to_string(),
                "items": []
            })),
        );
        let Err(err) = CollabHandler.handle(invocation).await else {
            panic!("empty items should be rejected");
        };
        assert_eq!(
            err,
            FunctionCallError::RespondToModel("Items can't be empty".to_string())
        );
    }

    #[tokio::test]
    async fn send_input_rejects_invalid_id() {
        let (session, turn) = make_session_and_context().await;
        let invocation = invocation(
            Arc::new(session),
            Arc::new(turn),
            "send_input",
            function_payload(
                json!({"agent_id": "not-a-uuid", "items": [{"type": "text", "text": "hi"}]}),
            ),
        );
        let Err(err) = CollabHandler.handle(invocation).await else {
            panic!("invalid id should be rejected");
        };
        let FunctionCallError::RespondToModel(msg) = err else {
            panic!("expected respond-to-model error");
        };
        assert!(msg.starts_with("invalid agent id not-a-uuid:"));
    }

    #[tokio::test]
    async fn send_input_reports_missing_agent() {
        let (mut session, turn) = make_session_and_context().await;
        let manager = thread_manager();
        session.services.agent_control = manager.agent_control();
        let agent_id = ThreadId::new();
        let invocation = invocation(
            Arc::new(session),
            Arc::new(turn),
            "send_input",
            function_payload(
                json!({"agent_id": agent_id.to_string(), "items": [{"type": "text", "text": "hi"}]}),
            ),
        );
        let Err(err) = CollabHandler.handle(invocation).await else {
            panic!("missing agent should be reported");
        };
        assert_eq!(
            err,
            FunctionCallError::RespondToModel(format!("agent with id {agent_id} not found"))
        );
    }

    #[tokio::test]
    async fn send_input_interrupts_before_prompt() {
        let (mut session, turn) = make_session_and_context().await;
        let manager = thread_manager();
        session.services.agent_control = manager.agent_control();
        let config = turn.config.as_ref().clone();
        let thread = manager.start_thread(config).await.expect("start thread");
        let agent_id = thread.thread_id;
        let invocation = invocation(
            Arc::new(session),
            Arc::new(turn),
            "send_input",
            function_payload(json!({
                "agent_id": agent_id.to_string(),
                "items": [{"type": "text", "text": "hi"}],
                "interrupt": true
            })),
        );
        CollabHandler
            .handle(invocation)
            .await
            .expect("send_input should succeed");

        let ops = manager.captured_ops();
        let ops_for_agent: Vec<&Op> = ops
            .iter()
            .filter_map(|(id, op)| (*id == agent_id).then_some(op))
            .collect();
        assert_eq!(ops_for_agent.len(), 2);
        assert!(matches!(ops_for_agent[0], Op::Interrupt));
        assert!(matches!(ops_for_agent[1], Op::UserInput { .. }));

        let _ = thread
            .thread
            .submit(Op::Shutdown {})
            .await
            .expect("shutdown should submit");
    }

    #[tokio::test]
    async fn send_input_accepts_structured_items() {
        let (mut session, turn) = make_session_and_context().await;
        let manager = thread_manager();
        session.services.agent_control = manager.agent_control();
        let config = turn.config.as_ref().clone();
        let thread = manager.start_thread(config).await.expect("start thread");
        let agent_id = thread.thread_id;
        let invocation = invocation(
            Arc::new(session),
            Arc::new(turn),
            "send_input",
            function_payload(json!({
                "agent_id": agent_id.to_string(),
                "items": [
                    {"type": "mention", "name": "drive", "path": "app://google_drive"},
                    {"type": "text", "text": "read the folder"}
                ]
            })),
        );
        CollabHandler
            .handle(invocation)
            .await
            .expect("send_input should succeed");

        let expected = Op::UserInput {
            items: vec![
                UserInput::Mention {
                    name: "drive".to_string(),
                    path: "app://google_drive".to_string(),
                },
                UserInput::Text {
                    text: "read the folder".to_string(),
                    text_elements: Vec::new(),
                },
            ],
            final_output_json_schema: None,
        };
        let captured = manager
            .captured_ops()
            .into_iter()
            .find(|(id, op)| *id == agent_id && *op == expected);
        assert_eq!(captured, Some((agent_id, expected)));

        let _ = thread
            .thread
            .submit(Op::Shutdown {})
            .await
            .expect("shutdown should submit");
    }

    #[tokio::test]
    async fn resume_agent_rejects_invalid_id() {
        let (session, turn) = make_session_and_context().await;
        let invocation = invocation(
            Arc::new(session),
            Arc::new(turn),
            "resume_agent",
            function_payload(json!({"agent_id": "not-a-uuid"})),
        );
        let Err(err) = CollabHandler.handle(invocation).await else {
            panic!("invalid id should be rejected");
        };
        let FunctionCallError::RespondToModel(msg) = err else {
            panic!("expected respond-to-model error");
        };
        assert!(msg.starts_with("invalid agent id not-a-uuid:"));
    }

    #[tokio::test]
    async fn resume_agent_reports_missing_agent() {
        let (mut session, turn) = make_session_and_context().await;
        let manager = thread_manager();
        session.services.agent_control = manager.agent_control();
        let agent_id = ThreadId::new();
        let invocation = invocation(
            Arc::new(session),
            Arc::new(turn),
            "resume_agent",
            function_payload(json!({"agent_id": agent_id.to_string()})),
        );
        let Err(err) = CollabHandler.handle(invocation).await else {
            panic!("missing agent should be reported");
        };
        assert_eq!(
            err,
            FunctionCallError::RespondToModel(format!("agent with id {agent_id} not found"))
        );
    }

    #[tokio::test]
    async fn resume_agent_noops_for_active_agent() {
        let (mut session, turn) = make_session_and_context().await;
        let manager = thread_manager();
        session.services.agent_control = manager.agent_control();
        let config = turn.config.as_ref().clone();
        let thread = manager.start_thread(config).await.expect("start thread");
        let agent_id = thread.thread_id;
        let status_before = manager.agent_control().get_status(agent_id).await;
        let invocation = invocation(
            Arc::new(session),
            Arc::new(turn),
            "resume_agent",
            function_payload(json!({"agent_id": agent_id.to_string()})),
        );

        let output = CollabHandler
            .handle(invocation)
            .await
            .expect("resume_agent should succeed");
        let ToolOutput::Function {
            body: FunctionCallOutputBody::Text(content),
            success,
            ..
        } = output
        else {
            panic!("expected function output");
        };
        let result: resume_agent::ResumeAgentResult =
            serde_json::from_str(&content).expect("resume_agent result should be json");
        assert_eq!(result.status, status_before);
        assert_eq!(success, Some(true));

        let thread_ids = manager.list_thread_ids().await;
        assert_eq!(thread_ids, vec![agent_id]);

        let _ = thread
            .thread
            .submit(Op::Shutdown {})
            .await
            .expect("shutdown should submit");
    }

    #[tokio::test]
    async fn resume_agent_restores_closed_agent_and_accepts_send_input() {
        let (mut session, turn) = make_session_and_context().await;
        let manager = thread_manager();
        session.services.agent_control = manager.agent_control();
        let config = turn.config.as_ref().clone();
        let thread = manager
            .resume_thread_with_history(
                config,
                InitialHistory::Forked(vec![RolloutItem::ResponseItem(ResponseItem::Message {
                    id: None,
                    role: "user".to_string(),
                    content: vec![ContentItem::InputText {
                        text: "materialized".to_string(),
                    }],
                    end_turn: None,
                    phase: None,
                })]),
                AuthManager::from_auth_for_testing(CodexAuth::from_api_key("dummy")),
            )
            .await
            .expect("start thread");
        let agent_id = thread.thread_id;
        let _ = manager
            .agent_control()
            .shutdown_agent(agent_id)
            .await
            .expect("shutdown agent");
        assert_eq!(
            manager.agent_control().get_status(agent_id).await,
            AgentStatus::NotFound
        );
        let session = Arc::new(session);
        let turn = Arc::new(turn);

        let resume_invocation = invocation(
            session.clone(),
            turn.clone(),
            "resume_agent",
            function_payload(json!({"agent_id": agent_id.to_string()})),
        );
        let output = CollabHandler
            .handle(resume_invocation)
            .await
            .expect("resume_agent should succeed");
        let ToolOutput::Function {
            body: FunctionCallOutputBody::Text(content),
            success,
            ..
        } = output
        else {
            panic!("expected function output");
        };
        let result: resume_agent::ResumeAgentResult =
            serde_json::from_str(&content).expect("resume_agent result should be json");
        assert_ne!(result.status, AgentStatus::NotFound);
        assert_eq!(success, Some(true));

        let send_invocation = invocation(
            session,
            turn,
            "send_input",
            function_payload(
                json!({"agent_id": agent_id.to_string(), "items": [{"type": "text", "text": "hello"}]}),
            ),
        );
        let output = CollabHandler
            .handle(send_invocation)
            .await
            .expect("send_input should succeed after resume");
        let ToolOutput::Function {
            body: FunctionCallOutputBody::Text(content),
            success,
            ..
        } = output
        else {
            panic!("expected function output");
        };
        let result: serde_json::Value =
            serde_json::from_str(&content).expect("send_input result should be json");
        let submission_id = result
            .get("submission_id")
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        assert!(!submission_id.is_empty());
        assert_eq!(success, Some(true));

        let _ = manager
            .agent_control()
            .shutdown_agent(agent_id)
            .await
            .expect("shutdown resumed agent");
    }

    #[tokio::test]
    async fn resume_agent_rejects_when_depth_limit_exceeded() {
        let (mut session, mut turn) = make_session_and_context().await;
        let manager = thread_manager();
        session.services.agent_control = manager.agent_control();

        turn.session_source = SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
            parent_thread_id: session.conversation_id,
            depth: MAX_THREAD_SPAWN_DEPTH,
        });

        let invocation = invocation(
            Arc::new(session),
            Arc::new(turn),
            "resume_agent",
            function_payload(json!({"agent_id": ThreadId::new().to_string()})),
        );
        let Err(err) = CollabHandler.handle(invocation).await else {
            panic!("resume should fail when depth limit exceeded");
        };
        assert_eq!(
            err,
            FunctionCallError::RespondToModel(
                "Agent depth limit reached. Solve the task yourself.".to_string()
            )
        );
    }

    #[derive(Debug, Deserialize, PartialEq, Eq)]
    struct WaitResult {
        status: HashMap<ThreadId, AgentStatus>,
        timed_out: bool,
        wakeup_reason: WaitWakeupReason,
    }

    #[tokio::test]
    async fn wait_rejects_negative_timeout() {
        let (session, turn) = make_session_and_context().await;
        let invocation = invocation(
            Arc::new(session),
            Arc::new(turn),
            "wait",
            function_payload(json!({
                "agent_ids": [ThreadId::new().to_string()],
                "timeout_ms": -1
            })),
        );
        let Err(err) = CollabHandler.handle(invocation).await else {
            panic!("negative timeout should be rejected");
        };
        assert_eq!(
            err,
            FunctionCallError::RespondToModel(
                "timeout_ms must be greater than or equal to zero".to_string()
            )
        );
    }

    #[tokio::test]
    async fn wait_timeout_zero_returns_non_blocking_snapshot() {
        let (mut session, turn) = make_session_and_context().await;
        let manager = thread_manager();
        session.services.agent_control = manager.agent_control();
        let config = turn.config.as_ref().clone();
        let thread = manager.start_thread(config).await.expect("start thread");
        let agent_id = thread.thread_id;
        let invocation = invocation(
            Arc::new(session),
            Arc::new(turn),
            "wait",
            function_payload(json!({
                "agent_ids": [agent_id.to_string()],
                "timeout_ms": 0
            })),
        );
        let output = timeout(Duration::from_millis(50), CollabHandler.handle(invocation))
            .await
            .expect("wait should be non-blocking")
            .expect("wait should succeed");
        let ToolOutput::Function {
            body: FunctionCallOutputBody::Text(content),
            success,
            ..
        } = output
        else {
            panic!("expected function output");
        };
        let result: WaitResult =
            serde_json::from_str(&content).expect("wait result should be json");
        assert_eq!(
            result,
            WaitResult {
                status: HashMap::new(),
                timed_out: true,
                wakeup_reason: WaitWakeupReason::Timeout,
            }
        );
        assert_eq!(success, None);

        let _ = thread
            .thread
            .submit(Op::Shutdown {})
            .await
            .expect("shutdown should submit");
    }

    #[tokio::test]
    async fn wait_rejects_invalid_id() {
        let (session, turn) = make_session_and_context().await;
        let invocation = invocation(
            Arc::new(session),
            Arc::new(turn),
            "wait",
            function_payload(json!({"agent_ids": ["invalid"]})),
        );
        let Err(err) = CollabHandler.handle(invocation).await else {
            panic!("invalid id should be rejected");
        };
        let FunctionCallError::RespondToModel(msg) = err else {
            panic!("expected respond-to-model error");
        };
        assert!(msg.starts_with("invalid agent id invalid:"));
    }

    #[tokio::test]
    async fn wait_rejects_empty_ids() {
        let (session, turn) = make_session_and_context().await;
        let invocation = invocation(
            Arc::new(session),
            Arc::new(turn),
            "wait",
            function_payload(json!({"agent_ids": []})),
        );
        let Err(err) = CollabHandler.handle(invocation).await else {
            panic!("empty ids should be rejected");
        };
        assert_eq!(
            err,
            FunctionCallError::RespondToModel("agent_ids must be non-empty".to_string())
        );
    }

    #[tokio::test]
    async fn wait_returns_not_found_for_missing_agents() {
        let (mut session, turn) = make_session_and_context().await;
        let manager = thread_manager();
        session.services.agent_control = manager.agent_control();
        let id_a = ThreadId::new();
        let id_b = ThreadId::new();
        let invocation = invocation(
            Arc::new(session),
            Arc::new(turn),
            "wait",
            function_payload(json!({
                "agent_ids": [id_a.to_string(), id_b.to_string()],
                "timeout_ms": 1000
            })),
        );
        let output = CollabHandler
            .handle(invocation)
            .await
            .expect("wait should succeed");
        let ToolOutput::Function {
            body: FunctionCallOutputBody::Text(content),
            success,
            ..
        } = output
        else {
            panic!("expected function output");
        };
        let result: WaitResult =
            serde_json::from_str(&content).expect("wait result should be json");
        assert_eq!(
            result,
            WaitResult {
                status: HashMap::from([
                    (id_a, AgentStatus::NotFound),
                    (id_b, AgentStatus::NotFound),
                ]),
                timed_out: false,
                wakeup_reason: WaitWakeupReason::AnyCompleted,
            }
        );
        assert_eq!(success, None);
    }

    #[tokio::test]
    async fn wait_times_out_when_status_is_not_final() {
        let (mut session, turn) = make_session_and_context().await;
        let manager = thread_manager();
        session.services.agent_control = manager.agent_control();
        let config = turn.config.as_ref().clone();
        let thread = manager.start_thread(config).await.expect("start thread");
        let agent_id = thread.thread_id;
        let invocation = invocation(
            Arc::new(session),
            Arc::new(turn),
            "wait",
            function_payload(json!({
                "agent_ids": [agent_id.to_string()],
                "timeout_ms": MIN_WAIT_TIMEOUT_MS
            })),
        );
        let output = CollabHandler
            .handle(invocation)
            .await
            .expect("wait should succeed");
        let ToolOutput::Function {
            body: FunctionCallOutputBody::Text(content),
            success,
            ..
        } = output
        else {
            panic!("expected function output");
        };
        let result: WaitResult =
            serde_json::from_str(&content).expect("wait result should be json");
        assert_eq!(
            result,
            WaitResult {
                status: HashMap::new(),
                timed_out: true,
                wakeup_reason: WaitWakeupReason::Timeout,
            }
        );
        assert_eq!(success, None);

        let _ = thread
            .thread
            .submit(Op::Shutdown {})
            .await
            .expect("shutdown should submit");
    }

    #[tokio::test]
    async fn wait_clamps_short_timeouts_to_minimum() {
        let (mut session, turn) = make_session_and_context().await;
        let manager = thread_manager();
        session.services.agent_control = manager.agent_control();
        let config = turn.config.as_ref().clone();
        let thread = manager.start_thread(config).await.expect("start thread");
        let agent_id = thread.thread_id;
        let invocation = invocation(
            Arc::new(session),
            Arc::new(turn),
            "wait",
            function_payload(json!({
                "agent_ids": [agent_id.to_string()],
                "timeout_ms": 10
            })),
        );

        let early = timeout(Duration::from_millis(50), CollabHandler.handle(invocation)).await;
        assert!(
            early.is_err(),
            "wait should not return before the minimum timeout clamp"
        );

        let _ = thread
            .thread
            .submit(Op::Shutdown {})
            .await
            .expect("shutdown should submit");
    }

    #[tokio::test]
    async fn wait_returns_final_status_without_timeout() {
        let (mut session, turn) = make_session_and_context().await;
        let manager = thread_manager();
        session.services.agent_control = manager.agent_control();
        let config = turn.config.as_ref().clone();
        let thread = manager.start_thread(config).await.expect("start thread");
        let agent_id = thread.thread_id;
        let mut status_rx = manager
            .agent_control()
            .subscribe_status(agent_id)
            .await
            .expect("subscribe should succeed");

        let _ = thread
            .thread
            .submit(Op::Shutdown {})
            .await
            .expect("shutdown should submit");
        let _ = timeout(Duration::from_secs(1), status_rx.changed())
            .await
            .expect("shutdown status should arrive");

        let invocation = invocation(
            Arc::new(session),
            Arc::new(turn),
            "wait",
            function_payload(json!({
                "agent_ids": [agent_id.to_string()],
                "timeout_ms": 1000
            })),
        );
        let output = CollabHandler
            .handle(invocation)
            .await
            .expect("wait should succeed");
        let ToolOutput::Function {
            body: FunctionCallOutputBody::Text(content),
            success,
            ..
        } = output
        else {
            panic!("expected function output");
        };
        let result: WaitResult =
            serde_json::from_str(&content).expect("wait result should be json");
        assert_eq!(
            result,
            WaitResult {
                status: HashMap::from([(agent_id, AgentStatus::Shutdown)]),
                timed_out: false,
                wakeup_reason: WaitWakeupReason::AnyCompleted,
            }
        );
        assert_eq!(success, None);
    }

    #[tokio::test]
    async fn wait_uses_configured_default_timeout_when_timeout_is_omitted() {
        let (mut session, mut turn) = make_session_and_context().await;
        overwrite_turn_config(&mut turn, |config| {
            config.default_wait_timeout_ms = 10;
        });
        let manager = thread_manager();
        session.services.agent_control = manager.agent_control();
        let config = turn.config.as_ref().clone();
        let thread = manager.start_thread(config).await.expect("start thread");
        let agent_id = thread.thread_id;
        let invocation = invocation(
            Arc::new(session),
            Arc::new(turn),
            "wait",
            function_payload(json!({"agent_id": agent_id.to_string()})),
        );
        let output = timeout(Duration::from_secs(1), CollabHandler.handle(invocation))
            .await
            .expect("wait should return using configured timeout")
            .expect("wait should succeed");
        let ToolOutput::Function {
            body: FunctionCallOutputBody::Text(content),
            ..
        } = output
        else {
            panic!("expected function output");
        };
        let value: serde_json::Value = serde_json::from_str(&content).expect("json result");
        assert_eq!(value.get("timed_out"), Some(&json!(true)));
        assert_eq!(value.get("wakeup_reason"), Some(&json!("timeout")));

        let _ = thread
            .thread
            .submit(Op::Shutdown {})
            .await
            .expect("shutdown should submit");
    }

    #[tokio::test]
    async fn wait_agents_any_returns_when_first_agent_finishes() {
        let (mut session, turn) = make_session_and_context().await;
        let manager = thread_manager();
        session.services.agent_control = manager.agent_control();
        let config = turn.config.as_ref().clone();
        let thread1 = manager
            .start_thread(config.clone())
            .await
            .expect("start thread 1");
        let thread2 = manager.start_thread(config).await.expect("start thread 2");
        let agent1 = thread1.thread_id;
        let agent2 = thread2.thread_id;
        let mut status_rx = manager
            .agent_control()
            .subscribe_status(agent1)
            .await
            .expect("subscribe should succeed");

        let _ = thread1
            .thread
            .submit(Op::Shutdown {})
            .await
            .expect("shutdown should submit");
        let _ = timeout(Duration::from_secs(1), status_rx.changed())
            .await
            .expect("shutdown status should arrive");

        let invocation = invocation(
            Arc::new(session),
            Arc::new(turn),
            "wait_agents",
            function_payload(
                json!({"agent_ids": [agent1.to_string(), agent2.to_string()], "mode": "any", "timeout_ms": 1000}),
            ),
        );
        let output = CollabHandler
            .handle(invocation)
            .await
            .expect("wait_agents should succeed");
        let ToolOutput::Function {
            body: FunctionCallOutputBody::Text(content),
            success,
            ..
        } = output
        else {
            panic!("expected function output");
        };
        let value: serde_json::Value = serde_json::from_str(&content).expect("json result");
        let completed_agent_ids = value
            .get("completed_agent_ids")
            .and_then(serde_json::Value::as_array)
            .expect("completed_agent_ids array");
        assert_eq!(value.get("timed_out"), Some(&json!(false)));
        assert_eq!(value.get("wakeup_reason"), Some(&json!("any_completed")));
        assert!(completed_agent_ids.contains(&json!(agent1.to_string())));
        assert_eq!(success, Some(true));
    }

    #[tokio::test]
    async fn wait_agents_all_waits_for_every_agent() {
        let (mut session, turn) = make_session_and_context().await;
        let manager = thread_manager();
        session.services.agent_control = manager.agent_control();
        let config = turn.config.as_ref().clone();
        let thread1 = manager
            .start_thread(config.clone())
            .await
            .expect("start thread 1");
        let thread2 = manager.start_thread(config).await.expect("start thread 2");
        let agent1 = thread1.thread_id;
        let agent2 = thread2.thread_id;
        let mut status_rx1 = manager
            .agent_control()
            .subscribe_status(agent1)
            .await
            .expect("subscribe should succeed");
        let thread2_task = thread2.thread.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(80)).await;
            let _ = thread2_task.submit(Op::Shutdown {}).await;
        });

        let _ = thread1
            .thread
            .submit(Op::Shutdown {})
            .await
            .expect("shutdown should submit");
        let _ = timeout(Duration::from_secs(1), status_rx1.changed())
            .await
            .expect("shutdown status should arrive");
        let started_at = tokio::time::Instant::now();

        let invocation = invocation(
            Arc::new(session),
            Arc::new(turn),
            "wait_agents",
            function_payload(
                json!({"agent_ids": [agent1.to_string(), agent2.to_string()], "mode": "all", "timeout_ms": 2000}),
            ),
        );
        let output = CollabHandler
            .handle(invocation)
            .await
            .expect("wait_agents should succeed");
        let elapsed = started_at.elapsed();
        let ToolOutput::Function {
            body: FunctionCallOutputBody::Text(content),
            success,
            ..
        } = output
        else {
            panic!("expected function output");
        };
        let value: serde_json::Value = serde_json::from_str(&content).expect("json result");
        let completed_agent_ids = value
            .get("completed_agent_ids")
            .and_then(serde_json::Value::as_array)
            .expect("completed_agent_ids array");
        assert_eq!(value.get("timed_out"), Some(&json!(false)));
        assert_eq!(value.get("wakeup_reason"), Some(&json!("all_completed")));
        assert!(completed_agent_ids.contains(&json!(agent1.to_string())));
        assert!(completed_agent_ids.contains(&json!(agent2.to_string())));
        assert!(elapsed >= Duration::from_millis(50));
        assert_eq!(success, Some(true));
    }

    #[tokio::test]
    async fn wait_agents_times_out_when_targets_are_not_final() {
        let (mut session, turn) = make_session_and_context().await;
        let manager = thread_manager();
        session.services.agent_control = manager.agent_control();
        let config = turn.config.as_ref().clone();
        let thread = manager.start_thread(config).await.expect("start thread");
        let agent_id = thread.thread_id;

        let invocation = invocation(
            Arc::new(session),
            Arc::new(turn),
            "wait_agents",
            function_payload(
                json!({"agent_ids": [agent_id.to_string()], "mode": "any", "timeout_ms": 0}),
            ),
        );
        let output = timeout(Duration::from_millis(50), CollabHandler.handle(invocation))
            .await
            .expect("wait_agents should be non-blocking")
            .expect("wait_agents should succeed");
        let ToolOutput::Function {
            body: FunctionCallOutputBody::Text(content),
            success,
            ..
        } = output
        else {
            panic!("expected function output");
        };
        let value: serde_json::Value = serde_json::from_str(&content).expect("json result");
        assert_eq!(value.get("timed_out"), Some(&json!(true)));
        assert_eq!(value.get("wakeup_reason"), Some(&json!("timeout")));
        assert_eq!(success, Some(false));

        let _ = thread
            .thread
            .submit(Op::Shutdown {})
            .await
            .expect("shutdown should submit");
    }

    #[tokio::test]
    async fn wait_agents_without_ids_waits_only_on_active_children() {
        let (mut session, turn) = make_session_and_context().await;
        let manager = thread_manager();
        session.services.agent_control = manager.agent_control();
        let config = turn.config.as_ref().clone();
        let control = manager.agent_control();
        let closed_agent_id = control
            .spawn_agent_with_metadata(
                config.clone(),
                "closed child".to_string(),
                AgentSpawnMetadata {
                    creator_thread_id: Some(session.conversation_id),
                    goal: "closed child".to_string(),
                    ..Default::default()
                },
            )
            .await
            .expect("spawn closed child");
        let running_agent_id = control
            .spawn_agent_with_metadata(
                config.clone(),
                "running child".to_string(),
                AgentSpawnMetadata {
                    creator_thread_id: Some(session.conversation_id),
                    goal: "running child".to_string(),
                    ..Default::default()
                },
            )
            .await
            .expect("spawn running child");
        let _unrelated_thread = manager
            .start_thread(config)
            .await
            .expect("start unrelated running thread");

        let mut closed_status_rx = control
            .subscribe_status(closed_agent_id)
            .await
            .expect("subscribe should succeed");
        let _ = control
            .shutdown_agent(closed_agent_id)
            .await
            .expect("shutdown should submit");
        let _ = timeout(Duration::from_secs(1), closed_status_rx.changed()).await;

        let invocation = invocation(
            Arc::new(session),
            Arc::new(turn),
            "wait_agents",
            function_payload(json!({"mode": "any", "timeout_ms": 20})),
        );
        let output = CollabHandler
            .handle(invocation)
            .await
            .expect("wait_agents should succeed");
        let ToolOutput::Function {
            body: FunctionCallOutputBody::Text(content),
            success,
            ..
        } = output
        else {
            panic!("expected function output");
        };
        let value: serde_json::Value = serde_json::from_str(&content).expect("json result");
        let statuses = value
            .get("statuses")
            .and_then(serde_json::Value::as_array)
            .expect("statuses array");
        assert_eq!(value.get("timed_out"), Some(&json!(true)));
        assert_eq!(statuses.len(), 1);
        assert_eq!(
            statuses[0].get("agent_id"),
            Some(&serde_json::Value::String(running_agent_id.to_string()))
        );
        assert_eq!(success, Some(false));
        assert_eq!(value.get("wakeup_reason"), Some(&json!("timeout")));

        let _ = control.shutdown_agent(running_agent_id).await;
    }

    #[tokio::test]
    async fn close_agent_submits_shutdown_and_returns_status() {
        let (mut session, turn, rx) = make_session_and_context_with_rx().await;
        let manager = thread_manager();
        Arc::get_mut(&mut session)
            .expect("session should not be shared")
            .services
            .agent_control = manager.agent_control();
        let config = turn.config.as_ref().clone();
        let thread = manager.start_thread(config).await.expect("start thread");
        let agent_id = thread.thread_id;

        let invocation = invocation(
            session,
            turn,
            "close_agent",
            function_payload(json!({"agent_id": agent_id.to_string()})),
        );
        let output = CollabHandler
            .handle(invocation)
            .await
            .expect("close_agent should succeed");
        let ToolOutput::Function {
            body: FunctionCallOutputBody::Text(content),
            success,
            ..
        } = output
        else {
            panic!("expected function output");
        };
        let result: close_agent::CloseAgentResult =
            serde_json::from_str(&content).expect("close_agent result should be json");
        assert_eq!(result.status, AgentStatus::Shutdown);
        assert_eq!(success, Some(true));
        assert_eq!(recv_close_end_status(&rx).await, AgentStatus::Shutdown);

        let ops = manager.captured_ops();
        let submitted_shutdown = ops
            .iter()
            .any(|(id, op)| *id == agent_id && matches!(op, Op::Shutdown));
        assert_eq!(submitted_shutdown, true);

        let status_after = manager.agent_control().get_status(agent_id).await;
        assert_eq!(status_after, AgentStatus::NotFound);
    }

    #[tokio::test]
    async fn close_agent_reports_not_found_error_with_post_close_status() {
        let (mut session, turn, rx) = make_session_and_context_with_rx().await;
        let manager = thread_manager();
        Arc::get_mut(&mut session)
            .expect("session should not be shared")
            .services
            .agent_control = manager.agent_control();
        let missing_id = ThreadId::new();

        let invocation = invocation(
            session,
            turn,
            "close_agent",
            function_payload(json!({"agent_id": missing_id.to_string()})),
        );
        let Err(err) = CollabHandler.handle(invocation).await else {
            panic!("close_agent should report missing agents");
        };
        let FunctionCallError::RespondToModel(message) = err else {
            panic!("expected respond-to-model error");
        };
        assert_eq!(message, format!("agent with id {missing_id} not found"));
        assert_eq!(recv_close_end_status(&rx).await, AgentStatus::NotFound);
    }

    #[tokio::test]
    async fn close_agent_releases_spawn_slot_for_follow_up_spawn() {
        let (mut session, mut turn) = make_session_and_context().await;
        overwrite_turn_config(&mut turn, |config| {
            config.agent_max_threads = Some(1);
        });
        let manager = thread_manager();
        session.services.agent_control = manager.agent_control();
        let session = Arc::new(session);
        let turn = Arc::new(turn);

        let first_spawn = invocation(
            session.clone(),
            turn.clone(),
            "spawn_agent",
            function_payload(json!({
                "items": [{"type": "text", "text": "first worker"}]
            })),
        );
        let first_output = CollabHandler
            .handle(first_spawn)
            .await
            .expect("first spawn should succeed");
        let ToolOutput::Function {
            body: FunctionCallOutputBody::Text(first_body),
            success: first_success,
            ..
        } = first_output
        else {
            panic!("expected function output");
        };
        assert_eq!(first_success, Some(true));
        let first_value: serde_json::Value =
            serde_json::from_str(&first_body).expect("first spawn result should be json");
        let first_agent_id = first_value
            .get("agent_id")
            .and_then(serde_json::Value::as_str)
            .and_then(|id| ThreadId::from_string(id).ok())
            .expect("first agent id should be present");

        let close_invocation = invocation(
            session.clone(),
            turn.clone(),
            "close_agent",
            function_payload(json!({ "agent_id": first_agent_id.to_string() })),
        );
        let close_output = CollabHandler
            .handle(close_invocation)
            .await
            .expect("close_agent should succeed");
        let ToolOutput::Function {
            success: close_success,
            ..
        } = close_output
        else {
            panic!("expected function output");
        };
        assert_eq!(close_success, Some(true));

        let second_spawn = invocation(
            session,
            turn,
            "spawn_agent",
            function_payload(json!({
                "items": [{"type": "text", "text": "second worker"}]
            })),
        );
        let second_output = CollabHandler
            .handle(second_spawn)
            .await
            .expect("second spawn should succeed after close");
        let ToolOutput::Function {
            success: second_success,
            ..
        } = second_output
        else {
            panic!("expected function output");
        };
        assert_eq!(second_success, Some(true));
    }

    #[tokio::test]
    async fn close_agent_does_not_auto_close_descendants_when_disabled_in_config() {
        let (mut session, mut turn) = make_session_and_context().await;
        overwrite_turn_config(&mut turn, |config| {
            config.auto_close_on_parent_shutdown = false;
        });
        let manager = thread_manager();
        let control = manager.agent_control();
        session.services.agent_control = control.clone();
        let config = turn.config.as_ref().clone();
        let parent_id = control
            .spawn_agent_with_metadata(
                config.clone(),
                "parent".to_string(),
                AgentSpawnMetadata {
                    creator_thread_id: Some(session.conversation_id),
                    allow_nested_agents: true,
                    ..Default::default()
                },
            )
            .await
            .expect("spawn parent");
        let child_id = control
            .spawn_agent_with_metadata(
                config,
                "child".to_string(),
                AgentSpawnMetadata {
                    creator_thread_id: Some(parent_id),
                    ..Default::default()
                },
            )
            .await
            .expect("spawn child");

        let invocation = invocation(
            Arc::new(session),
            Arc::new(turn),
            "close_agent",
            function_payload(json!({"agent_id": parent_id.to_string()})),
        );
        let output = CollabHandler
            .handle(invocation)
            .await
            .expect("close_agent should succeed");
        let ToolOutput::Function { success, .. } = output else {
            panic!("expected function output");
        };
        assert_eq!(success, Some(true));
        assert_eq!(control.get_status(parent_id).await, AgentStatus::NotFound);
        assert_ne!(control.get_status(child_id).await, AgentStatus::NotFound);

        let child_record = control
            .get_agent_record(child_id)
            .await
            .expect("load child record")
            .expect("child record should exist");
        assert!(!child_record.closed);

        let _ = control.shutdown_agent(child_id).await;
    }

    #[tokio::test]
    async fn close_agents_handles_missing_and_duplicates() {
        let (mut session, turn) = make_session_and_context().await;
        let manager = thread_manager();
        session.services.agent_control = manager.agent_control();
        let config = turn.config.as_ref().clone();
        let thread = manager.start_thread(config).await.expect("start thread");
        let agent_id = thread.thread_id;
        let missing_id = ThreadId::new();

        let invocation = invocation(
            Arc::new(session),
            Arc::new(turn),
            "close_agents",
            function_payload(json!({
                "agent_ids": [agent_id.to_string(), agent_id.to_string(), missing_id.to_string()],
                "ignore_missing": true
            })),
        );
        let output = CollabHandler
            .handle(invocation)
            .await
            .expect("close_agents should succeed");
        let ToolOutput::Function {
            body: FunctionCallOutputBody::Text(content),
            success,
            ..
        } = output
        else {
            panic!("expected function output");
        };
        let value: serde_json::Value = serde_json::from_str(&content).expect("json result");
        let results = value
            .get("results")
            .and_then(serde_json::Value::as_array)
            .expect("results array");
        assert_eq!(results.len(), 2);
        let result_agent_ids = results
            .iter()
            .map(|result| result.get("agent_id").cloned())
            .collect::<Vec<_>>();
        assert!(result_agent_ids.contains(&Some(json!(agent_id.to_string()))));
        assert!(result_agent_ids.contains(&Some(json!(missing_id.to_string()))));
        assert_eq!(success, Some(true));

        let shutdown_ops = manager
            .captured_ops()
            .into_iter()
            .filter(|(id, op)| *id == agent_id && matches!(op, Op::Shutdown))
            .count();
        assert_eq!(shutdown_ops, 1);
    }

    #[tokio::test]
    async fn close_agents_reports_missing_when_not_ignored() {
        let (session, turn) = make_session_and_context().await;
        let invocation = invocation(
            Arc::new(session),
            Arc::new(turn),
            "close_agents",
            function_payload(json!({
                "agent_ids": [ThreadId::new().to_string()],
                "ignore_missing": false
            })),
        );
        let output = CollabHandler
            .handle(invocation)
            .await
            .expect("close_agents should succeed");
        let ToolOutput::Function {
            body: FunctionCallOutputBody::Text(content),
            success,
            ..
        } = output
        else {
            panic!("expected function output");
        };
        let value: serde_json::Value = serde_json::from_str(&content).expect("json result");
        let results = value
            .get("results")
            .and_then(serde_json::Value::as_array)
            .expect("results array");
        assert_eq!(results.len(), 1);
        assert!(results[0].get("agent_id").is_some());
        assert!(
            results[0]
                .get("error")
                .and_then(serde_json::Value::as_str)
                .is_some()
        );
        assert_eq!(success, Some(false));
    }

    #[tokio::test]
    async fn build_agent_spawn_config_uses_turn_context_values() {
        let (session, mut turn) = make_session_and_context().await;
        turn.developer_instructions = Some("dev".to_string());
        turn.compact_prompt = Some("compact".to_string());
        turn.shell_environment_policy = ShellEnvironmentPolicy {
            use_profile: true,
            ..ShellEnvironmentPolicy::default()
        };
        let temp_dir = tempfile::tempdir().expect("temp dir");
        turn.cwd = temp_dir.path().to_path_buf();
        turn.codex_linux_sandbox_exe = Some(PathBuf::from("/bin/echo"));

        let config = build_agent_spawn_config(&session, &turn, &SpawnConfigOverrides::default())
            .await
            .expect("spawn config");
        let mut expected = (*turn.config).clone();
        expected.base_instructions = Some(session.get_base_instructions().await.text);
        expected.model = Some(turn.model_info.slug.clone());
        expected.model_provider = turn.provider.clone();
        expected.model_reasoning_effort = turn.reasoning_effort;
        expected.model_reasoning_summary = turn.reasoning_summary;
        expected.developer_instructions = turn.developer_instructions.clone();
        expected.compact_prompt = turn.compact_prompt.clone();
        expected.shell_environment_policy = turn.shell_environment_policy.clone();
        expected.codex_linux_sandbox_exe = turn.codex_linux_sandbox_exe.clone();
        expected.cwd = turn.cwd.clone();
        expected
            .approval_policy
            .set(turn.approval_policy)
            .expect("approval policy set");
        expected
            .sandbox_policy
            .set(turn.sandbox_policy)
            .expect("sandbox policy set");
        assert_eq!(config, expected);
    }

    #[tokio::test]
    async fn build_agent_spawn_config_applies_model_reasoning_overrides() {
        let (session, turn) = make_session_and_context().await;
        let current_model = turn.model_info.slug.clone();
        let available_models = session
            .services
            .models_manager
            .list_models(turn.config.as_ref(), RefreshStrategy::Offline)
            .await;
        let model = available_models
            .iter()
            .map(|preset| preset.model.clone())
            .find(|candidate| candidate != &current_model)
            .unwrap_or(current_model.clone());
        let model_info = session
            .services
            .models_manager
            .get_model_info(&model, turn.config.as_ref())
            .await;
        let effort = model_info
            .supported_reasoning_levels
            .first()
            .map(|preset| preset.effort)
            .unwrap_or(ReasoningEffort::Medium);

        let overrides = SpawnConfigOverrides {
            preset: None,
            model: Some(model.clone()),
            reasoning_effort: Some(effort),
            reasoning_summary: Some(ReasoningSummaryConfig::Detailed),
            approval_policy: None,
            sandbox_mode: None,
        };
        let config = build_agent_spawn_config(&session, &turn, &overrides)
            .await
            .expect("spawn config");

        assert_eq!(config.model, Some(model));
        assert_eq!(config.model_reasoning_effort, Some(effort));
        assert_eq!(
            config.model_reasoning_summary,
            ReasoningSummaryConfig::Detailed
        );
    }

    #[tokio::test]
    async fn build_agent_spawn_config_uses_model_override_precedence() {
        let (session, mut turn) = make_session_and_context().await;
        let default_model = turn.model_info.slug.clone();
        let preset_model = session
            .services
            .models_manager
            .list_models(turn.config.as_ref(), RefreshStrategy::Offline)
            .await
            .into_iter()
            .map(|preset| preset.model)
            .find(|candidate| candidate != &default_model)
            .expect("expected a model alternative to the default model");

        overwrite_turn_config(&mut turn, |config| {
            config.subagent_presets.edit.model = Some(preset_model.clone());
        });
        let preset_only_overrides = SpawnConfigOverrides {
            preset: Some(SubagentPreset::Edit),
            ..SpawnConfigOverrides::default()
        };
        let preset_only_config = build_agent_spawn_config(&session, &turn, &preset_only_overrides)
            .await
            .expect("preset-only config");
        assert_eq!(preset_only_config.model, Some(preset_model));

        overwrite_turn_config(&mut turn, |config| {
            config.subagent_presets.edit.model = Some("not-a-real-model".to_string());
        });
        let explicit_overrides = SpawnConfigOverrides {
            preset: Some(SubagentPreset::Edit),
            model: Some(default_model.clone()),
            ..SpawnConfigOverrides::default()
        };
        let explicit_config = build_agent_spawn_config(&session, &turn, &explicit_overrides)
            .await
            .expect("explicit model should override preset model");
        assert_eq!(explicit_config.model, Some(default_model));
    }

    #[tokio::test]
    async fn build_agent_spawn_config_websearch_preset_uses_deep_research_without_reasoning() {
        let (session, turn) = make_session_and_context().await;
        let overrides = SpawnConfigOverrides {
            preset: Some(SubagentPreset::Websearch),
            ..SpawnConfigOverrides::default()
        };

        let config = build_agent_spawn_config(&session, &turn, &overrides)
            .await
            .expect("websearch preset config");

        assert_eq!(config.model, Some("o4-mini-deep-research".to_string()));
        assert_eq!(config.model_reasoning_effort, None);
    }

    #[tokio::test]
    async fn build_agent_spawn_config_websearch_preset_allows_explicit_model_override() {
        let (session, turn) = make_session_and_context().await;
        let explicit_model = turn.model_info.slug.clone();
        let overrides = SpawnConfigOverrides {
            preset: Some(SubagentPreset::Websearch),
            model: Some(explicit_model.clone()),
            ..SpawnConfigOverrides::default()
        };

        let config = build_agent_spawn_config(&session, &turn, &overrides)
            .await
            .expect("explicit model should override websearch preset");

        assert_eq!(config.model, Some(explicit_model));
        assert_eq!(config.model_reasoning_effort, turn.reasoning_effort);
    }

    #[tokio::test]
    async fn build_agent_spawn_config_uses_reasoning_effort_override_precedence() {
        let (session, mut turn) = make_session_and_context().await;
        turn.reasoning_effort = None;

        let model = turn.model_info.slug.clone();
        let model_info = session
            .services
            .models_manager
            .get_model_info(&model, turn.config.as_ref())
            .await;
        let supported_effort = model_info
            .supported_reasoning_levels
            .first()
            .map(|preset| preset.effort)
            .expect("expected at least one supported reasoning effort");
        let unsupported_effort = [
            ReasoningEffort::None,
            ReasoningEffort::Minimal,
            ReasoningEffort::Low,
            ReasoningEffort::Medium,
            ReasoningEffort::High,
            ReasoningEffort::XHigh,
        ]
        .into_iter()
        .find(|effort| {
            !model_info
                .supported_reasoning_levels
                .iter()
                .any(|preset| preset.effort == *effort)
        })
        .expect("expected at least one unsupported effort for the default model");

        overwrite_turn_config(&mut turn, |config| {
            config.subagent_presets.edit.reasoning_effort = Some(supported_effort);
        });
        let preset_only_overrides = SpawnConfigOverrides {
            preset: Some(SubagentPreset::Edit),
            ..SpawnConfigOverrides::default()
        };
        let preset_only_config = build_agent_spawn_config(&session, &turn, &preset_only_overrides)
            .await
            .expect("preset-only config");
        assert_eq!(
            preset_only_config.model_reasoning_effort,
            Some(supported_effort)
        );

        overwrite_turn_config(&mut turn, |config| {
            config.subagent_presets.edit.reasoning_effort = Some(unsupported_effort);
        });
        let explicit_overrides = SpawnConfigOverrides {
            preset: Some(SubagentPreset::Edit),
            reasoning_effort: Some(supported_effort),
            ..SpawnConfigOverrides::default()
        };
        let explicit_config = build_agent_spawn_config(&session, &turn, &explicit_overrides)
            .await
            .expect("explicit reasoning_effort should override preset reasoning_effort");
        assert_eq!(
            explicit_config.model_reasoning_effort,
            Some(supported_effort)
        );
    }

    #[tokio::test]
    async fn build_agent_spawn_config_allows_permission_downscope() {
        let (session, mut turn) = make_session_and_context().await;
        turn.approval_policy = AskForApproval::OnFailure;
        turn.sandbox_policy = SandboxPolicy::DangerFullAccess;
        let overrides = SpawnConfigOverrides {
            preset: None,
            model: None,
            reasoning_effort: None,
            reasoning_summary: None,
            approval_policy: Some(AskForApproval::Never),
            sandbox_mode: Some(SandboxMode::ReadOnly),
        };
        let config = build_agent_spawn_config(&session, &turn, &overrides)
            .await
            .expect("spawn config");

        assert_eq!(config.approval_policy.value(), AskForApproval::Never);
        assert_eq!(config.sandbox_policy.get(), &SandboxPolicy::ReadOnly);
    }

    #[tokio::test]
    async fn build_agent_spawn_config_rejects_approval_policy_escalation() {
        let (session, mut turn) = make_session_and_context().await;
        turn.approval_policy = AskForApproval::Never;
        let overrides = SpawnConfigOverrides {
            preset: None,
            model: None,
            reasoning_effort: None,
            reasoning_summary: None,
            approval_policy: Some(AskForApproval::OnRequest),
            sandbox_mode: None,
        };
        let err = build_agent_spawn_config(&session, &turn, &overrides)
            .await
            .expect_err("escalation should be rejected");
        assert_eq!(
            err,
            FunctionCallError::RespondToModel(
                "approval_policy on-request is not allowed because it is more permissive than parent policy never".to_string()
            )
        );
    }

    #[tokio::test]
    async fn build_agent_spawn_config_rejects_sandbox_mode_escalation() {
        let (session, mut turn) = make_session_and_context().await;
        turn.sandbox_policy = SandboxPolicy::ReadOnly;
        let overrides = SpawnConfigOverrides {
            preset: None,
            model: None,
            reasoning_effort: None,
            reasoning_summary: None,
            approval_policy: None,
            sandbox_mode: Some(SandboxMode::WorkspaceWrite),
        };
        let err = build_agent_spawn_config(&session, &turn, &overrides)
            .await
            .expect_err("escalation should be rejected");
        assert_eq!(
            err,
            FunctionCallError::RespondToModel(
                "sandbox_mode workspace-write is not allowed because it is more permissive than parent mode read-only".to_string()
            )
        );
    }

    #[tokio::test]
    async fn build_agent_spawn_config_allows_permission_escalation_when_enabled() {
        let (session, mut turn) = make_session_and_context().await;
        turn.approval_policy = AskForApproval::Never;
        turn.sandbox_policy = SandboxPolicy::ReadOnly;
        overwrite_turn_config(&mut turn, |config| {
            config.allow_subagent_permission_escalation = true;
        });
        let overrides = SpawnConfigOverrides {
            preset: None,
            model: None,
            reasoning_effort: None,
            reasoning_summary: None,
            approval_policy: Some(AskForApproval::OnRequest),
            sandbox_mode: Some(SandboxMode::WorkspaceWrite),
        };
        let config = build_agent_spawn_config(&session, &turn, &overrides)
            .await
            .expect("escalation should be allowed when enabled");

        assert_eq!(config.approval_policy.value(), AskForApproval::OnRequest);
        assert!(matches!(
            config.sandbox_policy.get(),
            SandboxPolicy::WorkspaceWrite { .. }
        ));
    }

    #[tokio::test]
    async fn build_agent_resume_config_clears_base_instructions() {
        let (_session, mut turn) = make_session_and_context().await;
        let mut base_config = (*turn.config).clone();
        base_config.base_instructions = Some("caller-base".to_string());
        turn.config = Arc::new(base_config);

        let config = build_agent_resume_config(&turn, 0).expect("resume config");

        let mut expected = (*turn.config).clone();
        expected.base_instructions = None;
        expected.model = Some(turn.model_info.slug.clone());
        expected.model_provider = turn.provider.clone();
        expected.model_reasoning_effort = turn.reasoning_effort;
        expected.model_reasoning_summary = turn.reasoning_summary;
        expected.developer_instructions = turn.developer_instructions.clone();
        expected.compact_prompt = turn.compact_prompt.clone();
        expected.shell_environment_policy = turn.shell_environment_policy.clone();
        expected.codex_linux_sandbox_exe = turn.codex_linux_sandbox_exe.clone();
        expected.cwd = turn.cwd.clone();
        expected
            .approval_policy
            .set(turn.approval_policy)
            .expect("approval policy set");
        expected
            .sandbox_policy
            .set(turn.sandbox_policy)
            .expect("sandbox policy set");
        assert_eq!(config, expected);
    }
}
