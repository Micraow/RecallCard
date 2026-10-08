//! 大正文的有界读取。游标只定位同一授权快照，不授予任何读取权限。
use super::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadPageArgs {
    pub refs: Vec<String>,
    #[serde(
        default = "default_budget",
        rename = "budget_bytes",
        alias = "budget_tokens"
    )]
    pub budget_tokens: usize,
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub offset_bytes: Option<usize>,
}

impl Context<'_> {
    pub fn read_page(&self, args: ReadPageArgs) -> Result<Value> {
        self.read_page_internal(args, false)
    }
    pub fn sources_page(&self, args: ReadPageArgs) -> Result<Value> {
        self.read_page_internal(args, true)
    }
    fn read_page_internal(&self, args: ReadPageArgs, sources: bool) -> Result<Value> {
        check_budget(args.budget_tokens)?;
        if args.refs.is_empty() || args.refs.len() > 32 {
            return Err("refs 必须包含 1–32 个引用".into());
        }
        if args.cursor.is_some() && args.offset_bytes.is_some() {
            return Err("cursor 与 offset_bytes 不能同时使用".into());
        }
        if args.refs.len() != 1 {
            if args.cursor.is_some() || args.offset_bytes.is_some() {
                return Err("续读或定位只接受一个 ref".into());
            }
            return self.read_internal(
                ReadArgs {
                    refs: args.refs,
                    budget_tokens: args.budget_tokens,
                },
                sources,
            );
        }
        let reference = &args.refs[0];
        let (kind, id, revision) = parse_ref(reference)?;
        if kind == "view" {
            if args.cursor.is_some() || args.offset_bytes.is_some() {
                return Err("view 不支持正文游标；请逐个读取 records/pending_refs".into());
            }
            return self.read_internal(
                ReadArgs {
                    refs: args.refs,
                    budget_tokens: args.budget_tokens,
                },
                sources,
            );
        }
        let _read_guard = self.vault.read_guard()?;
        let suppressed = self.vault.suppressed_ids()?;
        // 每一页重新检查授权和抑制；不得凭游标绕过。
        let (record, text, metadata, projection, retained) = if kind == "event" {
            let event = self.vault.event(id)?;
            if !self.access.permits(&event.data.scope) || suppressed.contains(id) {
                return Err("引用不可访问或已被抑制".into());
            }
            let text = event.data.text();
            let retained = event.data.kind != "file" || !text.is_empty();
            let completeness = event.data.capture.completeness.as_deref();
            let mut source = serde_json::to_value(&event.data.source).map_err(|e| e.to_string())?;
            let mut source_truncated = false;
            for value in source.as_object_mut().unwrap().values_mut() {
                if let Some(text) = value.as_str() {
                    if text.len() > 256 {
                        *value = json!(truncate_utf8(text, 256));
                        source_truncated = true;
                    }
                }
            }
            let part_origins = event
                .data
                .parts
                .iter()
                .map(|part| {
                    serde_json::to_value(&part.origin)
                        .map(|v| v.as_str().unwrap_or("unknown").to_owned())
                        .map_err(|e| e.to_string())
                })
                .collect::<Result<BTreeSet<_>>>()?;
            let metadata = json!({"kind":"event","scope":event.data.scope,"role":event.data.role,"origin":event.data.origin,"occurred_at":event.data.occurred_at,"source":source,"source_truncated":source_truncated,"part_origins":part_origins,"revision_of":event.data.revision_of,"reply_to":event.data.reply_to,"content_retained":retained,"chatgpt":{"on_current_path":event.data.metadata.pointer("/chatgpt/on_current_path").and_then(Value::as_bool)},"capture":{"completeness":completeness.map(|s|truncate_utf8(s,64)),"completeness_truncated":completeness.is_some_and(|s|s.len()>64),"redacted":event.data.capture.redacted,"redaction_count":event.data.capture.redaction_count}});
            (
                serde_json::to_value(&event).map_err(|e| e.to_string())?,
                text,
                metadata,
                "event.text",
                retained,
            )
        } else {
            let memory = self.vault.memory(id)?;
            if !self.memory_visible(&memory, &suppressed)? {
                return Err("引用不可访问或已被抑制".into());
            }
            if revision.is_some_and(|r| r != memory.revision) {
                return Err("引用的记忆版本已变化，请重新搜索".into());
            }
            if sources {
                return self.memory_sources_page(&memory, &args);
            }
            let metadata = json!({"kind":"memory","scope":memory.data.scope,"revision":memory.revision,"state":memory.state,"evidence":memory.data.evidence,"observed_at":memory.data.observed_at,"valid_from":memory.data.valid_from,"valid_to":memory.data.valid_to,"time_note":truncate_utf8(&memory.data.time_note,256),"time_note_truncated":memory.data.time_note.len()>256,"source_count":memory.data.source_refs.len()});
            (
                serde_json::to_value(&memory).map_err(|e| e.to_string())?,
                memory.data.content,
                metadata,
                "memory.content",
                true,
            )
        };
        let snapshot = hash(&serde_json::to_vec(&record).map_err(|e| e.to_string())?);
        let binding = page_binding(
            reference,
            &snapshot,
            &self.access.scopes(),
            if sources { "source-text" } else { "text" },
        );
        let offset = page_position(&args, &binding)?;
        if offset > text.len() || !text.is_char_boundary(offset) {
            return Err("offset_bytes 必须是正文范围内的 UTF-8 字符边界".into());
        }
        if args.cursor.is_none() && args.offset_bytes.is_none() {
            let full = json!({"results":[{"ref":reference,"record":record,"content_retained":retained,"retention":if retained{"inline"}else{"content_not_retained"}}],"truncated":false,"pending_refs":[],"next_cursor":null,"status":"complete","budget_unit":"utf8_bytes"});
            if json_size(&full)? <= args.budget_tokens {
                return Ok(full);
            }
        }
        // text 是明确标记的正文投影，不伪装成完整 Event/Memory 正本。
        let mut end = (offset + args.budget_tokens).min(text.len());
        loop {
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            let next_cursor = (end < text.len()).then(|| format!("p1:{binding}:{end}"));
            let response = json!({"results":[{"ref":reference,"metadata":metadata,"text":&text[offset..end],"text_range":{"start_byte":offset,"end_byte":end,"total_bytes":text.len(),"projection":projection},"snapshot":snapshot,"record_complete":false}],"truncated":end<text.len(),"pending_refs":[],"next_cursor":next_cursor,"status":if end<text.len(){"partial"}else{"complete"},"budget_unit":"utf8_bytes"});
            let size = json_size(&response)?;
            if size <= args.budget_tokens && (end > offset || offset == text.len()) {
                return Ok(response);
            }
            if end == offset {
                return budget_empty(reference, args.budget_tokens);
            }
            end = end
                .saturating_sub(size.saturating_sub(args.budget_tokens).max(1))
                .max(offset);
        }
    }

    fn memory_sources_page(&self, memory: &Memory, args: &ReadPageArgs) -> Result<Value> {
        if args.offset_bytes.is_some() {
            return Err(
                "sources(memory) 返回来源列表；请对单个 event 使用 read 的 offset_bytes".into(),
            );
        }
        let reference = &args.refs[0];
        let snapshot = hash(&serde_json::to_vec(memory).map_err(|e| e.to_string())?);
        let binding = page_binding(reference, &snapshot, &self.access.scopes(), "source-refs");
        let offset = page_position(args, &binding)?;
        let refs = &memory.data.source_refs;
        if offset > refs.len() {
            return Err("来源游标超出范围".into());
        }
        // 小证据集合保留完整 events 合同；大集合不将所有正文同时加载到内存。
        if args.cursor.is_none() {
            let mut events = Vec::new();
            let mut bytes = 0usize;
            for id in refs {
                let event = self.vault.event(id)?;
                bytes += serde_json::to_vec(&event).map_err(|e| e.to_string())?.len();
                if bytes > args.budget_tokens {
                    break;
                }
                events.push(event);
            }
            if events.len() == refs.len() {
                let missing = events
                    .iter()
                    .filter(|e| e.data.kind == "file" && e.data.text().is_empty())
                    .map(|e| format!("event:{}", e.id))
                    .collect::<Vec<_>>();
                let full = json!({"results":[{"ref":reference,"events":events,"content_not_retained":missing}],"truncated":false,"pending_refs":[],"next_cursor":null,"status":"complete","budget_unit":"utf8_bytes"});
                if json_size(&full)? <= args.budget_tokens {
                    return Ok(full);
                }
            }
        }
        let mut items = Vec::new();
        let mut response = Value::Null;
        for id in refs.iter().skip(offset) {
            items.push(format!("event:{id}"));
            let next = offset + items.len();
            let candidate = json!({"results":[{"ref":reference,"source_refs":items,"source_range":{"start":offset,"end":next,"total":refs.len()},"snapshot":snapshot}],"truncated":next<refs.len(),"pending_refs":[],"next_cursor":if next<refs.len(){Some(format!("p1:{binding}:{next}"))}else{None},"status":"source_refs","budget_unit":"utf8_bytes","hint":"逐个 read source_refs 获取原始正文；按 next_cursor 继续来源列表"});
            if json_size(&candidate)? > args.budget_tokens {
                break;
            }
            response = candidate;
        }
        if response.is_null() {
            budget_empty(reference, args.budget_tokens)
        } else {
            Ok(response)
        }
    }
}

