//! 简洁的共同本机读取入口；不访问旧云配置，不隐式建索引。
use super::*;
use crate::local_events::LocalEvents;
use std::time::{Duration, Instant};
const BUDGET: usize = 8192;
const LIMIT: usize = 10;
const SCOPES: usize = 4;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecallSearchArgs {
    pub query: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecallFetchArgs {
    pub id: String,
}
fn scopes_hash(access: &Access) -> String {
    hash(json!(access.scopes()).to_string().as_bytes())
}
fn code(error: &str) -> &str {
    if error.len() <= 96 && error.bytes().all(|c| c.is_ascii_lowercase() || c == b'_') {
        error
    } else {
        "local_read_unavailable"
    }
}
fn hash_valid(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}
// 这是可校验的定位符，不是权限或会话票据。Web text/2 后续由 host 映射会话内短编号。
struct Locator {
    scope: String,
    reference: String,
    snapshot: String,
    epoch: String,
    config: String,
    scopes: String,
    offset: usize,
}
impl Locator {
    fn id(&self) -> String {
        format!(
            "rcs1|{}|{}|{}|{}|{}|{}|{}",
            self.scope,
            self.reference,
            self.snapshot,
            self.epoch,
            self.config,
            self.scopes,
            self.offset
        )
    }
    fn parse(id: &str) -> Result<Self> {
        if id.len() > 768 {
            return Err("资料定位符过长".into());
        }
        let p: Vec<_> = id.split('|').collect();
        if p.len() != 8
            || p[0] != "rcs1"
            || !hash_valid(p[3])
            || !hash_valid(p[4])
            || !(p[5] == "-" || hash_valid(p[5]))
            || !hash_valid(p[6])
        {
            return Err("资料定位符无效；请重新检索".into());
        }
        validate_scope(p[1])?;
        let reference = p[2];
        if let Some(id) = reference.strip_prefix("event:") {
            validate_id(id, "evt_")?;
            if p[5] == "-" {
                return Err("事件定位符缺少索引绑定".into());
            }
        } else if reference.starts_with("attachment:") {
            attachments::parse(reference)?;
        } else if let Some(id) = reference.strip_prefix("memory:") {
            let (id, revision) = id.split_once('@').ok_or("记忆定位符缺少版本")?;
            validate_id(id, "mem_")?;
            revision.parse::<u64>().map_err(|_| "记忆版本无效")?;
            if p[5] != "-" {
                return Err("记忆定位符无效".into());
            }
        } else {
            return Err("只能读取已返回的 Event/Memory/附件定位符".into());
        }
        let offset = p[7].parse::<usize>().map_err(|_| "资料字节起点无效")?;
        if offset > 4 * 1024 * 1024 {
            return Err("资料字节起点超限".into());
        }
        Ok(Self {
            scope: p[1].into(),
            reference: reference.into(),
            snapshot: p[3].into(),
            epoch: p[4].into(),
            config: p[5].into(),
            scopes: p[6].into(),
            offset,
        })
    }
}
fn merge_candidates(groups: &[Vec<Value>]) -> Vec<Value> {
    let mut rows = vec![];
    let mut seen = BTreeSet::new();
    // Never compare cosine across scopes/models. Preserve order within tiers.
    for protected in [true, false] {
        for i in 0..20 {
            for group in groups {
                if let Some(row) = group
                    .iter()
                    .filter(|row| (row["retrieval_support"]["protected"] == true) == protected)
                    .nth(i)
                {
                    let key = (
                        row["scope"].clone().to_string(),
                        row["ref"].clone().to_string(),
                        row["snapshot"].clone().to_string(),
                    );
                    if seen.insert(key) {
                        let mut row = row.clone();
                        row["final_rank"] = json!(rows.len() + 1);
                        rows.push(row);
                    }
                }
            }
        }
    }
    while rows.len() > LIMIT {
        rows.remove(budget_drop_index(&rows));
    }
    for (index, row) in rows.iter_mut().enumerate() {
        row["final_rank"] = json!(index + 1);
    }
    rows
}
// At the lowest remaining protected-anchor tier, discard a repeated source kind
// before its only representative. This is a bounded presentation policy, not a
// relevance judgment; it cannot introduce a source or cross an access scope.
fn budget_drop_index(rows: &[Value]) -> usize {
    let last = rows.len() - 1;
    let protected = rows[last]["retrieval_support"]["protected"] == true;
    let tier: Vec<_> = rows
        .iter()
        .enumerate()
        .filter(|(_, row)| (row["retrieval_support"]["protected"] == true) == protected)
        .collect();
    tier.iter()
        .rev()
        .find(|(_, row)| {
            tier.iter()
                .filter(|(_, other)| other["kind"] == row["kind"])
                .count()
                > 1
        })
        .map_or(last, |(index, _)| *index)
}
fn fit_search_budget(result: &mut Value) -> Result<()> {
    let bytes = |value: &Value| {
        serde_json::to_vec(value)
            .map(|bytes| bytes.len())
            .map_err(|_| "检索结果无法序列化".to_owned())
    };
    if bytes(result)? <= BUDGET {
        return Ok(());
    }
    result["truncated"] = json!(true);
    result["coverage"]["budget_limited"] = json!(true);
    result["status"] = json!("partial");
    // Full identity, fetch locator, text/range/hash, role, parent evidence and
    // channel scores remain unchanged. Omit the optional session label and
    // diagnostics already represented in nested retrieval metadata.
    for row in result["results"].as_array_mut().unwrap() {
        if let Some(object) = row.as_object_mut() {
            object.remove("session_ref");
            // Legacy semantic-only rows have no nested retrieval metadata.
            // Never erase their sole rank or the original indexed chunk.
            if object.get("semantic_rank").is_some()
                && object.get("semantic_rank")
                    == object.get("retrieval").and_then(|r| r.get("semantic_rank"))
            {
                object.remove("semantic_rank");
            }
        }
        if let Some(support) = row["retrieval_support"].as_object_mut() {
            support.remove("case_policy");
        }
    }
    result["coverage"]["presentation"] = json!("compact_index_metadata_full_source_via_fetch");
    if !result["coverage"]["budget_omitted_by_kind"].is_object() {
        result["coverage"]["budget_omitted_by_kind"] = json!({});
    }
    while bytes(result)? > BUDGET {
        if result["results"].as_array().unwrap().is_empty() {
            return Err("检索元数据超过固定预算".into());
        }
        // A row that cannot fit even alone must not evict smaller rows first.
        // Keep its omission explicit; no source text or binding is shortened.
        let mut impossible = None;
        for (index, row) in result["results"].as_array().unwrap().iter().enumerate() {
            let mut alone = result.clone();
            alone["results"] = json!([row]);
            alone["match_count"] = json!(1);
            if bytes(&alone)? > BUDGET {
                impossible = Some(index);
                break;
            }
        }
        let rows = result["results"].as_array_mut().unwrap();
        let removed = rows.remove(impossible.unwrap_or_else(|| budget_drop_index(rows)));
        let kind = removed["kind"].as_str().unwrap_or("unknown");
        let omitted = result["coverage"]["budget_omitted_by_kind"][kind]
            .as_u64()
            .unwrap_or(0);
        result["coverage"]["budget_omitted_by_kind"][kind] = json!(omitted + 1);
        let count = result["results"].as_array().unwrap().len();
        result["match_count"] = json!(count);
        if count == 0 {
            result["status"] = json!("budget_exhausted");
        }
        for (index, row) in result["results"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .enumerate()
        {
            row["final_rank"] = json!(index + 1);
        }
    }
    Ok(())
}
impl Context<'_> {
    pub fn recall_search(&self, args: RecallSearchArgs) -> Result<Value> {
        self.recall_question(&args.query, "all")
    }
    /// 桌面固定 target；模型简洁参数不能改变范围、预算、提供器或批准。
    pub fn recall_question(&self, query: &str, target: &str) -> Result<Value> {
        if query.trim().is_empty() || query.len() > 4096 || query.chars().any(|c| c == '\0') {
            return Err("请输入1–4096字节的检索问题".into());
        }
        if !matches!(target, "all" | "events" | "memories") {
            return Err("简洁检索只支持普通对话和记忆".into());
        }
        let scopes = self.access.scopes();
        if scopes.len() > SCOPES {
            return Ok(
                json!({"status":"scope_limit","results":[],"truncated":true,"coverage":{"scope_limit":SCOPES,"selected_scopes":scopes.len(),"note":"请在宿主连接设置中明确缩小本次范围；没有静默选择或发送查询。","cloud_fallback":false}}),
            );
        }
        let deadline = Instant::now() + Duration::from_secs(15);
        let mut groups: Vec<Vec<Value>> = vec![];
        let mut bindings = vec![];
        let mut coverage = vec![];
        let mut partial = false;
        let mut semantic_available = false;
        let mut attachment_semantic_available = false;
        let mut budget_limited = false;
        let mut memory_status = json!("not_requested");
        let mut attachment_status = json!({"status":"not_requested"});
        let scopes_digest = scopes_hash(&self.access);
        if target != "memories" {
            for scope in &scopes {
                if Instant::now() >= deadline {
                    coverage.push(json!({"scope":scope,"state":"deadline"}));
                    partial = true;
                    continue;
                }
                let loaded = LocalEvents::open(self.vault, scope);
                let index = match loaded {
                    Ok(Some(index)) => index,
                    Ok(None) => {
                        coverage.push(json!({"scope":scope,"state":"unconfigured"}));
                        partial = true;
                        continue;
                    }
                    Err(error) => {
                        coverage
                            .push(json!({"scope":scope,"state":"unavailable","code":code(&error)}));
                        partial = true;
                        continue;
                    }
                };
                match index.search_before(query, 20, deadline) {
                    Ok(page) => {
                        let epoch = page["coverage"]["event_index_snapshot"]
                            .as_str()
                            .ok_or("本机索引响应缺少版本")?
                            .to_owned();
                        let config = page["coverage"]["local_config_identity"]
                            .as_str()
                            .ok_or("本机索引响应缺少配置绑定")?
                            .to_owned();
                        let mut rows = page["results"]
                            .as_array()
                            .ok_or("本机索引响应无效")?
                            .clone();
                        for row in &mut rows {
                            let locator = Locator {
                                scope: scope.clone(),
                                reference: row["ref"].as_str().ok_or("本机来源无效")?.into(),
                                snapshot: row["snapshot"]
                                    .as_str()
                                    .ok_or("本机来源缺少快照")?
                                    .into(),
                                epoch: epoch.clone(),
                                config: config.clone(),
                                scopes: scopes_digest.clone(),
                                offset: row["text_range"]["start_byte"]
                                    .as_u64()
                                    .ok_or("本机来源缺少范围")?
                                    as usize,
                            };
                            row["id"] = json!(locator.id());
                            row["relevance"] = json!("unassessed");
                        }
                        partial |= page["truncated"] == true || page["coverage"]["partial"] == true;
                        semantic_available |= page["coverage"]["local_event_semantic"] == true;
                        attachment_semantic_available |=
                            page["coverage"]["attachments"]["semantic"] == true;
                        coverage.push(json!({"scope":scope,"state":if page["coverage"]["fusion"]=="lexical_only"{"degraded"}else{"ready"},"semantic":page["coverage"]["semantic"],"candidates":rows.len(),"truncated":page["truncated"],"query_encoding":page["coverage"]["semantic_query_encoding"],"candidate_policy":page["coverage"]["candidate_policy"],"verified_candidates":page["coverage"]["verified_candidates"],"verification_limited":page["coverage"]["candidate_verification_limited"],"lexical":page["coverage"]["lexical"],"fusion":page["coverage"]["fusion"],"attachments":page["coverage"]["attachments"]}));
                        bindings.push((index, epoch, config));
                        groups.push(rows);
                    }
                    Err(error) => {
                        coverage
                            .push(json!({"scope":scope,"state":"unavailable","code":code(&error)}));
                        partial = true;
                    }
                }
            }
        }
        // 所有提供器等待已结束；同一最终读锁下重验早先 scope 的来源/配置并构造最终包。
        let _guard = self.vault.read_guard()?;
        for (index, epoch, config) in &bindings {
            index.verify_binding(epoch, config)?;
        }
        for lexical_target in ["memories", "attachments"] {
            if (lexical_target == "memories" && target == "events")
                || (lexical_target == "attachments" && target == "memories")
            {
                continue;
            }
            // 可选词法通道没有支持的词项时，只报告该通道未覆盖；
            // 不能让纯标点/表情问题中断已完成的 Event 候选路线。
            if search::ranking_terms(query, "natural_question").is_empty() {
                if lexical_target == "memories" {
                    memory_status = json!("unsupported_query");
                } else {
                    attachment_status = json!({"status":"unsupported_query","semantic":false,"mode":"bounded_local_lexical_scan"});
                }
                partial = true;
                continue;
            }
            let local = Context::new(self.vault, self.access.clone());
            let args=serde_json::from_value(json!({"query":query,"target":lexical_target,"lexical_match":"natural_question","limit":10,"budget_bytes":8192})).map_err(|_|"内部检索参数无效")?;
            let page = local.search(args)?;
            if lexical_target == "memories" {
                memory_status = page["status"].clone();
            } else {
                attachment_status = page["coverage"]["attachments"].clone();
                partial |= attachment_status["status"] == "partial";
            }
            budget_limited |= page["status"] == "budget_exhausted";
            partial |= page["truncated"] == true;
            let epoch = crate::local_events::source_epoch(self.vault)?;
            let mut rows = vec![];
            for found in page["results"].as_array().ok_or("记忆候选响应无效")? {
                let reference = found["ref"].as_str().ok_or("记忆候选缺少引用")?;
                let read:ReadPageArgs=serde_json::from_value(json!({"refs":[reference],"detail":"compact","offset_bytes":found["text_range"]["start_byte"].as_u64().unwrap_or(0),"budget_bytes":2048})).map_err(|_|"内部读取参数无效")?;
                let resolved = local.read_page(read)?;
                let Some(row) = resolved["results"].as_array().and_then(|r| r.first()) else {
                    partial = true;
                    if resolved["status"] == "budget_exhausted" {
                        budget_limited = true;
                        if lexical_target == "memories" {
                            memory_status = json!("budget_exhausted");
                        }
                    }
                    continue;
                };
                let mut value = found.clone();
                for key in ["text", "snapshot", "text_range"] {
                    value[key] = row[key].clone();
                }
                value["text_hash"] =
                    json!(hash(row["text"].as_str().ok_or("记忆正文无效")?.as_bytes()));
                value["text_truncated"] = json!(
                    row["text_range"]["start_byte"] != 0
                        || row["text_range"]["end_byte"] != row["text_range"]["total_bytes"]
                );
                let locator = Locator {
                    scope: found["scope"].as_str().ok_or("记忆缺少范围")?.into(),
                    reference: reference.into(),
                    snapshot: row["snapshot"].as_str().ok_or("记忆缺少快照")?.into(),
                    epoch: epoch.clone(),
                    config: "-".into(),
                    scopes: scopes_digest.clone(),
                    offset: row["text_range"]["start_byte"]
                        .as_u64()
                        .ok_or("记忆范围无效")? as usize,
                };
                value["id"] = json!(locator.id());
                value["relevance"] = json!("unassessed");
                let support = crate::local_events::ranking::assess_excerpt(
                    query,
                    value["text"].as_str().unwrap_or(""),
                    value["text_range"]["start_byte"] == 0,
                    value["text_range"]["end_byte"] == value["text_range"]["total_bytes"],
                );
                value["retrieval_support"] = json!({"kind":"lexical_candidate","match_scope":"returned_excerpt","protected":support.complete,"protected_anchors":support.protected_anchors,"matched_protected_anchors":support.matched_protected_anchors,"limited":support.limited});
                rows.push(value);
            }
            groups.push(rows);
        }
        attachment_status["semantic"] = json!(attachment_semantic_available);
        let all_candidates = groups
            .iter()
            .flatten()
            .map(|r| {
                (
                    (
                        r["scope"].to_string(),
                        r["ref"].to_string(),
                        r["snapshot"].to_string(),
                    ),
                    r["kind"].as_str().unwrap_or("unknown").to_owned(),
                )
            })
            .collect::<std::collections::BTreeMap<_, _>>();
        let rows = merge_candidates(&groups);
        let selected = rows
            .iter()
            .map(|r| {
                (
                    r["scope"].to_string(),
                    r["ref"].to_string(),
                    r["snapshot"].to_string(),
                )
            })
            .collect::<BTreeSet<_>>();
        let mut omitted = std::collections::BTreeMap::<String, usize>::new();
        for (key, kind) in &all_candidates {
            if !selected.contains(key) {
                *omitted.entry(kind.clone()).or_default() += 1;
            }
        }
        partial |= rows.len() < all_candidates.len();
        budget_limited |= !omitted.is_empty();
        let mut result = json!({"results":rows,"match_count":rows.len(),"status":if partial {"partial"}else if rows.is_empty(){"no_verified_candidates"}else{"ok"},"truncated":partial,"budget_unit":"utf8_bytes","coverage":{"event_search":"local_only","memory_search":"lexical_only_no_cloud_fallback","memory_status":memory_status,"attachments":attachment_status,"budget_limited":budget_limited,"budget_omitted_by_kind":omitted,"local_event_semantic":semantic_available,"scopes":coverage,"cloud_fallback":false,"relevance":"unassessed","merge":"protected_excerpt_tiers_then_stable_round_robin_not_cross_model_score","candidate_budget_policy":"compact_metadata_then_keep_source_kind_within_protected_tier/v1","note":"候选不是答案；未配置或未覆盖不表示没有历史。精确锚点只在有界候选预览中核对，语义相近也可能完全无关；尚无经过校准的相关性或可回答性判定。旧search仍可显式进行关键词检索。"}});
        if result["results"].as_array().unwrap().is_empty() && partial {
            result["status"] = json!(if budget_limited {
                "budget_exhausted"
            } else {
                "unavailable"
            });
        }
        fit_search_budget(&mut result)?;
        Ok(result)
    }
    pub fn recall_fetch(&self, args: RecallFetchArgs) -> Result<Value> {
        let mut locator = Locator::parse(&args.id)?;
        if locator.scopes != scopes_hash(&self.access) || !self.access.permits(&locator.scope) {
            return Err("资料范围已变化，请重新检索".into());
        }
        let _guard = self.vault.read_guard()?;
        if locator.config == "-" && crate::local_events::source_epoch(self.vault)? != locator.epoch
        {
            return Err("资料来源已变化，请重新检索".into());
        }
        if locator.config != "-" {
            LocalEvents::open(self.vault, &locator.scope)?
                .ok_or("本机索引已停用，请重新检索")?
                .verify_source(&locator.epoch, &locator.config, &locator.reference)?;
        }
        let read=serde_json::from_value(json!({"refs":[locator.reference],"detail":"compact","offset_bytes":locator.offset,"budget_bytes":6144})).map_err(|_|"内部读取参数无效")?;
        let page = self.read_page(read)?;
        let row = page["results"]
            .as_array()
            .and_then(|r| r.first())
            .ok_or("当前读取预算不足，请使用原始读取入口")?;
        if row["ref"] != locator.reference
            || row["snapshot"] != locator.snapshot
            || row["metadata"]["scope"] != locator.scope
            || row["text_range"]["start_byte"] != locator.offset
            || row["text_range"]["projection"]
                != if locator.reference.starts_with("attachment:") {
                    attachments::PROJECTION
                } else if locator.config == "-" {
                    "memory.content"
                } else {
                    "event.text"
                }
        {
            return Err("资料版本或范围已变化，请重新检索".into());
        }
        let mut row = row.clone();
        row["id"] = json!(args.id);
        row["text_hash"] = json!(hash(row["text"].as_str().ok_or("读取正文无效")?.as_bytes()));
        let end = row["text_range"]["end_byte"]
            .as_u64()
            .ok_or("读取范围无效")? as usize;
        let next = if page["truncated"] == true && end > locator.offset {
            locator.offset = end;
            Some(locator.id())
        } else {
            None
        };
        let result = json!({"results":[row],"next_id":next,"status":page["status"],"truncated":page["truncated"],"budget_unit":"utf8_bytes","reference_data":true});
        if serde_json::to_vec(&result)
            .map_err(|_| "读取结果无法序列化")?
            .len()
            > BUDGET
        {
            return Err("读取结果超过固定预算".into());
        }
        Ok(result)
    }
}

#[cfg(test)]
mod ranking_tests {
    use super::*;
    fn row(id: &str, protected: bool) -> Value {
        json!({"ref":id,"retrieval_support":{"protected":protected}})
    }
    #[test]
    fn protection_precedes_other_scopes_and_preserves_fair_ties() {
        let rows = merge_candidates(&[
            vec![row("a-semantic", false), row("a-exact", true)],
            vec![row("b-exact", true), row("b-semantic", false)],
        ]);
        let refs: Vec<_> = rows.iter().map(|r| r["ref"].as_str().unwrap()).collect();
        assert_eq!(refs, vec!["a-exact", "b-exact", "a-semantic", "b-semantic"]);
        assert_eq!(rows[3]["final_rank"], 4);
    }
    #[test]
    fn no_protected_anchor_retains_previous_scope_round_robin() {
        let rows = merge_candidates(&[
            vec![row("a1", false), row("a2", false)],
            vec![row("b1", false)],
        ]);
        assert_eq!(
            rows.iter()
                .map(|r| r["ref"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["a1", "b1", "a2"]
        );
    }
    #[test]
    fn cap_cannot_put_semantic_candidates_ahead_of_protected_candidates() {
        let all = vec![
            (0..20)
                .map(|i| row(&format!("semantic-{i}"), false))
                .collect(),
            (0..20).map(|i| row(&format!("exact-{i}"), true)).collect(),
        ];
        let rows = merge_candidates(&all);
        assert_eq!(rows.len(), LIMIT);
        assert!(rows
            .iter()
            .all(|r| r["retrieval_support"]["protected"] == true));
    }
    fn full_row(kind: &str, number: usize, text_bytes: usize) -> Value {
        let reference = format!("{kind}:synthetic-{number}");
        let text = "x".repeat(text_bytes);
        json!({"ref":reference,"kind":kind,"scope":"personal","snapshot":"a".repeat(64),
            "id":format!("rcs1|personal|{reference}|{}|{}|{}|{}|0", "a".repeat(64), "b".repeat(64), "c".repeat(64), "d".repeat(64)),
            "text":text,"text_hash":hash(text.as_bytes()),"text_range":{"start_byte":0,"end_byte":text_bytes,"total_bytes":text_bytes,"projection":if kind=="attachment"{"attachment.utf8/1"}else{"event.text"}},
            "text_truncated":false,"role":if kind=="attachment"{Value::Null}else{json!("user")},
            "evidence":if kind=="attachment"{"Attachment/Content"}else{"User/UserInput"},"evidence_refs":["event:parent"],
            "score":0.5,"score_kind":"cosine","semantic_rank":number+1,
            "content_hash":"e".repeat(64),"matched_chunk":{"start_byte":0,"end_byte":text_bytes,"text_hash":"f".repeat(64)},
            "session_ref":"s".repeat(128),"retrieval":{"semantic_rank":number+1,"lexical_rank":null,"semantic_evidence":{"score":0.5}},
            "retrieval_support":{"protected":false,"kind":"semantic_candidate","case_policy":"protected_case_sensitive_ascii_support_case_insensitive"}})
    }
    fn budget_page(rows: Vec<Value>) -> Value {
        json!({"results":rows,"match_count":rows.len(),"status":"ok","truncated":false,
            "coverage":{"budget_limited":false,"note":"synthetic coverage ".repeat(70)}})
    }
    #[test]
    fn compact_diagnostics_before_discarding_otherwise_fitting_sources() {
        let original = vec![
            full_row("attachment", 0, 1050),
            full_row("event", 1, 1050),
            full_row("event", 2, 1050),
        ];
        let mut page = budget_page(original.clone());
        // Negative control: the previous tail-pop loop loses a source here.
        assert!(serde_json::to_vec(&page).unwrap().len() > BUDGET);
        fit_search_budget(&mut page).unwrap();
        assert!(serde_json::to_vec(&page).unwrap().len() <= BUDGET);
        assert_eq!(page["results"].as_array().unwrap().len(), 3);
        assert_eq!(page["coverage"]["budget_omitted_by_kind"], json!({}));
        for (before, after) in original.iter().zip(page["results"].as_array().unwrap()) {
            for key in [
                "id",
                "ref",
                "snapshot",
                "text",
                "text_hash",
                "text_range",
                "role",
                "evidence_refs",
                "content_hash",
                "matched_chunk",
                "retrieval",
            ] {
                assert_eq!(before[key], after[key], "{key}");
            }
        }
    }
    #[test]
    fn byte_budget_keeps_only_event_before_removing_repeated_attachments() {
        let mut page = budget_page(vec![
            full_row("attachment", 0, 3100),
            full_row("attachment", 1, 3100),
            full_row("event", 2, 200),
        ]);
        fit_search_budget(&mut page).unwrap();
        let rows = page["results"].as_array().unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["kind"], "attachment");
        assert_eq!(rows[1]["kind"], "event");
        assert_eq!(page["coverage"]["budget_omitted_by_kind"]["attachment"], 1);
        assert!(serde_json::to_vec(&page).unwrap().len() <= BUDGET);
        assert_eq!(rows[1]["final_rank"], 2);
    }
    #[test]
    fn source_kind_reservation_never_promotes_unprotected_over_protected() {
        let mut rows = vec![
            full_row("attachment", 0, 1),
            full_row("attachment", 1, 1),
            full_row("event", 2, 1),
        ];
        rows[0]["retrieval_support"]["protected"] = json!(true);
        rows[1]["retrieval_support"]["protected"] = json!(true);
        assert_eq!(budget_drop_index(&rows), 2);
    }
    #[test]
    fn result_limit_retains_source_kind_and_deduplicates_lexical_lane() {
        let mut mixed: Vec<_> = (0..10).map(|n| full_row("attachment", n, 1)).collect();
        mixed.push(full_row("event", 10, 1));
        let rows = merge_candidates(&[mixed.clone(), vec![mixed[0].clone()]]);
        assert_eq!(rows.len(), LIMIT);
        assert_eq!(rows.iter().filter(|row| row["kind"] == "event").count(), 1);
        assert_eq!(
            rows.iter()
                .filter(|row| row["ref"] == mixed[0]["ref"])
                .count(),
            1
        );
        assert_eq!(rows[9]["final_rank"], 10);
    }
    #[test]
    fn oversized_first_source_does_not_evict_smaller_other_kind() {
        let mut page = budget_page(vec![
            full_row("attachment", 0, 9000),
            full_row("event", 1, 100),
        ]);
        fit_search_budget(&mut page).unwrap();
        assert_eq!(page["results"].as_array().unwrap().len(), 1);
        assert_eq!(page["results"][0]["kind"], "event");
        assert_eq!(page["coverage"]["budget_omitted_by_kind"]["attachment"], 1);
    }
    #[test]
    fn legacy_semantic_rank_survives_compaction_and_small_pages_are_unchanged() {
        let mut source = full_row("event", 0, 100);
        source["retrieval"] = Value::Null;
        let mut small = budget_page(vec![source.clone()]);
        let original = small.clone();
        fit_search_budget(&mut small).unwrap();
        assert_eq!(small, original);
        let mut large = budget_page(vec![source, full_row("attachment", 1, 9000)]);
        fit_search_budget(&mut large).unwrap();
        assert_eq!(large["results"][0]["semantic_rank"], 1);
        assert!(large["results"][0]["retrieval"].is_null());
    }
}
