use crate::agent::AgentRole;
use crate::client_common::tools::ResponsesApiTool;
use crate::client_common::tools::ToolSpec;
use crate::features::Feature;
use crate::features::Features;
use crate::tools::handlers::PLAN_TOOL;
use crate::tools::handlers::SEARCH_TOOL_BM25_DEFAULT_LIMIT;
use crate::tools::handlers::apply_patch::create_apply_patch_freeform_tool;
use crate::tools::handlers::apply_patch::create_apply_patch_json_tool;
use crate::tools::handlers::collab::DEFAULT_WAIT_TIMEOUT_MS;
use crate::tools::handlers::collab::MAX_WAIT_TIMEOUT_MS;
use crate::tools::handlers::request_user_input_tool_description;
use crate::tools::registry::ToolRegistryBuilder;
use codex_protocol::config_types::WebSearchMode;
use codex_protocol::dynamic_tools::DynamicToolSpec;
use codex_protocol::models::VIEW_IMAGE_TOOL_NAME;
use codex_protocol::openai_models::ApplyPatchToolType;
use codex_protocol::openai_models::ConfigShellToolType;
use codex_protocol::openai_models::ModelInfo;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value as JsonValue;
use serde_json::json;
use std::collections::BTreeMap;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub(crate) struct ToolsConfig {
    pub shell_type: ConfigShellToolType,
    pub apply_patch_tool_type: Option<ApplyPatchToolType>,
    pub web_search_mode: Option<WebSearchMode>,
    pub search_tool: bool,
    pub collab_tools: bool,
    pub collaboration_modes_tools: bool,
    pub request_rule_enabled: bool,
    pub experimental_supported_tools: Vec<String>,
}

pub(crate) struct ToolsConfigParams<'a> {
    pub(crate) model_info: &'a ModelInfo,
    pub(crate) features: &'a Features,
    pub(crate) web_search_mode: Option<WebSearchMode>,
}

impl ToolsConfig {
    pub fn new(params: &ToolsConfigParams) -> Self {
        let ToolsConfigParams {
            model_info,
            features,
            web_search_mode,
        } = params;
        let include_apply_patch_tool = features.enabled(Feature::ApplyPatchFreeform);
        let include_collab_tools = features.enabled(Feature::Collab);
        let include_collaboration_modes_tools = features.enabled(Feature::CollaborationModes);
        let request_rule_enabled = features.enabled(Feature::RequestRule);
        let include_search_tool = features.enabled(Feature::SearchTool);

        let shell_type = if !features.enabled(Feature::ShellTool) {
            ConfigShellToolType::Disabled
        } else if features.enabled(Feature::UnifiedExec) {
            // If ConPTY not supported (for old Windows versions), fallback on ShellCommand.
            if codex_utils_pty::conpty_supported() {
                ConfigShellToolType::UnifiedExec
            } else {
                ConfigShellToolType::ShellCommand
            }
        } else {
            model_info.shell_type
        };

        let apply_patch_tool_type = match model_info.apply_patch_tool_type {
            Some(ApplyPatchToolType::Freeform) => Some(ApplyPatchToolType::Freeform),
            Some(ApplyPatchToolType::Function) => Some(ApplyPatchToolType::Function),
            None => {
                if include_apply_patch_tool {
                    Some(ApplyPatchToolType::Freeform)
                } else {
                    None
                }
            }
        };

        Self {
            shell_type,
            apply_patch_tool_type,
            web_search_mode: *web_search_mode,
            search_tool: include_search_tool,
            collab_tools: include_collab_tools,
            collaboration_modes_tools: include_collaboration_modes_tools,
            request_rule_enabled,
            experimental_supported_tools: model_info.experimental_supported_tools.clone(),
        }
    }
}

/// Generic JSON‑Schema subset needed for our tool definitions
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum JsonSchema {
    Boolean {
        #[serde(skip_serializing_if = "Option::is_none")]
        description: Option<String>,
    },
    String {
        #[serde(skip_serializing_if = "Option::is_none")]
        description: Option<String>,
    },
    /// MCP schema allows "number" | "integer" for Number
    #[serde(alias = "integer")]
    Number {
        #[serde(skip_serializing_if = "Option::is_none")]
        description: Option<String>,
    },
    Array {
        items: Box<JsonSchema>,

        #[serde(skip_serializing_if = "Option::is_none")]
        description: Option<String>,
    },
    Object {
        properties: BTreeMap<String, JsonSchema>,
        #[serde(skip_serializing_if = "Option::is_none")]
        required: Option<Vec<String>>,
        #[serde(
            rename = "additionalProperties",
            skip_serializing_if = "Option::is_none"
        )]
        additional_properties: Option<AdditionalProperties>,
    },
}

/// Whether additional properties are allowed, and if so, any required schema
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum AdditionalProperties {
    Boolean(bool),
    Schema(Box<JsonSchema>),
}

impl From<bool> for AdditionalProperties {
    fn from(b: bool) -> Self {
        Self::Boolean(b)
    }
}

impl From<JsonSchema> for AdditionalProperties {
    fn from(s: JsonSchema) -> Self {
        Self::Schema(Box::new(s))
    }
}

