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

pub struct ClaudeEditHandler;

#[derive(Deserialize)]
struct ClaudeEditArgs {
    file_path: String,
    old_string: String,
    new_string: String,
    #[serde(default)]
    replace_all: bool,
}

struct EditResult {
    updated_content: String,
    replacements: usize,
}

#[async_trait]
impl ToolHandler for ClaudeEditHandler {
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
                    "claude_edit handler received unsupported payload / claude_edit 处理器仅支持函数负载"
                        .to_string(),
                ));
            }
        };

        let args: ClaudeEditArgs = parse_arguments(&arguments)?;
        let path = validate_absolute_path(&args.file_path, "file_path")?;
        let edit_result =
            edit_file(&path, &args.old_string, &args.new_string, args.replace_all).await?;

        let output = json!({
            "ok": true,
            "file_path": path.to_string_lossy(),
            "replacements": edit_result.replacements,
            "replace_all": args.replace_all,
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

async fn edit_file(
    path: &Path,
    old_string: &str,
    new_string: &str,
    replace_all: bool,
) -> Result<EditResult, FunctionCallError> {
    let original = tokio::fs::read_to_string(path).await.map_err(|err| {
        let path_display = path.display();
        FunctionCallError::RespondToModel(format!(
            "failed to read text file `{path_display}`: {err} / 读取文本文件 `{path_display}` 失败: {err}"
        ))
    })?;

    let result = compute_edited_content(&original, old_string, new_string, replace_all)?;

    tokio::fs::write(path, &result.updated_content)
        .await
        .map_err(|err| {
            let path_display = path.display();
            FunctionCallError::RespondToModel(format!(
                "failed to write edited file `{path_display}`: {err} / 写入编辑后的文件 `{path_display}` 失败: {err}"
            ))
        })?;

    Ok(result)
}

fn compute_edited_content(
    original: &str,
    old_string: &str,
    new_string: &str,
    replace_all: bool,
) -> Result<EditResult, FunctionCallError> {
    if old_string.is_empty() {
        return Err(FunctionCallError::RespondToModel(
            "old_string must not be empty / old_string 不能为空".to_string(),
        ));
    }

    let occurrences = original.match_indices(old_string).count();
    if occurrences == 0 {
        let preview = preview_text(old_string, 80);
        return Err(FunctionCallError::RespondToModel(format!(
            "old_string not found in file: `{preview}` / 文件中未找到 old_string: `{preview}`"
        )));
    }

    if occurrences > 1 && !replace_all {
        return Err(FunctionCallError::RespondToModel(format!(
            "old_string is not unique (found {occurrences} matches); set replace_all=true / old_string 非唯一（找到 {occurrences} 处匹配），请设置 replace_all=true"
        )));
    }

    let updated_content = if replace_all {
        original.replace(old_string, new_string)
    } else {
        original.replacen(old_string, new_string, 1)
    };
    let replacements = if replace_all { occurrences } else { 1 };

    Ok(EditResult {
        updated_content,
        replacements,
    })
}

fn preview_text(input: &str, limit: usize) -> String {
    let mut preview = String::new();
    for ch in input.chars().take(limit) {
        preview.push(ch);
    }
    preview
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use tempfile::tempdir;

    #[test]
    fn computes_single_replacement() -> anyhow::Result<()> {
        let result = compute_edited_content("hello world", "world", "codex", false)?;
        assert_eq!(result.updated_content, "hello codex");
        assert_eq!(result.replacements, 1);
        Ok(())
    }

    #[test]
    fn rejects_non_unique_old_string_without_replace_all() {
        let err = match compute_edited_content("a b a", "a", "z", false) {
            Ok(_) => panic!("expected non-unique old_string to fail"),
            Err(err) => err,
        };

        assert_eq!(
            err,
            FunctionCallError::RespondToModel(
                "old_string is not unique (found 2 matches); set replace_all=true / old_string 非唯一（找到 2 处匹配），请设置 replace_all=true".to_string()
            )
        );
    }

    #[tokio::test]
    async fn edit_file_writes_result() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let file_path = dir.path().join("edit.txt");
        tokio::fs::write(&file_path, "one two").await?;

        let result = edit_file(&file_path, "two", "three", false).await?;

        assert_eq!(result.replacements, 1);
        let content = tokio::fs::read_to_string(&file_path).await?;
        assert_eq!(content, "one three");
        Ok(())
    }
}
