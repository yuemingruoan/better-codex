use async_trait::async_trait;
use codex_protocol::models::FunctionCallOutputBody;
use serde::Deserialize;
use serde_json::json;
use std::path::Path;
use std::path::PathBuf;

use crate::function_tool::FunctionCallError;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolOutput;
use crate::tools::context::ToolPayload;
use crate::tools::handlers::parse_arguments;
use crate::tools::registry::ToolHandler;
use crate::tools::registry::ToolKind;

pub struct ClaudeWriteHandler;

#[derive(Deserialize)]
struct ClaudeWriteArgs {
    file_path: String,
    content: String,
}

#[async_trait]
impl ToolHandler for ClaudeWriteHandler {
    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    async fn is_mutating(&self, _invocation: &ToolInvocation) -> bool {
        true
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<ToolOutput, FunctionCallError> {
        let ToolInvocation { payload, .. } = invocation;

        let arguments = match payload {
            ToolPayload::Function { arguments } => arguments,
            _ => {
                return Err(FunctionCallError::RespondToModel(
                    "claude_write handler received unsupported payload / claude_write 处理器仅支持函数负载"
                        .to_string(),
                ));
            }
        };

        let args: ClaudeWriteArgs = parse_arguments(&arguments)?;
        let path = validate_absolute_path(&args.file_path, "file_path")?;
        write_content(&path, &args.content).await?;

        let output = json!({
            "ok": true,
            "file_path": path.to_string_lossy(),
            "bytes_written": args.content.len(),
        });

        Ok(ToolOutput::Function {
            body: FunctionCallOutputBody::Text(output.to_string()),
            success: Some(true),
        })
    }
}

fn validate_absolute_path(raw_path: &str, field_name: &str) -> Result<PathBuf, FunctionCallError> {
    let path = PathBuf::from(raw_path);
    if !path.is_absolute() {
        return Err(FunctionCallError::RespondToModel(format!(
            "{field_name} must be an absolute path / {field_name} 必须是绝对路径"
        )));
    }
    Ok(path)
}

async fn write_content(path: &Path, content: &str) -> Result<(), FunctionCallError> {
    tokio::fs::write(path, content).await.map_err(|err| {
        let path_display = path.display();
        FunctionCallError::RespondToModel(format!(
            "failed to write file `{path_display}`: {err} / 写入文件 `{path_display}` 失败: {err}"
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use tempfile::tempdir;

    #[test]
    fn reject_relative_path() {
        let err = match validate_absolute_path("relative.txt", "file_path") {
            Ok(path) => panic!("expected relative path to fail, got {path:?}"),
            Err(err) => err,
        };

        assert_eq!(
            err,
            FunctionCallError::RespondToModel(
                "file_path must be an absolute path / file_path 必须是绝对路径".to_string()
            )
        );
    }

    #[tokio::test]
    async fn write_overwrites_existing_content() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let file_path = dir.path().join("sample.txt");
        tokio::fs::write(&file_path, "before").await?;

        write_content(&file_path, "after").await?;

        let content = tokio::fs::read_to_string(&file_path).await?;
        assert_eq!(content, "after");
        Ok(())
    }
}
