//! 仅对当前已授权父事件声明的有界对象做元数据盘点；不扫描 objects 目录。
use super::*;
use crate::context::attachments;

struct Declaration {
    parent: String,
    file: usize,
    object: String,
    metadata: Value,
}
#[derive(Default)]
pub(super) struct Plan {
    declarations: Vec<Declaration>,
    examined: usize,
    bytes: usize,
    skipped: usize,
    limited: bool,
}
impl Plan {
    /// 调用方先应用范围、修订、抑制、来源和独立附件批准；空正文父事件仍可入选。
    pub fn observe(&mut self, event: &Event, file_index: usize) {
        if self.limited {
            return;
        }
        for file in attachments::files(event) {
            if self.examined >= attachments::FILES {
                self.limited = true;
                break;
            }
            self.examined += 1;
            let Some(size) = file["byte_count"]
                .as_u64()
                .filter(|size| *size <= attachments::FILE_BYTES as u64)
            else {
                self.skipped += 1;
                continue;
            };
            let Some(object) = file["object_sha256"]
                .as_str()
                .filter(|object| attachments::digest_valid(object))
            else {
                self.skipped += 1;
                continue;
            };
            if !attachments::supported(file) {
                self.skipped += 1;
                continue;
            }
            if self.bytes + size as usize > attachments::TOTAL_BYTES {
                self.limited = true;
                break;
            }
            // 缺失、损坏、重复声明同样消耗元数据预算；不通过失败读取绕过上限。
            self.bytes += size as usize;
            // 仅保留验证器使用的有界字段；不缓存父事件正文或任意附件元数据。
            let metadata = json!({
                "object_sha256":object,"byte_count":size,"payload_status":"retained",
                "detected_media_type":file["detected_media_type"].as_str(),
                "declared_media_type":file["declared_media_type"].as_str().or_else(|| file["library_media_type"].as_str())
            });
            self.declarations.push(Declaration {
                parent: event.id.clone(),
                file: file_index,
                object: object.to_owned(),
                metadata,
            });
        }
    }
    pub fn stamps(&self, vault: &Vault) -> Vec<Value> {
        self.declarations
            .iter()
            .map(|declaration| {
                json!([
                    declaration.parent,
                    declaration.object,
                    declaration.metadata["byte_count"],
                    object_stamp(
                        vault,
                        &declaration.object,
                        declaration.metadata["byte_count"].as_u64().unwrap()
                    )
                ])
            })
            .collect()
    }
    pub fn append_sources(
        &self,
        vault: &Vault,
        sources: &mut Vec<snapshot::Source>,
        chunk_bytes: usize,
    ) -> Value {
        let mut skipped = self.skipped;
        let mut documents = 0usize;
        let mut seen = BTreeSet::new();
        for declaration in &self.declarations {
            let key = (declaration.parent.clone(), declaration.object.clone());
            if seen.contains(&key) {
                continue;
            }
            // 元数据只绑定快照，不证明正文可信；完整摘要和安全 UTF-8 验证不可省略。
            let text = match attachments::body(vault, &declaration.metadata) {
                Ok(text) => text,
                Err(_) => {
                    skipped += 1;
                    continue;
                }
            };
            let chunks = snapshot::ranges(&text, chunk_bytes).len();
            if chunks == 0 {
                skipped += 1;
                continue;
            }
            seen.insert(key);
            sources.push(snapshot::Source {
                id: declaration.parent.clone(),
                file: declaration.file,
                chunks,
                object: Some(declaration.object.clone()),
            });
            documents += 1;
        }
        json!({"status":if self.limited || skipped > 0 {"partial"}else{"ready"},
            "projection":attachments::PROJECTION,"mode":"bounded_local_semantic_index","semantic":true,
            "files_examined":self.examined,"documents":documents,"skipped":skipped,"limited":self.limited,
            "bytes_examined":self.bytes,"file_bytes_limit":attachments::FILE_BYTES,
            "total_bytes_limit":attachments::TOTAL_BYTES,"file_limit":attachments::FILES})
    }
}

