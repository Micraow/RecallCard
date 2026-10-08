//! 官方导出增量读取：一次只持有一个有界会话，不建立整份 JSON 树。
//! sink 只能写预检暂存；成员 CRC 和整包校验完成前不允许写入事实源。
use super::{AppError, AppResult, ErrorCode, ImportLimits};
use crate::{
    import::{parse_chatgpt_conversation_all, ChatgptCoverage},
    import_bundle,
    import_deepseek::{parse_deepseek_conversation, DeepseekCoverage},
    EventInput,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io::{self, BufRead, BufReader, Read, Seek, SeekFrom},
};
use zip::{CompressionMethod, ZipArchive};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReadReport {
    pub expanded_bytes: u64,
    pub archive_entries: u64,
    pub conversations: u64,
    pub events: u64,
    pub ignored_values: u64,
    pub ignored_files: u64,
    pub hidden_fragments: u64,
    pub unsupported_fragments: u64,
    pub omitted_messages: u64,
    pub file_references: u64,
    pub citations: u64,
    pub trace_placeholders: u64,
}

pub struct ImportedConversation {
    pub platform: String,
    pub source_id: String,
    pub title: Option<String>,
    pub events: Vec<EventInput>,
    pub chatgpt_coverage: ChatgptCoverage,
    pub deepseek_coverage: DeepseekCoverage,
}

fn invalid_archive(message: &str) -> AppError {
    AppError::new(
        ErrorCode::InvalidArchive,
        message,
        "请重新下载完整官方导出，原始资料未被修改",
    )
}
fn resource(message: &str) -> AppError {
    AppError::new(
        ErrorCode::ResourceLimit,
        message,
        "查看导入资源限制与受影响成员；不会截断数据",
    )
}
fn invalid_json() -> AppError {
    AppError::new(
        ErrorCode::InvalidJson,
        "JSON 不完整、存在重复字段或嵌套过深",
        "请重新取得完整导出；不要手工删减内容",
    )
}
fn io_error(_: io::Error) -> AppError {
    invalid_archive("导出读取、解压或完整性校验失败")
}

/// probe 既用于报告实际读入字节，也允许调用方按持久暂停标记停止解析。
/// 返回错误时此前 sink 数据只是未提交暂存，不能当作成功导入。
pub fn read_source<R, S, P>(
    reader: &mut R,
    format: &str,
    scope: &str,
    limits: ImportLimits,
    mut sink: S,
    mut probe: P,
) -> AppResult<ReadReport>
where
    R: Read + Seek,
    S: FnMut(ImportedConversation) -> AppResult<()>,
    P: FnMut(&ReadReport) -> AppResult<()>,
{
    crate::validate_scope(scope).map_err(|_| {
        AppError::new(
            ErrorCode::InvalidRequest,
            "资料范围无效",
            "请选择有效资料范围",
        )
    })?;
    if !matches!(format, "auto" | "deepseek-export" | "chatgpt-export") {
        return Err(AppError::new(
            ErrorCode::UnsupportedFormat,
            "此导入服务目前支持 DeepSeek 与 ChatGPT 官方 JSON / ZIP",
            "选择官方导出；其他旧格式仍可使用兼容命令",
        ));
    }
    let length = reader.seek(SeekFrom::End(0)).map_err(io_error)?;
    if length > limits.source_bytes {
        return Err(resource("导出文件超过本次配置的输入字节上限"));
    }
    reader.seek(SeekFrom::Start(0)).map_err(io_error)?;
    let mut magic = [0; 4];
    let count = reader.read(&mut magic).map_err(io_error)?;
    reader.seek(SeekFrom::Start(0)).map_err(io_error)?;
    let mut report = ReadReport::default();
    if count == 4 && (magic == *b"PK\x03\x04" || magic == *b"PK\x05\x06") {
        let directory = audit_zip(reader, limits)?;
        reader.seek(SeekFrom::Start(0)).map_err(io_error)?;
        let mut archive =
            ZipArchive::new(reader).map_err(|_| invalid_archive("ZIP 结构无效或不受支持"))?;
        if archive.len() != directory.len() {
            return Err(invalid_archive("ZIP 含重复或冲突条目"));
        }
        report.archive_entries = directory.len() as u64;
        for (i, entry) in directory.iter().enumerate() {
            probe(&report)?;
            let mut file = archive
                .by_index(i)
                .map_err(|_| invalid_archive("ZIP 成员无法读取"))?;
            if file.name() != entry.name
                || file.size() != entry.size
                || file.encrypted()
                || file.is_symlink()
                || !matches!(
                    file.compression(),
                    CompressionMethod::Stored | CompressionMethod::Deflated
                )
            {
                return Err(invalid_archive("ZIP 成员属性与已核验目录不一致"));
            }
            let member = entry.name.clone();
            let before = report.expanded_bytes;
            let result = if !file.is_dir() && entry.name.to_ascii_lowercase().ends_with(".json") {
                read_json_stream(
                    &mut file,
                    format,
                    scope,
                    limits,
                    &mut report,
                    &mut sink,
                    &mut probe,
                )
            } else {
                if !file.is_dir() {
                    report.ignored_files += 1;
                }
                drain(&mut file, limits, &mut report, &mut probe)
            };
            result.map_err(|mut e| {
                e.member = Some(member);
                e
            })?;
            if report.expanded_bytes - before != entry.size {
                return Err(invalid_archive("ZIP 实际展开字节数与声明不一致")
                    .at(None, Some(entry.name.clone())));
            }
        }
    } else {
        read_json_stream(
            reader,
            format,
            scope,
            limits,
            &mut report,
            &mut sink,
            &mut probe,
        )?;
    }
    probe(&report)?;
    Ok(report)
}

