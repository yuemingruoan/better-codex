use crate::agent::AgentStatus;
use crate::agent::guards::Guards;
use crate::error::CodexErr;
use crate::error::Result as CodexResult;
use crate::thread_manager::AgentRegistryRecord;
use crate::thread_manager::AgentSpawnMetadata;
use crate::thread_manager::ThreadManagerState;
use codex_protocol::ThreadId;
use codex_protocol::protocol::Op;
use codex_protocol::protocol::SessionSource;
use codex_protocol::user_input::UserInput;
use std::collections::HashMap;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Weak;
use tokio::sync::watch;

/// Control-plane handle for multi-agent operations.
/// `AgentControl` is held by each session (via `SessionServices`). It provides capability to
/// spawn new agents and the inter-agent communication layer.
/// An `AgentControl` instance is shared per "user session" which means the same `AgentControl`
/// is used for every sub-agent spawned by Codex. By doing so, we make sure the guards are
/// scoped to a user session.
#[derive(Clone, Default)]
pub(crate) struct AgentControl {
    /// Weak handle back to the global thread registry/state.
    /// This is `Weak` to avoid reference cycles and shadow persistence of the form
    /// `ThreadManagerState -> CodexThread -> Session -> SessionServices -> ThreadManagerState`.
    manager: Weak<ThreadManagerState>,
    state: Arc<Guards>,
}

impl AgentControl {
    /// Construct a new `AgentControl` that can spawn/message agents via the given manager state.
    pub(crate) fn new(manager: Weak<ThreadManagerState>) -> Self {
        Self {
            manager,
            ..Default::default()
        }
    }

    /// Spawn a new agent thread and submit the initial input items.
    pub(crate) async fn spawn_agent(
        &self,
        config: crate::config::Config,
        items: Vec<UserInput>,
        session_source: Option<SessionSource>,
    ) -> CodexResult<ThreadId> {
        self.spawn_agent_with_metadata_and_source(
            config,
            AgentSpawnMetadata::default(),
            items,
            session_source,
        )
        .await
    }

    #[allow(dead_code)] // Kept for compatibility with existing call sites/tests.
    /// Spawn a new agent thread using a plain-text prompt.
    pub(crate) async fn spawn_agent_with_metadata(
        &self,
        config: crate::config::Config,
        prompt: String,
        metadata: AgentSpawnMetadata,
    ) -> CodexResult<ThreadId> {
        self.spawn_agent_with_metadata_and_source(
            config,
            metadata,
            vec![UserInput::Text {
                text: prompt,
                // Agent control prompts are plain text with no UI text elements.
                text_elements: Vec::new(),
            }],
            None,
        )
        .await
    }

    pub(crate) async fn spawn_agent_with_metadata_and_source(
        &self,
        config: crate::config::Config,
        metadata: AgentSpawnMetadata,
        items: Vec<UserInput>,
        session_source: Option<SessionSource>,
    ) -> CodexResult<ThreadId> {
        let state = self.upgrade()?;
        let reservation = self.state.reserve_spawn_slot(config.agent_max_threads)?;

        // The same `AgentControl` is sent to spawn the thread.
        let new_thread = match session_source {
            Some(session_source) => {
                state
                    .spawn_new_thread_with_source(config, self.clone(), session_source)
                    .await?
            }
            None => state.spawn_new_thread(config, self.clone()).await?,
        };
        reservation.commit(new_thread.thread_id);

        // Notify a new thread has been created. This notification will be processed by clients
        // to subscribe or drain this newly created thread.
        // TODO(jif) add helper for drain
        state.notify_thread_created(new_thread.thread_id);
        state.register_agent(new_thread.thread_id, metadata).await;

        self.send_input(new_thread.thread_id, items).await?;

        Ok(new_thread.thread_id)
    }

