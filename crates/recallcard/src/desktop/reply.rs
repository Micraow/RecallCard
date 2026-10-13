//! 普通 AI 回答粘贴：只追加助手来源，预览不写入，也不启动记忆整理或外部请求。
use super::{check_scope, redact_event, token, DesktopSession, NOTE_LIMIT};
use crate::model::{EventInput, Origin, Result};
use serde::Serialize;
use serde_json::{json, Value};

#[derive(Debug, Clone, Serialize)]
pub struct ReplyPreview {
    pub preview_id: String,
    pub session_id: String,
    pub scope: String,
    pub content: String,
    pub redacted: bool,
    pub role: String,
    pub automatic_memory: bool,
}

pub(super) struct PendingReply {
    preview: ReplyPreview,
    input: EventInput,
    // 同一令牌重复确认只返回同一条回执，避免响应丢失后再创建消息。
    receipt: Option<Value>,
}

impl DesktopSession {
    pub fn preview_ai_reply(
        &mut self,
        session_id: &str,
        scope: &str,
        content: &str,
    ) -> Result<ReplyPreview> {
        self.vault(session_id)?;
        self.pending_reply = None;
        self.pending_note = None;
        self.pending_memory = None;
        self.pending_background = None;
        check_scope(scope)?;
        if content.trim().is_empty() || content.len() > NOTE_LIMIT {
            return Err("请粘贴非空的 AI 回答，单条最多 64 KiB".into());
        }
        let id = token();
        let mut input: EventInput = serde_json::from_value(json!({
            "occurred_at": null, "role": "assistant", "origin": "assistant_output",
            "scope": scope, "content": content,
            "source": {"platform":"manual-ai-paste", "conversation_id":id, "message_id":id},
            "metadata": {
                "entry_method":"desktop_ai_reply", "conversation_title":"手动保存的 AI 回答",
                "source_identity_verified":false, "exclude_from_automatic_memory":true,
                "provenance_note":"用户手动粘贴；平台、原会话、发生时间与父子关系未经核验。仅供检索，不作为用户事实自动整理。"
            }
        })).map_err(|_| "无法准备 AI 回答".to_owned())?;
        redact_event(&mut input).map_err(|_| "AI 回答无法安全处理，请检查后重试".to_owned())?;
        if input.origin != Origin::AssistantOutput {
            return Err("正文含 RecallCard 上下文或整理协议标记，为防止资料回流，本次未保存。请检查是否混入了复制的上下文。".into());
        }
        let preview = ReplyPreview {
            preview_id: token(),
            session_id: session_id.into(),
            scope: scope.into(),
            content: input.content.clone(),
            redacted: input.capture.redacted,
            role: "assistant".into(),
            automatic_memory: false,
        };
        self.pending_reply = Some(PendingReply {
            preview: preview.clone(),
            input,
            receipt: None,
        });
        Ok(preview)
    }

    /// 仅撤销指定令牌；旧窗口迟到的取消不能撤销后来生成的新预览。
    pub fn cancel_ai_reply(
        &mut self,
        session_id: &str,
        scope: &str,
        preview_id: &str,
    ) -> Result<()> {
        self.vault(session_id)?;
        if self.pending_reply.as_ref().is_some_and(|pending| {
            pending.preview.preview_id == preview_id && pending.preview.scope == scope
        }) {
            self.pending_reply = None;
        }
        Ok(())
    }

    pub fn confirm_ai_reply(
        &mut self,
        session_id: &str,
        scope: &str,
        preview_id: &str,
    ) -> Result<Value> {
        let vault = self.vault(session_id)?;
        check_scope(scope)?;
        let pending = self
            .pending_reply
            .as_ref()
            .filter(|p| {
                p.preview.preview_id == preview_id
                    && p.preview.scope == scope
                    && p.preview.session_id == session_id
            })
            .ok_or("AI 回答预览已失效或空间已变化，请重新预览")?;
        if let Some(receipt) = &pending.receipt {
            return Ok(receipt.clone());
        }
        let event = vault
            .capture(pending.input.clone())
            .map_err(|_| "AI 回答保存未确认，请重试同一笔保存".to_owned())?;
        let receipt =
            json!({"ref":format!("event:{}",event.id),"role":"assistant","automatic_memory":false});
        self.pending_reply
            .as_mut()
            .expect("已校验待确认回答")
            .receipt = Some(receipt.clone());
        Ok(receipt)
    }
}