fn create_approval_parameters(include_prefix_rule: bool) -> BTreeMap<String, JsonSchema> {
    let mut properties = BTreeMap::from([
        (
            "sandbox_permissions".to_string(),
            JsonSchema::String {
                description: Some(
                    "Sandbox permissions for the command. Set to \"require_escalated\" to request running without sandbox restrictions; defaults to \"use_default\"."
                        .to_string(),
                ),
            },
        ),
        (
            "justification".to_string(),
            JsonSchema::String {
                description: Some(
                    r#"Only set if sandbox_permissions is \"require_escalated\". 
                    Request approval from the user to run this command outside the sandbox. 
                    Phrased as a simple question that summarizes the purpose of the 
                    command as it relates to the task at hand - e.g. 'Do you want to 
                    fetch and pull the latest version of this git branch?'"#
                    .to_string(),
                ),
            },
        ),
    ]);

    if include_prefix_rule {
        properties.insert(
            "prefix_rule".to_string(),
            JsonSchema::Array {
                items: Box::new(JsonSchema::String { description: None }),
                description: Some(
                    r#"Only specify when sandbox_permissions is `require_escalated`. 
                    Suggest a prefix command pattern that will allow you to fulfill similar requests from the user in the future.
                    Should be a short but reasonable prefix, e.g. [\"git\", \"pull\"] or [\"uv\", \"run\"] or [\"pytest\"]."#.to_string(),
                ),
            });
    }

    properties
}

fn create_exec_command_tool(include_prefix_rule: bool) -> ToolSpec {
    let mut properties = BTreeMap::from([
        (
            "cmd".to_string(),
            JsonSchema::String {
                description: Some("Shell command to execute.".to_string()),
            },
        ),
        (
            "workdir".to_string(),
            JsonSchema::String {
                description: Some(
                    "Optional working directory to run the command in; defaults to the turn cwd."
                        .to_string(),
                ),
            },
        ),
        (
            "shell".to_string(),
            JsonSchema::String {
                description: Some("Shell binary to launch. Defaults to the user's default shell.".to_string()),
            },
        ),
        (
            "login".to_string(),
            JsonSchema::Boolean {
                description: Some(
                    "Whether to run the shell with -l/-i semantics. Defaults to true.".to_string(),
                ),
            },
        ),
        (
            "tty".to_string(),
            JsonSchema::Boolean {
                description: Some(
                    "Whether to allocate a TTY for the command. Defaults to false (plain pipes); set to true to open a PTY and access TTY process."
                        .to_string(),
                ),
            }
        ),
        (
            "yield_time_ms".to_string(),
            JsonSchema::Number {
                description: Some(
                    "How long to wait (in milliseconds) for output before yielding.".to_string(),
                ),
            },
        ),
        (
            "max_output_tokens".to_string(),
            JsonSchema::Number {
                description: Some(
                    "Maximum number of tokens to return. Excess output will be truncated."
                        .to_string(),
                ),
            },
        ),
    ]);
    properties.extend(create_approval_parameters(include_prefix_rule));

    ToolSpec::Function(ResponsesApiTool {
        name: "exec_command".to_string(),
        description:
            "Runs a command in a PTY, returning output or a session ID for ongoing interaction."
                .to_string(),
        strict: false,
        parameters: JsonSchema::Object {
            properties,
            required: Some(vec!["cmd".to_string()]),
            additional_properties: Some(false.into()),
        },
    })
}

fn create_write_stdin_tool() -> ToolSpec {
    let properties = BTreeMap::from([
        (
            "session_id".to_string(),
            JsonSchema::Number {
                description: Some("Identifier of the running unified exec session.".to_string()),
            },
        ),
        (
            "chars".to_string(),
            JsonSchema::String {
                description: Some("Bytes to write to stdin (may be empty to poll).".to_string()),
            },
        ),
        (
            "yield_time_ms".to_string(),
            JsonSchema::Number {
                description: Some(
                    "How long to wait (in milliseconds) for output before yielding.".to_string(),
                ),
            },
        ),
        (
            "max_output_tokens".to_string(),
            JsonSchema::Number {
                description: Some(
                    "Maximum number of tokens to return. Excess output will be truncated."
                        .to_string(),
                ),
            },
        ),
    ]);

    ToolSpec::Function(ResponsesApiTool {
        name: "write_stdin".to_string(),
        description:
            "Writes characters to an existing unified exec session and returns recent output."
                .to_string(),
        strict: false,
        parameters: JsonSchema::Object {
            properties,
            required: Some(vec!["session_id".to_string()]),
            additional_properties: Some(false.into()),
        },
    })
}

fn create_shell_tool(include_prefix_rule: bool) -> ToolSpec {
    let mut properties = BTreeMap::from([
        (
            "command".to_string(),
            JsonSchema::Array {
                items: Box::new(JsonSchema::String { description: None }),
                description: Some("The command to execute".to_string()),
            },
        ),
        (
            "workdir".to_string(),
            JsonSchema::String {
                description: Some("The working directory to execute the command in".to_string()),
            },
        ),
        (
            "timeout_ms".to_string(),
            JsonSchema::Number {
                description: Some("The timeout for the command in milliseconds".to_string()),
            },
        ),
    ]);
    properties.extend(create_approval_parameters(include_prefix_rule));

    let description  = if cfg!(windows) {
        r#"Runs a Powershell command (Windows) and returns its output. Arguments to `shell` will be passed to CreateProcessW(). Most commands should be prefixed with ["powershell.exe", "-Command"].
        
Examples of valid command strings:

- ls -a (show hidden): ["powershell.exe", "-Command", "Get-ChildItem -Force"]
- recursive find by name: ["powershell.exe", "-Command", "Get-ChildItem -Recurse -Filter *.py"]
- recursive grep: ["powershell.exe", "-Command", "Get-ChildItem -Path C:\\myrepo -Recurse | Select-String -Pattern 'TODO' -CaseSensitive"]
- ps aux | grep python: ["powershell.exe", "-Command", "Get-Process | Where-Object { $_.ProcessName -like '*python*' }"]
- setting an env var: ["powershell.exe", "-Command", "$env:FOO='bar'; echo $env:FOO"]
- running an inline Python script: ["powershell.exe", "-Command", "@'\\nprint('Hello, world!')\\n'@ | python -"]"#
    } else {
        r#"Runs a shell command and returns its output.
- The arguments to `shell` will be passed to execvp(). Most terminal commands should be prefixed with ["bash", "-lc"].
- Always set the `workdir` param when using the shell function. Do not use `cd` unless absolutely necessary."#
    }.to_string();

    ToolSpec::Function(ResponsesApiTool {
        name: "shell".to_string(),
        description,
        strict: false,
        parameters: JsonSchema::Object {
            properties,
            required: Some(vec!["command".to_string()]),
            additional_properties: Some(false.into()),
        },
    })
}

fn create_shell_command_tool(include_prefix_rule: bool) -> ToolSpec {
    let mut properties = BTreeMap::from([
        (
            "command".to_string(),
            JsonSchema::String {
                description: Some(
                    "The shell script to execute in the user's default shell".to_string(),
                ),
            },
        ),
        (
            "workdir".to_string(),
            JsonSchema::String {
                description: Some("The working directory to execute the command in".to_string()),
            },
        ),
        (
            "login".to_string(),
            JsonSchema::Boolean {
                description: Some(
                    "Whether to run the shell with login shell semantics. Defaults to true."
                        .to_string(),
                ),
            },
        ),
        (
            "timeout_ms".to_string(),
            JsonSchema::Number {
                description: Some("The timeout for the command in milliseconds".to_string()),
            },
        ),
    ]);
    properties.extend(create_approval_parameters(include_prefix_rule));

    let description = if cfg!(windows) {
        r#"Runs a Powershell command (Windows) and returns its output.
        
Examples of valid command strings:

- ls -a (show hidden): "Get-ChildItem -Force"
- recursive find by name: "Get-ChildItem -Recurse -Filter *.py"
- recursive grep: "Get-ChildItem -Path C:\\myrepo -Recurse | Select-String -Pattern 'TODO' -CaseSensitive"
- ps aux | grep python: "Get-Process | Where-Object { $_.ProcessName -like '*python*' }"
- setting an env var: "$env:FOO='bar'; echo $env:FOO"
- running an inline Python script: "@'\\nprint('Hello, world!')\\n'@ | python -"#
    } else {
        r#"Runs a shell command and returns its output.
- Always set the `workdir` param when using the shell_command function. Do not use `cd` unless absolutely necessary."#
    }.to_string();

    ToolSpec::Function(ResponsesApiTool {
        name: "shell_command".to_string(),
        description,
        strict: false,
        parameters: JsonSchema::Object {
            properties,
            required: Some(vec!["command".to_string()]),
            additional_properties: Some(false.into()),
        },
    })
}

fn create_view_image_tool() -> ToolSpec {
    // Support only local filesystem path.
    let properties = BTreeMap::from([(
        "path".to_string(),
        JsonSchema::String {
            description: Some("Local filesystem path to an image file".to_string()),
        },
    )]);

    ToolSpec::Function(ResponsesApiTool {
        name: VIEW_IMAGE_TOOL_NAME.to_string(),
        description: "View a local image from the filesystem (only use if given a full filepath by the user, and the image isn't already attached to the thread context within <image ...> tags)."
            .to_string(),
        strict: false,
        parameters: JsonSchema::Object {
            properties,
            required: Some(vec!["path".to_string()]),
            additional_properties: Some(false.into()),
        },
    })
}

fn create_collab_input_items_schema() -> JsonSchema {
    let properties = BTreeMap::from([
        (
            "type".to_string(),
            JsonSchema::String {
                description: Some(
                    "Input item type: text, image, local_image, skill, or mention.".to_string(),
                ),
            },
        ),
        (
            "text".to_string(),
            JsonSchema::String {
                description: Some("Text content when type is text.".to_string()),
            },
        ),
        (
            "image_url".to_string(),
            JsonSchema::String {
                description: Some("Image URL when type is image.".to_string()),
            },
        ),
        (
            "path".to_string(),
            JsonSchema::String {
                description: Some(
                    "Path when type is local_image/skill, or mention target such as app://<connector-id> when type is mention."
                        .to_string(),
                ),
            },
        ),
        (
            "name".to_string(),
            JsonSchema::String {
                description: Some("Display name when type is skill or mention.".to_string()),
            },
        ),
    ]);

    JsonSchema::Array {
        items: Box::new(JsonSchema::Object {
            properties,
            required: None,
            additional_properties: Some(false.into()),
        }),
        description: Some(
            "Structured input items. Use this to pass explicit mentions (for example app:// connector paths)."
                .to_string(),
        ),
    }
}

fn create_spawn_agent_parameters() -> JsonSchema {
    let mut properties = BTreeMap::new();
    properties.insert("items".to_string(), create_collab_input_items_schema());
    properties.insert(
        "agent_type".to_string(),
        JsonSchema::String {
            description: Some(format!(
                "Optional agent type ({}). Use an explicit type when delegating.",
                AgentRole::enum_values().join(", ")
            )),
        },
    );
    properties.insert(
        "name".to_string(),
        JsonSchema::String {
            description: Some(
                "Optional name for the spawned agent / 可选子 Agent 名称".to_string(),
            ),
        },
    );
    properties.insert(
        "preset".to_string(),
        JsonSchema::String {
            description: Some(
                "Optional spawn preset (edit|read|grep|run|websearch|expert) / 可选启动预设（edit|read|grep|run|websearch|expert）"
                    .to_string(),
            ),
        },
    );
    properties.insert(
        "must_call_reason".to_string(),
        JsonSchema::String {
            description: Some(
                "Required when preset=expert. Explain why this high-cost expert call is necessary."
                    .to_string(),
            ),
        },
    );
    properties.insert(
        "acceptance_criteria".to_string(),
        JsonSchema::Array {
            items: Box::new(JsonSchema::String {
                description: Some("Optional acceptance criterion.".to_string()),
            }),
            description: Some("Optional acceptance criteria for the task.".to_string()),
        },
    );
    properties.insert(
        "test_commands".to_string(),
        JsonSchema::Array {
            items: Box::new(JsonSchema::String {
                description: Some("Optional test command.".to_string()),
            }),
            description: Some("Optional validation commands for the spawned task.".to_string()),
        },
    );
    properties.insert(
        "allow_nested_agents".to_string(),
        JsonSchema::Boolean {
            description: Some("Whether the spawned agent may create nested agents.".to_string()),
        },
    );
    properties.insert(
        "model".to_string(),
        JsonSchema::String {
            description: Some("Optional model override for the spawned agent.".to_string()),
        },
    );
    properties.insert(
        "reasoning_effort".to_string(),
        JsonSchema::String {
            description: Some(
                "Optional reasoning effort override (none|minimal|low|medium|high|xhigh)."
                    .to_string(),
            ),
        },
    );
    properties.insert(
        "reasoning_summary".to_string(),
        JsonSchema::String {
            description: Some(
                "Optional reasoning summary override (auto|concise|detailed|none).".to_string(),
            ),
        },
    );
    properties.insert(
        "approval_policy".to_string(),
        JsonSchema::String {
            description: Some(
                "Optional approval policy override (never|on-request|on-failure|untrusted)."
                    .to_string(),
            ),
        },
    );
    properties.insert(
        "sandbox_mode".to_string(),
        JsonSchema::String {
            description: Some(
                "Optional sandbox mode override (read-only|workspace-write|danger-full-access)."
                    .to_string(),
            ),
        },
    );

    JsonSchema::Object {
        properties,
        required: Some(vec!["items".to_string()]),
        additional_properties: Some(false.into()),
    }
}

fn create_spawn_agent_tool() -> ToolSpec {
    ToolSpec::Function(ResponsesApiTool {
        name: "spawn_agent".to_string(),
        description:
            "Spawn a sub-agent for a well-scoped task. Returns the agent id to use to communicate with this agent."
                .to_string(),
        strict: false,
        parameters: create_spawn_agent_parameters(),
    })
}

fn create_send_input_parameters() -> JsonSchema {
    let properties = BTreeMap::from([
        (
            "agent_id".to_string(),
            JsonSchema::String {
                description: Some("Agent id to message (from spawn_agent).".to_string()),
            },
        ),
        ("items".to_string(), create_collab_input_items_schema()),
        (
            "interrupt".to_string(),
            JsonSchema::Boolean {
                description: Some(
                    "When true, stop the agent's current task and handle this immediately. When false (default), queue this message."
                        .to_string(),
                ),
            },
        ),
        (
            "must_call_reason".to_string(),
            JsonSchema::String {
                description: Some(
                    "Required when sending to an expert agent after its 3-round budget is exhausted. Explain why another budget window is necessary."
                        .to_string(),
                ),
            },
        ),
    ]);

    JsonSchema::Object {
        properties,
        required: Some(vec!["agent_id".to_string(), "items".to_string()]),
        additional_properties: Some(false.into()),
    }
}

fn create_send_input_tool() -> ToolSpec {
    ToolSpec::Function(ResponsesApiTool {
        name: "send_input".to_string(),
        description:
            "Send a message to an existing agent. Use interrupt=true to redirect work immediately."
                .to_string(),
        strict: false,
        parameters: create_send_input_parameters(),
    })
}

fn create_collab_batch_tool(
    name: &str,
    description: &str,
    operation_parameters: JsonSchema,
) -> ToolSpec {
    let operation_properties = BTreeMap::from([
        (
            "id".to_string(),
            JsonSchema::String {
                description: Some(
                    "Optional operation id echoed back in the corresponding result entry."
                        .to_string(),
                ),
            },
        ),
        ("params".to_string(), operation_parameters),
    ]);
    let properties = BTreeMap::from([
        (
            "operations".to_string(),
            JsonSchema::Array {
                items: Box::new(JsonSchema::Object {
                    properties: operation_properties,
                    required: Some(vec!["params".to_string()]),
                    additional_properties: Some(false.into()),
                }),
                description: Some(
                    "Batch operation entries. Each entry may include an optional id and required params."
                        .to_string(),
                ),
            },
        ),
        (
            "fail_fast".to_string(),
            JsonSchema::Boolean {
                description: Some(
                    "When true, stop processing after the first failed operation. Default false."
                        .to_string(),
                ),
            },
        ),
    ]);
    ToolSpec::Function(ResponsesApiTool {
        name: name.to_string(),
        description: description.to_string(),
        strict: false,
        parameters: JsonSchema::Object {
            properties,
            required: Some(vec!["operations".to_string()]),
            additional_properties: Some(false.into()),
        },
    })
}

fn create_task_batch_tool() -> ToolSpec {
    create_collab_batch_tool(
        "task_batch",
        "Create multiple agents in one call. Wraps spawn_agent semantics per operation.",
        create_spawn_agent_parameters(),
    )
}

fn create_task_send_batch_tool() -> ToolSpec {
    create_collab_batch_tool(
        "task_send_batch",
        "Send input to multiple agents in one call. Wraps send_input semantics per operation.",
        create_send_input_parameters(),
    )
}

fn create_resume_agent_tool() -> ToolSpec {
    let mut properties = BTreeMap::new();
    properties.insert(
        "agent_id".to_string(),
        JsonSchema::String {
            description: Some("Agent id to resume.".to_string()),
        },
    );

    ToolSpec::Function(ResponsesApiTool {
        name: "resume_agent".to_string(),
        description:
            "Resume a previously closed agent by agent_id so it can receive send_input and wait calls."
                .to_string(),
        strict: false,
        parameters: JsonSchema::Object {
            properties,
            required: Some(vec!["agent_id".to_string()]),
            additional_properties: Some(false.into()),
        },
    })
}

fn create_wait_tool() -> ToolSpec {
    let mut properties = BTreeMap::new();
    properties.insert(
        "agent_ids".to_string(),
        JsonSchema::Array {
            items: Box::new(JsonSchema::String { description: None }),
            description: Some(
                "Agent ids to wait on. Pass multiple ids to wait for whichever finishes first."
                    .to_string(),
            ),
        },
    );
    properties.insert(
        "timeout_ms".to_string(),
        JsonSchema::Number {
            description: Some(format!(
                "Optional timeout in milliseconds. Defaults to session config (fallback {DEFAULT_WAIT_TIMEOUT_MS}) and max {MAX_WAIT_TIMEOUT_MS}."
            )),
        },
    );

    ToolSpec::Function(ResponsesApiTool {
        name: "wait".to_string(),
        description: "Wait for agents to reach a final status. Completed statuses may include the agent's final message. Returns empty status when timed out."
            .to_string(),
        strict: false,
        parameters: JsonSchema::Object {
            properties,
            required: Some(vec!["agent_ids".to_string()]),
            additional_properties: Some(false.into()),
        },
    })
}

fn create_request_user_input_tool() -> ToolSpec {
    let mut option_props = BTreeMap::new();
    option_props.insert(
        "label".to_string(),
        JsonSchema::String {
            description: Some("User-facing label (1-5 words).".to_string()),
        },
    );
    option_props.insert(
        "description".to_string(),
        JsonSchema::String {
            description: Some(
                "One short sentence explaining impact/tradeoff if selected.".to_string(),
            ),
        },
    );

    let options_schema = JsonSchema::Array {
        description: Some(
            "Provide 2-3 mutually exclusive choices. Put the recommended option first and suffix its label with \"(Recommended)\". Do not include an \"Other\" option in this list; the client will add a free-form \"Other\" option automatically."
                .to_string(),
        ),
        items: Box::new(JsonSchema::Object {
            properties: option_props,
            required: Some(vec!["label".to_string(), "description".to_string()]),
            additional_properties: Some(false.into()),
        }),
    };

    let mut question_props = BTreeMap::new();
    question_props.insert(
        "id".to_string(),
        JsonSchema::String {
            description: Some("Stable identifier for mapping answers (snake_case).".to_string()),
        },
    );
    question_props.insert(
        "header".to_string(),
        JsonSchema::String {
            description: Some(
                "Short header label shown in the UI (12 or fewer chars).".to_string(),
            ),
        },
    );
    question_props.insert(
        "question".to_string(),
        JsonSchema::String {
            description: Some("Single-sentence prompt shown to the user.".to_string()),
        },
    );
    question_props.insert("options".to_string(), options_schema);

    let questions_schema = JsonSchema::Array {
        description: Some("Questions to show the user. Prefer 1 and do not exceed 3".to_string()),
        items: Box::new(JsonSchema::Object {
            properties: question_props,
            required: Some(vec![
                "id".to_string(),
                "header".to_string(),
                "question".to_string(),
                "options".to_string(),
            ]),
            additional_properties: Some(false.into()),
        }),
    };

    let mut properties = BTreeMap::new();
    properties.insert("questions".to_string(), questions_schema);

    ToolSpec::Function(ResponsesApiTool {
        name: "request_user_input".to_string(),
        description: request_user_input_tool_description(),
        strict: false,
        parameters: JsonSchema::Object {
            properties,
            required: Some(vec!["questions".to_string()]),
            additional_properties: Some(false.into()),
        },
    })
}

fn create_wait_agents_tool() -> ToolSpec {
    let mut properties = BTreeMap::new();
    properties.insert(
        "agent_ids".to_string(),
        JsonSchema::Array {
            items: Box::new(JsonSchema::String {
                description: Some("Agent id to wait on.".to_string()),
            }),
            description: Some(
                "Optional list of agent ids to wait on. When omitted, waits on active child agents created by the current thread."
                    .to_string(),
            ),
        },
    );
    properties.insert(
        "mode".to_string(),
        JsonSchema::String {
            description: Some("Wait mode: any (default) or all.".to_string()),
        },
    );
    properties.insert(
        "timeout_ms".to_string(),
        JsonSchema::Number {
            description: Some(format!(
                "Optional timeout in milliseconds. Defaults to session config (fallback {DEFAULT_WAIT_TIMEOUT_MS}) and max {MAX_WAIT_TIMEOUT_MS}."
            )),
        },
    );

    ToolSpec::Function(ResponsesApiTool {
        name: "wait_agents".to_string(),
        description: "Wait for one or more agents and return aggregated statuses.".to_string(),
        strict: false,
        parameters: JsonSchema::Object {
            properties,
            required: None,
            additional_properties: Some(false.into()),
        },
    })
}

fn create_list_agents_tool() -> ToolSpec {
    let mut properties = BTreeMap::new();
    properties.insert(
        "agent_id".to_string(),
        JsonSchema::String {
            description: Some("Optional agent id filter.".to_string()),
        },
    );
    properties.insert(
        "statuses".to_string(),
        JsonSchema::Array {
            items: Box::new(JsonSchema::String {
                description: Some(
                    "Optional status filter values: pending_init|running|completed|errored|shutdown|not_found."
                        .to_string(),
                ),
            }),
            description: Some("Optional list of statuses to include.".to_string()),
        },
    );
    properties.insert(
        "include_closed".to_string(),
        JsonSchema::Boolean {
            description: Some("Whether to include closed agents; defaults to false.".to_string()),
        },
    );

    ToolSpec::Function(ResponsesApiTool {
        name: "list_agents".to_string(),
        description: "List known child agents and their metadata.".to_string(),
        strict: false,
        parameters: JsonSchema::Object {
            properties,
            required: None,
            additional_properties: Some(false.into()),
        },
    })
}

fn create_close_agent_tool() -> ToolSpec {
    let mut properties = BTreeMap::new();
    properties.insert(
        "agent_id".to_string(),
        JsonSchema::String {
            description: Some("Agent id to close (from spawn_agent).".to_string()),
        },
    );

    ToolSpec::Function(ResponsesApiTool {
        name: "close_agent".to_string(),
        description: "Close an agent when it is no longer needed and return its last known status."
            .to_string(),
        strict: false,
        parameters: JsonSchema::Object {
            properties,
            required: Some(vec!["agent_id".to_string()]),
            additional_properties: Some(false.into()),
        },
    })
}

fn create_rename_agent_tool() -> ToolSpec {
    let properties = BTreeMap::from([
        (
            "agent_id".to_string(),
            JsonSchema::String {
                description: Some("Agent id to rename (from spawn_agent).".to_string()),
            },
        ),
        (
            "name".to_string(),
            JsonSchema::String {
                description: Some("New agent name / 新的子 Agent 名称".to_string()),
            },
        ),
    ]);

    ToolSpec::Function(ResponsesApiTool {
        name: "rename_agent".to_string(),
        description: "Rename an existing agent by agent_id / 按 agent_id 重命名现有子 Agent"
            .to_string(),
        strict: false,
        parameters: JsonSchema::Object {
            properties,
            required: Some(vec!["agent_id".to_string(), "name".to_string()]),
            additional_properties: Some(false.into()),
        },
    })
}

fn create_close_agents_tool() -> ToolSpec {
    let mut properties = BTreeMap::new();
    properties.insert(
        "agent_ids".to_string(),
        JsonSchema::Array {
            items: Box::new(JsonSchema::String {
                description: Some("Identifier of the agent to close.".to_string()),
            }),
            description: Some("List of agent ids to close.".to_string()),
        },
    );
    properties.insert(
        "ignore_missing".to_string(),
        JsonSchema::Boolean {
            description: Some("When true, missing agents are ignored.".to_string()),
        },
    );

    ToolSpec::Function(ResponsesApiTool {
        name: "close_agents".to_string(),
        description: "Close multiple agents and return per-agent results.".to_string(),
        strict: false,
        parameters: JsonSchema::Object {
            properties,
            required: Some(vec!["agent_ids".to_string()]),
            additional_properties: Some(false.into()),
        },
    })
}

fn create_claude_task_alias_tool() -> ToolSpec {
    let properties = BTreeMap::from([
        (
            "description".to_string(),
            JsonSchema::String {
                description: Some(
                    "Short (3-5 words) summary of the delegated task / 任务简述（3-5 个词）"
                        .to_string(),
                ),
            },
        ),
        (
            "prompt".to_string(),
            JsonSchema::String {
                description: Some(
                    "Detailed prompt for the delegated task / 委派任务的详细说明".to_string(),
                ),
            },
        ),
        (
            "subagent_type".to_string(),
            JsonSchema::String {
                description: Some(
                    "Requested sub-agent type / 请求的子代理类型（phase1 仅透传兼容）".to_string(),
                ),
            },
        ),
        (
            "name".to_string(),
            JsonSchema::String {
                description: Some("Optional delegated task name / 可选任务名称".to_string()),
            },
        ),
        (
            "model".to_string(),
            JsonSchema::String {
                description: Some("Optional model override / 可选模型覆盖".to_string()),
            },
        ),
        (
            "preset".to_string(),
            JsonSchema::String {
                description: Some(
                    "Optional spawn preset forwarded to spawn_agent / 可选 spawn 预设（透传至 spawn_agent）"
                        .to_string(),
                ),
            },
        ),
        (
            "max_turns".to_string(),
            JsonSchema::Number {
                description: Some("Optional max turns / 可选最大轮次".to_string()),
            },
        ),
        (
            "mode".to_string(),
            JsonSchema::String {
                description: Some("Optional mode override / 可选模式覆盖".to_string()),
            },
        ),
        (
            "resume".to_string(),
            JsonSchema::String {
                description: Some("Optional agent id to resume / 可选恢复 agent id".to_string()),
            },
        ),
        (
            "run_in_background".to_string(),
            JsonSchema::Boolean {
                description: Some(
                    "Whether to run the task in background / 是否后台运行".to_string(),
                ),
            },
        ),
        (
            "team_name".to_string(),
            JsonSchema::String {
                description: Some("Optional team name / 可选团队名".to_string()),
            },
        ),
    ]);

    ToolSpec::Function(ResponsesApiTool {
        name: "Task".to_string(),
        description: "Claude-compatible alias for spawn_agent / Claude 兼容的 spawn_agent 别名"
            .to_string(),
        strict: false,
        parameters: JsonSchema::Object {
            properties,
            required: Some(vec![
                "description".to_string(),
                "prompt".to_string(),
                "subagent_type".to_string(),
            ]),
            additional_properties: Some(false.into()),
        },
    })
}

fn create_claude_task_output_alias_tool() -> ToolSpec {
    let properties = BTreeMap::from([
        (
            "agent_id".to_string(),
            JsonSchema::String {
                description: Some("Agent id to inspect / 要查询的 agent id".to_string()),
            },
        ),
        (
            "block".to_string(),
            JsonSchema::Boolean {
                description: Some(
                    "When false, perform a non-blocking poll / 为 false 时执行非阻塞轮询"
                        .to_string(),
                ),
            },
        ),
        (
            "timeout".to_string(),
            JsonSchema::Number {
                description: Some(
                    "Maximum wait time in milliseconds / 最大等待时间（毫秒）".to_string(),
                ),
            },
        ),
    ]);

    ToolSpec::Function(ResponsesApiTool {
        name: "TaskOutput".to_string(),
        description:
            "Claude-compatible alias for wait/wait_agents / Claude 兼容的 wait/wait_agents 别名"
                .to_string(),
        strict: false,
        parameters: JsonSchema::Object {
            properties,
            required: Some(vec!["agent_id".to_string()]),
            additional_properties: Some(false.into()),
        },
    })
}

fn create_claude_task_stop_alias_tool() -> ToolSpec {
    let properties = BTreeMap::from([(
        "agent_id".to_string(),
        JsonSchema::String {
            description: Some("Agent id to stop / 要停止的 agent id".to_string()),
        },
    )]);

    ToolSpec::Function(ResponsesApiTool {
        name: "TaskStop".to_string(),
        description: "Claude-compatible alias for close_agent / Claude 兼容的 close_agent 别名"
            .to_string(),
        strict: false,
        parameters: JsonSchema::Object {
            properties,
            required: Some(vec!["agent_id".to_string()]),
            additional_properties: Some(false.into()),
        },
    })
}

fn create_claude_tool_search_alias_tool() -> ToolSpec {
    let properties = BTreeMap::from([
        (
            "query".to_string(),
            JsonSchema::String {
                description: Some("Tool search query / 工具搜索关键词".to_string()),
            },
        ),
        (
            "max_results".to_string(),
            JsonSchema::Number {
                description: Some("Maximum result count / 返回结果上限".to_string()),
            },
        ),
    ]);

    ToolSpec::Function(ResponsesApiTool {
        name: "ToolSearch".to_string(),
        description:
            "Claude-compatible alias for search_tool_bm25 / Claude 兼容的 search_tool_bm25 别名"
                .to_string(),
        strict: false,
        parameters: JsonSchema::Object {
            properties,
            required: Some(vec!["query".to_string()]),
            additional_properties: Some(false.into()),
        },
    })
}

fn create_claude_skill_alias_tool() -> ToolSpec {
    let properties = BTreeMap::from([
        (
            "skill".to_string(),
            JsonSchema::String {
                description: Some("Skill name / 技能名称".to_string()),
            },
        ),
        (
            "args".to_string(),
            JsonSchema::String {
                description: Some("Optional skill arguments / 可选技能参数".to_string()),
            },
        ),
    ]);

    ToolSpec::Function(ResponsesApiTool {
        name: "Skill".to_string(),
        description:
            "Claude-compatible alias for skill execution entry / Claude 兼容的技能执行入口别名"
                .to_string(),
        strict: false,
        parameters: JsonSchema::Object {
            properties,
            required: Some(vec!["skill".to_string()]),
            additional_properties: Some(false.into()),
        },
    })
}

fn create_claude_ask_user_question_alias_tool() -> ToolSpec {
    let option_properties = BTreeMap::from([
        (
            "label".to_string(),
            JsonSchema::String {
                description: Some("Choice label / 选项标签".to_string()),
            },
        ),
        (
            "description".to_string(),
            JsonSchema::String {
                description: Some("Choice description / 选项描述".to_string()),
            },
        ),
    ]);
    let question_properties = BTreeMap::from([
        (
            "id".to_string(),
            JsonSchema::String {
                description: Some("Optional stable id / 可选稳定 id".to_string()),
            },
        ),
        (
            "header".to_string(),
            JsonSchema::String {
                description: Some("Question header / 问题标题".to_string()),
            },
        ),
        (
            "question".to_string(),
            JsonSchema::String {
                description: Some("Question text / 问题内容".to_string()),
            },
        ),
        (
            "multiSelect".to_string(),
            JsonSchema::Boolean {
                description: Some(
                    "Whether multiple choices are allowed / 是否允许多选".to_string(),
                ),
            },
        ),
        (
            "options".to_string(),
            JsonSchema::Array {
                description: Some("Question options / 问题选项".to_string()),
                items: Box::new(JsonSchema::Object {
                    properties: option_properties,
                    required: Some(vec!["label".to_string(), "description".to_string()]),
                    additional_properties: Some(false.into()),
                }),
            },
        ),
    ]);
    let properties = BTreeMap::from([(
        "questions".to_string(),
        JsonSchema::Array {
            description: Some("Question list / 问题列表".to_string()),
            items: Box::new(JsonSchema::Object {
                properties: question_properties,
                required: Some(vec![
                    "header".to_string(),
                    "question".to_string(),
                    "options".to_string(),
                ]),
                additional_properties: Some(false.into()),
            }),
        },
    )]);

    ToolSpec::Function(ResponsesApiTool {
        name: "AskUserQuestion".to_string(),
        description:
            "Claude-compatible alias for request_user_input / Claude 兼容的 request_user_input 别名"
                .to_string(),
        strict: false,
        parameters: JsonSchema::Object {
            properties,
            required: Some(vec!["questions".to_string()]),
            additional_properties: Some(false.into()),
        },
    })
}

fn create_claude_bash_alias_tool() -> ToolSpec {
    let properties = BTreeMap::from([
        (
            "command".to_string(),
            JsonSchema::String {
                description: Some("Shell command to execute / 要执行的 shell 命令".to_string()),
            },
        ),
        (
            "timeout".to_string(),
            JsonSchema::Number {
                description: Some(
                    "Optional timeout milliseconds / 可选超时时间（毫秒）".to_string(),
                ),
            },
        ),
        (
            "description".to_string(),
            JsonSchema::String {
                description: Some("Optional command description / 可选命令描述".to_string()),
            },
        ),
        (
            "run_in_background".to_string(),
            JsonSchema::Boolean {
                description: Some("Whether to run in background / 是否后台运行".to_string()),
            },
        ),
    ]);
    ToolSpec::Function(ResponsesApiTool {
        name: "Bash".to_string(),
        description: "Claude-compatible shell alias / Claude 兼容的 shell 别名".to_string(),
        strict: false,
        parameters: JsonSchema::Object {
            properties,
            required: Some(vec!["command".to_string()]),
            additional_properties: Some(false.into()),
        },
    })
}

fn create_claude_read_alias_tool() -> ToolSpec {
    let properties = BTreeMap::from([
        (
            "file_path".to_string(),
            JsonSchema::String {
                description: Some("Absolute file path / 绝对文件路径".to_string()),
            },
        ),
        (
            "offset".to_string(),
            JsonSchema::Number {
                description: Some("Optional line offset / 可选起始行号".to_string()),
            },
        ),
        (
            "limit".to_string(),
            JsonSchema::Number {
                description: Some("Optional line limit / 可选读取行数上限".to_string()),
            },
        ),
        (
            "mode".to_string(),
            JsonSchema::String {
                description: Some("Read mode (slice/indentation) / 读取模式".to_string()),
            },
        ),
        (
            "indentation".to_string(),
            JsonSchema::Object {
                properties: BTreeMap::new(),
                required: None,
                additional_properties: Some(true.into()),
            },
        ),
        (
            "pages".to_string(),
            JsonSchema::String {
                description: Some("Optional PDF page range / 可选 PDF 页范围".to_string()),
            },
        ),
    ]);
    ToolSpec::Function(ResponsesApiTool {
        name: "Read".to_string(),
        description: "Claude-compatible alias for read_file / Claude 兼容的 read_file 别名"
            .to_string(),
        strict: false,
        parameters: JsonSchema::Object {
            properties,
            required: Some(vec!["file_path".to_string()]),
            additional_properties: Some(false.into()),
        },
    })
}

fn create_claude_grep_alias_tool() -> ToolSpec {
    let properties = BTreeMap::from([
        (
            "pattern".to_string(),
            JsonSchema::String {
                description: Some("Search pattern / 搜索模式".to_string()),
            },
        ),
        (
            "path".to_string(),
            JsonSchema::String {
                description: Some("Search root path / 搜索根路径".to_string()),
            },
        ),
        (
            "glob".to_string(),
            JsonSchema::String {
                description: Some("File glob filter / 文件 glob 过滤".to_string()),
            },
        ),
        (
            "output_mode".to_string(),
            JsonSchema::String {
                description: Some("Output mode / 输出模式".to_string()),
            },
        ),
        (
            "head_limit".to_string(),
            JsonSchema::Number {
                description: Some("Result limit / 结果数量上限".to_string()),
            },
        ),
        (
            "offset".to_string(),
            JsonSchema::Number {
                description: Some("Result offset / 结果偏移量".to_string()),
            },
        ),
    ]);
    ToolSpec::Function(ResponsesApiTool {
        name: "Grep".to_string(),
        description: "Claude-compatible alias for grep_files / Claude 兼容的 grep_files 别名"
            .to_string(),
        strict: false,
        parameters: JsonSchema::Object {
            properties,
            required: Some(vec!["pattern".to_string()]),
            additional_properties: Some(false.into()),
        },
    })
}

fn create_claude_todo_write_alias_tool() -> ToolSpec {
    let todo_properties = BTreeMap::from([
        (
            "content".to_string(),
            JsonSchema::String {
                description: Some("Task content / 任务描述".to_string()),
            },
        ),
        (
            "activeForm".to_string(),
            JsonSchema::String {
                description: Some("Task active form / 任务进行式".to_string()),
            },
        ),
        (
            "status".to_string(),
            JsonSchema::String {
                description: Some(
                    "Task status: pending|in_progress|completed / 任务状态".to_string(),
                ),
            },
        ),
    ]);
    let properties = BTreeMap::from([(
        "todos".to_string(),
        JsonSchema::Array {
            description: Some("Todo list items / 任务列表".to_string()),
            items: Box::new(JsonSchema::Object {
                properties: todo_properties,
                required: Some(vec!["status".to_string()]),
                additional_properties: Some(false.into()),
            }),
        },
    )]);
    ToolSpec::Function(ResponsesApiTool {
        name: "TodoWrite".to_string(),
        description: "Claude-compatible alias for update_plan / Claude 兼容的 update_plan 别名"
            .to_string(),
        strict: false,
        parameters: JsonSchema::Object {
            properties,
            required: Some(vec!["todos".to_string()]),
            additional_properties: Some(false.into()),
        },
    })
}

fn create_claude_enter_plan_mode_alias_tool() -> ToolSpec {
    ToolSpec::Function(ResponsesApiTool {
        name: "EnterPlanMode".to_string(),
        description:
            "Switch current session to plan collaboration mode / 切换当前会话到计划协作模式"
                .to_string(),
        strict: false,
        parameters: JsonSchema::Object {
            properties: BTreeMap::new(),
            required: None,
            additional_properties: Some(false.into()),
        },
    })
}

fn create_claude_exit_plan_mode_alias_tool() -> ToolSpec {
    let mut properties = BTreeMap::new();
    properties.insert(
        "allowedPrompts".to_string(),
        JsonSchema::Array {
            description: Some(
                "Optional prompt permissions for execution phase / 执行阶段可选权限提示"
                    .to_string(),
            ),
            items: Box::new(JsonSchema::Object {
                properties: BTreeMap::new(),
                required: None,
                additional_properties: Some(true.into()),
            }),
        },
    );
    ToolSpec::Function(ResponsesApiTool {
        name: "ExitPlanMode".to_string(),
        description: "Switch current session back to default collaboration mode / 切回默认协作模式"
            .to_string(),
        strict: false,
        parameters: JsonSchema::Object {
            properties,
            required: None,
            additional_properties: Some(true.into()),
        },
    })
}

fn create_claude_write_tool() -> ToolSpec {
    let properties = BTreeMap::from([
        (
            "file_path".to_string(),
            JsonSchema::String {
                description: Some("Absolute file path / 绝对文件路径".to_string()),
            },
        ),
        (
            "content".to_string(),
            JsonSchema::String {
                description: Some("File content to write / 写入文件内容".to_string()),
            },
        ),
    ]);
    ToolSpec::Function(ResponsesApiTool {
        name: "Write".to_string(),
        description: "Write file content with overwrite semantics / 覆盖写入文件内容".to_string(),
        strict: false,
        parameters: JsonSchema::Object {
            properties,
            required: Some(vec!["file_path".to_string(), "content".to_string()]),
            additional_properties: Some(false.into()),
        },
    })
}

fn create_claude_edit_tool() -> ToolSpec {
    let properties = BTreeMap::from([
        (
            "file_path".to_string(),
            JsonSchema::String {
                description: Some("Absolute file path / 绝对文件路径".to_string()),
            },
        ),
        (
            "old_string".to_string(),
            JsonSchema::String {
                description: Some("Original text to replace / 需要替换的原文本".to_string()),
            },
        ),
        (
            "new_string".to_string(),
            JsonSchema::String {
                description: Some("Replacement text / 替换后的文本".to_string()),
            },
        ),
        (
            "replace_all".to_string(),
            JsonSchema::Boolean {
                description: Some("Replace all matches / 是否替换全部匹配".to_string()),
            },
        ),
    ]);
    ToolSpec::Function(ResponsesApiTool {
        name: "Edit".to_string(),
        description: "Edit file content by exact string replacement / 通过精确文本替换编辑文件"
            .to_string(),
        strict: false,
        parameters: JsonSchema::Object {
            properties,
            required: Some(vec![
                "file_path".to_string(),
                "old_string".to_string(),
                "new_string".to_string(),
            ]),
            additional_properties: Some(false.into()),
        },
    })
}

fn create_claude_glob_tool() -> ToolSpec {
    let properties = BTreeMap::from([
        (
            "pattern".to_string(),
            JsonSchema::String {
                description: Some("Glob pattern / 通配符模式".to_string()),
            },
        ),
        (
            "path".to_string(),
            JsonSchema::String {
                description: Some("Optional search base path / 可选搜索根目录".to_string()),
            },
        ),
    ]);
    ToolSpec::Function(ResponsesApiTool {
        name: "Glob".to_string(),
        description: "Find files by glob pattern / 按 glob 模式查找文件".to_string(),
        strict: false,
        parameters: JsonSchema::Object {
            properties,
            required: Some(vec!["pattern".to_string()]),
            additional_properties: Some(false.into()),
        },
    })
}

fn create_claude_notebook_edit_tool() -> ToolSpec {
    let properties = BTreeMap::from([
        (
            "notebook_path".to_string(),
            JsonSchema::String {
                description: Some("Absolute notebook path / 绝对 notebook 路径".to_string()),
            },
        ),
        (
            "new_source".to_string(),
            JsonSchema::String {
                description: Some("New source content / 新的单元内容".to_string()),
            },
        ),
        (
            "edit_mode".to_string(),
            JsonSchema::String {
                description: Some("Edit mode: replace|insert|delete / 编辑模式".to_string()),
            },
        ),
        (
            "cell_id".to_string(),
            JsonSchema::String {
                description: Some("Target cell id / 目标单元 id".to_string()),
            },
        ),
        (
            "cell_number".to_string(),
            JsonSchema::Number {
                description: Some("Target cell index / 目标单元索引".to_string()),
            },
        ),
        (
            "cell_type".to_string(),
            JsonSchema::String {
                description: Some("Cell type override / 单元类型覆盖".to_string()),
            },
        ),
    ]);
    ToolSpec::Function(ResponsesApiTool {
        name: "NotebookEdit".to_string(),
        description: "Edit Jupyter notebook cells / 编辑 Jupyter notebook 单元".to_string(),
        strict: false,
        parameters: JsonSchema::Object {
            properties,
            required: Some(vec!["notebook_path".to_string(), "new_source".to_string()]),
            additional_properties: Some(false.into()),
        },
    })
}

fn create_test_sync_tool() -> ToolSpec {
    let barrier_properties = BTreeMap::from([
        (
            "id".to_string(),
            JsonSchema::String {
                description: Some(
                    "Identifier shared by concurrent calls that should rendezvous".to_string(),
                ),
            },
        ),
        (
            "participants".to_string(),
            JsonSchema::Number {
                description: Some(
                    "Number of tool calls that must arrive before the barrier opens".to_string(),
                ),
            },
        ),
        (
            "timeout_ms".to_string(),
            JsonSchema::Number {
                description: Some(
                    "Maximum time in milliseconds to wait at the barrier".to_string(),
                ),
            },
        ),
    ]);

    let properties = BTreeMap::from([
        (
            "sleep_before_ms".to_string(),
            JsonSchema::Number {
                description: Some(
                    "Optional delay in milliseconds before any other action".to_string(),
                ),
            },
        ),
        (
            "sleep_after_ms".to_string(),
            JsonSchema::Number {
                description: Some(
                    "Optional delay in milliseconds after completing the barrier".to_string(),
                ),
            },
        ),
        (
            "barrier".to_string(),
            JsonSchema::Object {
                properties: barrier_properties,
                required: Some(vec!["id".to_string(), "participants".to_string()]),
                additional_properties: Some(false.into()),
            },
        ),
    ]);

    ToolSpec::Function(ResponsesApiTool {
        name: "test_sync_tool".to_string(),
        description: "Internal synchronization helper used by Codex integration tests.".to_string(),
        strict: false,
        parameters: JsonSchema::Object {
            properties,
            required: None,
            additional_properties: Some(false.into()),
        },
    })
}

fn create_grep_files_tool() -> ToolSpec {
    let properties = BTreeMap::from([
        (
            "pattern".to_string(),
            JsonSchema::String {
                description: Some("Regular expression pattern to search for.".to_string()),
            },
        ),
        (
            "include".to_string(),
            JsonSchema::String {
                description: Some(
                    "Optional glob that limits which files are searched (e.g. \"*.rs\" or \
                     \"*.{ts,tsx}\")."
                        .to_string(),
                ),
            },
        ),
        (
            "path".to_string(),
            JsonSchema::String {
                description: Some(
                    "Directory or file path to search. Defaults to the session's working directory."
                        .to_string(),
                ),
            },
        ),
        (
            "limit".to_string(),
            JsonSchema::Number {
                description: Some(
                    "Maximum number of file paths to return (defaults to 100).".to_string(),
                ),
            },
        ),
    ]);

    ToolSpec::Function(ResponsesApiTool {
        name: "grep_files".to_string(),
        description: "Finds files whose contents match the pattern and lists them by modification \
                      time."
            .to_string(),
        strict: false,
        parameters: JsonSchema::Object {
            properties,
            required: Some(vec!["pattern".to_string()]),
            additional_properties: Some(false.into()),
        },
    })
}

fn create_search_tool_bm25_tool() -> ToolSpec {
    let properties = BTreeMap::from([
        (
            "query".to_string(),
            JsonSchema::String {
                description: Some("Search query for MCP tools.".to_string()),
            },
        ),
        (
            "limit".to_string(),
            JsonSchema::Number {
                description: Some(format!(
                    "Maximum number of tools to return (defaults to {SEARCH_TOOL_BM25_DEFAULT_LIMIT})."
                )),
            },
        ),
    ]);

    ToolSpec::Function(ResponsesApiTool {
        name: "search_tool_bm25".to_string(),
        description: "Searches MCP tool metadata with BM25 and exposes matching tools for the next model call.".to_string(),
        strict: false,
        parameters: JsonSchema::Object {
            properties,
            required: Some(vec!["query".to_string()]),
            additional_properties: Some(false.into()),
        },
    })
}

fn create_read_file_tool() -> ToolSpec {
    let indentation_properties = BTreeMap::from([
        (
            "anchor_line".to_string(),
            JsonSchema::Number {
                description: Some(
                    "Anchor line to center the indentation lookup on (defaults to offset)."
                        .to_string(),
                ),
            },
        ),
        (
            "max_levels".to_string(),
            JsonSchema::Number {
                description: Some(
                    "How many parent indentation levels (smaller indents) to include.".to_string(),
                ),
            },
        ),
        (
            "include_siblings".to_string(),
            JsonSchema::Boolean {
                description: Some(
                    "When true, include additional blocks that share the anchor indentation."
                        .to_string(),
                ),
            },
        ),
        (
            "include_header".to_string(),
            JsonSchema::Boolean {
                description: Some(
                    "Include doc comments or attributes directly above the selected block."
                        .to_string(),
                ),
            },
        ),
        (
            "max_lines".to_string(),
            JsonSchema::Number {
                description: Some(
                    "Hard cap on the number of lines returned when using indentation mode."
                        .to_string(),
                ),
            },
        ),
    ]);

    let properties = BTreeMap::from([
        (
            "file_path".to_string(),
            JsonSchema::String {
                description: Some("Absolute path to the file".to_string()),
            },
        ),
        (
            "offset".to_string(),
            JsonSchema::Number {
                description: Some(
                    "The line number to start reading from. Must be 1 or greater.".to_string(),
                ),
            },
        ),
        (
            "limit".to_string(),
            JsonSchema::Number {
                description: Some("The maximum number of lines to return.".to_string()),
            },
        ),
        (
            "mode".to_string(),
            JsonSchema::String {
                description: Some(
                    "Optional mode selector: \"slice\" for simple ranges (default) or \"indentation\" \
                     to expand around an anchor line."
                        .to_string(),
                ),
            },
        ),
        (
            "indentation".to_string(),
            JsonSchema::Object {
                properties: indentation_properties,
                required: None,
                additional_properties: Some(false.into()),
            },
        ),
    ]);

    ToolSpec::Function(ResponsesApiTool {
        name: "read_file".to_string(),
        description:
            "Reads a local file with 1-indexed line numbers, supporting slice and indentation-aware block modes."
                .to_string(),
        strict: false,
        parameters: JsonSchema::Object {
            properties,
            required: Some(vec!["file_path".to_string()]),
            additional_properties: Some(false.into()),
        },
    })
}

fn create_batches_read_file_tool() -> ToolSpec {
    let indentation_properties = BTreeMap::from([
        (
            "anchor_line".to_string(),
            JsonSchema::Number {
                description: Some(
                    "Anchor line to center the indentation lookup on (defaults to offset)."
                        .to_string(),
                ),
            },
        ),
        (
            "max_levels".to_string(),
            JsonSchema::Number {
                description: Some(
                    "How many parent indentation levels (smaller indents) to include.".to_string(),
                ),
            },
        ),
        (
            "include_siblings".to_string(),
            JsonSchema::Boolean {
                description: Some(
                    "When true, include additional blocks that share the anchor indentation."
                        .to_string(),
                ),
            },
        ),
        (
            "include_header".to_string(),
            JsonSchema::Boolean {
                description: Some(
                    "Include doc comments or attributes directly above the selected block."
                        .to_string(),
                ),
            },
        ),
        (
            "max_lines".to_string(),
            JsonSchema::Number {
                description: Some(
                    "Hard cap on the number of lines returned when using indentation mode."
                        .to_string(),
                ),
            },
        ),
    ]);

    let file_properties = BTreeMap::from([
        (
            "path".to_string(),
            JsonSchema::String {
                description: Some("File path (absolute or relative to the session cwd).".to_string()),
            },
        ),
        (
            "offset".to_string(),
            JsonSchema::Number {
                description: Some(
                    "The line number to start reading from. Must be 1 or greater.".to_string(),
                ),
            },
        ),
        (
            "limit".to_string(),
            JsonSchema::Number {
                description: Some("The maximum number of lines to return.".to_string()),
            },
        ),
        (
            "mode".to_string(),
            JsonSchema::String {
                description: Some(
                    "Optional mode selector: \"slice\" for simple ranges (default) or \"indentation\" \
                     to expand around an anchor line."
                        .to_string(),
                ),
            },
        ),
        (
            "indentation".to_string(),
            JsonSchema::Object {
                properties: indentation_properties.clone(),
                required: None,
                additional_properties: Some(false.into()),
            },
        ),
    ]);

    let properties = BTreeMap::from([
        (
            "paths".to_string(),
            JsonSchema::Array {
                items: Box::new(JsonSchema::Object {
                    properties: file_properties,
                    required: Some(vec!["path".to_string()]),
                    additional_properties: Some(false.into()),
                }),
                description: Some(
                    "Files to read; each entry can override the default read options.".to_string(),
                ),
            },
        ),
        (
            "offset".to_string(),
            JsonSchema::Number {
                description: Some(
                    "Default line number to start reading from when per-file offset is omitted."
                        .to_string(),
                ),
            },
        ),
        (
            "limit".to_string(),
            JsonSchema::Number {
                description: Some(
                    "Default maximum number of lines to return when per-file limit is omitted."
                        .to_string(),
                ),
            },
        ),
        (
            "mode".to_string(),
            JsonSchema::String {
                description: Some(
                    "Default mode selector: \"slice\" for simple ranges (default) or \"indentation\" \
                     to expand around an anchor line."
                        .to_string(),
                ),
            },
        ),
        (
            "indentation".to_string(),
            JsonSchema::Object {
                properties: indentation_properties,
                required: None,
                additional_properties: Some(false.into()),
            },
        ),
    ]);

    ToolSpec::Function(ResponsesApiTool {
        name: "batches_read_file".to_string(),
        description: "Reads multiple local files with 1-indexed line numbers and returns a structured JSON payload."
            .to_string(),
        strict: false,
        parameters: JsonSchema::Object {
            properties,
            required: Some(vec!["paths".to_string()]),
            additional_properties: Some(false.into()),
        },
    })
}

fn create_list_dir_tool() -> ToolSpec {
    let properties = BTreeMap::from([
        (
            "dir_path".to_string(),
            JsonSchema::String {
                description: Some("Absolute path to the directory to list.".to_string()),
            },
        ),
        (
            "offset".to_string(),
            JsonSchema::Number {
                description: Some(
                    "The entry number to start listing from. Must be 1 or greater.".to_string(),
                ),
            },
        ),
        (
            "limit".to_string(),
            JsonSchema::Number {
                description: Some("The maximum number of entries to return.".to_string()),
            },
        ),
        (
            "depth".to_string(),
            JsonSchema::Number {
                description: Some(
                    "The maximum directory depth to traverse. Must be 1 or greater.".to_string(),
                ),
            },
        ),
    ]);

    ToolSpec::Function(ResponsesApiTool {
        name: "list_dir".to_string(),
        description:
            "Lists entries in a local directory with 1-indexed entry numbers and simple type labels."
                .to_string(),
        strict: false,
        parameters: JsonSchema::Object {
            properties,
            required: Some(vec!["dir_path".to_string()]),
            additional_properties: Some(false.into()),
        },
    })
}

fn create_list_mcp_resources_tool() -> ToolSpec {
    let properties = BTreeMap::from([
        (
            "server".to_string(),
            JsonSchema::String {
                description: Some(
                    "Optional MCP server name. When omitted, lists resources from every configured server."
                        .to_string(),
                ),
            },
        ),
        (
            "cursor".to_string(),
            JsonSchema::String {
                description: Some(
                    "Opaque cursor returned by a previous list_mcp_resources call for the same server."
                        .to_string(),
                ),
            },
        ),
    ]);

    ToolSpec::Function(ResponsesApiTool {
        name: "list_mcp_resources".to_string(),
        description: "Lists resources provided by MCP servers. Resources allow servers to share data that provides context to language models, such as files, database schemas, or application-specific information. Prefer resources over web search when possible.".to_string(),
        strict: false,
        parameters: JsonSchema::Object {
            properties,
            required: None,
            additional_properties: Some(false.into()),
        },
    })
}

fn create_list_mcp_resource_templates_tool() -> ToolSpec {
    let properties = BTreeMap::from([
        (
            "server".to_string(),
            JsonSchema::String {
                description: Some(
                    "Optional MCP server name. When omitted, lists resource templates from all configured servers."
                        .to_string(),
                ),
            },
        ),
        (
            "cursor".to_string(),
            JsonSchema::String {
                description: Some(
                    "Opaque cursor returned by a previous list_mcp_resource_templates call for the same server."
                        .to_string(),
                ),
            },
        ),
    ]);

    ToolSpec::Function(ResponsesApiTool {
        name: "list_mcp_resource_templates".to_string(),
        description: "Lists resource templates provided by MCP servers. Parameterized resource templates allow servers to share data that takes parameters and provides context to language models, such as files, database schemas, or application-specific information. Prefer resource templates over web search when possible.".to_string(),
        strict: false,
        parameters: JsonSchema::Object {
            properties,
            required: None,
            additional_properties: Some(false.into()),
        },
    })
}

fn create_read_mcp_resource_tool() -> ToolSpec {
    let properties = BTreeMap::from([
        (
            "server".to_string(),
            JsonSchema::String {
                description: Some(
                    "MCP server name exactly as configured. Must match the 'server' field returned by list_mcp_resources."
                        .to_string(),
                ),
            },
        ),
        (
            "uri".to_string(),
            JsonSchema::String {
                description: Some(
                    "Resource URI to read. Must be one of the URIs returned by list_mcp_resources."
                        .to_string(),
                ),
            },
        ),
    ]);

    ToolSpec::Function(ResponsesApiTool {
        name: "read_mcp_resource".to_string(),
        description:
            "Read a specific resource from an MCP server given the server name and resource URI."
                .to_string(),
        strict: false,
        parameters: JsonSchema::Object {
            properties,
            required: Some(vec!["server".to_string(), "uri".to_string()]),
            additional_properties: Some(false.into()),
        },
    })
}

/// TODO(dylan): deprecate once we get rid of json tool
#[derive(Serialize, Deserialize)]
pub(crate) struct ApplyPatchToolArgs {
    pub(crate) input: String,
}

/// Returns JSON values that are compatible with Function Calling in the
/// Responses API:
/// https://platform.openai.com/docs/guides/function-calling?api-mode=responses
pub fn create_tools_json_for_responses_api(
    tools: &[ToolSpec],
) -> crate::error::Result<Vec<serde_json::Value>> {
    let mut tools_json = Vec::new();

    for tool in tools {
        let json = serde_json::to_value(tool)?;
        tools_json.push(json);
    }

    Ok(tools_json)
}

pub(crate) fn mcp_tool_to_openai_tool(
    fully_qualified_name: String,
    tool: rmcp::model::Tool,
) -> Result<ResponsesApiTool, serde_json::Error> {
    let rmcp::model::Tool {
        description,
        input_schema,
        ..
    } = tool;

    let mut serialized_input_schema = serde_json::Value::Object(input_schema.as_ref().clone());

    // OpenAI models mandate the "properties" field in the schema. Some MCP
    // servers omit it (or set it to null), so we insert an empty object to
    // match the behavior of the Agents SDK.
    if let serde_json::Value::Object(obj) = &mut serialized_input_schema
        && obj.get("properties").is_none_or(serde_json::Value::is_null)
    {
        obj.insert(
            "properties".to_string(),
            serde_json::Value::Object(serde_json::Map::new()),
        );
    }

    // Serialize to a raw JSON value so we can sanitize schemas coming from MCP
    // servers. Some servers omit the top-level or nested `type` in JSON
    // Schemas (e.g. using enum/anyOf), or use unsupported variants like
    // `integer`. Our internal JsonSchema is a small subset and requires
    // `type`, so we coerce/sanitize here for compatibility.
    sanitize_json_schema(&mut serialized_input_schema);
    let input_schema = serde_json::from_value::<JsonSchema>(serialized_input_schema)?;

    Ok(ResponsesApiTool {
        name: fully_qualified_name,
        description: description.map(Into::into).unwrap_or_default(),
        strict: false,
        parameters: input_schema,
    })
}

fn dynamic_tool_to_openai_tool(
    tool: &DynamicToolSpec,
) -> Result<ResponsesApiTool, serde_json::Error> {
    let input_schema = parse_tool_input_schema(&tool.input_schema)?;

    Ok(ResponsesApiTool {
        name: tool.name.clone(),
        description: tool.description.clone(),
        strict: false,
        parameters: input_schema,
    })
}

/// Parse the tool input_schema or return an error for invalid schema
pub fn parse_tool_input_schema(input_schema: &JsonValue) -> Result<JsonSchema, serde_json::Error> {
    let mut input_schema = input_schema.clone();
    sanitize_json_schema(&mut input_schema);
    serde_json::from_value::<JsonSchema>(input_schema)
}

/// Sanitize a JSON Schema (as serde_json::Value) so it can fit our limited
/// JsonSchema enum. This function:
/// - Ensures every schema object has a "type". If missing, infers it from
///   common keywords (properties => object, items => array, enum/const/format => string)
///   and otherwise defaults to "string".
/// - Fills required child fields (e.g. array items, object properties) with
///   permissive defaults when absent.
fn sanitize_json_schema(value: &mut JsonValue) {
    match value {
        JsonValue::Bool(_) => {
            // JSON Schema boolean form: true/false. Coerce to an accept-all string.
            *value = json!({ "type": "string" });
        }
        JsonValue::Array(arr) => {
            for v in arr.iter_mut() {
                sanitize_json_schema(v);
            }
        }
        JsonValue::Object(map) => {
            // First, recursively sanitize known nested schema holders
            if let Some(props) = map.get_mut("properties")
                && let Some(props_map) = props.as_object_mut()
            {
                for (_k, v) in props_map.iter_mut() {
                    sanitize_json_schema(v);
                }
            }
            if let Some(items) = map.get_mut("items") {
                sanitize_json_schema(items);
            }
            // Some schemas use oneOf/anyOf/allOf - sanitize their entries
            for combiner in ["oneOf", "anyOf", "allOf", "prefixItems"] {
                if let Some(v) = map.get_mut(combiner) {
                    sanitize_json_schema(v);
                }
            }

            // Normalize/ensure type
            let mut ty = map.get("type").and_then(|v| v.as_str()).map(str::to_string);

            // If type is an array (union), pick first supported; else leave to inference
            if ty.is_none()
                && let Some(JsonValue::Array(types)) = map.get("type")
            {
                for t in types {
                    if let Some(tt) = t.as_str()
                        && matches!(
                            tt,
                            "object" | "array" | "string" | "number" | "integer" | "boolean"
                        )
                    {
                        ty = Some(tt.to_string());
                        break;
                    }
                }
            }

            // Infer type if still missing
            if ty.is_none() {
                if map.contains_key("properties")
                    || map.contains_key("required")
                    || map.contains_key("additionalProperties")
                {
                    ty = Some("object".to_string());
                } else if map.contains_key("items") || map.contains_key("prefixItems") {
                    ty = Some("array".to_string());
                } else if map.contains_key("enum")
                    || map.contains_key("const")
                    || map.contains_key("format")
                {
                    ty = Some("string".to_string());
                } else if map.contains_key("minimum")
                    || map.contains_key("maximum")
                    || map.contains_key("exclusiveMinimum")
                    || map.contains_key("exclusiveMaximum")
                    || map.contains_key("multipleOf")
                {
                    ty = Some("number".to_string());
                }
            }
            // If we still couldn't infer, default to string
            let ty = ty.unwrap_or_else(|| "string".to_string());
            map.insert("type".to_string(), JsonValue::String(ty.to_string()));

            // Ensure object schemas have properties map
            if ty == "object" {
                if !map.contains_key("properties") {
                    map.insert(
                        "properties".to_string(),
                        JsonValue::Object(serde_json::Map::new()),
                    );
                }
                // If additionalProperties is an object schema, sanitize it too.
                // Leave booleans as-is, since JSON Schema allows boolean here.
                if let Some(ap) = map.get_mut("additionalProperties") {
                    let is_bool = matches!(ap, JsonValue::Bool(_));
                    if !is_bool {
                        sanitize_json_schema(ap);
                    }
                }
            }

            // Ensure array schemas have items
            if ty == "array" && !map.contains_key("items") {
                map.insert("items".to_string(), json!({ "type": "string" }));
            }
        }
        _ => {}
    }
}

/// Builds the tool registry builder while collecting tool specs for later serialization.
pub(crate) fn build_specs(
    config: &ToolsConfig,
    mcp_tools: Option<HashMap<String, rmcp::model::Tool>>,
    dynamic_tools: &[DynamicToolSpec],
) -> ToolRegistryBuilder {
    use crate::tools::handlers::ApplyPatchHandler;
    use crate::tools::handlers::BatchesReadFileHandler;
    use crate::tools::handlers::ClaudeEditHandler;
    use crate::tools::handlers::ClaudeGlobHandler;
    use crate::tools::handlers::ClaudeNotebookEditHandler;
    use crate::tools::handlers::ClaudeToolAdapterHandler;
    use crate::tools::handlers::ClaudeWriteHandler;
    use crate::tools::handlers::CollabBatchHandler;
    use crate::tools::handlers::CollabHandler;
    use crate::tools::handlers::DynamicToolHandler;
    use crate::tools::handlers::GrepFilesHandler;
    use crate::tools::handlers::ListDirHandler;
    use crate::tools::handlers::McpHandler;
    use crate::tools::handlers::McpResourceHandler;
    use crate::tools::handlers::PlanHandler;
    use crate::tools::handlers::ReadFileHandler;
    use crate::tools::handlers::RequestUserInputHandler;
    use crate::tools::handlers::SearchToolBm25Handler;
    use crate::tools::handlers::ShellCommandHandler;
    use crate::tools::handlers::ShellHandler;
    use crate::tools::handlers::TestSyncHandler;
    use crate::tools::handlers::UnifiedExecHandler;
    use crate::tools::handlers::ViewImageHandler;
    use std::sync::Arc;

    let mut builder = ToolRegistryBuilder::new();

    let shell_handler = Arc::new(ShellHandler);
    let unified_exec_handler = Arc::new(UnifiedExecHandler);
    let plan_handler = Arc::new(PlanHandler);
    let apply_patch_handler = Arc::new(ApplyPatchHandler);
    let dynamic_tool_handler = Arc::new(DynamicToolHandler);
    let view_image_handler = Arc::new(ViewImageHandler);
    let mcp_handler = Arc::new(McpHandler);
    let mcp_resource_handler = Arc::new(McpResourceHandler);
    let shell_command_handler = Arc::new(ShellCommandHandler);
    let batches_read_file_handler = Arc::new(BatchesReadFileHandler);
    let request_user_input_handler = Arc::new(RequestUserInputHandler);
    let search_tool_handler = Arc::new(SearchToolBm25Handler);
    let collab_batch_handler = Arc::new(CollabBatchHandler);
    let claude_tool_adapter_handler = Arc::new(ClaudeToolAdapterHandler);
    let claude_write_handler = Arc::new(ClaudeWriteHandler);
    let claude_edit_handler = Arc::new(ClaudeEditHandler);
    let claude_glob_handler = Arc::new(ClaudeGlobHandler);
    let claude_notebook_edit_handler = Arc::new(ClaudeNotebookEditHandler);

    match &config.shell_type {
        ConfigShellToolType::Default => {
            builder.push_spec_with_parallel_support(
                create_shell_tool(config.request_rule_enabled),
                true,
            );
        }
        ConfigShellToolType::Local => {
            builder.push_spec_with_parallel_support(ToolSpec::LocalShell {}, true);
        }
        ConfigShellToolType::UnifiedExec => {
            builder.push_spec_with_parallel_support(
                create_exec_command_tool(config.request_rule_enabled),
                true,
            );
            builder.push_spec(create_write_stdin_tool());
            builder.register_handler("exec_command", unified_exec_handler.clone());
            builder.register_handler("write_stdin", unified_exec_handler);
        }
        ConfigShellToolType::Disabled => {
            // Do nothing.
        }
        ConfigShellToolType::ShellCommand => {
            builder.push_spec_with_parallel_support(
                create_shell_command_tool(config.request_rule_enabled),
                true,
            );
        }
    }

    if config.shell_type != ConfigShellToolType::Disabled {
        // Always register shell aliases so older prompts remain compatible.
        builder.register_handler("shell", shell_handler.clone());
        builder.register_handler("container.exec", shell_handler.clone());
        builder.register_handler("local_shell", shell_handler);
        builder.register_handler("shell_command", shell_command_handler);
    }

    builder.push_spec_with_parallel_support(create_list_mcp_resources_tool(), true);
    builder.push_spec_with_parallel_support(create_list_mcp_resource_templates_tool(), true);
    builder.push_spec_with_parallel_support(create_read_mcp_resource_tool(), true);
    builder.register_handler("list_mcp_resources", mcp_resource_handler.clone());
    builder.register_handler("list_mcp_resource_templates", mcp_resource_handler.clone());
    builder.register_handler("read_mcp_resource", mcp_resource_handler);

    builder.push_spec(PLAN_TOOL.clone());
    builder.register_handler("update_plan", plan_handler);

    if config.collaboration_modes_tools {
        builder.push_spec(create_request_user_input_tool());
        builder.register_handler("request_user_input", request_user_input_handler);
    }

    if config.search_tool {
        builder.push_spec_with_parallel_support(create_search_tool_bm25_tool(), true);
        builder.push_spec_with_parallel_support(create_claude_tool_search_alias_tool(), true);
        builder.register_handler("search_tool_bm25", search_tool_handler);
        builder.register_handler("ToolSearch", claude_tool_adapter_handler.clone());
    }

    if let Some(apply_patch_tool_type) = &config.apply_patch_tool_type {
        match apply_patch_tool_type {
            ApplyPatchToolType::Freeform => {
                builder.push_spec(create_apply_patch_freeform_tool());
            }
            ApplyPatchToolType::Function => {
                builder.push_spec(create_apply_patch_json_tool());
            }
        }
        builder.register_handler("apply_patch", apply_patch_handler);
    }

    builder.push_spec_with_parallel_support(create_batches_read_file_tool(), true);
    builder.register_handler("batches_read_file", batches_read_file_handler);

    if config
        .experimental_supported_tools
        .contains(&"grep_files".to_string())
    {
        let grep_files_handler = Arc::new(GrepFilesHandler);
        builder.push_spec_with_parallel_support(create_grep_files_tool(), true);
        builder.register_handler("grep_files", grep_files_handler);
    }

    if config
        .experimental_supported_tools
        .contains(&"read_file".to_string())
    {
        let read_file_handler = Arc::new(ReadFileHandler);
        builder.push_spec_with_parallel_support(create_read_file_tool(), true);
        builder.register_handler("read_file", read_file_handler);
    }

    if config
        .experimental_supported_tools
        .iter()
        .any(|tool| tool == "list_dir")
    {
        let list_dir_handler = Arc::new(ListDirHandler);
        builder.push_spec_with_parallel_support(create_list_dir_tool(), true);
        builder.register_handler("list_dir", list_dir_handler);
    }

    if config
        .experimental_supported_tools
        .contains(&"test_sync_tool".to_string())
    {
        let test_sync_handler = Arc::new(TestSyncHandler);
        builder.push_spec_with_parallel_support(create_test_sync_tool(), true);
        builder.register_handler("test_sync_tool", test_sync_handler);
    }

    match config.web_search_mode {
        Some(WebSearchMode::Cached) => {
            builder.push_spec(ToolSpec::WebSearch {
                external_web_access: Some(false),
            });
        }
        Some(WebSearchMode::Live) => {
            builder.push_spec(ToolSpec::WebSearch {
                external_web_access: Some(true),
            });
        }
        Some(WebSearchMode::Disabled) | None => {}
    }

    builder.push_spec_with_parallel_support(create_view_image_tool(), true);
    builder.register_handler("view_image", view_image_handler);

    if config.collab_tools {
        let collab_handler = Arc::new(CollabHandler);
        builder.push_spec(create_spawn_agent_tool());
        builder.push_spec(create_send_input_tool());
        builder.push_spec(create_task_batch_tool());
        builder.push_spec(create_task_send_batch_tool());
        builder.push_spec(create_resume_agent_tool());
        builder.push_spec_with_parallel_support(create_wait_tool(), true);
        builder.push_spec_with_parallel_support(create_wait_agents_tool(), true);
        builder.push_spec_with_parallel_support(create_list_agents_tool(), true);
        builder.push_spec(create_rename_agent_tool());
        builder.push_spec(create_close_agent_tool());
        builder.push_spec(create_close_agents_tool());
        builder.push_spec(create_claude_task_alias_tool());
        builder.push_spec_with_parallel_support(create_claude_task_output_alias_tool(), true);
        builder.push_spec(create_claude_task_stop_alias_tool());
        builder.push_spec(create_claude_skill_alias_tool());
        if config.collaboration_modes_tools {
            builder.push_spec(create_claude_ask_user_question_alias_tool());
        }
        builder.push_spec(create_claude_bash_alias_tool());
        builder.push_spec_with_parallel_support(create_claude_read_alias_tool(), true);
        builder.push_spec_with_parallel_support(create_claude_grep_alias_tool(), true);
        builder.push_spec(create_claude_todo_write_alias_tool());
        builder.push_spec(create_claude_enter_plan_mode_alias_tool());
        builder.push_spec(create_claude_exit_plan_mode_alias_tool());
        builder.push_spec(create_claude_write_tool());
        builder.push_spec(create_claude_edit_tool());
        builder.push_spec_with_parallel_support(create_claude_glob_tool(), true);
        builder.push_spec(create_claude_notebook_edit_tool());
        builder.register_handler("spawn_agent", collab_handler.clone());
        builder.register_handler("send_input", collab_handler.clone());
        builder.register_handler("task_batch", collab_batch_handler.clone());
        builder.register_handler("task_send_batch", collab_batch_handler);
        builder.register_handler("resume_agent", collab_handler.clone());
        builder.register_handler("wait", collab_handler.clone());
        builder.register_handler("wait_agents", collab_handler.clone());
        builder.register_handler("list_agents", collab_handler.clone());
        builder.register_handler("rename_agent", collab_handler.clone());
        builder.register_handler("close_agent", collab_handler.clone());
        builder.register_handler("close_agents", collab_handler);
        builder.register_handler("Task", claude_tool_adapter_handler.clone());
        builder.register_handler("TaskOutput", claude_tool_adapter_handler.clone());
        builder.register_handler("TaskStop", claude_tool_adapter_handler.clone());
        builder.register_handler("Skill", claude_tool_adapter_handler.clone());
        if config.collaboration_modes_tools {
            builder.register_handler("AskUserQuestion", claude_tool_adapter_handler.clone());
        }
        builder.register_handler("Bash", claude_tool_adapter_handler.clone());
        builder.register_handler("Read", claude_tool_adapter_handler.clone());
        builder.register_handler("Grep", claude_tool_adapter_handler.clone());
        builder.register_handler("TodoWrite", claude_tool_adapter_handler.clone());
        builder.register_handler("EnterPlanMode", claude_tool_adapter_handler.clone());
        builder.register_handler("ExitPlanMode", claude_tool_adapter_handler);
        builder.register_handler("Write", claude_write_handler);
        builder.register_handler("Edit", claude_edit_handler);
        builder.register_handler("Glob", claude_glob_handler);
        builder.register_handler("NotebookEdit", claude_notebook_edit_handler);
    }

    if let Some(mcp_tools) = mcp_tools {
        let mut entries: Vec<(String, rmcp::model::Tool)> = mcp_tools.into_iter().collect();
        entries.sort_by(|a, b| a.0.cmp(&b.0));

        for (name, tool) in entries.into_iter() {
            match mcp_tool_to_openai_tool(name.clone(), tool.clone()) {
                Ok(converted_tool) => {
                    builder.push_spec(ToolSpec::Function(converted_tool));
                    builder.register_handler(name, mcp_handler.clone());
                }
                Err(e) => {
                    tracing::error!("Failed to convert {name:?} MCP tool to OpenAI tool: {e:?}");
                }
            }
        }
    }

    if !dynamic_tools.is_empty() {
        for tool in dynamic_tools {
            match dynamic_tool_to_openai_tool(tool) {
                Ok(converted_tool) => {
                    builder.push_spec(ToolSpec::Function(converted_tool));
                    builder.register_handler(tool.name.clone(), dynamic_tool_handler.clone());
                }
                Err(e) => {
                    tracing::error!(
                        "Failed to convert dynamic tool {:?} to OpenAI tool: {e:?}",
                        tool.name
                    );
                }
            }
        }
    }

    builder
}

#[cfg(test)]
mod tests {
    use crate::client_common::tools::FreeformTool;
    use crate::config::test_config;
    use crate::models_manager::manager::ModelsManager;
    use crate::models_manager::model_info::with_config_overrides;
    use crate::tools::registry::ConfiguredToolSpec;
    use codex_protocol::openai_models::ModelInfo;
    use codex_protocol::openai_models::ModelsResponse;
    use pretty_assertions::assert_eq;

    use super::*;

    fn mcp_tool(
        name: &str,
        description: &str,
        input_schema: serde_json::Value,
    ) -> rmcp::model::Tool {
        rmcp::model::Tool {
            name: name.to_string().into(),
            title: None,
            description: Some(description.to_string().into()),
            input_schema: std::sync::Arc::new(rmcp::model::object(input_schema)),
            output_schema: None,
            annotations: None,
            icons: None,
            meta: None,
        }
    }

    #[test]
    fn mcp_tool_to_openai_tool_inserts_empty_properties() {
        let mut schema = rmcp::model::JsonObject::new();
        schema.insert("type".to_string(), serde_json::json!("object"));

        let tool = rmcp::model::Tool {
            name: "no_props".to_string().into(),
            title: None,
            description: Some("No properties".to_string().into()),
            input_schema: std::sync::Arc::new(schema),
            output_schema: None,
            annotations: None,
            icons: None,
            meta: None,
        };

        let openai_tool =
            mcp_tool_to_openai_tool("server/no_props".to_string(), tool).expect("convert tool");
        let parameters = serde_json::to_value(openai_tool.parameters).expect("serialize schema");

        assert_eq!(parameters.get("properties"), Some(&serde_json::json!({})));
    }

    fn tool_name(tool: &ToolSpec) -> &str {
        match tool {
            ToolSpec::Function(ResponsesApiTool { name, .. }) => name,
            ToolSpec::LocalShell {} => "local_shell",
            ToolSpec::WebSearch { .. } => "web_search",
            ToolSpec::Freeform(FreeformTool { name, .. }) => name,
        }
    }

    // Avoid order-based assertions; compare via set containment instead.
    fn assert_contains_tool_names(tools: &[ConfiguredToolSpec], expected_subset: &[&str]) {
        use std::collections::HashSet;
        let mut names = HashSet::new();
        let mut duplicates = Vec::new();
        for name in tools.iter().map(|t| tool_name(&t.spec)) {
            if !names.insert(name) {
                duplicates.push(name);
            }
        }
        assert!(
            duplicates.is_empty(),
            "duplicate tool entries detected: {duplicates:?}"
        );
        for expected in expected_subset {
            assert!(
                names.contains(expected),
                "expected tool {expected} to be present; had: {names:?}"
            );
        }
    }

    fn shell_tool_name(config: &ToolsConfig) -> Option<&'static str> {
        match config.shell_type {
            ConfigShellToolType::Default => Some("shell"),
            ConfigShellToolType::Local => Some("local_shell"),
            ConfigShellToolType::UnifiedExec => None,
            ConfigShellToolType::Disabled => None,
            ConfigShellToolType::ShellCommand => Some("shell_command"),
        }
    }

    fn find_tool<'a>(
        tools: &'a [ConfiguredToolSpec],
        expected_name: &str,
    ) -> &'a ConfiguredToolSpec {
        tools
            .iter()
            .find(|tool| tool_name(&tool.spec) == expected_name)
            .unwrap_or_else(|| panic!("expected tool {expected_name}"))
    }

    fn strip_descriptions_schema(schema: &mut JsonSchema) {
        match schema {
            JsonSchema::Boolean { description }
            | JsonSchema::String { description }
            | JsonSchema::Number { description } => {
                *description = None;
            }
            JsonSchema::Array { items, description } => {
                strip_descriptions_schema(items);
                *description = None;
            }
            JsonSchema::Object {
                properties,
                required: _,
                additional_properties,
            } => {
                for v in properties.values_mut() {
                    strip_descriptions_schema(v);
                }
                if let Some(AdditionalProperties::Schema(s)) = additional_properties {
                    strip_descriptions_schema(s);
                }
            }
        }
    }

    fn strip_descriptions_tool(spec: &mut ToolSpec) {
        match spec {
            ToolSpec::Function(ResponsesApiTool { parameters, .. }) => {
                strip_descriptions_schema(parameters);
            }
            ToolSpec::Freeform(_) | ToolSpec::LocalShell {} | ToolSpec::WebSearch { .. } => {}
        }
    }

    fn model_info_from_models_json(slug: &str) -> ModelInfo {
        let config = test_config();
        let response: ModelsResponse =
            serde_json::from_str(include_str!("../../models.json")).expect("valid models.json");
        let model = response
            .models
            .into_iter()
            .find(|candidate| candidate.slug == slug)
            .unwrap_or_else(|| panic!("model slug {slug} is missing from models.json"));
        with_config_overrides(model, &config)
    }

    #[test]
    fn test_batches_read_file_tool_schema() {
        let mut tool = create_batches_read_file_tool();
        strip_descriptions_tool(&mut tool);
        let ToolSpec::Function(ResponsesApiTool {
            name, parameters, ..
        }) = tool
        else {
            panic!("expected batches_read_file to be a function tool");
        };
        assert_eq!(name, "batches_read_file");
        let JsonSchema::Object {
            properties,
            required,
            additional_properties,
        } = parameters
        else {
            panic!("expected object schema for batches_read_file");
        };
        assert_eq!(required, Some(vec!["paths".to_string()]));
        assert_eq!(additional_properties, Some(false.into()));
        let paths_schema = properties
            .get("paths")
            .expect("paths property should exist");
        let JsonSchema::Array { items, .. } = paths_schema else {
            panic!("paths should be an array schema");
        };
        let JsonSchema::Object {
            properties: file_properties,
            required: file_required,
            additional_properties: file_additional_properties,
        } = items.as_ref()
        else {
            panic!("paths items should be object schemas");
        };
        assert_eq!(file_required, &Some(vec!["path".to_string()]));
        assert_eq!(file_additional_properties, &Some(false.into()));
        for key in ["path", "offset", "limit", "mode", "indentation"] {
            assert!(
                file_properties.contains_key(key),
                "paths items should define {key}"
            );
        }
        for key in ["offset", "limit", "mode", "indentation"] {
            assert!(
                properties.contains_key(key),
                "top-level schema should define {key}"
            );
        }
    }

    #[test]
    fn test_spawn_agent_tool_schema() {
        let mut tool = create_spawn_agent_tool();
        strip_descriptions_tool(&mut tool);
        let ToolSpec::Function(ResponsesApiTool {
            name, parameters, ..
        }) = tool
        else {
            panic!("expected spawn_agent to be a function tool");
        };
        assert_eq!(name, "spawn_agent");
        let JsonSchema::Object {
            properties,
            required,
            additional_properties,
        } = parameters
        else {
            panic!("expected object schema for spawn_agent");
        };
        assert_eq!(required, Some(vec!["items".to_string()]));
        assert_eq!(additional_properties, Some(false.into()));
        for key in [
            "items",
            "agent_type",
            "name",
            "preset",
            "acceptance_criteria",
            "test_commands",
            "allow_nested_agents",
            "model",
            "reasoning_effort",
            "reasoning_summary",
            "approval_policy",
            "sandbox_mode",
        ] {
            assert!(
                properties.contains_key(key),
                "spawn_agent schema should define {key}"
            );
        }
        assert!(
            !properties.contains_key("label"),
            "spawn_agent schema should not expose legacy label"
        );
    }

    #[test]
    fn test_rename_agent_tool_schema() {
        let mut tool = create_rename_agent_tool();
        strip_descriptions_tool(&mut tool);
        let ToolSpec::Function(ResponsesApiTool {
            name, parameters, ..
        }) = tool
        else {
            panic!("expected rename_agent to be a function tool");
        };
        assert_eq!(name, "rename_agent");
        let JsonSchema::Object {
            properties,
            required,
            additional_properties,
        } = parameters
        else {
            panic!("expected object schema for rename_agent");
        };
        assert_eq!(
            required,
            Some(vec!["agent_id".to_string(), "name".to_string()])
        );
        assert_eq!(additional_properties, Some(false.into()));
        for key in ["agent_id", "name"] {
            assert!(
                properties.contains_key(key),
                "rename_agent schema should define {key}"
            );
        }
        assert!(
            !properties.contains_key("id"),
            "rename_agent schema should not expose legacy id"
        );
    }

    #[test]
    fn test_task_batch_tool_schema() {
        let mut tool = create_task_batch_tool();
        strip_descriptions_tool(&mut tool);
        let ToolSpec::Function(ResponsesApiTool {
            name, parameters, ..
        }) = tool
        else {
            panic!("expected task_batch to be a function tool");
        };
        assert_eq!(name, "task_batch");
        let JsonSchema::Object {
            properties,
            required,
            additional_properties,
        } = parameters
        else {
            panic!("expected object schema for task_batch");
        };
        assert_eq!(required, Some(vec!["operations".to_string()]));
        assert_eq!(additional_properties, Some(false.into()));
        let operations = properties
            .get("operations")
            .expect("operations property should exist");
        let JsonSchema::Array { items, .. } = operations else {
            panic!("operations should be an array schema");
        };
        let JsonSchema::Object {
            properties: operation_properties,
            required: operation_required,
            additional_properties: operation_additional_properties,
        } = items.as_ref()
        else {
            panic!("operations items should be object schemas");
        };
        assert_eq!(operation_required, &Some(vec!["params".to_string()]));
        assert_eq!(operation_additional_properties, &Some(false.into()));
        assert!(operation_properties.contains_key("id"));
        let params = operation_properties
            .get("params")
            .expect("operation params should exist");
        let JsonSchema::Object {
            properties: params_properties,
            required: params_required,
            additional_properties: params_additional_properties,
        } = params
        else {
            panic!("operation params should be an object schema");
        };
        assert_eq!(params_required, &Some(vec!["items".to_string()]));
        assert_eq!(params_additional_properties, &Some(false.into()));
        assert!(params_properties.contains_key("items"));
        assert!(params_properties.contains_key("name"));
        assert!(params_properties.contains_key("preset"));
        assert!(!params_properties.contains_key("label"));
    }

    #[test]
    fn test_task_send_batch_tool_schema() {
        let mut tool = create_task_send_batch_tool();
        strip_descriptions_tool(&mut tool);
        let ToolSpec::Function(ResponsesApiTool {
            name, parameters, ..
        }) = tool
        else {
            panic!("expected task_send_batch to be a function tool");
        };
        assert_eq!(name, "task_send_batch");
        let JsonSchema::Object {
            properties,
            required,
            additional_properties,
        } = parameters
        else {
            panic!("expected object schema for task_send_batch");
        };
        assert_eq!(required, Some(vec!["operations".to_string()]));
        assert_eq!(additional_properties, Some(false.into()));
        let operations = properties
            .get("operations")
            .expect("operations property should exist");
        let JsonSchema::Array { items, .. } = operations else {
            panic!("operations should be an array schema");
        };
        let JsonSchema::Object {
            properties: operation_properties,
            required: operation_required,
            additional_properties: operation_additional_properties,
        } = items.as_ref()
        else {
            panic!("operations items should be object schemas");
        };
        assert_eq!(operation_required, &Some(vec!["params".to_string()]));
        assert_eq!(operation_additional_properties, &Some(false.into()));
        assert!(operation_properties.contains_key("id"));
        let params = operation_properties
            .get("params")
            .expect("operation params should exist");
        let JsonSchema::Object {
            properties: params_properties,
            required: params_required,
            additional_properties: params_additional_properties,
        } = params
        else {
            panic!("operation params should be an object schema");
        };
        assert_eq!(
            params_required,
            &Some(vec!["agent_id".to_string(), "items".to_string()])
        );
        assert_eq!(params_additional_properties, &Some(false.into()));
        assert!(params_properties.contains_key("agent_id"));
        assert!(params_properties.contains_key("items"));
        assert!(!params_properties.contains_key("id"));
    }

    #[test]
    fn test_claude_task_alias_tool_schema() {
        let mut tool = create_claude_task_alias_tool();
        strip_descriptions_tool(&mut tool);
        let ToolSpec::Function(ResponsesApiTool {
            name, parameters, ..
        }) = tool
        else {
            panic!("expected Task to be a function tool");
        };
        assert_eq!(name, "Task");
        let JsonSchema::Object {
            properties,
            required,
            additional_properties,
        } = parameters
        else {
            panic!("expected object schema for Task");
        };
        assert_eq!(
            required,
            Some(vec![
                "description".to_string(),
                "prompt".to_string(),
                "subagent_type".to_string(),
            ])
        );
        assert_eq!(additional_properties, Some(false.into()));
        for key in [
            "description",
            "prompt",
            "subagent_type",
            "name",
            "model",
            "preset",
            "max_turns",
            "mode",
            "resume",
            "run_in_background",
            "team_name",
        ] {
            assert!(
                properties.contains_key(key),
                "Task schema should define {key}"
            );
        }
    }

    #[test]
    fn test_claude_task_output_alias_tool_schema() {
        let mut tool = create_claude_task_output_alias_tool();
        strip_descriptions_tool(&mut tool);
        let ToolSpec::Function(ResponsesApiTool {
            name, parameters, ..
        }) = tool
        else {
            panic!("expected TaskOutput to be a function tool");
        };
        assert_eq!(name, "TaskOutput");
        let JsonSchema::Object {
            properties,
            required,
            additional_properties,
        } = parameters
        else {
            panic!("expected object schema for TaskOutput");
        };
        assert_eq!(required, Some(vec!["agent_id".to_string()]));
        assert_eq!(additional_properties, Some(false.into()));
        for key in ["agent_id", "block", "timeout"] {
            assert!(
                properties.contains_key(key),
                "TaskOutput schema should define {key}"
            );
        }
        assert!(
            !properties.contains_key("task_id"),
            "TaskOutput schema should not expose legacy task_id"
        );
    }

    #[test]
    fn test_claude_task_stop_alias_tool_schema() {
        let mut tool = create_claude_task_stop_alias_tool();
        strip_descriptions_tool(&mut tool);
        let ToolSpec::Function(ResponsesApiTool {
            name, parameters, ..
        }) = tool
        else {
            panic!("expected TaskStop to be a function tool");
        };
        assert_eq!(name, "TaskStop");
        let JsonSchema::Object {
            properties,
            required,
            additional_properties,
        } = parameters
        else {
            panic!("expected object schema for TaskStop");
        };
        assert_eq!(required, Some(vec!["agent_id".to_string()]));
        assert_eq!(additional_properties, Some(false.into()));
        assert!(properties.contains_key("agent_id"));
        assert!(
            !properties.contains_key("task_id"),
            "TaskStop schema should not expose legacy task_id"
        );
    }

    #[test]
    fn test_collab_agent_identifier_schemas() {
        let mut send_input = create_send_input_tool();
        strip_descriptions_tool(&mut send_input);
        let ToolSpec::Function(ResponsesApiTool {
            parameters: send_input_parameters,
            ..
        }) = send_input
        else {
            panic!("expected send_input to be a function tool");
        };
        let JsonSchema::Object {
            properties: send_input_properties,
            required: send_input_required,
            ..
        } = send_input_parameters
        else {
            panic!("expected object schema for send_input");
        };
        assert_eq!(
            send_input_required,
            Some(vec!["agent_id".to_string(), "items".to_string()])
        );
        assert!(send_input_properties.contains_key("agent_id"));
        assert!(!send_input_properties.contains_key("id"));

        let mut resume_agent = create_resume_agent_tool();
        strip_descriptions_tool(&mut resume_agent);
        let ToolSpec::Function(ResponsesApiTool {
            parameters: resume_agent_parameters,
            ..
        }) = resume_agent
        else {
            panic!("expected resume_agent to be a function tool");
        };
        let JsonSchema::Object {
            properties: resume_agent_properties,
            required: resume_agent_required,
            ..
        } = resume_agent_parameters
        else {
            panic!("expected object schema for resume_agent");
        };
        assert_eq!(resume_agent_required, Some(vec!["agent_id".to_string()]));
        assert!(resume_agent_properties.contains_key("agent_id"));
        assert!(!resume_agent_properties.contains_key("id"));

        let mut wait = create_wait_tool();
        strip_descriptions_tool(&mut wait);
        let ToolSpec::Function(ResponsesApiTool {
            parameters: wait_parameters,
            ..
        }) = wait
        else {
            panic!("expected wait to be a function tool");
        };
        let JsonSchema::Object {
            properties: wait_properties,
            required: wait_required,
            ..
        } = wait_parameters
        else {
            panic!("expected object schema for wait");
        };
        assert_eq!(wait_required, Some(vec!["agent_ids".to_string()]));
        assert!(wait_properties.contains_key("agent_ids"));
        assert!(!wait_properties.contains_key("ids"));

        let mut wait_agents = create_wait_agents_tool();
        strip_descriptions_tool(&mut wait_agents);
        let ToolSpec::Function(ResponsesApiTool {
            parameters: wait_agents_parameters,
            ..
        }) = wait_agents
        else {
            panic!("expected wait_agents to be a function tool");
        };
        let JsonSchema::Object {
            properties: wait_agents_properties,
            required: wait_agents_required,
            ..
        } = wait_agents_parameters
        else {
            panic!("expected object schema for wait_agents");
        };
        assert_eq!(wait_agents_required, None);
        assert!(wait_agents_properties.contains_key("agent_ids"));
        assert!(!wait_agents_properties.contains_key("ids"));

        let mut list_agents = create_list_agents_tool();
        strip_descriptions_tool(&mut list_agents);
        let ToolSpec::Function(ResponsesApiTool {
            parameters: list_agents_parameters,
            ..
        }) = list_agents
        else {
            panic!("expected list_agents to be a function tool");
        };
        let JsonSchema::Object {
            properties: list_agents_properties,
            required: list_agents_required,
            ..
        } = list_agents_parameters
        else {
            panic!("expected object schema for list_agents");
        };
        assert_eq!(list_agents_required, None);
        assert!(list_agents_properties.contains_key("agent_id"));
        assert!(!list_agents_properties.contains_key("creator_id"));

        let mut close_agent = create_close_agent_tool();
        strip_descriptions_tool(&mut close_agent);
        let ToolSpec::Function(ResponsesApiTool {
            parameters: close_agent_parameters,
            ..
        }) = close_agent
        else {
            panic!("expected close_agent to be a function tool");
        };
        let JsonSchema::Object {
            properties: close_agent_properties,
            required: close_agent_required,
            ..
        } = close_agent_parameters
        else {
            panic!("expected object schema for close_agent");
        };
        assert_eq!(close_agent_required, Some(vec!["agent_id".to_string()]));
        assert!(close_agent_properties.contains_key("agent_id"));
        assert!(!close_agent_properties.contains_key("id"));

        let mut close_agents = create_close_agents_tool();
        strip_descriptions_tool(&mut close_agents);
        let ToolSpec::Function(ResponsesApiTool {
            parameters: close_agents_parameters,
            ..
        }) = close_agents
        else {
            panic!("expected close_agents to be a function tool");
        };
        let JsonSchema::Object {
            properties: close_agents_properties,
            required: close_agents_required,
            ..
        } = close_agents_parameters
        else {
            panic!("expected object schema for close_agents");
        };
        assert_eq!(close_agents_required, Some(vec!["agent_ids".to_string()]));
        assert!(close_agents_properties.contains_key("agent_ids"));
        assert!(!close_agents_properties.contains_key("ids"));
    }

    #[test]
    fn test_full_toolset_specs_for_gpt5_codex_unified_exec_web_search() {
        let model_info = model_info_from_models_json("gpt-5-codex");
        let mut features = Features::with_defaults();
        features.enable(Feature::UnifiedExec);
        features.enable(Feature::CollaborationModes);
        let config = ToolsConfig::new(&ToolsConfigParams {
            model_info: &model_info,
            features: &features,
            web_search_mode: Some(WebSearchMode::Live),
        });
        let (tools, _) = build_specs(&config, None, &[]).build();

        // Build actual map name -> spec
        use std::collections::BTreeMap;
        use std::collections::HashSet;
        let mut actual: BTreeMap<String, ToolSpec> = BTreeMap::from([]);
        let mut duplicate_names = Vec::new();
        for t in &tools {
            let name = tool_name(&t.spec).to_string();
            if actual.insert(name.clone(), t.spec.clone()).is_some() {
                duplicate_names.push(name);
            }
        }
        assert!(
            duplicate_names.is_empty(),
            "duplicate tool entries detected: {duplicate_names:?}"
        );

        // Build expected from the same helpers used by the builder.
        let mut expected: BTreeMap<String, ToolSpec> = BTreeMap::from([]);
        for spec in [
            create_exec_command_tool(true),
            create_write_stdin_tool(),
            create_list_mcp_resources_tool(),
            create_list_mcp_resource_templates_tool(),
            create_read_mcp_resource_tool(),
            PLAN_TOOL.clone(),
            create_request_user_input_tool(),
            create_apply_patch_freeform_tool(),
            create_batches_read_file_tool(),
            ToolSpec::WebSearch {
                external_web_access: Some(true),
            },
            create_view_image_tool(),
        ] {
            expected.insert(tool_name(&spec).to_string(), spec);
        }

        // Exact name set match — this is the only test allowed to fail when tools change.
        let actual_names: HashSet<_> = actual.keys().cloned().collect();
        let expected_names: HashSet<_> = expected.keys().cloned().collect();
        assert_eq!(actual_names, expected_names, "tool name set mismatch");

        // Compare specs ignoring human-readable descriptions.
        for name in expected.keys() {
            let mut a = actual.get(name).expect("present").clone();
            let mut e = expected.get(name).expect("present").clone();
            strip_descriptions_tool(&mut a);
            strip_descriptions_tool(&mut e);
            assert_eq!(a, e, "spec mismatch for {name}");
        }
    }

    #[test]
    fn test_build_specs_collab_tools_enabled() {
        let config = test_config();
        let model_info = ModelsManager::construct_model_info_offline("gpt-5-codex", &config);
        let mut features = Features::with_defaults();
        features.enable(Feature::Collab);
        features.enable(Feature::CollaborationModes);
        let tools_config = ToolsConfig::new(&ToolsConfigParams {
            model_info: &model_info,
            features: &features,
            web_search_mode: Some(WebSearchMode::Cached),
        });
        let (tools, _) = build_specs(&tools_config, None, &[]).build();
        assert_contains_tool_names(
            &tools,
            &[
                "spawn_agent",
                "send_input",
                "task_batch",
                "task_send_batch",
                "resume_agent",
                "wait",
                "wait_agents",
                "list_agents",
                "rename_agent",
                "close_agent",
                "close_agents",
                "Task",
                "TaskOutput",
                "TaskStop",
                "Skill",
                "AskUserQuestion",
                "Bash",
                "Read",
                "Grep",
                "TodoWrite",
                "EnterPlanMode",
                "ExitPlanMode",
                "Write",
                "Edit",
                "Glob",
                "NotebookEdit",
            ],
        );
    }

    #[test]
    fn collab_waiting_tools_support_parallel_tool_calls() {
        let config = test_config();
        let model_info = ModelsManager::construct_model_info_offline("gpt-5-codex", &config);
        let mut features = Features::with_defaults();
        features.enable(Feature::Collab);
        features.enable(Feature::CollaborationModes);
        let tools_config = ToolsConfig::new(&ToolsConfigParams {
            model_info: &model_info,
            features: &features,
            web_search_mode: Some(WebSearchMode::Cached),
        });
        let (tools, _) = build_specs(&tools_config, None, &[]).build();

        assert!(find_tool(&tools, "wait").supports_parallel_tool_calls);
        assert!(find_tool(&tools, "wait_agents").supports_parallel_tool_calls);
        assert!(find_tool(&tools, "list_agents").supports_parallel_tool_calls);
        assert!(find_tool(&tools, "TaskOutput").supports_parallel_tool_calls);
        assert!(find_tool(&tools, "Read").supports_parallel_tool_calls);
        assert!(find_tool(&tools, "Grep").supports_parallel_tool_calls);
        assert!(find_tool(&tools, "Glob").supports_parallel_tool_calls);
        assert!(!find_tool(&tools, "spawn_agent").supports_parallel_tool_calls);
        assert!(!find_tool(&tools, "send_input").supports_parallel_tool_calls);
        assert!(!find_tool(&tools, "task_batch").supports_parallel_tool_calls);
        assert!(!find_tool(&tools, "task_send_batch").supports_parallel_tool_calls);
        assert!(!find_tool(&tools, "rename_agent").supports_parallel_tool_calls);
        assert!(!find_tool(&tools, "close_agent").supports_parallel_tool_calls);
        assert!(!find_tool(&tools, "close_agents").supports_parallel_tool_calls);
        assert!(!find_tool(&tools, "Task").supports_parallel_tool_calls);
        assert!(!find_tool(&tools, "TaskStop").supports_parallel_tool_calls);
        assert!(!find_tool(&tools, "Skill").supports_parallel_tool_calls);
        assert!(!find_tool(&tools, "AskUserQuestion").supports_parallel_tool_calls);
        assert!(!find_tool(&tools, "Bash").supports_parallel_tool_calls);
        assert!(!find_tool(&tools, "TodoWrite").supports_parallel_tool_calls);
        assert!(!find_tool(&tools, "EnterPlanMode").supports_parallel_tool_calls);
        assert!(!find_tool(&tools, "ExitPlanMode").supports_parallel_tool_calls);
        assert!(!find_tool(&tools, "Write").supports_parallel_tool_calls);
        assert!(!find_tool(&tools, "Edit").supports_parallel_tool_calls);
        assert!(!find_tool(&tools, "NotebookEdit").supports_parallel_tool_calls);
    }

    #[test]
    fn request_user_input_requires_collaboration_modes_feature() {
        let config = test_config();
        let model_info = ModelsManager::construct_model_info_offline("gpt-5-codex", &config);
        let mut features = Features::with_defaults();
        features.disable(Feature::CollaborationModes);
        let tools_config = ToolsConfig::new(&ToolsConfigParams {
            model_info: &model_info,
            features: &features,
            web_search_mode: Some(WebSearchMode::Cached),
        });
        let (tools, _) = build_specs(&tools_config, None, &[]).build();
        assert!(
            !tools.iter().any(|t| t.spec.name() == "request_user_input"),
            "request_user_input should be disabled when collaboration_modes feature is off"
        );

        features.enable(Feature::CollaborationModes);
        let tools_config = ToolsConfig::new(&ToolsConfigParams {
            model_info: &model_info,
            features: &features,
            web_search_mode: Some(WebSearchMode::Cached),
        });
        let (tools, _) = build_specs(&tools_config, None, &[]).build();
        assert_contains_tool_names(&tools, &["request_user_input"]);
    }

    fn assert_model_tools(
        model_slug: &str,
        features: &Features,
        web_search_mode: Option<WebSearchMode>,
        expected_tools: &[&str],
    ) {
        let model_info = model_info_from_models_json(model_slug);
        let tools_config = ToolsConfig::new(&ToolsConfigParams {
            model_info: &model_info,
            features,
            web_search_mode,
        });
        let (tools, _) = build_specs(&tools_config, Some(HashMap::new()), &[]).build();
        let tool_names = tools.iter().map(|t| t.spec.name()).collect::<Vec<_>>();
        assert_eq!(&tool_names, &expected_tools,);
    }

    fn assert_default_model_tools(
        model_slug: &str,
        features: &Features,
        web_search_mode: Option<WebSearchMode>,
        shell_tool: &'static str,
        expected_tail: &[&str],
    ) {
        let mut expected = if features.enabled(Feature::UnifiedExec) {
            vec!["exec_command", "write_stdin"]
        } else {
            vec![shell_tool]
        };
        expected.extend(expected_tail);
        assert_model_tools(model_slug, features, web_search_mode, &expected);
    }

    #[test]
    fn web_search_mode_cached_sets_external_web_access_false() {
        let config = test_config();
        let model_info = ModelsManager::construct_model_info_offline("gpt-5-codex", &config);
        let features = Features::with_defaults();

        let tools_config = ToolsConfig::new(&ToolsConfigParams {
            model_info: &model_info,
            features: &features,
            web_search_mode: Some(WebSearchMode::Cached),
        });
        let (tools, _) = build_specs(&tools_config, None, &[]).build();

        let tool = find_tool(&tools, "web_search");
        assert_eq!(
            tool.spec,
            ToolSpec::WebSearch {
                external_web_access: Some(false),
            }
        );
    }

    #[test]
    fn web_search_mode_live_sets_external_web_access_true() {
        let config = test_config();
        let model_info = ModelsManager::construct_model_info_offline("gpt-5-codex", &config);
        let features = Features::with_defaults();

        let tools_config = ToolsConfig::new(&ToolsConfigParams {
            model_info: &model_info,
            features: &features,
            web_search_mode: Some(WebSearchMode::Live),
        });
        let (tools, _) = build_specs(&tools_config, None, &[]).build();

        let tool = find_tool(&tools, "web_search");
        assert_eq!(
            tool.spec,
            ToolSpec::WebSearch {
                external_web_access: Some(true),
            }
        );
    }

    #[test]
    fn test_build_specs_gpt5_codex_default() {
        let mut features = Features::with_defaults();
        features.enable(Feature::CollaborationModes);
        assert_default_model_tools(
            "gpt-5-codex",
            &features,
            Some(WebSearchMode::Cached),
            "shell_command",
            &[
                "list_mcp_resources",
                "list_mcp_resource_templates",
                "read_mcp_resource",
                "update_plan",
                "request_user_input",
                "apply_patch",
                "batches_read_file",
                "web_search",
                "view_image",
            ],
        );
    }

    #[test]
    fn test_build_specs_gpt51_codex_default() {
        let mut features = Features::with_defaults();
        features.enable(Feature::CollaborationModes);
        assert_default_model_tools(
            "gpt-5.1-codex",
            &features,
            Some(WebSearchMode::Cached),
            "shell_command",
            &[
                "list_mcp_resources",
                "list_mcp_resource_templates",
                "read_mcp_resource",
                "update_plan",
                "request_user_input",
                "apply_patch",
                "batches_read_file",
                "web_search",
                "view_image",
            ],
        );
    }

    #[test]
    fn test_build_specs_gpt5_codex_unified_exec_web_search() {
        let mut features = Features::with_defaults();
        features.enable(Feature::UnifiedExec);
        features.enable(Feature::CollaborationModes);
        assert_model_tools(
            "gpt-5-codex",
            &features,
            Some(WebSearchMode::Live),
            &[
                "exec_command",
                "write_stdin",
                "list_mcp_resources",
                "list_mcp_resource_templates",
                "read_mcp_resource",
                "update_plan",
                "request_user_input",
                "apply_patch",
                "batches_read_file",
                "web_search",
                "view_image",
            ],
        );
    }

    #[test]
    fn test_build_specs_gpt51_codex_unified_exec_web_search() {
        let mut features = Features::with_defaults();
        features.enable(Feature::UnifiedExec);
        features.enable(Feature::CollaborationModes);
        assert_model_tools(
            "gpt-5.1-codex",
            &features,
            Some(WebSearchMode::Live),
            &[
                "exec_command",
                "write_stdin",
                "list_mcp_resources",
                "list_mcp_resource_templates",
                "read_mcp_resource",
                "update_plan",
                "request_user_input",
                "apply_patch",
                "batches_read_file",
                "web_search",
                "view_image",
            ],
        );
    }

    #[test]
    fn test_gpt_5_1_codex_max_defaults() {
        let mut features = Features::with_defaults();
        features.enable(Feature::CollaborationModes);
        assert_default_model_tools(
            "gpt-5.1-codex-max",
            &features,
            Some(WebSearchMode::Cached),
            "shell_command",
            &[
                "list_mcp_resources",
                "list_mcp_resource_templates",
                "read_mcp_resource",
                "update_plan",
                "request_user_input",
                "apply_patch",
                "batches_read_file",
                "web_search",
                "view_image",
            ],
        );
    }

    #[test]
    fn test_codex_5_1_mini_defaults() {
        let mut features = Features::with_defaults();
        features.enable(Feature::CollaborationModes);
        assert_default_model_tools(
            "gpt-5.1-codex-mini",
            &features,
            Some(WebSearchMode::Cached),
            "shell_command",
            &[
                "list_mcp_resources",
                "list_mcp_resource_templates",
                "read_mcp_resource",
                "update_plan",
                "request_user_input",
                "apply_patch",
                "batches_read_file",
                "web_search",
                "view_image",
            ],
        );
    }

    #[test]
    fn test_gpt_5_defaults() {
        let mut features = Features::with_defaults();
        features.enable(Feature::CollaborationModes);
        assert_default_model_tools(
            "gpt-5",
            &features,
            Some(WebSearchMode::Cached),
            "shell",
            &[
                "list_mcp_resources",
                "list_mcp_resource_templates",
                "read_mcp_resource",
                "update_plan",
                "request_user_input",
                "batches_read_file",
                "web_search",
                "view_image",
            ],
        );
    }

    #[test]
    fn test_gpt_5_1_defaults() {
        let mut features = Features::with_defaults();
        features.enable(Feature::CollaborationModes);
        assert_default_model_tools(
            "gpt-5.1",
            &features,
            Some(WebSearchMode::Cached),
            "shell_command",
            &[
                "list_mcp_resources",
                "list_mcp_resource_templates",
                "read_mcp_resource",
                "update_plan",
                "request_user_input",
                "apply_patch",
                "batches_read_file",
                "web_search",
                "view_image",
            ],
        );
    }

    #[test]
    fn test_gpt_5_1_codex_max_unified_exec_web_search() {
        let mut features = Features::with_defaults();
        features.enable(Feature::UnifiedExec);
        features.enable(Feature::CollaborationModes);
        assert_model_tools(
            "gpt-5.1-codex-max",
            &features,
            Some(WebSearchMode::Live),
            &[
                "exec_command",
                "write_stdin",
                "list_mcp_resources",
                "list_mcp_resource_templates",
                "read_mcp_resource",
                "update_plan",
                "request_user_input",
                "apply_patch",
                "batches_read_file",
                "web_search",
                "view_image",
            ],
        );
    }

    #[test]
    fn test_build_specs_default_shell_present() {
        let config = test_config();
        let model_info = ModelsManager::construct_model_info_offline("o3", &config);
        let mut features = Features::with_defaults();
        features.enable(Feature::UnifiedExec);
        let tools_config = ToolsConfig::new(&ToolsConfigParams {
            model_info: &model_info,
            features: &features,
            web_search_mode: Some(WebSearchMode::Live),
        });
        let (tools, _) = build_specs(&tools_config, Some(HashMap::new()), &[]).build();

        // Only check the shell variant and a couple of core tools.
        let mut subset = vec!["exec_command", "write_stdin", "update_plan"];
        if let Some(shell_tool) = shell_tool_name(&tools_config) {
            subset.push(shell_tool);
        }
        assert_contains_tool_names(&tools, &subset);
    }

    #[test]
    #[ignore]
    fn test_parallel_support_flags() {
        let config = test_config();
        let model_info = ModelsManager::construct_model_info_offline("gpt-5-codex", &config);
        let mut features = Features::with_defaults();
        features.enable(Feature::UnifiedExec);
        let tools_config = ToolsConfig::new(&ToolsConfigParams {
            model_info: &model_info,
            features: &features,
            web_search_mode: Some(WebSearchMode::Cached),
        });
        let (tools, _) = build_specs(&tools_config, None, &[]).build();

        assert!(find_tool(&tools, "exec_command").supports_parallel_tool_calls);
        assert!(!find_tool(&tools, "write_stdin").supports_parallel_tool_calls);
        assert!(find_tool(&tools, "batches_read_file").supports_parallel_tool_calls);
        assert!(find_tool(&tools, "grep_files").supports_parallel_tool_calls);
        assert!(find_tool(&tools, "list_dir").supports_parallel_tool_calls);
        assert!(find_tool(&tools, "read_file").supports_parallel_tool_calls);
    }

    #[test]
    fn test_test_model_info_includes_sync_tool() {
        let mut model_info = model_info_from_models_json("gpt-5-codex");
        model_info.experimental_supported_tools = vec![
            "test_sync_tool".to_string(),
            "read_file".to_string(),
            "grep_files".to_string(),
            "list_dir".to_string(),
        ];
        let features = Features::with_defaults();
        let tools_config = ToolsConfig::new(&ToolsConfigParams {
            model_info: &model_info,
            features: &features,
            web_search_mode: Some(WebSearchMode::Cached),
        });
        let (tools, _) = build_specs(&tools_config, None, &[]).build();

        assert!(
            tools
                .iter()
                .any(|tool| tool_name(&tool.spec) == "test_sync_tool")
        );
        assert!(
            tools
                .iter()
                .any(|tool| tool_name(&tool.spec) == "read_file")
        );
        assert!(
            tools
                .iter()
                .any(|tool| tool_name(&tool.spec) == "batches_read_file")
        );
        assert!(
            tools
                .iter()
                .any(|tool| tool_name(&tool.spec) == "grep_files")
        );
        assert!(tools.iter().any(|tool| tool_name(&tool.spec) == "list_dir"));
    }

    #[test]
    fn test_build_specs_mcp_tools_converted() {
        let config = test_config();
        let model_info = ModelsManager::construct_model_info_offline("o3", &config);
        let mut features = Features::with_defaults();
        features.enable(Feature::UnifiedExec);
        let tools_config = ToolsConfig::new(&ToolsConfigParams {
            model_info: &model_info,
            features: &features,
            web_search_mode: Some(WebSearchMode::Live),
        });
        let (tools, _) = build_specs(
            &tools_config,
            Some(HashMap::from([(
                "test_server/do_something_cool".to_string(),
                mcp_tool(
                    "do_something_cool",
                    "Do something cool",
                    serde_json::json!({
                        "type": "object",
                        "properties": {
                            "string_argument": { "type": "string" },
                            "number_argument": { "type": "number" },
                            "object_argument": {
                                "type": "object",
                                "properties": {
                                    "string_property": { "type": "string" },
                                    "number_property": { "type": "number" },
                                },
                                "required": ["string_property", "number_property"],
                                "additionalProperties": false,
                            },
                        },
                    }),
                ),
            )])),
            &[],
        )
        .build();

        let tool = find_tool(&tools, "test_server/do_something_cool");
        assert_eq!(
            &tool.spec,
            &ToolSpec::Function(ResponsesApiTool {
                name: "test_server/do_something_cool".to_string(),
                parameters: JsonSchema::Object {
                    properties: BTreeMap::from([
                        (
                            "string_argument".to_string(),
                            JsonSchema::String { description: None }
                        ),
                        (
                            "number_argument".to_string(),
                            JsonSchema::Number { description: None }
                        ),
                        (
                            "object_argument".to_string(),
                            JsonSchema::Object {
                                properties: BTreeMap::from([
                                    (
                                        "string_property".to_string(),
                                        JsonSchema::String { description: None }
                                    ),
                                    (
                                        "number_property".to_string(),
                                        JsonSchema::Number { description: None }
                                    ),
                                ]),
                                required: Some(vec![
                                    "string_property".to_string(),
                                    "number_property".to_string(),
                                ]),
                                additional_properties: Some(false.into()),
                            },
                        ),
                    ]),
                    required: None,
                    additional_properties: None,
                },
                description: "Do something cool".to_string(),
                strict: false,
            })
        );
    }

    #[test]
    fn test_build_specs_mcp_tools_sorted_by_name() {
        let config = test_config();
        let model_info = ModelsManager::construct_model_info_offline("o3", &config);
        let mut features = Features::with_defaults();
        features.enable(Feature::UnifiedExec);
        let tools_config = ToolsConfig::new(&ToolsConfigParams {
            model_info: &model_info,
            features: &features,
            web_search_mode: Some(WebSearchMode::Cached),
        });

        // Intentionally construct a map with keys that would sort alphabetically.
        let tools_map: HashMap<String, rmcp::model::Tool> = HashMap::from([
            (
                "test_server/do".to_string(),
                mcp_tool("a", "a", serde_json::json!({"type": "object"})),
            ),
            (
                "test_server/something".to_string(),
                mcp_tool("b", "b", serde_json::json!({"type": "object"})),
            ),
            (
                "test_server/cool".to_string(),
                mcp_tool("c", "c", serde_json::json!({"type": "object"})),
            ),
        ]);

        let (tools, _) = build_specs(&tools_config, Some(tools_map), &[]).build();

        // Only assert that the MCP tools themselves are sorted by fully-qualified name.
        let mcp_names: Vec<_> = tools
            .iter()
            .map(|t| tool_name(&t.spec).to_string())
            .filter(|n| n.starts_with("test_server/"))
            .collect();
        let expected = vec![
            "test_server/cool".to_string(),
            "test_server/do".to_string(),
            "test_server/something".to_string(),
        ];
        assert_eq!(mcp_names, expected);
    }

    #[test]
    fn test_mcp_tool_property_missing_type_defaults_to_string() {
        let config = test_config();
        let model_info = ModelsManager::construct_model_info_offline("gpt-5-codex", &config);
        let mut features = Features::with_defaults();
        features.enable(Feature::UnifiedExec);
        let tools_config = ToolsConfig::new(&ToolsConfigParams {
            model_info: &model_info,
            features: &features,
            web_search_mode: Some(WebSearchMode::Cached),
        });

        let (tools, _) = build_specs(
            &tools_config,
            Some(HashMap::from([(
                "dash/search".to_string(),
                mcp_tool(
                    "search",
                    "Search docs",
                    serde_json::json!({
                        "type": "object",
                        "properties": {
                            "query": {"description": "search query"}
                        }
                    }),
                ),
            )])),
            &[],
        )
        .build();

        let tool = find_tool(&tools, "dash/search");
        assert_eq!(
            tool.spec,
            ToolSpec::Function(ResponsesApiTool {
                name: "dash/search".to_string(),
                parameters: JsonSchema::Object {
                    properties: BTreeMap::from([(
                        "query".to_string(),
                        JsonSchema::String {
                            description: Some("search query".to_string())
                        }
                    )]),
                    required: None,
                    additional_properties: None,
                },
                description: "Search docs".to_string(),
                strict: false,
            })
        );
    }

    #[test]
    fn test_mcp_tool_integer_normalized_to_number() {
        let config = test_config();
        let model_info = ModelsManager::construct_model_info_offline("gpt-5-codex", &config);
        let mut features = Features::with_defaults();
        features.enable(Feature::UnifiedExec);
        let tools_config = ToolsConfig::new(&ToolsConfigParams {
            model_info: &model_info,
            features: &features,
            web_search_mode: Some(WebSearchMode::Cached),
        });

        let (tools, _) = build_specs(
            &tools_config,
            Some(HashMap::from([(
                "dash/paginate".to_string(),
                mcp_tool(
                    "paginate",
                    "Pagination",
                    serde_json::json!({
                        "type": "object",
                        "properties": {"page": {"type": "integer"}}
                    }),
                ),
            )])),
            &[],
        )
        .build();

        let tool = find_tool(&tools, "dash/paginate");
        assert_eq!(
            tool.spec,
            ToolSpec::Function(ResponsesApiTool {
                name: "dash/paginate".to_string(),
                parameters: JsonSchema::Object {
                    properties: BTreeMap::from([(
                        "page".to_string(),
                        JsonSchema::Number { description: None }
                    )]),
                    required: None,
                    additional_properties: None,
                },
                description: "Pagination".to_string(),
                strict: false,
            })
        );
    }

    #[test]
    fn test_mcp_tool_array_without_items_gets_default_string_items() {
        let config = test_config();
        let model_info = ModelsManager::construct_model_info_offline("gpt-5-codex", &config);
        let mut features = Features::with_defaults();
        features.enable(Feature::UnifiedExec);
        features.enable(Feature::ApplyPatchFreeform);
        let tools_config = ToolsConfig::new(&ToolsConfigParams {
            model_info: &model_info,
            features: &features,
            web_search_mode: Some(WebSearchMode::Cached),
        });

        let (tools, _) = build_specs(
            &tools_config,
            Some(HashMap::from([(
                "dash/tags".to_string(),
                mcp_tool(
                    "tags",
                    "Tags",
                    serde_json::json!({
                        "type": "object",
                        "properties": {"tags": {"type": "array"}}
                    }),
                ),
            )])),
            &[],
        )
        .build();

        let tool = find_tool(&tools, "dash/tags");
        assert_eq!(
            tool.spec,
            ToolSpec::Function(ResponsesApiTool {
                name: "dash/tags".to_string(),
                parameters: JsonSchema::Object {
                    properties: BTreeMap::from([(
                        "tags".to_string(),
                        JsonSchema::Array {
                            items: Box::new(JsonSchema::String { description: None }),
                            description: None
                        }
                    )]),
                    required: None,
                    additional_properties: None,
                },
                description: "Tags".to_string(),
                strict: false,
            })
        );
    }

    #[test]
    fn test_mcp_tool_anyof_defaults_to_string() {
        let config = test_config();
        let model_info = ModelsManager::construct_model_info_offline("gpt-5-codex", &config);
        let mut features = Features::with_defaults();
        features.enable(Feature::UnifiedExec);
        let tools_config = ToolsConfig::new(&ToolsConfigParams {
            model_info: &model_info,
            features: &features,
            web_search_mode: Some(WebSearchMode::Cached),
        });

        let (tools, _) = build_specs(
            &tools_config,
            Some(HashMap::from([(
                "dash/value".to_string(),
                mcp_tool(
                    "value",
                    "AnyOf Value",
                    serde_json::json!({
                        "type": "object",
                        "properties": {
                            "value": {"anyOf": [{"type": "string"}, {"type": "number"}]}
                        }
                    }),
                ),
            )])),
            &[],
        )
        .build();

        let tool = find_tool(&tools, "dash/value");
        assert_eq!(
            tool.spec,
            ToolSpec::Function(ResponsesApiTool {
                name: "dash/value".to_string(),
                parameters: JsonSchema::Object {
                    properties: BTreeMap::from([(
                        "value".to_string(),
                        JsonSchema::String { description: None }
                    )]),
                    required: None,
                    additional_properties: None,
                },
                description: "AnyOf Value".to_string(),
                strict: false,
            })
        );
    }

    #[test]
    fn test_shell_tool() {
        let tool = super::create_shell_tool(true);
        let ToolSpec::Function(ResponsesApiTool {
            description, name, ..
        }) = &tool
        else {
            panic!("expected function tool");
        };
        assert_eq!(name, "shell");

        let expected = if cfg!(windows) {
            r#"Runs a Powershell command (Windows) and returns its output. Arguments to `shell` will be passed to CreateProcessW(). Most commands should be prefixed with ["powershell.exe", "-Command"].
        
Examples of valid command strings:

- ls -a (show hidden): ["powershell.exe", "-Command", "Get-ChildItem -Force"]
- recursive find by name: ["powershell.exe", "-Command", "Get-ChildItem -Recurse -Filter *.py"]
- recursive grep: ["powershell.exe", "-Command", "Get-ChildItem -Path C:\\myrepo -Recurse | Select-String -Pattern 'TODO' -CaseSensitive"]
- ps aux | grep python: ["powershell.exe", "-Command", "Get-Process | Where-Object { $_.ProcessName -like '*python*' }"]
- setting an env var: ["powershell.exe", "-Command", "$env:FOO='bar'; echo $env:FOO"]
- running an inline Python script: ["powershell.exe", "-Command", "@'\\nprint('Hello, world!')\\n'@ | python -"]"#
        } else {
            r#"Runs a shell command and returns its output.
- The arguments to `shell` will be passed to execvp(). Most terminal commands should be prefixed with ["bash", "-lc"].
- Always set the `workdir` param when using the shell function. Do not use `cd` unless absolutely necessary."#
        }.to_string();
        assert_eq!(description, &expected);
    }

    #[test]
    fn test_shell_command_tool() {
        let tool = super::create_shell_command_tool(true);
        let ToolSpec::Function(ResponsesApiTool {
            description, name, ..
        }) = &tool
        else {
            panic!("expected function tool");
        };
        assert_eq!(name, "shell_command");

        let expected = if cfg!(windows) {
            r#"Runs a Powershell command (Windows) and returns its output.
        
Examples of valid command strings:

- ls -a (show hidden): "Get-ChildItem -Force"
- recursive find by name: "Get-ChildItem -Recurse -Filter *.py"
- recursive grep: "Get-ChildItem -Path C:\\myrepo -Recurse | Select-String -Pattern 'TODO' -CaseSensitive"
- ps aux | grep python: "Get-Process | Where-Object { $_.ProcessName -like '*python*' }"
- setting an env var: "$env:FOO='bar'; echo $env:FOO"
- running an inline Python script: "@'\\nprint('Hello, world!')\\n'@ | python -"#.to_string()
        } else {
            r#"Runs a shell command and returns its output.
- Always set the `workdir` param when using the shell_command function. Do not use `cd` unless absolutely necessary."#.to_string()
        };
        assert_eq!(description, &expected);
    }

    #[test]
    fn test_get_openai_tools_mcp_tools_with_additional_properties_schema() {
        let config = test_config();
        let model_info = ModelsManager::construct_model_info_offline("gpt-5-codex", &config);
        let mut features = Features::with_defaults();
        features.enable(Feature::UnifiedExec);
        let tools_config = ToolsConfig::new(&ToolsConfigParams {
            model_info: &model_info,
            features: &features,
            web_search_mode: Some(WebSearchMode::Cached),
        });
        let (tools, _) = build_specs(
            &tools_config,
            Some(HashMap::from([(
                "test_server/do_something_cool".to_string(),
                mcp_tool(
                    "do_something_cool",
                    "Do something cool",
                    serde_json::json!({
                        "type": "object",
                        "properties": {
                            "string_argument": {"type": "string"},
                            "number_argument": {"type": "number"},
                            "object_argument": {
                                "type": "object",
                                "properties": {
                                    "string_property": {"type": "string"},
                                    "number_property": {"type": "number"}
                                },
                                "required": ["string_property", "number_property"],
                                "additionalProperties": {
                                    "type": "object",
                                    "properties": {
                                        "addtl_prop": {"type": "string"}
                                    },
                                    "required": ["addtl_prop"],
                                    "additionalProperties": false
                                }
                            }
                        }
                    }),
                ),
            )])),
            &[],
        )
        .build();

        let tool = find_tool(&tools, "test_server/do_something_cool");
        assert_eq!(
            tool.spec,
            ToolSpec::Function(ResponsesApiTool {
                name: "test_server/do_something_cool".to_string(),
                parameters: JsonSchema::Object {
                    properties: BTreeMap::from([
                        (
                            "string_argument".to_string(),
                            JsonSchema::String { description: None }
                        ),
                        (
                            "number_argument".to_string(),
                            JsonSchema::Number { description: None }
                        ),
                        (
                            "object_argument".to_string(),
                            JsonSchema::Object {
                                properties: BTreeMap::from([
                                    (
                                        "string_property".to_string(),
                                        JsonSchema::String { description: None }
                                    ),
                                    (
                                        "number_property".to_string(),
                                        JsonSchema::Number { description: None }
                                    ),
                                ]),
                                required: Some(vec![
                                    "string_property".to_string(),
                                    "number_property".to_string(),
                                ]),
                                additional_properties: Some(
                                    JsonSchema::Object {
                                        properties: BTreeMap::from([(
                                            "addtl_prop".to_string(),
                                            JsonSchema::String { description: None }
                                        ),]),
                                        required: Some(vec!["addtl_prop".to_string(),]),
                                        additional_properties: Some(false.into()),
                                    }
                                    .into()
                                ),
                            },
                        ),
                    ]),
                    required: None,
                    additional_properties: None,
                },
                description: "Do something cool".to_string(),
                strict: false,
            })
        );
    }

    #[test]
    fn chat_tools_include_top_level_name() {
        let properties =
            BTreeMap::from([("foo".to_string(), JsonSchema::String { description: None })]);
        let tools = vec![ToolSpec::Function(ResponsesApiTool {
            name: "demo".to_string(),
            description: "A demo tool".to_string(),
            strict: false,
            parameters: JsonSchema::Object {
                properties,
                required: None,
                additional_properties: None,
            },
        })];

        let responses_json = create_tools_json_for_responses_api(&tools).unwrap();
        assert_eq!(
            responses_json,
            vec![json!({
                "type": "function",
                "name": "demo",
                "description": "A demo tool",
                "strict": false,
                "parameters": {
                    "type": "object",
                    "properties": {
                        "foo": { "type": "string" }
                    },
                },
            })]
        );
    }
}
