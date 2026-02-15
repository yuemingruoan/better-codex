use async_trait::async_trait;
use codex_protocol::config_types::CollaborationMode;
use codex_protocol::config_types::ModeKind;
use codex_protocol::models::FunctionCallOutputBody;
use serde::Deserialize;
use serde_json::Map as JsonMap;
use serde_json::Value as JsonValue;
use serde_json::json;

use crate::codex::SessionSettingsUpdate;
use crate::function_tool::FunctionCallError;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolOutput;
use crate::tools::context::ToolPayload;
use crate::tools::handlers::GrepFilesHandler;
use crate::tools::handlers::PlanHandler;
use crate::tools::handlers::ReadFileHandler;
use crate::tools::handlers::RequestUserInputHandler;
use crate::tools::handlers::SearchToolBm25Handler;
use crate::tools::handlers::UnifiedExecHandler;
use crate::tools::handlers::collab::CollabHandler;
use crate::tools::handlers::parse_arguments;
use crate::tools::registry::ToolHandler;
use crate::tools::registry::ToolKind;

pub struct ClaudeToolAdapterHandler;

const TASK_TOOL_NAME: &str = "Task";
const TASK_OUTPUT_TOOL_NAME: &str = "TaskOutput";
const TASK_STOP_TOOL_NAME: &str = "TaskStop";
const TOOL_SEARCH_TOOL_NAME: &str = "ToolSearch";
const SKILL_TOOL_NAME: &str = "Skill";
const ASK_USER_QUESTION_TOOL_NAME: &str = "AskUserQuestion";
const BASH_TOOL_NAME: &str = "Bash";
const READ_TOOL_NAME: &str = "Read";
const GREP_TOOL_NAME: &str = "Grep";
const TODO_WRITE_TOOL_NAME: &str = "TodoWrite";
const ENTER_PLAN_MODE_TOOL_NAME: &str = "EnterPlanMode";
const EXIT_PLAN_MODE_TOOL_NAME: &str = "ExitPlanMode";

fn default_block() -> bool {
    true
}

