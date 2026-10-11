//! 本机 query-only 配置和原子查询缓存；没有来源正文或文件路径进入 worker 请求。
use super::*;
use std::io::Write;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalQueryConfig {
    pub endpoint: String,
    pub scopes: Vec<String>,
    #[serde(default = "default_calls")]
    pub max_local_calls: usize,
    #[serde(default = "default_bytes")]
    pub max_input_bytes: usize,
}
impl LocalQueryConfig {
    pub(super) fn valid(&self) -> bool {
        let rest = self.endpoint.strip_prefix("http://").unwrap_or("");
        let (authority, path) = rest.split_once('/').unwrap_or(("", ""));
        let port = authority
            .strip_prefix("127.0.0.1:")
            .or_else(|| authority.strip_prefix("[::1]:"));
        path == "v1/embeddings"
            && port.is_some_and(|p| {
                !p.is_empty()
                    && p.bytes().all(|b| b.is_ascii_digit())
                    && p.parse::<u16>().is_ok_and(|v| v > 0)
            })
            && !self.scopes.is_empty()
            && valid_scopes(&self.scopes)
            && (1..=1000).contains(&self.max_local_calls)
            && (1..=4 * 1024 * 1024).contains(&self.max_input_bytes)
    }
}
impl SemanticSearch {
    pub(super) fn persist_local_query(
        &self,
        space: &Space,
        query: &str,
        encoded: &EncodedQuery,
    ) -> SResult<()> {
        if (self.config.local_query.is_none() && self.managed.is_none())
            || encoded.input_hash != prepared_hash(&space.query_instruction, query)
        {
            return failure("malformed_response");
        }
        validate_vector(
            &encoded.vector,
            space.dimensions,
            space.normalization == "l2",
        )?;
        let path = self
            .config
            .query_cache_path
            .as_ref()
            .ok_or(SemanticError("query_cache_unavailable"))?;
        let parent = path
            .parent()
            .ok_or(SemanticError("query_cache_unavailable"))?;
        if !std::fs::symlink_metadata(parent).is_ok_and(|m| m.is_dir()) {
            return failure("query_cache_unavailable");
        }
        let mut lock_name = path.as_os_str().to_os_string();
        lock_name.push(".query-lock");
        let lock_path = PathBuf::from(lock_name);
        let lock = match std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&lock_path)
        {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                crate::filesystem::open_local_file(&lock_path)
                    .map_err(|_| SemanticError("query_cache_unavailable"))?
            }
            Err(_) => return failure("query_cache_unavailable"),
        };
        if !lock.metadata().is_ok_and(|m| m.is_file()) {
            return failure("query_cache_unavailable");
        }
        lock.try_lock()
            .map_err(|_| SemanticError("query_cache_busy"))?;
        // File drop 释放锁；不删除锁文件，避免并发者锁住两个不同 inode。
        let mut cache = match std::fs::symlink_metadata(path) {
            Ok(_) => read_local::<QueryCache>(path)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => QueryCache {
                schema: "recallcard.embedding-query-cache/1".into(),
                space_signature: self.config.expected_space_signature.clone(),
                queries: vec![],
            },
            Err(_) => return failure("query_cache_unavailable"),
        };
        self.validate_query_cache(&cache, space)?;
        let query_hash = hash(query.as_bytes());
        cache.queries.retain(|row| row.query_hash != query_hash);
        cache.queries.push(CachedQuery {
            query_hash,
            input_hash: encoded.input_hash.clone(),
            vector: encoded.vector.clone(),
        });
        if cache.queries.len() > 32 {
            cache.queries.drain(..cache.queries.len() - 32);
        }
        let bytes =
            serde_json::to_vec(&cache).map_err(|_| SemanticError("query_cache_write_failed"))?;
        if bytes.len() as u64 > MAX_CACHE_BYTES {
            return failure("query_cache_write_failed");
        }
        let mut file = tempfile::NamedTempFile::new_in(parent)
            .map_err(|_| SemanticError("query_cache_write_failed"))?;
        file.write_all(&bytes)
            .and_then(|()| file.as_file().sync_all())
            .map_err(|_| SemanticError("query_cache_write_failed"))?;
        crate::vault::reject_symlink(path)
            .map_err(|_| SemanticError("query_cache_write_failed"))?;
        file.persist(path)
            .map_err(|_| SemanticError("query_cache_write_failed"))?;
        #[cfg(unix)]
        File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|_| SemanticError("query_cache_write_failed"))?;
        Ok(())
    }
    pub(super) fn validate_query_cache(&self, cache: &QueryCache, space: &Space) -> SResult<()> {
        if cache.schema != "recallcard.embedding-query-cache/1"
            || cache.space_signature != self.config.expected_space_signature
            || cache.queries.len() > 32
        {
            return failure("query_cache_incompatible");
        }
        let mut seen = BTreeSet::new();
        for row in &cache.queries {
            if !digest(&row.query_hash) || !digest(&row.input_hash) || !seen.insert(&row.query_hash)
            {
                return failure("invalid_cache");
            }
            validate_vector(&row.vector, space.dimensions, false)?;
        }
        Ok(())
    }
}
