//! 已授权的增量整理运行时。配置、任务、预算和结果缓存只保存在本机状态目录。
//! 原始捕获/检索不依赖此模块；模型只产出未信任提议，发布复用 Dream 事务。
use super::{AppError, AppResult, ErrorCode, JobPhase, JobState, JobStatus, JOB_SCHEMA};
use crate::{
    dream::{DreamJob, DreamOperation, DreamReceipt, DreamResult},
    filesystem::{file_identity, open_local_file, FileIdentity},
    model::{hash, validate_scope, Event, Evidence, MemoryState, Origin, Role},
    vault::{files_recursive, read_json, reject_symlink},
    Vault,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
};

const CONFIG_SCHEMA: &str = "recallcard.memory-runtime/1";
const MAX_RECORD_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MemoryProviderConfig {
    pub endpoint: String,
    pub model: String,
}

/// 配置模型不是数据授权。此记录须由人的本机设置动作提供，模型接口不能创建。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MemoryConsent {
    pub endpoint: String,
    pub model: String,
    pub scope: String,
    pub send_source_snapshots: bool,
    pub send_memory_snapshots: bool,
    pub auto_apply: bool,
    pub accepted_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MemoryBudget {
    pub max_calls_per_month: u64,
    pub max_reserved_tokens_per_month: u64,
    pub max_output_tokens_per_call: u64,
    pub max_request_bytes_per_call: u64,
}
impl Default for MemoryBudget {
    fn default() -> Self {
        Self {
            max_calls_per_month: 100,
            max_reserved_tokens_per_month: 1_000_000,
            max_output_tokens_per_call: 4096,
            max_request_bytes_per_call: 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MemoryConfig {
    pub schema: String,
    pub enabled: bool,
    pub paused: bool,
    pub scope: String,
    pub provider: Option<MemoryProviderConfig>,
    #[serde(default)]
    pub credential_storage: Option<super::credentials::CredentialStorage>,
    pub consent: Option<MemoryConsent>,
    pub budget: MemoryBudget,
    pub quiet_seconds: u64,
    pub batch_size: usize,
    pub max_projection_bytes: usize,
}
impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            schema: CONFIG_SCHEMA.into(),
            enabled: false,
            paused: false,
            scope: "personal".into(),
            provider: None,
            credential_storage: None,
            consent: None,
            budget: MemoryBudget::default(),
            quiet_seconds: 30,
            batch_size: 16,
            max_projection_bytes: 256 * 1024,
        }
    }
}
impl MemoryConfig {
    pub fn validate(&self) -> AppResult<()> {
        if self.schema != CONFIG_SCHEMA
            || validate_scope(&self.scope).is_err()
            || self.quiet_seconds > 86400
            || !(1..=64).contains(&self.batch_size)
            || !(4096..=512 * 1024).contains(&self.max_projection_bytes)
        {
            return Err(invalid("后台整理配置或投影限额无效"));
        }
        let b = &self.budget;
        if !(1..=1_000_000).contains(&b.max_calls_per_month)
            || b.max_reserved_tokens_per_month == 0
            || !(1..=131072).contains(&b.max_output_tokens_per_call)
            || !(4096..=4 * 1024 * 1024).contains(&b.max_request_bytes_per_call)
        {
            return Err(invalid("请求、输出或月度预算无效"));
        }
        if let Some(provider) = &self.provider {
            let address = provider.endpoint.strip_prefix("https://").unwrap_or("");
            let (host, path) = address.split_once('/').unwrap_or(("", ""));
            if host.is_empty()
                || path.is_empty()
                || provider.endpoint.len() > 2048
                || !provider.endpoint.is_ascii()
                || provider.endpoint.chars().any(|c| {
                    c.is_control() || c.is_whitespace() || matches!(c, '?' | '#' | '@' | '\\')
                })
                || provider.model.trim().is_empty()
                || provider.model.len() > 256
                || provider.model.chars().any(char::is_control)
            {
                return Err(invalid(
                    "请填写无凭据或查询参数的完整 HTTPS 模型地址与模型名称",
                ));
            }
        }
        if let Some(consent) = &self.consent {
            let Some(provider) = &self.provider else {
                return Err(permission());
            };
            if consent.endpoint != provider.endpoint
                || consent.model != provider.model
                || consent.scope != self.scope
                || !consent.send_source_snapshots
                || !consent.send_memory_snapshots
                || !consent.auto_apply
                || consent.accepted_at > Utc::now() + chrono::Duration::minutes(5)
            {
                return Err(permission());
            }
        }
        Ok(())
    }
    fn authorized(&self) -> AppResult<()> {
        self.validate()?;
        if !self.enabled || self.provider.is_none() || self.consent.is_none() {
            return Err(permission());
        }
        Ok(())
    }
    /// 暂停不更换输入身份；更改目的地、范围、策略或预算会使旧任务失效。
    fn binding(&self) -> AppResult<String> {
        let mut config = self.clone();
        config.paused = false;
        digest(&config)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct MemoryProgress {
    pub sources_selected: u64,
    pub memories_read: u64,
    pub sources_committed: u64,
    pub memories_committed: u64,
    pub sources_skipped: u64,
    pub provider_calls: u64,
    pub reserved_tokens: u64,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub source_cursor: Option<String>,
    pub receipt_id: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct MemoryUsage {
    pub month: String,
    pub reserved_calls: u64,
    pub reserved_tokens: u64,
    pub reported_input_tokens: u64,
    pub reported_output_tokens: u64,
    pub calls_with_unknown_usage: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryRuntimeStatus {
    pub schema: String,
    pub state: String,
    pub message: String,
    pub config: MemoryConfig,
    pub usage: MemoryUsage,
    pub jobs: Vec<JobStatus<MemoryProgress>>,
    pub raw_search_available: bool,
    pub oversized_sources: u64,
    pub non_current_sources_skipped: u64,
    pub budget_note: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct DiscoveryStatus {
    scope: String,
    oversized_sources: u64,
    #[serde(default)]
    non_current_sources_skipped: u64,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MemoryJobControl {
    Pause,
    Resume,
    Retry,
    Cancel,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ProviderUsage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub request_bytes: u64,
    pub network_calls: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderOutput {
    pub result: DreamResult,
    pub usage: ProviderUsage,
    pub verification: String,
}

/// 实际 worker 与合成 fake 共用协议。实现不能修改 Vault 或扩大配置权限。
pub trait MemoryProvider {
    fn credential_status(&self, _config: &MemoryConfig) -> super::credentials::CredentialStatus {
        super::credentials::CredentialStatus::default()
    }
    fn available(&self, _config: &MemoryConfig) -> AppResult<()> {
        Ok(())
    }
    fn execute(&mut self, config: &MemoryConfig, job: &DreamJob) -> AppResult<ProviderOutput>;
}
#[derive(Debug, Clone, Serialize, Deserialize)]
struct MemoryJobRecord {
    status: JobStatus<MemoryProgress>,
    config_hash: String,
    projection: DreamJob,
    result: Option<ProviderOutput>,
    dispatch_started: bool,
    receipt: Option<DreamReceipt>,
    #[serde(default)]
    reservation_month: Option<String>,
    #[serde(default)]
    paused_by_config: bool,
    #[serde(default)]
    superseded_projection: bool,
}
// 列表与调度只保留轻量身份；不把全部已完成任务的证据正文重新装入内存。
#[derive(Deserialize)]
struct MemoryJobSummary {
    status: JobStatus<MemoryProgress>,
    config_hash: String,
    projection: ProjectionSummary,
    #[serde(default)]
    paused_by_config: bool,
    #[serde(default)]
    superseded_projection: bool,
}
#[derive(Deserialize)]
struct ProjectionSummary {
    source_refs: Vec<SourceSummary>,
}
#[derive(Deserialize)]
struct SourceSummary {
    event: EventIdentity,
}
#[derive(Deserialize)]
struct EventIdentity {
    id: String,
}
#[derive(Serialize, Deserialize)]
struct MemoryConfigRecord {
    config: MemoryConfig,
    root_identity: FileIdentity,
    marker: FileIdentity,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryJobReview {
    pub job: JobStatus<MemoryProgress>,
    pub review: Option<crate::dream::DreamReview>,
    pub note: String,
}
struct RuntimeGuard(File);
impl Drop for RuntimeGuard {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

pub struct MemoryRuntime<'a> {
    vault: &'a Vault,
}
impl<'a> MemoryRuntime<'a> {
    pub fn new(vault: &'a Vault) -> Self {
        Self { vault }
    }
    fn directory(&self) -> AppResult<PathBuf> {
        let dir = self
            .vault
            .state_dir()
            .map_err(storage)?
            .join("background-memory");
        reject_symlink(&dir).map_err(storage)?;
        fs::create_dir_all(dir.join("jobs")).map_err(storage)?;
        reject_symlink(&dir.join("jobs")).map_err(storage)?;
        Ok(dir)
    }
    fn lock(&self, name: &str) -> AppResult<RuntimeGuard> {
        let path = self.directory()?.join(name);
        reject_symlink(&path).map_err(storage)?;
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
        }
        let file = options.open(path).map_err(storage)?;
        if !file.metadata().map_err(storage)?.is_file() {
            return Err(storage("invalid lease"));
        }
        file.try_lock().map_err(|_| {
            AppError::new(
                ErrorCode::Conflict,
                "另一进程正在处理后台整理",
                "稍后读取同一任务；不要重复启动",
            )
        })?;
        Ok(RuntimeGuard(file))
    }
    fn vault_identity(&self) -> AppResult<(FileIdentity, FileIdentity)> {
        Ok((
            file_identity(&open_local_file(self.vault.root()).map_err(storage)?)
                .map_err(storage)?,
            file_identity(
                &open_local_file(&self.vault.root().join("control/schema-version.json"))
                    .map_err(storage)?,
            )
            .map_err(storage)?,
        ))
    }
    fn binding(&self, config: &MemoryConfig) -> AppResult<String> {
        digest(&(config.binding()?, self.vault_identity()?))
    }
    fn read_config(&self) -> AppResult<MemoryConfig> {
        let path = self.directory()?.join("config.json");
        if !path.exists() {
            return Ok(MemoryConfig::default());
        }
        let record: MemoryConfigRecord = bounded_read(&path)?;
        let (root_identity, marker) = self.vault_identity()?;
        if record.root_identity != root_identity || record.marker != marker {
            return Err(AppError::new(
                ErrorCode::PermissionDenied,
                "资料库文件身份已经改变，旧的后台授权已失效",
                "重新打开并确认当前资料库和范围；不会沿用旧目的地授权",
            ));
        }
        record.config.validate()?;
        Ok(record.config)
    }
    pub fn configuration(&self) -> AppResult<MemoryConfig> {
        let _guard = self.lock("state.lock")?;
        self.read_config()
    }
    pub fn configure(&self, config: MemoryConfig) -> AppResult<MemoryRuntimeStatus> {
        config.validate()?;
        let guard = self.lock("state.lock")?;
        let (root_identity, marker) = self.vault_identity()?;
        self.vault
            .write_replace(
                &self.directory()?.join("config.json"),
                &MemoryConfigRecord {
                    config: config.clone(),
                    root_identity,
                    marker,
                },
            )
            .map_err(storage)?;
        if !config.paused && config.authorized().is_ok() {
            for summary in self.records()? {
                if summary.paused_by_config
                    && summary.status.state == JobState::Paused
                    && summary.config_hash == self.binding(&config)?
                {
                    let mut record = self.load(&summary.status.job_id)?;
                    record.paused_by_config = false;
                    record.status.state = JobState::Queued;
                    record.status.error = None;
                    self.save(&mut record)?;
                }
            }
        }
        drop(guard);
        self.status()
    }
    pub fn status(&self) -> AppResult<MemoryRuntimeStatus> {
        let _guard = self.lock("state.lock")?;
        let config = self.read_config()?;
        let jobs = self
            .records()?
            .into_iter()
            .map(|r| r.status)
            .collect::<Vec<_>>();
        let scan_path = self.directory()?.join("discovery.json");
        let scan: DiscoveryStatus = if scan_path.exists() {
            bounded_read(&scan_path)?
        } else {
            DiscoveryStatus::default()
        };
        let oversized_sources = if scan.scope == config.scope {
            scan.oversized_sources
        } else {
            0
        };
        let non_current_sources_skipped = if scan.scope == config.scope {
            scan.non_current_sources_skipped
        } else {
            0
        };
        let (state, message) = if !config.enabled || config.provider.is_none() {
            ("unconfigured", "已保存的来源可搜索；后台整理尚未启用")
        } else if config.consent.is_none() {
            (
                "needs_input",
                "模型已配置；尚未授权向指定地址发送所选范围的来源和旧记忆",
            )
        } else if config.paused {
            ("paused", "后台整理已暂停；原始捕获与搜索不受影响")
        } else if oversized_sources > 0 {
            (
                "needs_input",
                "部分来源超过单次投影限制，未截断也未发送；原文仍可搜索",
            )
        } else if jobs.iter().any(|j| j.state == JobState::NeedsInput) {
            ("needs_input", "部分整理任务需要处理；其他来源仍可搜索")
        } else if jobs.iter().any(|j| j.state == JobState::Failed) {
            (
                "failed",
                "部分整理任务失败；可查看原因并重试，原始来源未丢失",
            )
        } else {
            (
                "ready",
                "后台整理配置已就绪；运行服务启动后会增量处理新来源",
            )
        };
        Ok(MemoryRuntimeStatus { schema: CONFIG_SCHEMA.into(), state: state.into(), message: message.into(), config, usage: self.usage()?, jobs, raw_search_available: self.vault.read_guard().is_ok(), oversized_sources, non_current_sources_skipped, budget_note: "请求次数与输出上限是本机限制；预留 token 按字节保守估计，供应商用量不是账单，不保证货币费用上限。未知用量不退还预留。".into() })
    }
    pub fn status_for_scope(&self, scope: &str) -> AppResult<MemoryRuntimeStatus> {
        validate_scope(scope).map_err(|_| invalid("资料范围无效"))?;
        let mut status = self.status()?;
        status.jobs.retain(|job| job.scope == scope);
        if status.config.scope != scope {
            status.config = MemoryConfig {
                scope: scope.into(),
                ..Default::default()
            };
            status.oversized_sources = 0;
            status.non_current_sources_skipped = 0;
            status.state = "unconfigured".into();
            status.message =
                "此范围的后台整理尚未启用；当前版本每个资料库仅支持一个活动整理范围".into();
        }
        Ok(status)
    }
    pub fn jobs(&self) -> AppResult<Vec<JobStatus<MemoryProgress>>> {
        Ok(self.status()?.jobs)
    }
    fn records(&self) -> AppResult<Vec<MemoryJobSummary>> {
        let mut records = Vec::new();
        for path in files_recursive(&self.directory()?.join("jobs"), "json").map_err(storage)? {
            let record: MemoryJobSummary = bounded_read(&path)?;
            if path.file_stem().and_then(|s| s.to_str()) != Some(&record.status.job_id)
                || record.status.kind != "memory"
                || record.status.schema != JOB_SCHEMA
            {
                return Err(storage("invalid record"));
            }
            records.push(record);
        }
        records.sort_by(|a, b| {
            a.status
                .created_at
                .cmp(&b.status.created_at)
                .then(a.status.job_id.cmp(&b.status.job_id))
        });
        Ok(records)
    }
    fn job_path(&self, id: &str) -> AppResult<PathBuf> {
        if !id.starts_with("memory_")
            || id.len() != 71
            || !id[7..].bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err(invalid("整理任务编号无效"));
        }
        Ok(self.directory()?.join("jobs").join(format!("{id}.json")))
    }
    fn save(&self, record: &mut MemoryJobRecord) -> AppResult<()> {
        record.status.updated_at = Utc::now();
        self.vault
            .write_replace(&self.job_path(&record.status.job_id)?, record)
            .map_err(storage)
    }
    fn load(&self, id: &str) -> AppResult<MemoryJobRecord> {
        bounded_read(&self.job_path(id)?)
    }
    fn usage(&self) -> AppResult<MemoryUsage> {
        let month = Utc::now().format("%Y-%m").to_string();
        self.usage_for_month(&month)
    }
    fn usage_for_month(&self, month: &str) -> AppResult<MemoryUsage> {
        if month.len() != 7
            || month.as_bytes()[4] != b'-'
            || !month
                .bytes()
                .enumerate()
                .all(|(i, b)| i == 4 || b.is_ascii_digit())
        {
            return Err(storage("invalid month"));
        }
        let path = self.directory()?.join(format!("usage-{month}.json"));
        if path.exists() {
            bounded_read(&path)
        } else {
            Ok(MemoryUsage {
                month: month.into(),
                ..Default::default()
            })
        }
    }
    fn save_usage(&self, usage: &MemoryUsage) -> AppResult<()> {
        self.vault
            .write_replace(
                &self
                    .directory()?
                    .join(format!("usage-{}.json", usage.month)),
                usage,
            )
            .map_err(storage)
    }
    pub fn review(&self, id: &str, scope: &str) -> AppResult<MemoryJobReview> {
        validate_scope(scope).map_err(|_| invalid("资料范围无效"))?;
        let _guard = self.lock("state.lock")?;
        let record = self.load(id)?;
        if record.status.scope != scope {
            return Err(permission());
        }
        self.revalidate_projection(&record.projection)?;
        let review = record
            .result
            .as_ref()
            .map(|output| {
                self.vault.dream_review(&output.result).map_err(|_| {
                    AppError::new(
                        ErrorCode::InvalidRequest,
                        "候选目前不符合读取或证据校验边界",
                        "按任务原因重新整理；不会显示已撤权的旧快照",
                    )
                })
            })
            .transpose()?;
        Ok(MemoryJobReview {
            job: record.status,
            review,
            note: "候选尚需结合证据判断；模型评分不能证明事实。修正正文或标签后，该记忆由你维护。"
                .into(),
        })
    }
    pub fn control(
        &self,
        id: &str,
        action: MemoryJobControl,
    ) -> AppResult<JobStatus<MemoryProgress>> {
        let _guard = self.lock("state.lock")?;
        let mut record = self.load(id)?;
        match action {
            MemoryJobControl::Pause
                if !matches!(
                    record.status.state,
                    JobState::Completed | JobState::Cancelled
                ) =>
            {
                record.status.state = JobState::Paused;
                record.paused_by_config = false;
            }
            MemoryJobControl::Cancel if record.status.state != JobState::Completed => {
                record.status.state = JobState::Cancelled
            }
            MemoryJobControl::Resume if record.status.state == JobState::Paused => {
                record.status.state = JobState::Queued;
                record.paused_by_config = false;
            }
            MemoryJobControl::Retry
                if matches!(
                    record.status.state,
                    JobState::Failed | JobState::NeedsInput | JobState::Cancelled
                ) =>
            {
                let config = self.read_config()?;
                config.authorized()?;
                if record.projection.allowed_scope != config.scope
                    || record.config_hash != self.binding(&config)?
                {
                    return Err(AppError::new(
                        ErrorCode::PermissionDenied,
                        "任务所用配置已经改变，不能沿用旧授权重试",
                        "恢复原配置或在已确认的新范围重新整理",
                    ));
                }
                let has_receipt = self.receipt_exists(&record.projection)?;
                if !has_receipt && self.revalidate_projection(&record.projection).is_err() {
                    // 下一 tick 用当前证据重新建立输入身份，绝不修改旧任务快照。
                    record.superseded_projection = true;
                    record.status.state = JobState::Cancelled;
                    record.status.error = Some(AppError::new(
                        ErrorCode::SourceChanged,
                        "旧输入已失效，已退出原任务；将按当前有效来源重新排队",
                        "被遗忘或撤权的来源不会再次进入队列",
                    ));
                } else {
                    if !has_receipt
                        && record
                            .status
                            .error
                            .as_ref()
                            .is_some_and(|error| error.code == ErrorCode::InvalidRequest)
                    {
                        record.result = None;
                    }
                    record.dispatch_started = false;
                    record.status.error = None;
                    record.paused_by_config = false;
                    record.status.state = JobState::Queued;
                }
            }
            _ => return Err(invalid("当前任务状态不支持此操作")),
        }
        record.status.can_resume = matches!(
            record.status.state,
            JobState::Paused | JobState::Failed | JobState::NeedsInput
        );
        self.save(&mut record)?;
        Ok(record.status)
    }
    /// 单实例每次推进一个有界批次。服务循环调用；不持有 Vault 锁等待网络。
    pub fn tick<P: MemoryProvider + ?Sized>(
        &self,
        provider: &mut P,
    ) -> AppResult<Option<JobStatus<MemoryProgress>>> {
        let _runner = self.lock("runner.lock")?;
        if self
            .vault
            .state_dir()
            .map_err(storage)?
            .join("dream-transaction.json")
            .exists()
        {
            self.vault.dream_recover().map_err(|_| {
                AppError::new(
                    ErrorCode::Conflict,
                    "已批准的记忆事务需要恢复",
                    "保留资料与事务文件，检查来源状态后恢复",
                )
            })?;
        }
        let mut record = {
            let _state = self.lock("state.lock")?;
            let config = self.read_config()?;
            if !config.enabled || config.paused || config.authorized().is_err() {
                return Ok(None);
            }
            let mut records = self.records()?;
            let binding = self.binding(&config)?;
            if provider.available(&config).is_ok() {
                for summary in &records {
                    if summary.config_hash == binding
                        && summary.status.state == JobState::NeedsInput
                        && summary.status.progress.provider_calls == 0
                        && summary
                            .status
                            .error
                            .as_ref()
                            .is_some_and(|error| error.code == ErrorCode::ModelUnavailable)
                    {
                        let mut record = self.load(&summary.status.job_id)?;
                        record.status.state = JobState::Queued;
                        record.status.error = None;
                        self.save(&mut record)?;
                    }
                }
                records = self.records()?;
            }
            if let Some(record) = records
                .iter()
                .find(|r| matches!(r.status.state, JobState::Queued | JobState::Running))
            {
                self.load(&record.status.job_id)?
            } else {
                // 连接/额度故障是全局阻塞，不能按下一批来源不断消耗调用预算。
                let binding = self.binding(&config)?;
                if records.iter().any(|record| {
                    record.config_hash == binding
                        && matches!(record.status.state, JobState::NeedsInput | JobState::Failed)
                        && record.status.error.as_ref().is_some_and(|error| {
                            matches!(
                                error.code,
                                ErrorCode::ModelUnavailable
                                    | ErrorCode::BudgetExceeded
                                    | ErrorCode::InvalidRequest
                            ) || record.status.phase == JobPhase::Executing
                        })
                }) {
                    return Ok(None);
                }
                let Some(record) = self.discover(&config, &records)? else {
                    return Ok(None);
                };
                record
            }
        };
        let result = self.advance(&mut record, provider);
        match result {
            Ok(()) => Ok(Some(record.status)),
            Err(error) => {
                let _guard = self.lock("state.lock")?;
                // 暂停或取消可能发生在 provider 调用期间；保留控制状态。
                let current = self.load(&record.status.job_id)?;
                if matches!(current.status.state, JobState::Paused | JobState::Cancelled) {
                    record.status.state = current.status.state;
                    record.paused_by_config = current.paused_by_config;
                } else if self.read_config()?.paused {
                    record.status.state = JobState::Paused;
                    record.paused_by_config = true;
                } else {
                    record.status.state = if matches!(
                        error.code,
                        ErrorCode::PermissionDenied
                            | ErrorCode::Conflict
                            | ErrorCode::SourceChanged
                            | ErrorCode::ResourceLimit
                            | ErrorCode::BudgetExceeded
                            | ErrorCode::ModelUnavailable
                    ) {
                        JobState::NeedsInput
                    } else {
                        JobState::Failed
                    };
                }
                record.status.error = Some(error);
                record.status.can_resume = record.status.state != JobState::Cancelled;
                self.save(&mut record)?;
                Ok(Some(record.status))
            }
        }
    }
    fn checkpoint(&self, record: &MemoryJobRecord) -> AppResult<MemoryConfig> {
        let config = self.read_config()?;
        config.authorized()?;
        if config.paused
            || self
                .vault
                .state_dir()
                .map_err(storage)?
                .join("runtime/stop.json")
                .exists()
            || matches!(
                self.load(&record.status.job_id)?.status.state,
                JobState::Paused | JobState::Cancelled
            )
        {
            return Err(AppError::new(
                ErrorCode::Cancelled,
                "整理已暂停或取消，已提交内容保持不变",
                "恢复后继续未完成阶段",
            ));
        }
        if record.projection.allowed_scope != config.scope
            || record.config_hash != self.binding(&config)?
        {
            return Err(permission());
        }
        Ok(config)
    }
    fn advance<P: MemoryProvider + ?Sized>(
        &self,
        record: &mut MemoryJobRecord,
        provider: &mut P,
    ) -> AppResult<()> {
        if record.dispatch_started && record.result.is_none() {
            return Err(AppError::new(
                ErrorCode::Conflict,
                "上次模型调用中断，是否产生费用未知；未自动重发",
                "检查用量后重试；重试将占用新的请求预算",
            ));
        }
        if record.result.is_none() {
            let config = {
                let _state = self.lock("state.lock")?;
                let config = self.checkpoint(record)?;
                self.revalidate_projection(&record.projection)?;
                provider.available(&config)?;
                let mut usage = self.usage()?;
                let reserved = (serde_json::to_vec(&record.projection)
                    .map_err(storage)?
                    .len() as u64)
                    .saturating_mul(2)
                    .saturating_add(16_384)
                    .saturating_add(config.budget.max_output_tokens_per_call);
                if usage.reserved_calls >= config.budget.max_calls_per_month
                    || usage.reserved_tokens.saturating_add(reserved)
                        > config.budget.max_reserved_tokens_per_month
                {
                    return Err(AppError::new(
                        ErrorCode::BudgetExceeded,
                        "本月后台整理请求或预留 token 预算已用尽",
                        "调整预算或下月重试；原始资料仍可搜索",
                    ));
                }
                // 先预留，崩溃不得使已发请求成为免费重试。
                usage.reserved_calls += 1;
                usage.reserved_tokens += reserved;
                usage.calls_with_unknown_usage += 1;
                self.save_usage(&usage)?;
                record.dispatch_started = true;
                record.reservation_month = Some(usage.month.clone());
                record.status.state = JobState::Running;
                record.status.phase = JobPhase::Executing;
                record.status.progress.provider_calls += 1;
                record.status.progress.reserved_tokens += reserved;
                self.save(record)?;
                config
            };
            let output = provider
                .execute(&config, &record.projection)
                .map_err(|error| {
                    let mut safe = AppError::new(
                        error.code,
                        "模型整理未返回可用的完整结果；未记录供应商正文或凭据",
                        "查看模型连接与额度后重试；已预留预算不会自动退还",
                    );
                    safe.retryable = true;
                    safe
                })?;
            if output.result.job_id != record.projection.job_id
                || output.result.input_hash != record.projection.input_hash
                || output.usage.network_calls > 1
                || output
                    .usage
                    .output_tokens
                    .is_some_and(|n| n > config.budget.max_output_tokens_per_call)
                || output.usage.request_bytes > config.budget.max_request_bytes_per_call
                || serde_json::to_vec(&output).map_err(storage)?.len() > 2 * 1024 * 1024
            {
                return Err(invalid("模型结果身份或用量超出本次协议边界"));
            }
            let _state = self.lock("state.lock")?;
            let mut usage = self.usage_for_month(
                record
                    .reservation_month
                    .as_deref()
                    .ok_or_else(|| storage("missing reservation"))?,
            )?;
            usage.reported_input_tokens = usage
                .reported_input_tokens
                .saturating_add(output.usage.input_tokens.unwrap_or(0));
            usage.reported_output_tokens = usage
                .reported_output_tokens
                .saturating_add(output.usage.output_tokens.unwrap_or(0));
            if output.usage.input_tokens.is_some() && output.usage.output_tokens.is_some() {
                usage.calls_with_unknown_usage = usage.calls_with_unknown_usage.saturating_sub(1);
            }
            self.save_usage(&usage)?;
            record.status.progress.input_tokens = output.usage.input_tokens;
            record.status.progress.output_tokens = output.usage.output_tokens;
            record.result = Some(output);
            record.status.phase = JobPhase::Validating;
            let current = self.load(&record.status.job_id)?;
            if matches!(current.status.state, JobState::Paused | JobState::Cancelled) {
                record.status.state = current.status.state;
                record.paused_by_config = current.paused_by_config;
            }
            self.save(record)?;
        }
        let _state = self.lock("state.lock")?;
        self.checkpoint(record)?;
        let result = &record
            .result
            .as_ref()
            .ok_or_else(|| storage("missing result"))?
            .result;
        // 已有收据优先恢复，不因成功发布改变了 read-set 而再调用模型。
        if !self.receipt_exists(&record.projection)? {
            self.revalidate_projection(&record.projection)?;
        }
        let review = self.vault.dream_review(result).map_err(|_| {
            AppError::new(
                ErrorCode::InvalidRequest,
                "候选的来源、证据、范围或结果绑定不符合发布协议；没有自动覆盖",
                "检查候选后重试；无效候选会重新提取并占用新的请求预算",
            )
        })?;
        if !review.already_applied {
            self.revalidate_projection(&record.projection)?;
            if !review.can_apply
                || review.requires_protected_approval
                || requires_decision(&record.projection, result)
            {
                return Err(AppError::new(
                    ErrorCode::Conflict,
                    "提议包含冲突、用户维护的记忆、受保护更改或缺乏依据的时间",
                    "在记忆中核对并纠正；后台不会替你批准这些更改",
                ));
            }
        }
        record.status.phase = JobPhase::Committing;
        self.save(record)?;
        let result = &record
            .result
            .as_ref()
            .ok_or_else(|| storage("missing result"))?
            .result;
        let receipt = self
            .vault
            .dream_apply(result, &review.result_hash, false)
            .map_err(|_| {
                AppError::new(
                    ErrorCode::Conflict,
                    "记忆发布未确认完成；重试会先检查已提交收据",
                    "保留资料库与任务，恢复同一任务；不要删除事务记录",
                )
            })?;
        record.status.progress.sources_committed = receipt.source_coverage.len() as u64;
        record.status.progress.memories_committed = receipt.changes.len() as u64;
        record.status.progress.receipt_id = Some(receipt.job_id.clone());
        record.receipt = Some(receipt);
        record.status.state = JobState::Completed;
        record.status.phase = JobPhase::Finished;
        record.status.error = None;
        record.status.can_resume = false;
        self.save(record)
    }
    fn receipt_exists(&self, job: &DreamJob) -> AppResult<bool> {
        if !job.job_id.starts_with("dream_")
            || job.job_id.len() != 70
            || !job.job_id[6..]
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        {
            return Err(invalid("整理快照身份无效"));
        }
        let path = self
            .vault
            .root()
            .join("control/dream-receipts")
            .join(format!("{}.json", job.job_id));
        reject_symlink(&path).map_err(storage)?;
        Ok(path.exists())
    }
    fn revalidate_projection(&self, job: &DreamJob) -> AppResult<()> {
        if job
            .source_refs
            .iter()
            .any(|source| !current_branch(&source.event))
        {
            return Err(AppError::new(
                ErrorCode::SourceChanged,
                "任务包含未选择的替代分支，不能自动整理为当前事实",
                "原文仍可搜索；按当前有效分支重新排队",
            ));
        }
        let sources = job
            .source_refs
            .iter()
            .map(|s| s.event.id.clone())
            .collect::<Vec<_>>();
        let memories = job
            .memory_read_set
            .iter()
            .map(|m| m.reference.clone())
            .collect::<Vec<_>>();
        let current = self
            .vault
            .dream_export(&sources, &memories, &job.allowed_scope)
            .map_err(|_| {
                AppError::new(
                    ErrorCode::SourceChanged,
                    "来源或旧记忆已撤权、修订或不可读取",
                    "重新核对资料范围；旧任务不会继续发送或覆盖",
                )
            })?;
        if current.input_hash != job.input_hash {
            return Err(AppError::new(
                ErrorCode::SourceChanged,
                "整理输入已改变",
                "按新来源重新整理",
            ));
        }
        let _guard = self.vault.read_guard().map_err(storage)?;
        let ids: BTreeMap<_, _> = job
            .source_refs
            .iter()
            .map(|source| (source.event.id.clone(), source.event.data.revision_key()))
            .collect();
        let mut revised = false;
        scan_events(self.vault, |event| {
            if event.data.revision_of.as_ref().is_some_and(|id| {
                ids.get(id)
                    .is_some_and(|key| *key == event.data.revision_key())
            }) {
                revised = true;
            }
            Ok(())
        })?;
        if revised {
            return Err(AppError::new(
                ErrorCode::SourceChanged,
                "整理来源已有新修订",
                "旧版本不会被重新提炼；使用最新来源",
            ));
        }
        Ok(())
    }
    fn discover(
        &self,
        config: &MemoryConfig,
        records: &[MemoryJobSummary],
    ) -> AppResult<Option<MemoryJobRecord>> {
        let (sources, memories, skipped) = {
            let _guard = self.vault.read_guard().map_err(storage)?;
            let suppressed = self.vault.suppressed_ids().map_err(storage)?;
            let mut covered = BTreeSet::new();
            for record in records {
                if (!record.superseded_projection && record.config_hash == self.binding(config)?)
                    || record.status.state == JobState::Completed
                {
                    for source in &record.projection.source_refs {
                        covered.insert(source.event.id.clone());
                    }
                }
            }
            // 收据是已提交证据覆盖的事实；状态目录丢失也不能重复应用同一来源。
            for path in files_recursive(&self.vault.root().join("control/dream-receipts"), "json")
                .map_err(storage)?
            {
                let receipt: DreamReceipt = bounded_read(&path)?;
                for source in receipt.source_coverage {
                    covered.insert(source.strip_prefix("event:").unwrap_or(&source).to_string());
                }
            }
            let mut identities = BTreeMap::new();
            scan_events(self.vault, |event| {
                if event.data.scope == config.scope {
                    identities.insert(event.id, event.data.revision_key());
                }
                Ok(())
            })?;
            let mut revisions = BTreeSet::new();
            scan_events(self.vault, |event| {
                if let Some(previous) = &event.data.revision_of {
                    if identities
                        .get(previous)
                        .is_some_and(|key| *key == event.data.revision_key())
                    {
                        revisions.insert(previous.clone());
                    }
                }
                Ok(())
            })?;
            drop(identities);
            let mut selected = Vec::new();
            let mut bytes = 4096;
            let mut skipped = 0;
            let mut non_current_sources_skipped = 0;
            scan_events(self.vault, |event| {
                if event.data.scope != config.scope
                    || covered.contains(&event.id)
                    || suppressed.contains(&event.id)
                    || revisions.contains(&event.id)
                    || !event.data.has_original_evidence()
                    || is_echo(&event)
                {
                    return Ok(());
                }
                if !current_branch(&event) {
                    non_current_sources_skipped += 1;
                    return Ok(());
                }
                let size = serde_json::to_vec_pretty(&event).map_err(storage)?.len() + 256;
                if size + 4096 > config.max_projection_bytes {
                    skipped += 1;
                    return Ok(());
                }
                if selected.len() < config.batch_size && bytes + size <= config.max_projection_bytes
                {
                    bytes += size;
                    selected.push(event);
                }
                Ok(())
            })?;
            self.vault
                .write_replace(
                    &self.directory()?.join("discovery.json"),
                    &DiscoveryStatus {
                        scope: config.scope.clone(),
                        oversized_sources: skipped,
                        non_current_sources_skipped,
                    },
                )
                .map_err(storage)?;
            if selected.is_empty() {
                return Ok(None);
            }
            if selected.len() < config.batch_size
                && selected
                    .iter()
                    .map(|e| e.captured_at)
                    .max()
                    .is_some_and(|last| {
                        Utc::now().signed_duration_since(last).num_seconds()
                            < config.quiet_seconds as i64
                    })
            {
                return Ok(None);
            }
            let source_ids: BTreeSet<_> = selected.iter().map(|e| e.id.clone()).collect();
            let terms: BTreeSet<_> = selected
                .iter()
                .flat_map(|e| text_terms(&e.data.text()))
                .collect();
            let mut related = Vec::new();
            for path in
                files_recursive(&self.vault.root().join("memories"), "md").map_err(storage)?
            {
                let id = path
                    .file_stem()
                    .and_then(|name| name.to_str())
                    .ok_or_else(|| storage("invalid memory"))?;
                let memory = self.vault.memory(id).map_err(storage)?;
                if memory.data.scope != config.scope
                    || !matches!(memory.state, MemoryState::Active | MemoryState::Tentative)
                    || suppressed.contains(&memory.id)
                    || memory
                        .data
                        .source_refs
                        .iter()
                        .any(|id| suppressed.contains(id))
                {
                    continue;
                }
                let score = text_terms(&memory.data.content)
                    .intersection(&terms)
                    .count()
                    + 100
                        * memory
                            .data
                            .source_refs
                            .iter()
                            .filter(|id| source_ids.contains(*id))
                            .count();
                if score > 0 {
                    related.push((score, memory));
                    related.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.id.cmp(&b.1.id)));
                    related.truncate(32);
                }
            }
            related.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.id.cmp(&b.1.id)));
            let mut memories = Vec::new();
            for (_, memory) in related {
                let size = serde_json::to_vec_pretty(&memory).map_err(storage)?.len() + 256;
                if memories.len() < 32 && bytes + size <= config.max_projection_bytes {
                    bytes += size;
                    memories.push(memory.id);
                }
            }
            (selected, memories, skipped + non_current_sources_skipped)
        };
        let projection = self
            .vault
            .dream_export(
                &sources.iter().map(|e| e.id.clone()).collect::<Vec<_>>(),
                &memories,
                &config.scope,
            )
            .map_err(|_| {
                AppError::new(
                    ErrorCode::SourceChanged,
                    "本批来源在投影期间改变或超过任务限制",
                    "稍后从最新来源重试；没有向模型发送数据",
                )
            })?;
        let config_hash = self.binding(config)?;
        let request_id = digest(&(config_hash.as_str(), projection.input_hash.as_str()))?;
        let now = Utc::now();
        let mut record = MemoryJobRecord {
            status: JobStatus {
                schema: JOB_SCHEMA.into(),
                job_id: format!("memory_{request_id}"),
                request_id,
                kind: "memory".into(),
                scope: config.scope.clone(),
                state: JobState::Queued,
                phase: JobPhase::Preparing,
                progress: MemoryProgress {
                    sources_selected: projection.source_refs.len() as u64,
                    memories_read: projection.memory_read_set.len() as u64,
                    sources_skipped: skipped,
                    source_cursor: sources.last().map(|e| e.id.clone()),
                    ..Default::default()
                },
                error: None,
                can_resume: false,
                created_at: now,
                updated_at: now,
            },
            config_hash,
            projection,
            result: None,
            dispatch_started: false,
            receipt: None,
            reservation_month: None,
            paused_by_config: false,
            superseded_projection: false,
        };
        self.save(&mut record)?;
        Ok(Some(record))
    }
}

fn current_branch(event: &Event) -> bool {
    event.data.metadata.pointer("/chatgpt/on_current_path") != Some(&serde_json::Value::Bool(false))
}
fn is_echo(event: &Event) -> bool {
    matches!(
        event.data.origin,
        Origin::ContextInjection | Origin::RecallcardDreamJob
    ) || event.data.parts.iter().any(|p| {
        matches!(
            p.origin,
            Origin::ContextInjection | Origin::RecallcardDreamJob
        )
    })
}
fn requires_decision(job: &DreamJob, result: &DreamResult) -> bool {
    result.proposals.iter().any(|p| {
        if matches!(p.operation, DreamOperation::Noop | DreamOperation::Conflict) {
            return p.operation == DreamOperation::Conflict;
        }
        let sources = job
            .source_refs
            .iter()
            .filter(|s| {
                p.source_refs
                    .iter()
                    .any(|id| id.strip_prefix("event:").unwrap_or(id) == s.event.id)
            })
            .collect::<Vec<_>>();
        let unknown_time = sources.iter().all(|s| s.event.data.occurred_at.is_none());
        let invented_time = (unknown_time
            && (p.observed_at.is_some() || p.valid_from.is_some() || p.valid_to.is_some()))
            || p.observed_at
                .is_some_and(|at| !sources.iter().any(|s| s.event.data.occurred_at == Some(at)));
        let user_owned = p.target_ref.as_ref().is_some_and(|target| {
            job.memory_read_set.iter().any(|old| {
                (old.reference == *target
                    || target
                        .strip_prefix("memory:")
                        .unwrap_or(target)
                        .split('@')
                        .next()
                        == Some(old.memory.id.as_str()))
                    && old.memory.data.authority != "dream"
            })
        });
        let mixed_provenance = match p.evidence {
            Evidence::UserExplicit => sources.iter().any(|source| {
                source.event.data.role != Role::User
                    || source
                        .event
                        .data
                        .parts
                        .iter()
                        .any(|part| !matches!(part.origin, Origin::UserInput | Origin::Native))
            }),
            Evidence::Observed => sources.iter().any(|source| {
                !matches!(source.event.data.role, Role::User | Role::Tool)
                    || source.event.data.parts.iter().any(|part| {
                        !matches!(
                            part.origin,
                            Origin::UserInput | Origin::Native | Origin::ToolOutput
                        )
                    })
            }),
            Evidence::AssistantSuggestion => false,
        };
        mixed_provenance
            || invented_time
            || user_owned
            || (p.operation == DreamOperation::Supersede
                && p.evidence == Evidence::AssistantSuggestion)
    })
}
fn text_terms(text: &str) -> BTreeSet<String> {
    let chars = text
        .chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .take(8192)
        .collect::<Vec<_>>();
    chars.windows(2).map(|pair| pair.iter().collect()).collect()
}
fn scan_events<F: FnMut(Event) -> AppResult<()>>(vault: &Vault, mut visit: F) -> AppResult<()> {
    let mut failure = None;
    let result = vault.visit_events(|event| {
        visit(event).map_err(|error| {
            failure = Some(error);
            "后台扫描中断".to_string()
        })
    });
    if let Some(error) = failure {
        Err(error)
    } else {
        result.map_err(storage)
    }
}
fn bounded_read<T: serde::de::DeserializeOwned>(path: &Path) -> AppResult<T> {
    reject_symlink(path).map_err(storage)?;
    if fs::metadata(path).map_err(storage)?.len() > MAX_RECORD_BYTES {
        return Err(storage("record limit"));
    }
    read_json(path).map_err(storage)
}
fn digest(value: &impl Serialize) -> AppResult<String> {
    Ok(hash(&serde_json::to_vec(value).map_err(storage)?))
}
fn invalid(message: &str) -> AppError {
    AppError::new(
        ErrorCode::InvalidRequest,
        message,
        "检查后台整理设置或任务状态",
    )
}
fn permission() -> AppError {
    AppError::new(
        ErrorCode::PermissionDenied,
        "后台整理没有有效的目的地、资料范围和自动应用授权",
        "在设置中确认具体模型接收地址、来源与旧记忆范围、预算和自动应用策略",
    )
}
fn storage(_error: impl std::fmt::Display) -> AppError {
    AppError::new(
        ErrorCode::Storage,
        "后台整理本机状态无法安全读取或保存；未输出私密内容",
        "检查本机资料库与状态目录，保留原始文件后重试",
    )
}

/// 在受保护启动环境中读取密钥；不把凭据放入配置、参数、日志或任务快照。
/// 可执行文件和模块路径由发行物启动器提供，不接受模型输出或任务数据指定。
pub struct PythonMemoryProvider {
    pub python: PathBuf,
    pub python_path: PathBuf,
    pub timeout_seconds: u64,
    credential: Option<super::credentials::PreparedCredential>,
    environment_binding: Option<MemoryProviderConfig>,
    allow_environment: bool,
    worker_probe: std::sync::Mutex<Option<(std::time::Instant, bool)>>,
}
impl PythonMemoryProvider {
    pub fn new(python: PathBuf, python_path: PathBuf) -> Self {
        Self {
            python,
            python_path,
            timeout_seconds: 45,
            credential: None,
            environment_binding: None,
            allow_environment: true,
            worker_probe: std::sync::Mutex::new(None),
        }
    }
}
impl PythonMemoryProvider {
    fn verify_worker(&self) -> AppResult<()> {
        let unavailable = || {
            AppError::new(
                ErrorCode::ModelUnavailable,
                "Python 3.10+ 模型 worker 尚不可运行",
                "安装或修复发行物支持的 Python 运行时；没有预留模型请求或发送资料",
            )
        };
        let mut cache = self.worker_probe.lock().map_err(|_| unavailable())?;
        if let Some((at, ready)) = *cache {
            if ready {
                return Ok(());
            }
            if at.elapsed() < std::time::Duration::from_secs(30) {
                return Err(unavailable());
            }
        }
        let mut command = std::process::Command::new(&self.python);
        command
            .args([
                "-I",
                "-S",
                "-c",
                "import sys; raise SystemExit(0 if sys.version_info >= (3,10) else 1)",
            ])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .env_clear();
        for name in ["PATH", "SystemRoot", "WINDIR", "TEMP", "TMP"] {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
        let ready = if let Ok(mut child) = command.spawn() {
            let until = std::time::Instant::now() + std::time::Duration::from_secs(2);
            loop {
                match child.try_wait() {
                    Ok(Some(status)) => break status.success(),
                    Err(_) => break false,
                    _ => {}
                }
                if std::time::Instant::now() >= until {
                    let _ = child.kill();
                    let _ = child.wait();
                    break false;
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        } else {
            false
        };
        *cache = Some((std::time::Instant::now(), ready));
        if ready {
            Ok(())
        } else {
            Err(unavailable())
        }
    }
    pub fn without_environment(mut self) -> Self {
        self.allow_environment = false;
        self
    }
    pub fn with_credential(mut self, credential: super::credentials::PreparedCredential) -> Self {
        self.credential = Some(credential);
        self
    }
}
impl MemoryProvider for PythonMemoryProvider {
    fn credential_status(&self, config: &MemoryConfig) -> super::credentials::CredentialStatus {
        let Some(target) = &config.provider else {
            return Default::default();
        };
        if let Some(credential) = &self.credential {
            return if credential.for_target(target).is_some() {
                credential.status.clone()
            } else {
                Default::default()
            };
        }
        if !self.allow_environment
            || matches!(
                config.credential_storage,
                Some(
                    super::credentials::CredentialStorage::OsProtected
                        | super::credentials::CredentialStorage::SessionOnly
                )
            )
            || self
                .environment_binding
                .as_ref()
                .is_some_and(|bound| bound != target)
        {
            return Default::default();
        }
        if std::env::var("RECALLCARD_DREAM_API_KEY").is_ok_and(|key| {
            !key.is_empty() && key.len() <= 8192 && key.bytes().all(|b| (33..=126).contains(&b))
        }) {
            super::credentials::CredentialStatus::environment().for_provider(target)
        } else {
            Default::default()
        }
    }
    fn available(&self, config: &MemoryConfig) -> AppResult<()> {
        config.authorized()?;
        if self.python.as_os_str().is_empty()
            || !self.python_path.is_absolute()
            || !self
                .python_path
                .join("recallcard_dream/runtime.py")
                .is_file()
            || !(1..=120).contains(&self.timeout_seconds)
            || !self.credential_status(config).present
        {
            return Err(AppError::new(
                ErrorCode::ModelUnavailable,
                "模型 worker 或安全启动凭据不可用",
                "在应用的模型设置中提供凭据（系统保护存储或本次会话），并检查 worker 安装；环境变量仅为高级兼容入口",
            ));
        }
        self.verify_worker()
    }
    fn execute(&mut self, config: &MemoryConfig, job: &DreamJob) -> AppResult<ProviderOutput> {
        self.available(config)?;
        if self.credential.is_none() && self.environment_binding.is_none() {
            self.environment_binding = config.provider.clone();
        }
        let request = serde_json::to_vec(&serde_json::json!({"schema":"recallcard.memory-provider-request/1","config":config,"job":job})).map_err(storage)?;
        if request.len() > 2 * 1024 * 1024 {
            return Err(invalid("模型 worker 输入超过 2 MiB"));
        }
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .enable_time()
            .build()
            .map_err(storage)?;
        runtime.block_on(async {
            use std::process::Stdio;
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let mut command = tokio::process::Command::new(&self.python);
            command.args(["-I", "-S", "-c", "import sys; sys.path.insert(0, sys.argv[1]); from recallcard_dream.runtime import main; raise SystemExit(main([]))"]).arg(&self.python_path).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).kill_on_drop(true).env_clear();
            for name in ["PATH", "SystemRoot", "WINDIR", "TEMP", "TMP"] { if let Some(value) = std::env::var_os(name) { command.env(name, value); } }
            if let Some(credential)=&self.credential {let target=config.provider.as_ref().ok_or_else(permission)?;command.env("RECALLCARD_DREAM_API_KEY",credential.for_target(target).ok_or_else(permission)?);}
            else if let Some(value)=std::env::var_os("RECALLCARD_DREAM_API_KEY") {command.env("RECALLCARD_DREAM_API_KEY",value);}
            let unavailable = || AppError::new(ErrorCode::ModelUnavailable,"模型 worker 不可用或调用超时；没有记录原始输出","检查 Python 安装、网络和凭据；重试将再次占用预算");
            let mut child = command.spawn().map_err(|_| unavailable())?;
            let exchange = async {
                let mut stdin = child.stdin.take().ok_or_else(unavailable)?;
                let stdout = child.stdout.take().ok_or_else(unavailable)?;
                // EOF 关闭 stdin；超时监督覆盖写入和读取，异常 worker 不能永久阻塞。
                let mut output = Vec::new();
                let write = async { stdin.write_all(&request).await.map_err(|_| unavailable())?; stdin.shutdown().await.map_err(|_| unavailable()) };
                let read = async { stdout.take(2 * 1024 * 1024 + 1).read_to_end(&mut output).await.map_err(|_| unavailable()).map(|_| ()) };
                // tokio 无宏特性；输入至多 2 MiB，正规 worker 读完 EOF 才响应。
                write.await?;
                // Unix ChildStdin::shutdown 不关闭文件描述符；显式 drop 才向 worker 发送 EOF。
                drop(stdin);
                read.await?;
                if output.len() > 2 * 1024 * 1024 { return Err(invalid("模型 worker 输出超过协议限制")); }
                let status = child.wait().await.map_err(|_| unavailable())?;
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Envelope { ok: bool, result: Option<DreamResult>, usage: Option<ProviderUsage>, verification: Option<String>, error: Option<WorkerError> }
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct WorkerError { code: String, message: String }
                let envelope: Envelope = serde_json::from_slice(&output).map_err(|_| invalid("模型 worker 未返回完整协议结果"))?;
                if !status.success() || !envelope.ok {
                    // 只解释固定 code；绝不传播任意 worker message。
                    let code = envelope.error.as_ref().map(|e| e.code.as_str()).unwrap_or("");
                    let _ = envelope.error.as_ref().map(|e| &e.message);
                    return Err(match code {
                        "approval_required" | "scope_denied" => permission(),
                        "missing_key" | "transport_error" | "timeout" | "provider_error" => unavailable(),
                        _ => invalid("模型返回内容或运行配置不符合协议；未自动修复或重发"),
                    });
                }
                if envelope.error.is_some() || envelope.verification.as_deref() != Some("provider_reported") { return Err(invalid("模型 worker 的验证标记无效")); }
                Ok(ProviderOutput { result: envelope.result.ok_or_else(|| invalid("模型结果缺失"))?, usage: envelope.usage.ok_or_else(|| invalid("模型用量字段缺失"))?, verification: "provider_reported".into() })
            };
            let result = tokio::time::timeout(std::time::Duration::from_secs(self.timeout_seconds), exchange).await.unwrap_or_else(|_| Err(unavailable()));
            if result.is_err() { let _ = child.kill().await; }
            result
        })
    }
}
