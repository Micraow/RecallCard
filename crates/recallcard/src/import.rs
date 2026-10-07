//! 用户主动提供的官方导出/日志文件；不会扫描本机目录或获取隐藏推理。
use crate::{model::*, Vault};
use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use std::collections::BTreeSet;

pub fn import_text(vault: &Vault, format: &str, text: &str, scope: &str) -> Result<Value> {
    let inputs = parse_text(format, text, scope)?;
    let before = vault.events()?.len();
    let ids = vault
        .capture_batch(inputs)?
        .into_iter()
        .map(|event| event.id)
        .collect::<Vec<_>>();
    let after = vault.events()?.len();
    Ok(
        json!({"ok":true,"events_added":after.saturating_sub(before),"events_seen":ids.len(),"refs":ids.iter().map(|i|format!("event:{i}")).collect::<Vec<_>>(),"coverage":{"messages":"partial","tools":if format=="claude-code"{"partial"}else{"unsupported"},"files":"unsupported","citations":"partial","branches":if format=="chatgpt-export"{"selected_current_branch"}else if format=="deepseek-export"{"all_exported_nodes_not_linear"}else{"partial"},"hidden_reasoning":"not_collected"},"note":"仅导入显式提供的文件；导入中断可安全重复运行，已写原始事件不回滚"}),
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
            "chatgpt-export" | "deepseek-export" => crate::import_bundle::parse_import_bytes(format, text.as_bytes(), scope)?.events,
            "recallcard-conversation" => {
                crate::conversation::Conversation::parse(text)?.events(scope)?
            }
            _ => return Err(
                "支持的格式：recallcard-conversation、manual-jsonl、claude-code、chatgpt-export、deepseek-export"
                    .into(),
            ),
        };
    if inputs.len() > 5000 && !matches!(format, "deepseek-export" | "chatgpt-export") {
        return Err(
            "单批最多导入 5000 个事件；请在备份预览中选择较少会话，或将过长会话分批导出".into(),
        );
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
/// 只包括当前分支；不会把丢失的时间替换成导入时间。
#[derive(Debug, Default, Clone, serde::Serialize, PartialEq, Eq)]
pub struct ChatgptCoverage {
    pub selected_branch_messages: usize,
    pub other_branch_messages_skipped: usize,
    pub hidden_reasoning_messages_skipped: usize,
    pub unsupported_messages_skipped: usize,
    pub empty_messages_skipped: usize,
    pub unsupported_content_parts_skipped: usize,
}

pub(crate) struct ChatgptConversation {
    pub source_id: String,
    pub title: Option<String>,
    pub events: Vec<EventInput>,
    pub coverage: ChatgptCoverage,
}

pub(crate) fn chatgpt_source_id(conv: &Value) -> Result<&str> {
    conv["id"]
        .as_str()
        .filter(|id| !id.trim().is_empty())
        .or_else(|| {
            conv["conversation_id"]
                .as_str()
                .filter(|id| !id.trim().is_empty())
        })
        .ok_or_else(|| "ChatGPT 对话缺少编号".into())
}

pub(crate) fn parse_chatgpt_conversation(conv: &Value, scope: &str) -> Result<ChatgptConversation> {
    let session = chatgpt_source_id(conv)?;
    let mapping = conv["mapping"]
        .as_object()
        .ok_or("ChatGPT 对话缺少 mapping")?;
    let mut node = conv["current_node"]
        .as_str()
        .ok_or("ChatGPT 对话缺少 current_node，无法可靠选择分支")?;
    let mut chain = Vec::new();
    let mut seen = BTreeSet::new();
    loop {
        if !seen.insert(node) {
            return Err("ChatGPT 分支包含循环引用".into());
        }
        let item = mapping.get(node).ok_or("ChatGPT 分支引用缺失节点")?;
        if !item.is_object() {
            return Err("ChatGPT 分支节点必须是对象".into());
        }
        chain.push(item);
        match item.get("parent") {
            Some(Value::String(parent)) if !parent.is_empty() => node = parent,
            Some(Value::Null) => break,
            _ => return Err("ChatGPT 分支 parent 必须是节点编号或 null".into()),
        }
    }
    chain.reverse();
    let mut out = ChatgptConversation {
        source_id: session.into(),
        title: conv["title"].as_str().map(str::to_owned),
        events: Vec::new(),
        coverage: ChatgptCoverage::default(),
    };
    out.coverage.other_branch_messages_skipped = mapping
        .iter()
        .filter(|(id, item)| !seen.contains(id.as_str()) && !item["message"].is_null())
        .count();
    for item in chain {
        let message = &item["message"];
        if message.is_null() {
            continue;
        }
        out.coverage.selected_branch_messages += 1;
        let metadata = &message["metadata"];
        let ctype = message["content"]["content_type"].as_str().unwrap_or("");
        // 某些第三方备份将 analysis 的 channel 清空，但保留 reasoning_status。
        // 先过滤再解释角色，工具消息也绝不能绕过这条边界。
        if metadata["is_visually_hidden_from_conversation"] == true
            || message["is_visually_hidden_from_conversation"] == true
            || metadata["is_thinking_preamble_message"] == true
            || metadata["reasoning_status"] == "is_reasoning"
            || ["analysis", "reasoning", "thoughts"]
                .iter()
                .any(|channel| message["channel"] == *channel || metadata["channel"] == *channel)
            || [
                "thoughts",
                "reasoning_recap",
                "reasoning",
                "thinking",
                "redacted_thinking",
            ]
            .contains(&ctype)
        {
            out.coverage.hidden_reasoning_messages_skipped += 1;
            continue;
        }
        let role = match message["author"]["role"].as_str() {
            Some("user") => Role::User,
            Some("assistant") => Role::Assistant,
            Some("tool") => Role::Tool,
            _ => {
                out.coverage.unsupported_messages_skipped += 1;
                continue;
            }
        };
        if !["text", "multimodal_text", "code", "execution_output"].contains(&ctype) {
            out.coverage.unsupported_messages_skipped += 1;
            continue;
        }
        let content = if let Some(parts) = message["content"]["parts"].as_array() {
            out.coverage.unsupported_content_parts_skipped +=
                parts.iter().filter(|part| !part.is_string()).count();
            parts
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join("\n")
        } else {
            message["content"]["text"].as_str().unwrap_or("").to_owned()
        };
        if content.trim().is_empty() {
            out.coverage.empty_messages_skipped += 1;
            continue;
        }
        let id = message["id"].as_str().ok_or("ChatGPT 消息缺少编号")?;
        let time = chatgpt_time(&message["create_time"])?;
        let origin = match role {
            Role::User => Origin::UserInput,
            Role::Assistant => Origin::AssistantOutput,
            _ => Origin::ToolOutput,
        };
        let event = input(
            "chatgpt-export",
            session,
            id,
            role,
            origin,
            content,
            time,
            scope,
            "message",
            json!({"branch":"current_node", "conversation_title":out.title,
                "citations":metadata["citations"]}),
        )?;
        event.validate()?;
        out.events.push(event);
    }
    Ok(out)
}

fn chatgpt_time(value: &Value) -> Result<Option<DateTime<Utc>>> {
    if value.is_null() {
        return Ok(None);
    }
    let seconds = value
        .as_f64()
        .ok_or("ChatGPT create_time 必须是原始 Unix 时间或 null")?;
    if !seconds.is_finite() || seconds < i64::MIN as f64 || seconds >= i64::MAX as f64 {
        return Err("ChatGPT create_time 超出可表示范围".into());
    }
    DateTime::from_timestamp(
        seconds.floor() as i64,
        ((seconds - seconds.floor()) * 1e9) as u32,
    )
    .map(Some)
    .ok_or_else(|| "ChatGPT create_time 超出可表示范围".into())
}
