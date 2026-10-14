//! 工作区的可读来源信息。只从当前授权投影补充显示字段，不扩大检索或读取权限。
use super::{health_error, DesktopSession, RESPONSE_LIMIT};
use crate::{
    context::{truncate_utf8, Context, Document},
    model::{Event, EventInput, Result},
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

    fn attachment_parent(&self, reference: &str) -> Option<&Event> {
        let (id, object) = reference.strip_prefix("attachment:")?.split_once('/')?;
        if object.len() != 64
            || !object
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return None;
        }
        let event = self.events.get(&format!("event:{id}"))?;
        event.data.metadata["source_assets"]["files"]
            .as_array()?
            .iter()
            .any(|file| file["object_sha256"] == object && file["payload_status"] == "retained")
            .then_some(event)
    }

    fn fields(&self, reference: &str) -> Value {
        if let Some(event) = self.attachment_parent(reference) {
            let mut fields = event_fields(event);
            fields["source_ref"] = json!(format!("event:{}", event.id));
            return fields;
        }
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

    pub(super) fn enrich_current_search(&self, response: Value) -> Result<Value> {
        let now = chrono::Utc::now();
        if response["results"].as_array().is_none_or(|rows| {
            rows.iter().any(|r| {
                r["ref"].as_str().is_none_or(|reference| {
                    self.attachment_parent(reference).is_none()
                        && self
                            .documents
                            .get(reference)
                            .is_none_or(|d| d.kind == "memory" && !d.current_memory_at(now))
                })
            })
        }) {
            return Err("查询期间来源或版本已改变，请重新检索".into());
        }
        self.enrich_list(response)
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

    /// 冻结输入只能用于确认成员关系，标题与数量仍从当前可见正本读取。
    /// 与 capture_batch 相同，自动补上的 revision_of 不改变导入版本身份。
    pub(super) fn imported_input_conversations(&self, inputs: &[EventInput]) -> Result<Vec<Value>> {
        fn version_key(input: &EventInput) -> Result<String> {
            let mut normalized = input.clone();
            normalized.revision_of = None;
            normalized.id()
        }
        let versions = inputs
            .iter()
            .map(version_key)
            .collect::<Result<BTreeSet<_>>>()?;
        let mut references = Vec::new();
        for (reference, event) in &self.events {
            if versions.contains(&version_key(&event.data)?) {
                references.push(reference.clone());
            }
        }
        Ok(self.imported_conversations(&references))
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
        let branches = super::branches::ConversationBranches::new(events)?;
        let index = branches
            .order()
            .iter()
            .position(|index| branches.events[*index].id == event.id)
            .ok_or("会话记录已经变化，请重新检索")?;
        let mut location = event_fields(event);
        location["ref"] = json!(reference);
        location["message_index"] = json!(index);
        // 直接以目标开页，不能以 index/20 猜页：长正文会提前耗尽单页预算。
        location["offset"] = json!(index);
        location["total"] = json!(branches.events.len());
        Ok(location)
    }
}

/// 只返回受限引用、原件状态和类型元数据，不打开网址、原件或提取附件正文。
pub(super) fn source_assets(event: &crate::Event) -> Value {
    let assets = &event.data.metadata["source_assets"];
    if !assets.is_object() {
        return Value::Null;
    }
    let mut used = 0usize;
    let mut truncated = false;
    let mut project = |key: &str| -> Vec<Value> {
        let Some(entries) = assets[key].as_array() else {
            return Vec::new();
        };
        let mut output = Vec::new();
        for entry in entries {
            let mut row = serde_json::Map::new();
            for field in [
                "source_file_id",
                "name",
                "url",
                "title",
                "type",
                "payload_status",
            ] {
                if let Some(value) = entry[field].as_str() {
                    let max = if field == "url" { 600 } else { 200 };
                    let clipped = truncate_utf8(value, max);
                    truncated |= clipped.len() != value.len();
                    row.insert(field.into(), json!(clipped));
                }
            }
            for field in ["byte_count", "reference_count"] {
                if let Some(value) = entry[field].as_u64() {
                    row.insert(field.into(), json!(value));
                }
            }
            if key == "files" {
                for field in [
                    "declared_media_type",
                    "library_media_type",
                    "detected_media_type",
                    "object_sha256",
                    "archive_name_alias",
                ] {
                    if let Some(value) = entry[field].as_str() {
                        let clipped = truncate_utf8(value, 200);
                        truncated |= clipped.len() != value.len();
                        row.insert(field.into(), json!(clipped));
                    }
                }
                if let Some(value) = entry["declared_media_type_conflict"].as_bool() {
                    row.insert("declared_media_type_conflict".into(), json!(value));
                }
            }
            let row = Value::Object(row);
            let bytes = row.to_string().len();
            if output.len() >= 20 || used + bytes > 6000 {
                truncated = true;
                continue;
            }
            used += bytes;
            output.push(row);
        }
        output
    };
    let files = project("files");
    let citations = project("citations");
    let tool_trace = project("tool_trace");
    let total = |key: &str| assets[key].as_array().map_or(0, Vec::len);
    json!({"files":files,"citations":citations,"tool_trace":tool_trace,"files_total":total("files"),"citations_total":total("citations"),"trace_total":total("tool_trace"),"unsupported_total":total("unsupported_fragments"),"truncated":truncated})
}

#[cfg(test)]
mod attachment_projection_tests {
    use super::*;

    fn synthetic_event(assets: Value) -> Event {
        serde_json::from_value(json!({
            "schema_version": 1,
            "id": "evt_synthetic_attachment_projection",
            "captured_at": "2026-01-01T00:00:00Z",
            "role": "user",
            "source": {"platform": "chatgpt", "conversation_id": "synthetic", "message_id": "message"},
            "metadata": {"source_assets": assets}
        }))
        .unwrap()
    }

    #[test]
    fn attachment_projection_preserves_status_name_alias_and_metadata_without_payload() {
        let files: Vec<_> = ["retained", "missing", "available_not_retained", "not_in_export"]
            .iter()
            .map(|status| {
                json!({
                    "source_file_id": "file_synthetic", "name": "<script>message-name.pdf</script>",
                    "byte_count": 512, "payload_status": status,
                    "declared_media_type": "application/pdf", "library_media_type": "application/octet-stream",
                    "detected_media_type": "application/zip", "declared_media_type_conflict": true,
                    "object_sha256": "a".repeat(64), "archive_name_alias": "file_synthetic.dat",
                    "local_path": "/synthetic/not-for-ui", "content": "do not project raw bytes",
                    "payload": "synthetic opaque bytes", "extracted_text": "not indexed"
                })
            })
            .collect();
        let projection = source_assets(&synthetic_event(json!({"files": files})));
        assert_eq!(projection["files_total"], 4);
        assert_eq!(projection["truncated"], false);
        for (index, status) in [
            "retained",
            "missing",
            "available_not_retained",
            "not_in_export",
        ]
        .iter()
        .enumerate()
        {
            let row = &projection["files"][index];
            assert_eq!(row["payload_status"], *status);
            assert_eq!(row["name"], "<script>message-name.pdf</script>");
            assert_eq!(row["archive_name_alias"], "file_synthetic.dat");
            assert_eq!(row["source_file_id"], "file_synthetic");
            assert_eq!(row["declared_media_type"], "application/pdf");
            assert_eq!(row["library_media_type"], "application/octet-stream");
            assert_eq!(row["detected_media_type"], "application/zip");
            assert_eq!(row["declared_media_type_conflict"], true);
            assert_eq!(row["object_sha256"], "a".repeat(64));
            for field in ["local_path", "content", "payload", "extracted_text"] {
                assert!(row.get(field).is_none());
            }
        }
    }

    #[test]
    fn attachment_projection_keeps_existing_utf8_count_and_byte_limits() {
        let row = json!({
            "name": "合成".repeat(200), "archive_name_alias": "别名".repeat(200),
            "payload_status": "retained", "object_sha256": "a".repeat(64),
            "declared_media_type_conflict": "not a boolean"
        });
        let projection = source_assets(&synthetic_event(json!({"files": vec![row; 25]})));
        assert_eq!(projection["files_total"], 25);
        assert_eq!(projection["truncated"], true);
        let rows = projection["files"].as_array().unwrap();
        assert!(rows.len() <= 20);
        assert!(rows.iter().map(|row| row.to_string().len()).sum::<usize>() <= 6000);
        for row in rows {
            assert!(row["name"].as_str().unwrap().len() <= 200);
            assert!(row["archive_name_alias"].as_str().unwrap().len() <= 200);
            assert!(row.get("declared_media_type_conflict").is_none());
        }
        assert!(source_assets(&synthetic_event(Value::Null)).is_null());
    }
}
