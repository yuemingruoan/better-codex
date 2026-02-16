use super::collab::CollabHandler;
use super::parse_arguments;
use crate::codex::Session;
use crate::codex::TurnContext;
use crate::function_tool::FunctionCallError;
use crate::tools::context::SharedTurnDiffTracker;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolOutput;
use crate::tools::context::ToolPayload;
use crate::tools::registry::ToolHandler;
use crate::tools::registry::ToolKind;
use async_trait::async_trait;
use codex_protocol::models::FunctionCallOutputBody;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Map as JsonMap;
use serde_json::Value as JsonValue;
use std::sync::Arc;

pub struct CollabBatchHandler;

#[derive(Debug, Deserialize)]
struct BatchArgs {
    operations: Vec<BatchOperation>,
    #[serde(default)]
    fail_fast: bool,
}

#[derive(Debug, Deserialize)]
struct BatchOperation {
    id: Option<String>,
    params: JsonMap<String, JsonValue>,
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
struct BatchSummary {
    total: usize,
    succeeded: usize,
    failed: usize,
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
struct TaskBatchOperationResult {
    id: Option<String>,
    success: bool,
    agent_id: Option<String>,
    error: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
struct TaskBatchResult {
    results: Vec<TaskBatchOperationResult>,
    summary: BatchSummary,
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
struct TaskSendBatchOperationResult {
    id: Option<String>,
    success: bool,
    agent_id: Option<String>,
    submission_id: Option<String>,
    error: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
struct TaskSendBatchResult {
    results: Vec<TaskSendBatchOperationResult>,
    summary: BatchSummary,
}

#[derive(Debug, Deserialize)]
struct SpawnAgentResult {
    agent_id: String,
}

#[derive(Debug, Deserialize)]
struct SendInputResult {
    submission_id: String,
}

#[async_trait]
impl ToolHandler for CollabBatchHandler {
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
            tracker,
            call_id,
            tool_name,
            payload,
            ..
        } = invocation;

        let arguments = match payload {
            ToolPayload::Function { arguments } => arguments,
            _ => {
                return Err(FunctionCallError::RespondToModel(
                    "collab batch handler received unsupported payload".to_string(),
                ));
            }
        };