    /// Resume an existing agent thread from a recorded rollout file.
    pub(crate) async fn resume_agent_from_rollout(
        &self,
        config: crate::config::Config,
        rollout_path: PathBuf,
        session_source: SessionSource,
    ) -> CodexResult<ThreadId> {
        let state = self.upgrade()?;
        let reservation = self.state.reserve_spawn_slot(config.agent_max_threads)?;

        let resumed_thread = state
            .resume_thread_from_rollout_with_source(
                config,
                rollout_path,
                self.clone(),
                session_source,
            )
            .await?;
        reservation.commit(resumed_thread.thread_id);
        // Resumed threads are re-registered in-memory and need the same listener
        // attachment path as freshly spawned threads.
        state.notify_thread_created(resumed_thread.thread_id);

        Ok(resumed_thread.thread_id)
    }

    /// Interrupt the current task for an existing agent thread.
    pub(crate) async fn interrupt_agent(&self, agent_id: ThreadId) -> CodexResult<String> {
        let state = self.upgrade()?;
        state.send_op(agent_id, Op::Interrupt).await
    }

    /// Send rich user input items to an existing agent thread.
    pub(crate) async fn send_input(
        &self,
        agent_id: ThreadId,
        items: Vec<UserInput>,
    ) -> CodexResult<String> {
        let state = self.upgrade()?;
        let result = state
            .send_op(
                agent_id,
                Op::UserInput {
                    items,
                    final_output_json_schema: None,
                },
            )
            .await;
        if matches!(result, Err(CodexErr::InternalAgentDied)) {
            let _ = state.remove_thread(&agent_id).await;
            self.state.release_spawned_thread(agent_id);
        }
        result
    }

    #[allow(dead_code)] // Kept for compatibility with existing call sites/tests.
    /// Send a `user` prompt to an existing agent thread.
    pub(crate) async fn send_prompt(
        &self,
        agent_id: ThreadId,
        prompt: String,
    ) -> CodexResult<String> {
        self.send_input(
            agent_id,
            vec![UserInput::Text {
                text: prompt,
                // Agent control prompts are plain text with no UI text elements.
                text_elements: Vec::new(),
            }],
        )
        .await
    }

    #[allow(dead_code)] // Kept for compatibility with existing call sites/tests.
    /// Submit a shutdown request to an existing agent thread.
    pub(crate) async fn shutdown_agent(&self, agent_id: ThreadId) -> CodexResult<String> {
        self.shutdown_agent_with_descendants(agent_id, true).await
    }

    /// Submit a shutdown request and optionally close active descendants first.
    pub(crate) async fn shutdown_agent_with_descendants(
        &self,
        agent_id: ThreadId,
        auto_close_descendants: bool,
    ) -> CodexResult<String> {
        let mut close_errors = Vec::new();
        if auto_close_descendants {
            for descendant_id in self.collect_active_descendant_ids(agent_id).await? {
                if let Err(err) = self.shutdown_agent_only(descendant_id).await {
                    close_errors.push(format!("failed to close descendant {descendant_id}: {err}"));
                }
            }
        }
        let close_result = self.shutdown_agent_only(agent_id).await;
        if let Err(err) = &close_result {
            close_errors.push(format!("failed to close agent {agent_id}: {err}"));
        }
        if close_errors.is_empty() {
            close_result
        } else {
            Err(CodexErr::Fatal(close_errors.join("; ")))
        }
    }

    /// Fetch the last known status for `agent_id`, returning `NotFound` when unavailable.
    pub(crate) async fn get_status(&self, agent_id: ThreadId) -> AgentStatus {
        let Ok(state) = self.upgrade() else {
            // No agent available if upgrade fails.
            return AgentStatus::NotFound;
        };
        let Ok(thread) = state.get_thread(agent_id).await else {
            return AgentStatus::NotFound;
        };
        thread.agent_status().await
    }

    /// Subscribe to status updates for `agent_id`, yielding the latest value and changes.
    pub(crate) async fn subscribe_status(
        &self,
        agent_id: ThreadId,
    ) -> CodexResult<watch::Receiver<AgentStatus>> {
        let state = self.upgrade()?;
        let thread = state.get_thread(agent_id).await?;
        Ok(thread.subscribe_status())
    }

    pub(crate) async fn list_agents(&self) -> CodexResult<Vec<AgentRegistryRecord>> {
        let state = self.upgrade()?;
        Ok(state.list_agent_records().await)
    }

