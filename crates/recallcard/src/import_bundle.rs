//! 有界、只读的导入解析。这里从不解压到磁盘，也不持有或写入 Vault。
//! GUI 仍须将文件身份、完整文件摘要、选中会话与预览令牌绑定，确认后才能写入。
use crate::{
    import::{chatgpt_source_id, parse_chatgpt_conversation, parse_text, ChatgptCoverage},
    model::{validate_scope, EventInput, Result, Role},
};
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, OpenOptions},
    io::{Cursor, Read},
    path::Path,
};
use zip::{CompressionMethod, ZipArchive};

pub const MAX_ARCHIVE_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_ARCHIVE_ENTRIES: usize = 2048;
pub const MAX_JSON_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_UNCOMPRESSED_BYTES: usize = 128 * 1024 * 1024;
pub const MAX_BATCH_EVENTS: usize = 5000;

#[derive(Debug, Default, Clone, Serialize, PartialEq, Eq)]
pub struct ImportCoverage {
    pub archive_entries: usize,
    pub directories_skipped: usize,
    pub json_files: usize,
    pub recognized_json_files: usize,
    pub invalid_json_files_skipped: usize,
    pub unrecognized_json_values_skipped: usize,
    pub markdown_files_skipped: usize,
    pub markdown_copies_skipped: usize,
    pub other_files_skipped: usize,
    pub uncompressed_bytes: usize,
    pub conversation_versions_seen: usize,
    pub conversations_available: usize,
    pub conversations_selected: usize,
    pub events_available: usize,
    pub events_selected: usize,
    pub duplicate_events_skipped: usize,
    pub messages: ChatgptCoverage,
    /// 固定说明供预览展示，不能把 ZIP 中未解析的内容说成已经导入。
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ConversationSummary {
    /// 来自 JSON 的原始 conversation id；UI 展示 title，用此字段提交选择。
    pub source_id: String,
    pub title: Option<String>,
    pub event_count: usize,
    pub user_messages: usize,
    pub assistant_messages: usize,
    pub tool_messages: usize,
    pub coverage: ChatgptCoverage,
}

#[derive(Debug, Clone, Serialize)]
pub struct ImportArchiveSummary {
    pub coverage: ImportCoverage,
    pub conversation_summaries: Vec<ConversationSummary>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ParsedImport {
    /// 保留每个会话 current_node 链的原始先后，不按时间或内容排序。
    pub events: Vec<EventInput>,
    pub coverage: ImportCoverage,
    pub conversation_summaries: Vec<ConversationSummary>,
}

/// 获取所有会话的可选列表；即使全备份超过单批 5000 条，也能先展示概要。
pub fn inspect_archive(bytes: &[u8], scope: &str) -> Result<ImportArchiveSummary> {
    let parsed = archive_impl(bytes, scope, None, false)?;
    Ok(ImportArchiveSummary {
        coverage: parsed.coverage,
        conversation_summaries: parsed.conversation_summaries,
    })
}

pub fn parse_archive(bytes: &[u8], scope: &str) -> Result<ParsedImport> {
    archive_impl(bytes, scope, None, true)
}

/// 空集合代表未选中任何会话；未知编号报错，不静默扩大或缩小用户选择。
pub fn parse_archive_selected(
    bytes: &[u8],
    scope: &str,
    source_ids: &BTreeSet<String>,
) -> Result<ParsedImport> {
    archive_impl(bytes, scope, Some(source_ids), true)
}

/// chatgpt-export 按内容自动识别 JSON 对象、官方数组或 ZIP；不按文件名猜平台。
pub fn parse_import_bytes(format: &str, bytes: &[u8], scope: &str) -> Result<ParsedImport> {
    validate_scope(scope)?;
    if format == "chatgpt-export" && is_zip(bytes) {
        return parse_archive(bytes, scope);
    }
    if bytes.len() > MAX_JSON_BYTES {
        return Err("单个导入文本上限 16 MiB；备份 ZIP 请使用 chatgpt-export 格式".into());
    }
    let text = std::str::from_utf8(bytes).map_err(|_| "导入文本不是有效 UTF-8")?;
    if format == "chatgpt-export" {
        let value: Value = serde_json::from_str(text).map_err(|_| "ChatGPT JSON 无效")?;
        let mut builder = ImportBuilder::new(scope, None, true);
        builder.coverage.json_files = 1;
        builder.coverage.uncompressed_bytes = bytes.len();
        if builder.json(&value)? {
            builder.coverage.recognized_json_files = 1;
        }
        return builder.finish();
    }
    let events = parse_text(format, text, scope)?;
    let coverage = ImportCoverage {
        events_available: events.len(),
        events_selected: events.len(),
        uncompressed_bytes: bytes.len(),
        ..ImportCoverage::default()
    };
    Ok(ParsedImport {
        events,
        coverage,
        conversation_summaries: vec![],
    })
}

/// 只读有界文件入口；调用方必须在确认时按原预览机制复核身份和整个文件摘要。
pub fn read_import_file(path: &Path, format: &str, scope: &str) -> Result<ParsedImport> {
    let before = fs::symlink_metadata(path).map_err(|_| "无法读取导入文件属性")?;
    if !before.is_file() || before.len() > MAX_ARCHIVE_BYTES as u64 {
        return Err("导入源必须是普通文件，不能是符号链接，且不超过 64 MiB".into());
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // 防止属性检查与打开之间被换成符号链接或会阻塞的 FIFO。
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let mut file = options.open(path).map_err(|_| "无法安全打开导入文件")?;
    let metadata = file.metadata().map_err(|_| "无法读取导入文件属性")?;
    if !metadata.is_file() || metadata.len() > MAX_ARCHIVE_BYTES as u64 {
        return Err("导入源必须是普通文件，且不超过 64 MiB".into());
    }
    let mut bytes = Vec::new();
    (&mut file)
        .take(MAX_ARCHIVE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "读取导入文件失败")?;
    if bytes.len() > MAX_ARCHIVE_BYTES {
        return Err("导入文件读取时超过 64 MiB 上限".into());
    }
    parse_import_bytes(format, &bytes, scope)
}

pub fn is_zip(bytes: &[u8]) -> bool {
    bytes.starts_with(b"PK\x03\x04") || bytes.starts_with(b"PK\x05\x06")
}

struct ImportBuilder<'a> {
    scope: &'a str,
    selected: Option<&'a BTreeSet<String>>,
    collect_events: bool,
    events: Vec<EventInput>,
    coverage: ImportCoverage,
    conversations: Vec<ConversationSummary>,
    conversation_index: BTreeMap<String, usize>,
    versions: BTreeSet<String>,
}

impl<'a> ImportBuilder<'a> {
    fn new(scope: &'a str, selected: Option<&'a BTreeSet<String>>, collect_events: bool) -> Self {
        Self {
            scope,
            selected,
            collect_events,
            events: vec![],
            coverage: ImportCoverage::default(),
            conversations: vec![],
            conversation_index: BTreeMap::new(),
            versions: BTreeSet::new(),
        }
    }

