//! 有界、只读的导入解析。这里从不解压到磁盘，也不持有或写入 Vault。
//! GUI 将全部源文件身份、完整摘要、选中会话与一个导入任务绑定。
use crate::{
    import::{parse_chatgpt_conversation, parse_text, ChatgptCoverage},
    import_deepseek::{looks_like_deepseek, parse_deepseek_conversation, DeepseekCoverage},
    model::{validate_scope, EventInput, Result, Role},
};
use serde::{Deserialize, Serialize};
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
/// 旧版调用方的批量建议值；官方导出不再强制用户每 5000 条拆分。
pub const MAX_BATCH_EVENTS: usize = 5000;
pub const MAX_IMPORT_EVENTS: usize = 100_000;
pub const MAX_IMPORT_FILES: usize = 32;
pub const MAX_IMPORT_BYTES: usize = 128 * 1024 * 1024;
pub const MAX_IMPORT_UNCOMPRESSED_BYTES: usize = 256 * 1024 * 1024;

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
    pub deepseek: DeepseekCoverage,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ConversationSummary {
    /// 原始平台会话编号，不来自文件名。
    pub source_id: String,
    /// 平台限定的稳定选择键；与清单令牌 selection_id 不同。
    pub selection_key: String,
    pub platform: String,
    pub title: Option<String>,
    pub event_count: usize,
    pub user_messages: usize,
    pub assistant_messages: usize,
    pub tool_messages: usize,
    pub coverage: ChatgptCoverage,
    pub deepseek_coverage: DeepseekCoverage,
}

