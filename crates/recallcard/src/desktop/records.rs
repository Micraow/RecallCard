//! 工作区的可读来源信息。只从当前授权投影补充显示字段，不扩大检索或读取权限。
use super::{health_error, RESPONSE_LIMIT};
use crate::{
    context::{truncate_utf8, Context, Document},
    model::{Event, Result},
    Vault,
};
use serde_json::{json, Value};

pub(super) struct WorkspaceRecords {
    snapshot: std::sync::Arc<crate::context::Snapshot>,
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
    /// 直接用 Context 已经读好的快照：界面补字段时不再把正本重读一遍。
    pub(super) fn load(_vault: &Vault, context: &Context<'_>) -> Result<Self> {
        Ok(Self {
            snapshot: context.snapshot().map_err(|_| health_error())?,
        })
    }

    fn document(&self, reference: &str) -> Option<&Document> {
        self.snapshot
            .by_reference
            .get(reference)
            .map(|index| &self.snapshot.documents[*index])
    }

    /// 只给当前可见的事件：被修订或被抑制的旧事件不能借补字段透出去。
    fn event(&self, reference: &str) -> Option<&Event> {
        let document = self.snapshot.by_reference.get(reference)?;
        if self.snapshot.documents[*document].kind != "event" {
            return None;
        }
        self.snapshot
            .event_by_reference
            .get(reference)
            .map(|index| &self.snapshot.events[*index])
    }

    fn fields(&self, reference: &str) -> Value {
        if let Some(event) = self.event(reference) {
            return event_fields(event);
        }
        if let Some(doc) = self.document(reference).filter(|d| d.kind == "memory") {
            let sources: Vec<_> = doc
                .evidence_refs
                .iter()
                .filter_map(|reference| self.event(reference).map(|event| (reference, event)))
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
            if let Some(doc) = self.document(reference) {
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
}