fn page_binding(reference: &str, snapshot: &str, scopes: &[String], mode: &str) -> String {
    hash(
        json!([reference, snapshot, scopes, mode])
            .to_string()
            .as_bytes(),
    )
}
fn page_position(args: &ReadPageArgs, binding: &str) -> Result<usize> {
    match &args.cursor {
        None => Ok(args.offset_bytes.unwrap_or(0)),
        Some(cursor) => {
            if cursor.len() > 128 {
                return Err("读取游标无效".into());
            }
            let parts = cursor.split(':').collect::<Vec<_>>();
            if parts.len() != 3 || parts[0] != "p1" || parts[1] != binding {
                return Err("读取游标已失效或不属于此引用/范围，请重新搜索或读取".into());
            }
            parts[2].parse().map_err(|_| "读取游标偏移无效".into())
        }
    }
}
fn budget_empty(reference: &str, budget: usize) -> Result<Value> {
    let response = json!({"results":[],"pending_refs":[reference],"truncated":true,"next_cursor":null,"status":"budget_exhausted","budget_exhausted":true,"budget_unit":"utf8_bytes","hint":"资料存在，但当前预算放不下出处和正文；增大 budget_bytes，最大 32768"});
    if json_size(&response)? > budget {
        return Err("预算不足以输出读取状态".into());
    }
    Ok(response)
}
