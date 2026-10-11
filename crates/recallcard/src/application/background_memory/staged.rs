//! 正常后台的持久分阶段协调。模型只产出候选；目录/来源重导出由可信宿主完成。
use super::*;
use crate::{context::Context, policy::Access};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MemoryPipeline {
    LegacySingle,
    StagedReview,
}
impl MemoryPipeline {
    pub(super) fn legacy() -> Self {
        Self::LegacySingle
    }
    pub(super) fn is_legacy(&self) -> bool {
        *self == Self::LegacySingle
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtractionContext {
    pub job: DreamJob,
    pub result: DreamResult,
}
/// 仅报告检索覆盖，不存查询正文、文件路径或模型分数准确率。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct SemanticLookup {
    pub status: String,
    pub queries: usize,
    pub cache_hits: usize,
    #[serde(default)]
    pub local_encodings: usize,
    pub errors: BTreeSet<String>,
    pub memory_refs: BTreeSet<String>,
    pub directory_refs: BTreeSet<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Staging {
    source_job: DreamJob,
    extraction: Option<ProviderOutput>,
    candidates_exported: bool,
    candidate_refs: Vec<String>,
    lookup: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    semantic: Option<SemanticLookup>,
    boundary_findings: Vec<String>,
}
impl Staging {
    pub(super) fn new(source_job: DreamJob) -> Self {
        Self {
            source_job,
            extraction: None,
            candidates_exported: false,
            candidate_refs: vec![],
            lookup: "尚未查找".into(),
            semantic: None,
            boundary_findings: vec![],
        }
    }
    pub(super) fn retry_invalid(&mut self) {
        if !self.candidates_exported {
            self.extraction = None;
        }
        self.boundary_findings.clear();
    }
    pub(super) fn semantic_lookup(&self) -> Option<SemanticLookup> {
        self.semantic.clone()
    }
    pub(super) fn has_extraction(&self) -> bool {
        self.extraction.is_some()
    }
    pub(super) fn context(&self) -> AppResult<ExtractionContext> {
        Ok(ExtractionContext {
            job: self.source_job.clone(),
            result: self
                .extraction
                .as_ref()
                .ok_or_else(|| storage("missing extraction"))?
                .result
                .clone(),
        })
    }
}
pub(super) fn sum_usage(before: Option<u64>, next: Option<u64>, calls: u64) -> Option<u64> {
    if calls == 1 {
        next
    } else {
        before.zip(next).map(|(a, b)| a.saturating_add(b))
    }
}
fn source_ids(result: &crate::dream::DreamProposal) -> BTreeSet<String> {
    result
        .source_refs
        .iter()
        .map(|r| r.strip_prefix("event:").unwrap_or(r).to_owned())
        .collect()
}
fn verify_context(context: &ExtractionContext, job: &DreamJob) -> AppResult<()> {
    if !context.job.memory_read_set.is_empty()
        || context.job.allowed_scope != job.allowed_scope
        || context.result.job_id != context.job.job_id
        || context.result.input_hash != context.job.input_hash
        || !context.result.split_plans.is_empty()
        || context.result.proposals.iter().any(|p| {
            !matches!(
                p.operation,
                DreamOperation::Add | DreamOperation::Noop | DreamOperation::Conflict
            )
        })
    {
        return Err(invalid("提取阶段身份无效或提前修改旧记忆"));
    }
    let original: BTreeMap<_, _> = context
        .job
        .source_refs
        .iter()
        .map(|s| (s.reference.as_str(), s))
        .collect();
    let current: BTreeMap<_, _> = job
        .source_refs
        .iter()
        .map(|s| (s.reference.as_str(), s))
        .collect();
    for (reference, source) in &original {
        let Some(other) = current.get(reference) else {
            return Err(invalid("候选重导出缺失提取来源；必须重新提取"));
        };
        if digest(source)? != digest(other)? {
            return Err(invalid("候选重导出改变了提取来源；必须重新提取"));
        }
    }
    let inherited: BTreeSet<_> = job
        .memory_read_set
        .iter()
        .flat_map(|m| m.memory.data.source_refs.iter().map(String::as_str))
        .collect();
    if job.source_refs.iter().any(|s| {
        !original.contains_key(s.reference.as_str()) && !inherited.contains(s.event.id.as_str())
    }) {
        return Err(invalid("整合新增来源必须属于当前已授权候选的原有证据"));
    }
    Ok(())
}
pub(super) fn boundary_findings(
    context: &ExtractionContext,
    result: &DreamResult,
    job: &DreamJob,
) -> AppResult<Vec<String>> {
    verify_context(context, job)?;
    let units: Vec<_> = context
        .result
        .proposals
        .iter()
        .filter(|p| p.operation == DreamOperation::Add)
        .map(source_ids)
        .collect();
    let extracted: BTreeSet<_> = units.iter().flatten().cloned().collect();
    let mut findings = Vec::new();
    for (index, proposal) in result.proposals.iter().enumerate().filter(|(_, p)| {
        matches!(
            p.operation,
            DreamOperation::Add | DreamOperation::Update | DreamOperation::Supersede
        )
    }) {
        let refs = source_ids(proposal);
        let touched: Vec<_> = units.iter().filter(|u| !refs.is_disjoint(u)).collect();
        if touched
            .iter()
            .enumerate()
            .any(|(i, a)| touched[i + 1..].iter().any(|b| a.is_disjoint(b)))
        {
            findings.push(format!(
                "提议 {index} 合并了来源独立的提取任务；共享词不构成合并依据"
            ));
        }
        let target = proposal
            .target_ref
            .as_deref()
            .unwrap_or("")
            .trim_start_matches("memory:")
            .split('@')
            .next()
            .unwrap_or("");
        let inherited: BTreeSet<_> = job
            .memory_read_set
            .iter()
            .filter(|m| m.memory.id == target)
            .flat_map(|m| m.memory.data.source_refs.iter().cloned())
            .collect();
        if refs
            .iter()
            .any(|id| !extracted.contains(id) && !inherited.contains(id))
        {
            findings.push(format!("提议 {index} 借用了未经提取或不属于目标的来源"));
        }
    }
    Ok(findings)
}
impl MemoryRuntime<'_> {
    fn staged_candidates(
        &self,
        context: &ExtractionContext,
        config: &MemoryConfig,
    ) -> AppResult<(DreamJob, SemanticLookup)> {
        self.revalidate_projection(&context.job)?;
        let mut semantic_lookup = SemanticLookup {
            status: "unconfigured".into(),
            ..Default::default()
        };
        let semantic = if let Some(path) = &config.candidate_semantic_config {
            semantic_lookup.status = "unavailable".into();
            match crate::semantic::SemanticSearch::from_application_config_file(
                self.vault,
                path,
                std::slice::from_ref(&config.scope),
            ) {
                Ok(worker) => Some(worker),
                Err(_) => {
                    semantic_lookup
                        .errors
                        .insert("offline_config_invalid".into());
                    None
                }
            }
        } else {
            None
        };
        let ctx = Context::new(
            self.vault,
            Access::new(vec![config.scope.clone()]).map_err(storage)?,
        );
        let docs = {
            let guard = self.vault.read_guard().map_err(storage)?;
            ctx.memory_documents_locked(&guard).map_err(storage)?
        };
        let now = Utc::now();
        let nav = crate::navigation::Index::build(&docs, std::slice::from_ref(&config.scope), now)
            .map_err(storage)?;
        let mut candidates = BTreeSet::new();
        for proposal in context
            .result
            .proposals
            .iter()
            .filter(|p| p.operation == DreamOperation::Add)
        {
            // 只使用提取结果给出的实际目录入口；不存在的路径不当作已知目录。
            for hint in proposal.navigation.iter().flatten() {
                if let Some(node) =
                    nav.nodes
                        .get(&format!("{}{}", crate::navigation::PREFIX, hint.path))
                {
                    candidates.extend(node.memories.iter().cloned());
                }
            }
            // 目录优先；词法/来源身份及可选离线向量只补候选，不授予合并许可。
            let terms = text_terms(proposal.content.as_deref().unwrap_or(""));
            let sources = source_ids(proposal);
            for doc in docs.iter().filter(|d| d.current_memory_at(now)) {
                if !terms.is_disjoint(&text_terms(&doc.text))
                    || doc
                        .evidence_refs
                        .iter()
                        .any(|r| sources.contains(r.strip_prefix("event:").unwrap_or(r)))
                {
                    candidates.insert(doc.reference.clone());
                }
            }
            if let Some(semantic) = &semantic {
                let query = proposal.content.as_deref().unwrap_or("");
                if semantic_lookup.queries >= 8 {
                    semantic_lookup
                        .errors
                        .insert("query_budget_exhausted".into());
                } else if query.is_empty() || query.len() > 4096 {
                    semantic_lookup.errors.insert("query_size_exceeded".into());
                } else {
                    semantic_lookup.queries += 1;
                    // 不持有 Vault 锁等 worker；返回后重读权限、版本及完整索引 generation。
                    let selected: Vec<_> = docs
                        .iter()
                        .filter(|d| d.current_memory_at(now))
                        .cloned()
                        .collect();
                    match semantic.candidates(
                        query,
                        &docs,
                        &selected,
                        std::slice::from_ref(&config.scope),
                        now,
                    ) {
                        Ok(found) => {
                            let guard = self.vault.read_guard().map_err(storage)?;
                            let current = ctx.memory_documents_locked(&guard).map_err(storage)?;
                            let current_time = Utc::now();
                            match found.revalidate(
                                &current,
                                std::slice::from_ref(&config.scope),
                                current_time,
                            ) {
                                Ok(()) => {
                                    if found.query_mode == "local_encoded" {
                                        semantic_lookup.local_encodings += 1;
                                    } else {
                                        semantic_lookup.cache_hits += 1;
                                    }
                                    let current_nav = crate::navigation::Index::build(
                                        &current,
                                        std::slice::from_ref(&config.scope),
                                        current_time,
                                    )
                                    .map_err(storage)?;
                                    for reference in found.references {
                                        for node in current_nav
                                            .nodes
                                            .values()
                                            .filter(|node| node.memories.contains(&reference))
                                        {
                                            semantic_lookup
                                                .directory_refs
                                                .insert(node.reference.clone());
                                        }
                                        semantic_lookup.memory_refs.insert(reference.clone());
                                        candidates.insert(reference);
                                    }
                                }
                                Err(error) => {
                                    semantic_lookup.errors.insert(error.0.into());
                                }
                            }
                        }
                        Err(error) => {
                            semantic_lookup.errors.insert(error.0.into());
                        }
                    }
                }
            }
            if candidates.len() > 32 {
                return Err(AppError::new(
                    ErrorCode::ResourceLimit,
                    "提取后候选超过32条，未截断或自动整合",
                    "缩小批次或审阅提取边界后重新排队",
                ));
            }
        }
        let mut sources: BTreeSet<_> = context
            .job
            .source_refs
            .iter()
            .map(|s| s.event.id.clone())
            .collect();
        for doc in docs.iter().filter(|d| candidates.contains(&d.reference)) {
            sources.extend(
                doc.evidence_refs
                    .iter()
                    .map(|r| r.strip_prefix("event:").unwrap_or(r).to_owned()),
            );
        }
        if sources.len() > 64 {
            return Err(AppError::new(
                ErrorCode::ResourceLimit,
                "整合来源超过64条；未截断来源",
                "缩小批次或候选范围",
            ));
        }
        let job = self
            .vault
            .dream_export(
                &sources.into_iter().collect::<Vec<_>>(),
                &candidates.into_iter().collect::<Vec<_>>(),
                &config.scope,
            )
            .map_err(|_| {
                AppError::new(
                    ErrorCode::SourceChanged,
                    "候选或其来源已改变/撤权，停止整合",
                    "重新查找当前候选",
                )
            })?;
        verify_context(context, &job)?;
        if serde_json::to_vec(&job).map_err(storage)?.len() > config.max_projection_bytes {
            return Err(AppError::new(
                ErrorCode::ResourceLimit,
                "完整整合投影超过配置预算，未截断",
                "缩小批次或明确调整预算",
            ));
        }
        if semantic_lookup.cache_hits + semantic_lookup.local_encodings > 0 {
            semantic_lookup.status = if semantic_lookup.errors.is_empty() {
                "available"
            } else {
                "partial"
            }
            .into();
        }
        Ok((job, semantic_lookup))
    }
    pub(super) fn advance_staged<P: MemoryProvider + ?Sized>(
        &self,
        record: &mut MemoryJobRecord,
        provider: &mut P,
    ) -> AppResult<()> {
        if record
            .staging
            .as_ref()
            .is_none_or(|s| s.extraction.is_none())
        {
            let output = self.request_provider(record, provider, None)?;
            // 缓存完整提取响应后再验证；中断恢复不能悄悄重发模型请求。
            let _guard = self.lock("state.lock")?;
            record
                .staging
                .as_mut()
                .ok_or_else(|| storage("missing staging"))?
                .extraction = Some(output);
            record.dispatch_started = false;
            record.status.phase = JobPhase::Validating;
            self.save(record)?;
        }
        let context = record
            .staging
            .as_ref()
            .ok_or_else(|| storage("missing staging"))?
            .context()?;
        verify_context(&context, &context.job)?;
        if !self.receipt_exists(&record.projection)? {
            self.vault
                .dream_review(&context.result)
                .map_err(|_| invalid("提取结果未通过实时来源/角色校验；没有进入候选查找"))?;
        }
        if !record.staging.as_ref().unwrap().candidates_exported {
            let config = {
                let _guard = self.lock("state.lock")?;
                self.checkpoint(record)?
            };
            let (projection, semantic_lookup) = self.staged_candidates(&context, &config)?;
            let _guard = self.lock("state.lock")?;
            self.checkpoint(record)?;
            self.revalidate_projection(&projection)?;
            let staging = record.staging.as_mut().unwrap();
            staging.candidate_refs = projection
                .memory_read_set
                .iter()
                .map(|m| m.reference.clone())
                .collect();
            staging.lookup = format!(
                "live_directories_then_lexical_and_source_identity; semantic_search={}",
                if semantic_lookup.status == "unconfigured" {
                    "unavailable"
                } else {
                    &semantic_lookup.status
                }
            );
            staging.semantic = Some(semantic_lookup);
            staging.candidates_exported = true;
            record.status.progress.memories_read = projection.memory_read_set.len() as u64;
            record.projection = projection;
            self.save(record)?;
        }
        verify_context(&context, &record.projection)?;
        if record.result.is_none() {
            let output = self.request_provider(record, provider, Some(&context))?;
            let _guard = self.lock("state.lock")?;
            record.result = Some(output);
            record.status.phase = JobPhase::Validating;
            self.save(record)?;
        }
        let _guard = self.lock("state.lock")?;
        self.checkpoint(record)?;
        if !self.receipt_exists(&record.projection)? {
            self.revalidate_projection(&record.projection)?;
        }
        let result = record
            .result
            .as_ref()
            .ok_or_else(|| storage("missing consolidated result"))?
            .result
            .clone();
        let findings = boundary_findings(&context, &result, &record.projection)?;
        record.staging.as_mut().unwrap().boundary_findings = findings.clone();
        self.save(record)?;
        if !findings.is_empty() {
            return Err(invalid(
                "整合跨越任务边界或借用其他任务来源；结果已留存，未发布",
            ));
        }
        let review = self
            .vault
            .dream_review(&result)
            .map_err(|_| invalid("整合未通过实时来源、版本或角色校验；未发布"))?;
        if !review.already_applied {
            return Err(AppError::new(
                ErrorCode::Conflict,
                "分阶段整理完成，等待人工 review；没有自动发布",
                "核对提取单元、完整来源与最终差异，再显式批准同一结果摘要",
            ));
        }
        let receipt = self
            .vault
            .dream_apply(&result, &review.result_hash, false)
            .map_err(storage)?;
        // 候选继承来源是旧证据，不应使“本批已整理”超过新选来源数量。
        let selected: BTreeSet<_> = context
            .job
            .source_refs
            .iter()
            .map(|s| s.event.id.as_str())
            .collect();
        record.status.progress.sources_committed = receipt
            .source_coverage
            .iter()
            .filter(|reference| {
                selected.contains(reference.strip_prefix("event:").unwrap_or(reference))
            })
            .count() as u64;
        record.status.progress.memories_committed = receipt.changes.len() as u64;
        record.status.progress.receipt_id = Some(receipt.job_id.clone());
        record.receipt = Some(receipt);
        record.status.state = JobState::Completed;
        record.status.phase = JobPhase::Finished;
        record.status.error = None;
        record.status.can_resume = false;
        self.save(record)
    }
}