fn account<P: FnMut(&ReadReport) -> AppResult<()>>(
    count: usize,
    limits: ImportLimits,
    report: &mut ReadReport,
    probe: &mut P,
) -> AppResult<()> {
    report.expanded_bytes = report
        .expanded_bytes
        .checked_add(count as u64)
        .ok_or_else(|| resource("展开字节计数溢出"))?;
    if report.expanded_bytes > limits.expanded_bytes {
        return Err(resource("实际展开内容超过本次配置的总字节上限"));
    }
    probe(report)
}
fn drain<R: Read, P: FnMut(&ReadReport) -> AppResult<()>>(
    r: &mut R,
    limits: ImportLimits,
    report: &mut ReadReport,
    probe: &mut P,
) -> AppResult<()> {
    let mut buf = [0; 65536];
    loop {
        let n = r.read(&mut buf).map_err(io_error)?;
        if n == 0 {
            break;
        }
        account(n, limits, report, probe)?;
    }
    Ok(())
}

struct JsonReader<'a, R, P> {
    reader: BufReader<R>,
    report: &'a mut ReadReport,
    limits: ImportLimits,
    probe: &'a mut P,
    pending: usize,
}
impl<R: Read, P: FnMut(&ReadReport) -> AppResult<()>> JsonReader<'_, R, P> {
    fn byte(&mut self) -> AppResult<Option<u8>> {
        let data = self.reader.fill_buf().map_err(io_error)?;
        if data.is_empty() {
            self.flush()?;
            return Ok(None);
        }
        let b = data[0];
        self.reader.consume(1);
        self.pending += 1;
        if self.pending >= 65536 {
            self.flush()?;
        }
        Ok(Some(b))
    }
    fn flush(&mut self) -> AppResult<()> {
        let n = std::mem::take(&mut self.pending);
        if n > 0 {
            account(n, self.limits, self.report, self.probe)?;
        }
        Ok(())
    }
    fn nonspace(&mut self) -> AppResult<Option<u8>> {
        loop {
            match self.byte()? {
                Some(b' ' | b'\n' | b'\r' | b'\t') => {}
                v => return Ok(v),
            }
        }
    }
    /// 以 JSON 词法边界框住一个值，严格语法/重复键由公共验证器检查。
    fn value(&mut self, first: u8) -> AppResult<(Vec<u8>, Option<u8>)> {
        let mut bytes = vec![first];
        let mut depth = usize::from(matches!(first, b'{' | b'['));
        let mut string = first == b'"';
        let mut escape = false;
        if !string && depth == 0 && matches!(first, b',' | b']' | b'}') {
            return Err(invalid_json());
        }
        loop {
            if depth == 0 && !string && matches!(bytes.last(), Some(b'}' | b']' | b'"')) {
                return Ok((bytes, None));
            }
            let Some(b) = self.byte()? else {
                return if depth == 0 && !string {
                    Ok((bytes, None))
                } else {
                    Err(invalid_json())
                };
            };
            if depth == 0
                && !string
                && (matches!(b, b' ' | b'\n' | b'\r' | b'\t') || b == b',' || b == b']')
            {
                return Ok((bytes, Some(b)));
            }
            bytes.push(b);
            if bytes.len() > self.limits.conversation_bytes {
                return Err(resource(
                    "单个会话超过配置的内存字节上限；整份导出不受此单会话阈值限制",
                ));
            }
            if string {
                if escape {
                    escape = false;
                } else if b == b'\\' {
                    escape = true;
                } else if b == b'"' {
                    string = false;
                }
            } else {
                match b {
                    b'"' => string = true,
                    b'{' | b'[' => {
                        depth += 1;
                        if depth > 128 {
                            return Err(invalid_json());
                        }
                    }
                    b'}' | b']' => {
                        if depth == 0 {
                            return Err(invalid_json());
                        }
                        depth -= 1;
                    }
                    _ => {}
                }
            }
        }
    }
}

