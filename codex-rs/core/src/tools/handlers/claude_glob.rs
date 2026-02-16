use async_trait::async_trait;
use codex_protocol::models::FunctionCallOutputBody;
use serde::Deserialize;
use serde_json::json;
use std::path::Path;

use crate::function_tool::FunctionCallError;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolOutput;
use crate::tools::context::ToolPayload;
use crate::tools::handlers::parse_arguments;
use crate::tools::registry::ToolHandler;
use crate::tools::registry::ToolKind;

pub struct ClaudeGlobHandler;

#[derive(Deserialize)]
struct ClaudeGlobArgs {
    pattern: String,
    #[serde(default)]
    path: Option<String>,
}

#[async_trait]
impl ToolHandler for ClaudeGlobHandler {
    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<ToolOutput, FunctionCallError> {
        let ToolInvocation { payload, turn, .. } = invocation;

        let arguments = match payload {
            ToolPayload::Function { arguments } => arguments,
            _ => {
                return Err(FunctionCallError::RespondToModel(
                    "claude_glob handler received unsupported payload / claude_glob 处理器仅支持函数负载"
                        .to_string(),
                ));
            }
        };

        let args: ClaudeGlobArgs = parse_arguments(&arguments)?;
        let pattern = args.pattern.trim();
        if pattern.is_empty() {
            return Err(FunctionCallError::RespondToModel(
                "pattern must not be empty / pattern 不能为空".to_string(),
            ));
        }

        let base_path = turn.resolve_path(args.path);
        validate_search_root(&base_path)?;
        let matches = collect_matches(pattern, &base_path)?;

        let output = json!({
            "ok": true,
            "base_path": base_path.to_string_lossy(),
            "pattern": pattern,
            "matches": matches,
            "count": matches.len(),
        });

        Ok(ToolOutput::Function {
            body: FunctionCallOutputBody::Text(output.to_string()),
            success: Some(true),
        })
    }
}

fn validate_search_root(base_path: &Path) -> Result<(), FunctionCallError> {
    let metadata = std::fs::metadata(base_path).map_err(|err| {
        let path_display = base_path.display();
        FunctionCallError::RespondToModel(format!(
            "failed to access glob path `{path_display}`: {err} / 无法访问 glob 路径 `{path_display}`: {err}"
        ))
    })?;

    if !metadata.is_dir() {
        let path_display = base_path.display();
        return Err(FunctionCallError::RespondToModel(format!(
            "glob path `{path_display}` must be a directory / glob 路径 `{path_display}` 必须是目录"
        )));
    }

    Ok(())
}

fn collect_matches(pattern: &str, base_path: &Path) -> Result<Vec<String>, FunctionCallError> {
    let search_pattern = build_search_pattern(pattern, base_path);
    let mut matches = Vec::new();
    for entry in glob::glob(&search_pattern).map_err(|err| {
        FunctionCallError::RespondToModel(format!(
            "invalid glob pattern `{pattern}`: {err} / glob 模式 `{pattern}` 无效: {err}"
        ))
    })? {
        let matched_path = entry.map_err(|err| {
            FunctionCallError::RespondToModel(format!(
                "failed to read glob match: {err} / 读取 glob 匹配结果失败: {err}"
            ))
        })?;
        matches.push(normalize_match_path(&matched_path, base_path));
    }

    matches.sort_unstable();
    matches.dedup();
    Ok(matches)
}

fn build_search_pattern(pattern: &str, base_path: &Path) -> String {
    let pattern_path = Path::new(pattern);
    if pattern_path.is_absolute() {
        pattern.to_string()
    } else {
        base_path.join(pattern).to_string_lossy().into_owned()
    }
}

fn normalize_match_path(path: &Path, base_path: &Path) -> String {
    if let Ok(relative_path) = path.strip_prefix(base_path) {
        relative_path.to_string_lossy().replace('\\', "/")
    } else {
        path.to_string_lossy().replace('\\', "/")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use tempfile::tempdir;

    #[test]
    fn returns_sorted_relative_paths() -> anyhow::Result<()> {
        let dir = tempdir()?;
        std::fs::write(dir.path().join("b.txt"), "b")?;
        std::fs::write(dir.path().join("a.txt"), "a")?;
        std::fs::write(dir.path().join("skip.md"), "skip")?;

        let matches = collect_matches("*.txt", dir.path())?;
        assert_eq!(matches, vec!["a.txt".to_string(), "b.txt".to_string()]);
        Ok(())
    }

    #[test]
    fn rejects_invalid_pattern() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let err = match collect_matches("[", dir.path()) {
            Ok(_) => panic!("expected invalid glob pattern to fail"),
            Err(err) => err,
        };

        let FunctionCallError::RespondToModel(message) = err else {
            panic!("expected RespondToModel error");
        };
        assert!(message.contains("invalid glob pattern `[`"));
        assert!(message.contains("glob 模式 `[` 无效"));
        Ok(())
    }
}