    fn json(&mut self, value: &Value) -> Result<bool> {
        let values: Vec<&Value> = match value {
            Value::Array(items) => items.iter().collect(),
            _ => vec![value],
        };
        let mut recognized = false;
        for conv in values {
            // 通过结构识别来源；像 ChatGPT 但损坏的记录必须报错，不能当普通附件跳过。
            if !conv.is_object()
                || (!conv.get("mapping").is_some_and(Value::is_object)
                    && conv.get("current_node").is_none())
            {
                self.coverage.unrecognized_json_values_skipped += 1;
                continue;
            }
            chatgpt_source_id(conv)?;
            let parsed = parse_chatgpt_conversation(conv, self.scope)?;
            recognized = true;
            self.coverage.conversation_versions_seen += 1;
            let selected = self
                .selected
                .is_none_or(|ids| ids.contains(&parsed.source_id));
            let index = *self
                .conversation_index
                .entry(parsed.source_id.clone())
                .or_insert_with(|| {
                    self.conversations.push(ConversationSummary {
                        source_id: parsed.source_id.clone(),
                        title: parsed.title.clone(),
                        event_count: 0,
                        user_messages: 0,
                        assistant_messages: 0,
                        tool_messages: 0,
                        coverage: ChatgptCoverage::default(),
                    });
                    self.conversations.len() - 1
                });
            add_coverage(&mut self.coverage.messages, &parsed.coverage);
            add_coverage(&mut self.conversations[index].coverage, &parsed.coverage);
            for event in parsed.events {
                if !self.versions.insert(event.id()?) {
                    self.coverage.duplicate_events_skipped += 1;
                    continue;
                }
                let summary = &mut self.conversations[index];
                summary.event_count += 1;
                match event.role {
                    Role::User => summary.user_messages += 1,
                    Role::Assistant => summary.assistant_messages += 1,
                    Role::Tool => summary.tool_messages += 1,
                    Role::System => {}
                }
                self.coverage.events_available += 1;
                if selected {
                    self.coverage.events_selected += 1;
                    // 超限仍完成概要计数，但不继续积累正文；finish 会明确报错。
                    if self.collect_events && self.coverage.events_selected <= MAX_BATCH_EVENTS {
                        self.events.push(event);
                    }
                }
            }
        }
        Ok(recognized)
    }

