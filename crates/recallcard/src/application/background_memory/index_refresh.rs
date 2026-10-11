//! 批准写入之外的可恢复索引任务；失败绝不撤销 Memory 事务。
use super::*;
use crate::{context::Context, policy::Access, semantic::local_index::Work};
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IndexRefreshStatus {
    pub state: String,
    pub message: String,
    pub generation: Option<String>,
    pub indexed_generation: Option<String>,
    pub ready_documents: usize,
    pub total_documents: usize,
    pub new_vectors: usize,
    pub attempts: u64,
    pub failures: u32,
    pub error_code: Option<String>,
    pub can_retry: bool,
    pub checked_at: DateTime<Utc>,
    pub retry_after: Option<DateTime<Utc>>,
}
impl Default for IndexRefreshStatus {
    fn default() -> Self {
        Self {
            state: "pending".into(),
            message: "等待核验文档向量索引；已批准记忆保留可读".into(),
            generation: None,
            indexed_generation: None,
            ready_documents: 0,
            total_documents: 0,
            new_vectors: 0,
            attempts: 0,
            failures: 0,
            error_code: None,
            can_retry: false,
            checked_at: Utc::now(),
            retry_after: None,
        }
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    binding: String,
    config_hash: String,
    status: IndexRefreshStatus,
}
impl MemoryRuntime<'_> {
    fn index_path(&self) -> AppResult<PathBuf> {
        Ok(self.directory()?.join("index-refresh.json"))
    }
    fn read_index_record(&self) -> AppResult<Option<Record>> {
        let p = self.index_path()?;
        if p.exists() {
            bounded_read(&p).map(Some)
        } else {
            Ok(None)
        }
    }
    fn save_index_record(&self, record: &mut Record) -> AppResult<()> {
        record.status.checked_at = Utc::now();
        self.vault
            .write_replace(&self.index_path()?, record)
            .map_err(storage)
    }
    fn index_config_hash(path: &Path) -> AppResult<String> {
        let file = open_local_file(path).map_err(storage)?;
        if !file.metadata().map_err(storage)?.is_file()
            || file.metadata().map_err(storage)?.len() > 65536
        {
            return Err(invalid("本机索引配置不是有界普通文件"));
        }
        use std::io::Read;
        let mut bytes = Vec::new();
        file.take(65537).read_to_end(&mut bytes).map_err(storage)?;
        if bytes.len() > 65536 {
            return Err(invalid("本机索引配置超过上限"));
        }
        Ok(hash(&bytes))
    }
    pub(super) fn index_status_locked(
        &self,
        config: &MemoryConfig,
    ) -> AppResult<Option<IndexRefreshStatus>> {
        if config.candidate_semantic_config.is_none() {
            return Ok(None);
        }
        let mut status = self
            .read_index_record()?
            .filter(|r| self.binding(config).is_ok_and(|b| r.binding == b))
            .map(|r| r.status)
            .unwrap_or_default();
        if config.paused || (!config.enabled && !config.index_maintenance_enabled) {
            status.state = "paused".into();
            status.message = "索引维护暂停；已保存 Memory 与词法检索保留".into();
            status.can_retry = false;
        }
        Ok(Some(status))
    }
    pub(super) fn queue_index_locked(&self, config: &MemoryConfig) -> AppResult<()> {
        let Some(path) = &config.candidate_semantic_config else {
            return Ok(());
        };
        let binding = self.binding(config)?;
        let config_hash = Self::index_config_hash(path).unwrap_or_default();
        let mut status = self
            .read_index_record()?
            .filter(|r| r.binding == binding && r.config_hash == config_hash)
            .map(|r| r.status)
            .unwrap_or_default();
        status.state = "pending".into();
        status.message = "Memory 已保存；等待独立索引维护核验".into();
        status.generation = None;
        status.ready_documents = 0;
        status.total_documents = 0;
        status.failures = 0;
        status.error_code = None;
        status.can_retry = false;
        status.retry_after = None;
        self.save_index_record(&mut Record {
            binding,
            config_hash,
            status,
        })
    }
    pub fn retry_index(&self, scope: &str) -> AppResult<IndexRefreshStatus> {
        let _guard = self.lock("state.lock")?;
        let config = self.read_config()?;
        if scope != config.scope
            || config.candidate_semantic_config.is_none()
            || (!config.enabled && !config.index_maintenance_enabled)
            || config.paused
        {
            return Err(permission());
        }
        self.queue_index_locked(&config)?;
        Ok(self.index_status_locked(&config)?.unwrap())
    }
    /// 已持有 runner.lock。无 Vault/state 锁等待本机提供器；每次只推进一个有界批次。
    pub(super) fn maintain_index_once(&self) -> AppResult<bool> {
        let (config, path, mut record, partial) = {
            let _guard = self.lock("state.lock")?;
            let config = self.read_config()?;
            if (!config.enabled && !config.index_maintenance_enabled) || config.paused {
                return Ok(false);
            }
            let Some(path) = config.candidate_semantic_config.clone() else {
                return Ok(false);
            };
            let binding = self.binding(&config)?;
            let config_hash = Self::index_config_hash(&path).unwrap_or_default();
            let record = self
                .read_index_record()?
                .filter(|r| r.binding == binding && r.config_hash == config_hash)
                .unwrap_or(Record {
                    binding,
                    config_hash,
                    status: IndexRefreshStatus::default(),
                });
            if record.status.failures >= 3
                || record.status.retry_after.is_some_and(|at| at > Utc::now())
            {
                return Ok(false);
            }
            (
                config,
                path,
                record,
                self.directory()?.join("index-partial.json"),
            )
        };
        let context = Context::new(
            self.vault,
            Access::new(vec![config.scope.clone()]).map_err(storage)?,
        );
        let documents = {
            let guard = self.vault.read_guard().map_err(storage)?;
            context.memory_documents_locked(&guard).map_err(storage)?
        };
        let mut work = match Work::prepare(
            self.vault,
            &path,
            &config.scope,
            &documents,
            Utc::now(),
            &partial,
        ) {
            Ok(Some(work)) => work,
            Ok(None) => {
                let _guard = self.lock("state.lock")?;
                record.status.state = "unconfigured".into();
                record.status.message = "文档向量自动维护未配置；query-only 不批准语料编码".into();
                record.status.can_retry = false;
                self.save_index_record(&mut record)?;
                return Ok(false);
            }
            Err(error) => {
                self.index_failure(&mut record, error.0)?;
                return Ok(false);
            }
        };
        if work.already_current {
            let _guard = self.lock("state.lock")?;
            if self.binding(&self.read_config()?)? != record.binding || !work.config_current(&path)
            {
                return Ok(false);
            }
            let guard = self.vault.read_guard().map_err(storage)?;
            let fresh = context.memory_documents_locked(&guard).map_err(storage)?;
            if !work.same_generation(&fresh, Utc::now()) {
                return Ok(false);
            }
            record.status.state = "ready".into();
            record.status.message =
                "上次核验的 Memory 文档向量索引已就绪；读取仍实时检查权限".into();
            record.status.generation = Some(work.generation.clone());
            record.status.indexed_generation = Some(work.generation);
            record.status.total_documents = work.total;
            record.status.ready_documents = work.indexed;
            record.status.error_code = None;
            record.status.failures = 0;
            record.status.can_retry = false;
            record.status.retry_after = None;
            self.save_index_record(&mut record)?;
            return Ok(false);
        }
        {
            let _guard = self.lock("state.lock")?;
            if self.binding(&self.read_config()?)? != record.binding || self.read_config()?.paused {
                return Ok(false);
            }
            if !work.config_current(&path) {
                return Ok(false);
            }
            let guard = self.vault.read_guard().map_err(storage)?;
            let fresh = context.memory_documents_locked(&guard).map_err(storage)?;
            if !work.same_generation(&fresh, Utc::now()) {
                return Ok(false);
            }
            record.status.state = "running".into();
            record.status.message = "正在按独立批准增量编码；已批准记忆不等待索引成功".into();
            record.status.generation = Some(work.generation.clone());
            record.status.total_documents = work.total;
            record.status.ready_documents = work.indexed;
            record.status.attempts = record.status.attempts.saturating_add(1);
            record.status.can_retry = false;
            self.save_index_record(&mut record)?;
        }
        if let Err(error) = work.encode_step() {
            self.index_failure(&mut record, error.0)?;
            return Ok(true);
        }
        let _state = self.lock("state.lock")?;
        let current = self.read_config()?;
        if self.binding(&current)? != record.binding
            || (!current.enabled && !current.index_maintenance_enabled)
            || current.paused
            || !work.config_current(&path)
        {
            record.status.state = "pending".into();
            record.status.message = "配置或运行授权已改变；旧批次没有发布".into();
            record.status.error_code = Some("configuration_changed".into());
            self.save_index_record(&mut record)?;
            return Ok(true);
        }
        let guard = self.vault.read_guard().map_err(storage)?;
        let fresh = context.memory_documents_locked(&guard).map_err(storage)?;
        match work.persist(&fresh, Utc::now(), &partial) {
            Ok(complete) => {
                record.status.state = if complete { "ready" } else { "pending" }.into();
                record.status.message = if complete {
                    "当前代次文档向量已原子发布；Memory 正本未改动"
                } else {
                    "有界批次已保存；下轮或重启继续，半成品不用于检索"
                }
                .into();
                record.status.ready_documents = work.indexed;
                record.status.new_vectors = record.status.new_vectors.saturating_add(work.encoded);
                record.status.failures = 0;
                record.status.error_code = None;
                record.status.can_retry = false;
                record.status.retry_after = None;
                if complete {
                    record.status.indexed_generation = Some(work.generation);
                }
            }
            Err(error) if error.0 == "generation_mismatch" => {
                record.status.state = "pending".into();
                record.status.message =
                    "Memory/来源权限已改变；丢弃旧批次，下轮重算当前代次".into();
                record.status.error_code = Some(error.0.into());
                record.status.failures = 0;
            }
            Err(error) => Self::mark_index_failure(&mut record, error.0),
        }
        self.save_index_record(&mut record)?;
        Ok(true)
    }
    fn mark_index_failure(record: &mut Record, code: &str) {
        record.status.failures = record.status.failures.saturating_add(1);
        record.status.state = "failed".into();
        record.status.message =
            "文档向量未更新；已批准 Memory 与词法检索保留，可检查本机提供器后重试".into();
        record.status.error_code = Some(code.into());
        record.status.can_retry = true;
        record.status.retry_after =
            Some(Utc::now() + chrono::Duration::seconds(1_i64 << record.status.failures.min(5)));
    }
    fn index_failure(&self, record: &mut Record, code: &str) -> AppResult<()> {
        let _guard = self.lock("state.lock")?;
        Self::mark_index_failure(record, code);
        self.save_index_record(record)
    }
}
