//! ChatGPT 附件显式联接与不透明字节保存；不执行、不提取正文、不发给模型。
use super::{AppError, AppResult, ErrorCode};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
};
fn invalid() -> AppError {
    AppError::new(
        ErrorCode::InvalidArchive,
        "附件目录或原件与导出声明不一致",
        "重新取得完整导出；未猜测关联或覆盖原件",
    )
}
fn clean(value: &Value, max: usize) -> Option<&str> {
    value
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= max && !s.contains('\0'))
}
fn valid_id(s: &str) -> bool {
    (s.starts_with("file_") || s.starts_with("file-"))
        && s.len() <= 256
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}
pub(crate) fn pointer_id(part: &Value) -> Option<&str> {
    if part["content_type"] != "image_asset_pointer" {
        return None;
    }
    clean(&part["asset_pointer"], 512)
        .and_then(|s| s.strip_prefix("sediment://"))
        .filter(|s| valid_id(s))
}
/// 调用方已过滤隐藏推理；仅解释明确附件字段，不遍历任意嵌套内容。
pub(crate) fn references(message: &Value) -> Vec<Value> {
    let mut rows = BTreeMap::new();
    if let Some(items) = message["metadata"]["attachments"].as_array() {
        for a in items {
            if let Some(id) = clean(&a["id"], 256).filter(|s| valid_id(s)) {
                rows.entry(id.to_owned()).or_insert_with(||json!({"source_file_id":id,"library_file_id":clean(&a["library_file_id"],256),"name":clean(&a["name"],4096),"declared_media_type":clean(&a["mime_type"],256),"byte_count":a["size"].as_u64(),"payload_status":"missing","association":"message.metadata.attachments","content_indexed":false}));
            }
        }
    }
    if let Some(parts) = message["content"]["parts"].as_array() {
        for (index, part) in parts.iter().enumerate() {
            if part["content_type"] != "image_asset_pointer" {
                continue;
            }
            if let Some(pointer) = clean(&part["asset_pointer"], 512) {
                if let Some(id) = pointer.strip_prefix("sediment://").filter(|s| valid_id(s)) {
                    let row=rows.entry(id.into()).or_insert_with(||json!({"source_file_id":id,"name":null,"byte_count":part["size_bytes"].as_u64(),"payload_status":"missing","association":"message.content.parts","content_indexed":false}));
                    row["asset_pointer"] = json!(pointer);
                    if row.get("part_indices").is_none() {
                        row["part_indices"] = json!([]);
                    }
                    row["part_indices"]
                        .as_array_mut()
                        .unwrap()
                        .push(json!(index));
                }
            }
        }
    }
    rows.into_values().collect()
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetPayload {
    pub sha256: String,
    pub byte_count: u64,
    pub media_type: String,
    pub payload_status: String,
}
fn media_type(p: &[u8]) -> &'static str {
    if p.starts_with(b"%PDF-") {
        "application/pdf"
    } else if p.starts_with(b"\x89PNG\r\n\x1a\n") {
        "image/png"
    } else if p.starts_with(b"\xff\xd8\xff") {
        "image/jpeg"
    } else if p.starts_with(b"GIF87a") || p.starts_with(b"GIF89a") {
        "image/gif"
    } else if p.starts_with(b"RIFF") && p.get(8..12) == Some(b"WEBP") {
        "image/webp"
    } else if p.starts_with(b"PK\x03\x04") {
        "application/zip"
    } else {
        "application/octet-stream"
    }
}
pub(crate) fn consume(
    reader: &mut dyn Read,
    expected: u64,
    mut output: Option<&mut dyn Write>,
    mut check: impl FnMut() -> AppResult<()>,
) -> AppResult<AssetPayload> {
    let mut hash = Sha256::new();
    let mut count = 0u64;
    let mut prefix = Vec::new();
    let mut buffer = [0u8; 65536];
    loop {
        check()?;
        let n = reader.read(&mut buffer).map_err(|_| invalid())?;
        if n == 0 {
            break;
        }
        count = count.checked_add(n as u64).ok_or_else(invalid)?;
        if count > expected {
            return Err(invalid());
        }
        let take = (64 - prefix.len()).min(n);
        prefix.extend_from_slice(&buffer[..take]);
        hash.update(&buffer[..n]);
        if let Some(w) = output.as_mut() {
            w.write_all(&buffer[..n]).map_err(|_| invalid())?;
        }
    }
    if count != expected {
        return Err(invalid());
    }
    Ok(AssetPayload {
        sha256: format!("{:x}", hash.finalize()),
        byte_count: count,
        media_type: media_type(&prefix).into(),
        payload_status: if output.is_some() {
            "retained"
        } else {
            "available_not_retained"
        }
        .into(),
    })
}
#[derive(Default)]
pub(crate) struct Catalog {
    library: BTreeMap<String, Value>,
    aliases: BTreeMap<String, String>,
    declared: BTreeMap<String, u64>,
    logical: BTreeMap<String, Vec<String>>,
    payloads: BTreeMap<String, AssetPayload>,
    members: BTreeMap<String, Vec<String>>,
}
impl Catalog {
    pub fn metadata(&mut self, name: &str, value: Value) -> AppResult<()> {
        match name {
            "library_files.json" => {
                for row in value.as_array().ok_or_else(invalid)? {
                    if let Some(id) = clean(&row["file_id"], 256).filter(|s| valid_id(s)) {
                        if self.library.insert(id.into(), row.clone()).is_some() {
                            return Err(invalid());
                        }
                    }
                }
            }
            "conversation_asset_file_names.json" => {
                for (k, v) in value.as_object().ok_or_else(invalid)? {
                    if let Some(alias) = clean(v, 4096) {
                        self.aliases.insert(k.clone(), alias.into());
                    }
                }
            }
            "export_manifest.json" => {
                if let Some(files) = value["export_files"].as_array() {
                    for f in files {
                        if let (Some(path), Some(size)) =
                            (clean(&f["path"], 4096), f["size_bytes"].as_u64())
                        {
                            if self.declared.insert(path.into(), size).is_some() {
                                return Err(invalid());
                            }
                        }
                    }
                }
                if let Some(files) = value["logical_files"].as_object() {
                    for (name, row) in files {
                        let parts = row["files"]
                            .as_array()
                            .filter(|p| row["sharded"] == false && p.len() == 1)
                            .and_then(|p| clean(&p[0], 4096))
                            .map(|p| vec![p.into()])
                            .unwrap_or_default();
                        self.logical.insert(name.clone(), parts);
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
    pub fn prepare_members(&mut self) {
        let mut ids: std::collections::BTreeSet<String> = self.library.keys().cloned().collect();
        for logical in self.aliases.keys().chain(self.logical.keys()) {
            if let Some(id) = logical.strip_suffix(".dat").filter(|id| valid_id(id)) {
                ids.insert(id.into());
            }
        }
        for id in ids {
            let logical = format!("{id}.dat");
            let paths = self
                .logical
                .get(&logical)
                .cloned()
                .unwrap_or_else(|| vec![logical]);
            for path in paths {
                self.members.entry(path).or_default().push(id.clone());
            }
        }
    }
    pub fn member_ids(&self, name: &str) -> Vec<String> {
        self.members.get(name).cloned().unwrap_or_else(|| {
            if self.logical.contains_key(name) {
                return Vec::new();
            }
            name.strip_suffix(".dat")
                .filter(|id| valid_id(id))
                .map(|id| vec![id.to_owned()])
                .unwrap_or_default()
        })
    }
    pub fn add_payload(
        &mut self,
        name: &str,
        ids: &[String],
        payload: AssetPayload,
    ) -> AppResult<()> {
        if self
            .declared
            .get(name)
            .is_some_and(|size| *size != payload.byte_count)
        {
            return Err(invalid());
        }
        for id in ids {
            if self.payloads.insert(id.clone(), payload.clone()).is_some() {
                return Err(invalid());
            }
        }
        Ok(())
    }
    pub fn enrich(
        &self,
        event: &mut crate::EventInput,
        report: &mut super::import_reader::ReadReport,
    ) -> AppResult<()> {
        let Some(files) = event
            .metadata
            .get_mut("source_assets")
            .and_then(|assets| assets.get_mut("files"))
            .and_then(Value::as_array_mut)
        else {
            return Ok(());
        };
        for f in files {
            let id = f["source_file_id"].as_str().ok_or_else(invalid)?.to_owned();
            report.file_references += 1;
            if self
                .logical
                .get(&format!("{id}.dat"))
                .is_some_and(Vec::is_empty)
            {
                return Err(AppError::new(
                    ErrorCode::UnsupportedFormat,
                    "附件使用尚未支持的分片布局",
                    "保留完整导出，当前任务未提交；不会把分片猜成完整原件",
                ));
            }
            if let Some(l) = self.library.get(&id) {
                f["library_name"] = json!(clean(&l["file_name"], 4096));
                f["library_media_type"] = json!(clean(&l["mime_type"], 256));
                f["library_byte_count"] = json!(l["file_size_bytes"].as_u64());
                f["library_state"] = json!(clean(&l["state"], 64));
                if f["name"].is_null() {
                    f["name"] = f["library_name"].clone();
                }
                if f["byte_count"].is_null() {
                    f["byte_count"] = f["library_byte_count"].clone();
                }
                f["declared_media_type_conflict"] = json!(
                    !f["declared_media_type"].is_null()
                        && !f["library_media_type"].is_null()
                        && f["declared_media_type"] != f["library_media_type"]
                );
            }
            f["archive_name_alias"] = json!(self.aliases.get(&format!("{id}.dat")));
            if let Some(p) = self.payloads.get(&id) {
                if f["byte_count"].as_u64().is_some_and(|n| n != p.byte_count)
                    || f["library_byte_count"]
                        .as_u64()
                        .is_some_and(|n| n != p.byte_count)
                {
                    return Err(invalid());
                }
                f["object_sha256"] = json!(p.sha256);
                f["detected_media_type"] = json!(p.media_type);
                f["byte_count"] = json!(p.byte_count);
                f["payload_status"] = json!(p.payload_status);
                if p.payload_status == "retained" {
                    report.attachment_payloads_retained += 1;
                }
            } else {
                report.attachment_payloads_missing += 1;
                f["missing_reason"] = json!("not_present_in_supplied_archive");
            }
        }
        Ok(())
    }
}