    pub(crate) async fn get_agent_record(
        &self,
        agent_id: ThreadId,
    ) -> CodexResult<Option<AgentRegistryRecord>> {
        let state = self.upgrade()?;
        Ok(state.get_agent_record(agent_id).await)
    }

    pub(crate) async fn rename_agent(
        &self,
        agent_id: ThreadId,
        name: String,
    ) -> CodexResult<AgentRegistryRecord> {
        let state = self.upgrade()?;
        state.rename_agent(agent_id, name).await
    }

    async fn shutdown_agent_only(&self, agent_id: ThreadId) -> CodexResult<String> {
        let state = self.upgrade()?;
        let result = state.send_op(agent_id, Op::Shutdown {}).await;
        let _ = state.remove_thread(&agent_id).await;
        self.state.release_spawned_thread(agent_id);
        if result.is_ok() {
            state
                .mark_agent_closed(agent_id, AgentStatus::Shutdown)
                .await;
        }
        result
    }

    async fn collect_active_descendant_ids(
        &self,
        agent_id: ThreadId,
    ) -> CodexResult<Vec<ThreadId>> {
        let state = self.upgrade()?;
        let records = state.list_agent_records().await;
        let mut children_by_parent: HashMap<ThreadId, Vec<ThreadId>> = HashMap::new();
        for record in records {
            if record.closed {
                continue;
            }
            if let Some(parent_id) = record.creator_thread_id {
                children_by_parent
                    .entry(parent_id)
                    .or_default()
                    .push(record.agent_id);
            }
        }

        let mut seen = HashSet::new();
        let mut stack = vec![agent_id];
        let mut descendants = Vec::new();
        while let Some(parent_id) = stack.pop() {
            if let Some(children) = children_by_parent.get(&parent_id) {
                for child_id in children {
                    if seen.insert(*child_id) {
                        stack.push(*child_id);
                        descendants.push(*child_id);
                    }
                }
            }
        }

        // Close deepest descendants first so parent shutdown doesn't strand nested workers.
        descendants.reverse();
        Ok(descendants)
    }