/// Unix 不读原件正文；其他平台补有界摘要，避免同长改写后恢复 mtime 沿用旧 epoch。
/// 同路径替换、缺失和类型变化也会改变 epoch。
fn object_stamp(vault: &Vault, object: &str, declared_size: u64) -> Value {
    let path = vault.root().join("objects").join(object);
    if crate::vault::reject_symlink(&path).is_err() {
        return json!(["invalid_path"]);
    }
    let input = match open_local_file(&path) {
        Ok(input) => input,
        Err(_) => {
            return match std::fs::symlink_metadata(&path) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => json!(["missing"]),
                _ => json!(["unavailable"]),
            }
        }
    };
    let Ok(metadata) = input.metadata() else {
        return json!(["metadata_unavailable"]);
    };
    let Ok(identity) = crate::filesystem::file_identity(&input) else {
        return json!(["identity_unavailable"]);
    };
    let Ok(modified) = metadata.modified() else {
        return json!(["timestamp_unavailable"]);
    };
    let mut stamp = json!([
        if metadata.is_file() && metadata.len() == declared_size {
            "available"
        } else {
            "invalid_size_or_kind"
        },
        identity,
        metadata.len(),
        modified
    ]);
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        stamp
            .as_array_mut()
            .unwrap()
            .push(json!([metadata.ctime(), metadata.ctime_nsec()]));
    }
    #[cfg(not(unix))]
    if metadata.is_file() && metadata.len() == declared_size {
        // Windows/其他平台没有这里可依赖的 ctime；读量受已经批准的 Plan 总预算限制。
        let mut bytes = Vec::new();
        let content = match input.take(declared_size + 1).read_to_end(&mut bytes) {
            Ok(_) if bytes.len() as u64 == declared_size => json!(["sha256", hash(&bytes)]),
            _ => json!(["content_unavailable"]),
        };
        stamp.as_array_mut().unwrap().push(content);
    }
    stamp
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn malformed_unused_metadata_cannot_expand_the_in_memory_plan() {
        let directory = tempfile::tempdir().unwrap();
        let vault = Vault::init(&directory.path().join("vault")).unwrap();
        let text = b"safe-body";
        let object = hash(text);
        std::fs::write(vault.root().join("objects").join(&object), text).unwrap();
        let large_wrong_type = json!({"unused":"x".repeat(64 * 1024)});
        let event = vault.capture(serde_json::from_value(json!({
            "role":"user","origin":"user_input","content":"",
            "source":{"platform":"synthetic","conversation_id":"metadata-contract","message_id":"bounded"},
            "metadata":{"source_assets":{"files":[{
                "payload_status":"retained","detected_media_type":"application/octet-stream",
                "declared_media_type":"text/plain","object_sha256":object,"byte_count":text.len(),
                "library_media_type":large_wrong_type,"declared_media_type_conflict":large_wrong_type
            }]}}
        })).unwrap()).unwrap();
        let mut plan = Plan::default();
        plan.observe(&event, 0);
        assert_eq!(plan.declarations.len(), 1);
        let metadata = &plan.declarations[0].metadata;
        assert!(serde_json::to_vec(metadata).unwrap().len() < 512);
        assert!(metadata.get("library_media_type").is_none());
        assert!(metadata.get("declared_media_type_conflict").is_none());
        assert_eq!(attachments::body(&vault, metadata).unwrap(), "safe-body");
    }
    #[cfg(unix)]
    #[test]
    fn object_directory_symlink_is_rejected_before_opening_any_object() {
        let directory = tempfile::tempdir().unwrap();
        let vault = Vault::init(&directory.path().join("vault")).unwrap();
        let outside = directory.path().join("outside");
        std::fs::create_dir(&outside).unwrap();
        let object = hash(b"outside");
        std::fs::write(outside.join(&object), b"outside").unwrap();
        std::fs::rename(
            vault.root().join("objects"),
            vault.root().join("original-objects"),
        )
        .unwrap();
        std::os::unix::fs::symlink(&outside, vault.root().join("objects")).unwrap();
        assert_eq!(object_stamp(&vault, &object, 7), json!(["invalid_path"]));
    }
}
