//! 云向量独立批准及跨进程月度预留；只有本机设置可更改，模型接口不能创建批准。
//! 本模块不访问网络、不读取模型凭据、不保存查询/Memory/来源正文。
use super::{AppError, AppResult, ErrorCode};
use crate::{
    filesystem::{file_identity, open_local_file, FileIdentity},
    model::{hash, validate_scope},
    vault::reject_symlink,
    Vault,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::Read,
    path::PathBuf,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CloudEmbeddingSettings {
    pub endpoint: String,
    pub model: String,
    pub dimensions: usize,
    pub revision: String,
    #[serde(default)]
    pub query_instruction: String,
    #[serde(default)]
    pub document_instruction: String,
    pub query_approved: bool,
    pub memory_approved: bool,
    #[serde(default, skip_serializing_if = "super::budget::BudgetMode::is_limited")]
    pub budget_mode: super::budget::BudgetMode,
    pub max_calls_per_month: u64,
    pub max_bytes_per_month: u64,
}
impl CloudEmbeddingSettings {
    pub fn validate(&self) -> AppResult<()> {
        if !valid_settings_contract(
            &self.endpoint,
            &self.model,
            self.dimensions,
            &self.revision,
            (&self.query_instruction, &self.document_instruction),
            (self.max_calls_per_month, self.max_bytes_per_month),
        ) || (!self.query_approved && !self.memory_approved)
        {
            return Err(invalid(
                "云向量设置无效；需完整 HTTPS 接口、模型空间、独立数据选择及有限月度请求/字节预算",
            ));
        }
        Ok(())
    }
}
/// 来源检索的批准合同与 Memory 检索严格分离；缺省数据开关均为关闭。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceCloudSettings {
    pub endpoint: String,
    pub model: String,
    pub dimensions: usize,
    pub revision: String,
    #[serde(default)]
    pub query_instruction: String,
    #[serde(default)]
    pub document_instruction: String,
    #[serde(default)]
    pub query_approved: bool,
    #[serde(default)]
    pub events_approved: bool,
    #[serde(default)]
    pub attachments_approved: bool,
    #[serde(default, skip_serializing_if = "super::budget::BudgetMode::is_limited")]
    pub budget_mode: super::budget::BudgetMode,
    pub max_calls_per_month: u64,
    pub max_bytes_per_month: u64,
}
impl SourceCloudSettings {
    pub fn validate(&self) -> AppResult<()> {
        if !valid_settings_contract(
            &self.endpoint,
            &self.model,
            self.dimensions,
            &self.revision,
            (&self.query_instruction, &self.document_instruction),
            (self.max_calls_per_month, self.max_bytes_per_month),
        ) || (!self.query_approved && !self.events_approved && !self.attachments_approved)
        {
            return Err(invalid(
                "云来源向量设置无效；需完整 HTTPS 接口、模型空间、独立数据选择及有限月度请求/字节预算",
            ));
        }
        Ok(())
    }
}
// 仅共享目标与限额格式校验；不转换批准类型，也不共享任何数据类型授权。
fn valid_settings_contract(
    endpoint: &str,
    model: &str,
    dimensions: usize,
    revision: &str,
    instructions: (&str, &str),
    limits: (u64, u64),
) -> bool {
    let rest = endpoint.strip_prefix("https://").unwrap_or("");
    let (host, path) = rest.split_once('/').unwrap_or(("", ""));
    let (name, port) = host
        .split_once(':')
        .map_or((host, None), |(h, p)| (h, Some(p)));
    endpoint.len() <= 2048
        && endpoint.is_ascii()
        && !name.is_empty()
        && name.split('.').all(|label| {
            !label.is_empty()
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'-')
        })
        && port.is_none_or(|p| {
            !p.is_empty()
                && p.bytes().all(|b| b.is_ascii_digit())
                && p.parse::<u16>().is_ok_and(|v| v > 0)
        })
        && !path.is_empty()
        && !path.ends_with('/')
        && !endpoint
            .bytes()
            .any(|b| b <= 32 || b == 127 || matches!(b, b'?' | b'#' | b'@' | b'\\'))
        && text(model, 256, false)
        && text(revision, 256, false)
        && !model.chars().any(char::is_control)
        && !revision.chars().any(char::is_control)
        && text(instructions.0, 2048, true)
        && text(instructions.1, 2048, true)
        && (1..=8192).contains(&dimensions)
        && (1..=1_000_000).contains(&limits.0)
        && (1..=4 * 1024 * 1024 * 1024).contains(&limits.1)
}
fn text(s: &str, limit: usize, empty: bool) -> bool {
    s.len() <= limit
        && (empty || !s.trim().is_empty())
        && !s.chars().any(|c| c.is_control() && c != '\n')
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EmbeddingData {
    Query,
    CurrentMemory,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceEmbeddingData {
    Query,
    Event,
    Attachment,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CloudEmbeddingGrant {
    pub revision: String,
    pub scope: String,
    pub settings: CloudEmbeddingSettings,
    pub accepted_at: DateTime<Utc>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceCloudGrant {
    pub revision: String,
    pub scope: String,
    pub settings: SourceCloudSettings,
    pub accepted_at: DateTime<Utc>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ApprovalRecord {
    schema: String,
    root: FileIdentity,
    marker: FileIdentity,
    grant: CloudEmbeddingGrant,
    active: bool,
    #[serde(default)]
    configuration_hash: Option<String>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceApprovalRecord {
    schema: String,
    root: FileIdentity,
    marker: FileIdentity,
    grant: SourceCloudGrant,
    active: bool,
    #[serde(default)]
    configuration_hash: Option<String>,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmbeddingUsage {
    pub month: String,
    pub reserved_calls: u64,
    pub reserved_bytes: u64,
    pub query_calls: u64,
    pub memory_calls: u64,
    #[serde(default)]
    pub source_query_calls: u64,
    #[serde(default)]
    pub event_calls: u64,
    #[serde(default)]
    pub attachment_calls: u64,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct UsageRecord {
    schema: String,
    root: FileIdentity,
    marker: FileIdentity,
    usage: EmbeddingUsage,
}
#[derive(Debug, Serialize)]
pub struct CloudEmbeddingStatus {
    pub grant: Option<CloudEmbeddingGrant>,
    pub usage: EmbeddingUsage,
    pub budget_note: &'static str,
}
#[derive(Debug, Serialize)]
pub struct SourceCloudStatus {
    pub grant: Option<SourceCloudGrant>,
    pub usage: EmbeddingUsage,
    pub budget_note: &'static str,
}
enum UsageData {
    Memory(EmbeddingData),
    Source(SourceEmbeddingData),
}
pub struct CloudEmbeddingRuntime<'a> {
    vault: &'a Vault,
}
impl<'a> CloudEmbeddingRuntime<'a> {
    pub fn new(vault: &'a Vault) -> Self {
        Self { vault }
    }
    fn directory(&self) -> AppResult<PathBuf> {
        let path = self
            .vault
            .state_dir()
            .map_err(storage)?
            .join("embedding-cloud");
        reject_symlink(&path).map_err(storage)?;
        fs::create_dir_all(&path).map_err(|_| storage("无法创建云向量状态目录"))?;
        Ok(path)
    }
    fn lock(&self) -> AppResult<File> {
        let path = self.directory()?.join("approval.lock");
        let file = match OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(&path)
        {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                open_local_file(&path).map_err(|_| storage("批准锁不可读取"))?
            }
            Err(_) => return Err(storage("批准锁不可创建")),
        };
        if !file.metadata().is_ok_and(|m| m.is_file()) {
            return Err(storage("批准锁不是普通文件"));
        }
        let until = std::time::Instant::now() + std::time::Duration::from_millis(300);
        loop {
            match file.try_lock() {
                Ok(()) => return Ok(file),
                Err(std::fs::TryLockError::WouldBlock) if std::time::Instant::now() < until => {
                    std::thread::sleep(std::time::Duration::from_millis(5))
                }
                Err(std::fs::TryLockError::WouldBlock) => {
                    let mut error = AppError::new(
                        ErrorCode::Conflict,
                        "云向量设置或预算正忙",
                        "稍后重试；尚未发送数据",
                    );
                    error.retryable = true;
                    return Err(error);
                }
                Err(_) => return Err(storage("云向量批准锁失败；尚未发送数据")),
            }
        }
    }

    fn identity(&self) -> AppResult<(FileIdentity, FileIdentity)> {
        let identity = |p: &std::path::Path| {
            file_identity(&open_local_file(p).map_err(|_| storage("资料库身份不可读取"))?)
                .map_err(|_| storage("资料库身份不可核验"))
        };
        Ok((
            identity(self.vault.root())?,
            identity(&self.vault.root().join("control/schema-version.json"))?,
        ))
    }
    fn read<T: serde::de::DeserializeOwned>(&self, name: &str) -> AppResult<Option<T>> {
        let path = self.directory()?.join(name);
        match fs::symlink_metadata(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Ok(m) if m.is_file() && m.len() <= 65536 => (),
            _ => return Err(storage("云向量批准/用量不是有界普通文件")),
        }
        let mut bytes = Vec::new();
        open_local_file(&path)
            .map_err(|_| storage("云向量状态不可读取"))?
            .take(65537)
            .read_to_end(&mut bytes)
            .map_err(|_| storage("云向量状态不可读取"))?;
        if bytes.len() > 65536 {
            return Err(storage("云向量状态超过上限"));
        }
        serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|_| storage("云向量状态损坏；未重置预算或恢复批准"))
    }
    fn approval(&self) -> AppResult<Option<ApprovalRecord>> {
        let Some(record) = self.read::<ApprovalRecord>("approval.json")? else {
            return Ok(None);
        };
        let (root, marker) = self.identity()?;
        if record.schema != "recallcard.cloud-embedding-approval/1"
            || record.root != root
            || record.marker != marker
            || uuid::Uuid::parse_str(&record.grant.revision).is_err()
            || validate_scope(&record.grant.scope).is_err()
            || record.grant.settings.validate().is_err()
        {
            return Err(storage("云向量批准与当前资料库或合同不符"));
        }
        Ok(Some(record))
    }
    fn source_approval_name(scope: &str) -> AppResult<String> {
        validate_scope(scope).map_err(invalid)?;
        Ok(format!("source-approval-{}.json", hash(scope.as_bytes())))
    }
    fn source_approval(&self, scope: &str) -> AppResult<Option<SourceApprovalRecord>> {
        let name = Self::source_approval_name(scope)?;
        let Some(record) = self.read::<SourceApprovalRecord>(&name)? else {
            return Ok(None);
        };
        let (root, marker) = self.identity()?;
        if record.schema != "recallcard.source-cloud-embedding-approval/1"
            || record.root != root
            || record.marker != marker
            || uuid::Uuid::parse_str(&record.grant.revision).is_err()
            || record.grant.scope != scope
            || record.grant.settings.validate().is_err()
            || record
                .configuration_hash
                .as_deref()
                .is_some_and(|digest| !valid_digest(digest))
        {
            return Err(storage("云来源向量批准与当前资料库、范围或合同不符"));
        }
        Ok(Some(record))
    }
    pub fn configure_sources(
        &self,
        scope: &str,
        settings: SourceCloudSettings,
    ) -> AppResult<SourceCloudGrant> {
        settings.validate()?;
        let name = Self::source_approval_name(scope)?;
        let _lock = self.lock()?;
        if let Some(old) = self.source_approval(scope)?.filter(|r| r.active) {
            if old.grant.settings == settings {
                return Ok(old.grant);
            }
        }
        let (root, marker) = self.identity()?;
        let grant = SourceCloudGrant {
            revision: uuid::Uuid::new_v4().to_string(),
            scope: scope.into(),
            settings,
            accepted_at: Utc::now(),
        };
        self.vault
            .write_replace(
                &self.directory()?.join(name),
                &SourceApprovalRecord {
                    schema: "recallcard.source-cloud-embedding-approval/1".into(),
                    root,
                    marker,
                    grant: grant.clone(),
                    active: true,
                    configuration_hash: None,
                },
            )
            .map_err(storage)?;
        Ok(grant)
    }
    pub(crate) fn bind_source_configuration(
        &self,
        grant: &SourceCloudGrant,
        digest: &str,
    ) -> AppResult<()> {
        if !valid_digest(digest) {
            return Err(invalid("配置摘要无效"));
        }
        let name = Self::source_approval_name(&grant.scope)?;
        let _lock = self.lock()?;
        let mut record = self
            .source_approval(&grant.scope)?
            .filter(|r| r.active && r.grant == *grant)
            .ok_or_else(|| denied("云来源批准已改变"))?;
        record.configuration_hash = Some(digest.into());
        self.vault
            .write_replace(&self.directory()?.join(name), &record)
            .map_err(storage)
    }
    pub(crate) fn source_configuration_current(
        &self,
        grant: &SourceCloudGrant,
        digest: &str,
    ) -> AppResult<bool> {
        Ok(self.source_approval(&grant.scope)?.is_some_and(|r| {
            r.active && r.grant == *grant && r.configuration_hash.as_deref() == Some(digest)
        }))
    }
    pub fn revoke_sources(&self, scope: &str) -> AppResult<()> {
        let name = Self::source_approval_name(scope)?;
        let _lock = self.lock()?;
        if let Some(mut record) = self.source_approval(scope)? {
            record.active = false;
            self.vault
                .write_replace(&self.directory()?.join(name), &record)
                .map_err(storage)?;
        }
        Ok(())
    }
    pub fn source_status(&self, scope: &str) -> AppResult<SourceCloudStatus> {
        validate_scope(scope).map_err(invalid)?;
        let _lock = self.lock()?;
        let grant = self
            .source_approval(scope)?
            .filter(|r| r.active)
            .map(|r| r.grant);
        Ok(SourceCloudStatus {
            grant,
            usage: self.usage_at(Utc::now())?,
            budget_note: "UTC 自然月、整个资料库累计；来源查询、事件、附件与旧查询、Memory 共用同一计量。每次请求按该请求当前批准的限额检查全资料库累计值，不同批准的限额不是独立可叠加的余额。无限模式不按月度配额停止，仍保留计量、单次上限及撤权。更换范围、目标、重启或重新批准均不清零。发送前预留，失败不退还；正文 UTF-8 字节上限不等于供应商账单上限。",
        })
    }
    pub fn source_is_current(&self, grant: &SourceCloudGrant) -> AppResult<bool> {
        Ok(self
            .source_approval(&grant.scope)?
            .is_some_and(|r| r.active && r.grant == *grant))
    }
    pub fn configure(
        &self,
        scope: &str,
        settings: CloudEmbeddingSettings,
    ) -> AppResult<CloudEmbeddingGrant> {
        settings.validate()?;
        validate_scope(scope).map_err(invalid)?;
        let _lock = self.lock()?;
        if let Some(old) = self.approval()?.filter(|r| r.active) {
            if old.grant.scope != scope {
                return Err(denied("先在原范围停用云向量；没有覆盖其他范围批准"));
            }
            if old.grant.settings == settings {
                return Ok(old.grant);
            }
        }
        let (root, marker) = self.identity()?;
        let grant = CloudEmbeddingGrant {
            revision: uuid::Uuid::new_v4().to_string(),
            scope: scope.into(),
            settings,
            accepted_at: Utc::now(),
        };
        self.vault
            .write_replace(
                &self.directory()?.join("approval.json"),
                &ApprovalRecord {
                    schema: "recallcard.cloud-embedding-approval/1".into(),
                    root,
                    marker,
                    grant: grant.clone(),
                    active: true,
                    configuration_hash: None,
                },
            )
            .map_err(storage)?;
        Ok(grant)
    }
    pub(crate) fn bind_configuration(
        &self,
        grant: &CloudEmbeddingGrant,
        digest: &str,
    ) -> AppResult<()> {
        if !valid_digest(digest) {
            return Err(invalid("配置摘要无效"));
        }
        let _lock = self.lock()?;
        let mut record = self
            .approval()?
            .filter(|r| r.active && r.grant == *grant)
            .ok_or_else(|| denied("云批准已改变"))?;
        record.configuration_hash = Some(digest.into());
        self.vault
            .write_replace(&self.directory()?.join("approval.json"), &record)
            .map_err(storage)
    }
    pub(crate) fn configuration_current(
        &self,
        grant: &CloudEmbeddingGrant,
        digest: &str,
    ) -> AppResult<bool> {
        Ok(self.approval()?.is_some_and(|r| {
            r.active && r.grant == *grant && r.configuration_hash.as_deref() == Some(digest)
        }))
    }
    pub fn revoke(&self, scope: &str) -> AppResult<()> {
        validate_scope(scope).map_err(invalid)?;
        let _lock = self.lock()?;
        if let Some(mut record) = self.approval()? {
            if record.grant.scope != scope {
                return Err(denied("当前范围不能停用其他范围批准"));
            }
            record.active = false;
            self.vault
                .write_replace(&self.directory()?.join("approval.json"), &record)
                .map_err(storage)?;
        }
        Ok(())
    }
    pub fn status(&self, scope: &str) -> AppResult<CloudEmbeddingStatus> {
        validate_scope(scope).map_err(invalid)?;
        let _lock = self.lock()?;
        let grant = self
            .approval()?
            .filter(|r| r.active && r.grant.scope == scope)
            .map(|r| r.grant);
        Ok(CloudEmbeddingStatus{grant,usage:self.usage_at(Utc::now())?,budget_note:"UTC 自然月、整个资料库累计；旧查询、Memory 与来源查询、事件、附件共用同一计量，每次请求按该请求当前批准的限额检查累计值，不同批准的限额不是独立可叠加的余额。无限模式不按月度配额停止，仍保留计量、单次上限及撤权。更换模型、重启或重新批准均不清零。发送前预留，失败不退还；请求/正文 UTF-8 字节上限不等于供应商账单上限。"})
    }
    pub fn is_current(&self, grant: &CloudEmbeddingGrant) -> AppResult<bool> {
        Ok(self
            .approval()?
            .is_some_and(|r| r.active && r.grant == *grant))
    }
    #[allow(clippy::too_many_arguments)]
    pub fn reserve(
        &self,
        grant: &CloudEmbeddingGrant,
        data: EmbeddingData,
        scope: &str,
        endpoint: &str,
        model: &str,
        body_bytes: u64,
    ) -> AppResult<EmbeddingUsage> {
        self.reserve_at(grant, data, scope, endpoint, model, body_bytes, Utc::now())
    }
    #[allow(clippy::too_many_arguments)]
    fn reserve_at(
        &self,
        grant: &CloudEmbeddingGrant,
        data: EmbeddingData,
        scope: &str,
        endpoint: &str,
        model: &str,
        body_bytes: u64,
        now: DateTime<Utc>,
    ) -> AppResult<EmbeddingUsage> {
        let _lock = self.lock()?;
        if !self.is_current(grant)?
            || scope != grant.scope
            || endpoint != grant.settings.endpoint
            || model != grant.settings.model
            || !(1..=4 * 1024 * 1024).contains(&body_bytes)
            || !match data {
                EmbeddingData::Query => grant.settings.query_approved,
                EmbeddingData::CurrentMemory => grant.settings.memory_approved,
            }
        {
            return Err(denied(
                "云向量数据类型、范围、接收者或批准版本不匹配；没有发送",
            ));
        }
        self.reserve_usage(
            now,
            body_bytes,
            grant.settings.budget_mode,
            (
                grant.settings.max_calls_per_month,
                grant.settings.max_bytes_per_month,
            ),
            UsageData::Memory(data),
        )
    }
    #[allow(clippy::too_many_arguments)]
    pub fn reserve_source(
        &self,
        grant: &SourceCloudGrant,
        data: SourceEmbeddingData,
        scope: &str,
        endpoint: &str,
        model: &str,
        body_bytes: u64,
    ) -> AppResult<EmbeddingUsage> {
        self.reserve_source_at(grant, data, scope, endpoint, model, body_bytes, Utc::now())
    }
    #[allow(clippy::too_many_arguments)]
    fn reserve_source_at(
        &self,
        grant: &SourceCloudGrant,
        data: SourceEmbeddingData,
        scope: &str,
        endpoint: &str,
        model: &str,
        body_bytes: u64,
        now: DateTime<Utc>,
    ) -> AppResult<EmbeddingUsage> {
        let _lock = self.lock()?;
        if !self.source_is_current(grant)?
            || scope != grant.scope
            || endpoint != grant.settings.endpoint
            || model != grant.settings.model
            || !(1..=4 * 1024 * 1024).contains(&body_bytes)
            || !match data {
                SourceEmbeddingData::Query => grant.settings.query_approved,
                SourceEmbeddingData::Event => grant.settings.events_approved,
                SourceEmbeddingData::Attachment => grant.settings.attachments_approved,
            }
        {
            return Err(denied(
                "云来源向量数据类型、范围、接收者或批准版本不匹配；没有发送",
            ));
        }
        self.reserve_usage(
            now,
            body_bytes,
            grant.settings.budget_mode,
            (
                grant.settings.max_calls_per_month,
                grant.settings.max_bytes_per_month,
            ),
            UsageData::Source(data),
        )
    }
    // 调用方必须持有同一 approval.lock，并已核对批准与单次正文上限。
    fn reserve_usage(
        &self,
        now: DateTime<Utc>,
        body_bytes: u64,
        budget_mode: super::budget::BudgetMode,
        limits: (u64, u64),
        data: UsageData,
    ) -> AppResult<EmbeddingUsage> {
        let mut usage = self.usage_at(now)?;
        if budget_mode.is_limited()
            && (usage.reserved_calls >= limits.0
                || usage.reserved_bytes.saturating_add(body_bytes) > limits.1)
        {
            return Err(AppError::new(
                ErrorCode::BudgetExceeded,
                "本月云向量预算不足；没有发送",
                "等待下一 UTC 月，或在设置中重新审阅并明确调整预算",
            ));
        }
        usage.reserved_calls = usage
            .reserved_calls
            .checked_add(1)
            .ok_or_else(|| storage("用量计数溢出；没有发送"))?;
        usage.reserved_bytes = usage
            .reserved_bytes
            .checked_add(body_bytes)
            .ok_or_else(|| storage("用量计数溢出；没有发送"))?;
        match data {
            UsageData::Memory(EmbeddingData::Query) => usage.query_calls += 1,
            UsageData::Memory(EmbeddingData::CurrentMemory) => usage.memory_calls += 1,
            UsageData::Source(SourceEmbeddingData::Query) => usage.source_query_calls += 1,
            UsageData::Source(SourceEmbeddingData::Event) => usage.event_calls += 1,
            UsageData::Source(SourceEmbeddingData::Attachment) => usage.attachment_calls += 1,
        }
        let (root, marker) = self.identity()?;
        self.vault
            .write_replace(
                &self
                    .directory()?
                    .join(format!("usage-{}.json", usage.month)),
                &UsageRecord {
                    schema: "recallcard.cloud-embedding-usage/1".into(),
                    root,
                    marker,
                    usage: usage.clone(),
                },
            )
            .map_err(storage)?;
        Ok(usage)
    }
    fn usage_at(&self, now: DateTime<Utc>) -> AppResult<EmbeddingUsage> {
        let month = now.format("%Y-%m").to_string();
        let Some(record) = self.read::<UsageRecord>(&format!("usage-{month}.json"))? else {
            return Ok(EmbeddingUsage {
                month,
                ..Default::default()
            });
        };
        let (root, marker) = self.identity()?;
        if record.schema != "recallcard.cloud-embedding-usage/1"
            || record.root != root
            || record.marker != marker
            || record.usage.month != month
            || record
                .usage
                .query_calls
                .checked_add(record.usage.memory_calls)
                .and_then(|n| n.checked_add(record.usage.source_query_calls))
                .and_then(|n| n.checked_add(record.usage.event_calls))
                .and_then(|n| n.checked_add(record.usage.attachment_calls))
                != Some(record.usage.reserved_calls)
        {
            return Err(storage("云向量用量损坏或不属于当前资料库；没有清零"));
        }
        Ok(record.usage)
    }
}
fn valid_digest(digest: &str) -> bool {
    digest.len() == 64
        && digest
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}
/// 凭据命名空间与 LLM 隔离；相同 endpoint/model 也绝不共享。
pub fn credential_provider_ref(settings: &CloudEmbeddingSettings) -> String {
    hash(
        &serde_json::to_vec(&("cloud-embedding", &settings.endpoint, &settings.model))
            .expect("固定字段可序列化"),
    )
}
fn invalid(message: impl Into<String>) -> AppError {
    AppError::new(
        ErrorCode::InvalidRequest,
        message,
        "核对云向量接收者、数据范围与预算后重试",
    )
}
fn denied(message: impl Into<String>) -> AppError {
    AppError::new(
        ErrorCode::PermissionDenied,
        message,
        "重新检查当前范围的独立云向量批准",
    )
}
fn storage(message: impl Into<String>) -> AppError {
    AppError::new(
        ErrorCode::Storage,
        message,
        "保留原状态并检查资料库；不会自动放宽批准或重置用量",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn settings() -> CloudEmbeddingSettings {
        CloudEmbeddingSettings {
            endpoint: "https://embedding.example.invalid/v1/embeddings".into(),
            model: "synthetic-vector".into(),
            dimensions: 2,
            revision: "test-v1".into(),
            query_instruction: "问题".into(),
            document_instruction: "记忆".into(),
            query_approved: true,
            memory_approved: false,
            budget_mode: super::super::budget::BudgetMode::Limited,
            max_calls_per_month: 2,
            max_bytes_per_month: 1000,
        }
    }
    fn vault() -> (tempfile::TempDir, Vault) {
        let dir = tempfile::tempdir().unwrap();
        let vault = Vault::init(&dir.path().join("vault")).unwrap();
        (dir, vault)
    }
    fn reserve(
        runtime: &CloudEmbeddingRuntime<'_>,
        grant: &CloudEmbeddingGrant,
        kind: EmbeddingData,
        bytes: u64,
    ) -> AppResult<EmbeddingUsage> {
        runtime.reserve(
            grant,
            kind,
            &grant.scope,
            &grant.settings.endpoint,
            &grant.settings.model,
            bytes,
        )
    }
    fn source_settings() -> SourceCloudSettings {
        let original = settings();
        SourceCloudSettings {
            endpoint: original.endpoint,
            model: original.model,
            dimensions: original.dimensions,
            revision: original.revision,
            query_instruction: original.query_instruction,
            document_instruction: "来源".into(),
            query_approved: true,
            events_approved: false,
            attachments_approved: false,
            budget_mode: original.budget_mode,
            max_calls_per_month: 20,
            max_bytes_per_month: 1000,
        }
    }
    fn source_month() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2098-02-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }
    fn reserve_source(
        runtime: &CloudEmbeddingRuntime<'_>,
        grant: &SourceCloudGrant,
        kind: SourceEmbeddingData,
        bytes: u64,
    ) -> AppResult<EmbeddingUsage> {
        runtime.reserve_source_at(
            grant,
            kind,
            &grant.scope,
            &grant.settings.endpoint,
            &grant.settings.model,
            bytes,
            source_month(),
        )
    }
    fn reserve_memory_same_month(
        runtime: &CloudEmbeddingRuntime<'_>,
        grant: &CloudEmbeddingGrant,
        kind: EmbeddingData,
        bytes: u64,
    ) -> AppResult<EmbeddingUsage> {
        runtime.reserve_at(
            grant,
            kind,
            &grant.scope,
            &grant.settings.endpoint,
            &grant.settings.model,
            bytes,
            source_month(),
        )
    }
    #[test]
    fn source_contract_defaults_off_and_cannot_deserialize_memory_permissions() {
        let mut value = serde_json::to_value(source_settings()).unwrap();
        for field in ["query_approved", "events_approved", "attachments_approved"] {
            value.as_object_mut().unwrap().remove(field);
        }
        let omitted = serde_json::from_value::<SourceCloudSettings>(value.clone()).unwrap();
        assert!(!omitted.query_approved);
        assert!(!omitted.events_approved);
        assert!(!omitted.attachments_approved);
        assert!(omitted.validate().is_err());
        value["events_approved"] = serde_json::json!(true);
        let events = serde_json::from_value::<SourceCloudSettings>(value.clone()).unwrap();
        events.validate().unwrap();
        assert!(!events.query_approved);
        assert!(!events.attachments_approved);
        value["memory_approved"] = serde_json::json!(true);
        assert!(serde_json::from_value::<SourceCloudSettings>(value).is_err());
        assert!(serde_json::from_value::<SourceCloudSettings>(
            serde_json::to_value(settings()).unwrap()
        )
        .is_err());
        assert!(serde_json::from_value::<CloudEmbeddingSettings>(
            serde_json::to_value(source_settings()).unwrap()
        )
        .is_err());
        for endpoint in [
            "http://example.invalid/api",
            "https://key@example.invalid/api",
            "https://example.invalid/api?key=synthetic",
            "https://example.invalid:0/api",
            "https://example.invalid/api/",
        ] {
            let mut bad = source_settings();
            bad.endpoint = endpoint.into();
            assert!(bad.validate().is_err(), "{endpoint}");
        }
        for (dimensions, calls, bytes) in [(0, 1, 1), (8193, 1, 1), (2, 0, 1), (2, 1, 0)] {
            let mut bad = source_settings();
            bad.dimensions = dimensions;
            bad.max_calls_per_month = calls;
            bad.max_bytes_per_month = bytes;
            assert!(bad.validate().is_err());
        }
    }
    #[test]
    fn memory_approval_never_authorizes_sources_and_source_flags_are_independent() {
        let (_dir, vault) = vault();
        let runtime = CloudEmbeddingRuntime::new(&vault);
        let memory = runtime.configure("personal", settings()).unwrap();
        let memory_path = runtime.directory().unwrap().join("approval.json");
        let original = fs::read(&memory_path).unwrap();
        assert!(runtime.source_status("personal").unwrap().grant.is_none());
        let forged = SourceCloudGrant {
            revision: memory.revision.clone(),
            scope: memory.scope.clone(),
            settings: source_settings(),
            accepted_at: memory.accepted_at,
        };
        assert!(!runtime.source_is_current(&forged).unwrap());
        assert_eq!(
            reserve_source(&runtime, &forged, SourceEmbeddingData::Query, 1)
                .unwrap_err()
                .code,
            ErrorCode::PermissionDenied
        );
        let kinds = [
            SourceEmbeddingData::Query,
            SourceEmbeddingData::Event,
            SourceEmbeddingData::Attachment,
        ];
        for (index, selected) in kinds.iter().enumerate() {
            let mut config = source_settings();
            config.query_approved = *selected == SourceEmbeddingData::Query;
            config.events_approved = *selected == SourceEmbeddingData::Event;
            config.attachments_approved = *selected == SourceEmbeddingData::Attachment;
            let grant = runtime.configure_sources("personal", config).unwrap();
            for kind in kinds {
                let result = reserve_source(&runtime, &grant, kind, 10);
                if kind == *selected {
                    assert_eq!(result.unwrap().reserved_calls, index as u64 + 1);
                } else {
                    assert_eq!(result.unwrap_err().code, ErrorCode::PermissionDenied);
                }
            }
        }
        let usage = runtime.usage_at(source_month()).unwrap();
        assert_eq!(usage.query_calls, 0);
        assert_eq!(usage.memory_calls, 0);
        assert_eq!(usage.source_query_calls, 1);
        assert_eq!(usage.event_calls, 1);
        assert_eq!(usage.attachment_calls, 1);
        assert!(runtime.is_current(&memory).unwrap());
        assert_eq!(fs::read(memory_path).unwrap(), original);
    }
    #[test]
    fn source_grants_bind_exact_scope_target_revision_and_configuration() {
        let (_dir, vault) = vault();
        let runtime = CloudEmbeddingRuntime::new(&vault);
        let grant = runtime
            .configure_sources("personal", source_settings())
            .unwrap();
        let digest = hash(b"synthetic-config-a");
        let changed_digest = hash(b"synthetic-config-b");
        assert!(!runtime
            .source_configuration_current(&grant, &digest)
            .unwrap());
        runtime.bind_source_configuration(&grant, &digest).unwrap();
        assert!(runtime
            .source_configuration_current(&grant, &digest)
            .unwrap());
        assert!(!runtime
            .source_configuration_current(&grant, &changed_digest)
            .unwrap());
        for invalid in ["", "short", &"A".repeat(64)] {
            assert_eq!(
                runtime
                    .bind_source_configuration(&grant, invalid)
                    .unwrap_err()
                    .code,
                ErrorCode::InvalidRequest
            );
        }
        for (scope, endpoint, model) in [
            (
                "work",
                grant.settings.endpoint.as_str(),
                grant.settings.model.as_str(),
            ),
            (
                "personal",
                "https://other.invalid/api",
                grant.settings.model.as_str(),
            ),
            ("personal", grant.settings.endpoint.as_str(), "other-model"),
        ] {
            assert_eq!(
                runtime
                    .reserve_source_at(
                        &grant,
                        SourceEmbeddingData::Query,
                        scope,
                        endpoint,
                        model,
                        1,
                        source_month(),
                    )
                    .unwrap_err()
                    .code,
                ErrorCode::PermissionDenied
            );
        }
        let mut stale = grant.clone();
        stale.revision = uuid::Uuid::new_v4().to_string();
        let mut different_model_space = grant.clone();
        different_model_space.settings.revision = "different-model-space".into();
        let mut different_dimensions = grant.clone();
        different_dimensions.settings.dimensions += 1;
        let mut different_instructions = grant.clone();
        different_instructions.settings.document_instruction = "不同指令".into();
        for forged in [
            stale,
            different_model_space,
            different_dimensions,
            different_instructions,
        ] {
            assert!(!runtime.source_is_current(&forged).unwrap());
            assert!(!runtime
                .source_configuration_current(&forged, &digest)
                .unwrap());
            assert_eq!(
                reserve_source(&runtime, &forged, SourceEmbeddingData::Query, 1)
                    .unwrap_err()
                    .code,
                ErrorCode::PermissionDenied
            );
            assert!(runtime.bind_source_configuration(&forged, &digest).is_err());
        }
        assert_eq!(runtime.usage_at(source_month()).unwrap().reserved_calls, 0);
        let work = runtime
            .configure_sources("work", source_settings())
            .unwrap();
        assert!(runtime.source_is_current(&grant).unwrap());
        assert!(runtime.source_is_current(&work).unwrap());
        runtime.revoke_sources("work").unwrap();
        assert!(runtime.source_is_current(&grant).unwrap());
        assert!(!runtime.source_is_current(&work).unwrap());
        assert!(runtime.source_status("work").unwrap().grant.is_none());
    }
    #[test]
    fn source_and_memory_share_one_budget_using_current_request_limits() {
        let (_dir, vault) = vault();
        let runtime = CloudEmbeddingRuntime::new(&vault);
        let mut old_config = settings();
        old_config.max_calls_per_month = 3;
        let old = runtime.configure("personal", old_config).unwrap();
        let mut config = source_settings();
        config.max_calls_per_month = 2;
        let source = runtime.configure_sources("work", config).unwrap();
        reserve_memory_same_month(&runtime, &old, EmbeddingData::Query, 100).unwrap();
        let usage = reserve_source(&runtime, &source, SourceEmbeddingData::Query, 200).unwrap();
        assert_eq!(usage.reserved_calls, 2);
        assert_eq!(usage.reserved_bytes, 300);
        assert_eq!(
            reserve_source(&runtime, &source, SourceEmbeddingData::Query, 1)
                .unwrap_err()
                .code,
            ErrorCode::BudgetExceeded
        );
        let usage = reserve_memory_same_month(&runtime, &old, EmbeddingData::Query, 100).unwrap();
        assert_eq!(usage.reserved_calls, 3);
        assert_eq!(usage.reserved_bytes, 400);
        assert_eq!(
            reserve_memory_same_month(&runtime, &old, EmbeddingData::Query, 1)
                .unwrap_err()
                .code,
            ErrorCode::BudgetExceeded
        );
        let mut other_config = source_settings();
        other_config.max_bytes_per_month = 401;
        let other = runtime.configure_sources("other", other_config).unwrap();
        assert_eq!(
            reserve_source(&runtime, &other, SourceEmbeddingData::Query, 2)
                .unwrap_err()
                .code,
            ErrorCode::BudgetExceeded
        );
        let usage = reserve_source(&runtime, &other, SourceEmbeddingData::Query, 1).unwrap();
        assert_eq!(usage.reserved_calls, 4);
        assert_eq!(usage.reserved_bytes, 401);
        assert_eq!(usage.query_calls, 2);
        assert_eq!(usage.source_query_calls, 2);
    }
    #[test]
    fn legacy_usage_without_source_counters_is_preserved_and_invalid_sums_fail_closed() {
        let (_dir, vault) = vault();
        let runtime = CloudEmbeddingRuntime::new(&vault);
        let old = runtime.configure("personal", settings()).unwrap();
        reserve_memory_same_month(&runtime, &old, EmbeddingData::Query, 100).unwrap();
        let path = runtime.directory().unwrap().join("usage-2098-02.json");
        let mut value: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        for field in ["source_query_calls", "event_calls", "attachment_calls"] {
            value["usage"].as_object_mut().unwrap().remove(field);
        }
        fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        let source = runtime
            .configure_sources("personal", source_settings())
            .unwrap();
        let usage = reserve_source(&runtime, &source, SourceEmbeddingData::Query, 50).unwrap();
        assert_eq!(usage.reserved_calls, 2);
        assert_eq!(usage.reserved_bytes, 150);
        assert_eq!(usage.query_calls, 1);
        assert_eq!(usage.source_query_calls, 1);
        assert_eq!(usage.event_calls, 0);
        assert_eq!(usage.attachment_calls, 0);
        let mut updated: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        updated["usage"]["attachment_calls"] = serde_json::json!(1);
        fs::write(&path, serde_json::to_vec(&updated).unwrap()).unwrap();
        assert_eq!(
            reserve_source(&runtime, &source, SourceEmbeddingData::Query, 1)
                .unwrap_err()
                .code,
            ErrorCode::Storage
        );
        assert_eq!(
            reserve_memory_same_month(&runtime, &old, EmbeddingData::Query, 1)
                .unwrap_err()
                .code,
            ErrorCode::Storage
        );
        assert_eq!(
            fs::read(&path).unwrap(),
            serde_json::to_vec(&updated).unwrap()
        );
    }
    #[test]
    fn source_restarts_retargeting_revoke_and_reapproval_keep_usage() {
        let (_dir, vault) = vault();
        let runtime = CloudEmbeddingRuntime::new(&vault);
        let mut config = source_settings();
        config.max_calls_per_month = 2;
        let grant = runtime
            .configure_sources("personal", config.clone())
            .unwrap();
        assert_eq!(
            runtime
                .configure_sources("personal", config.clone())
                .unwrap(),
            grant
        );
        reserve_source(&runtime, &grant, SourceEmbeddingData::Query, 200).unwrap();
        let reopened = Vault::open(vault.root()).unwrap();
        let runtime = CloudEmbeddingRuntime::new(&reopened);
        config.endpoint = "https://other.invalid/v2/embeddings".into();
        config.model = "another-model".into();
        config.revision = "another-revision".into();
        config.query_approved = false;
        config.attachments_approved = true;
        let next = runtime
            .configure_sources("personal", config.clone())
            .unwrap();
        assert!(!runtime.source_is_current(&grant).unwrap());
        assert!(reserve_source(&runtime, &grant, SourceEmbeddingData::Query, 1).is_err());
        let usage = reserve_source(&runtime, &next, SourceEmbeddingData::Attachment, 300).unwrap();
        assert_eq!(usage.reserved_calls, 2);
        assert_eq!(usage.reserved_bytes, 500);
        assert_eq!(usage.source_query_calls, 1);
        assert_eq!(usage.attachment_calls, 1);
        runtime.revoke_sources("personal").unwrap();
        assert!(reserve_source(&runtime, &next, SourceEmbeddingData::Attachment, 1).is_err());
        let renewed = runtime.configure_sources("personal", config).unwrap();
        assert_ne!(renewed.revision, next.revision);
        assert_eq!(
            reserve_source(&runtime, &renewed, SourceEmbeddingData::Attachment, 1)
                .unwrap_err()
                .code,
            ErrorCode::BudgetExceeded
        );
        let next_month = DateTime::parse_from_rfc3339("2098-03-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let usage = runtime
            .reserve_source_at(
                &renewed,
                SourceEmbeddingData::Attachment,
                "personal",
                &renewed.settings.endpoint,
                &renewed.settings.model,
                1,
                next_month,
            )
            .unwrap();
        assert_eq!(usage.reserved_calls, 1);
        assert_eq!(usage.reserved_bytes, 1);
        assert_eq!(usage.attachment_calls, 1);
    }
    #[test]
    fn source_approval_rejects_copied_vault_scope_bad_uuid_and_config_digest() {
        let (_a, a) = vault();
        let (_b, b) = vault();
        let original = CloudEmbeddingRuntime::new(&a);
        let copied = CloudEmbeddingRuntime::new(&b);
        let grant = original
            .configure_sources("personal", source_settings())
            .unwrap();
        let name = CloudEmbeddingRuntime::source_approval_name("personal").unwrap();
        let path = original.directory().unwrap().join(&name);
        let bytes = fs::read(&path).unwrap();
        fs::write(copied.directory().unwrap().join(&name), &bytes).unwrap();
        assert!(copied.source_is_current(&grant).is_err());
        assert!(copied
            .configure_sources("personal", source_settings())
            .is_err());
        let work_name = CloudEmbeddingRuntime::source_approval_name("work").unwrap();
        fs::write(original.directory().unwrap().join(work_name), &bytes).unwrap();
        assert_eq!(
            original.source_status("work").unwrap_err().code,
            ErrorCode::Storage
        );
        for corrupt in ["scope", "revision", "configuration_hash"] {
            let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            match corrupt {
                "scope" => value["grant"]["scope"] = serde_json::json!("work"),
                "revision" => value["grant"]["revision"] = serde_json::json!("not-a-uuid"),
                _ => value["configuration_hash"] = serde_json::json!("not-a-digest"),
            }
            let invalid = serde_json::to_vec(&value).unwrap();
            fs::write(&path, &invalid).unwrap();
            assert_eq!(
                original.source_is_current(&grant).unwrap_err().code,
                ErrorCode::Storage
            );
            assert!(original
                .configure_sources("personal", source_settings())
                .is_err());
            assert_eq!(fs::read(&path).unwrap(), invalid);
        }
        fs::write(&path, vec![b' '; 65537]).unwrap();
        assert_eq!(
            original.source_is_current(&grant).unwrap_err().code,
            ErrorCode::Storage
        );
        fs::write(&path, &bytes).unwrap();
        assert!(original.source_is_current(&grant).unwrap());
    }
    #[test]
    fn source_unlimited_is_metered_revocable_and_retains_usage_when_limited_again() {
        let (_dir, vault) = vault();
        let runtime = CloudEmbeddingRuntime::new(&vault);
        let mut config = source_settings();
        config.budget_mode = super::super::budget::BudgetMode::Unlimited;
        config.max_calls_per_month = 1;
        config.max_bytes_per_month = 1;
        let grant = runtime
            .configure_sources("personal", config.clone())
            .unwrap();
        for _ in 0..3 {
            reserve_source(&runtime, &grant, SourceEmbeddingData::Query, 100).unwrap();
        }
        let usage = runtime.usage_at(source_month()).unwrap();
        assert_eq!(usage.reserved_calls, 3);
        assert_eq!(usage.reserved_bytes, 300);
        assert_eq!(usage.source_query_calls, 3);
        assert!(reserve_source(&runtime, &grant, SourceEmbeddingData::Event, 1).is_err());
        assert!(reserve_source(&runtime, &grant, SourceEmbeddingData::Attachment, 1).is_err());
        for bytes in [0, 4 * 1024 * 1024 + 1] {
            assert!(reserve_source(&runtime, &grant, SourceEmbeddingData::Query, bytes).is_err());
        }
        runtime.revoke_sources("personal").unwrap();
        assert!(reserve_source(&runtime, &grant, SourceEmbeddingData::Query, 1).is_err());
        config.budget_mode = super::super::budget::BudgetMode::Limited;
        let limited = runtime.configure_sources("personal", config).unwrap();
        assert_eq!(
            reserve_source(&runtime, &limited, SourceEmbeddingData::Query, 1)
                .unwrap_err()
                .code,
            ErrorCode::BudgetExceeded
        );
        let legacy = runtime.configure("personal", settings()).unwrap();
        assert_eq!(
            reserve_memory_same_month(&runtime, &legacy, EmbeddingData::Query, 1)
                .unwrap_err()
                .code,
            ErrorCode::BudgetExceeded
        );
    }
    #[test]
    fn concurrent_source_and_memory_reservations_use_the_same_lock_and_budget() {
        let (_dir, vault) = vault();
        let runtime = CloudEmbeddingRuntime::new(&vault);
        let legacy = runtime.configure("personal", settings()).unwrap();
        let mut config = source_settings();
        config.max_calls_per_month = 2;
        let source = runtime.configure_sources("personal", config).unwrap();
        let barrier = std::sync::Barrier::new(4);
        let results = std::thread::scope(|threads| {
            let handles = (0..4)
                .map(|index| {
                    let barrier = &barrier;
                    let runtime = &runtime;
                    let legacy = &legacy;
                    let source = &source;
                    threads.spawn(move || {
                        barrier.wait();
                        let started = std::time::Instant::now();
                        loop {
                            let result = if index % 2 == 0 {
                                reserve_memory_same_month(runtime, legacy, EmbeddingData::Query, 1)
                            } else {
                                reserve_source(runtime, source, SourceEmbeddingData::Query, 1)
                            };
                            if let Err(error) = &result {
                                if error.code == ErrorCode::Conflict && error.retryable {
                                    assert!(started.elapsed() < std::time::Duration::from_secs(5));
                                    std::thread::sleep(std::time::Duration::from_millis(5));
                                    continue;
                                }
                            }
                            break result;
                        }
                    })
                })
                .collect::<Vec<_>>();
            handles
                .into_iter()
                .map(|h| h.join().unwrap())
                .collect::<Vec<_>>()
        });
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 2);
        assert!(results
            .into_iter()
            .filter_map(Result::err)
            .all(|error| error.code == ErrorCode::BudgetExceeded));
        let usage = runtime.usage_at(source_month()).unwrap();
        assert_eq!(usage.reserved_calls, 2);
        assert_eq!(usage.reserved_bytes, 2);
        assert_eq!(usage.query_calls + usage.source_query_calls, 2);
        let _lock = runtime.lock().unwrap();
        let busy = reserve_source(&runtime, &source, SourceEmbeddingData::Query, 1).unwrap_err();
        assert_eq!(busy.code, ErrorCode::Conflict);
        assert!(busy.retryable);
    }
    #[test]
    fn contract_rejects_injected_permissions_and_bad_destinations() {
        let good = settings();
        good.validate().unwrap();
        for endpoint in [
            "http://example.invalid/v1/embeddings",
            "https://key@example.invalid/api",
            "https://example.invalid/api?key=synthetic",
            "https://example.invalid/api#fragment",
            "https://example.invalid:0/api",
            "https://example.invalid:xyz/api",
            "https://example.invalid/api/",
            "https://example.invalid/api\n",
            "https://[bad]/api",
        ] {
            let mut bad = good.clone();
            bad.endpoint = endpoint.into();
            assert!(bad.validate().is_err(), "{endpoint}");
        }
        let mut value = serde_json::to_value(&good).unwrap();
        value["send_source_snapshots"] = serde_json::json!(true);
        assert!(serde_json::from_value::<CloudEmbeddingSettings>(value).is_err());
        let mut bad = good.clone();
        bad.query_approved = false;
        assert!(bad.validate().is_err());
        bad = good;
        bad.max_calls_per_month = 0;
        assert!(bad.validate().is_err());
    }
    #[test]
    fn data_scope_and_destination_are_independent_of_llm_and_each_other() {
        let (_dir, vault) = vault();
        let runtime = CloudEmbeddingRuntime::new(&vault);
        let grant = runtime.configure("personal", settings()).unwrap();
        assert!(reserve(&runtime, &grant, EmbeddingData::CurrentMemory, 1).is_err());
        for (scope, endpoint, model) in [
            (
                "work",
                grant.settings.endpoint.as_str(),
                grant.settings.model.as_str(),
            ),
            (
                "personal",
                "https://other.invalid/api",
                grant.settings.model.as_str(),
            ),
            ("personal", grant.settings.endpoint.as_str(), "other-model"),
        ] {
            assert!(runtime
                .reserve(&grant, EmbeddingData::Query, scope, endpoint, model, 1)
                .is_err());
        }
        assert_eq!(runtime.status("personal").unwrap().usage.reserved_calls, 0);
        assert!(runtime.configure("work", settings()).is_err());
        assert!(runtime.status("work").unwrap().grant.is_none());
        assert!(runtime.revoke("work").is_err());
        assert_eq!(
            reserve(&runtime, &grant, EmbeddingData::Query, 100)
                .unwrap()
                .query_calls,
            1
        );
    }
    #[test]
    fn restarts_reapproval_retargeting_and_failures_do_not_reset_monthly_budget() {
        let (_dir, vault) = vault();
        let runtime = CloudEmbeddingRuntime::new(&vault);
        let grant = runtime.configure("personal", settings()).unwrap();
        assert_eq!(runtime.configure("personal", settings()).unwrap(), grant);
        reserve(&runtime, &grant, EmbeddingData::Query, 300).unwrap(); // 模拟随后网络失败，无退款入口。
        let reopened = Vault::open(vault.root()).unwrap();
        let runtime = CloudEmbeddingRuntime::new(&reopened);
        let mut changed = settings();
        changed.model = "second-model".into();
        changed.query_approved = false;
        changed.memory_approved = true;
        let next = runtime.configure("personal", changed).unwrap();
        assert!(!runtime.is_current(&grant).unwrap());
        assert!(reserve(&runtime, &grant, EmbeddingData::Query, 1).is_err());
        let usage = reserve(&runtime, &next, EmbeddingData::CurrentMemory, 600).unwrap();
        assert_eq!(usage.reserved_bytes, 900);
        assert_eq!(usage.reserved_calls, 2);
        assert_eq!(
            reserve(&runtime, &next, EmbeddingData::CurrentMemory, 1)
                .unwrap_err()
                .code,
            ErrorCode::BudgetExceeded
        );
        runtime.revoke("personal").unwrap();
        assert!(reserve(&runtime, &next, EmbeddingData::CurrentMemory, 1).is_err());
        let replacement = runtime.configure("work", settings()).unwrap();
        assert!(reserve(&runtime, &replacement, EmbeddingData::Query, 1).is_err());
        let next_month = DateTime::parse_from_rfc3339("2099-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(
            runtime
                .reserve_at(
                    &replacement,
                    EmbeddingData::Query,
                    "work",
                    &replacement.settings.endpoint,
                    &replacement.settings.model,
                    1,
                    next_month
                )
                .unwrap()
                .reserved_calls,
            1
        );
    }
    #[test]
    fn exact_bytes_lock_contention_corrupt_state_and_revoke_fail_closed() {
        let (_dir, vault) = vault();
        let runtime = CloudEmbeddingRuntime::new(&vault);
        let mut config = settings();
        config.max_bytes_per_month = 5;
        let grant = runtime.configure("personal", config).unwrap();
        let lock = runtime.lock().unwrap();
        assert_eq!(
            reserve(&runtime, &grant, EmbeddingData::Query, 1)
                .unwrap_err()
                .code,
            ErrorCode::Conflict
        );
        drop(lock);
        assert!(reserve(&runtime, &grant, EmbeddingData::Query, 0).is_err());
        reserve(&runtime, &grant, EmbeddingData::Query, 5).unwrap();
        assert!(reserve(&runtime, &grant, EmbeddingData::Query, 1).is_err());
        let path = runtime
            .directory()
            .unwrap()
            .join(format!("usage-{}.json", Utc::now().format("%Y-%m")));
        fs::write(&path, b"{}").unwrap();
        assert_eq!(
            reserve(&runtime, &grant, EmbeddingData::Query, 1)
                .unwrap_err()
                .code,
            ErrorCode::Storage
        );
        runtime.revoke("personal").unwrap();
        assert!(!runtime.is_current(&grant).unwrap());
        let text = fs::read_to_string(runtime.directory().unwrap().join("approval.json")).unwrap();
        assert!(!text.contains("api_key"));
        assert!(!text.contains("source_snapshots"));
    }
    #[test]
    fn unlimited_is_explicit_metered_and_still_revocable_without_reset_on_return_to_limited() {
        let (_dir, vault) = vault();
        let runtime = CloudEmbeddingRuntime::new(&vault);
        let mut setup = settings();
        setup.budget_mode = super::super::budget::BudgetMode::Unlimited;
        setup.max_calls_per_month = 1;
        setup.max_bytes_per_month = 1;
        let grant = runtime.configure("personal", setup.clone()).unwrap();
        for _ in 0..3 {
            reserve(&runtime, &grant, EmbeddingData::Query, 100).unwrap();
        }
        let usage = runtime.status("personal").unwrap().usage;
        assert_eq!(usage.reserved_calls, 3);
        assert_eq!(usage.reserved_bytes, 300);
        assert_eq!(
            serde_json::to_value(&grant).unwrap()["settings"]["budget_mode"],
            "unlimited"
        );
        assert!(reserve(&runtime, &grant, EmbeddingData::CurrentMemory, 1).is_err());
        assert!(reserve(&runtime, &grant, EmbeddingData::Query, 4 * 1024 * 1024 + 1).is_err());
        runtime.revoke("personal").unwrap();
        assert!(reserve(&runtime, &grant, EmbeddingData::Query, 1).is_err());
        setup.budget_mode = super::super::budget::BudgetMode::Limited;
        let limited = runtime.configure("personal", setup).unwrap();
        assert_eq!(
            reserve(&runtime, &limited, EmbeddingData::Query, 1)
                .unwrap_err()
                .code,
            ErrorCode::BudgetExceeded
        );
    }
    #[test]
    fn concurrent_reservations_never_overspend_and_busy_is_retryable() {
        let (_dir, vault) = vault();
        let runtime = CloudEmbeddingRuntime::new(&vault);
        let grant = runtime.configure("personal", settings()).unwrap();
        let barrier = std::sync::Barrier::new(3);
        let results = std::thread::scope(|threads| {
            let handles = (0..3)
                .map(|_| {
                    threads.spawn(|| {
                        barrier.wait();
                        let started = std::time::Instant::now();
                        loop {
                            let result = reserve(
                                &CloudEmbeddingRuntime::new(&vault),
                                &grant,
                                EmbeddingData::Query,
                                1,
                            );
                            // 300ms 内的锁忙是明确的可重试结果，不是预算最终判定。
                            // 重试仍须由同一真实批准/预算路径决定，不能增加配额或吞掉其他错误。
                            if let Err(error) = &result {
                                if error.code == ErrorCode::Conflict && error.retryable {
                                    assert!(
                                        started.elapsed() < std::time::Duration::from_secs(5),
                                        "合成预算测试持续锁忙：{error:?}"
                                    );
                                    std::thread::sleep(std::time::Duration::from_millis(5));
                                    continue;
                                }
                            }
                            break result;
                        }
                    })
                })
                .collect::<Vec<_>>();
            handles
                .into_iter()
                .map(|h| h.join().unwrap())
                .collect::<Vec<_>>()
        });
        assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 2);
        assert_eq!(
            results.into_iter().find_map(|r| r.err()).unwrap().code,
            ErrorCode::BudgetExceeded
        );
        let usage = runtime.status("personal").unwrap().usage;
        assert_eq!(usage.reserved_calls, 2);
        assert_eq!(usage.reserved_bytes, 2);
        let _lock = runtime.lock().unwrap();
        let busy = reserve(&runtime, &grant, EmbeddingData::Query, 1).unwrap_err();
        assert_eq!(busy.code, ErrorCode::Conflict);
        assert!(busy.retryable);
    }
    #[test]
    fn copied_approval_and_usage_cannot_authorize_another_vault() {
        let (_a, a) = vault();
        let (_b, b) = vault();
        let original = CloudEmbeddingRuntime::new(&a);
        let copied = CloudEmbeddingRuntime::new(&b);
        let grant = original.configure("personal", settings()).unwrap();
        fs::copy(
            original.directory().unwrap().join("approval.json"),
            copied.directory().unwrap().join("approval.json"),
        )
        .unwrap();
        assert!(copied.is_current(&grant).is_err());
        assert!(copied.configure("personal", settings()).is_err());
    }
}
