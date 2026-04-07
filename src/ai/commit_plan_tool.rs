use anyhow::{Context, Result, anyhow};
use serde_json::{Value, json};

use crate::commit::CommitPlan;

use super::provider::ChatCompletionResponse;

pub(super) fn extract_commit_plan_tool_args(
    response: &ChatCompletionResponse,
) -> Result<CommitPlan> {
    let tool_call = find_tool_call(response, "plan_commit")
        .ok_or_else(|| anyhow!("AI response did not include plan_commit tool call"))?;
    let raw = sanitize_json_arguments(&tool_call.function.arguments);
    let mut arguments: Value = serde_json::from_str(&raw)
        .context("AI response was not valid commit plan tool arguments")?;
    normalize_commit_plan_arguments(&mut arguments)?;
    serde_json::from_value(arguments)
        .context("AI response was not valid commit plan tool arguments")
}

/// Strip trailing garbage after the top-level JSON object.
/// Some models emit extra whitespace, markdown fences, or duplicate fragments.
fn sanitize_json_arguments(raw: &str) -> String {
    let trimmed = raw.trim();
    // Strip markdown code fences if present
    let trimmed = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .unwrap_or(trimmed);
    let trimmed = trimmed.strip_suffix("```").unwrap_or(trimmed).trim();

    // Find the end of the first balanced top-level JSON object
    if let Some(end) = find_json_object_end(trimmed) {
        trimmed[..=end].to_string()
    } else {
        trimmed.to_string()
    }
}

/// Find the byte index of the closing `}` that balances the first `{`.
/// Respects JSON string escaping.
fn find_json_object_end(s: &str) -> Option<usize> {
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escape_next = false;

    for (i, ch) in s.char_indices() {
        if escape_next {
            escape_next = false;
            continue;
        }
        if in_string {
            match ch {
                '\\' => escape_next = true,
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

pub(super) fn extract_resolve_conflicts_tool_args(
    response: &ChatCompletionResponse,
) -> Result<crate::ai::AiPatch> {
    let tool_call = find_tool_call(response, "resolve_conflicts")
        .ok_or_else(|| anyhow!("AI response did not include resolve_conflicts tool call"))?;
    serde_json::from_str(&tool_call.function.arguments)
        .context("AI response was not valid resolve_conflicts tool arguments")
}

fn find_tool_call<'a>(
    response: &'a ChatCompletionResponse,
    tool_name: &str,
) -> Option<&'a super::provider::ToolCall> {
    response.choices.first().and_then(|choice| {
        choice
            .message
            .tool_calls
            .iter()
            .find(|tool_call| tool_call.kind == "function" && tool_call.function.name == tool_name)
    })
}

fn normalize_commit_plan_arguments(arguments: &mut Value) -> Result<()> {
    let Some(object) = arguments.as_object_mut() else {
        return Ok(());
    };

    if let Some(single_group) = object.get_mut("single_group") {
        normalize_group_value(single_group)?;
    }
    if let Some(groups) = object.get_mut("groups") {
        if groups.is_string() {
            *groups = serde_json::from_str(groups.as_str().unwrap_or_default())
                .context("AI response was not valid commit plan tool arguments")?;
        }
        if let Some(groups_array) = groups.as_array_mut() {
            for group in groups_array {
                normalize_group_value(group)?;
            }
        }
    }

    Ok(())
}

fn normalize_group_value(value: &mut Value) -> Result<()> {
    if value.is_null() || value.is_object() {
        return Ok(());
    }
    if let Some(raw) = value.as_str() {
        *value = serde_json::from_str(raw)
            .context("AI response was not valid commit plan tool arguments")?;
    }
    Ok(())
}

pub(super) fn commit_plan_tool_definition() -> Value {
    json!({
        "type": "function",
        "function": {
            "name": "plan_commit",
            "description": "Return the commit grouping plan.",
            "parameters": {
                "type": "object",
                "properties": {
                    "grouping_decision": {
                        "type": "string",
                        "enum": ["single", "split"]
                    },
                    "grouping_confidence": {
                        "type": "number"
                    },
                    "single_group": {
                        "anyOf": [
                            commit_group_schema(),
                            { "type": "null" }
                        ]
                    },
                    "groups": {
                        "type": "array",
                        "items": commit_group_schema()
                    },
                    "confidence": {
                        "type": "number"
                    },
                    "warnings": {
                        "type": "array",
                        "items": { "type": "string" }
                    },
                    "auto_executable": {
                        "type": "boolean"
                    }
                },
                "required": [
                    "grouping_decision",
                    "grouping_confidence",
                    "single_group",
                    "groups",
                    "confidence",
                    "warnings",
                    "auto_executable"
                ],
                "additionalProperties": false
            }
        }
    })
}

pub(super) fn resolve_conflicts_tool_definition() -> Value {
    json!({
        "type": "function",
        "function": {
            "name": "resolve_conflicts",
            "description": "Return the full resolved contents for every conflicted file.",
            "parameters": {
                "type": "object",
                "properties": {
                    "confidence": {
                        "type": "number"
                    },
                    "summary": {
                        "type": "string"
                    },
                    "files": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "path": {
                                    "type": "string"
                                },
                                "explanation": {
                                    "type": "string"
                                },
                                "resolved_content": {
                                    "type": "string"
                                }
                            },
                            "required": ["path", "explanation", "resolved_content"],
                            "additionalProperties": false
                        }
                    }
                },
                "required": ["confidence", "summary", "files"],
                "additionalProperties": false
            }
        }
    })
}

fn commit_group_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "scope": {
                "anyOf": [
                    { "type": "string" },
                    { "type": "null" }
                ]
            },
            "files": {
                "type": "array",
                "items": { "type": "string" }
            },
            "commit_message": {
                "type": "string"
            },
            "rationale": {
                "type": "string"
            }
        },
        "required": ["scope", "files", "commit_message", "rationale"],
        "additionalProperties": false
    })
}
