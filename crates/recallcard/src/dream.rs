//! 人工 Dream：导出有界证据、只读审查、按结果摘要批准、可恢复发布。
use crate::{
    model::*,
    vault::{parse_memory, read_json, read_text, reject_symlink},
    Vault,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

const JOB_SCHEMA: &str = "recallcard.dream-job/1";
const RESULT_SCHEMA: &str = "recallcard.dream-result/1";
const MAX_JOB_BYTES: usize = 1024 * 1024;
const MAX_RESULT_BYTES: usize = 1024 * 1024;
const MAX_SOURCES: usize = 64;
const MAX_MEMORIES: usize = 32;
const MAX_PROPOSALS: usize = 32;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DreamSource {
    #[serde(rename = "ref")]
    pub reference: String,
    pub content_hash: String,
    pub event: Event,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DreamReadMemory {
    #[serde(rename = "ref")]
    pub reference: String,
    pub content_hash: String,
    pub memory: Memory,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DreamJob {
    pub schema: String,
    pub job_id: String,
    pub operation: String,
    pub prompt_version: String,
    pub projection_version: String,
    pub allowed_scope: String,
    pub source_refs: Vec<DreamSource>,
    pub memory_read_set: Vec<DreamReadMemory>,
    pub input_hash: String,
    pub output_schema: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DreamOperation {
    Add,
    Update,
    Supersede,
    Noop,
    Conflict,
}
fn suggestion() -> Evidence {
    Evidence::AssistantSuggestion
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DreamProposal {
    pub operation: DreamOperation,
    pub scope: String,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub source_refs: Vec<String>,
    #[serde(default = "suggestion")]
    pub evidence: Evidence,
    #[serde(default)]
    pub target_ref: Option<String>,
    #[serde(default)]
    pub expected_revision: Option<u64>,
    #[serde(default)]
    pub model_score: f64,
    #[serde(default)]
    pub observed_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub valid_from: Option<DateTime<Utc>>,
    #[serde(default)]
    pub valid_to: Option<DateTime<Utc>>,
    #[serde(default)]
    pub time_note: String,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default)]
    pub entities: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DreamResult {
    pub schema: String,
    pub job_id: String,
    pub input_hash: String,
    pub proposals: Vec<DreamProposal>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DreamChange {
    pub operation: DreamOperation,
    pub before: Option<Memory>,
    pub after: Option<Memory>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DreamAppliedMemory {
    pub id: String,
    pub revision: u64,
    pub content_hash: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DreamReceipt {
    pub schema_version: u32,
    pub job_id: String,
    pub input_hash: String,
    pub result_hash: String,
    pub applied_at: DateTime<Utc>,
    pub source_coverage: Vec<String>,
    pub changes: Vec<DreamAppliedMemory>,
    pub already_applied: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DreamReview {
    pub job_id: String,
    pub input_hash: String,
    pub result_hash: String,
    pub already_applied: bool,
    pub requires_protected_approval: bool,
    pub can_apply: bool,
    pub changes: Vec<DreamChange>,
    pub diagnostics: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Transaction {
    schema_version: u32,
    vault_root: String,
    job: DreamJob,
    result: DreamResult,
    approve_protected: bool,
    writes: Vec<DreamChange>,
    receipt: DreamReceipt,
}

impl Vault {
    /// 导出本地有界任务。选中被抑制或越界来源时整体拒绝，绝不偷偷发送。
    pub fn dream_export(
        &self,
        source_ids: &[String],
        memory_ids: &[String],
        scope: &str,
    ) -> Result<DreamJob> {
        let _lock = self.lock()?;
        validate_scope(scope)?;
        if source_ids.is_empty()
            || source_ids.len() > MAX_SOURCES
            || memory_ids.len() > MAX_MEMORIES
        {
            return Err("Dream 需要 1–64 条来源，最多 32 条旧记忆；请拆分任务".into());
        }
        let sources = unique(source_ids, "Dream 来源")?;
        let memories = unique(memory_ids, "Dream 旧记忆")?;
        let mut job = DreamJob {
            schema: JOB_SCHEMA.into(),
            job_id: String::new(),
            operation: "extract".into(),
            prompt_version: "manual-extract-v1".into(),
            projection_version: "bounded-full-v1".into(),
            allowed_scope: scope.into(),
            source_refs: Vec::new(),
            memory_read_set: Vec::new(),
            input_hash: String::new(),
            output_schema: RESULT_SCHEMA.into(),
        };
        for id in sources {
            let id = event_ref(&id)?;
            let event = self.event(&id)?;
            self.check_dream_source(&event, scope)?;
            job.source_refs.push(DreamSource {
                reference: format!("event:{id}"),
                content_hash: digest(&event)?,
                event,
            });
        }
        for id in memories {
            let (id, requested_revision) = memory_ref(&id)?;
            let memory = self.memory(&id)?;
            if requested_revision.is_some_and(|r| r != memory.revision) {
                return Err("导出指定的记忆版本已经过期".into());
            }
            self.check_dream_memory(&memory, scope)?;
            job.memory_read_set.push(DreamReadMemory {
                reference: format!("memory:{}@{}", memory.id, memory.revision),
                content_hash: digest(&memory)?,
                memory,
            });
        }
        job.input_hash = job_hash(&job)?;
        job.job_id = format!("dream_{}", job.input_hash);
        validate_job(&job)?;
        let dir = self.state_dir()?.join("dream-jobs");
        reject_symlink(&dir)?;
        fs::create_dir_all(&dir).map_err(error)?;
        let path = dir.join(format!("{}.json", job.job_id));
        if path.exists() {
            let stored: DreamJob = read_json(&path)?;
            if digest(&stored)? != digest(&job)? {
                return Err("已有 Dream Job 与同名输入不一致".into());
            }
        } else {
            self.write_new(&path, &job)?;
        }
        Ok(job)
    }

    /// 只读生成完整 diff；审查本身不批准、不发布 Memory。
    pub fn dream_review(&self, result: &DreamResult) -> Result<DreamReview> {
        let _lock = self.lock()?;
        let (job, result_hash) = self.load_result(result)?;
        if self.existing_receipt(result, &result_hash)?.is_some() {
            return Ok(DreamReview {
                job_id: result.job_id.clone(),
                input_hash: result.input_hash.clone(),
                result_hash,
                already_applied: true,
                requires_protected_approval: false,
                can_apply: true,
                changes: vec![],
                diagnostics: vec!["相同结果已发布，无需再次写入".into()],
            });
        }
        self.review_current(&job, result, result_hash)
    }

    /// 必须显式批准该完整结果摘要；受保护目标另外需要明确批准。
    pub fn dream_apply(
        &self,
        result: &DreamResult,
        approved_result_hash: &str,
        approve_protected: bool,
    ) -> Result<DreamReceipt> {
        let _lock = self.lock()?;
        let (job, result_hash) = self.load_result(result)?;
        if result_hash != approved_result_hash {
            return Err("批准摘要与 DreamResult 不一致，请重新 review 后批准".into());
        }
        if let Some(mut receipt) = self.existing_receipt(result, &result_hash)? {
            receipt.already_applied = true;
            drop(_lock);
            self.refresh_dream_views()?;
            return Ok(receipt);
        }
        let review = self.review_current(&job, result, result_hash.clone())?;
        if !review.can_apply {
            return Err("结果包含未解决的 conflict，不能发布；请重新整理完整结果".into());
        }
        if review.requires_protected_approval && !approve_protected {
            return Err("结果将修改 protected 记忆；需要明确 --approve-protected".into());
        }
        let receipt = DreamReceipt {
            schema_version: 1,
            job_id: job.job_id.clone(),
            input_hash: job.input_hash.clone(),
            result_hash,
            applied_at: Utc::now(),
            source_coverage: job
                .source_refs
                .iter()
                .map(|s| s.reference.clone())
                .collect(),
            changes: review
                .changes
                .iter()
                .filter_map(|c| c.after.as_ref())
                .map(|m| {
                    Ok(DreamAppliedMemory {
                        id: m.id.clone(),
                        revision: m.revision,
                        content_hash: digest(m)?,
                    })
                })
                .collect::<Result<_>>()?,
            already_applied: false,
        };
        let transaction = Transaction {
            schema_version: 1,
            vault_root: self.root().to_string_lossy().into(),
            job,
            result: result.clone(),
            approve_protected,
            writes: review.changes,
            receipt,
        };
        if serde_json::to_vec_pretty(&transaction)
            .map_err(error)?
            .len()
            > 8 * 1024 * 1024
        {
            return Err("Dream 暂存事务超过 8 MiB，请拆分任务；未发布任何记录".into());
        }
        // 日志先完整落盘并同步，之后任意中断都能按同一快照恢复。
        self.write_new(&self.dream_transaction_path()?, &transaction)?;
        self.finish_dream_transaction(&transaction)?;
        drop(_lock);
        self.refresh_dream_views()?;
        Ok(transaction.receipt)
    }

    /// 恢复已获批准、已准备的本机事务；不会批准或导入新的模型结果。
    pub fn dream_recover(&self) -> Result<Value> {
        let _lock = self.lock_unchecked()?;
        let path = self.dream_transaction_path()?;
        if !path.exists() {
            return Ok(json!({"ok":true,"recovered":false}));
        }
        let transaction: Transaction = read_json(&path)?;
        self.finish_dream_transaction(&transaction)?;
        drop(_lock);
        self.refresh_dream_views()?;
        Ok(json!({"ok":true,"recovered":true,"receipt":transaction.receipt}))
    }

    fn refresh_dream_views(&self) -> Result<()> {
        self.rebuild_views().map_err(|e| {
            format!("Memory 与成功收据已安全发布；派生视图刷新失败，请运行 views 重建：{e}")
        })?;
        Ok(())
    }
    fn dream_transaction_path(&self) -> Result<PathBuf> {
        Ok(self.state_dir()?.join("dream-transaction.json"))
    }
    fn dream_receipt_path(&self, id: &str) -> Result<PathBuf> {
        validate_job_id(id)?;
        Ok(self
            .root()
            .join("control/dream-receipts")
            .join(format!("{id}.json")))
    }
    fn load_result(&self, result: &DreamResult) -> Result<(DreamJob, String)> {
        validate_result(result)?;
        let path = self
            .state_dir()?
            .join("dream-jobs")
            .join(format!("{}.json", result.job_id));
        let job: DreamJob = read_json(&path)?;
        validate_job(&job)?;
        if job.job_id != result.job_id || job.input_hash != result.input_hash {
            return Err("DreamResult 与已导出的 Job/input_hash 不一致".into());
        }
        Ok((job, digest(result)?))
    }
    fn existing_receipt(
        &self,
        result: &DreamResult,
        result_hash: &str,
    ) -> Result<Option<DreamReceipt>> {
        let path = self.dream_receipt_path(&result.job_id)?;
        reject_symlink(&path)?;
        if !path.exists() {
            return Ok(None);
        }
        let receipt: DreamReceipt = read_json(&path)?;
        if receipt.schema_version != 1
            || receipt.job_id != result.job_id
            || receipt.input_hash != result.input_hash
            || receipt.result_hash != result_hash
        {
            return Err("该 Job 已发布另一结果，或持久收据损坏；拒绝重复覆盖".into());
        }
        Ok(Some(receipt))
    }
    fn check_dream_source(&self, event: &Event, scope: &str) -> Result<()> {
        if self.is_suppressed(&event.id)? {
            return Err("Dream 来源已被抑制，不能导出或重新提炼".into());
        }
        if event.data.scope != scope {
            return Err("Dream 来源超出明确授权的 scope".into());
        }
        if !event.data.has_original_evidence() {
            return Err("注入上下文/Dream 对话不能作为新 Dream 证据".into());
        }
        Ok(())
    }
    fn check_dream_memory(&self, memory: &Memory, scope: &str) -> Result<()> {
        if self.is_suppressed(&memory.id)?
            || memory
                .data
                .source_refs
                .iter()
                .any(|id| self.is_suppressed(id).unwrap_or(true))
        {
            return Err("Dream 旧记忆或其来源已被抑制".into());
        }
        if memory.data.scope != scope {
            return Err("Dream 旧记忆超出授权 scope".into());
        }
        self.validate_evidence(&memory.data)
    }
    fn review_current(
        &self,
        job: &DreamJob,
        result: &DreamResult,
        result_hash: String,
    ) -> Result<DreamReview> {
        let mut allowed_sources = BTreeSet::new();
        for source in &job.source_refs {
            let current = self.event(&source.event.id)?;
            self.check_dream_source(&current, &job.allowed_scope)?;
            if digest(&current)? != source.content_hash {
                return Err("Dream 来源哈希已改变，请重新导出任务".into());
            }
            allowed_sources.insert(current.id);
        }
        // 在提交所持同一写锁内拒绝同范围、同来源的旧版本；跨范围历史坏边无效。
        let source_keys: BTreeMap<_, _> = job
            .source_refs
            .iter()
            .map(|source| (source.event.id.clone(), source.event.data.revision_key()))
            .collect();
        self.visit_events(|event| {
            if event.data.revision_of.as_ref().is_some_and(|id| {
                source_keys
                    .get(id)
                    .is_some_and(|key| *key == event.data.revision_key())
            }) {
                return Err("Dream 来源已有新修订，不能发布旧事实".into());
            }
            Ok(())
        })?;
        let mut baseline = BTreeMap::new();
        for old in &job.memory_read_set {
            let current = self.memory(&old.memory.id)?;
            self.check_dream_memory(&current, &job.allowed_scope)?;
            if current.revision != old.memory.revision || digest(&current)? != old.content_hash {
                return Err("Dream 旧记忆 read-set 已过期，请重新读取和整理，不能覆盖".into());
            }
            baseline.insert(current.id.clone(), current);
        }
        let mut changes = Vec::new();
        let mut touched = BTreeSet::new();
        let mut protected = false;
        let mut diagnostics = vec!["结构/来源校验不能证明自然语言结论真实，请人工核对 diff".into()];
        let mut can_apply = true;
        for (index, proposal) in result.proposals.iter().enumerate() {
            if proposal.scope != job.allowed_scope {
                return Err("Dream 提议不能扩大或改变授权 scope".into());
            }
            let refs = proposal
                .source_refs
                .iter()
                .map(|r| event_ref(r))
                .collect::<Result<Vec<_>>>()?;
            if refs.iter().any(|r| !allowed_sources.contains(r)) {
                return Err("Dream 提议引用了任务外或编造的来源".into());
            }
            if matches!(
                proposal.operation,
                DreamOperation::Noop | DreamOperation::Conflict
            ) {
                if proposal.target_ref.is_some() || proposal.expected_revision.is_some() {
                    return Err("noop/conflict 不接受隐式目标修改".into());
                }
                if proposal.operation == DreamOperation::Conflict {
                    can_apply = false;
                    diagnostics.push(
                        proposal
                            .content
                            .clone()
                            .unwrap_or_else(|| "存在未解决冲突".into()),
                    );
                }
                continue;
            }
            let content = proposal
                .content
                .clone()
                .ok_or("新增/修改提议必须提供 content")?;
            let mut input = MemoryInput {
                content,
                source_refs: refs,
                evidence: proposal.evidence.clone(),
                confidence: proposal.model_score,
                scope: proposal.scope.clone(),
                authority: "dream".into(),
                protected: false,
                observed_at: proposal.observed_at,
                time_note: proposal.time_note.clone(),
                supersedes: vec![],
                entities: proposal.entities.clone(),
                tags: proposal.labels.clone(),
                valid_from: proposal.valid_from,
                valid_to: proposal.valid_to,
            };
            self.validate_evidence(&input)?;
            match proposal.operation {
                DreamOperation::Add => {
                    if proposal.target_ref.is_some() || proposal.expected_revision.is_some() {
                        return Err("add 不能携带修改目标或旧版本".into());
                    }
                    let after = proposed_memory(self, input, &result_hash, index);
                    if self.memory_path(&after.id).exists() {
                        return Err("Dream 新增记录编号已存在且没有成功收据".into());
                    }
                    changes.push(DreamChange {
                        operation: DreamOperation::Add,
                        before: None,
                        after: Some(after),
                    });
                }
                DreamOperation::Update | DreamOperation::Supersede => {
                    let target = proposal
                        .target_ref
                        .as_deref()
                        .ok_or("修改提议必须指定 target_ref")?;
                    let (id, ref_revision) = memory_ref(target)?;
                    let expected = proposal
                        .expected_revision
                        .ok_or("修改提议必须指定 expected_revision")?;
                    if ref_revision.is_some_and(|r| r != expected) {
                        return Err("target_ref 与 expected_revision 不一致".into());
                    }
                    let before = baseline
                        .get(&id)
                        .ok_or("修改目标不在 Job 的 memory_read_set 内")?
                        .clone();
                    if before.revision != expected {
                        return Err("修改目标版本与任务基线不一致".into());
                    }
                    if !touched.insert(id.clone()) {
                        return Err("一个 DreamResult 不得重复修改同一目标".into());
                    }
                    if !matches!(before.state, MemoryState::Active | MemoryState::Tentative) {
                        return Err("Dream 不能自动恢复已撤回/替代的记忆".into());
                    }
                    protected |= before.data.protected;
                    if proposal.operation == DreamOperation::Update {
                        input.protected = before.data.protected;
                        input.authority = if before.data.protected {
                            before.data.authority.clone()
                        } else {
                            "dream".into()
                        };
                        let mut after = before.clone();
                        after.data = input;
                        after.revision =
                            after.revision.checked_add(1).ok_or("记忆版本已达到上限")?;
                        after.updated_at = Utc::now();
                        after.state = if after.data.evidence == Evidence::AssistantSuggestion {
                            MemoryState::Tentative
                        } else {
                            MemoryState::Active
                        };
                        after.validate()?;
                        changes.push(DreamChange {
                            operation: DreamOperation::Update,
                            before: Some(before),
                            after: Some(after),
                        });
                    } else {
                        if input.evidence == Evidence::AssistantSuggestion {
                            return Err("助手建议不能自动替代既有事实，请先获得新证据".into());
                        }
                        input.supersedes = vec![id];
                        input.protected = before.data.protected;
                        if before.data.protected {
                            input.authority = before.data.authority.clone();
                        }
                        let after = proposed_memory(self, input, &result_hash, index);
                        if self.memory_path(&after.id).exists() {
                            return Err("替代记忆编号已存在且没有成功收据".into());
                        }
                        let mut superseded = before.clone();
                        superseded.state = MemoryState::Superseded;
                        superseded.revision = superseded
                            .revision
                            .checked_add(1)
                            .ok_or("记忆版本已达到上限")?;
                        superseded.updated_at = Utc::now();
                        changes.push(DreamChange {
                            operation: DreamOperation::Supersede,
                            before: Some(before),
                            after: Some(superseded),
                        });
                        changes.push(DreamChange {
                            operation: DreamOperation::Add,
                            before: None,
                            after: Some(after),
                        });
                    }
                }
                DreamOperation::Noop | DreamOperation::Conflict => unreachable!(),
            }
        }
        Ok(DreamReview {
            job_id: job.job_id.clone(),
            input_hash: job.input_hash.clone(),
            result_hash,
            already_applied: false,
            requires_protected_approval: protected,
            can_apply,
            changes,
            diagnostics,
        })
    }

    fn finish_dream_transaction(&self, transaction: &Transaction) -> Result<()> {
        self.validate_transaction(transaction)?;
        // 所有目标先校验，再写任何文件；仅接受旧快照或已完成的新快照。
        for change in &transaction.writes {
            let after = change.after.as_ref().ok_or("事务缺少目标记忆")?;
            let path = self.memory_path(&after.id);
            reject_symlink(&path)?;
            if path.exists() {
                let current = parse_memory(&read_text(&path)?)?;
                current.validate()?;
                let unchanged = change
                    .before
                    .as_ref()
                    .is_some_and(|before| current == *before);
                if current != *after && !unchanged {
                    return Err(
                        "事务恢复遇到外部编辑冲突，已停止且未覆盖；保留日志供人工核对".into(),
                    );
                }
            } else if change.before.is_some() {
                return Err("事务恢复发现原目标丢失，不能盲目重建覆盖".into());
            }
        }
        let receipt_path = self.dream_receipt_path(&transaction.receipt.job_id)?;
        reject_symlink(&receipt_path)?;
        if receipt_path.exists() {
            let current: DreamReceipt = read_json(&receipt_path)?;
            if digest(&current)? != digest(&transaction.receipt)? {
                return Err("事务收据与已发布收据冲突".into());
            }
        }
        for change in &transaction.writes {
            let after = change.after.as_ref().ok_or("事务缺少目标记忆")?;
            let path = self.memory_path(&after.id);
            let already_written = path.exists() && parse_memory(&read_text(&path)?)? == *after;
            if !already_written {
                self.write_memory(after, change.before.is_some())?;
            }
        }
        if !receipt_path.exists() {
            self.write_new(&receipt_path, &transaction.receipt)?;
        }
        let journal = self.dream_transaction_path()?;
        fs::remove_file(&journal).map_err(error)?;
        sync_directory(journal.parent().ok_or("事务日志无父目录")?)?;
        Ok(())
    }

    fn validate_transaction(&self, tx: &Transaction) -> Result<()> {
        if tx.schema_version != 1 || tx.vault_root != self.root().to_string_lossy() {
            return Err("事务不属于当前 Vault 或版本不受支持".into());
        }
        validate_job(&tx.job)?;
        validate_result(&tx.result)?;
        if tx.result.job_id != tx.job.job_id
            || tx.result.input_hash != tx.job.input_hash
            || tx.receipt.job_id != tx.job.job_id
            || tx.receipt.input_hash != tx.job.input_hash
            || tx.receipt.result_hash != digest(&tx.result)?
            || tx.receipt.schema_version != 1
            || tx.receipt.already_applied
        {
            return Err("事务的 Job/Result/收据摘要不一致".into());
        }
        if tx.writes.len() > MAX_PROPOSALS * 2 || tx.receipt.changes.len() != tx.writes.len() {
            return Err("事务写入数量非法".into());
        }
        let coverage: Vec<String> = tx
            .job
            .source_refs
            .iter()
            .map(|s| s.reference.clone())
            .collect();
        if coverage != tx.receipt.source_coverage {
            return Err("事务来源覆盖收据不一致".into());
        }
        // 恢复时不经过公共读取屏障，但仍逐条确认来源及 suppression 没有被外部替换。
        let suppressed = self.suppressed_ids_for_dream_recovery()?;
        let allowed: BTreeSet<_> = tx
            .job
            .source_refs
            .iter()
            .map(|s| s.event.id.as_str())
            .collect();
        let mut recovery_sources: BTreeSet<String> =
            allowed.iter().map(|id| (*id).to_string()).collect();
        for after in tx.writes.iter().filter_map(|change| change.after.as_ref()) {
            recovery_sources.extend(after.data.source_refs.iter().cloned());
        }
        let source_keys: BTreeMap<_, _> = tx
            .job
            .source_refs
            .iter()
            .map(|source| (source.event.id.clone(), source.event.data.revision_key()))
            .collect();
        let mut current_events = BTreeMap::new();
        self.visit_events_unchecked(|event| {
            if event.data.revision_of.as_ref().is_some_and(|id| {
                source_keys
                    .get(id)
                    .is_some_and(|key| *key == event.data.revision_key())
            }) {
                return Err("恢复时整理来源已有新修订，停止发布旧事实".into());
            }
            if recovery_sources.contains(&event.id) {
                current_events.insert(event.id.clone(), event);
            }
            Ok(())
        })?;
        for source in &tx.job.source_refs {
            let current = current_events
                .get(&source.event.id)
                .ok_or("恢复时来源已经丢失")?;
            if suppressed.contains(&current.id) || digest(current)? != source.content_hash {
                return Err("恢复时来源已改变或被抑制，停止发布".into());
            }
        }
        for old in &tx.job.memory_read_set {
            let current = parse_memory(&read_text(&self.memory_path(&old.memory.id))?)?;
            current.validate()?;
            let after = tx
                .writes
                .iter()
                .filter_map(|c| c.after.as_ref())
                .find(|m| m.id == old.memory.id);
            if current != old.memory && after != Some(&current) {
                return Err("恢复时 read-set 发生外部修改，停止发布".into());
            }
            if suppressed.contains(&current.id)
                || current
                    .data
                    .source_refs
                    .iter()
                    .any(|r| suppressed.contains(r))
            {
                return Err("恢复时旧记忆已被抑制".into());
            }
        }
        let mut ids = BTreeSet::new();
        for (change, receipt) in tx.writes.iter().zip(&tx.receipt.changes) {
            let after = change.after.as_ref().ok_or("事务缺少目标")?;
            after.validate()?;
            if !ids.insert(after.id.clone())
                || after.data.scope != tx.job.allowed_scope
                || suppressed.contains(&after.id)
                || after.data.source_refs.iter().any(|id| {
                    suppressed.contains(id)
                        || (!allowed.contains(id.as_str())
                            && !change.before.as_ref().is_some_and(|b| b.data == after.data))
                })
            {
                return Err("事务目标重复、越界或来源被抑制".into());
            }
            if after.data.source_refs.iter().any(|id| {
                current_events.get(id).is_none_or(|event| {
                    event.data.scope != after.data.scope || !event.data.has_original_evidence()
                })
            }) {
                return Err("事务目标的完整来源已丢失或不再有效".into());
            }
            if receipt.id != after.id
                || receipt.revision != after.revision
                || receipt.content_hash != digest(after)?
            {
                return Err("事务目标与收据内容摘要不一致".into());
            }
            if let Some(before) = &change.before {
                before.validate()?;
                let baseline = tx
                    .job
                    .memory_read_set
                    .iter()
                    .find(|m| m.memory.id == before.id)
                    .ok_or("事务修改目标未在 read-set 中")?;
                if baseline.memory != *before
                    || before.id != after.id
                    || before.revision.checked_add(1) != Some(after.revision)
                {
                    return Err("事务修改的旧版本/目标不一致".into());
                }
                if before.data.protected && !tx.approve_protected {
                    return Err("事务缺少 protected 修改批准".into());
                }
            } else {
                if after.revision != 1 {
                    return Err("Dream 新增版本必须是 1".into());
                }
                if after.data.protected || after.data.authority != "dream" {
                    let inherited = after.data.supersedes.len() == 1
                        && tx.writes.iter().any(|change| {
                            change.before.as_ref().is_some_and(|old| {
                                old.id == after.data.supersedes[0]
                                    && old.data.protected
                                    && old.data.authority == after.data.authority
                            }) && change
                                .after
                                .as_ref()
                                .is_some_and(|new| new.state == MemoryState::Superseded)
                        });
                    if !tx.approve_protected || !inherited {
                        return Err("Dream 新增记忆不能伪造保护权限".into());
                    }
                }
            }
        }
        Ok(())
    }
}

fn proposed_memory(vault: &Vault, input: MemoryInput, result_hash: &str, index: usize) -> Memory {
    let mut memory = vault.new_memory(input);
    memory.id = format!(
        "mem_{}",
        &hash(format!("{result_hash}:{index}").as_bytes())[..32]
    );
    memory
}
fn digest<T: Serialize>(value: &T) -> Result<String> {
    Ok(hash(&serde_json::to_vec(value).map_err(error)?))
}
fn unique(values: &[String], name: &str) -> Result<Vec<String>> {
    let set: BTreeSet<String> = values.iter().cloned().collect();
    if set.len() != values.len() {
        return Err(format!("{name} 不得重复"));
    }
    Ok(set.into_iter().collect())
}
fn event_ref(reference: &str) -> Result<String> {
    let id = reference.strip_prefix("event:").unwrap_or(reference);
    validate_id(id, "evt_")?;
    Ok(id.into())
}
fn memory_ref(reference: &str) -> Result<(String, Option<u64>)> {
    let value = reference.strip_prefix("memory:").unwrap_or(reference);
    let (id, revision) = match value.split_once('@') {
        Some((id, revision)) => (
            id,
            Some(revision.parse::<u64>().map_err(|_| "记忆引用版本无效")?),
        ),
        None => (value, None),
    };
    validate_id(id, "mem_")?;
    if revision == Some(0) {
        return Err("记忆引用版本必须大于 0".into());
    }
    Ok((id.into(), revision))
}
fn validate_job_id(id: &str) -> Result<()> {
    let suffix = id.strip_prefix("dream_").ok_or("Dream Job 编号前缀无效")?;
    if suffix.len() != 64
        || !suffix
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err("Dream Job 编号无效，不接受路径".into());
    }
    Ok(())
}
fn job_hash(job: &DreamJob) -> Result<String> {
    let mut copy = job.clone();
    copy.job_id.clear();
    copy.input_hash.clear();
    digest(&copy)
}
fn validate_job(job: &DreamJob) -> Result<()> {
    validate_job_id(&job.job_id)?;
    validate_scope(&job.allowed_scope)?;
    if job.schema != JOB_SCHEMA
        || job.output_schema != RESULT_SCHEMA
        || job.operation != "extract"
        || job.prompt_version != "manual-extract-v1"
        || job.projection_version != "bounded-full-v1"
    {
        return Err("不支持的 Dream Job schema/策略版本".into());
    }
    if job.input_hash != job_hash(job)? || job.job_id != format!("dream_{}", job.input_hash) {
        return Err("Dream Job 输入摘要不匹配".into());
    }
    if job.source_refs.is_empty()
        || job.source_refs.len() > MAX_SOURCES
        || job.memory_read_set.len() > MAX_MEMORIES
        || serde_json::to_vec_pretty(job).map_err(error)?.len() > MAX_JOB_BYTES
    {
        return Err("Dream Job 超过有界任务上限（1 MiB/64 来源/32 旧记忆）".into());
    }
    let mut sources = BTreeSet::new();
    for source in &job.source_refs {
        source.event.validate()?;
        if source.reference != format!("event:{}", source.event.id)
            || source.content_hash != digest(&source.event)?
            || source.event.data.scope != job.allowed_scope
            || !sources.insert(source.event.id.clone())
        {
            return Err("Dream Job 来源快照/范围/摘要无效".into());
        }
    }
    let mut memories = BTreeSet::new();
    for old in &job.memory_read_set {
        old.memory.validate()?;
        if old.reference != format!("memory:{}@{}", old.memory.id, old.memory.revision)
            || old.content_hash != digest(&old.memory)?
            || old.memory.data.scope != job.allowed_scope
            || !memories.insert(old.memory.id.clone())
        {
            return Err("Dream Job 旧记忆快照/范围/摘要无效".into());
        }
    }
    Ok(())
}
fn validate_result(result: &DreamResult) -> Result<()> {
    validate_job_id(&result.job_id)?;
    if result.schema != RESULT_SCHEMA {
        return Err("不支持的 DreamResult schema；自由聊天回复不是完整结果".into());
    }
    if result.proposals.len() > MAX_PROPOSALS
        || serde_json::to_vec_pretty(result).map_err(error)?.len() > MAX_RESULT_BYTES
    {
        return Err("DreamResult 超过 32 条提议或 1 MiB，请拆分任务".into());
    }
    if result.proposals.is_empty() {
        return Err("DreamResult 必须明确给出提议或 noop，不能把空输出视为完成".into());
    }
    for p in &result.proposals {
        validate_scope(&p.scope)?;
        if p.source_refs.len() > MAX_SOURCES
            || p.content.as_ref().is_some_and(|c| c.len() > 64 * 1024)
        {
            return Err("Dream 提议超过来源/内容上限".into());
        }
    }
    Ok(())
}
fn sync_directory(path: &Path) -> Result<()> {
    #[cfg(not(unix))]
    let _ = path;
    #[cfg(unix)]
    fs::File::open(path)
        .map_err(error)?
        .sync_all()
        .map_err(error)?;
    Ok(())
}
fn error(error: impl std::fmt::Display) -> String {
    error.to_string()
}