fn read_json_stream<R, S, P>(
    reader: R,
    format: &str,
    scope: &str,
    limits: ImportLimits,
    report: &mut ReadReport,
    sink: &mut S,
    probe: &mut P,
) -> AppResult<()>
where
    R: Read,
    S: FnMut(ImportedConversation) -> AppResult<()>,
    P: FnMut(&ReadReport) -> AppResult<()>,
{
    let mut stream = JsonReader {
        reader: BufReader::with_capacity(65536, reader),
        report,
        limits,
        probe,
        pending: 0,
    };
    let first = stream.nonspace()?.ok_or_else(invalid_json)?;
    if first == b'[' {
        let mut next = stream.nonspace()?.ok_or_else(invalid_json)?;
        if next != b']' {
            loop {
                let (bytes, end) = stream.value(next)?;
                process_value(&bytes, format, scope, limits, stream.report, sink)?;
                let delimiter = match end {
                    Some(b) if !matches!(b, b' ' | b'\n' | b'\r' | b'\t') => Some(b),
                    _ => stream.nonspace()?,
                };
                match delimiter {
                    Some(b']') => break,
                    Some(b',') => {
                        next = stream.nonspace()?.ok_or_else(invalid_json)?;
                        if next == b']' {
                            return Err(invalid_json());
                        }
                    }
                    _ => return Err(invalid_json()),
                }
            }
        }
    } else {
        let (bytes, end) = stream.value(first)?;
        process_value(&bytes, format, scope, limits, stream.report, sink)?;
        if end.is_some_and(|b| !matches!(b, b' ' | b'\n' | b'\r' | b'\t')) {
            return Err(invalid_json());
        }
    }
    if stream.nonspace()?.is_some() {
        return Err(invalid_json());
    }
    stream.flush()
}

