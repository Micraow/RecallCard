//! 用户主动维护记忆；所有变更先展示差异，在同一写锁内重新核验后保存。
use super::*;
use crate::{
    model::{Memory, MemoryInput, MemoryState},
    policy::Suppression,
    vault::read_json,
};

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryEdit {
    pub content: String,
    pub protected: bool,
    pub labels: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MemoryReview {
    pub preview_id: String,
    pub operation: String,
    pub before: Memory,
    pub after: Option<MemoryInput>,
    pub affected_events: usize,
    pub affected_memories: usize,
    pub requires_protected_approval: bool,
    pub warning: String,
}

pub(super) struct PendingMemory {
    preview_id: String,
    scope: String,
    target: String,
    snapshot: String,
    protected: bool,
    operation: Operation,
}

enum Operation {
    Edit(Box<MemoryInput>, u64),
    Forget(String),
    Restore,
}

impl DesktopSession {
    /// 隐藏资料仅在用户主动进入管理模式后返回，不进入普通检索或交接。
    pub fn manage_memories(
        &self,
        session_id: &str,
        scope: &str,
        include_hidden: bool,
        offset: usize,
    ) -> Result<Value> {
        check_scope(scope)?;
        if offset > 1_000_000 {
            return Err("记忆分页参数无效".into());
        }
        let vault = self.vault(session_id)?;
        let _guard = vault.read_guard()?;
        let suppressed = vault.suppressed_ids()?;
        let mut rows = Vec::new();
        for memory in vault
            .memories()?
            .into_iter()
            .filter(|m| m.data.scope == scope)
        {
            let hidden = suppressed.contains(&memory.id)
                || memory
                    .data
                    .source_refs
                    .iter()
                    .any(|id| suppressed.contains(id));
            let inactive = !matches!(memory.state, MemoryState::Active | MemoryState::Tentative);
            if !include_hidden && (hidden || inactive) {
                continue;
            }
            let can_restore = own_rule(vault, &memory.id)?.is_some_and(|r| r.active);
            rows.push(json!({"id":memory.id,"revision":memory.revision,"content":truncate_utf8(&memory.data.content,1000),
                "text_truncated":memory.data.content.len()>1000,"status":memory.state,"protected":memory.data.protected,
                "evidence":memory.data.evidence,"source_refs":memory.data.source_refs,"hidden":hidden,
                "can_restore":can_restore,"updated_at":memory.updated_at}));
        }
        rows.sort_by(|a, b| {
            b["updated_at"]
                .as_str()
                .cmp(&a["updated_at"].as_str())
                .then(a["id"].as_str().cmp(&b["id"].as_str()))
        });
        let total = rows.len();
        let page: Vec<_> = rows.into_iter().skip(offset).take(30).collect();
        let next = offset + page.len();
        Ok(json!({"memories":page,"total":total,"next_offset":if next<total{Some(next)}else{None}}))
    }

    pub fn managed_memory(&self, session_id: &str, scope: &str, id: &str) -> Result<Memory> {
        check_scope(scope)?;
        let vault = self.vault(session_id)?;
        let _guard = vault.read_guard()?;
        scoped_memory(vault, scope, id)
    }

    /// 管理页中用户点开一条来源时读取完整原文，包括已被自己的规则隐藏的内容。
    /// 不能用任意 event_id 绕过所属记忆和范围检查。
    pub fn managed_memory_source(
        &self,
        session_id: &str,
        scope: &str,
        memory_id: &str,
        event_id: &str,
    ) -> Result<crate::Event> {
        check_scope(scope)?;
        let vault = self.vault(session_id)?;
        let _guard = vault.read_guard()?;
        let memory = scoped_memory(vault, scope, memory_id)?;
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

    pub fn review_memory_edit(
        &mut self,
        session_id: &str,
        scope: &str,
        id: &str,
        revision: u64,
        edit: MemoryEdit,
    ) -> Result<MemoryReview> {
        self.pending_memory = None;
        check_scope(scope)?;
        let vault = self.vault(session_id)?;
        let _guard = vault.read_guard()?;
        let memory = scoped_memory(vault, scope, id)?;
        if memory.revision != revision {
            return Err("记忆已有新版本，请重新打开后修改".into());
        }
        let suppressed = vault.suppressed_ids()?;
        if suppressed.contains(id)
            || memory
                .data
                .source_refs
                .iter()
                .any(|r| suppressed.contains(r))
        {
            return Err("已遗忘的记忆不能直接编辑，请先检查并恢复相关规则".into());
        }
        if !matches!(memory.state, MemoryState::Active | MemoryState::Tentative) {
            return Err("已替代或撤回的记忆不能隐式恢复".into());
        }
        let mut after = memory.data.clone();
        after.content = edit.content;
        after.protected = edit.protected;
        after.tags = edit.labels;
        // 纠正正文不改变原证据性质、时间、来源或事实状态。
        after.validate()?;
        vault.validate_evidence(&after)?;
        if after == memory.data {
            return Err("没有需要保存的修改".into());
        }
        let review=MemoryReview {preview_id:token(),operation:"edit".into(),before:memory.clone(),after:Some(after.clone()),
            affected_events:0,affected_memories:1,requires_protected_approval:memory.data.protected,
            warning:"保存新的记忆版本，原始对话保持不变。请检查正文中没有不适合保留的信息；证据性质不会因手工编辑自动升级。".into()};
        let snapshot = memory_snapshot(vault, scope)?;
        self.pending_memory = Some(PendingMemory {
            preview_id: review.preview_id.clone(),
            scope: scope.into(),
            target: id.into(),
            snapshot,
            protected: memory.data.protected,
            operation: Operation::Edit(Box::new(after), revision),
        });
        Ok(review)
    }

    pub fn review_memory_visibility(
        &mut self,
        session_id: &str,
        scope: &str,
        id: &str,
        restore: bool,
        reason: &str,
    ) -> Result<MemoryReview> {
        self.pending_memory = None;
        check_scope(scope)?;
        let vault = self.vault(session_id)?;
        let _guard = vault.read_guard()?;
        let memory = scoped_memory(vault, scope, id)?;
        let rule = own_rule(vault, id)?;
        if restore && !rule.as_ref().is_some_and(|r| r.active) {
            return Err("这条记忆没有可撤销的独立遗忘规则；请检查它的来源或其他记忆的规则".into());
        }
        if !restore && rule.as_ref().is_some_and(|r| r.active) {
            return Err("这条记忆已经设置遗忘规则".into());
        }
        if !restore && (reason.trim().is_empty() || reason.len() > 4096) {
            return Err("请填写不超过4096字节的遗忘原因".into());
        }
        let events = vault.events()?;
        let keys: BTreeSet<_> = events
            .iter()
            .filter(|e| memory.data.source_refs.contains(&e.id))
            .map(|e| e.data.source_key())
            .collect();
        let affected: BTreeSet<_> = events
            .iter()
            .filter(|e| e.data.scope == scope && keys.contains(&e.data.source_key()))
            .map(|e| e.id.clone())
            .collect();
        let memories = vault.memories()?;
        let impacted: Vec<_> = memories
            .iter()
            .filter(|m| {
                m.data.scope == scope
                    && (m.id == id || m.data.source_refs.iter().any(|r| affected.contains(r)))
            })
            .collect();
        let affected_memories = impacted.len();
        let requires_protected = impacted.iter().any(|m| m.data.protected);
        let review=MemoryReview{preview_id:token(),operation:if restore{"restore"}else{"forget"}.into(),before:memory.clone(),after:None,
            affected_events:affected.len(),affected_memories,requires_protected_approval:requires_protected,
            warning:if restore{"只撤销这条规则。其他遗忘规则、撤回状态仍然生效；恢复后以实际检索状态为准。"}else{"这条记忆及其来源将退出检索、交接和后续整理；共用来源的其他记忆也可能受到影响。资料文件不会被删除，可在管理页撤销这条规则。"}.into()};
        let snapshot = memory_snapshot(vault, scope)?;
        self.pending_memory = Some(PendingMemory {
            preview_id: review.preview_id.clone(),
            scope: scope.into(),
            target: id.into(),
            snapshot,
            protected: requires_protected,
            operation: if restore {
                Operation::Restore
            } else {
                Operation::Forget(reason.into())
            },
        });
        Ok(review)
    }

    pub fn confirm_memory_change(
        &mut self,
        session_id: &str,
        preview_id: &str,
        approve_protected: bool,
    ) -> Result<Value> {
        self.vault(session_id)?;
        let pending = self
            .pending_memory
            .as_ref()
            .filter(|p| p.preview_id == preview_id)
            .ok_or("记忆确认已失效，请重新审阅")?;
        if pending.protected && !approve_protected {
            return Err("请单独确认修改受保护记忆".into());
        }
        let vault = self.vault(session_id)?;
        let _guard = vault.lock()?;
        if memory_snapshot(vault, &pending.scope)? != pending.snapshot {
            self.pending_memory = None;
            return Err("资料、来源或遗忘规则已改变，请重新审阅影响".into());
        }
        let output = match &pending.operation {
            Operation::Edit(input, revision) => {
                json!({"memory":vault.update_memory_locked(&pending.target,*revision,(**input).clone())?})
            }
            Operation::Forget(reason) => {
                json!({"rule":vault.suppress_locked(&pending.target,reason.clone())?})
            }
            Operation::Restore => json!({"rule":vault.restore_locked(&pending.target)?}),
        };
        let memory = scoped_memory(vault, &pending.scope, &pending.target)?;
        let suppressed = vault.suppressed_ids()?;
        let hidden = suppressed.contains(&memory.id)
            || memory
                .data
                .source_refs
                .iter()
                .any(|r| suppressed.contains(r));
        self.pending_memory = None;
        Ok(json!({"result":output,"hidden":hidden,"status":memory.state}))
    }
}

fn scoped_memory(vault: &Vault, scope: &str, id: &str) -> Result<Memory> {
    crate::model::validate_id(id, "mem_")?;
    let memory = vault.memory(id)?;
    if memory.data.scope != scope {
        return Err("记忆不在当前资料分类中".into());
    }
    Ok(memory)
}
fn own_rule(vault: &Vault, id: &str) -> Result<Option<Suppression>> {
    crate::model::validate_id(
        id,
        if id.starts_with("mem_") {
            "mem_"
        } else {
            "evt_"
        },
    )?;
    let path = vault
        .root()
        .join("control/suppressions")
        .join(format!("{id}.json"));
    reject_symlink(&path)?;
    if !path.exists() {
        return Ok(None);
    }
    let rule: Suppression = read_json(&path)?;
    if rule.id != id || rule.schema_version != 1 {
        return Err("遗忘规则与记录不匹配".into());
    }
    Ok(Some(rule))
}
fn memory_snapshot(vault: &Vault, scope: &str) -> Result<String> {
    let mut records = Vec::new();
    let mut ids = BTreeSet::new();
    for memory in vault
        .memories()?
        .into_iter()
        .filter(|m| m.data.scope == scope)
    {
        ids.insert(memory.id.clone());
        records.push(hash(
            &serde_json::to_vec(&memory).map_err(|_| "无法核验记忆")?,
        ));
    }
    for event in vault
        .events()?
        .into_iter()
        .filter(|e| e.data.scope == scope)
    {
        ids.insert(event.id.clone());
        records.push(hash(
            &serde_json::to_vec(&event).map_err(|_| "无法核验来源")?,
        ));
    }
    for id in &ids {
        if let Some(rule) = own_rule(vault, id)? {
            records.push(hash(
                &serde_json::to_vec(&rule).map_err(|_| "无法核验遗忘规则")?,
            ));
        }
    }
    let suppressed: Vec<_> = vault
        .suppressed_ids()?
        .into_iter()
        .filter(|id| ids.contains(id))
        .collect();
    records.sort();
    Ok(hash(
        &serde_json::to_vec(&(records, suppressed)).map_err(|_| "无法核验资料快照")?,
    ))
}
