use async_trait::async_trait;
use codex_protocol::models::FunctionCallOutputBody;
use serde::Deserialize;
use serde_json::Map as JsonMap;
use serde_json::Value as JsonValue;
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

pub struct ClaudeNotebookEditHandler;

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum NotebookEditMode {
    #[default]
    Replace,
    Insert,
    Delete,
}

#[derive(Debug, Deserialize)]
struct ClaudeNotebookEditArgs {
    notebook_path: String,
    new_source: String,
    #[serde(default)]
    edit_mode: NotebookEditMode,
    #[serde(default)]
    cell_id: Option<String>,
    #[serde(default)]
    cell_number: Option<usize>,
    #[serde(default)]
    cell_type: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
struct NotebookEditResult {
    cell_index: Option<usize>,
    cell_count: usize,
}

#[async_trait]
impl ToolHandler for ClaudeNotebookEditHandler {
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
                    "claude_notebook_edit handler received unsupported payload / claude_notebook_edit 处理器仅支持函数负载".to_string(),
                ));
            }
        };

        let args: ClaudeNotebookEditArgs = parse_arguments(&arguments)?;
        let notebook_path = validate_absolute_path(&args.notebook_path, "notebook_path")?;
        let mut notebook = read_notebook_json(&notebook_path).await?;
        let edit_result = apply_notebook_edit(&mut notebook, &args)?;
        write_notebook_json(&notebook_path, &notebook).await?;

        let output = json!({
            "ok": true,
            "notebook_path": notebook_path.to_string_lossy(),
            "edit_mode": mode_label(args.edit_mode),
            "cell_index": edit_result.cell_index,
            "cell_count": edit_result.cell_count,
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

async fn read_notebook_json(path: &Path) -> Result<JsonValue, FunctionCallError> {
    let raw = tokio::fs::read_to_string(path).await.map_err(|err| {
        let path_display = path.display();
        FunctionCallError::RespondToModel(format!(
            "failed to read notebook `{path_display}`: {err} / 读取 notebook `{path_display}` 失败: {err}"
        ))
    })?;

    serde_json::from_str(&raw).map_err(|err| {
        let path_display = path.display();
        FunctionCallError::RespondToModel(format!(
            "notebook `{path_display}` is not valid JSON: {err} / notebook `{path_display}` 不是有效 JSON: {err}"
        ))
    })
}

async fn write_notebook_json(path: &Path, notebook: &JsonValue) -> Result<(), FunctionCallError> {
    let serialized = serde_json::to_string_pretty(notebook).map_err(|err| {
        FunctionCallError::RespondToModel(format!(
            "failed to serialize notebook JSON: {err} / 序列化 notebook JSON 失败: {err}"
        ))
    })?;

    tokio::fs::write(path, serialized).await.map_err(|err| {
        let path_display = path.display();
        FunctionCallError::RespondToModel(format!(
            "failed to write notebook `{path_display}`: {err} / 写入 notebook `{path_display}` 失败: {err}"
        ))
    })
}

fn apply_notebook_edit(
    notebook: &mut JsonValue,
    args: &ClaudeNotebookEditArgs,
) -> Result<NotebookEditResult, FunctionCallError> {
    let normalized_cell_type = normalize_cell_type(args.cell_type.as_deref())?;
    let target_index = {
        let cells = notebook_cells(notebook)?;
        resolve_cell_index(
            cells,
            args.cell_id.as_deref(),
            args.cell_number,
            args.edit_mode,
        )?
    };
    let cells = notebook_cells_mut(notebook)?;

    match args.edit_mode {
        NotebookEditMode::Replace => {
            let index = match target_index {
                Some(index) => index,
                None => {
                    return Err(FunctionCallError::RespondToModel(
                        "replace mode requires cell_id or cell_number / replace 模式需要 cell_id 或 cell_number".to_string(),
                    ))
                }
            };
            let cell_count = cells.len();
            let cell = cells
                .get_mut(index)
                .ok_or_else(|| out_of_range_error(index, cell_count))?;
            replace_cell(cell, &args.new_source, normalized_cell_type.as_deref())?;
            Ok(NotebookEditResult {
                cell_index: Some(index),
                cell_count: cells.len(),
            })
        }
        NotebookEditMode::Insert => {
            let index = target_index.unwrap_or(cells.len());
            let cell_type = normalized_cell_type.as_deref().unwrap_or("code");
            let cell = create_cell(cell_type, &args.new_source)?;
            cells.insert(index, cell);
            Ok(NotebookEditResult {
                cell_index: Some(index),
                cell_count: cells.len(),
            })
        }
        NotebookEditMode::Delete => {
            let index = match target_index {
                Some(index) => index,
                None => {
                    return Err(FunctionCallError::RespondToModel(
                        "delete mode requires cell_id or cell_number / delete 模式需要 cell_id 或 cell_number".to_string(),
                    ))
                }
            };
            if index >= cells.len() {
                return Err(out_of_range_error(index, cells.len()));
            }
            cells.remove(index);
            Ok(NotebookEditResult {
                cell_index: Some(index),
                cell_count: cells.len(),
            })
        }
    }
}

fn notebook_cells(notebook: &JsonValue) -> Result<&[JsonValue], FunctionCallError> {
    notebook
        .get("cells")
        .and_then(JsonValue::as_array)
        .map(Vec::as_slice)
        .ok_or_else(|| {
            FunctionCallError::RespondToModel(
                "notebook JSON must contain an array field `cells` / notebook JSON 必须包含数组字段 `cells`".to_string(),
            )
        })
}

fn notebook_cells_mut(notebook: &mut JsonValue) -> Result<&mut Vec<JsonValue>, FunctionCallError> {
    notebook
        .get_mut("cells")
        .and_then(JsonValue::as_array_mut)
        .ok_or_else(|| {
            FunctionCallError::RespondToModel(
                "notebook JSON must contain an array field `cells` / notebook JSON 必须包含数组字段 `cells`".to_string(),
            )
        })
}

fn resolve_cell_index(
    cells: &[JsonValue],
    cell_id: Option<&str>,
    cell_number: Option<usize>,
    edit_mode: NotebookEditMode,
) -> Result<Option<usize>, FunctionCallError> {
    let index_from_id = if let Some(cell_id) = cell_id {
        let matched_index = find_cell_index_by_id(cells, cell_id)?;
        if edit_mode == NotebookEditMode::Insert {
            Some(matched_index + 1)
        } else {
            Some(matched_index)
        }
    } else {
        None
    };

    if let (Some(index_from_id), Some(cell_number)) = (index_from_id, cell_number)
        && index_from_id != cell_number
    {
        return Err(FunctionCallError::RespondToModel(format!(
            "cell_id and cell_number point to different cells ({index_from_id} vs {cell_number}) / cell_id 与 cell_number 指向的单元不一致（{index_from_id} vs {cell_number}）"
        )));
    }

    let mut selected_index = index_from_id.or(cell_number);
    if selected_index.is_none() && edit_mode == NotebookEditMode::Insert {
        selected_index = Some(cells.len());
    }

    if let Some(index) = selected_index {
        let valid_index = match edit_mode {
            NotebookEditMode::Insert => index <= cells.len(),
            NotebookEditMode::Replace | NotebookEditMode::Delete => index < cells.len(),
        };
        if !valid_index {
            return Err(out_of_range_error(index, cells.len()));
        }
    }

    Ok(selected_index)
}

fn find_cell_index_by_id(cells: &[JsonValue], cell_id: &str) -> Result<usize, FunctionCallError> {
    for (index, cell) in cells.iter().enumerate() {
        if cell.get("id").and_then(JsonValue::as_str) == Some(cell_id) {
            return Ok(index);
        }
    }

    Err(FunctionCallError::RespondToModel(format!(
        "cell_id `{cell_id}` was not found / 未找到 cell_id `{cell_id}`"
    )))
}

fn normalize_cell_type(cell_type: Option<&str>) -> Result<Option<String>, FunctionCallError> {
    cell_type
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| {
            if is_supported_cell_type(value) {
                Ok(value.to_string())
            } else {
                Err(FunctionCallError::RespondToModel(format!(
                    "unsupported cell_type `{value}`; expected one of code|markdown|raw / 不支持的 cell_type `{value}`，可选值为 code|markdown|raw"
                )))
            }
        })
        .transpose()
}

fn is_supported_cell_type(cell_type: &str) -> bool {
    matches!(cell_type, "code" | "markdown" | "raw")
}

fn replace_cell(
    cell: &mut JsonValue,
    new_source: &str,
    cell_type: Option<&str>,
) -> Result<(), FunctionCallError> {
    let cell_object = cell.as_object_mut().ok_or_else(|| {
        FunctionCallError::RespondToModel(
            "target cell must be an object / 目标单元必须是对象".to_string(),
        )
    })?;
    let target_cell_type = cell_type
        .map(ToString::to_string)
        .or_else(|| {
            cell_object
                .get("cell_type")
                .and_then(JsonValue::as_str)
                .map(ToString::to_string)
        })
        .unwrap_or_else(|| "code".to_string());

    cell_object.insert("source".to_string(), source_to_json(new_source));
    configure_cell_shape(cell_object, &target_cell_type)
}

fn create_cell(cell_type: &str, new_source: &str) -> Result<JsonValue, FunctionCallError> {
    let mut cell = JsonMap::new();
    cell.insert("metadata".to_string(), JsonValue::Object(JsonMap::new()));
    cell.insert("source".to_string(), source_to_json(new_source));
    configure_cell_shape(&mut cell, cell_type)?;
    Ok(JsonValue::Object(cell))
}

fn configure_cell_shape(
    cell: &mut JsonMap<String, JsonValue>,
    cell_type: &str,
) -> Result<(), FunctionCallError> {
    if !is_supported_cell_type(cell_type) {
        return Err(FunctionCallError::RespondToModel(format!(
            "unsupported cell_type `{cell_type}`; expected one of code|markdown|raw / 不支持的 cell_type `{cell_type}`，可选值为 code|markdown|raw"
        )));
    }

    cell.insert(
        "cell_type".to_string(),
        JsonValue::String(cell_type.to_string()),
    );
    if !cell.get("metadata").is_some_and(JsonValue::is_object) {
        cell.insert("metadata".to_string(), JsonValue::Object(JsonMap::new()));
    }

    if cell_type == "code" {
        if !cell.get("outputs").is_some_and(JsonValue::is_array) {
            cell.insert("outputs".to_string(), JsonValue::Array(Vec::new()));
        }
        if !cell.contains_key("execution_count") {
            cell.insert("execution_count".to_string(), JsonValue::Null);
        }
    } else if cell_type == "markdown" || cell_type == "raw" {
        cell.remove("outputs");
        cell.remove("execution_count");
    }

    Ok(())
}

fn source_to_json(new_source: &str) -> JsonValue {
    let lines = if new_source.is_empty() {
        Vec::new()
    } else {
        new_source
            .split_inclusive('\n')
            .map(|line| JsonValue::String(line.to_string()))
            .collect()
    };
    JsonValue::Array(lines)
}

fn out_of_range_error(index: usize, cell_count: usize) -> FunctionCallError {
    FunctionCallError::RespondToModel(format!(
        "cell index {index} is out of range (cell count: {cell_count}) / cell 索引 {index} 越界（当前单元数量: {cell_count}）"
    ))
}

fn mode_label(mode: NotebookEditMode) -> &'static str {
    match mode {
        NotebookEditMode::Replace => "replace",
        NotebookEditMode::Insert => "insert",
        NotebookEditMode::Delete => "delete",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn sample_notebook() -> JsonValue {
        json!({
            "cells": [
                {
                    "id": "cell-a",
                    "cell_type": "markdown",
                    "metadata": {},
                    "source": ["hello\n"],
                },
                {
                    "id": "cell-b",
                    "cell_type": "code",
                    "metadata": {},
                    "source": ["print(1)\n"],
                    "outputs": [],
                    "execution_count": null,
                }
            ],
            "metadata": {},
            "nbformat": 4,
            "nbformat_minor": 5
        })
    }

    #[test]
    fn replace_mode_updates_source_by_cell_number() -> anyhow::Result<()> {
        let mut notebook = sample_notebook();
        let args = ClaudeNotebookEditArgs {
            notebook_path: "/tmp/unused.ipynb".to_string(),
            new_source: "updated line".to_string(),
            edit_mode: NotebookEditMode::Replace,
            cell_id: None,
            cell_number: Some(0),
            cell_type: Some("markdown".to_string()),
        };

        let result = apply_notebook_edit(&mut notebook, &args)?;

        assert_eq!(
            result,
            NotebookEditResult {
                cell_index: Some(0),
                cell_count: 2,
            }
        );
        assert_eq!(notebook["cells"][0]["source"], json!(["updated line"]));
        Ok(())
    }

    #[test]
    fn delete_mode_removes_cell_by_id() -> anyhow::Result<()> {
        let mut notebook = sample_notebook();
        let args = ClaudeNotebookEditArgs {
            notebook_path: "/tmp/unused.ipynb".to_string(),
            new_source: String::new(),
            edit_mode: NotebookEditMode::Delete,
            cell_id: Some("cell-b".to_string()),
            cell_number: None,
            cell_type: None,
        };

        let result = apply_notebook_edit(&mut notebook, &args)?;

        assert_eq!(
            result,
            NotebookEditResult {
                cell_index: Some(1),
                cell_count: 1,
            }
        );
        assert_eq!(notebook["cells"].as_array().map(Vec::len), Some(1));
        assert_eq!(notebook["cells"][0]["id"], json!("cell-a"));
        Ok(())
    }
}
