//! DeepSeek 官方导出适配器。仅保存明确可见的正文，保留导出树的来源关系。
//! 不选择「最长」分支，不把重新生成的兄弟回答拼成一段线性对话。
use crate::model::{validate_scope, EventInput, Origin, Result, Role};
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Default, Clone, Serialize, PartialEq, Eq)]
pub struct DeepseekCoverage {
    pub nodes_seen: usize,
    pub message_nodes_seen: usize,
    pub visible_messages_imported: usize,
    pub empty_messages_skipped: usize,
    pub hidden_only_messages_skipped: usize,
    pub unsupported_messages_skipped: usize,
    pub ambiguous_role_messages_skipped: usize,
    pub hidden_fragments_skipped: usize,
    pub unsupported_fragments_skipped: usize,
    pub attachments_skipped: usize,
    pub branch_points: usize,
    pub roots: usize,
    pub missing_message_timestamps: usize,
}

impl DeepseekCoverage {
    pub(crate) fn add(&mut self, other: &Self) {
        self.nodes_seen += other.nodes_seen;
        self.message_nodes_seen += other.message_nodes_seen;
        self.visible_messages_imported += other.visible_messages_imported;
        self.empty_messages_skipped += other.empty_messages_skipped;
        self.hidden_only_messages_skipped += other.hidden_only_messages_skipped;
        self.unsupported_messages_skipped += other.unsupported_messages_skipped;
        self.ambiguous_role_messages_skipped += other.ambiguous_role_messages_skipped;
        self.hidden_fragments_skipped += other.hidden_fragments_skipped;
        self.unsupported_fragments_skipped += other.unsupported_fragments_skipped;
        self.attachments_skipped += other.attachments_skipped;
        self.branch_points += other.branch_points;
        self.roots += other.roots;
        self.missing_message_timestamps += other.missing_message_timestamps;
    }
}

pub(crate) struct DeepseekConversation {
    pub source_id: String,
    pub title: Option<String>,
    pub events: Vec<EventInput>,
    pub coverage: DeepseekCoverage,
}

/// 不依赖 conversations.json 文件名，也不把只有 mapping 的未知平台当成 DeepSeek。
pub(crate) fn looks_like_deepseek(value: &Value) -> bool {
    value.is_object()
        && (value.get("inserted_at").is_some() && value.get("updated_at").is_some()
            || value["mapping"].as_object().is_some_and(|nodes| {
                nodes
                    .values()
                    .any(|node| node["message"].get("fragments").is_some())
            }))
        && value.get("mapping").is_some()
}

fn identity<'a>(value: &'a Value, label: &str) -> Result<&'a str> {
    value
        .as_str()
        .filter(|s| !s.trim().is_empty() && s.len() <= 2048 && !s.chars().any(char::is_control))
        .ok_or_else(|| format!("DeepSeek {label}缺失或无效"))
}

fn timestamp(value: Option<&Value>, label: &str) -> Result<Option<DateTime<Utc>>> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => DateTime::parse_from_rfc3339(text)
            .map(|t| Some(t.with_timezone(&Utc)))
            .map_err(|_| format!("DeepSeek {label}必须是有效的原始 RFC 3339 时间或 null")),
        _ => Err(format!("DeepSeek {label}必须是原始 RFC 3339 时间或 null")),
    }
}