#[derive(Debug, Deserialize)]
struct TaskArgs {
    description: Option<String>,
    prompt: Option<String>,
    subagent_type: Option<String>,
    max_turns: Option<u32>,
    mode: Option<String>,
    model: Option<String>,
    preset: Option<String>,
    name: Option<String>,
    resume: Option<String>,
    run_in_background: Option<bool>,
    team_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TaskOutputArgs {
    agent_id: Option<String>,
    #[serde(default = "default_block")]
    block: bool,
    timeout: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct TaskStopArgs {
    agent_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ToolSearchArgs {
    query: Option<String>,
    max_results: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct SkillArgs {
    skill: Option<String>,
    args: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AskUserQuestionArgs {
    questions: Option<Vec<AskUserQuestion>>,
}

#[derive(Debug, Deserialize)]
struct AskUserQuestion {
    id: Option<String>,
    header: Option<String>,
    question: Option<String>,
    #[serde(rename = "multiSelect")]
    multi_select: Option<bool>,
    options: Option<Vec<AskUserQuestionOption>>,
}

#[derive(Debug, Deserialize)]
struct AskUserQuestionOption {
    label: Option<String>,
    description: Option<String>,
}

#[derive(Debug, Deserialize)]
struct BashArgs {
    command: Option<String>,
    timeout: Option<i64>,
    description: Option<String>,
    run_in_background: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct ReadArgs {
    file_path: Option<String>,
    offset: Option<usize>,
    limit: Option<usize>,
    mode: Option<String>,
    indentation: Option<JsonValue>,
    pages: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GrepArgs {
    pattern: Option<String>,
    path: Option<String>,
    glob: Option<String>,
    head_limit: Option<usize>,
    output_mode: Option<String>,
    offset: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct TodoWriteArgs {
    todos: Option<Vec<TodoEntry>>,
}

#[derive(Debug, Deserialize)]
struct TodoEntry {
    content: Option<String>,
    #[serde(rename = "activeForm")]
    active_form: Option<String>,
    status: Option<String>,
}

#[async_trait]
impl ToolHandler for ClaudeToolAdapterHandler {
    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    fn matches_kind(&self, payload: &ToolPayload) -> bool {
        matches!(payload, ToolPayload::Function { .. })
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<ToolOutput, FunctionCallError> {
        let arguments = match invocation.payload.clone() {
            ToolPayload::Function { arguments } => arguments,
            _ => {
                return Err(FunctionCallError::RespondToModel(
                    "Claude tool adapter only supports function payloads / Claude 工具适配仅支持函数负载"
                        .to_string(),
                ));
            }
        };

        match invocation.tool_name.as_str() {
            TASK_TOOL_NAME => {
                let args: TaskArgs = parse_arguments(&arguments)?;
                let mapped_args = map_task_to_spawn_payload(args)?;
                dispatch_to_collab(invocation, "spawn_agent", mapped_args).await
            }
            TASK_OUTPUT_TOOL_NAME => {
                let args: TaskOutputArgs = parse_arguments(&arguments)?;
                let mapped_args = map_task_output_to_wait_payload(args)?;
                dispatch_to_collab(invocation, "wait", mapped_args).await
            }
            TASK_STOP_TOOL_NAME => {
                let args: TaskStopArgs = parse_arguments(&arguments)?;
                let mapped_args = map_task_stop_to_close_payload(args)?;
                dispatch_to_collab(invocation, "close_agent", mapped_args).await
            }
            TOOL_SEARCH_TOOL_NAME => {
                let args: ToolSearchArgs = parse_arguments(&arguments)?;
                let mapped_args = map_tool_search_payload(args)?;
                dispatch_to_search_tool(invocation, mapped_args).await
            }
            SKILL_TOOL_NAME => {
                let args: SkillArgs = parse_arguments(&arguments)?;
                let mapped_args = map_skill_to_spawn_payload(args)?;
                dispatch_to_collab(invocation, "spawn_agent", mapped_args).await
            }
            ASK_USER_QUESTION_TOOL_NAME => {
                let args: AskUserQuestionArgs = parse_arguments(&arguments)?;
                let mapped_args = map_ask_user_question_to_request_payload(args)?;
                dispatch_to_request_user_input(invocation, mapped_args).await
            }
            BASH_TOOL_NAME => {
                let args: BashArgs = parse_arguments(&arguments)?;
                let mapped_args = map_bash_to_exec_payload(args)?;
                dispatch_to_exec_command(invocation, mapped_args).await
            }
            READ_TOOL_NAME => {
                let args: ReadArgs = parse_arguments(&arguments)?;
                let mapped_args = map_read_to_read_file_payload(args)?;
                dispatch_to_read_file(invocation, mapped_args).await
            }
            GREP_TOOL_NAME => {
                let args: GrepArgs = parse_arguments(&arguments)?;
                let mapped_args = map_grep_to_grep_files_payload(args)?;
                dispatch_to_grep_files(invocation, mapped_args).await
            }
            TODO_WRITE_TOOL_NAME => {
                let args: TodoWriteArgs = parse_arguments(&arguments)?;
                let mapped_args = map_todo_write_to_update_plan_payload(args)?;
                dispatch_to_update_plan(invocation, mapped_args).await
            }
            ENTER_PLAN_MODE_TOOL_NAME => {
                switch_collaboration_mode(invocation, ModeKind::Plan).await
            }
            EXIT_PLAN_MODE_TOOL_NAME => {
                switch_collaboration_mode(invocation, ModeKind::Default).await
            }
            other => Err(FunctionCallError::RespondToModel(format!(
                "unsupported Claude tool alias {other} / 不支持的 Claude 工具别名: {other}"
            ))),
        }
    }
}

fn map_task_to_spawn_payload(args: TaskArgs) -> Result<JsonValue, FunctionCallError> {
    let TaskArgs {
        description,
        prompt,
        subagent_type,
        max_turns,
        mode,
        model,
        preset,
        name,
        resume,
        run_in_background,
        team_name,
    } = args;

    let _ignored = (max_turns, mode, resume, run_in_background, team_name);
    let prompt = required_non_empty_text(
        prompt.as_deref(),
        "prompt must not be empty / prompt 不能为空",
    )?;

    let mut payload = JsonMap::new();
    payload.insert(
        "items".to_string(),
        json!([{ "type": "text", "text": prompt }]),
    );

    if let Some(agent_type) = normalize_text(subagent_type.as_deref())
        && is_supported_agent_type(&agent_type)
    {
        payload.insert("agent_type".to_string(), JsonValue::String(agent_type));
    }

    if let Some(name) =
        normalize_text(name.as_deref()).or_else(|| normalize_text(description.as_deref()))
    {
        payload.insert("name".to_string(), JsonValue::String(name));
    }

    if let Some(model) = normalize_text(model.as_deref()) {
        payload.insert("model".to_string(), JsonValue::String(model));
    }
    if let Some(preset) = normalize_text(preset.as_deref()) {
        payload.insert("preset".to_string(), JsonValue::String(preset));
    }

    Ok(JsonValue::Object(payload))
}

fn map_task_output_to_wait_payload(args: TaskOutputArgs) -> Result<JsonValue, FunctionCallError> {
    let agent_id = required_non_empty_text(
        args.agent_id.as_deref(),
        "agent_id must not be empty / agent_id 不能为空",
    )?;
    let mut payload = JsonMap::new();
    payload.insert("agent_ids".to_string(), json!([agent_id]));
    let timeout_ms = if args.block { args.timeout } else { Some(0) };
    if let Some(timeout_ms) = timeout_ms {
        if args.block && timeout_ms < 0 {
            return Err(FunctionCallError::RespondToModel(
                "timeout must be greater than or equal to zero / timeout 必须大于等于 0"
                    .to_string(),
            ));
        }
        payload.insert("timeout_ms".to_string(), json!(timeout_ms));
    }
    Ok(JsonValue::Object(payload))
}

fn map_task_stop_to_close_payload(args: TaskStopArgs) -> Result<JsonValue, FunctionCallError> {
    let agent_id = required_non_empty_text(
        args.agent_id.as_deref(),
        "agent_id must not be empty / agent_id 不能为空",
    )?;
    Ok(json!({ "agent_id": agent_id }))
}

fn map_tool_search_payload(args: ToolSearchArgs) -> Result<JsonValue, FunctionCallError> {
    let query = required_non_empty_text(
        args.query.as_deref(),
        "query must not be empty / query 不能为空",
    )?;
    let mut payload = JsonMap::new();
    payload.insert("query".to_string(), JsonValue::String(query));
    if let Some(limit) = args.max_results {
        if limit == 0 {
            return Err(FunctionCallError::RespondToModel(
                "max_results must be greater than zero / max_results 必须大于 0".to_string(),
            ));
        }
        payload.insert("limit".to_string(), json!(limit));
    }
    Ok(JsonValue::Object(payload))
}

fn map_skill_to_spawn_payload(args: SkillArgs) -> Result<JsonValue, FunctionCallError> {
    let skill = required_non_empty_text(
        args.skill.as_deref(),
        "skill must not be empty / skill 不能为空",
    )?;

    let mut items = vec![json!({
        "type": "skill",
        "name": skill,
        "path": format!("skill://{skill}"),
    })];

    if let Some(text) = normalize_text(args.args.as_deref()) {
        items.push(json!({ "type": "text", "text": text }));
    }

    Ok(json!({
        "items": items,
        "name": format!("skill:{skill}"),
    }))
}

fn map_ask_user_question_to_request_payload(
    args: AskUserQuestionArgs,
) -> Result<JsonValue, FunctionCallError> {
    let questions = args.questions.ok_or_else(|| {
        FunctionCallError::RespondToModel(
            "questions must not be empty / questions 不能为空".to_string(),
        )
    })?;
    if questions.is_empty() {
        return Err(FunctionCallError::RespondToModel(
            "questions must not be empty / questions 不能为空".to_string(),
        ));
    }

    let mapped_questions = questions
        .into_iter()
        .enumerate()
        .map(|(index, question)| map_single_question(question, index))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(json!({ "questions": mapped_questions }))
}

fn map_single_question(
    question: AskUserQuestion,
    index: usize,
) -> Result<JsonValue, FunctionCallError> {
    let AskUserQuestion {
        id,
        header,
        question,
        multi_select,
        options,
    } = question;
    let _ignore_multi_select = multi_select;

    let header = required_non_empty_text(
        header.as_deref(),
        "question.header must not be empty / question.header 不能为空",
    )?;
    let question_text = required_non_empty_text(
        question.as_deref(),
        "question.question must not be empty / question.question 不能为空",
    )?;
    let options = options.ok_or_else(|| {
        FunctionCallError::RespondToModel(
            "question.options must not be empty / question.options 不能为空".to_string(),
        )
    })?;
    if options.is_empty() {
        return Err(FunctionCallError::RespondToModel(
            "question.options must not be empty / question.options 不能为空".to_string(),
        ));
    }

    let mapped_options = options
        .into_iter()
        .map(|option| {
            let label = required_non_empty_text(
                option.label.as_deref(),
                "option.label must not be empty / option.label 不能为空",
            )?;
            let description = required_non_empty_text(
                option.description.as_deref(),
                "option.description must not be empty / option.description 不能为空",
            )?;
            Ok(json!({
                "label": label,
                "description": description,
            }))
        })
        .collect::<Result<Vec<_>, FunctionCallError>>()?;
    let question_id = normalize_text(id.as_deref())
        .or_else(|| slugify_to_identifier(Some(&header)))
        .unwrap_or_else(|| format!("question_{}", index + 1));

    Ok(json!({
        "id": question_id,
        "header": header,
        "question": question_text,
        "options": mapped_options,
    }))
}

fn map_bash_to_exec_payload(args: BashArgs) -> Result<JsonValue, FunctionCallError> {
    let command = required_non_empty_text(
        args.command.as_deref(),
        "command must not be empty / command 不能为空",
    )?;
    if let Some(timeout) = args.timeout
        && timeout < 0
    {
        return Err(FunctionCallError::RespondToModel(
            "timeout must be greater than or equal to zero / timeout 必须大于等于 0".to_string(),
        ));
    }

    let mut payload = JsonMap::new();
    payload.insert("cmd".to_string(), JsonValue::String(command));
    if let Some(timeout) = args.timeout {
        payload.insert("yield_time_ms".to_string(), json!(timeout));
    }
    if let Some(description) = normalize_text(args.description.as_deref()) {
        payload.insert("description".to_string(), JsonValue::String(description));
    }
    if let Some(run_in_background) = args.run_in_background {
        payload.insert(
            "run_in_background".to_string(),
            JsonValue::Bool(run_in_background),
        );
    }
    Ok(JsonValue::Object(payload))
}

fn map_read_to_read_file_payload(args: ReadArgs) -> Result<JsonValue, FunctionCallError> {
    if args.pages.is_some() {
        return Err(FunctionCallError::RespondToModel(
            "Read.pages is not supported yet / 暂不支持 Read.pages".to_string(),
        ));
    }
    if let Some(offset) = args.offset
        && offset == 0
    {
        return Err(FunctionCallError::RespondToModel(
            "offset must be greater than zero / offset 必须大于 0".to_string(),
        ));
    }
    if let Some(limit) = args.limit
        && limit == 0
    {
        return Err(FunctionCallError::RespondToModel(
            "limit must be greater than zero / limit 必须大于 0".to_string(),
        ));
    }

    let file_path = required_non_empty_text(
        args.file_path.as_deref(),
        "file_path must not be empty / file_path 不能为空",
    )?;
    let mut payload = JsonMap::new();
    payload.insert("file_path".to_string(), JsonValue::String(file_path));
    if let Some(offset) = args.offset {
        payload.insert("offset".to_string(), json!(offset));
    }
    if let Some(limit) = args.limit {
        payload.insert("limit".to_string(), json!(limit));
    }
    if let Some(mode) = normalize_text(args.mode.as_deref()) {
        payload.insert("mode".to_string(), JsonValue::String(mode));
    }
    if let Some(indentation) = args.indentation {
        payload.insert("indentation".to_string(), indentation);
    }
    Ok(JsonValue::Object(payload))
}

fn map_grep_to_grep_files_payload(args: GrepArgs) -> Result<JsonValue, FunctionCallError> {
    if let Some(output_mode) = normalize_text(args.output_mode.as_deref())
        && output_mode != "files_with_matches"
    {
        return Err(FunctionCallError::RespondToModel(
            "Grep.output_mode only supports files_with_matches / Grep.output_mode 仅支持 files_with_matches".to_string(),
        ));
    }
    if let Some(offset) = args.offset
        && offset > 0
    {
        return Err(FunctionCallError::RespondToModel(
            "Grep.offset is not supported / Grep.offset 暂不支持".to_string(),
        ));
    }

    let pattern = required_non_empty_text(
        args.pattern.as_deref(),
        "pattern must not be empty / pattern 不能为空",
    )?;
    let mut payload = JsonMap::new();
    payload.insert("pattern".to_string(), JsonValue::String(pattern));
    if let Some(path) = normalize_text(args.path.as_deref()) {
        payload.insert("path".to_string(), JsonValue::String(path));
    }
    if let Some(include) = normalize_text(args.glob.as_deref()) {
        payload.insert("include".to_string(), JsonValue::String(include));
    }
    if let Some(limit) = args.head_limit {
        payload.insert("limit".to_string(), json!(limit));
    }
    Ok(JsonValue::Object(payload))
}

fn map_todo_write_to_update_plan_payload(
    args: TodoWriteArgs,
) -> Result<JsonValue, FunctionCallError> {
    let todos = args.todos.ok_or_else(|| {
        FunctionCallError::RespondToModel("todos must not be empty / todos 不能为空".to_string())
    })?;
    if todos.is_empty() {
        return Err(FunctionCallError::RespondToModel(
            "todos must not be empty / todos 不能为空".to_string(),
        ));
    }

    let mut mapped_plan = Vec::new();
    for todo in todos {
        let step = normalize_text(todo.content.as_deref())
            .or_else(|| normalize_text(todo.active_form.as_deref()))
            .ok_or_else(|| {
                FunctionCallError::RespondToModel(
                    "todo.content or todo.activeForm must not be empty / todo.content 或 todo.activeForm 不能为空".to_string(),
                )
            })?;
        let status = normalize_text(todo.status.as_deref())
            .unwrap_or_else(|| "pending".to_string())
            .to_lowercase();
        if !matches!(status.as_str(), "pending" | "in_progress" | "completed") {
            return Err(FunctionCallError::RespondToModel(format!(
                "unsupported todo status `{status}`; expected pending|in_progress|completed / 不支持的 todo status `{status}`，可选值 pending|in_progress|completed"
            )));
        }
        mapped_plan.push(json!({
            "step": step,
            "status": status,
        }));
    }

    Ok(json!({ "plan": mapped_plan }))
}

fn required_non_empty_text(
    value: Option<&str>,
    error_message: &'static str,
) -> Result<String, FunctionCallError> {
    normalize_text(value)
        .ok_or_else(|| FunctionCallError::RespondToModel(error_message.to_string()))
}

fn normalize_text(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn slugify_to_identifier(value: Option<&str>) -> Option<String> {
    let text = normalize_text(value)?;
    let mut out = String::new();
    let mut previous_was_sep = false;
    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            previous_was_sep = false;
        } else if !previous_was_sep {
            out.push('_');
            previous_was_sep = true;
        }
    }
    let trimmed = out.trim_matches('_');
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn is_supported_agent_type(value: &str) -> bool {
    matches!(value, "default" | "worker" | "explorer" | "orchestrator")
}

async fn dispatch_to_collab(
    invocation: ToolInvocation,
    target_tool_name: &str,
    mapped_arguments: JsonValue,
) -> Result<ToolOutput, FunctionCallError> {
    let ToolInvocation {
        session,
        turn,
        tracker,
        call_id,
        ..
    } = invocation;
    let arguments = serde_json::to_string(&mapped_arguments).map_err(|err| {
        FunctionCallError::Fatal(format!(
            "failed to serialize {target_tool_name} alias arguments: {err}"
        ))
    })?;

    CollabHandler
        .handle(ToolInvocation {
            session,
            turn,
            tracker,
            call_id,
            tool_name: target_tool_name.to_string(),
            payload: ToolPayload::Function { arguments },
        })
        .await
}

async fn dispatch_to_search_tool(
    invocation: ToolInvocation,
    mapped_arguments: JsonValue,
) -> Result<ToolOutput, FunctionCallError> {
    let ToolInvocation {
        session,
        turn,
        tracker,
        call_id,
        ..
    } = invocation;
    let arguments = serde_json::to_string(&mapped_arguments).map_err(|err| {
        FunctionCallError::Fatal(format!(
            "failed to serialize search_tool_bm25 alias arguments: {err}"
        ))
    })?;

    SearchToolBm25Handler
        .handle(ToolInvocation {
            session,
            turn,
            tracker,
            call_id,
            tool_name: "search_tool_bm25".to_string(),
            payload: ToolPayload::Function { arguments },
        })
        .await
}

async fn dispatch_to_exec_command(
    invocation: ToolInvocation,
    mapped_arguments: JsonValue,
) -> Result<ToolOutput, FunctionCallError> {
    let ToolInvocation {
        session,
        turn,
        tracker,
        call_id,
        ..
    } = invocation;
    let arguments = serde_json::to_string(&mapped_arguments).map_err(|err| {
        FunctionCallError::Fatal(format!(
            "failed to serialize exec_command alias arguments: {err}"
        ))
    })?;

    UnifiedExecHandler
        .handle(ToolInvocation {
            session,
            turn,
            tracker,
            call_id,
            tool_name: "exec_command".to_string(),
            payload: ToolPayload::Function { arguments },
        })
        .await
}

async fn dispatch_to_read_file(
    invocation: ToolInvocation,
    mapped_arguments: JsonValue,
) -> Result<ToolOutput, FunctionCallError> {
    let ToolInvocation {
        session,
        turn,
        tracker,
        call_id,
        ..
    } = invocation;
    let arguments = serde_json::to_string(&mapped_arguments).map_err(|err| {
        FunctionCallError::Fatal(format!(
            "failed to serialize read_file alias arguments: {err}"
        ))
    })?;

    ReadFileHandler
        .handle(ToolInvocation {
            session,
            turn,
            tracker,
            call_id,
            tool_name: "read_file".to_string(),
            payload: ToolPayload::Function { arguments },
        })
        .await
}

async fn dispatch_to_grep_files(
    invocation: ToolInvocation,
    mapped_arguments: JsonValue,
) -> Result<ToolOutput, FunctionCallError> {
    let ToolInvocation {
        session,
        turn,
        tracker,
        call_id,
        ..
    } = invocation;
    let arguments = serde_json::to_string(&mapped_arguments).map_err(|err| {
        FunctionCallError::Fatal(format!(
            "failed to serialize grep_files alias arguments: {err}"
        ))
    })?;

    GrepFilesHandler
        .handle(ToolInvocation {
            session,
            turn,
            tracker,
            call_id,
            tool_name: "grep_files".to_string(),
            payload: ToolPayload::Function { arguments },
        })
        .await
}

async fn dispatch_to_request_user_input(
    invocation: ToolInvocation,
    mapped_arguments: JsonValue,
) -> Result<ToolOutput, FunctionCallError> {
    let ToolInvocation {
        session,
        turn,
        tracker,
        call_id,
        ..
    } = invocation;
    let arguments = serde_json::to_string(&mapped_arguments).map_err(|err| {
        FunctionCallError::Fatal(format!(
            "failed to serialize request_user_input alias arguments: {err}"
        ))
    })?;

    RequestUserInputHandler
        .handle(ToolInvocation {
            session,
            turn,
            tracker,
            call_id,
            tool_name: "request_user_input".to_string(),
            payload: ToolPayload::Function { arguments },
        })
        .await
}

async fn dispatch_to_update_plan(
    invocation: ToolInvocation,
    mapped_arguments: JsonValue,
) -> Result<ToolOutput, FunctionCallError> {
    let ToolInvocation {
        session,
        turn,
        tracker,
        call_id,
        ..
    } = invocation;
    let arguments = serde_json::to_string(&mapped_arguments).map_err(|err| {
        FunctionCallError::Fatal(format!(
            "failed to serialize update_plan alias arguments: {err}"
        ))
    })?;

    PlanHandler
        .handle(ToolInvocation {
            session,
            turn,
            tracker,
            call_id,
            tool_name: "update_plan".to_string(),
            payload: ToolPayload::Function { arguments },
        })
        .await
}

async fn switch_collaboration_mode(
    invocation: ToolInvocation,
    target_mode: ModeKind,
) -> Result<ToolOutput, FunctionCallError> {
    let session = invocation.session;
    let previous = session.collaboration_mode().await;
    let mut changed = false;
    if previous.mode != target_mode {
        let next_mode = CollaborationMode {
            mode: target_mode,
            settings: previous.settings.clone(),
        };
        session
            .update_settings(SessionSettingsUpdate {
                collaboration_mode: Some(next_mode),
                ..Default::default()
            })
            .await
            .map_err(|err| {
                FunctionCallError::RespondToModel(format!(
                    "failed to switch collaboration mode: {err} / 切换协作模式失败: {err}"
                ))
            })?;
        changed = true;
    }
    let current = session.collaboration_mode().await;
    if current.mode != target_mode {
        return Err(FunctionCallError::RespondToModel(format!(
            "failed to reach target mode `{}` / 未能切换到目标模式 `{}`",
            target_mode.display_name(),
            target_mode.display_name()
        )));
    }

    Ok(ToolOutput::Function {
        body: FunctionCallOutputBody::Text(
            json!({
                "changed": changed,
                "previous_mode": previous.mode.display_name(),
                "current_mode": current.mode.display_name(),
            })
            .to_string(),
        ),
        success: Some(true),
    })
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
    use codex_protocol::models::FunctionCallOutputBody;
    use pretty_assertions::assert_eq;
    use serde_json::Value as JsonValue;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::sync::Mutex;
    use tokio::time::timeout;

    fn invocation(
        session: Arc<crate::codex::Session>,
        turn: Arc<crate::codex::TurnContext>,
        tool_name: &str,
        arguments: JsonValue,
    ) -> ToolInvocation {
        ToolInvocation {
            session,
            turn,
            tracker: Arc::new(Mutex::new(TurnDiffTracker::default())),
            call_id: "call-1".to_string(),
            tool_name: tool_name.to_string(),
            payload: ToolPayload::Function {
                arguments: arguments.to_string(),
            },
        }
    }

    fn thread_manager() -> ThreadManager {
        ThreadManager::with_models_provider(
            CodexAuth::from_api_key("dummy"),
            built_in_model_providers()["openai"].clone(),
        )
    }

    #[test]
    fn task_output_non_blocking_maps_to_zero_timeout() {
        let payload = map_task_output_to_wait_payload(TaskOutputArgs {
            agent_id: Some("agent-1".to_string()),
            block: false,
            timeout: Some(5000),
        })
        .expect("payload should be valid");

        assert_eq!(
            payload,
            json!({
                "agent_ids": ["agent-1"],
                "timeout_ms": 0,
            })
        );
    }

    #[test]
    fn task_maps_supported_agent_type_and_name() {
        let payload = map_task_to_spawn_payload(TaskArgs {
            description: Some("Investigate failing test".to_string()),
            prompt: Some("Check latest regression".to_string()),
            subagent_type: Some("explorer".to_string()),
            max_turns: None,
            mode: None,
            model: Some("gpt-5.1-codex-mini".to_string()),
            preset: Some("run".to_string()),
            name: None,
            resume: None,
            run_in_background: None,
            team_name: None,
        })
        .expect("payload should be valid");

        assert_eq!(
            payload,
            json!({
                "items": [{"type": "text", "text": "Check latest regression"}],
                "agent_type": "explorer",
                "name": "Investigate failing test",
                "model": "gpt-5.1-codex-mini",
                "preset": "run",
            })
        );
    }

    #[test]
    fn tool_search_maps_max_results_to_limit() {
        let payload = map_tool_search_payload(ToolSearchArgs {
            query: Some("slack send".to_string()),
            max_results: Some(3),
        })
        .expect("payload should be valid");

        assert_eq!(
            payload,
            json!({
                "query": "slack send",
                "limit": 3,
            })
        );
    }

    #[test]
    fn ask_user_question_maps_to_request_user_input_payload() {
        let payload = map_ask_user_question_to_request_payload(AskUserQuestionArgs {
            questions: Some(vec![AskUserQuestion {
                id: None,
                header: Some("Mode".to_string()),
                question: Some("Pick one?".to_string()),
                multi_select: Some(false),
                options: Some(vec![
                    AskUserQuestionOption {
                        label: Some("A".to_string()),
                        description: Some("opt a".to_string()),
                    },
                    AskUserQuestionOption {
                        label: Some("B".to_string()),
                        description: Some("opt b".to_string()),
                    },
                ]),
            }]),
        })
        .expect("payload should map");

        assert_eq!(
            payload,
            json!({
                "questions": [{
                    "id": "mode",
                    "header": "Mode",
                    "question": "Pick one?",
                    "options": [
                        {"label": "A", "description": "opt a"},
                        {"label": "B", "description": "opt b"}
                    ],
                }]
            })
        );
    }

    #[test]
    fn bash_maps_command_and_timeout() {
        let payload = map_bash_to_exec_payload(BashArgs {
            command: Some("ls -la".to_string()),
            timeout: Some(1234),
            description: Some("List files".to_string()),
            run_in_background: Some(true),
        })
        .expect("payload should map");

        assert_eq!(
            payload,
            json!({
                "cmd": "ls -la",
                "yield_time_ms": 1234,
                "description": "List files",
                "run_in_background": true
            })
        );
    }

    #[test]
    fn todo_write_maps_to_update_plan_schema() {
        let payload = map_todo_write_to_update_plan_payload(TodoWriteArgs {
            todos: Some(vec![TodoEntry {
                content: Some("Implement feature".to_string()),
                active_form: Some("Implementing feature".to_string()),
                status: Some("in_progress".to_string()),
            }]),
        })
        .expect("payload should map");

        assert_eq!(
            payload,
            json!({
                "plan": [{
                    "step": "Implement feature",
                    "status": "in_progress",
                }]
            })
        );
    }

    #[tokio::test]
    async fn enter_exit_plan_mode_switches_collaboration_mode() {
        let (session, turn) = make_session_and_context().await;
        let session = Arc::new(session);
        let turn = Arc::new(turn);

        let enter = invocation(
            session.clone(),
            turn.clone(),
            ENTER_PLAN_MODE_TOOL_NAME,
            json!({}),
        );
        ClaudeToolAdapterHandler
            .handle(enter)
            .await
            .expect("enter should succeed");
        assert_eq!(session.collaboration_mode().await.mode, ModeKind::Plan);

        let exit = invocation(session.clone(), turn, EXIT_PLAN_MODE_TOOL_NAME, json!({}));
        ClaudeToolAdapterHandler
            .handle(exit)
            .await
            .expect("exit should succeed");
        assert_eq!(session.collaboration_mode().await.mode, ModeKind::Default);
    }

    #[test]
    fn skill_maps_to_spawnable_skill_item() {
        let payload = map_skill_to_spawn_payload(SkillArgs {
            skill: Some("review-pr".to_string()),
            args: Some("123".to_string()),
        })
        .expect("skill payload should be valid");

        assert_eq!(
            payload,
            json!({
                "items": [
                    {
                        "type": "skill",
                        "name": "review-pr",
                        "path": "skill://review-pr",
                    },
                    {
                        "type": "text",
                        "text": "123",
                    }
                ],
                "name": "skill:review-pr",
            })
        );
    }

    #[test]
    fn task_output_rejects_missing_agent_id() {
        let err = map_task_output_to_wait_payload(TaskOutputArgs {
            agent_id: None,
            block: true,
            timeout: None,
        })
        .expect_err("missing agent_id should fail");

        assert_eq!(
            err,
            FunctionCallError::RespondToModel(
                "agent_id must not be empty / agent_id 不能为空".to_string()
            )
        );
    }

    #[test]
    fn task_output_rejects_negative_blocking_timeout() {
        let err = map_task_output_to_wait_payload(TaskOutputArgs {
            agent_id: Some("agent-1".to_string()),
            block: true,
            timeout: Some(-1),
        })
        .expect_err("negative timeout should fail");

        assert_eq!(
            err,
            FunctionCallError::RespondToModel(
                "timeout must be greater than or equal to zero / timeout 必须大于等于 0"
                    .to_string()
            )
        );
    }

    #[test]
    fn task_output_rejects_legacy_task_id_argument() {
        let args: TaskOutputArgs = parse_arguments(
            &json!({
                "task_id": "agent-1",
                "block": true
            })
            .to_string(),
        )
        .expect("legacy payload should deserialize");
        let err = map_task_output_to_wait_payload(args).expect_err("legacy task_id should fail");

        assert_eq!(
            err,
            FunctionCallError::RespondToModel(
                "agent_id must not be empty / agent_id 不能为空".to_string()
            )
        );
    }

    #[test]
    fn task_stop_maps_agent_id_to_close_payload() {
        let payload = map_task_stop_to_close_payload(TaskStopArgs {
            agent_id: Some("agent-2".to_string()),
        })
        .expect("agent_id should map");

        assert_eq!(payload, json!({ "agent_id": "agent-2" }));
    }

    #[test]
    fn task_stop_rejects_legacy_shell_id_argument() {
        let args: TaskStopArgs = parse_arguments(
            &json!({
                "shell_id": "agent-2"
            })
            .to_string(),
        )
        .expect("legacy payload should deserialize");
        let err = map_task_stop_to_close_payload(args).expect_err("legacy shell_id should fail");

        assert_eq!(
            err,
            FunctionCallError::RespondToModel(
                "agent_id must not be empty / agent_id 不能为空".to_string()
            )
        );
    }

    #[test]
    fn tool_search_rejects_zero_max_results() {
        let err = map_tool_search_payload(ToolSearchArgs {
            query: Some("list".to_string()),
            max_results: Some(0),
        })
        .expect_err("max_results=0 should fail");

        assert_eq!(
            err,
            FunctionCallError::RespondToModel(
                "max_results must be greater than zero / max_results 必须大于 0".to_string()
            )
        );
    }

    #[tokio::test]
    async fn task_output_block_false_is_non_blocking_end_to_end() {
        let (mut session, turn) = make_session_and_context().await;
        let manager = thread_manager();
        session.services.agent_control = manager.agent_control();
        let config = turn.config.as_ref().clone();
        let thread = manager.start_thread(config).await.expect("start thread");
        let agent_id = thread.thread_id.to_string();

        let invocation = invocation(
            Arc::new(session),
            Arc::new(turn),
            TASK_OUTPUT_TOOL_NAME,
            json!({
                "agent_id": agent_id,
                "block": false,
                "timeout": 5000
            }),
        );

        let output = timeout(
            Duration::from_millis(500),
            ClaudeToolAdapterHandler.handle(invocation),
        )
        .await
        .expect("TaskOutput block=false should return quickly")
        .expect("TaskOutput alias should succeed");

        let ToolOutput::Function {
            body: FunctionCallOutputBody::Text(content),
            success,
            ..
        } = output
        else {
            panic!("expected function output");
        };
        assert_eq!(success, None);

        let result: JsonValue =
            serde_json::from_str(&content).expect("TaskOutput result should be valid json");
        assert_eq!(result.get("timed_out"), Some(&JsonValue::Bool(true)));
        assert_eq!(
            result.get("wakeup_reason"),
            Some(&JsonValue::String("timeout".to_string()))
        );

        let _ = thread
            .thread
            .submit(Op::Shutdown {})
            .await
            .expect("shutdown should submit");
    }
}
