//! 中断的一对多拆分可以显式撤销；只回滚日志拥有且未被外部编辑的快照。
use super::*;

impl Vault {
    pub fn dream_abort(&self, approved_result_hash: &str) -> Result<Value> {
        let lock = self.lock_unchecked()?;
        let path = self.dream_transaction_path()?;
        if !path.exists() {
            return Err("没有待回滚的 Dream 事务".into());
        }
        let mut tx: Transaction = read_json(&path)?;
        if tx.receipt.result_hash != approved_result_hash {
            return Err("回滚摘要与当前事务不一致；未改变任何记忆".into());
        }
        self.check_split_rollback(&tx)?;
        tx.rollback = true;
        self.write_replace(&path, &tx)?;
        self.finish_split_rollback(&tx)?;
        drop(lock);
        self.refresh_split_rollback_views()?;
        Ok(json!({"rolled_back":true,"result_hash":approved_result_hash,"replay_blocked":true}))
    }

    pub(super) fn refresh_split_rollback_views(&self) -> Result<()> {
        self.rebuild_views().map_err(|e| {
            format!("未完成拆分已回滚，旧记录已恢复；派生视图刷新失败，请运行 views 重建：{e}")
        })?;
        Ok(())
    }

    pub(super) fn rollback_path(&self, result_hash: &str) -> Result<PathBuf> {
        // 调用者必须已从严格的结果/事务计算摘要；不接受路径。
        if result_hash.len() != 64 || !result_hash.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("无效的回滚结果摘要".into());
        }
        Ok(self
            .state_dir()?
            .join("dream-rollbacks")
            .join(format!("{result_hash}.json")))
    }

    fn check_split_rollback(&self, tx: &Transaction) -> Result<()> {
        self.check_dream_recovery_barrier()?;
        validate_job(&tx.job)?;
        validate_result(&tx.result)?;
        let branches = split::targets(&tx.job, &tx.result)?;
        let parents: BTreeSet<_> = branches.values().map(|m| m.id.clone()).collect();
        if tx.result.split_plans.is_empty()
            || branches.len() != tx.result.proposals.len()
            || tx.schema_version != 1
            || tx.vault_root != self.root().to_string_lossy()
            || tx.result.job_id != tx.job.job_id
            || tx.result.input_hash != tx.job.input_hash
            || tx.receipt.job_id != tx.job.job_id
            || tx.receipt.input_hash != tx.job.input_hash
            || tx.receipt.result_hash != digest(&tx.result)?
            || tx.receipt.schema_version != 1
            || tx.receipt.already_applied
            || tx.writes.len() != branches.len() + parents.len()
            || tx.receipt.changes.len() != tx.writes.len()
            || tx.receipt.source_coverage
                != tx
                    .job
                    .source_refs
                    .iter()
                    .map(|s| s.reference.clone())
                    .collect::<Vec<_>>()
        {
            return Err("回滚只接受完整、摘要一致的纯拆分事务".into());
        }
        split::validate_writes(tx)?;
        let receipt_path = self.dream_receipt_path(&tx.job.job_id)?;
        reject_symlink(&receipt_path)?;
        if receipt_path.exists() {
            return Err("成功收据已发布，不能用中断回滚撤销已完成事务；请审阅新的撤回提议".into());
        }
        let mut ids = BTreeSet::new();
        for (change, receipt) in tx.writes.iter().zip(&tx.receipt.changes) {
            let after = change.after.as_ref().ok_or("回滚缺少目标快照")?;
            after.validate()?;
            if !ids.insert(after.id.clone())
                || receipt.id != after.id
                || receipt.revision != after.revision
                || receipt.content_hash != digest(after)?
                || change.before.as_ref().is_some_and(|m| m.data.protected) && !tx.approve_protected
            {
                return Err("回滚目标/摘要/保护批准不一致".into());
            }
            let path = self.memory_path(&after.id);
            reject_symlink(&path)?;
            if path.exists() {
                let current = parse_memory(&read_text(&path)?)?;
                if current != *after && change.before.as_ref() != Some(&current) {
                    return Err("回滚遇到外部编辑，未覆盖或删除；保留事务供人工核对".into());
                }
            } else if change.before.is_some() {
                return Err("回滚的旧记忆已丢失，不能盲目恢复覆盖".into());
            }
        }
        // 不要求来源仍可披露：回滚只撤掉未提交分支，当前抑制规则仍控制旧记录可见性。
        Ok(())
    }

    pub(super) fn finish_split_rollback(&self, tx: &Transaction) -> Result<()> {
        self.check_split_rollback(tx)?;
        let archive = self.rollback_path(&tx.receipt.result_hash)?;
        let parent = archive.parent().ok_or("回滚日志无父目录")?;
        reject_symlink(parent)?;
        fs::create_dir_all(parent).map_err(error)?;
        reject_symlink(&archive)?;
        if archive.exists() {
            let existing: Transaction = read_json(&archive)?;
            if digest(&existing)? != digest(tx)? {
                return Err("回滚审计快照冲突".into());
            }
        } else {
            self.write_new(&archive, tx)?;
        }
        for change in tx.writes.iter().rev() {
            let after = change.after.as_ref().ok_or("回滚缺少目标")?;
            let path = self.memory_path(&after.id);
            if let Some(before) = &change.before {
                if parse_memory(&read_text(&path)?)? != *before {
                    self.write_memory(before, true)?;
                }
            } else if path.exists() {
                // 上面已核对完整快照；不删除外来/用户编辑的记录。
                fs::remove_file(&path).map_err(error)?;
                sync_directory(path.parent().ok_or("记忆没有父目录")?)?;
            }
        }
        let journal = self.dream_transaction_path()?;
        fs::remove_file(&journal).map_err(error)?;
        sync_directory(journal.parent().ok_or("事务没有父目录")?)?;
        Ok(())
    }
}
