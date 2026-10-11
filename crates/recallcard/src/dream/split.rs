//! 一对多拆分只重分配既有证据；任务语义仍由用户审阅。
use super::*;

pub(super) fn targets(job: &DreamJob, result: &DreamResult) -> Result<BTreeMap<usize, Memory>> {
    let mut branches = BTreeMap::new();
    let mut parents = BTreeSet::new();
    for plan in &result.split_plans {
        let (id, revision) = memory_ref(&plan.target_ref)?;
        if revision != Some(plan.expected_revision) || !parents.insert(id.clone()) {
            return Err("拆分必须指定唯一的完整旧版本引用".into());
        }
        let old = job
            .memory_read_set
            .iter()
            .find(|m| m.memory.id == id)
            .ok_or("拆分目标不在本次已授权 read-set")?
            .memory
            .clone();
        if old.revision != plan.expected_revision
            || !matches!(old.state, MemoryState::Active | MemoryState::Tentative)
        {
            return Err("拆分目标版本过期或已撤回/替代".into());
        }
        old.revision.checked_add(1).ok_or("记忆版本已达到上限")?;
        let previous: BTreeSet<_> = old.data.source_refs.iter().cloned().collect();
        let mut partition = BTreeSet::new();
        for index in &plan.proposal_indexes {
            let p = result.proposals.get(*index).ok_or("拆分分支超出提议范围")?;
            if p.operation != DreamOperation::Add
                || p.target_ref.is_some()
                || p.expected_revision.is_some()
                || p.evidence == Evidence::AssistantSuggestion
                || p.scope != old.data.scope
                || p.navigation.is_none()
                || branches.insert(*index, old.clone()).is_some()
            {
                return Err(
                    "拆分分支必须是同范围的独立 add、明确导航及非助手建议，且不能重复归属".into(),
                );
            }
            if p.source_refs.is_empty() {
                return Err("拆分分支必须有原始来源".into());
            }
            for reference in &p.source_refs {
                let source = event_ref(reference)?;
                if !previous.contains(&source) || !partition.insert(source) {
                    return Err("拆分来源必须严格分区：不得夹带新来源或重复借用来源".into());
                }
            }
        }
        if partition != previous {
            return Err("拆分分支必须完整保留旧来源，不得遗漏".into());
        }
        if result.proposals.iter().any(|p| {
            p.target_ref
                .as_deref()
                .and_then(|r| memory_ref(r).ok())
                .is_some_and(|(target, _)| target == id)
        }) {
            return Err("拆分目标不能同时被其他提议修改".into());
        }
    }
    Ok(branches)
}

pub(super) fn history(result: &DreamResult, result_hash: &str) -> Result<Vec<DreamSplitHistory>> {
    result
        .split_plans
        .iter()
        .map(|plan| {
            let (id, _) = memory_ref(&plan.target_ref)?;
            let next = plan
                .expected_revision
                .checked_add(1)
                .ok_or("记忆版本已达到上限")?;
            Ok(DreamSplitHistory {
                target_ref: plan.target_ref.clone(),
                retired_ref: format!("memory:{id}@{next}"),
                branch_refs: plan
                    .proposal_indexes
                    .iter()
                    .map(|index| format!("memory:{}@1", proposed_id(result_hash, *index)))
                    .collect(),
            })
        })
        .collect()
}

// 日志恢复必须再次核对原生拆分的全部边，不能只信任写入列表/收据里自填的摘要。
pub(super) fn validate_writes(tx: &Transaction) -> Result<()> {
    let targets = targets(&tx.job, &tx.result)?;
    if tx.receipt.split_history != history(&tx.result, &tx.receipt.result_hash)? {
        return Err("拆分历史收据与批准计划不一致".into());
    }
    let mut checked = BTreeSet::new();
    for (index, old) in targets {
        if checked.insert(old.id.clone()) {
            let write = tx
                .writes
                .iter()
                .find(|c| c.before.as_ref().is_some_and(|m| m.id == old.id))
                .ok_or("拆分缺少旧记忆保留记录")?;
            let after = write.after.as_ref().ok_or("拆分缺少旧记忆目标")?;
            if write.operation != DreamOperation::Supersede
                || write.before.as_ref() != Some(&old)
                || after.id != old.id
                || after.data != old.data
                || after.state != MemoryState::Superseded
                || old.revision.checked_add(1) != Some(after.revision)
            {
                return Err("拆分不得改写旧记忆正文/来源或丢失旧版本".into());
            }
        }
        let id = proposed_id(&tx.receipt.result_hash, index);
        let write = tx
            .writes
            .iter()
            .find(|c| c.after.as_ref().is_some_and(|m| m.id == id))
            .ok_or("拆分缺少已批准的新分支")?;
        let after = write.after.as_ref().ok_or("拆分缺少新分支内容")?;
        let mut input = proposal_input(&tx.result.proposals[index])?;
        input.supersedes = vec![old.id.clone()];
        input.protected = old.data.protected;
        if input.protected {
            input.authority = old.data.authority.clone();
        }
        if write.operation != DreamOperation::Add
            || write.before.is_some()
            || after.revision != 1
            || after.data != input
            || after.state != MemoryState::Active
        {
            return Err("拆分分支内容/来源/历史关联与批准计划不一致".into());
        }
    }
    Ok(())
}