#[derive(Debug, Clone, Serialize)]
pub struct ImportArchiveSummary {
    pub coverage: ImportCoverage,
    pub conversation_summaries: Vec<ConversationSummary>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ParsedImport {
    /// ChatGPT 保留 current_node 链；DeepSeek 保留全部节点的拓扑排放，不宣称时间顺序。
    pub events: Vec<EventInput>,
    pub coverage: ImportCoverage,
    pub conversation_summaries: Vec<ConversationSummary>,
}

pub fn inspect_archive(bytes: &[u8], scope: &str) -> Result<ImportArchiveSummary> {
    check_archive_input(bytes)?;
    inspect_import_bytes("auto", bytes, scope)
}
pub fn parse_archive(bytes: &[u8], scope: &str) -> Result<ParsedImport> {
    check_archive_input(bytes)?;
    parse_import_bytes("auto", bytes, scope)
}
pub fn parse_archive_selected(
    bytes: &[u8],
    scope: &str,
    ids: &BTreeSet<String>,
) -> Result<ParsedImport> {
    check_archive_input(bytes)?;
    parse_import_bytes_selected("auto", bytes, scope, ids)
}
fn check_archive_input(bytes: &[u8]) -> Result<()> {
    if bytes.len() > MAX_ARCHIVE_BYTES {
        return Err("ZIP 文件超过 64 MiB 上限".into());
    }
    if !is_zip(bytes) {
        return Err("ZIP 结构无效或不受支持".into());
    }
    Ok(())
}
pub fn inspect_import_bytes(
    format: &str,
    bytes: &[u8],
    scope: &str,
) -> Result<ImportArchiveSummary> {
    inspect_import_files(format, &[bytes], scope)
}
pub fn inspect_import_files(
    format: &str,
    files: &[&[u8]],
    scope: &str,
) -> Result<ImportArchiveSummary> {
    let parsed = import_files(format, files, scope, None, false)?;
    Ok(ImportArchiveSummary {
        coverage: parsed.coverage,
        conversation_summaries: parsed.conversation_summaries,
    })
}
pub fn parse_import_bytes(format: &str, bytes: &[u8], scope: &str) -> Result<ParsedImport> {
    import_files(format, &[bytes], scope, None, true)
}
pub fn parse_import_bytes_selected(
    format: &str,
    bytes: &[u8],
    scope: &str,
    ids: &BTreeSet<String>,
) -> Result<ParsedImport> {
    parse_import_files_selected(format, &[bytes], scope, ids)
}
/// 空集合代表不导入，绝不自动变成全选。先核对整份任务，再统一去重。
pub fn parse_import_files_selected(
    format: &str,
    files: &[&[u8]],
    scope: &str,
    ids: &BTreeSet<String>,
) -> Result<ParsedImport> {
    import_files(format, files, scope, Some(ids), true)
}

fn import_files(
    format: &str,
    files: &[&[u8]],
    scope: &str,
    selected: Option<&BTreeSet<String>>,
    collect: bool,
) -> Result<ParsedImport> {
    validate_scope(scope)?;
    if ![
        "auto",
        "chatgpt-export",
        "deepseek-export",
        "manual-jsonl",
        "claude-code",
        "recallcard-conversation",
    ]
    .contains(&format)
    {
        return Err("不支持的导入格式".into());
    }
    if files.is_empty() || files.len() > MAX_IMPORT_FILES {
        return Err("一次导入需选择 1–32 个文件".into());
    }
    let total = files.iter().try_fold(0usize, |sum, bytes| {
        sum.checked_add(bytes.len()).ok_or("导入文件总大小溢出")
    })?;
    if total > MAX_IMPORT_BYTES {
        return Err("本次导入文件总大小超过 128 MiB".into());
    }
    let mut builder = ImportBuilder::new(scope, selected, collect, format);
    for bytes in files {
        if is_zip(bytes) {
            if !["auto", "chatgpt-export", "deepseek-export"].contains(&format) {
                return Err("ZIP 备份请选择 DeepSeek、ChatGPT 或自动识别格式".into());
            }
            append_archive(bytes, &mut builder)?;
            continue;
        }
        if bytes.len() > MAX_JSON_BYTES {
            return Err("单个导入文本上限 16 MiB；不会截断内容".into());
        }
        builder.add_bytes(bytes.len())?;
        let text = std::str::from_utf8(bytes).map_err(|_| "导入文本不是有效 UTF-8")?;
        let detected = if format == "auto" {
            match detect_import_format(text) {
                Ok(detected) => detected,
                Err(error) => {
                    // 独立账号/设置 JSON 和 ZIP 中同类 JSON 一样，只统计未识别内容。
                    // JSONL 仍按其独立适配器识别；损坏或重复键的 JSON 不猜测来源。
                    if let Ok(value) = strict_json(bytes) {
                        builder.coverage.json_files += 1;
                        if builder.json(&value)? {
                            builder.coverage.recognized_json_files += 1;
                        }
                        continue;
                    }
                    return Err(error);
                }
            }
        } else {
            format.into()
        };
        if ["auto", "chatgpt-export", "deepseek-export"].contains(&detected.as_str()) {
            let value = strict_json(bytes)?;
            builder.coverage.json_files += 1;
            if builder.json(&value)? {
                builder.coverage.recognized_json_files += 1;
            }
        } else {
            let events = parse_text(&detected, text, scope)?;
            let mut groups: BTreeMap<(String, String), Vec<EventInput>> = BTreeMap::new();
            for event in events {
                groups
                    .entry((
                        event.source.platform.clone(),
                        event.source.conversation_id.clone(),
                    ))
                    .or_default()
                    .push(event);
            }
            for ((platform, source_id), events) in groups {
                let title = events
                    .first()
                    .and_then(|e| e.metadata["conversation_title"].as_str())
                    .map(str::to_owned);
                builder.add_conversation(
                    &platform,
                    source_id,
                    title,
                    events,
                    ChatgptCoverage::default(),
                    DeepseekCoverage::default(),
                )?;
            }
        }
    }
    builder.finish()
}

/// 结构判别来源，两个平台的 mapping 不能互相冒充。
fn provider(value: &Value) -> Result<Option<&'static str>> {
    let deepseek = looks_like_deepseek(value);
    let chatgpt = value.get("current_node").is_some()
        || value["mapping"]
            .as_object()
            .is_some_and(|nodes| nodes.values().any(|n| n["message"].get("author").is_some()));
    match (deepseek, chatgpt) {
        (true, true) => Err("会话同时包含 DeepSeek 与 ChatGPT 特征，来源有歧义".into()),
        (true, false) => Ok(Some("deepseek-export")),
        (false, true) => Ok(Some("chatgpt-export")),
        (false, false) => Ok(None),
    }
}