    fn finish(mut self) -> Result<ParsedImport> {
        if let Some(selected) = self.selected {
            if selected
                .iter()
                .any(|id| !self.conversation_index.contains_key(id))
            {
                return Err("选中的会话已不在本备份中；请重新预览后选择".into());
            }
        }
        self.coverage.conversations_available = self.conversations.len();
        self.coverage.conversations_selected = self
            .conversations
            .iter()
            .filter(|conv| {
                self.selected
                    .is_none_or(|ids| ids.contains(&conv.source_id))
            })
            .count();
        if self.collect_events && self.coverage.events_selected > MAX_BATCH_EVENTS {
            return Err(format!("本次选择有 {} 条事件；单批上限 5000。请在会话列表中减少选择后分批导入；单个超长会话需分批导出。没有写入任何事件。", self.coverage.events_selected));
        }
        self.coverage.notes.push("仅导入 current_node 选定分支的可见文本；隐藏推理不收集，附件原件不导入，工具和引用仅保留导出提供的部分信息。".into());
        if self.coverage.recognized_json_files == 0 {
            self.coverage.notes.push("文件中没有可识别的 ChatGPT 会话 JSON；Markdown 和其他文件已统计为跳过，没有导入消息。".into());
        }
        if self.coverage.invalid_json_files_skipped > 0
            || self.coverage.unrecognized_json_values_skipped > 0
        {
            self.coverage
                .notes
                .push("存在损坏或不支持的 JSON，已明确计入跳过统计；请核对覆盖范围。".into());
        }
        if self.coverage.events_selected > MAX_BATCH_EVENTS {
            self.coverage
                .notes
                .push("全备份超过单批 5000 条上限；可先在列表中选择会话分批导入。".into());
        }
        Ok(ParsedImport {
            events: self.events,
            coverage: self.coverage,
            conversation_summaries: self.conversations,
        })
    }
}

fn add_coverage(target: &mut ChatgptCoverage, source: &ChatgptCoverage) {
    target.selected_branch_messages += source.selected_branch_messages;
    target.other_branch_messages_skipped += source.other_branch_messages_skipped;
    target.hidden_reasoning_messages_skipped += source.hidden_reasoning_messages_skipped;
    target.unsupported_messages_skipped += source.unsupported_messages_skipped;
    target.empty_messages_skipped += source.empty_messages_skipped;
    target.unsupported_content_parts_skipped += source.unsupported_content_parts_skipped;
}

fn archive_impl(
    bytes: &[u8],
    scope: &str,
    selected: Option<&BTreeSet<String>>,
    collect: bool,
) -> Result<ParsedImport> {
    validate_scope(scope)?;
    let entries = audit_directory(bytes)?;
    let mut archive = ZipArchive::new(Cursor::new(bytes)).map_err(|_| "ZIP 结构无效或不受支持")?;
    if archive.len() != entries.len() {
        return Err("ZIP 包含重复或冲突条目".into());
    }
    let mut builder = ImportBuilder::new(scope, selected, collect);
    builder.coverage.archive_entries = entries.len();
    let mut names = BTreeMap::new();
    let mut json_names = BTreeSet::new();
    let mut markdown_names = Vec::new();
    let mut total = 0usize;
    for (index, entry) in entries.iter().enumerate() {
        let mut file = archive
            .by_index(index)
            .map_err(|_| "ZIP 条目无法读取（可能加密、损坏或压缩方式不受支持）")?;
        if file.encrypted() || file.is_symlink() {
            return Err("ZIP 不允许加密条目或符号链接".into());
        }
        if !matches!(
            file.compression(),
            CompressionMethod::Stored | CompressionMethod::Deflated
        ) {
            return Err("ZIP 仅支持 Stored 和 Deflate 压缩方式".into());
        }
        if file.name_raw() != entry.name.as_bytes() || file.size() != entry.size as u64 {
            return Err("ZIP 条目名称或大小存在冲突".into());
        }
        validate_name(file.name(), file.is_dir(), &mut names)?;
        let name = file.name().to_lowercase();
        let is_json = !file.is_dir() && name.ends_with(".json");
        if is_json && file.size() > MAX_JSON_BYTES as u64 {
            return Err("ZIP 中单个 JSON 超过 16 MiB；请分批导出".into());
        }
        if file.size() > MAX_UNCOMPRESSED_BYTES as u64 {
            return Err("ZIP 条目展开大小超过 128 MiB".into());
        }
        let mut data = Vec::new();
        let mut actual = 0usize;
        let mut buffer = [0u8; 8192];
        loop {
            let count = file
                .read(&mut buffer)
                .map_err(|_| "ZIP 条目解压或完整性校验失败")?;
            if count == 0 {
                break;
            }
            actual = actual.checked_add(count).ok_or("ZIP 展开大小溢出")?;
            total = total.checked_add(count).ok_or("ZIP 总展开大小溢出")?;
            if total > MAX_UNCOMPRESSED_BYTES {
                return Err("ZIP 实际总展开大小超过 128 MiB".into());
            }
            if is_json {
                if actual > MAX_JSON_BYTES {
                    return Err("ZIP 中单个 JSON 实际展开超过 16 MiB".into());
                }
                data.extend_from_slice(&buffer[..count]);
            }
        }
        if actual != entry.size {
            return Err("ZIP 实际展开大小与声明不一致".into());
        }
        if file.is_dir() {
            builder.coverage.directories_skipped += 1;
        } else if is_json {
            builder.coverage.json_files += 1;
            match serde_json::from_slice::<Value>(&data) {
                Ok(value) if builder.json(&value)? => {
                    builder.coverage.recognized_json_files += 1;
                    json_names.insert(name.trim_end_matches(".json").to_owned());
                }
                Ok(_) => {}
                Err(_) => builder.coverage.invalid_json_files_skipped += 1,
            }
        } else if name.ends_with(".md") || name.ends_with(".markdown") {
            builder.coverage.markdown_files_skipped += 1;
            markdown_names.push(
                name.rsplit_once('.')
                    .map(|(stem, _)| stem.to_owned())
                    .unwrap_or(name),
            );
        } else {
            builder.coverage.other_files_skipped += 1;
        }
    }
    builder.coverage.uncompressed_bytes = total;
    builder.coverage.markdown_copies_skipped = markdown_names
        .iter()
        .filter(|name| json_names.contains(*name))
        .count();
    builder.finish()
}

struct DirectoryEntry {
    name: String,
    size: usize,
}

/// 在 zip crate 分配中央目录前计数并检查原始记录。该库会按名称合并重复项，
/// 因此不能只在 ZipArchive::len() 后检查。限于单卷 ZIP32；64 MiB 输入无需 ZIP64。
fn audit_directory(bytes: &[u8]) -> Result<Vec<DirectoryEntry>> {
    if bytes.len() > MAX_ARCHIVE_BYTES {
        return Err("ZIP 文件超过 64 MiB 上限".into());
    }
    if bytes.len() < 22 {
        return Err("ZIP 文件不完整".into());
    }
    let footer = (bytes.len().saturating_sub(65557)..=bytes.len() - 22)
        .rev()
        .find(|&offset| {
            bytes[offset..].starts_with(b"PK\x05\x06")
                && offset + 22 + u16_at(bytes, offset + 20).unwrap_or(0) as usize == bytes.len()
        })
        .ok_or("找不到完整 ZIP 目录尾部")?;
    let count = u16_at(bytes, footer + 10)? as usize;
    if u16_at(bytes, footer + 4)? != 0
        || u16_at(bytes, footer + 6)? != 0
        || u16_at(bytes, footer + 8)? as usize != count
        || count == 65535
    {
        return Err("暂不支持分卷或 ZIP64 备份；请使用普通 ZIP".into());
    }
    if count > MAX_ARCHIVE_ENTRIES {
        return Err("ZIP 条目数超过 2048 上限".into());
    }
    let directory_size = u32_at(bytes, footer + 12)? as usize;
    let mut cursor = u32_at(bytes, footer + 16)? as usize;
    if cursor.checked_add(directory_size) != Some(footer) {
        return Err("ZIP 中央目录位置不一致或使用了不支持的 ZIP64".into());
    }
    let directory_start = cursor;
    let mut entries = Vec::with_capacity(count);
    let mut names = BTreeMap::new();
    let mut ranges = Vec::new();
    let mut declared_total = 0usize;
    for _ in 0..count {
        if bytes.get(cursor..cursor + 4) != Some(b"PK\x01\x02") {
            return Err("ZIP 中央目录条目无效".into());
        }
        let flags = u16_at(bytes, cursor + 8)?;
        let method = u16_at(bytes, cursor + 10)?;
        if flags & (1 | 64 | 8192) != 0 {
            return Err("ZIP 不允许加密条目".into());
        }
        if ![0, 8].contains(&method) {
            return Err("ZIP 仅支持 Stored 和 Deflate 压缩方式".into());
        }
        let compressed = u32_at(bytes, cursor + 20)? as usize;
        let size = u32_at(bytes, cursor + 24)? as usize;
        let name_length = u16_at(bytes, cursor + 28)? as usize;
        let extra_length = u16_at(bytes, cursor + 30)? as usize;
        let comment_length = u16_at(bytes, cursor + 32)? as usize;
        if u16_at(bytes, cursor + 34)? != 0 {
            return Err("ZIP 不允许分卷条目".into());
        }
        let attrs = u32_at(bytes, cursor + 38)?;
        let mode = (attrs >> 16) & 0o170000;
        if ![0, 0o100000, 0o040000].contains(&mode) {
            return Err("ZIP 不允许符号链接或特殊文件".into());
        }
        let local = u32_at(bytes, cursor + 42)? as usize;
        let end = cursor
            .checked_add(46 + name_length + extra_length + comment_length)
            .ok_or("ZIP 目录长度溢出")?;
        if end > footer {
            return Err("ZIP 中央目录条目越界".into());
        }
        let name_bytes = bytes
            .get(cursor + 46..cursor + 46 + name_length)
            .ok_or("ZIP 文件名越界")?;
        let name = std::str::from_utf8(name_bytes).map_err(|_| "ZIP 文件名必须使用 UTF-8")?;
        let directory = name.ends_with('/');
        validate_name(name, directory, &mut names)?;
        if (mode == 0o040000 && !directory) || (mode == 0o100000 && directory) {
            return Err("ZIP 文件名和目录类型冲突".into());
        }
        if directory && size != 0 {
            return Err("ZIP 目录条目不能携带文件内容".into());
        }
        if name.to_lowercase().ends_with(".json") && size > MAX_JSON_BYTES {
            return Err("ZIP 中单个 JSON 超过 16 MiB；请分批导出".into());
        }
        declared_total = declared_total.checked_add(size).ok_or("ZIP 声明大小溢出")?;
        if declared_total > MAX_UNCOMPRESSED_BYTES {
            return Err("ZIP 声明的总展开大小超过 128 MiB".into());
        }
        if bytes.get(local..local + 4) != Some(b"PK\x03\x04")
            || u16_at(bytes, local + 6)? != flags
            || u16_at(bytes, local + 8)? != method
        {
            return Err("ZIP 本地条目与中央目录冲突".into());
        }
        let local_name_length = u16_at(bytes, local + 26)? as usize;
        let local_extra_length = u16_at(bytes, local + 28)? as usize;
        if bytes.get(local + 30..local + 30 + local_name_length) != Some(name_bytes) {
            return Err("ZIP 本地文件名与中央目录冲突".into());
        }
        let data_start = local
            .checked_add(30 + local_name_length + local_extra_length)
            .ok_or("ZIP 数据位置溢出")?;
        let data_end = data_start
            .checked_add(compressed)
            .ok_or("ZIP 数据长度溢出")?;
        if data_end > directory_start {
            return Err("ZIP 文件数据越界".into());
        }
        ranges.push((local, data_end));
        entries.push(DirectoryEntry {
            name: name.to_owned(),
            size,
        });
        cursor = end;
    }
    if cursor != footer {
        return Err("ZIP 中央目录数量或长度不一致".into());
    }
    ranges.sort_unstable();
    if ranges.windows(2).any(|pair| pair[0].1 > pair[1].0) {
        return Err("ZIP 条目数据互相覆盖".into());
    }
    Ok(entries)
}

fn validate_name(name: &str, directory: bool, names: &mut BTreeMap<String, bool>) -> Result<()> {
    if name.is_empty()
        || name.starts_with('/')
        || name.ends_with("//")
        || name.contains(['\\', ':', '\0'])
    {
        return Err("ZIP 包含不安全的绝对路径或文件名".into());
    }
    let normalized = name.trim_end_matches('/').to_lowercase();
    if normalized
        .split('/')
        .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err("ZIP 路径不允许空段、. 或 ..".into());
    }
    if names.contains_key(&normalized) {
        return Err("ZIP 包含重复或冲突同名条目".into());
    }
    for (path, is_directory) in names.iter() {
        if (!is_directory && normalized.starts_with(&format!("{path}/")))
            || (!directory && path.starts_with(&format!("{normalized}/")))
        {
            return Err("ZIP 文件和目录路径冲突".into());
        }
    }
    names.insert(normalized, directory);
    Ok(())
}

fn u16_at(bytes: &[u8], offset: usize) -> Result<u16> {
    let value = bytes.get(offset..offset + 2).ok_or("ZIP 头部截断")?;
    Ok(u16::from_le_bytes([value[0], value[1]]))
}
fn u32_at(bytes: &[u8], offset: usize) -> Result<u32> {
    let value = bytes.get(offset..offset + 4).ok_or("ZIP 头部截断")?;
    Ok(u32::from_le_bytes([value[0], value[1], value[2], value[3]]))
}
