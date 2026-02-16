#![cfg(not(target_os = "windows"))]

use anyhow::Result;
use codex_core::CodexAuth;
use codex_core::features::Feature;
use codex_protocol::openai_models::ConfigShellToolType;
use codex_protocol::openai_models::ModelInfo;
use codex_protocol::openai_models::ModelVisibility;
use codex_protocol::openai_models::ModelsResponse;
use codex_protocol::openai_models::ReasoningEffort;
use codex_protocol::openai_models::ReasoningEffortPreset;
use codex_protocol::openai_models::TruncationPolicyConfig;
use codex_protocol::openai_models::default_input_modalities;
use core_test_support::responses::mount_function_call_agent_response;
use core_test_support::responses::mount_models_once;
use core_test_support::skip_if_no_network;
use core_test_support::test_codex::TestCodex;
use core_test_support::test_codex::test_codex;
use std::collections::HashSet;
use std::path::Path;
use std::process::Command as StdCommand;
use wiremock::BodyPrintLimit;
use wiremock::MockServer;

const MODEL_WITH_TOOL: &str = "test-gpt-5.1-codex";

fn ripgrep_available() -> bool {
    StdCommand::new("rg")
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

macro_rules! skip_if_ripgrep_missing {
    ($ret:expr $(,)?) => {{
        if !ripgrep_available() {
            eprintln!("rg not available in PATH; skipping test");
            return $ret;
        }
    }};
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn grep_files_tool_collects_matches() -> Result<()> {
    skip_if_no_network!(Ok(()));
    skip_if_ripgrep_missing!(Ok(()));

    let server = MockServer::builder()
        .body_print_limit(BodyPrintLimit::Limited(80_000))
        .start()
        .await;
    let test = build_test_codex(&server).await?;

    let search_dir = test.cwd.path().join("src");
    std::fs::create_dir_all(&search_dir)?;
    let alpha = search_dir.join("alpha.rs");
    let beta = search_dir.join("beta.rs");
    let gamma = search_dir.join("gamma.txt");
    std::fs::write(&alpha, "alpha needle\n")?;
    std::fs::write(&beta, "beta needle\n")?;
    std::fs::write(&gamma, "needle in text but excluded\n")?;

    let call_id = "grep-files-collect";
    let arguments = serde_json::json!({
        "pattern": "needle",
        "path": search_dir.to_string_lossy(),
        "include": "*.rs",
    })
    .to_string();

    let mocks =
        mount_function_call_agent_response(&server, call_id, &arguments, "grep_files").await;
    test.submit_turn("please find uses of needle").await?;

    let req = mocks.completion.single_request();
    let (content_opt, success_opt) = req
        .function_call_output_content_and_success(call_id)
        .expect("tool output present");
    let content = content_opt.expect("content present");
    let success = success_opt.unwrap_or(true);
    assert!(
        success,
        "expected success for matches, got content={content}"
    );

    let entries = collect_file_names(&content);
    assert_eq!(entries.len(), 2, "content: {content}");
    assert!(
        entries.contains("alpha.rs"),
        "missing alpha.rs in {entries:?}"
    );
    assert!(
        entries.contains("beta.rs"),
        "missing beta.rs in {entries:?}"
    );
    assert!(
        !entries.contains("gamma.txt"),
        "txt file should be filtered out: {entries:?}"
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn grep_files_tool_reports_empty_results() -> Result<()> {
    skip_if_no_network!(Ok(()));
    skip_if_ripgrep_missing!(Ok(()));

    let server = MockServer::builder()
        .body_print_limit(BodyPrintLimit::Limited(80_000))
        .start()
        .await;
    let test = build_test_codex(&server).await?;

    let search_dir = test.cwd.path().join("logs");
    std::fs::create_dir_all(&search_dir)?;
    std::fs::write(search_dir.join("output.txt"), "no hits here")?;

    let call_id = "grep-files-empty";
    let arguments = serde_json::json!({
        "pattern": "needle",
        "path": search_dir.to_string_lossy(),
        "limit": 5,
    })
    .to_string();

    let mocks =
        mount_function_call_agent_response(&server, call_id, &arguments, "grep_files").await;
    test.submit_turn("search again").await?;

    let req = mocks.completion.single_request();
    let (content_opt, success_opt) = req
        .function_call_output_content_and_success(call_id)
        .expect("tool output present");
    let content = content_opt.expect("content present");
    if let Some(success) = success_opt {
        assert!(!success, "expected success=false content={content}");
    }
    assert_eq!(content, "No matches found.");

    Ok(())
}

#[allow(clippy::expect_used)]
async fn build_test_codex(server: &wiremock::MockServer) -> Result<TestCodex> {
    mount_models_once(
        server,
        ModelsResponse {
            models: vec![ModelInfo {
                slug: MODEL_WITH_TOOL.to_string(),
                display_name: "test-gpt-5.1-codex".to_string(),
                description: Some("Test model with grep_files enabled".to_string()),
                default_reasoning_level: Some(ReasoningEffort::Medium),
                supported_reasoning_levels: vec![ReasoningEffortPreset {
                    effort: ReasoningEffort::Medium,
                    description: ReasoningEffort::Medium.to_string(),
                }],
                shell_type: ConfigShellToolType::Default,
                visibility: ModelVisibility::List,
                supported_in_api: true,
                priority: 1,
                upgrade: None,
                base_instructions: "base instructions".to_string(),
                model_messages: None,
                supports_reasoning_summaries: false,
                support_verbosity: false,
                default_verbosity: None,
                apply_patch_tool_type: None,
                truncation_policy: TruncationPolicyConfig::bytes(10_000),
                supports_parallel_tool_calls: false,
                context_window: Some(272_000),
                auto_compact_token_limit: None,
                effective_context_window_percent: 95,
                experimental_supported_tools: vec!["grep_files".to_string()],
                input_modalities: default_input_modalities(),
                prefer_websockets: false,
            }],
        },
    )
    .await;

    let mut builder = test_codex()
        .with_auth(CodexAuth::create_dummy_chatgpt_auth_for_testing())
        .with_model(MODEL_WITH_TOOL)
        .with_config(|config| {
            config.features.enable(Feature::RemoteModels);
        });
    builder.build(server).await
}

fn collect_file_names(content: &str) -> HashSet<String> {
    content
        .lines()
        .filter_map(|line| {
            if line.trim().is_empty() {
                return None;
            }
            Path::new(line)
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .collect()
}