pub fn detect_import_format(text: &str) -> Result<String> {
    let parsed = strict_json(text.as_bytes());
    if let Err(error) = &parsed {
        if error.contains("重复字段") {
            return Err(error.clone());
        }
    }
    if let Ok(value) = parsed {
        if value["schema"] == "recallcard.conversation/1" {
            return Ok("recallcard-conversation".into());
        }
        let values: Vec<&Value> = value
            .as_array()
            .map(|a| a.iter().collect())
            .unwrap_or_else(|| vec![&value]);
        let mut found = BTreeSet::new();
        for value in values {
            if let Some(platform) = provider(value)? {
                found.insert(platform);
            }
        }
        if found.len() == 1 {
            return Ok(found.into_iter().next().unwrap().into());
        }
        if found.len() > 1 {
            return Ok("auto".into());
        }
    }
    if let Some(first) = text
        .lines()
        .find(|line| !line.trim().is_empty())
        .and_then(|line| serde_json::from_str::<Value>(line).ok())
    {
        if first["sessionId"].is_string() && first["type"].is_string() {
            return Ok("claude-code".into());
        }
        if first["source"].is_object() && first["role"].is_string() {
            return Ok("manual-jsonl".into());
        }
    }
    Err("未识别出支持的会话结构；请选择官方 DeepSeek / ChatGPT 导出、RecallCard 会话 JSON 或 Claude Code JSONL".into())
}

/// 只读有界文件入口；调用方确认时还必须核对身份及整文件摘要。
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
pub fn selection_key(platform: &str, source_id: &str) -> String {
    format!("{platform}:{source_id}")
}

