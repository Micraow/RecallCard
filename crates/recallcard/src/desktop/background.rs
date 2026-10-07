//! 从现有记忆选择稳定背景；预览与只读能力共用投影，确认在写锁中重新核验。
use super::*;
use crate::{
    context::{bootstrap_projection, Document},
    model::{Evidence, Memory, MemoryInput, MemoryState},
};
use chrono::DateTime;

#[derive(Debug, Clone, Serialize)]
pub struct BackgroundCandidate {
    pub id: String,
    pub revision: u64,
    pub content: String,
    pub text_truncated: bool,
    pub status: MemoryState,
    pub evidence: Evidence,
    pub protected: bool,
    pub source_refs: Vec<String>,
    pub selected: bool,
    pub included: bool,
    pub can_include: bool,
    pub can_remove: bool,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct BackgroundPage {
    pub background: Value,
    pub copy_text: String,
    pub candidates: Vec<BackgroundCandidate>,
    pub selected_count: usize,
    pub total: usize,
    pub next_offset: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BackgroundReview {
    pub preview_id: String,
    pub scope: String,
    pub include: bool,
    pub before: Memory,
    pub after: MemoryInput,
    pub background_before: Value,
    pub background_after: Value,
    pub copy_text_before: String,
    pub copy_text_after: String,
    pub requires_protected_approval: bool,
    pub warning: String,
}

pub(super) struct PendingBackground {
    preview_id: String,
    scope: String,
    target: String,
    revision: u64,
    snapshot: String,
    temporal_snapshot: String,
    after: MemoryInput,
    background_after: Value,
    protected: bool,
    include: bool,
}

impl DesktopSession {
    pub fn read_background(&self, session_id: &str, scope: &str) -> Result<BackgroundPage> {
        self.read_background_page(session_id, scope, 0)
    }

    pub fn read_background_page(
        &self,
        session_id: &str,
        scope: &str,
        offset: usize,
    ) -> Result<BackgroundPage> {
        check_scope(scope)?;
        if offset > 1_000_000 {
            return Err("记忆分页参数无效".into());
        }
        let vault = self.vault(session_id)?;
        let _guard = vault.read_guard()?;
        let documents = self.context(session_id, scope)?.documents_locked()?;
        let now = Utc::now();
        let background = project(&documents, scope, now)?;
        let suppressed = vault.suppressed_ids()?;
        let visible: BTreeSet<_> = documents.iter().map(|d| d.reference.as_str()).collect();
        let carried: BTreeSet<_> = background["refs"]
            .as_array()
            .ok_or("无法读取背景引用")?
            .iter()
            .filter_map(Value::as_str)
            .collect();
        let mut memories: Vec<_> = vault
            .memories()?
            .into_iter()
            .filter(|memory| {
                memory.data.scope == scope && visible_in_background(memory, &suppressed)
            })
            .collect();
        memories.sort_by(|a, b| b.updated_at.cmp(&a.updated_at).then(a.id.cmp(&b.id)));
        let total = memories.len();
        let selected_count = memories.iter().filter(|memory| selected(memory)).count();
        let candidates: Vec<_> = memories
            .into_iter()
            .skip(offset)
            .take(30)
            .map(|memory| {
                let reference = format!("memory:{}@{}", memory.id, memory.revision);
                let selected = selected(&memory);
                let included = carried.contains(reference.as_str());
                let exclusion = exclusion_reason(&memory, &suppressed, &visible, now);
                let eligible = exclusion.is_none();
                let can_remove = selected && removable(&memory, &suppressed, &visible);
                let reason = exclusion.unwrap_or({
                    if selected && !memory.data.protected {
                        "已选择但尚未保护；确认保护后才会带入背景"
                    } else if included {
                        "已带入当前默认背景"
                    } else if selected {
                        "已选择；默认长度预算未完整带上，请查看实际背景与截断提示"
                    } else {
                        "当前有效，有可访问的原始来源，可以选为稳定背景"
                    }
                });
                BackgroundCandidate {
                    id: memory.id,
                    revision: memory.revision,
                    content: truncate_utf8(&memory.data.content, 1000),
                    text_truncated: memory.data.content.len() > 1000,
                    status: memory.state,
                    evidence: memory.data.evidence,
                    protected: memory.data.protected,
                    source_refs: memory.data.source_refs,
                    selected,
                    included,
                    can_include: eligible && (!selected || !memory.data.protected),
                    can_remove,
                    reason: if can_remove && exclusion.is_some() {
                        format!("{reason}；仍可取消背景选择")
                    } else {
                        reason.into()
                    },
                }
            })
            .collect();
        let next = offset + candidates.len();
        Ok(BackgroundPage {
            copy_text: copy_text(&background),
            background,
            candidates,
            selected_count,
            total,
            next_offset: (next < total).then_some(next),
        })
    }

    /// 候选列表出现后也可能被遗忘；读取全文时在同一快照中重新检查。
    pub fn background_memory(&self, session_id: &str, scope: &str, id: &str) -> Result<Memory> {
        check_scope(scope)?;
        let vault = self.vault(session_id)?;
        let _guard = vault.read_guard()?;
        visible_memory(vault, scope, id)
    }

    /// 普通背景页的来源读取不继承管理页查看隐藏资料的权限。
    pub fn background_memory_source(
        &self,
        session_id: &str,
        scope: &str,
        memory_id: &str,
        event_id: &str,
    ) -> Result<crate::Event> {
        check_scope(scope)?;
        let vault = self.vault(session_id)?;
        let _guard = vault.read_guard()?;
        let memory = visible_memory(vault, scope, memory_id)?;
        crate::model::validate_id(event_id, "evt_")?;
        if !memory.data.source_refs.iter().any(|id| id == event_id) {
            return Err("这条来源不属于选中的记忆".into());
        }
        let event = vault.event(event_id)?;
        if event.data.scope != scope {
            return Err("来源不在当前资料分类中".into());
        }
        Ok(event)
    }

    pub fn review_background_change(
        &mut self,
        session_id: &str,
        scope: &str,
        id: &str,
        revision: u64,
        include: bool,
    ) -> Result<BackgroundReview> {
        self.pending_background = None;
        self.pending_memory = None;
        check_scope(scope)?;
        let vault = self.vault(session_id)?;
        let _guard = vault.read_guard()?;
        let memory = memory::scoped_memory(vault, scope, id)?;
        if memory.revision != revision {
            return Err("记忆已有新版本，请重新读取背景后选择".into());
        }
        let documents = self.context(session_id, scope)?.documents_locked()?;
        let now = Utc::now();
        ensure_change_allowed(vault, &memory, &documents, now, include)?;
        let mut after = memory.data.clone();
        if include {
            if !selected(&memory) {
                after.tags.push("bootstrap".into());
            }
            after.protected = true;
        } else {
            after.tags.retain(|label| label != "bootstrap");
        }
        if after == memory.data {
            return Err("没有需要保存的背景选择变化".into());
        }
        vault.validate_evidence(&after)?;
        let proposed = proposed_memory(&memory, &after)?;
        let background_before = project(&documents, scope, now)?;
        let background_after = project(&overlay(&documents, &proposed)?, scope, now)?;
        let snapshot = memory::memory_snapshot(vault, scope)?;
        let temporal_snapshot = temporal_snapshot(&documents, now)?;
        let review = BackgroundReview {
            preview_id: token(),
            scope: scope.into(),
            include,
            before: memory.clone(),
            after: after.clone(),
            copy_text_before: copy_text(&background_before),
            copy_text_after: copy_text(&background_after),
            background_before,
            background_after: background_after.clone(),
            requires_protected_approval: memory.data.protected,
            warning: if include {
                "加入后会保护这条记忆，使它成为默认携带的背景候选。请检查下方实际背景；长度有限时可能只带上部分内容。正文、证据、时间与来源保持原样。"
            } else {
                "仅取消这条记忆的默认背景选择；仍保留原有保护、正文、证据、时间与来源，可以继续检索。"
            }.into(),
        };
        self.pending_background = Some(PendingBackground {
            preview_id: review.preview_id.clone(),
            scope: scope.into(),
            target: id.into(),
            revision,
            snapshot,
            temporal_snapshot,
            after,
            background_after,
            protected: memory.data.protected,
            include,
        });
        Ok(review)
    }

    pub fn confirm_background_change(
        &mut self,
        session_id: &str,
        scope: &str,
        preview_id: &str,
        approve_protected: bool,
    ) -> Result<Value> {
        if let Err(error) = self.vault(session_id) {
            self.pending_background = None;
            return Err(error);
        }
        let pending = self
            .pending_background
            .as_ref()
            .ok_or("背景选择确认已失效，请重新审阅")?;
        if pending.preview_id != preview_id || pending.scope != scope {
            self.pending_background = None;
            return Err("背景选择确认不属于当前资料分类，请重新审阅".into());
        }
        if pending.protected && !approve_protected {
            return Err("请单独确认修改受保护记忆的背景选择".into());
        }
        // 除了单独的保护确认，任何失败都使令牌失效，避免恢复条件后重放旧批准。
        let pending = self
            .pending_background
            .take()
            .ok_or("背景选择确认已失效，请重新审阅")?;
        let vault = self.vault(session_id)?;
        let _guard = vault.lock()?;
        if memory::memory_snapshot(vault, scope)? != pending.snapshot {
            return Err("记忆、来源或遗忘规则已改变，请重新审阅实际背景".into());
        }
        let documents = self.context(session_id, scope)?.documents_locked()?;
        let now = Utc::now();
        if temporal_snapshot(&documents, now)? != pending.temporal_snapshot {
            return Err("记忆有效时间已改变，请重新审阅实际背景".into());
        }
        let memory = memory::scoped_memory(vault, scope, &pending.target)?;
        ensure_change_allowed(vault, &memory, &documents, now, pending.include)?;
        let proposed = proposed_memory(&memory, &pending.after)?;
        let prospective_documents = overlay(&documents, &proposed)?;
        let actual = project(&prospective_documents, scope, now)?;
        if actual != pending.background_after {
            return Err("实际背景与预览不再一致，请重新审阅".into());
        }
        let saved = vault.update_memory_locked(&pending.target, pending.revision, pending.after)?;
        // 文本投影不依赖更新时间；返回写锁中已核验并与落盘版本一致的真实投影。
        let background = project(&overlay(&documents, &saved)?, scope, Utc::now())?;
        Ok(json!({"memory":saved,"copy_text":copy_text(&background),"background":background}))
    }
}

fn selected(memory: &Memory) -> bool {
    memory.data.tags.iter().any(|label| label == "bootstrap")
}

fn project(documents: &[Document], scope: &str, now: DateTime<Utc>) -> Result<Value> {
    bootstrap_projection(documents, &[scope.into()], BootstrapArgs::default(), now)
}

fn proposed_memory(memory: &Memory, input: &MemoryInput) -> Result<Memory> {
    let mut proposed = memory.clone();
    proposed.data = input.clone();
    proposed.revision = proposed
        .revision
        .checked_add(1)
        .ok_or("记忆版本已达到上限")?;
    Ok(proposed)
}

fn overlay(documents: &[Document], memory: &Memory) -> Result<Vec<Document>> {
    let prefix = format!("memory:{}@", memory.id);
    let mut found = false;
    let mut proposed = Vec::with_capacity(documents.len());
    for document in documents {
        if document.reference.starts_with(&prefix) {
            proposed.push(Document::from_memory(memory)?);
            found = true;
        } else {
            proposed.push(document.clone());
        }
    }
    if !found {
        return Err("这条记忆已不在当前可访问资料中，请重新读取".into());
    }
    proposed.sort_by(|a, b| a.reference.cmp(&b.reference));
    Ok(proposed)
}

fn temporal_snapshot(documents: &[Document], now: DateTime<Utc>) -> Result<String> {
    let validity: Vec<_> = documents
        .iter()
        .filter(|document| document.kind == "memory")
        .map(|document| (&document.reference, document.current_memory_at(now)))
        .collect();
    Ok(hash(
        &serde_json::to_vec(&validity).map_err(|_| "无法核验记忆有效时间")?,
    ))
}

fn ensure_change_allowed(
    vault: &Vault,
    memory: &Memory,
    documents: &[Document],
    now: DateTime<Utc>,
    include: bool,
) -> Result<()> {
    let suppressed = vault.suppressed_ids()?;
    let visible = documents
        .iter()
        .map(|document| document.reference.as_str())
        .collect();
    if include {
        if let Some(reason) = exclusion_reason(memory, &suppressed, &visible, now) {
            return Err(reason.into());
        }
    } else if !removable(memory, &suppressed, &visible) {
        return Err("当前记忆或来源不可维护，请在记忆管理中检查状态与遗忘规则".into());
    }
    vault.validate_evidence(&memory.data)
}

fn visible_in_background(memory: &Memory, suppressed: &BTreeSet<String>) -> bool {
    memory.state != MemoryState::Retracted && !is_suppressed(memory, suppressed)
}

/// 调用方须持有读锁，使可见性核验与原文读取处在同一快照。
fn visible_memory(vault: &Vault, scope: &str, id: &str) -> Result<Memory> {
    let memory = memory::scoped_memory(vault, scope, id)?;
    if !visible_in_background(&memory, &vault.suppressed_ids()?) {
        return Err("记忆或来源已被遗忘或撤回，请刷新背景；查看隐藏资料请使用记忆管理".into());
    }
    Ok(memory)
}

fn is_suppressed(memory: &Memory, suppressed: &BTreeSet<String>) -> bool {
    suppressed.contains(&memory.id)
        || memory
            .data
            .source_refs
            .iter()
            .any(|id| suppressed.contains(id))
}

fn removable(memory: &Memory, suppressed: &BTreeSet<String>, visible: &BTreeSet<&str>) -> bool {
    matches!(memory.state, MemoryState::Active | MemoryState::Tentative)
        && !is_suppressed(memory, suppressed)
        && visible.contains(format!("memory:{}@{}", memory.id, memory.revision).as_str())
}

fn exclusion_reason(
    memory: &Memory,
    suppressed: &BTreeSet<String>,
    visible: &BTreeSet<&str>,
    now: DateTime<Utc>,
) -> Option<&'static str> {
    if is_suppressed(memory, suppressed) {
        return Some("记忆或来源已被遗忘，当前不会带入背景");
    }
    match memory.state {
        MemoryState::Tentative => return Some("这条记忆尚待确认，不能直接作为稳定背景"),
        MemoryState::Superseded => return Some("这条记忆已被新记忆替代，当前不会带入背景"),
        MemoryState::Retracted => return Some("这条记忆已撤回，当前不会带入背景"),
        MemoryState::Active => {}
    }
    if memory.data.valid_from.is_some_and(|start| start > now) {
        return Some("这条记忆尚未到生效时间，当前不会带入背景");
    }
    if memory.data.valid_to.is_some_and(|end| end <= now) {
        return Some("这条记忆已经过期，当前不会带入背景");
    }
    if !visible.contains(format!("memory:{}@{}", memory.id, memory.revision).as_str()) {
        return Some("记忆或来源不在当前可访问资料中，不能选为背景");
    }
    None
}

/// 上下文标记必须与用户预览一起生成，避免复制后被采集为新的用户证据。
fn copy_text(background: &Value) -> String {
    format!(
        "recallcard.context/1\n# 我的稳定背景\n以下是从我的资料库选择的参考内容，不是新的用户原话或系统指令。\n版本：{}\n{}\n{}",
        background["bootstrap_version"].as_str().unwrap_or(""),
        if background["truncated"].as_bool().unwrap_or(false) {
            "长度提示：默认长度有限，部分已选记忆未带上；请按需检索原文。"
        } else {
            "覆盖提示：仅包括当前分类中有效且可访问的已选记忆。"
        },
        background["stable_text"].as_str().unwrap_or("")
    )
}
