use anyhow::{Context, Result, anyhow};
use serde_json::{Value, json};

use crate::commit::CommitPlan;

use super::provider::ChatCompletionResponse;

pub(super) fn extract_commit_plan_tool_args(response: &ChatCompletionResponse) -> Result<CommitPlan> {
    let tool_call = response
        .choices
        .first()
        .and_then(|choice| {
            choice.message.tool_calls.iter().find(|tool_call| {
                tool_call.kind == "function" && tool_call.function.name == "plan_commit"
            })
        })
        .ok_or_else(|| anyhow!("AI response did not include plan_commit tool call"))?;
    let mut arguments: Value = serde_json::from_str(&tool_call.function.arguments)
        .context("AI response was not valid commit plan tool arguments")?;
    normalize_commit_plan_arguments(&mut arguments)?;
    serde_json::from_value(arguments).context("AI response was not valid commit plan tool arguments")
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