struct ImportBuilder<'a> {
    scope: &'a str,
    selected: Option<&'a BTreeSet<String>>,
    collect_events: bool,
    format: &'a str,
    events: Vec<EventInput>,
    coverage: ImportCoverage,
    conversations: Vec<ConversationSummary>,
    conversation_index: BTreeMap<String, usize>,
    versions: BTreeSet<String>,
}
impl<'a> ImportBuilder<'a> {
    fn new(
        scope: &'a str,
        selected: Option<&'a BTreeSet<String>>,
        collect_events: bool,
        format: &'a str,
    ) -> Self {
        Self {
            scope,
            selected,
            collect_events,
            format,
            events: vec![],
            coverage: ImportCoverage::default(),
            conversations: vec![],
            conversation_index: BTreeMap::new(),
            versions: BTreeSet::new(),
        }
    }
    fn add_bytes(&mut self, count: usize) -> Result<()> {
        self.coverage.uncompressed_bytes = self
            .coverage
            .uncompressed_bytes
            .checked_add(count)
            .ok_or("导入展开大小溢出")?;
        if self.coverage.uncompressed_bytes > MAX_IMPORT_UNCOMPRESSED_BYTES {
            return Err("本次导入实际展开总大小超过 256 MiB".into());
        }
        Ok(())
    }
    fn json(&mut self, value: &Value) -> Result<bool> {
        let values: Vec<&Value> = value
            .as_array()
            .map(|a| a.iter().collect())
            .unwrap_or_else(|| vec![value]);
        let mut recognized = false;
        for conv in values {
            let Some(platform) = provider(conv)? else {
                self.coverage.unrecognized_json_values_skipped += 1;
                continue;
            };
            if self.format != "auto" && self.format != platform {
                return Err("文件中的平台与所选导入格式不一致；请使用对应平台或自动识别".into());
            }
            recognized = true;
            if platform == "deepseek-export" {
                let parsed = parse_deepseek_conversation(conv, self.scope)?;
                self.add_conversation(
                    "deepseek",
                    parsed.source_id,
                    parsed.title,
                    parsed.events,
                    ChatgptCoverage::default(),
                    parsed.coverage,
                )?;
            } else {
                let parsed = parse_chatgpt_conversation(conv, self.scope)?;
                self.add_conversation(
                    "chatgpt-export",
                    parsed.source_id,
                    parsed.title,
                    parsed.events,
                    parsed.coverage,
                    DeepseekCoverage::default(),
                )?;
            }
        }
        Ok(recognized)
    }
    fn add_conversation(
        &mut self,
        platform: &str,
        source_id: String,
        title: Option<String>,
        events: Vec<EventInput>,
        coverage: ChatgptCoverage,
        deepseek_coverage: DeepseekCoverage,
    ) -> Result<()> {
        self.coverage.conversation_versions_seen += 1;
        let key = selection_key(platform, &source_id);
        let index = *self
            .conversation_index
            .entry(key.clone())
            .or_insert_with(|| {
                self.conversations.push(ConversationSummary {
                    source_id,
                    selection_key: key,
                    platform: platform.into(),
                    title,
                    event_count: 0,
                    user_messages: 0,
                    assistant_messages: 0,
                    tool_messages: 0,
                    coverage: ChatgptCoverage::default(),
                    deepseek_coverage: DeepseekCoverage::default(),
                });
                self.conversations.len() - 1
            });
        add_coverage(&mut self.coverage.messages, &coverage);
        add_coverage(&mut self.conversations[index].coverage, &coverage);
        self.coverage.deepseek.add(&deepseek_coverage);
        self.conversations[index]
            .deepseek_coverage
            .add(&deepseek_coverage);
        for event in events {
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
            if self.coverage.events_available > MAX_IMPORT_EVENTS {
                return Err("导入任务超过 100000 条事件安全上限；没有截断或写入事件".into());
            }
            if self.collect_events {
                self.events.push(event);
            }
        }
        Ok(())
    }
    fn finish(mut self) -> Result<ParsedImport> {
        let selected = if let Some(ids) = self.selected {
            let mut keys = BTreeSet::new();
            for id in ids {
                if self.conversation_index.contains_key(id) {
                    keys.insert(id.clone());
                    continue;
                }
                let matches: Vec<_> = self
                    .conversations
                    .iter()
                    .filter(|c| &c.source_id == id)
                    .collect();
                if matches.len() != 1 {
                    return Err(
                        "选中的会话已不在本备份中或编号跨平台重名；请重新预览并使用平台限定编号"
                            .into(),
                    );
                }
                keys.insert(matches[0].selection_key.clone());
            }
            keys
        } else {
            self.conversation_index.keys().cloned().collect()
        };
        self.coverage.conversations_available = self.conversations.len();
        self.coverage.conversations_selected = selected.len();
        self.coverage.events_selected = self
            .conversations
            .iter()
            .filter(|c| selected.contains(&c.selection_key))
            .map(|c| c.event_count)
            .sum();
        self.events.retain(|event| {
            selected.contains(&selection_key(
                &event.source.platform,
                &event.source.conversation_id,
            ))
        });
        if self
            .conversations
            .iter()
            .any(|c| c.platform == "chatgpt-export")
        {
            self.coverage.notes.push(
                "ChatGPT 仅导入 current_node 分支的可见文本；隐藏推理和附件原件不导入。".into(),
            );
        }
        if self.conversations.iter().any(|c| c.platform == "deepseek") {
            self.coverage.notes.push("DeepSeek 保留导出树全部可识别的可见消息节点及父子编号；兄弟分支不是连续对话，也无法判断网站当前选中分支。THINK、工具/搜索片段、附件内容和附件链接均不导入；混合角色和不支持的消息按项统计跳过。只覆盖所提供文件，不表示完整账号历史。".into());
        }
        if self.conversations.is_empty() {
            self.coverage.notes.push("文件中没有可识别的会话 JSON；Markdown、账号信息及其他未支持内容仅统计为跳过，没有导入消息。".into());
        }
        if self.coverage.invalid_json_files_skipped > 0
            || self.coverage.unrecognized_json_values_skipped > 0
        {
            self.coverage
                .notes
                .push("存在损坏或不支持的 JSON，已明确计入跳过统计；请核对覆盖范围。".into());
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

fn append_archive(bytes: &[u8], builder: &mut ImportBuilder<'_>) -> Result<()> {
    let entries = audit_directory(bytes)?;
    let mut archive = ZipArchive::new(Cursor::new(bytes)).map_err(|_| "ZIP 结构无效或不受支持")?;
    if archive.len() != entries.len() {
        return Err("ZIP 包含重复或冲突条目".into());
    }
    builder.coverage.archive_entries += entries.len();
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
            return Err("ZIP 中单个 JSON 超过 16 MiB；不会截断导入".into());
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
            builder.add_bytes(count)?;
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
            match strict_json(&data) {
                Ok(value) if builder.json(&value)? => {
                    builder.coverage.recognized_json_files += 1;
                    json_names.insert(name.trim_end_matches(".json").to_owned());
                }
                Ok(_) => {}
                Err(error) if error.contains("重复字段") => return Err(error),
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
    builder.coverage.markdown_copies_skipped += markdown_names
        .iter()
        .filter(|name| json_names.contains(*name))
        .count();
    Ok(())
}

/// serde_json::Value 默认覆盖重复对象键；导入身份不能依赖该行为。
struct UniqueValue(Value);
impl<'de> Deserialize<'de> for UniqueValue {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = UniqueValue;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("无重复字段的 JSON")
            }
            fn visit_bool<E: serde::de::Error>(
                self,
                v: bool,
            ) -> std::result::Result<Self::Value, E> {
                Ok(UniqueValue(v.into()))
            }
            fn visit_i64<E: serde::de::Error>(self, v: i64) -> std::result::Result<Self::Value, E> {
                Ok(UniqueValue(v.into()))
            }
            fn visit_u64<E: serde::de::Error>(self, v: u64) -> std::result::Result<Self::Value, E> {
                Ok(UniqueValue(v.into()))
            }
            fn visit_f64<E: serde::de::Error>(self, v: f64) -> std::result::Result<Self::Value, E> {
                Ok(UniqueValue(Value::Number(
                    serde_json::Number::from_f64(v).ok_or_else(|| E::custom("无效 JSON 数字"))?,
                )))
            }
            fn visit_str<E: serde::de::Error>(
                self,
                v: &str,
            ) -> std::result::Result<Self::Value, E> {
                Ok(UniqueValue(v.into()))
            }
            fn visit_string<E: serde::de::Error>(
                self,
                v: String,
            ) -> std::result::Result<Self::Value, E> {
                Ok(UniqueValue(v.into()))
            }
            fn visit_unit<E: serde::de::Error>(self) -> std::result::Result<Self::Value, E> {
                Ok(UniqueValue(Value::Null))
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> std::result::Result<Self::Value, A::Error> {
                let mut array = Vec::new();
                while let Some(value) = seq.next_element::<UniqueValue>()? {
                    array.push(value.0);
                }
                Ok(UniqueValue(Value::Array(array)))
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> std::result::Result<Self::Value, A::Error> {
                let mut object = serde_json::Map::new();
                while let Some(key) = map.next_key::<String>()? {
                    if object.contains_key(&key) {
                        return Err(serde::de::Error::custom(
                            "JSON 包含重复字段；无法可靠核对节点身份",
                        ));
                    }
                    object.insert(key, map.next_value::<UniqueValue>()?.0);
                }
                Ok(UniqueValue(Value::Object(object)))
            }
        }
        deserializer.deserialize_any(Visitor)
    }
}
fn strict_json(bytes: &[u8]) -> Result<Value> {
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let value =
        UniqueValue::deserialize(&mut deserializer).map_err(|e| format!("导出 JSON 无效：{e}"))?;
    deserializer
        .end()
        .map_err(|_| "导出 JSON 尾部存在多余内容")?;
    Ok(value.0)
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
