//! 工作区的可读来源信息。只从当前授权投影补充显示字段，不扩大检索或读取权限。
use super::{health_error, DesktopSession, RESPONSE_LIMIT};
use crate::{
    context::{truncate_utf8, Context, Document},
    model::{Event, Result},
    Vault,
};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

pub(super) struct WorkspaceRecords {
    documents: BTreeMap<String, Document>,
    events: BTreeMap<String, Event>,
}

pub(super) fn conversation_title(event: &Event) -> String {
    event.data.metadata["conversation_title"]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .map(|s| truncate_utf8(s, 180))
        .unwrap_or_else(|| truncate_utf8(&event.data.text(), 120))
}

fn event_fields(event: &Event) -> Value {
    json!({
        "conversation_ref":event.data.session_key(),
        "conversation_title":conversation_title(event),
        "platform":truncate_utf8(&event.data.source.platform, 80),
        "role":event.data.role
    })
}

fn extend(item: &mut Value, fields: Value) {
    if let (Some(item), Some(fields)) = (item.as_object_mut(), fields.as_object()) {
        item.extend(fields.clone());
    }
}

fn response_size(value: &Value) -> Result<usize> {
    serde_json::to_vec(value)
        .map(|v| v.len())
        .map_err(|_| health_error())
}

impl WorkspaceRecords {
    /// 调用方在构造原响应之前持读锁；补充字段与原响应来自同一授权快照。
    pub(super) fn load(vault: &Vault, context: &Context<'_>) -> Result<Self> {
        let documents: BTreeMap<_, _> = context
            .documents_locked()
            .map_err(|_| health_error())?
            .into_iter()
            .map(|d| (d.reference.clone(), d))
            .collect();
        let events = vault
            .events()
            .map_err(|_| health_error())?
            .into_iter()
            .filter_map(|event| {
                let reference = format!("event:{}", event.id);
                documents
                    .get(&reference)
                    .filter(|d| d.kind == "event")
                    .map(|_| (reference, event))
            })
            .collect();
        Ok(Self { documents, events })
    }

    fn fields(&self, reference: &str) -> Value {
        if let Some(event) = self.events.get(reference) {
            return event_fields(event);
        }
        if let Some(doc) = self.documents.get(reference).filter(|d| d.kind == "memory") {
            let sources: Vec<_> = doc
                .evidence_refs
                .iter()
                .filter_map(|reference| self.events.get(reference).map(|event| (reference, event)))
                .collect();
            let rows: Vec<_> = sources
                .iter()
                .take(2)
                .map(|(reference, event)| {
                    let mut row = event_fields(event);
                    row["ref"] = json!(reference);
                    row["occurred_at"] = json!(event.data.occurred_at);
                    row
                })
                .collect();
            return json!({"source_summary":{"count":sources.len(),"sources":rows,"truncated":sources.len()>2}});
        }
        json!({})
    }

    pub(super) fn enrich_list(&self, mut response: Value) -> Result<Value> {
        let count = response["results"].as_array().map_or(0, Vec::len);
        for index in 0..count {
            let original = response["results"][index].clone();
            let reference = original["ref"].as_str().unwrap_or("");
            extend(&mut response["results"][index], self.fields(reference));
            if let Some(doc) = self.documents.get(reference) {
                if original["text"]
                    .as_str()
                    .is_some_and(|text| text.len() < doc.text.len())
                {
                    response["results"][index]["text_truncated"] = json!(true);
                }
            }
            // 显示字段计入原有 UTF-8 字节预算。优先缩短片段，不移除结果、不改变游标。
            while response_size(&response)? > RESPONSE_LIMIT {
                let text = response["results"][index]["text"].as_str().unwrap_or("");
                if text.len() <= 128 {
                    // 异常长的自定义 session_id 等仍可能无法放入预算；保留原合法响应。
                    response["results"][index] = original;
                    break;
                }
                response["results"][index]["text"] =
                    json!(truncate_utf8(text, (text.len() / 2).max(128)));
                response["results"][index]["text_truncated"] = json!(true);
            }
        }
        Ok(response)
    }

    pub(super) fn enrich_read(&self, response: Value) -> Result<Value> {
        let mut enriched = response.clone();
        if let Some(rows) = enriched["results"].as_array_mut() {
            for row in rows {
                let reference = row["ref"].as_str().unwrap_or("").to_owned();
                extend(row, self.fields(&reference));
                if let Some(events) = row["events"].as_array_mut() {
                    for event in events {
                        let reference = format!("event:{}", event["id"].as_str().unwrap_or(""));
                        extend(event, self.fields(&reference));
                    }
                }
            }
        }
        // 正文和来源必须保持完整；仅可选显示字段在极限预算下省略。
        Ok(if response_size(&enriched)? <= RESPONSE_LIMIT {
            enriched
        } else {
            response
        })
    }

    pub(super) fn imported_conversations(&self, references: &[String]) -> Vec<Value> {
        let mut seen = BTreeSet::new();
        let mut groups = BTreeMap::new();
        for reference in references {
            if !seen.insert(reference) {
                continue;
            }
            let Some(event) = self.events.get(reference) else {
                continue;
            };
            let key = event.data.session_key();
            let group = groups.entry(key).or_insert((event, 0, event.captured_at));
            group.1 += 1;
            group.2 = group.2.max(event.captured_at);
        }
        groups
            .into_iter()
            .map(|(key, (event, count, captured_at))| {
                json!({
                    "session_ref":key,"title":conversation_title(event),
                    "platform":truncate_utf8(&event.data.source.platform,80),
                    "message_count":count,"captured_at":captured_at,"coverage":"partial"
                })
            })
            .collect()
    }
}

impl DesktopSession {
    /// 从当前可见的 Event 正本定位会话。UI 不应推导 session hash 或使用历史页序号。
    pub fn event_location(&self, session_id: &str, scope: &str, reference: &str) -> Result<Value> {
        super::read_args(reference)?;
        if !reference.starts_with("event:") {
            return Err("请选择一条原始消息进行定位".into());
        }
        let vault = self.vault(session_id)?;
        let _guard = vault.read_guard().map_err(|_| health_error())?;
        let context = self.context(session_id, scope)?;
        let records = WorkspaceRecords::load(vault, &context)?;
        let event = records
            .events
            .get(reference)
            .ok_or("该消息不可访问、已变化或已被遗忘，请重新检索")?;
        let key = event.data.session_key();
        if key.len() > 4096 {
            return Err("会话编号过长，无法定位，请从原始记录查看".into());
        }
        // 与 conversation_messages 使用相同的正本顺序；断开的片段也不能按 hash 重排。
        let events = vault
            .events()
            .map_err(|_| health_error())?
            .into_iter()
            .filter(|event| {
                event.data.session_key() == key
                    && records.events.contains_key(&format!("event:{}", event.id))
            })
            .collect();
        let (events, _) = crate::conversation::ordered_events(events);
        let index = events
            .iter()
            .position(|candidate| candidate.id == event.id)
            .ok_or("会话记录已经变化，请重新检索")?;
        let mut location = event_fields(event);
        location["ref"] = json!(reference);
        location["message_index"] = json!(index);
        // 直接以目标开页，不能以 index/20 猜页：长正文会提前耗尽单页预算。
        location["offset"] = json!(index);
        location["total"] = json!(events.len());
        Ok(location)
    }
}
