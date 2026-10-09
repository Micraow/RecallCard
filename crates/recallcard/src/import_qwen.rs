//! Qwen 官方历史导出：history 是唯一正本，messages 是当前路径的重复投影。
//! 只取明确可见的用户正文和 assistant answer；推理、工具负载及附件外链不进入证据。
use crate::{EventInput, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct QwenCoverage {
    pub nodes_seen: usize,
    pub visible_messages_imported: usize,
    pub file_only_messages_imported: usize,
    pub empty_messages_skipped: usize,
    pub hidden_fragments_skipped: usize,
    pub unsupported_fragments_skipped: usize,
    pub answer_fragments_imported: usize,
    pub attachment_references: usize,
    pub attachment_payloads_omitted: usize,
    pub attachments_with_missing_metadata: usize,
    pub branch_points: usize,
    pub roots: usize,
    pub off_current_path_nodes: usize,
    pub off_current_path_imported: usize,
    pub linear_duplicates_ignored: usize,
    pub missing_message_timestamps: usize,
    pub error_messages_seen: usize,
}
impl QwenCoverage {
    pub(crate) fn add(&mut self, other: &Self) {
        macro_rules! add { ($($field:ident),*) => { $(self.$field += other.$field;)* }; }
        add!(
            nodes_seen,
            visible_messages_imported,
            file_only_messages_imported,
            empty_messages_skipped,
            hidden_fragments_skipped,
            unsupported_fragments_skipped,
            answer_fragments_imported,
            attachment_references,
            attachment_payloads_omitted,
            attachments_with_missing_metadata,
            branch_points,
            roots,
            off_current_path_nodes,
            off_current_path_imported,
            linear_duplicates_ignored,
            missing_message_timestamps,
            error_messages_seen
        );
    }
}
pub(crate) struct QwenConversation {
    pub source_id: String,
    pub title: Option<String>,
    pub events: Vec<EventInput>,
    pub coverage: QwenCoverage,
}
pub(crate) fn looks_like_qwen(value: &Value) -> bool {
    value.is_object()
        && value
            .get("chat")
            .is_some_and(|chat| chat.get("history").is_some() && chat.get("messages").is_some())
}
pub(crate) fn wrapper_values(value: &Value) -> Result<Option<&Vec<Value>>> {
    if value.get("data").is_none() || value.get("success").is_none() {
        return Ok(None);
    }
    if value["success"] != true
        || value["request_id"].as_str().is_none_or(str::is_empty)
        || value.as_object().is_none_or(|object| object.len() != 3)
    {
        return Err("Qwen 导出包装无效或请求未成功".into());
    }
    let values = value["data"].as_array().ok_or("Qwen data 必须是会话数组")?;
    if values.iter().any(|item| !looks_like_qwen(item)) {
        return Err("Qwen data 包含非 Qwen 会话；不猜测其他来源".into());
    }
    Ok(Some(values))
}
fn text<'a>(value: &'a Value, field: &str, max: usize, empty: bool) -> Result<&'a str> {
    value
        .as_str()
        .filter(|s| s.len() <= max && !s.contains('\0') && (empty || !s.trim().is_empty()))
        .ok_or_else(|| format!("Qwen {field}缺失、类型错误或超过安全长度"))
}
fn id<'a>(value: &'a Value, field: &str) -> Result<&'a str> {
    let value = text(value, field, 2048, false)?;
    if value.chars().any(char::is_control) {
        return Err(format!("Qwen {field}含控制字符"));
    }
    Ok(value)
}
fn timestamp(value: Option<&Value>) -> Result<Option<DateTime<Utc>>> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_i64()
            .and_then(|seconds| DateTime::from_timestamp(seconds, 0))
            .map(Some)
            .ok_or_else(|| "Qwen 时间必须是有效 Unix 秒整数或 null".into()),
    }
}
fn optional_id(value: Option<&Value>, field: &str) -> Result<Option<String>> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(v) => Ok(Some(id(v, field)?.into())),
    }
}

