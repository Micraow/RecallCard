use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const SCHEMA_VERSION: u32 = 1;
pub type Result<T> = std::result::Result<T, String>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    User,
    Assistant,
    Tool,
    System,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    Native,
    UserInput,
    AssistantOutput,
    ToolOutput,
    ExternalQuote,
    Unknown,
    #[serde(alias = "recallcard_context")]
    ContextInjection,
    RecallcardDreamJob,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ContentPart {
    pub text: String,
    pub origin: Origin,
    #[serde(default)]
    pub refs: Vec<String>,
}

fn personal_scope() -> String {
    "personal".into()
}
fn message_kind() -> String {
    "message".into()
}
fn native_origin() -> Origin {
    Origin::Unknown
}
fn user_authority() -> String {
    "user".into()
}
fn protected_default() -> bool {
    true
}
fn default_score() -> f64 {
    0.0
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Source {
    #[serde(alias = "adapter")]
    pub platform: String,
    #[serde(default)]
    pub account_namespace: String,
    #[serde(alias = "external_session_id")]
    pub conversation_id: String,
    #[serde(alias = "external_event_id")]
    pub message_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EventInput {
    #[serde(default)]
    pub occurred_at: Option<DateTime<Utc>>,
    #[serde(default = "personal_scope")]
    pub scope: String,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub turn_id: Option<String>,
    #[serde(default)]
    pub run_id: Option<String>,
    #[serde(default)]
    pub step_id: Option<String>,
    #[serde(default)]
    pub revision_of: Option<String>,
    #[serde(default)]
    pub reply_to: Option<String>,
    #[serde(default)]
    pub caused_by: Option<String>,
    #[serde(default = "message_kind")]
    pub kind: String,
    #[serde(default)]
    pub parts: Vec<ContentPart>,
    #[serde(default)]
    pub metadata: serde_json::Value,
    #[serde(default)]
    pub capture: CaptureInfo,
    pub role: Role,
    #[serde(default = "native_origin")]
    pub origin: Origin,
    #[serde(default)]
    pub content: String,
    pub source: Source,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CaptureInfo {
    #[serde(default)]
    pub completeness: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub redacted: bool,
    #[serde(default)]
    pub redaction_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Event {
    pub schema_version: u32,
    pub id: String,
    pub captured_at: DateTime<Utc>,
    #[serde(flatten)]
    pub data: EventInput,
}

impl EventInput {
    pub fn validate(&self) -> Result<()> {
        validate_scope(&self.scope)?;
        if self.content.trim().is_empty() && self.parts.is_empty() && self.metadata.is_null() {
            return Err("事件内容不能为空".into());
        }
        if ![
            "message",
            "lifecycle",
            "tool_call",
            "tool_result",
            "file",
            "citation",
            "approval",
            "custom",
        ]
        .contains(&self.kind.as_str())
        {
            return Err("不支持的事件 kind".into());
        }
        for id in [&self.revision_of, &self.reply_to, &self.caused_by]
            .into_iter()
            .flatten()
        {
            validate_id(id, "evt_")?;
        }
        if self.parts.iter().map(|p| p.text.len()).sum::<usize>() > 2 * 1024 * 1024 {
            return Err("事件分块超过 2 MiB".into());
        }
        nonempty(&self.source.platform, "来源平台")?;
        nonempty(&self.source.conversation_id, "对话编号")?;
        nonempty(&self.source.message_id, "消息编号")?;
        if self.content.len() > 2 * 1024 * 1024 {
            return Err("单条事件不能超过 2 MiB".into());
        }
        if serde_json::to_vec(self).map_err(|e| e.to_string())?.len() > 4 * 1024 * 1024 {
            return Err("单条事件整体超过 4 MiB".into());
        }
        Ok(())
    }
    pub fn id(&self) -> Result<String> {
        self.validate()?;
        let bytes = serde_json::to_vec(self).map_err(|e| e.to_string())?;
        Ok(format!("evt_{}", hash(&bytes)))
    }
}
impl Event {
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != SCHEMA_VERSION {
            return Err("不支持的 Event schema_version".into());
        }
        if self.id != self.data.id()? {
            return Err(format!("事件 {} 的内容摘要不匹配", self.id));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MemoryState {
    Active,
    Tentative,
    Superseded,
    Retracted,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Evidence {
    UserExplicit,
    Observed,
    AssistantSuggestion,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct MemoryInput {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub navigation: Vec<crate::navigation::Hint>,
    pub content: String,
    pub source_refs: Vec<String>,
    pub evidence: Evidence,
    #[serde(
        default = "default_score",
        rename = "model_score",
        alias = "confidence"
    )]
    pub confidence: f64,
    #[serde(default = "personal_scope")]
    pub scope: String,
    #[serde(default = "user_authority")]
    pub authority: String,
    #[serde(default = "protected_default")]
    pub protected: bool,
    #[serde(default)]
    pub observed_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub time_note: String,
    #[serde(default)]
    pub supersedes: Vec<String>,
    #[serde(default)]
    pub entities: Vec<String>,
    #[serde(default, rename = "labels", alias = "tags")]
    pub tags: Vec<String>,
    #[serde(default)]
    pub valid_from: Option<DateTime<Utc>>,
    #[serde(default)]
    pub valid_to: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Memory {
    pub schema_version: u32,
    pub id: String,
    pub revision: u64,
    #[serde(rename = "status", alias = "state")]
    pub state: MemoryState,
    #[serde(rename = "recorded_at", alias = "created_at")]
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(flatten)]
    pub data: MemoryInput,
}

impl MemoryInput {
    pub fn validate(&self) -> Result<()> {
        crate::navigation::validate(&self.navigation)?;
        nonempty(&self.content, "记忆内容")?;
        validate_scope(&self.scope)?;
        if !["user", "dream", "import"].contains(&self.authority.as_str()) {
            return Err("无效的 authority".into());
        }
        for id in &self.supersedes {
            validate_id(id, "mem_")?;
        }
        if self.content.len() > 64 * 1024 {
            return Err("单条记忆不能超过 64 KiB".into());
        }
        if !(0.0..=1.0).contains(&self.confidence) || !self.confidence.is_finite() {
            return Err("confidence 必须是 0 到 1 之间的有限数".into());
        }
        if self.source_refs.is_empty() {
            return Err("记忆必须至少关联一条原始事件".into());
        }
        let mut refs = self.source_refs.clone();
        refs.sort();
        refs.dedup();
        if refs.len() != self.source_refs.len() {
            return Err("source_refs 不得重复".into());
        }
        for id in &refs {
            validate_id(id, "evt_")?;
        }
        if serde_json::to_vec(self).map_err(|e| e.to_string())?.len() > 128 * 1024 {
            return Err("单条记忆整体超过 128 KiB".into());
        }
        if let (Some(start), Some(end)) = (self.valid_from, self.valid_to) {
            if start >= end {
                return Err("valid_to 必须晚于 valid_from".into());
            }
        }
        Ok(())
    }
}
impl Memory {
    pub fn validate(&self) -> Result<()> {
        validate_id(&self.id, "mem_")?;
        if self.schema_version != SCHEMA_VERSION || self.revision == 0 {
            return Err("无效的 Memory 版本".into());
        }
        if self.updated_at < self.created_at {
            return Err("记忆更新时间早于创建时间".into());
        }
        self.data.validate()
    }
}

pub fn validate_id(id: &str, prefix: &str) -> Result<()> {
    let suffix = id
        .strip_prefix(prefix)
        .ok_or_else(|| "无效的记录编号前缀".to_string())?;
    let len = if prefix == "evt_" { 64 } else { 32 };
    if suffix.len() != len
        || !suffix
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return Err("无效的记录编号；不接受路径".into());
    }
    Ok(())
}
pub fn nonempty(value: &str, label: &str) -> Result<()> {
    if value.trim().is_empty() {
        Err(format!("{label}不能为空"))
    } else {
        Ok(())
    }
}
pub fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub fn validate_scope(scope: &str) -> Result<()> {
    if scope.is_empty()
        || scope.len() > 128
        || !scope
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, ':' | '-' | '_' | '.'))
    {
        return Err("无效的 scope".into());
    }
    Ok(())
}
impl EventInput {
    pub fn text(&self) -> String {
        if self.parts.is_empty() {
            self.content.clone()
        } else {
            self.parts
                .iter()
                .map(|p| p.text.as_str())
                .collect::<Vec<_>>()
                .join("\n")
        }
    }
    pub fn session_key(&self) -> String {
        self.session_id.clone().unwrap_or_else(|| {
            format!(
                "ses_{}",
                &hash(
                    format!(
                        "{}\0{}\0{}",
                        self.source.platform,
                        self.source.account_namespace,
                        self.source.conversation_id
                    )
                    .as_bytes()
                )[..24]
            )
        })
    }
    /// 修订身份包括资料范围；结构化编码避免来源字段里的分隔符造成碰撞。
    /// source_key 保持旧版语义，供已有来源/抑制记录兼容读取。
    pub fn revision_key(&self) -> String {
        hash(
            serde_json::json!([
                self.scope,
                self.source.platform,
                self.source.account_namespace,
                self.source.conversation_id,
                self.source.message_id
            ])
            .to_string()
            .as_bytes(),
        )
    }
    pub fn source_key(&self) -> String {
        format!(
            "{}\0{}\0{}\0{}",
            self.source.platform,
            self.source.account_namespace,
            self.source.conversation_id,
            self.source.message_id
        )
    }
    pub fn has_original_evidence(&self) -> bool {
        if self.parts.is_empty() {
            !matches!(
                self.origin,
                Origin::ContextInjection | Origin::RecallcardDreamJob | Origin::Unknown
            )
        } else {
            !self.parts.iter().any(|p| {
                matches!(
                    p.origin,
                    Origin::ContextInjection | Origin::RecallcardDreamJob | Origin::Unknown
                )
            })
        }
    }
}
