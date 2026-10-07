//! 用户主动保存的可见会话交换格式；来源、角色与覆盖范围不能被一段普通笔记替代。
use crate::{capture::redact_event, model::*, Vault};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeSet;

pub const FORMAT: &str = "recallcard.conversation/1";
pub const MAX_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Conversation {
    pub schema: String,
    pub capture_id: String,
    pub captured_at: DateTime<Utc>,
    pub title: String,
    pub source: ConversationSource,
    pub coverage: Coverage,
    pub messages: Vec<Message>,
    #[serde(default)]
    pub metadata: Value,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConversationSource {
    pub platform: String,
    pub conversation_id: String,
    pub url: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Coverage {
    pub extent: String,
    pub complete: bool,
    pub reason: String,
    pub warnings: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Message {
    pub id: String,
    pub role: Role,
    pub text: String,
    pub occurred_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub metadata: Value,
}

impl Conversation {
    pub fn parse(text: &str) -> Result<Self> {
        if text.len() > MAX_BYTES {
            return Err("会话文件超过 16 MiB，请分批保存".into());
        }
        let conversation: Self =
            serde_json::from_str(text).map_err(|_| "不是完整的 RecallCard 会话文件")?;
        conversation.validate()?;
        Ok(conversation)
    }
    pub fn validate(&self) -> Result<()> {
        if self.schema != FORMAT {
            return Err("不支持的会话文件版本".into());
        }
        for (value, limit) in [
            (&self.capture_id, 128),
            (&self.title, 1024),
            (&self.source.conversation_id, 2048),
        ] {
            if value.trim().is_empty() || value.len() > limit || value.chars().any(char::is_control)
            {
                return Err("会话编号或标题无效".into());
            }
        }
        let expected_host = match self.source.platform.as_str() {
            "chatgpt" | "chatgpt-web" => "chatgpt.com",
            "deepseek" | "deepseek-web" => "chat.deepseek.com",
            "qwen" | "qwen-web" => "chat.qwen.ai",
            "zai" | "z-ai" | "zai-web" => "chat.z.ai",
            _ => return Err("不支持的会话来源平台".into()),
        };
        let tail = self
            .source
            .url
            .strip_prefix("https://")
            .ok_or("来源必须是 HTTPS 网站")?;
        let host = tail.split('/').next().unwrap_or("");
        if host != expected_host
            || self.source.url.len() > 4096
            || self
                .source
                .url
                .chars()
                .any(|c| c.is_control() || c.is_whitespace())
        {
            return Err("会话来源与网站不匹配".into());
        }
        if self.coverage.extent != "visible_only"
            || self.coverage.complete
            || self.coverage.reason.trim().is_empty()
            || self.coverage.reason.len() > 1024
            || self.coverage.warnings.len() > 32
            || self.coverage.warnings.iter().any(|w| w.len() > 2048)
        {
            return Err("可见会话必须明确标注部分覆盖，不能声明完整历史".into());
        }
        if self.messages.is_empty() || self.messages.len() > 5000 {
            return Err("一次保存需要 1–5000 条消息".into());
        }
        let mut ids = BTreeSet::new();
        for message in &self.messages {
            if message.id.trim().is_empty()
                || message.id.len() > 2048
                || message.id.chars().any(char::is_control)
                || !ids.insert(&message.id)
            {
                return Err("消息编号缺失或重复".into());
            }
            if !matches!(message.role, Role::User | Role::Assistant) {
                return Err("会话消息必须有明确的用户或助手角色".into());
            }
            if message.text.trim().is_empty() || message.text.len() > 2 * 1024 * 1024 {
                return Err("消息正文为空或超过上限".into());
            }
        }
        if serde_json::to_vec(self).map_err(|e| e.to_string())?.len() > MAX_BYTES {
            return Err("会话文件超过 16 MiB".into());
        }
        Ok(())
    }
    pub fn events(&self, scope: &str) -> Result<Vec<EventInput>> {
        self.validate()?;
        validate_scope(scope)?;
        self.messages.iter().enumerate().map(|(index, message)| {
            let mut input: EventInput = serde_json::from_value(json!({
                "occurred_at": message.occurred_at,
                "scope": scope,
                "role": message.role,
                "origin": if message.role == Role::User { "user_input" } else { "assistant_output" },
                "content": message.text,
                "source": {"platform": self.source.platform, "conversation_id": self.source.conversation_id, "message_id": message.id, "url": self.source.url},
                "capture": {"completeness": "partial", "reason": self.coverage.reason},
                // 捕获时间不冒充原消息时间；它只用于覆盖说明。
                "metadata": {"conversation_title": self.title, "coverage": self.coverage, "message_identity": message.metadata, "previous_message_id": index.checked_sub(1).map(|i|self.messages[i].id.as_str())}
            })).map_err(|e| e.to_string())?;
            redact_event(&mut input)?;
            Ok(input)
        }).collect()
    }
}

pub fn preview(vault: &Vault, conversation: &Conversation, scope: &str) -> Result<Value> {
    let events = conversation.events(scope)?;
    let approval_hash = hash(
        &serde_json::to_vec(
            &json!({"connection_id":connection_id(vault,scope)?,"scope":scope,"events":events}),
        )
        .map_err(|e| e.to_string())?,
    );
    Ok(
        json!({"approval_hash":approval_hash,"connection_id":connection_id(vault,scope)?,"vault_name":vault.root().file_name().and_then(|s|s.to_str()).unwrap_or("RecallCard"),"event_count":events.len(),"redacted_event_count":events.iter().filter(|e| e.capture.redacted).count(),
        "samples":events.iter().take(8).map(|e| json!({"role":e.role,"content":crate::context::truncate_utf8(&e.text(),2048),"occurred_at":e.occurred_at,"redacted":e.capture.redacted})).collect::<Vec<_>>(),
        "samples_complete":events.len()<=8 && events.iter().all(|e| e.text().len()<=2048),"coverage":conversation.coverage,"scope":scope,"title":conversation.title}),
    )
}

pub fn save(
    vault: &Vault,
    conversation: &Conversation,
    scope: &str,
    approval_hash: &str,
) -> Result<Value> {
    let checked = preview(vault, conversation, scope)?;
    if checked["approval_hash"].as_str() != Some(approval_hash) {
        return Err("会话或资料范围已改变，请重新预览并确认".into());
    }
    let events = conversation.events(scope)?;
    let before = vault.events()?.len();
    let mut refs = Vec::new();
    for event in events {
        refs.push(format!("event:{}", vault.capture(event)?.id));
    }
    let added = vault.events()?.len().saturating_sub(before);
    Ok(
        json!({"events_added":added,"events_seen":refs.len(),"refs":refs,"coverage":conversation.coverage,"note":"只保存了预览中的可见消息；中断后可以重试，相同来源版本会去重"}),
    )
}

/// 同一路径换成另一个资料库也会失效；不把私人路径直接返回给网页。
pub fn connection_id(vault: &Vault, scope: &str) -> Result<String> {
    let marker = vault.root().join("control/schema-version.json");
    crate::vault::reject_symlink(&marker)?;
    let file = std::fs::File::open(&marker).map_err(|_| "无法核对资料库身份")?;
    let metadata = file.metadata().map_err(|_| "无法核对资料库身份")?;
    #[cfg(unix)]
    let identity = {
        use std::os::unix::fs::MetadataExt;
        format!("{}:{}", metadata.dev(), metadata.ino())
    };
    #[cfg(windows)]
    let identity = {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{
            GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
        };
        let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
        // SAFETY: file 拥有有效的只读句柄，info 是完整可写的结构。
        if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
            return Err("无法核对资料库原生身份".into());
        }
        format!(
            "{}:{}:{}",
            info.dwVolumeSerialNumber, info.nFileIndexHigh, info.nFileIndexLow
        )
    };
    #[cfg(not(any(unix, windows)))]
    let identity = format!(
        "{:?}",
        metadata.created().map_err(|_| "此平台无法核对资料库身份")?
    );
    let _ = metadata;
    Ok(hash(
        format!("{}\0{}\0{}", vault.root().display(), identity, scope).as_bytes(),
    ))
}

/// 相对先后关系仅来自已捕获片段，不把写入时间冒充原对话顺序。
pub fn ordered_events(events: Vec<Event>) -> (Vec<Event>, bool) {
    use std::collections::{BTreeMap, BTreeSet};
    let positions: BTreeMap<_, _> = events
        .iter()
        .enumerate()
        .map(|(i, e)| (e.data.source.message_id.as_str(), i))
        .collect();
    let mut children = vec![Vec::new(); events.len()];
    let mut incoming = vec![false; events.len()];
    for (i, event) in events.iter().enumerate() {
        if let Some(previous) = event.data.metadata["previous_message_id"]
            .as_str()
            .and_then(|id| positions.get(id))
            .copied()
        {
            if previous != i {
                children[previous].push(i);
                incoming[i] = true;
            }
        }
    }
    let mut ready: BTreeSet<usize> = incoming
        .iter()
        .enumerate()
        .filter_map(|(i, has)| if !has { Some(i) } else { None })
        .collect();
    let connected = ready.len() == 1 && children.iter().all(|next| next.len() <= 1);
    let mut indexes = Vec::new();
    while let Some(i) = ready.pop_first() {
        indexes.push(i);
        for next in &children[i] {
            ready.insert(*next);
        }
    }
    if indexes.len() != events.len() {
        return (events, false);
    }
    let mut slots: Vec<_> = events.into_iter().map(Some).collect();
    (
        indexes
            .into_iter()
            .map(|i| slots[i].take().expect("每条会话消息只安排一次"))
            .collect(),
        connected,
    )
}
