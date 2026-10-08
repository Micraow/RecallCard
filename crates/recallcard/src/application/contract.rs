use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{fmt, path::PathBuf};

pub const JOB_SCHEMA: &str = "recallcard.application-job/1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    InvalidRequest,
    UnsupportedFormat,
    InvalidArchive,
    InvalidJson,
    InvalidConversation,
    ResourceLimit,
    SourceChanged,
    PermissionDenied,
    Conflict,
    Cancelled,
    Storage,
    ModelUnavailable,
    BudgetExceeded,
    ConsentRequired,
    Internal,
}

/// context 只能含操作位置，不能含导入正文、服务器响应或秘密。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppError {
    pub code: ErrorCode,
    pub message: String,
    pub action: String,
    pub file_name: Option<String>,
    pub member: Option<String>,
    pub retryable: bool,
    pub committed_events: u64,
}
impl AppError {
    pub fn new(code: ErrorCode, message: impl Into<String>, action: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            action: action.into(),
            file_name: None,
            member: None,
            retryable: false,
            committed_events: 0,
        }
    }
    pub fn at(mut self, file_name: Option<String>, member: Option<String>) -> Self {
        self.file_name = file_name;
        self.member = member;
        self
    }
    pub fn exit_code(&self) -> i32 {
        match self.code {
            ErrorCode::InvalidRequest => 2,
            ErrorCode::UnsupportedFormat
            | ErrorCode::InvalidArchive
            | ErrorCode::InvalidJson
            | ErrorCode::InvalidConversation => 3,
            ErrorCode::ResourceLimit => 4,
            ErrorCode::PermissionDenied | ErrorCode::ConsentRequired => 5,
            ErrorCode::ModelUnavailable => 8,
            ErrorCode::BudgetExceeded => 9,
            ErrorCode::Conflict | ErrorCode::SourceChanged => 6,
            ErrorCode::Cancelled => 7,
            ErrorCode::Storage | ErrorCode::Internal => 1,
        }
    }
}
impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for AppError {}
pub type AppResult<T> = std::result::Result<T, AppError>;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Queued,
    Running,
    Paused,
    NeedsInput,
    Failed,
    Completed,
    Cancelled,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JobPhase {
    Preflight,
    Parsing,
    Preparing,
    Executing,
    Validating,
    Committing,
    Indexing,
    Finished,
}
pub type ImportPhase = JobPhase;

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct ImportProgress {
    pub source_bytes_read: u64,
    pub expanded_bytes_read: u64,
    pub files_processed: u64,
    pub files_total: u64,
    pub conversations: u64,
    pub events_staged: u64,
    pub events_total: Option<u64>,
    pub events_processed: u64,
    pub events_added: u64,
    pub events_duplicates: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobStatus<P = ImportProgress> {
    pub schema: String,
    pub job_id: String,
    pub request_id: String,
    pub kind: String,
    pub scope: String,
    pub state: JobState,
    pub phase: JobPhase,
    pub progress: P,
    pub error: Option<AppError>,
    pub can_resume: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// 由已授权的本机入口提供路径；模型只读接口不暴露此类型。
#[derive(Debug, Clone)]
pub struct ImportRequest {
    pub request_id: String,
    pub paths: Vec<PathBuf>,
    pub scope: String,
    pub format: String,
}

/// 限额约束攻击面；会话限额不再冒充整份 conversations.json 限额。
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ImportLimits {
    pub source_bytes: u64,
    pub expanded_bytes: u64,
    pub archive_entries: usize,
    pub conversation_bytes: usize,
    pub events: u64,
    pub files: usize,
}
impl Default for ImportLimits {
    fn default() -> Self {
        Self {
            source_bytes: 1024 * 1024 * 1024,
            expanded_bytes: 2 * 1024 * 1024 * 1024,
            archive_entries: 2048,
            conversation_bytes: 16 * 1024 * 1024,
            events: 1_000_000,
            files: 32,
        }
    }
}