        match tool_name.as_str() {
            "task_batch" => handle_task_batch(session, turn, tracker, call_id, arguments).await,
            "task_send_batch" => {
                handle_task_send_batch(session, turn, tracker, call_id, arguments).await
            }
            other => Err(FunctionCallError::RespondToModel(format!(
                "unsupported collab batch tool {other}"
            ))),
        }
    }
}

async fn handle_task_batch(
    session: Arc<Session>,
    turn: Arc<TurnContext>,
    tracker: SharedTurnDiffTracker,
    call_id: String,
    arguments: String,
) -> Result<ToolOutput, FunctionCallError> {
    let args = parse_batch_args(&arguments)?;
    let mut results = Vec::with_capacity(args.operations.len());
    for (index, operation) in args.operations.iter().enumerate() {
        let operation_call_id = format!("{call_id}:{index}");
        let output = run_collab_operation(
            session.clone(),
            turn.clone(),
            tracker.clone(),
            operation_call_id,
            "spawn_agent",
            &operation.params,
        )
        .await;
        match output {
            Ok(output) => match parse_spawn_result(output) {
                Ok(agent_id) => results.push(TaskBatchOperationResult {
                    id: operation.id.clone(),
                    success: true,
                    agent_id: Some(agent_id),
                    error: None,
                }),
                Err(error) => {
                    results.push(TaskBatchOperationResult {
                        id: operation.id.clone(),
                        success: false,
                        agent_id: None,
                        error: Some(error),
                    });
                    if args.fail_fast {
                        break;
                    }
                }
            },
            Err(err) => {
                results.push(TaskBatchOperationResult {
                    id: operation.id.clone(),
                    success: false,
                    agent_id: None,
                    error: Some(err.to_string()),
                });
                if args.fail_fast {
                    break;
                }
            }
        }
    }

    let summary = summarize(
        results.len(),
        results.iter().filter(|result| result.success).count(),
    );
    let all_succeeded = summary.failed == 0;
    let content = serde_json::to_string(&TaskBatchResult { results, summary }).map_err(|err| {
        FunctionCallError::Fatal(format!("failed to serialize task_batch result: {err}"))
    })?;
    Ok(ToolOutput::Function {
        body: FunctionCallOutputBody::Text(content),
        success: Some(all_succeeded),
    })
}

async fn handle_task_send_batch(
    session: Arc<Session>,
    turn: Arc<TurnContext>,
    tracker: SharedTurnDiffTracker,
    call_id: String,
    arguments: String,
) -> Result<ToolOutput, FunctionCallError> {
    let args = parse_batch_args(&arguments)?;
    let mut results = Vec::with_capacity(args.operations.len());
    for (index, operation) in args.operations.iter().enumerate() {
        let operation_call_id = format!("{call_id}:{index}");
        let agent_id = operation
            .params
            .get("agent_id")
            .and_then(JsonValue::as_str)
            .map(ToString::to_string);
        let output = run_collab_operation(
            session.clone(),
            turn.clone(),
            tracker.clone(),
            operation_call_id,
            "send_input",
            &operation.params,
        )
        .await;
        match output {
            Ok(output) => match parse_send_input_result(output) {
                Ok(submission_id) => results.push(TaskSendBatchOperationResult {
                    id: operation.id.clone(),
                    success: true,
                    agent_id,
                    submission_id: Some(submission_id),
                    error: None,
                }),
                Err(error) => {
                    results.push(TaskSendBatchOperationResult {
                        id: operation.id.clone(),
                        success: false,
                        agent_id,
                        submission_id: None,
                        error: Some(error),
                    });
                    if args.fail_fast {
                        break;
                    }
                }
            },
            Err(err) => {
                results.push(TaskSendBatchOperationResult {
                    id: operation.id.clone(),
                    success: false,
                    agent_id,
                    submission_id: None,
                    error: Some(err.to_string()),
                });
                if args.fail_fast {
                    break;
                }
            }
        }
    }

    let summary = summarize(
        results.len(),
        results.iter().filter(|result| result.success).count(),
    );
    let all_succeeded = summary.failed == 0;
    let content =
        serde_json::to_string(&TaskSendBatchResult { results, summary }).map_err(|err| {
            FunctionCallError::Fatal(format!("failed to serialize task_send_batch result: {err}"))
        })?;
    Ok(ToolOutput::Function {
        body: FunctionCallOutputBody::Text(content),
        success: Some(all_succeeded),
    })
}

fn summarize(total: usize, succeeded: usize) -> BatchSummary {
    BatchSummary {
        total,
        succeeded,
        failed: total.saturating_sub(succeeded),
    }
}

fn parse_batch_args(arguments: &str) -> Result<BatchArgs, FunctionCallError> {
    let args: BatchArgs = parse_arguments(arguments)?;
    if args.operations.is_empty() {
        return Err(FunctionCallError::RespondToModel(
            "operations must not be empty".to_string(),
        ));
    }
    Ok(args)
}

async fn run_collab_operation(
    session: Arc<Session>,
    turn: Arc<TurnContext>,
    tracker: SharedTurnDiffTracker,
    call_id: String,
    tool_name: &str,
    params: &JsonMap<String, JsonValue>,
) -> Result<ToolOutput, FunctionCallError> {
    let arguments = serde_json::to_string(params).map_err(|err| {
        FunctionCallError::Fatal(format!(
            "failed to serialize {tool_name} batch operation: {err}"
        ))
    })?;
    CollabHandler
        .handle(ToolInvocation {
            session,
            turn,
            tracker,
            call_id,
            tool_name: tool_name.to_string(),
            payload: ToolPayload::Function { arguments },
        })
        .await
}

fn parse_spawn_result(output: ToolOutput) -> Result<String, String> {
    let content = parse_tool_output_text(output, "spawn_agent")?;
    let result: SpawnAgentResult = serde_json::from_str(&content)
        .map_err(|err| format!("failed to parse spawn_agent result: {err}"))?;
    Ok(result.agent_id)
}

fn parse_send_input_result(output: ToolOutput) -> Result<String, String> {
    let content = parse_tool_output_text(output, "send_input")?;
    let result: SendInputResult = serde_json::from_str(&content)
        .map_err(|err| format!("failed to parse send_input result: {err}"))?;
    Ok(result.submission_id)
}

fn parse_tool_output_text(output: ToolOutput, tool_name: &str) -> Result<String, String> {
    match output {
        ToolOutput::Function { body, .. } => body
            .to_text()
            .ok_or_else(|| format!("failed to parse {tool_name} result: expected text output")),
        ToolOutput::Mcp { .. } => Err(format!(
            "failed to parse {tool_name} result: unexpected MCP output"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CodexAuth;
    use crate::ThreadManager;
    use crate::built_in_model_providers;
    use crate::codex::make_session_and_context;
    use crate::protocol::Op;
    use crate::turn_diff_tracker::TurnDiffTracker;
    use codex_protocol::ThreadId;
    use pretty_assertions::assert_eq;
    use serde_json::json;
    use std::sync::Arc;
    use tokio::sync::Mutex;

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

    #[tokio::test]
    async fn handler_rejects_empty_operations() {
        let (session, turn) = make_session_and_context().await;
        let invocation = invocation(
            Arc::new(session),
            Arc::new(turn),
            "task_batch",
            function_payload(json!({"operations": []})),
        );
        let Err(err) = CollabBatchHandler.handle(invocation).await else {
            panic!("empty operations should fail");
        };
        assert_eq!(
            err,
            FunctionCallError::RespondToModel("operations must not be empty".to_string())
        );
    }

    #[tokio::test]
    async fn task_send_batch_mixed_success_and_failure() {
        let (mut session, turn) = make_session_and_context().await;
        let manager = thread_manager();
        session.services.agent_control = manager.agent_control();
        let config = turn.config.as_ref().clone();
        let thread = manager.start_thread(config).await.expect("start thread");
        let existing_agent_id = thread.thread_id;
        let existing_agent_id_string = existing_agent_id.to_string();
        let missing_agent_id = ThreadId::new();
        let missing_agent_id_string = missing_agent_id.to_string();

        let invocation = invocation(
            Arc::new(session),
            Arc::new(turn),
            "task_send_batch",
            function_payload(json!({
                "operations": [
                    {
                        "id": "ok",
                        "params": {
                            "agent_id": existing_agent_id_string,
                            "items": [{"type": "text", "text": "hello"}]
                        }
                    },
                    {
                        "id": "missing",
                        "params": {
                            "agent_id": missing_agent_id_string,
                            "items": [{"type": "text", "text": "hello"}]
                        }
                    }
                ]
            })),
        );

        let output = CollabBatchHandler
            .handle(invocation)
            .await
            .expect("task_send_batch should return a batched result");
        let ToolOutput::Function {
            body: FunctionCallOutputBody::Text(content),
            success,
            ..
        } = output
        else {
            panic!("expected function output");
        };
        assert_eq!(success, Some(false));
        let result: TaskSendBatchResult =
            serde_json::from_str(&content).expect("task_send_batch result should be valid json");
        assert_eq!(
            result.summary,
            BatchSummary {
                total: 2,
                succeeded: 1,
                failed: 1,
            }
        );
        assert_eq!(result.results.len(), 2);
        assert_eq!(result.results[0].id.as_deref(), Some("ok"));
        assert_eq!(result.results[0].success, true);
        assert_eq!(
            result.results[0].agent_id.as_deref(),
            Some(existing_agent_id_string.as_str())
        );
        assert!(result.results[0].submission_id.is_some());
        assert_eq!(result.results[0].error, None);
        assert_eq!(result.results[1].id.as_deref(), Some("missing"));
        assert_eq!(result.results[1].success, false);
        assert_eq!(
            result.results[1].agent_id.as_deref(),
            Some(missing_agent_id_string.as_str())
        );
        assert_eq!(result.results[1].submission_id, None);
        assert_eq!(
            result.results[1].error,
            Some(format!("agent with id {missing_agent_id} not found"))
        );

        let _ = thread
            .thread
            .submit(Op::Shutdown {})
            .await
            .expect("shutdown should submit");
    }

    #[tokio::test]
    async fn task_send_batch_fail_fast_stops_after_first_error() {
        let (mut session, turn) = make_session_and_context().await;
        let manager = thread_manager();
        session.services.agent_control = manager.agent_control();
        let config = turn.config.as_ref().clone();
        let thread = manager.start_thread(config).await.expect("start thread");
        let existing_agent_id = thread.thread_id;
        let missing_agent_id = ThreadId::new();

        let invocation = invocation(
            Arc::new(session),
            Arc::new(turn),
            "task_send_batch",
            function_payload(json!({
                "fail_fast": true,
                "operations": [
                    {
                        "id": "missing",
                        "params": {
                            "agent_id": missing_agent_id.to_string(),
                            "items": [{"type": "text", "text": "hello"}]
                        }
                    },
                    {
                        "id": "ok",
                        "params": {
                            "agent_id": existing_agent_id.to_string(),
                            "items": [{"type": "text", "text": "should-not-run"}]
                        }
                    }
                ]
            })),
        );

        let output = CollabBatchHandler
            .handle(invocation)
            .await
            .expect("task_send_batch should return a batched result");
        let ToolOutput::Function {
            body: FunctionCallOutputBody::Text(content),
            success,
            ..
        } = output
        else {
            panic!("expected function output");
        };
        assert_eq!(success, Some(false));
        let result: TaskSendBatchResult =
            serde_json::from_str(&content).expect("task_send_batch result should be valid json");
        assert_eq!(
            result.summary,
            BatchSummary {
                total: 1,
                succeeded: 0,
                failed: 1,
            }
        );
        assert_eq!(result.results.len(), 1);
        assert_eq!(result.results[0].id.as_deref(), Some("missing"));
        assert_eq!(result.results[0].success, false);

        let ops = manager.captured_ops();
        assert_eq!(ops.is_empty(), true);

        let _ = thread
            .thread
            .submit(Op::Shutdown {})
            .await
            .expect("shutdown should submit");
    }
}