    fn upgrade(&self) -> CodexResult<Arc<ThreadManagerState>> {
        self.manager
            .upgrade()
            .ok_or_else(|| CodexErr::UnsupportedOperation("thread manager dropped".to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CodexAuth;
    use crate::CodexThread;
    use crate::ThreadManager;
    use crate::agent::agent_status_from_event;
    use crate::config::Config;
    use crate::config::ConfigBuilder;
    use assert_matches::assert_matches;
    use codex_protocol::config_types::ModeKind;
    use codex_protocol::protocol::ErrorEvent;
    use codex_protocol::protocol::EventMsg;
    use codex_protocol::protocol::TurnAbortReason;
    use codex_protocol::protocol::TurnAbortedEvent;
    use codex_protocol::protocol::TurnCompleteEvent;
    use codex_protocol::protocol::TurnStartedEvent;
    use pretty_assertions::assert_eq;
    use tempfile::TempDir;
    use toml::Value as TomlValue;

    async fn test_config_with_cli_overrides(
        cli_overrides: Vec<(String, TomlValue)>,
    ) -> (TempDir, Config) {
        let home = TempDir::new().expect("create temp dir");
        let config = ConfigBuilder::default()
            .codex_home(home.path().to_path_buf())
            .cli_overrides(cli_overrides)
            .build()
            .await
            .expect("load default test config");
        (home, config)
    }

    async fn test_config() -> (TempDir, Config) {
        test_config_with_cli_overrides(Vec::new()).await
    }

    fn text_input(text: &str) -> Vec<UserInput> {
        vec![UserInput::Text {
            text: text.to_string(),
            text_elements: Vec::new(),
        }]
    }

    struct AgentControlHarness {
        _home: TempDir,
        config: Config,
        manager: ThreadManager,
        control: AgentControl,
    }

    impl AgentControlHarness {
        async fn new() -> Self {
            let (home, config) = test_config().await;
            let manager = ThreadManager::with_models_provider_and_home(
                CodexAuth::from_api_key("dummy"),
                config.model_provider.clone(),
                config.codex_home.clone(),
            );
            let control = manager.agent_control();
            Self {
                _home: home,
                config,
                manager,
                control,
            }
        }

        async fn start_thread(&self) -> (ThreadId, Arc<CodexThread>) {
            let new_thread = self
                .manager
                .start_thread(self.config.clone())
                .await
                .expect("start thread");
            (new_thread.thread_id, new_thread.thread)
        }
    }

    #[tokio::test]
    async fn send_prompt_errors_when_manager_dropped() {
        let control = AgentControl::default();
        let err = control
            .send_prompt(ThreadId::new(), "hello".to_string())
            .await
            .expect_err("send_prompt should fail without a manager");
        assert_eq!(
            err.to_string(),
            "unsupported operation: thread manager dropped"
        );
    }

    #[tokio::test]
    async fn get_status_returns_not_found_without_manager() {
        let control = AgentControl::default();
        let got = control.get_status(ThreadId::new()).await;
        assert_eq!(got, AgentStatus::NotFound);
    }

    #[tokio::test]
    async fn on_event_updates_status_from_task_started() {
        let status = agent_status_from_event(&EventMsg::TurnStarted(TurnStartedEvent {
            model_context_window: None,
            collaboration_mode_kind: ModeKind::Default,
        }));
        assert_eq!(status, Some(AgentStatus::Running));
    }

    #[tokio::test]
    async fn on_event_updates_status_from_task_complete() {
        let status = agent_status_from_event(&EventMsg::TurnComplete(TurnCompleteEvent {
            last_agent_message: Some("done".to_string()),
        }));
        let expected = AgentStatus::Completed(Some("done".to_string()));
        assert_eq!(status, Some(expected));
    }

    #[tokio::test]
    async fn on_event_updates_status_from_error() {
        let status = agent_status_from_event(&EventMsg::Error(ErrorEvent {
            message: "boom".to_string(),
            codex_error_info: None,
        }));

        let expected = AgentStatus::Errored("boom".to_string());
        assert_eq!(status, Some(expected));
    }

    #[tokio::test]
    async fn on_event_updates_status_from_turn_aborted() {
        let status = agent_status_from_event(&EventMsg::TurnAborted(TurnAbortedEvent {
            reason: TurnAbortReason::Interrupted,
        }));

        let expected = AgentStatus::Errored("Interrupted".to_string());
        assert_eq!(status, Some(expected));
    }

    #[tokio::test]
    async fn on_event_updates_status_from_shutdown_complete() {
        let status = agent_status_from_event(&EventMsg::ShutdownComplete);
        assert_eq!(status, Some(AgentStatus::Shutdown));
    }

    #[tokio::test]
    async fn spawn_agent_errors_when_manager_dropped() {
        let control = AgentControl::default();
        let (_home, config) = test_config().await;
        let err = control
            .spawn_agent(config, text_input("hello"), None)
            .await
            .expect_err("spawn_agent should fail without a manager");
        assert_eq!(
            err.to_string(),
            "unsupported operation: thread manager dropped"
        );
    }

    #[tokio::test]
    async fn send_prompt_errors_when_thread_missing() {
        let harness = AgentControlHarness::new().await;
        let thread_id = ThreadId::new();
        let err = harness
            .control
            .send_prompt(thread_id, "hello".to_string())
            .await
            .expect_err("send_prompt should fail for missing thread");
        assert_matches!(err, CodexErr::ThreadNotFound(id) if id == thread_id);
    }

    #[tokio::test]
    async fn get_status_returns_not_found_for_missing_thread() {
        let harness = AgentControlHarness::new().await;
        let status = harness.control.get_status(ThreadId::new()).await;
        assert_eq!(status, AgentStatus::NotFound);
    }

    #[tokio::test]
    async fn get_status_returns_pending_init_for_new_thread() {
        let harness = AgentControlHarness::new().await;
        let (thread_id, _) = harness.start_thread().await;
        let status = harness.control.get_status(thread_id).await;
        assert_eq!(status, AgentStatus::PendingInit);
    }

    #[tokio::test]
    async fn subscribe_status_errors_for_missing_thread() {
        let harness = AgentControlHarness::new().await;
        let thread_id = ThreadId::new();
        let err = harness
            .control
            .subscribe_status(thread_id)
            .await
            .expect_err("subscribe_status should fail for missing thread");
        assert_matches!(err, CodexErr::ThreadNotFound(id) if id == thread_id);
    }

    #[tokio::test]
    async fn subscribe_status_updates_on_shutdown() {
        let harness = AgentControlHarness::new().await;
        let (thread_id, thread) = harness.start_thread().await;
        let mut status_rx = harness
            .control
            .subscribe_status(thread_id)
            .await
            .expect("subscribe_status should succeed");
        assert_eq!(status_rx.borrow().clone(), AgentStatus::PendingInit);

        let _ = thread
            .submit(Op::Shutdown {})
            .await
            .expect("shutdown should submit");

        let _ = status_rx.changed().await;
        assert_eq!(status_rx.borrow().clone(), AgentStatus::Shutdown);
    }

    #[tokio::test]
    async fn send_prompt_submits_user_message() {
        let harness = AgentControlHarness::new().await;
        let (thread_id, _thread) = harness.start_thread().await;

        let submission_id = harness
            .control
            .send_prompt(thread_id, "hello from tests".to_string())
            .await
            .expect("send_prompt should succeed");
        assert!(!submission_id.is_empty());
        let expected = (
            thread_id,
            Op::UserInput {
                items: vec![UserInput::Text {
                    text: "hello from tests".to_string(),
                    text_elements: Vec::new(),
                }],
                final_output_json_schema: None,
            },
        );
        let captured = harness
            .manager
            .captured_ops()
            .into_iter()
            .find(|entry| *entry == expected);
        assert_eq!(captured, Some(expected));
    }

    #[tokio::test]
    async fn spawn_agent_creates_thread_and_sends_prompt() {
        let harness = AgentControlHarness::new().await;
        let thread_id = harness
            .control
            .spawn_agent(harness.config.clone(), text_input("spawned"), None)
            .await
            .expect("spawn_agent should succeed");
        let _thread = harness
            .manager
            .get_thread(thread_id)
            .await
            .expect("thread should be registered");
        let expected = (
            thread_id,
            Op::UserInput {
                items: vec![UserInput::Text {
                    text: "spawned".to_string(),
                    text_elements: Vec::new(),
                }],
                final_output_json_schema: None,
            },
        );
        let captured = harness
            .manager
            .captured_ops()
            .into_iter()
            .find(|entry| *entry == expected);
        assert_eq!(captured, Some(expected));
    }

    #[tokio::test]
    async fn shutdown_agent_releases_spawn_slot() {
        let harness = AgentControlHarness::new().await;
        let mut config = harness.config.clone();
        config.agent_max_threads = Some(1);

        let first = harness
            .control
            .spawn_agent(config.clone(), text_input("first"), None)
            .await
            .expect("first spawn should succeed");

        let _ = harness
            .control
            .shutdown_agent(first)
            .await
            .expect("shutdown should succeed");

        let second = harness
            .control
            .spawn_agent(config, text_input("second"), None)
            .await
            .expect("second spawn should succeed after slot release");

        assert_ne!(first, second);

        let _ = harness
            .control
            .shutdown_agent(second)
            .await
            .expect("second shutdown should succeed");
    }

    #[tokio::test]
    async fn shutdown_agent_closes_active_descendants_first() {
        let harness = AgentControlHarness::new().await;
        let root_thread_id = ThreadId::new();
        let parent_id = harness
            .control
            .spawn_agent_with_metadata(
                harness.config.clone(),
                "parent".to_string(),
                AgentSpawnMetadata {
                    creator_thread_id: Some(root_thread_id),
                    goal: "parent".to_string(),
                    allow_nested_agents: true,
                    ..Default::default()
                },
            )
            .await
            .expect("spawn parent agent");
        let child_id = harness
            .control
            .spawn_agent_with_metadata(
                harness.config.clone(),
                "child".to_string(),
                AgentSpawnMetadata {
                    creator_thread_id: Some(parent_id),
                    goal: "child".to_string(),
                    ..Default::default()
                },
            )
            .await
            .expect("spawn child agent");

        let _ = harness
            .control
            .shutdown_agent(parent_id)
            .await
            .expect("shutdown parent should succeed");

        assert_eq!(
            harness.control.get_status(parent_id).await,
            AgentStatus::NotFound
        );
        assert_eq!(
            harness.control.get_status(child_id).await,
            AgentStatus::NotFound
        );

        let parent_record = harness
            .control
            .get_agent_record(parent_id)
            .await
            .expect("load parent record")
            .expect("parent record should exist");
        let child_record = harness
            .control
            .get_agent_record(child_id)
            .await
            .expect("load child record")
            .expect("child record should exist");
        assert!(parent_record.closed);
        assert_eq!(parent_record.status, AgentStatus::Shutdown);
        assert!(child_record.closed);
        assert_eq!(child_record.status, AgentStatus::Shutdown);
    }

    #[tokio::test]
    async fn shutdown_agent_reports_close_errors() {
        let harness = AgentControlHarness::new().await;
        let root_thread_id = ThreadId::new();
        let parent_id = harness
            .control
            .spawn_agent_with_metadata(
                harness.config.clone(),
                "parent".to_string(),
                AgentSpawnMetadata {
                    creator_thread_id: Some(root_thread_id),
                    goal: "parent".to_string(),
                    allow_nested_agents: true,
                    ..Default::default()
                },
            )
            .await
            .expect("spawn parent agent");
        let _child_id = harness
            .control
            .spawn_agent_with_metadata(
                harness.config.clone(),
                "child".to_string(),
                AgentSpawnMetadata {
                    creator_thread_id: Some(parent_id),
                    goal: "child".to_string(),
                    ..Default::default()
                },
            )
            .await
            .expect("spawn child agent");

        harness
            .manager
            .remove_and_close_all_threads()
            .await
            .expect("remove all threads");

        let err = harness
            .control
            .shutdown_agent(parent_id)
            .await
            .expect_err("shutdown should report descendant close errors");
        let err_text = err.to_string();
        assert_eq!(
            err_text.contains(&format!("failed to close agent {parent_id}:")),
            true
        );
    }

    #[tokio::test]
    async fn shutdown_agent_can_leave_descendants_running_when_auto_close_disabled() {
        let harness = AgentControlHarness::new().await;
        let root_thread_id = ThreadId::new();
        let parent_id = harness
            .control
            .spawn_agent_with_metadata(
                harness.config.clone(),
                "parent".to_string(),
                AgentSpawnMetadata {
                    creator_thread_id: Some(root_thread_id),
                    goal: "parent".to_string(),
                    allow_nested_agents: true,
                    ..Default::default()
                },
            )
            .await
            .expect("spawn parent agent");
        let child_id = harness
            .control
            .spawn_agent_with_metadata(
                harness.config.clone(),
                "child".to_string(),
                AgentSpawnMetadata {
                    creator_thread_id: Some(parent_id),
                    goal: "child".to_string(),
                    ..Default::default()
                },
            )
            .await
            .expect("spawn child agent");

        let _ = harness
            .control
            .shutdown_agent_with_descendants(parent_id, false)
            .await
            .expect("shutdown parent should succeed");

        assert_eq!(
            harness.control.get_status(parent_id).await,
            AgentStatus::NotFound
        );
        assert_ne!(
            harness.control.get_status(child_id).await,
            AgentStatus::NotFound
        );

        let parent_record = harness
            .control
            .get_agent_record(parent_id)
            .await
            .expect("load parent record")
            .expect("parent record should exist");
        let child_record = harness
            .control
            .get_agent_record(child_id)
            .await
            .expect("load child record")
            .expect("child record should exist");
        assert!(parent_record.closed);
        assert_eq!(parent_record.status, AgentStatus::Shutdown);
        assert!(!child_record.closed);

        let _ = harness.control.shutdown_agent(child_id).await;
    }
}
