//! 用户主动提供的官方导出/日志文件；不会扫描本机目录或获取隐藏推理。
use crate::{model::*, Vault};
use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use std::collections::BTreeSet;

pub fn import_text(vault: &Vault, format: &str, text: &str, scope: &str) -> Result<Value> {
    let inputs = parse_text(format, text, scope)?;
    let before = vault.events()?.len();
    let mut ids = Vec::new();
    for input in inputs {
        ids.push(vault.capture(input)?.id);
    }
    let after = vault.events()?.len();
    Ok(
        json!({"ok":true,"events_added":after.saturating_sub(before),"events_seen":ids.len(),"refs":ids.iter().map(|i|format!("event:{i}")).collect::<Vec<_>>(),"coverage":{"messages":"partial","tools":if format=="claude-code"{"partial"}else{"unsupported"},"files":"unsupported","citations":"partial","branches":if format=="chatgpt-export"{"selected_current_branch"}else{"partial"},"hidden_reasoning":"not_collected"},"note":"仅导入显式提供的文件；导入中断可安全重复运行，已写原始事件不回滚"}),
    )
}

/// 复用正式导入适配器的无副作用解析；桌面预览不得先写入用户 Vault。
pub(crate) fn parse_text(format: &str, text: &str, scope: &str) -> Result<Vec<EventInput>> {
    validate_scope(scope)?;
    if text.len() > 16 * 1024 * 1024 {
        return Err("单次导入上限 16 MiB；请分批导出".into());
    }
    let inputs =
        match format {
            "manual-jsonl" => text
                .lines()
                .filter(|l| !l.trim().is_empty())
                .map(|l| serde_json::from_str::<EventInput>(l).map_err(|e| e.to_string()))
                .collect::<Result<Vec<_>>>()?,
            "claude-code" => claude_code(text, scope)?,
            "chatgpt-export" => chatgpt_export(text, scope)?,
            "recallcard-conversation" => {
                crate::conversation::Conversation::parse(text)?.events(scope)?
            }
            _ => return Err(
                "支持的格式：recallcard-conversation、manual-jsonl、claude-code、chatgpt-export"
                    .into(),
            ),
        };
    if inputs.len() > 5000 {
        return Err("单次最多导入 5000 个事件".into());
    }
    for input in &inputs {
        input.validate()?;
        if input.scope != scope {
            return Err("导入数据的 scope 与指定范围不一致".into());
        }
    }
    Ok(inputs)
}
// 仅供已知导出适配器映射固定 Event 字段；不是对外接口。
#[allow(clippy::too_many_arguments)]
fn input(
    source: &str,
    session: &str,
    id: &str,
    role: Role,
    origin: Origin,
    text: String,
    time: Option<DateTime<Utc>>,
    scope: &str,
    kind: &str,
    metadata: Value,
) -> Result<EventInput> {
    serde_json::from_value(json!({"occurred_at":time,"role":role,"origin":origin,"content":text,"scope":scope,"kind":kind,"source":{"platform":source,"conversation_id":session,"message_id":id},"metadata":metadata,"capture":{"completeness":"partial","reason":"only_provided_export_is_available"}})).map_err(|e|e.to_string())
}
fn claude_code(text: &str, scope: &str) -> Result<Vec<EventInput>> {
    let mut out = Vec::new();
    for (number, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let v: Value = serde_json::from_str(line)
            .map_err(|e| format!("Claude Code 第 {} 行无效：{e}", number + 1))?;
        let kind = v["type"].as_str().unwrap_or("");
        if !["user", "assistant"].contains(&kind) {
            continue;
        }
        let session = v["sessionId"]
            .as_str()
            .ok_or("Claude Code 消息缺少 sessionId")?;
        let id = v["uuid"].as_str().ok_or("Claude Code 消息缺少 uuid")?;
        let time = v["timestamp"]
            .as_str()
            .map(|s| {
                s.parse::<DateTime<Utc>>()
                    .map_err(|_| "无效 timestamp".to_string())
            })
            .transpose()?;
        let role = if kind == "user" {
            Role::User
        } else {
            Role::Assistant
        };
        let origin = if kind == "user" {
            Origin::UserInput
        } else {
            Origin::AssistantOutput
        };
        let content = &v["message"]["content"];
        if let Some(text) = content.as_str() {
            out.push(input(
                "claude-code",
                session,
                id,
                role,
                origin,
                text.into(),
                time,
                scope,
                "message",
                Value::Null,
            )?);
            continue;
        }
        let parts = content
            .as_array()
            .ok_or("Claude Code message.content 必须为文本或数组")?;
        let text = parts
            .iter()
            .filter(|p| p["type"] == "text")
            .filter_map(|p| p["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n");
        if !text.trim().is_empty() {
            out.push(input(
                "claude-code",
                session,
                id,
                role.clone(),
                origin,
                text,
                time,
                scope,
                "message",
                Value::Null,
            )?);
        }
        for (index, part) in parts.iter().enumerate() {
            let typ = part["type"].as_str().unwrap_or("");
            if typ == "tool_use" {
                out.push(input("claude-code",session,&format!("{id}/tool/{index}"),Role::Assistant,Origin::AssistantOutput,part["name"].as_str().unwrap_or("工具调用").into(),time,scope,"tool_call",json!({"tool_call_id":part["id"],"name":part["name"],"arguments":part["input"]}))?);
            }
            if typ == "tool_result" {
                let content = if let Some(s) = part["content"].as_str() {
                    s.to_owned()
                } else {
                    part["content"]
                        .as_array()
                        .map(|a| {
                            a.iter()
                                .filter_map(|p| p["text"].as_str())
                                .collect::<Vec<_>>()
                                .join("\n")
                        })
                        .unwrap_or_default()
                };
                out.push(input(
                    "claude-code",
                    session,
                    &format!("{id}/result/{index}"),
                    Role::Tool,
                    Origin::ToolOutput,
                    content,
                    time,
                    scope,
                    "tool_result",
                    json!({"tool_call_id":part["tool_use_id"],"is_error":part["is_error"]}),
                )?);
            }
            // thinking / redacted_thinking 等不被收集。
        }
    }
    Ok(out)
}
fn chatgpt_export(text: &str, scope: &str) -> Result<Vec<EventInput>> {
    let value: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let conversations = value
        .as_array()
        .ok_or("ChatGPT 官方导出必须是 conversation 数组")?;
    let mut out = Vec::new();
    for conv in conversations {
        let session = conv["id"]
            .as_str()
            .or_else(|| conv["conversation_id"].as_str())
            .ok_or("ChatGPT 对话缺少编号")?;
        let mapping = conv["mapping"]
            .as_object()
            .ok_or("ChatGPT 对话缺少 mapping")?;
        let mut node = conv["current_node"]
            .as_str()
            .ok_or("ChatGPT 对话缺少 current_node，无法可靠选择分支")?;
        let mut chain = Vec::new();
        let mut seen = BTreeSet::new();
        loop {
            if !seen.insert(node.to_owned()) {
                return Err("ChatGPT 分支包含循环引用".into());
            }
            let item = mapping.get(node).ok_or("ChatGPT 分支引用缺失节点")?;
            chain.push(item);
            match item["parent"].as_str() {
                Some(parent) => node = parent,
                None => break,
            }
        }
        chain.reverse();
        for item in chain {
            let message = &item["message"];
            if message.is_null() {
                continue;
            }
            if message["metadata"]["is_visually_hidden_from_conversation"] == true
                || message["channel"] == "analysis"
            {
                continue;
            }
            let role = match message["author"]["role"].as_str() {
                Some("user") => Role::User,
                Some("assistant") => Role::Assistant,
                Some("tool") => Role::Tool,
                _ => continue,
            };
            let ctype = message["content"]["content_type"].as_str().unwrap_or("");
            if ["thoughts", "reasoning_recap"].contains(&ctype) {
                continue;
            }
            let content = message["content"]["parts"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .or_else(|| message["content"]["text"].as_str().map(str::to_owned))
                .unwrap_or_default();
            if content.trim().is_empty() {
                continue;
            }
            let id = message["id"].as_str().ok_or("ChatGPT 消息缺少编号")?;
            let time = message["create_time"].as_f64().and_then(|s| {
                if s.is_finite() {
                    DateTime::from_timestamp(s.floor() as i64, ((s - s.floor()) * 1e9) as u32)
                } else {
                    None
                }
            });
            let origin = match role {
                Role::User => Origin::UserInput,
                Role::Assistant => Origin::AssistantOutput,
                _ => Origin::ToolOutput,
            };
            out.push(input(
                "chatgpt-export",
                session,
                id,
                role,
                origin,
                content,
                time,
                scope,
                "message",
                json!({"branch":"current_node","citations":message["metadata"]["citations"]}),
            )?);
        }
    }
    Ok(out)
}
