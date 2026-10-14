//! 附件只读 UTF-8 投影；不拼接 Event 正文、不执行文件，也不调用模型。
use super::*;
use std::io::Read;

const FILE_BYTES: usize = 1024 * 1024;
const TOTAL_BYTES: usize = 16 * FILE_BYTES;
const FILES: usize = 256;
pub(super) const PROJECTION: &str = "attachment.utf8/1";

pub(super) fn parse(reference: &str) -> Result<(&str, &str)> {
    let (event, object) = reference
        .strip_prefix("attachment:")
        .and_then(|r| r.split_once('/'))
        .ok_or("附件引用无效")?;
    validate_id(event, "evt_")?;
    if !digest_valid(object) {
        return Err("附件引用无效".into());
    }
    Ok((event, object))
}
fn digest_valid(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn supported(file: &Value) -> bool {
    // 声明只选解码器；完整字节仍须验证。已识别的 PDF、图片和压缩包不能冒充文本。
    let media = file["declared_media_type"]
        .as_str()
        .or_else(|| file["library_media_type"].as_str());
    file["payload_status"] == "retained"
        && file["declared_media_type_conflict"] != true
        && file["detected_media_type"]
            .as_str()
            .is_some_and(|m| matches!(m, "application/octet-stream" | "text/plain"))
        && media.is_some_and(|m| {
            matches!(
                m,
                "text/plain"
                    | "text/markdown"
                    | "text/csv"
                    | "text/tab-separated-values"
                    | "application/json"
            )
        })
}
fn body(vault: &Vault, file: &Value) -> Result<String> {
    if !supported(file) {
        return Err("附件类型尚不支持安全文本读取".into());
    }
    let object = file["object_sha256"]
        .as_str()
        .filter(|s| digest_valid(s))
        .ok_or("附件身份无效")?;
    let size = file["byte_count"]
        .as_u64()
        .filter(|n| *n <= FILE_BYTES as u64)
        .ok_or("附件超过文本读取上限")?;
    let directory = vault.root().join("objects");
    crate::vault::reject_symlink(&directory)?;
    let path = directory.join(object);
    crate::vault::reject_symlink(&path)?;
    let input = crate::filesystem::open_local_file(&path)?;
    let metadata = input.metadata().map_err(|_| "附件原件不可用")?;
    if !metadata.is_file() || metadata.len() != size {
        return Err("附件原件大小已变化".into());
    }
    let mut bytes = Vec::new();
    input
        .take(size + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "附件原件不可用")?;
    if bytes.len() as u64 != size || hash(&bytes) != object {
        return Err("附件原件摘要已变化".into());
    }
    // 不归一化换行、不移除 BOM，所有坐标始终指向原件的 UTF-8 字节。
    let text = String::from_utf8(bytes).map_err(|_| "附件不是有效 UTF-8 文本")?;
    if text.starts_with("%PDF-")
        || text.starts_with("PK\u{3}\u{4}")
        || text
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\t' | '\r' | '\n'))
    {
        return Err("附件包含非文本内容".into());
    }
    Ok(text)
}
fn files(event: &Event) -> impl Iterator<Item = &Value> {
    event.data.metadata["source_assets"]["files"]
        .as_array()
        .into_iter()
        .flatten()
}
// 与分页、复制共用同一验证器；调用者必须先核验当前父事件的授权。
fn verified_body(vault: &Vault, event: &Event, object: &str) -> Result<(Value, String)> {
    // 多个附件声明可能共用同一原件；与搜索一样跳过不支持或损坏的声明。
    // 不能让无效的第一条声明遮蔽后面的有效关联，也不能无限重复读取同一对象。
    let mut examined_bytes = 0usize;
    let mut selected = None;
    for file in files(event)
        .take(FILES)
        .filter(|f| f["object_sha256"] == object)
    {
        let Some(size) = file["byte_count"]
            .as_u64()
            .filter(|size| *size <= FILE_BYTES as u64)
        else {
            continue;
        };
        if !supported(file) {
            continue;
        }
        if examined_bytes + size as usize > TOTAL_BYTES {
            break;
        }
        examined_bytes += size as usize;
        if let Ok(text) = body(vault, file) {
            selected = Some((file, text));
            break;
        }
    }
    selected
        .map(|(file, text)| (file.clone(), text))
        .ok_or_else(|| "附件关联或原件无法核验".into())
}
fn projection(parent: &Document, event: &Event, file: &Value, text: String) -> Document {
    let mut doc = parent.clone();
    doc.reference = format!(
        "attachment:{}/{}",
        event.id,
        file["object_sha256"].as_str().unwrap_or("")
    );
    doc.kind = "attachment".into();
    doc.text = text;
    doc.evidence = "Attachment/Content".into();
    doc.evidence_refs = vec![parent.reference.clone()];
    doc.entities = [
        "name",
        "library_name",
        "archive_name_alias",
        "source_file_id",
    ]
    .iter()
    .filter_map(|key| file[*key].as_str().map(str::to_owned))
    .collect();
    doc
}
fn ordinary(doc: &Document) -> bool {
    doc.kind == "event" && matches!(doc.role, Some(Role::User | Role::Assistant))
}
impl Context<'_> {
    /// 输入必须来自当前读锁内 documents_locked 的授权/修订/抑制过滤。
    pub(super) fn attachment_documents(
        &self,
        parents: &[Document],
    ) -> Result<(Vec<Document>, Value)> {
        let mut docs = Vec::new();
        let (mut examined, mut bytes, mut skipped, mut limited) = (0usize, 0usize, 0usize, false);
        let allowed: BTreeMap<_, _> = parents
            .iter()
            .filter(|d| ordinary(d))
            .map(|d| (d.reference.as_str(), d))
            .collect();
        self.vault.visit_events(|event| {
            if limited {
                return Ok(());
            }
            let key = format!("event:{}", event.id);
            let Some(parent) = allowed.get(key.as_str()) else {
                return Ok(());
            };
            let mut seen = BTreeSet::new();
            for file in files(&event) {
                if examined >= FILES {
                    limited = true;
                    return Ok(());
                }
                examined += 1;
                let declared = file["byte_count"].as_u64().unwrap_or(u64::MAX);
                if declared > FILE_BYTES as u64 || !supported(file) {
                    skipped += 1;
                    continue;
                }
                if bytes + declared as usize > TOTAL_BYTES {
                    limited = true;
                    return Ok(());
                }
                // 失败读取也消耗预算，不让损坏原件绕过总 I/O 上限。
                bytes += declared as usize;
                let text = match body(self.vault, file) {
                    Ok(text) => text,
                    Err(_) => {
                        skipped += 1;
                        continue;
                    }
                };
                let object = file["object_sha256"].as_str().ok_or("附件身份无效")?;
                if !seen.insert(object.to_owned()) {
                    continue;
                }
                docs.push(projection(parent, &event, file, text));
            }
            Ok(())
        })?;
        let coverage = json!({"status":if limited || skipped > 0 {"partial"}else{"ready"},"projection":PROJECTION,"mode":"bounded_local_lexical_scan","semantic":false,"files_examined":examined,"documents":docs.len(),"skipped":skipped,"limited":limited,"bytes_examined":bytes,"file_bytes_limit":FILE_BYTES,"total_bytes_limit":TOTAL_BYTES,"file_limit":FILES});
        Ok((docs, coverage))
    }
    /// 调用者持有 vault 读锁，parents 必须来自该锁内的 documents_locked。
    /// 只读取所选附件，不扫描其他对象；不获取嵌套读锁。
    pub(crate) fn attachment_document_locked(
        &self,
        reference: &str,
        parents: &[Document],
    ) -> Result<Document> {
        let (id, object) = parse(reference)?;
        let parent = parents
            .iter()
            .find(|d| d.reference == format!("event:{id}") && ordinary(d))
            .ok_or("附件来源不可访问、已修订或已抑制")?;
        let event = self.vault.event(id)?;
        let (file, text) = verified_body(self.vault, &event, object)?;
        Ok(projection(parent, &event, &file, text))
    }
    pub(super) fn attachment_page(&self, args: &ReadPageArgs, sources: bool) -> Result<Value> {
        if args.refs.len() != 1
            || args.include_adjacent
            || args.context_anchor_ref.is_some()
            || args.event_filter.is_some()
            || args.as_of.is_some()
            || args.context_confirmation.is_some()
            || args.context_review.is_some()
        {
            return Err("附件只支持单个引用的正文或来源读取，不支持会话导航".into());
        }
        let reference = &args.refs[0];
        let (id, object) = parse(reference)?;
        let _guard = self.vault.read_guard()?;
        // 定位符不是权限；每页检查父事件仍在当前有效的授权快照内。
        let parent = self
            .documents_locked()?
            .into_iter()
            .find(|d| d.reference == format!("event:{id}") && ordinary(d))
            .ok_or("附件来源不可访问、已修订或已抑制")?;
        let event = self.vault.event(id)?;
        let (file, text) = verified_body(self.vault, &event, object)?;
        let snapshot = hash(
            &serde_json::to_vec(&json!([PROJECTION, event, object])).map_err(|_| "附件快照无效")?,
        );
        if sources {
            if args.cursor.is_some() || args.offset_bytes.is_some() {
                return Err("附件 sources 返回父事件引用，不支持正文偏移".into());
            }
            let result = json!({"results":[{"ref":reference,"source_refs":[parent.reference],"snapshot":snapshot}],"status":"source_refs","truncated":false,"next_cursor":null,"budget_unit":"utf8_bytes"});
            return if json_size(&result)? <= args.budget_tokens {
                Ok(result)
            } else {
                paging::budget_empty(reference, args.budget_tokens, json_size(&result)?)
            };
        }
        let binding = paging::page_binding(reference, &snapshot, &self.access.scopes(), PROJECTION);
        let offset = paging::page_position(args, &binding)?;
        if offset > text.len() || !text.is_char_boundary(offset) {
            return Err("附件偏移必须位于 UTF-8 字符边界".into());
        }
        let mut end = (offset + args.budget_tokens).min(text.len());
        loop {
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            let result = json!({"results":[{"ref":reference,"metadata":{"kind":"attachment","scope":parent.scope,"source_ref":parent.reference,"object_sha256":object,"content_retained":true,"name":file["name"].as_str().map(|s|truncate_utf8(s,128))},"text":&text[offset..end],"text_range":{"start_byte":offset,"end_byte":end,"total_bytes":text.len(),"projection":PROJECTION},"snapshot":snapshot,"record_complete":false}],"truncated":end<text.len(),"next_cursor":(end<text.len()).then(||format!("p1:{binding}:{end}")),"pending_refs":[],"status":if end<text.len(){"partial"}else{"complete"},"budget_unit":"utf8_bytes"});
            let size = json_size(&result)?;
            if size <= args.budget_tokens && (end > offset || offset == text.len()) {
                return Ok(result);
            }
            if end == offset {
                return paging::budget_empty(reference, args.budget_tokens, size + 16);
            }
            end = end
                .saturating_sub(size.saturating_sub(args.budget_tokens).max(1))
                .max(offset);
        }
    }
}