pub(crate) fn parse_deepseek_conversation(
    value: &Value,
    scope: &str,
) -> Result<DeepseekConversation> {
    validate_scope(scope)?;
    let source_id = identity(&value["id"], "会话编号")?;
    let title = match value.get("title") {
        None | Some(Value::Null) => None,
        Some(Value::String(title)) if title.len() <= 4096 => Some(title.clone()),
        _ => return Err("DeepSeek 会话标题无效或超过 4096 字节".into()),
    };
    timestamp(value.get("inserted_at"), "会话 inserted_at")?;
    timestamp(value.get("updated_at"), "会话 updated_at")?;
    let mapping = value["mapping"]
        .as_object()
        .ok_or("DeepSeek 对话缺少有效 mapping")?;
    let mut parents: BTreeMap<&str, Option<&str>> = BTreeMap::new();
    let mut children: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    let mut coverage = DeepseekCoverage {
        nodes_seen: mapping.len(),
        ..Default::default()
    };
    // 结构损坏时整份预览失败，避免有效正文掩盖丢失或歧义的节点身份。
    for (key, node) in mapping {
        if !node.is_object() || identity(&node["id"], "节点编号")? != key {
            return Err("DeepSeek 节点编号与 mapping 键不一致".into());
        }
        let parent = match node.get("parent") {
            Some(Value::Null) => None,
            // 结构模板允许空根节点省略 parent；统一规范化为空引用。
            None if node.get("message") == Some(&Value::Null) => None,
            Some(value) => Some(identity(value, "父节点编号")?),
            None => return Err("DeepSeek 消息节点缺少 parent".into()),
        };
        let list = node["children"]
            .as_array()
            .ok_or("DeepSeek 节点缺少有效 children 数组")?;
        let mut ids = BTreeSet::new();
        for child in list {
            if !ids.insert(identity(child, "子节点编号")?) {
                return Err("DeepSeek children 包含重复节点编号".into());
            }
        }
        if node.get("message").is_none() {
            return Err("DeepSeek 节点缺少 message".into());
        }
        coverage.branch_points += usize::from(ids.len() > 1);
        coverage.roots += usize::from(parent.is_none());
        parents.insert(key, parent);
        children.insert(key, ids.into_iter().collect());
    }
    for (id, parent) in &parents {
        if let Some(parent) = parent {
            if !parents.contains_key(parent) || children[parent].binary_search(id).is_err() {
                return Err("DeepSeek 包含孤立节点或 parent / children 引用不一致".into());
            }
        }
        for child in &children[id] {
            if parents.get(child) != Some(&Some(*id)) {
                return Err("DeepSeek 包含缺失子节点或 parent / children 引用不一致".into());
            }
        }
    }
    // 有界拓扑遍历覆盖所有根和所有分支；同层按编号排放仅用于稳定存储，不是时间顺序。
    let mut ready: BTreeSet<&str> = parents
        .iter()
        .filter_map(|(id, p)| p.is_none().then_some(*id))
        .collect();
    let mut order = Vec::with_capacity(mapping.len());
    while let Some(id) = ready.pop_first() {
        order.push(id);
        ready.extend(children[id].iter().copied());
    }
    if order.len() != mapping.len() {
        return Err("DeepSeek 会话包含循环引用；没有导入任何消息".into());
    }
    let mut events = Vec::new();
    for id in order.iter().copied() {
        let message = &mapping[id]["message"];
        if message.is_null() {
            continue;
        }
        coverage.message_nodes_seen += 1;
        if !message.is_object() {
            return Err("DeepSeek message 必须是对象或 null".into());
        }
        let model = match message.get("model") {
            None | Some(Value::Null) => None,
            Some(Value::String(model))
                if model.len() <= 256 && !model.chars().any(char::is_control) =>
            {
                Some(model.as_str())
            }
            _ => return Err("DeepSeek model 字段无效".into()),
        };
        let time = timestamp(message.get("inserted_at"), "消息 inserted_at")?;
        coverage.missing_message_timestamps += usize::from(time.is_none());
        let attachments = match message.get("files") {
            None | Some(Value::Null) => 0,
            Some(Value::Array(files)) => files.len(),
            _ => return Err("DeepSeek files 必须是数组或 null".into()),
        };
        coverage.attachments_skipped += attachments;
        let fragments = message["fragments"]
            .as_array()
            .ok_or("DeepSeek 消息缺少 fragments 数组")?;
        let mut user = false;
        let mut assistant = false;
        let mut texts = Vec::new();
        let mut hidden = 0usize;
        let mut unsupported = 0usize;
        let mut malformed_visible = false;
        for fragment in fragments {
            match fragment["type"].as_str() {
                Some("REQUEST") | Some("RESPONSE") | Some("TEMPLATE_RESPONSE") => {
                    user |= fragment["type"] == "REQUEST";
                    assistant |= fragment["type"] != "REQUEST";
                    match fragment["content"].as_str() {
                        Some(text) => texts.push(text),
                        None => {
                            unsupported += 1;
                            malformed_visible = true;
                        }
                    }
                }
                Some("THINK") => hidden += 1,
                _ => unsupported += 1,
            }
        }
        coverage.hidden_fragments_skipped += hidden;
        coverage.unsupported_fragments_skipped += unsupported;
        if user && assistant {
            coverage.ambiguous_role_messages_skipped += 1;
            continue;
        }
        if malformed_visible {
            coverage.unsupported_messages_skipped += 1;
            continue;
        }
        if !user && !assistant {
            if hidden > 0 && unsupported == 0 {
                coverage.hidden_only_messages_skipped += 1;
            } else if fragments.is_empty() {
                coverage.empty_messages_skipped += 1;
            } else {
                coverage.unsupported_messages_skipped += 1;
            }
            continue;
        }
        // 同一消息的 fragment 是连续正文片段，不凭空插入换行或其他内容。
        let content = texts.concat();
        if content.trim().is_empty() {
            coverage.empty_messages_skipped += 1;
            continue;
        }
        let role = if user { Role::User } else { Role::Assistant };
        let origin = if user {
            Origin::UserInput
        } else {
            Origin::AssistantOutput
        };
        let event: EventInput = serde_json::from_value(json!({
            "occurred_at":time,"scope":scope,"role":role,"origin":origin,"content":content,
            "source":{"platform":"deepseek","conversation_id":source_id,"message_id":id},
            "capture":{"completeness":"partial","reason":"provided_export_visible_fragments_only_all_branches"},
            "metadata":{"import_adapter":"deepseek-export","conversation_title":title,"branch":"all_exported_nodes","previous_message_id":parents[id],
                "deepseek":{"model":model,"node_id":id,"parent_id":parents[id],"children_ids":children[id],
                    "conversation_has_branches":coverage.branch_points > 0,
                    "parent_is_structural_root": parents[id].is_some_and(|parent| mapping[parent]["message"].is_null() && parents[parent].is_none()),
                    "role_inferred_from_fragments":true,"attachments_omitted":attachments,
                    "hidden_fragments_omitted":hidden,"unsupported_fragments_omitted":unsupported}}
        })).map_err(|e| e.to_string())?;
        event.validate()?;
        coverage.visible_messages_imported += 1;
        events.push(event);
    }
    let imported_ids: BTreeSet<String> =
        events.iter().map(|e| e.source.message_id.clone()).collect();
    // 一次拓扑传播可见祖先；跨过故意省略的消息时保留缺口，不能每节点回溯整链。
    let mut visible_ancestor: BTreeMap<&str, Option<&str>> = BTreeMap::new();
    let mut omitted_run: BTreeMap<&str, usize> = BTreeMap::new();
    for id in order {
        let parent = parents[id];
        let ancestor = if imported_ids.contains(id) {
            Some(id)
        } else {
            parent.and_then(|p| visible_ancestor[p])
        };
        let omitted = if imported_ids.contains(id)
            || (parent.is_none() && mapping[id]["message"].is_null())
        {
            0
        } else {
            parent.map(|p| omitted_run[p]).unwrap_or(0) + 1
        };
        visible_ancestor.insert(id, ancestor);
        omitted_run.insert(id, omitted);
    }
    for event in &mut events {
        let parent = parents[event.source.message_id.as_str()];
        event.metadata["deepseek"]["nearest_visible_parent_id"] =
            json!(parent.and_then(|p| visible_ancestor[p]));
        event.metadata["deepseek"]["omitted_parent_nodes"] =
            json!(parent.map(|p| omitted_run[p]).unwrap_or(0));
        event.metadata["deepseek"]["parent_message_omitted"] =
            json!(parent
                .is_some_and(|p| !mapping[p]["message"].is_null() && !imported_ids.contains(p)));
    }
    Ok(DeepseekConversation {
        source_id: source_id.into(),
        title,
        events,
        coverage,
    })
}