pub(super) fn adapter_error(message: String, platform: &str) -> AppError {
    if matches!(
        message.as_str(),
        "事件分块超过 2 MiB"
            | "单条事件不能超过 2 MiB"
            | "单条事件整体超过 4 MiB"
            | "DeepSeek 会话标题超过 4096 字节"
            | "规范化会话超过 60 MiB 规范化输出预算"
    ) {
        resource(&message)
    } else {
        AppError::new(
            ErrorCode::InvalidConversation,
            format!("{platform} 会话身份、时间或分支关系无效"),
            "重新取得完整官方导出；原始资料未被修改",
        )
    }
}
fn process_value<S: FnMut(ImportedConversation) -> AppResult<()>>(
    bytes: &[u8],
    format: &str,
    scope: &str,
    limits: ImportLimits,
    report: &mut ReadReport,
    sink: &mut S,
) -> AppResult<()> {
    let value = import_bundle::strict_json(bytes).map_err(|_| invalid_json())?;
    let provider = import_bundle::provider(&value).map_err(|_| {
        AppError::new(
            ErrorCode::InvalidConversation,
            "会话包含互相冲突的平台标记",
            "重新导出或核对数据来源",
        )
    })?;
    let Some(provider) = provider else {
        report.ignored_values += 1;
        return Ok(());
    };
    if format != "auto" && provider != format {
        return Err(AppError::new(
            ErrorCode::UnsupportedFormat,
            "导出平台与指定格式不一致",
            "使用自动识别或正确的平台格式",
        ));
    }
    let mut conversation = if provider == "deepseek-export" {
        let p =
            parse_deepseek_conversation(&value, scope).map_err(|e| adapter_error(e, "DeepSeek"))?;
        ImportedConversation {
            platform: "deepseek".into(),
            source_id: p.source_id,
            title: p.title,
            events: p.events,
            chatgpt_coverage: ChatgptCoverage::default(),
            deepseek_coverage: p.coverage,
        }
    } else {
        let p = parse_chatgpt_conversation_all(&value, scope)
            .map_err(|e| adapter_error(e, "ChatGPT"))?;
        ImportedConversation {
            platform: "chatgpt-export".into(),
            source_id: p.source_id,
            title: p.title,
            events: p.events,
            chatgpt_coverage: p.coverage,
            deepseek_coverage: DeepseekCoverage::default(),
        }
    };
    if provider == "deepseek-export" {
        super::source_assets::deepseek(&value, &mut conversation, report, scope)?;
    }
    for event in &mut conversation.events {
        crate::capture::redact_event(event).map_err(|_| {
            AppError::new(
                ErrorCode::InvalidConversation,
                "事件内容无法安全规范化",
                "查看受影响来源，原始文件未改变",
            )
        })?;
    }
    report.conversations += 1;
    report.events += conversation.events.len() as u64;
    if report.events > limits.events {
        return Err(resource("导入事件数量超过配置上限"));
    }
    report.hidden_fragments += conversation.deepseek_coverage.hidden_fragments_skipped as u64
        + conversation
            .chatgpt_coverage
            .hidden_reasoning_messages_skipped as u64;
    report.unsupported_fragments += conversation.deepseek_coverage.unsupported_fragments_skipped
        as u64
        + conversation
            .chatgpt_coverage
            .unsupported_content_parts_skipped as u64;
    report.omitted_messages += (conversation.deepseek_coverage.empty_messages_skipped
        + conversation.deepseek_coverage.hidden_only_messages_skipped
        + conversation.deepseek_coverage.unsupported_messages_skipped
        + conversation
            .deepseek_coverage
            .ambiguous_role_messages_skipped
        + conversation.chatgpt_coverage.empty_messages_skipped
        + conversation.chatgpt_coverage.unsupported_messages_skipped
        + conversation
            .chatgpt_coverage
            .hidden_reasoning_messages_skipped
        + conversation.chatgpt_coverage.other_branch_messages_skipped)
        as u64;
    sink(conversation)
}

struct ZipEntry {
    name: String,
    size: u64,
}
fn at<R: Read + Seek>(reader: &mut R, offset: u64, len: usize) -> AppResult<Vec<u8>> {
    reader.seek(SeekFrom::Start(offset)).map_err(io_error)?;
    let mut bytes = vec![0; len];
    reader.read_exact(&mut bytes).map_err(io_error)?;
    Ok(bytes)
}
fn u16le(bytes: &[u8], offset: usize) -> AppResult<u16> {
    let v = bytes
        .get(offset..offset + 2)
        .ok_or_else(|| invalid_archive("ZIP 目录被截断"))?;
    Ok(u16::from_le_bytes([v[0], v[1]]))
}
fn u32le(bytes: &[u8], offset: usize) -> AppResult<u32> {
    let v = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| invalid_archive("ZIP 目录被截断"))?;
    Ok(u32::from_le_bytes([v[0], v[1], v[2], v[3]]))
}

