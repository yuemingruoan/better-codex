#![allow(clippy::unwrap_used)]

use codex_core::features::Feature;
use codex_protocol::config_types::WebSearchMode;
use core_test_support::responses;
use core_test_support::responses::start_mock_server;
use core_test_support::skip_if_no_network;
use core_test_support::test_codex::test_codex;

#[allow(clippy::expect_used)]
fn tool_identifiers(body: &serde_json::Value) -> Vec<String> {
    body["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| {
            tool.get("name")
                .and_then(|v| v.as_str())
                .or_else(|| tool.get("type").and_then(|v| v.as_str()))
                .map(std::string::ToString::to_string)
                .expect("tool should have either name or type")
        })
        .collect()
}

#[allow(clippy::expect_used)]
async fn collect_tool_identifiers_for_model(model: &str) -> Vec<String> {
    let server = start_mock_server().await;
    let sse = responses::sse(vec![
        responses::ev_response_created(model),
        responses::ev_completed(model),
    ]);
    let resp_mock = responses::mount_sse_once(&server, sse).await;

    let mut builder = test_codex()
        .with_model(model)
        // Keep tool expectations stable when the default web_search mode changes.
        .with_config(|config| {
            config.features.enable(Feature::RemoteModels);
            config
                .web_search_mode
                .set(WebSearchMode::Cached)
                .expect("test web_search_mode should satisfy constraints");
            config.features.enable(Feature::CollaborationModes);
        });
    let test = builder
        .build(&server)
        .await
        .expect("create test Codex conversation");

    test.submit_turn("hello tools").await.expect("submit turn");

    let body = resp_mock.single_request().body_json();
    tool_identifiers(&body)
}

fn has_tool(tools: &[String], tool: &str) -> bool {
    tools.iter().any(|candidate| candidate == tool)
}

fn assert_has_common_tools(model: &str, tools: &[String]) {
    for required in [
        "list_mcp_resources",
        "list_mcp_resource_templates",
        "read_mcp_resource",
        "update_plan",
        "batches_read_file",
        "web_search",
        "view_image",
    ] {
        assert!(
            has_tool(tools, required),
            "{model} should expose {required}; got {tools:?}"
        );
    }
}

fn assert_has_shell_or_unified_exec(model: &str, tools: &[String], shell_aliases: &[&str]) {
    let has_shell_alias = shell_aliases.iter().any(|name| has_tool(tools, name));
    let has_unified_exec = has_tool(tools, "exec_command") && has_tool(tools, "write_stdin");
    assert!(
        has_shell_alias || has_unified_exec,
        "{model} should expose one shell entrypoint (alias={shell_aliases:?} or unified exec); got {tools:?}"
    );
}

fn assert_has_apply_patch(model: &str, tools: &[String]) {
    assert!(
        has_tool(tools, "apply_patch"),
        "{model} should expose apply_patch; got {tools:?}"
    );
}

fn assert_no_apply_patch(model: &str, tools: &[String]) {
    assert!(
        !has_tool(tools, "apply_patch"),
        "{model} should not expose apply_patch; got {tools:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn model_selects_expected_tools() {
    skip_if_no_network!();

    let codex_tools = collect_tool_identifiers_for_model("codex-mini-latest").await;
    assert_has_common_tools("codex-mini-latest", &codex_tools);
    assert_has_shell_or_unified_exec("codex-mini-latest", &codex_tools, &["local_shell"]);
    assert_no_apply_patch("codex-mini-latest", &codex_tools);

    let gpt51_codex_max_tools = collect_tool_identifiers_for_model("gpt-5.1-codex-max").await;
    assert_has_common_tools("gpt-5.1-codex-max", &gpt51_codex_max_tools);
    assert_has_shell_or_unified_exec(
        "gpt-5.1-codex-max",
        &gpt51_codex_max_tools,
        &["shell_command"],
    );
    assert_has_apply_patch("gpt-5.1-codex-max", &gpt51_codex_max_tools);

    let gpt5_codex_tools = collect_tool_identifiers_for_model("gpt-5-codex").await;
    assert_has_common_tools("gpt-5-codex", &gpt5_codex_tools);
    assert_has_shell_or_unified_exec("gpt-5-codex", &gpt5_codex_tools, &["shell_command"]);
    assert_has_apply_patch("gpt-5-codex", &gpt5_codex_tools);

    let gpt51_codex_tools = collect_tool_identifiers_for_model("gpt-5.1-codex").await;
    assert_has_common_tools("gpt-5.1-codex", &gpt51_codex_tools);
    assert_has_shell_or_unified_exec("gpt-5.1-codex", &gpt51_codex_tools, &["shell_command"]);
    assert_has_apply_patch("gpt-5.1-codex", &gpt51_codex_tools);

    let gpt5_tools = collect_tool_identifiers_for_model("gpt-5").await;
    assert_has_common_tools("gpt-5", &gpt5_tools);
    assert_has_shell_or_unified_exec("gpt-5", &gpt5_tools, &["shell"]);
    assert_no_apply_patch("gpt-5", &gpt5_tools);

    let gpt51_tools = collect_tool_identifiers_for_model("gpt-5.1").await;
    assert_has_common_tools("gpt-5.1", &gpt51_tools);
    assert_has_shell_or_unified_exec("gpt-5.1", &gpt51_tools, &["shell_command"]);
    assert_has_apply_patch("gpt-5.1", &gpt51_tools);

    let exp_tools = collect_tool_identifiers_for_model("exp-5.1").await;
    assert_has_common_tools("exp-5.1", &exp_tools);
    assert!(has_tool(&exp_tools, "exec_command"));
    assert!(has_tool(&exp_tools, "write_stdin"));
    assert_no_apply_patch("exp-5.1", &exp_tools);
}