pub(crate) fn parse_qwen_conversation(value: &Value, scope: &str) -> Result<QwenConversation> {
    crate::validate_scope(scope)?;
    let source_id = id(&value["id"], "会话编号")?;
    let account = match value.get("user_id") {
        None | Some(Value::Null) => "",
        Some(v) => id(v, "账号编号")?,
    };
    let title = match value.get("title") {
        None | Some(Value::Null) => None,
        Some(v) => Some(text(v, "标题", 4096, true)?.to_owned()),
    };
    timestamp(value.get("created_at"))?;
    timestamp(value.get("updated_at"))?;
    let history = value["chat"]["history"]
        .as_object()
        .ok_or("Qwen 缺少 history 对象")?;
    let messages = history
        .get("messages")
        .and_then(Value::as_object)
        .ok_or("Qwen 缺少 history.messages 映射")?;
    let current = optional_id(history.get("currentId"), "当前节点")?;
    if value.get("currentId").is_some()
        && optional_id(value.get("currentId"), "当前节点")? != current
    {
        return Err("Qwen 当前节点标记互相冲突".into());
    }
    let mut parents = BTreeMap::new();
    let mut children = BTreeMap::new();
    let mut coverage = QwenCoverage {
        nodes_seen: messages.len(),
        ..Default::default()
    };
    for (key, message) in messages {
        if id(&message["id"], "消息编号")? != key {
            return Err("Qwen 消息编号与 history 键不一致".into());
        }
        let parent = optional_id(message.get("parentId"), "父节点")?;
        if message.get("parentId").is_none() {
            return Err("Qwen 消息缺少 parentId".into());
        }
        let raw_children = message["childrenIds"]
            .as_array()
            .ok_or("Qwen childrenIds 必须是数组")?;
        let mut ids = Vec::new();
        let mut unique = BTreeSet::new();
        for child in raw_children {
            let child = id(child, "子节点")?.to_owned();
            if !unique.insert(child.clone()) {
                return Err("Qwen childrenIds 编号重复".into());
            }
            ids.push(child);
        }
        coverage.roots += usize::from(parent.is_none());
        coverage.branch_points += usize::from(ids.len() > 1);
        parents.insert(key.clone(), parent);
        children.insert(key.clone(), ids);
    }
    for (node, parent) in &parents {
        if let Some(parent) = parent {
            if !children.get(parent).is_some_and(|ids| ids.contains(node)) {
                return Err("Qwen 父子关系缺失或不一致".into());
            }
        }
        for child in &children[node] {
            if parents.get(child) != Some(&Some(node.clone())) {
                return Err("Qwen 父子关系缺失或不一致".into());
            }
        }
    }
    let mut ready: BTreeSet<String> = parents
        .iter()
        .filter_map(|(id, p)| p.is_none().then_some(id.clone()))
        .collect();
    let mut order = Vec::with_capacity(messages.len());
    while let Some(node) = ready.pop_first() {
        ready.extend(children[&node].iter().cloned());
        order.push(node);
    }
    if order.len() != messages.len() {
        return Err("Qwen 会话包含循环；没有导入消息".into());
    }
    let mut selected = BTreeSet::new();
    let mut node = current.clone();
    while let Some(id) = node {
        if !selected.insert(id.clone()) || !parents.contains_key(&id) {
            return Err("Qwen 当前路径无效".into());
        }
        node = parents[&id].clone();
    }
    let mut projected = BTreeSet::new();
    for message in value["chat"]["messages"]
        .as_array()
        .ok_or("Qwen chat.messages 必须是数组")?
    {
        let id = id(&message["id"], "投影消息编号")?;
        if !projected.insert(id.to_owned()) || messages.get(id) != Some(message) {
            return Err("Qwen messages 与 history 重复投影内容不一致".into());
        }
    }
    if current.is_some() && projected != selected {
        return Err("Qwen 当前路径与 messages 投影不一致".into());
    }
    coverage.linear_duplicates_ignored = projected.len();
    coverage.off_current_path_nodes = messages.len() - selected.len();
    let mut events = Vec::new();
    let mut budget = crate::import::NormalizedBudget::default();
    for node in &order {
        let message = &messages[node];
        let role = message["role"].as_str().ok_or("Qwen 消息缺少 role")?;
        if !matches!(role, "user" | "assistant") {
            return Err("Qwen 消息角色未支持；没有猜测归属".into());
        }
        let time = timestamp(message.get("timestamp"))?;
        coverage.missing_message_timestamps += usize::from(time.is_none());
        coverage.error_messages_seen +=
            usize::from(message.get("error").is_some_and(|v| !v.is_null()));
        let body = text(&message["content"], "content", 2 * 1024 * 1024, true)?;
        let mut content = if role == "user" {
            body.to_owned()
        } else {
            String::new()
        };
        let mut answer_indices = Vec::new();
        let mut hidden = 0usize;
        let mut unsupported = 0usize;
        if message
            .get("reasoning_content")
            .is_some_and(|v| v.as_str().is_some_and(|s| !s.is_empty()))
        {
            hidden += 1;
        }
        match message.get("content_list") {
            None | Some(Value::Null) => {}
            Some(Value::Array(parts)) => {
                for (index, part) in parts.iter().enumerate() {
                    match part["phase"].as_str() {
                        Some("think" | "thinking_summary") => hidden += 1,
                        Some("answer") if role == "assistant" && part["role"] == "assistant" => {
                            content.push_str(text(
                                &part["content"],
                                "answer",
                                2 * 1024 * 1024,
                                true,
                            )?);
                            answer_indices.push(index);
                        }
                        Some(_) => unsupported += 1,
                        None => return Err("Qwen content_list 段缺少明确 phase".into()),
                    }
                }
            }
            Some(_) => return Err("Qwen content_list 必须是数组或 null".into()),
        }
        if role == "assistant" && !body.is_empty() {
            if answer_indices.is_empty() {
                unsupported += 1;
            } else if body != content {
                return Err("Qwen 顶层回答与 answer 分段互相冲突".into());
            }
        }
        coverage.hidden_fragments_skipped += hidden;
        coverage.unsupported_fragments_skipped += unsupported;
        coverage.answer_fragments_imported += answer_indices.len();
        let raw_files = match message.get("files") {
            None | Some(Value::Null) => &[][..],
            Some(Value::Array(files)) => files.as_slice(),
            _ => return Err("Qwen files 必须是数组或 null".into()),
        };
        let mut files = Vec::new();
        for file in raw_files {
            if !file.is_object() {
                return Err("Qwen 附件必须是对象".into());
            }
            let file_id = optional_id(file.get("id"), "附件编号")?;
            let name = match file.get("name") {
                None | Some(Value::Null) => None,
                Some(v) => Some(text(v, "附件名", 4096, true)?),
            };
            let size = match file.get("size") {
                None | Some(Value::Null) => None,
                Some(v) => Some(v.as_u64().ok_or("Qwen 附件大小无效")?),
            };
            coverage.attachments_with_missing_metadata +=
                usize::from(file_id.is_none() || name.is_none() || size.is_none());
            let mut normalized = json!({"source_file_id":file_id,"name":name,"byte_count":size,"payload_status":"not_retained","url_status":"not_collected"});
            for key in ["type", "file_type", "file_class"] {
                if let Some(v) = file.get(key) {
                    normalized[key] = if v.is_null() {
                        Value::Null
                    } else {
                        json!(text(v, "附件类型", 256, true)?)
                    };
                }
            }
            files.push(normalized);
        }
        coverage.attachment_references += files.len();
        coverage.attachment_payloads_omitted += files.len();
        let file_only = content.trim().is_empty() && !files.is_empty();
        if content.trim().is_empty() && files.is_empty() {
            coverage.empty_messages_skipped += 1;
            continue;
        }
        let event: EventInput = serde_json::from_value(json!({
            "occurred_at":time,"scope":scope,"role":role,"origin":if file_only {"external_quote"} else if role=="user" {"user_input"} else {"assistant_output"},
            "kind":if file_only {"file"} else {"message"},"content":content,
            "source":{"platform":"qwen","account_namespace":account,"conversation_id":source_id,"message_id":node},
            "capture":{"completeness":"partial","reason":"qwen_history_all_branches_visible_answers_attachment_metadata_only"},
            "metadata":{"import_adapter":"qwen-export","conversation_title":title,"branch":"all_exported_nodes","previous_message_id":parents[node],
                "qwen":{"node_id":node,"parent_id":parents[node],"children_ids":children[node],"on_current_path":selected.contains(node),"current_node":current,"conversation_has_branches":coverage.branch_points>0,"answer_segment_indices":answer_indices,"hidden_fragments_omitted":hidden,"unsupported_fragments_omitted":unsupported,"body_present":!file_only,"original_timestamp_seconds":message.get("timestamp")},
                "source_assets":{"schema":"recallcard.source-assets/1","files":files,"citations":[],"tool_trace":[]}}
        })).map_err(|e| e.to_string())?;
        event.validate()?;
        budget.add(&event)?;
        coverage.visible_messages_imported += usize::from(!file_only);
        coverage.file_only_messages_imported += usize::from(file_only);
        coverage.off_current_path_imported += usize::from(!selected.contains(node));
        events.push(event);
    }
    let saved: BTreeSet<_> = events.iter().map(|e| e.source.message_id.clone()).collect();
    let mut nearest: BTreeMap<String, Option<String>> = BTreeMap::new();
    let mut omitted = BTreeMap::new();
    for id in order {
        let parent = parents[&id].as_ref();
        let ancestor = if saved.contains(&id) {
            Some(id.clone())
        } else {
            parent.and_then(|p| nearest[p].clone())
        };
        let gap = if saved.contains(&id) {
            0usize
        } else {
            parent.map(|p| omitted[p]).unwrap_or(0) + 1
        };
        nearest.insert(id.clone(), ancestor);
        omitted.insert(id, gap);
    }
    for event in &mut events {
        let parent = parents[&event.source.message_id].as_ref();
        event.metadata["qwen"]["nearest_visible_parent_id"] =
            json!(parent.and_then(|p| nearest[p].clone()));
        event.metadata["qwen"]["omitted_parent_nodes"] =
            json!(parent.map(|p| omitted[p]).unwrap_or(0));
    }
    Ok(QwenConversation {
        source_id: source_id.into(),
        title,
        events,
        coverage,
    })
}