/// 在 zip crate 分配目录之前检查数量、路径与数据区域；仅分配有界目录片段。
fn audit_zip<R: Read + Seek>(reader: &mut R, limits: ImportLimits) -> AppResult<Vec<ZipEntry>> {
    let length = reader.seek(SeekFrom::End(0)).map_err(io_error)?;
    if length < 22 {
        return Err(invalid_archive("ZIP 文件不完整"));
    }
    let tail_len = length.min(65557) as usize;
    let tail = at(reader, length - tail_len as u64, tail_len)?;
    let footer = (0..=tail.len() - 22)
        .rev()
        .find(|&i| {
            tail[i..].starts_with(b"PK\x05\x06")
                && u16le(&tail, i + 20).is_ok_and(|n| i + 22 + n as usize == tail.len())
        })
        .ok_or_else(|| invalid_archive("ZIP 缺少完整目录尾部"))?;
    let count = u16le(&tail, footer + 10)? as usize;
    if u16le(&tail, footer + 4)? != 0
        || u16le(&tail, footer + 6)? != 0
        || u16le(&tail, footer + 8)? as usize != count
        || count == 65535
    {
        return Err(invalid_archive("暂不支持分卷或 ZIP64 导出"));
    }
    if count > limits.archive_entries {
        return Err(resource("ZIP 成员数超过配置上限"));
    }
    let size = u32le(&tail, footer + 12)? as u64;
    let start = u32le(&tail, footer + 16)? as u64;
    let end = length - tail_len as u64 + footer as u64;
    if start.checked_add(size) != Some(end) {
        return Err(invalid_archive("ZIP 中央目录位置不一致"));
    }
    let mut cursor = start;
    let mut names = BTreeMap::new();
    let mut ranges = Vec::with_capacity(count);
    let mut entries = Vec::with_capacity(count);
    let mut total = 0u64;
    for _ in 0..count {
        let head = at(reader, cursor, 46)?;
        if &head[..4] != b"PK\x01\x02" {
            return Err(invalid_archive("ZIP 中央目录条目无效"));
        }
        let flags = u16le(&head, 8)?;
        let method = u16le(&head, 10)?;
        if flags & (1 | 64 | 8192) != 0 || ![0, 8].contains(&method) {
            return Err(invalid_archive("ZIP 仅支持未加密的 Stored / Deflate 成员"));
        }
        let compressed = u32le(&head, 20)? as u64;
        let expanded = u32le(&head, 24)? as u64;
        if compressed == u32::MAX as u64 || expanded == u32::MAX as u64 || u16le(&head, 34)? != 0 {
            return Err(invalid_archive("ZIP64 或分卷成员不受支持"));
        }
        let name_len = u16le(&head, 28)? as usize;
        let extra = u16le(&head, 30)? as usize;
        let comment = u16le(&head, 32)? as usize;
        let next = cursor
            .checked_add((46 + name_len + extra + comment) as u64)
            .ok_or_else(|| invalid_archive("ZIP 目录位置溢出"))?;
        if next > end {
            return Err(invalid_archive("ZIP 目录条目越界"));
        }
        let name_bytes = at(reader, cursor + 46, name_len)?;
        let name = std::str::from_utf8(&name_bytes)
            .map_err(|_| invalid_archive("ZIP 成员名称不是 UTF-8"))?
            .to_owned();
        let directory = name.ends_with('/');
        import_bundle::validate_name(&name, directory, &mut names)
            .map_err(|_| invalid_archive("ZIP 含不安全路径、同名冲突或目录冲突"))?;
        let mode = (u32le(&head, 38)? >> 16) & 0o170000;
        if ![0, 0o100000, 0o040000].contains(&mode)
            || (mode == 0o040000 && !directory)
            || (mode == 0o100000 && directory)
            || (directory && expanded != 0)
        {
            return Err(invalid_archive("ZIP 不允许链接、特殊文件或冲突目录"));
        }
        total = total
            .checked_add(expanded)
            .ok_or_else(|| resource("ZIP 展开大小溢出"))?;
        if total > limits.expanded_bytes {
            return Err(resource("ZIP 声明展开总量超过配置上限"));
        }
        let local = u32le(&head, 42)? as u64;
        let lh = at(reader, local, 30)?;
        if &lh[..4] != b"PK\x03\x04" || u16le(&lh, 6)? != flags || u16le(&lh, 8)? != method {
            return Err(invalid_archive("ZIP 本地条目与中央目录冲突"));
        }
        if flags & 8 == 0
            && (u32le(&lh, 14)? != u32le(&head, 16)?
                || u32le(&lh, 18)? as u64 != compressed
                || u32le(&lh, 22)? as u64 != expanded)
        {
            return Err(invalid_archive("ZIP 本地 CRC 或大小与中央目录冲突"));
        }
        let ln = u16le(&lh, 26)? as usize;
        let le = u16le(&lh, 28)? as usize;
        if at(reader, local + 30, ln)? != name_bytes {
            return Err(invalid_archive("ZIP 本地文件名与中央目录冲突"));
        }
        let data_end = local
            .checked_add((30 + ln + le) as u64)
            .and_then(|n| n.checked_add(compressed))
            .ok_or_else(|| invalid_archive("ZIP 数据位置溢出"))?;
        if data_end > start {
            return Err(invalid_archive("ZIP 成员数据越界"));
        }
        ranges.push((local, data_end));
        entries.push(ZipEntry {
            name,
            size: expanded,
        });
        cursor = next;
    }
    if cursor != end {
        return Err(invalid_archive("ZIP 目录数量或长度不一致"));
    }
    ranges.sort_unstable();
    if ranges.windows(2).any(|w| w[0].1 > w[1].0) {
        return Err(invalid_archive("ZIP 成员数据区域互相覆盖"));
    }
    Ok(entries)
}
