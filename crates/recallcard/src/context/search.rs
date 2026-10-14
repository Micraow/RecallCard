//! 词组覆盖、来源合并与公平摘要；不改变授权、抑制和时间过滤。
use super::*;

fn han(c: char) -> bool {
    ('\u{3400}'..='\u{9fff}').contains(&c) || ('\u{f900}'..='\u{faff}').contains(&c)
}
/// 连续中文是完整检索词组；单字仅在用户确实单独查询它时才参与匹配。
/// 不为某个项目或特定词维护字典，也不改可重建索引的分词格式。
pub(super) fn query_terms(text: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut was_han = None;
    for c in text.to_lowercase().chars() {
        let is_han = han(c);
        let word = c.is_alphanumeric() || matches!(c, '_' | '-' | '.' | '/' | ':');
        if (!word || was_han.is_some_and(|old| old != is_han)) && !current.is_empty() {
            words.push(std::mem::take(&mut current));
        }
        if word {
            current.push(c);
            was_han = Some(is_han);
        } else {
            was_han = None;
        }
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
        .into_iter()
        .filter(|s| s.chars().any(char::is_alphanumeric))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}
/// 默认仍按既有长句/标点启发式；natural_question 显式请求汉字双字组候选。
/// 两种路径均保留最少六个连续汉字及引号精确保护，不将候选当作语义答案。
/// 不使用项目名、问题模板、领域词典或验收问题特判；带引号的查询保持精确模式。
pub(super) fn natural_han_query(text: &str, mode: &str) -> bool {
    if text.contains(['"', '“', '”']) {
        return false;
    }
    let terms = query_terms(text);
    let longest = terms
        .iter()
        .filter(|s| s.chars().all(han))
        .map(|s| s.chars().count())
        .max()
        .unwrap_or(0);
    let count = text.chars().filter(|c| han(*c)).count();
    longest >= 6
        && (mode == "natural_question"
            || count >= 12
            || text.contains(['，', '。', '？', '?', '；', '：', ',', ';']))
}
pub(super) fn ranking_terms(text: &str, mode: &str) -> Vec<String> {
    if !natural_han_query(text, mode) {
        return query_terms(text);
    }
    super::tokenize(text)
        .into_iter()
        // 问题里的独立 ASCII 标点不是必需代码标识符；含字母/数字的词保持原样。
        // 仅新模式纠正此候选入口，默认/前缀模式保留历史合同。
        .filter(|s| mode != "natural_question" || s.chars().any(char::is_alphanumeric))
        .filter(|s| !s.chars().all(han) || s.chars().count() == 2)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}
/// 新模式的说明在计算完整 JSON 预算前加入，旧模式响应保持原合同。
pub(super) fn add_question_coverage(coverage: &mut Value, args: &SearchArgs) {
    if args.lexical_match != "natural_question" {
        return;
    }
    coverage["lexical_match"] = json!("natural_question");
    coverage["lexical_matching"] = json!(if natural_han_query(&args.query, &args.lexical_match) {
        "natural_han_bigrams_exact_identifiers"
    } else {
        "all_terms"
    });
    coverage["lexical_note"] = json!("字面候选不等于答案；否定、相对时间与同义关系须核对原文。");
}
/// 最小预算回执仍保留请求模式，不用长说明挤掉可继续读取的状态。
pub(super) fn preserve_question_mode(page: &mut Value, coverage: &Value) {
    if coverage["lexical_match"] == "natural_question" {
        if !page["coverage"].is_object() {
            page["coverage"] = json!({});
        }
        page["coverage"]["lexical_match"] = json!("natural_question");
    }
}
pub(super) fn han_term(term: &str) -> bool {
    term.chars().all(han)
}
pub(super) fn coherent_han_overlap(query: &str, fields: &[String]) -> bool {
    let chars = query.chars().collect::<Vec<_>>();
    chars
        .windows(3)
        .filter(|part| part.iter().all(|c| han(*c)))
        .any(|part| {
            fields
                .iter()
                .any(|field| field.contains(&part.iter().collect::<String>()))
        })
}
pub(super) fn term_matches(field: &str, term: &str) -> bool {
    if term.chars().all(han) {
        return field.contains(term);
    }
    let word = |c: char| (!han(c) && c.is_alphanumeric()) || c == '_';
    field.match_indices(term).any(|(at, _)| {
        !field[..at].chars().next_back().is_some_and(word)
            && !field[at + term.len()..].chars().next().is_some_and(word)
    })
}
pub(super) fn evidence_weight(doc: &Document) -> f64 {
    // 有限权重只在相关候选间偏向直接证据；不能使缺词事实进入结果。
    let evidence = if doc.evidence.starts_with("User/") || doc.evidence == "UserExplicit" {
        1.3
    } else if doc.evidence.starts_with("Assistant/") || doc.evidence == "AssistantSuggestion" {
        0.8
    } else {
        1.0
    };
    let state = if doc.state == "tentative" { 0.85 } else { 1.0 };
    evidence * state
}
pub(super) struct Match<'a> {
    pub score: f64,
    pub document: &'a Document,
    pub related_refs: Vec<String>,
    pub score_source_ref: Option<String>,
    pub hit_range: Option<(usize, usize)>,
}
pub(super) fn group_sources<'a>(ranked: Vec<(f64, &'a Document)>, query: &str) -> Vec<Match<'a>> {
    let mut owners = BTreeMap::new();
    let terms = query_terms(query);
    // 只合并已经分别命中的原始事件；多个不同记忆不因共享来源而被丢弃。
    for (_, doc) in &ranked {
        if doc.kind == "memory"
            && terms.iter().all(|term| {
                std::iter::once(&doc.text)
                    .chain(&doc.entities)
                    .chain(&doc.labels)
                    .any(|field| term_matches(&field.to_lowercase(), term))
            })
        {
            for source in &doc.evidence_refs {
                owners
                    .entry(source.clone())
                    .or_insert(doc.reference.clone());
            }
        }
    }
    let mut related: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut source_scores: BTreeMap<String, (f64, String)> = BTreeMap::new();
    for (score, doc) in &ranked {
        if doc.kind == "event" {
            if let Some(owner) = owners.get(&doc.reference) {
                related
                    .entry(owner.clone())
                    .or_default()
                    .push(doc.reference.clone());
                let best = source_scores
                    .entry(owner.clone())
                    .or_insert((*score, doc.reference.clone()));
                if score.total_cmp(&best.0).is_gt()
                    || (score.total_cmp(&best.0).is_eq() && doc.reference < best.1)
                {
                    *best = (*score, doc.reference.clone());
                }
            }
        }
    }
    let mut groups: Vec<_> = ranked
        .into_iter()
        .filter_map(|(score, doc)| {
            if doc.kind == "event" && owners.contains_key(&doc.reference) {
                return None;
            }
            // 仅保留已授权且实际命中的成员最高分，不因来源数量增加而加分。
            let (score, score_source_ref) = match source_scores.remove(&doc.reference) {
                Some((source_score, reference)) if source_score > score => {
                    (source_score, Some(reference))
                }
                _ => (score, None),
            };
            Some(Match {
                score,
                document: doc,
                related_refs: related.remove(&doc.reference).unwrap_or_default(),
                score_source_ref,
                hit_range: None,
            })
        })
        .collect();
    groups.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then(a.document.reference.cmp(&b.document.reference))
    });
    groups
}
fn item(entry: &Match<'_>, query: &str, limit: usize, mode: &str) -> Result<Value> {
    let doc = entry.document;
    let mut value = serde_json::to_value(doc).map_err(|e| e.to_string())?;
    value["ref"] = value["reference"].take();
    value.as_object_mut().unwrap().remove("reference");
    // hints 是目录生成输入，不能把每条最多 8 KiB 的元数据重复塞进搜索首屏。
    value.as_object_mut().unwrap().remove("navigation");
    if !doc.navigation.is_empty() {
        value["navigation_entry_count"] = json!(doc.navigation.len());
    }
    value["score"] = json!(entry.score);
    if let Some(reference) = &entry.score_source_ref {
        value["score_source_ref"] = json!(reference);
    }
    let (start, end) = if let Some((start, until)) = entry.hit_range {
        value["matched_chunk_range"] = json!({"start_byte":start,"end_byte":until});
        let mut end = (start + limit).min(until);
        while !doc.text.is_char_boundary(end) {
            end -= 1;
        }
        (start, end)
    } else {
        matching_window(&doc.text, query, limit, mode)
    };
    value["text"] = json!(&doc.text[start..end]);
    value["text_truncated"] = json!(start != 0 || end != doc.text.len());
    value["text_range"] = json!({"start_byte":start,"end_byte":end,"total_bytes":doc.text.len(),"projection":match doc.kind.as_str(){"event"=>"event.text","attachment"=>attachments::PROJECTION,"view"=>"view.navigation",_=>"memory.content"}});
    if !entry.related_refs.is_empty() {
        value["related_refs"] = json!(&entry.related_refs[..entry.related_refs.len().min(4)]);
        value["related_ref_count"] = json!(entry.related_refs.len());
    }
    // 来源完整集合由 sources 提供，不让长标签或几百个出处吞掉首屏预算。
    for key in ["evidence_refs", "entities", "labels"] {
        let array = value[key].as_array().unwrap();
        let count = array.len();
        let bounded = array
            .iter()
            .take(4)
            .map(|v| {
                v.as_str()
                    .map(|s| json!(truncate_utf8(s, 128)))
                    .unwrap_or(v.clone())
            })
            .collect::<Vec<_>>();
        if count > 4 || bounded != *array {
            value[format!("{key}_truncated")] = json!(true);
            value[format!("{key}_count")] = json!(count);
        }
        value[key] = json!(bounded);
    }
    if doc.time_note.len() > 128 {
        value["time_note"] = json!(truncate_utf8(&doc.time_note, 128));
        value["time_note_truncated"] = json!(true);
    }
    Ok(value)
}
fn envelope(
    results: Vec<Value>,
    total: usize,
    offset: usize,
    binding: &str,
    coverage: &Value,
) -> Value {
    let next = offset + results.len();
    json!({"results":results,"coverage":coverage,"truncated":next<total || coverage["attachments"]["status"] == "partial","next_cursor":if next<total&&next>offset{Some(format!("{binding}:{next}"))}else{None},"budget_exhausted":next==offset&&offset<total,"budget_unit":"utf8_bytes","match_count":total,"status":if total==0{"no_matches"}else if next==offset&&offset<total{"budget_exhausted"}else if next==offset{"end_of_results"}else{"results"}})
}
fn compact_status(value: &mut Value) {
    let coverage = &value["coverage"];
    let mut compact = json!({"event_search":coverage["event_search"],"semantic_search":coverage["semantic_search"],"scope_filtered":true});
    if let Some(vectors) = coverage.get("event_vectors") {
        compact["event_vectors"] = json!({"status":vectors["status"],"error":vectors["error"],"ranking_policy":vectors["ranking_policy"],"query_encoding":"existing_cache_only","network_requests":0});
    }
    if let Some(filter) = coverage.get("event_filter") {
        compact["event_filter"] = filter.clone();
    }
    if let Some(nav) = coverage.get("navigation") {
        compact["navigation"] = json!({"view_candidates":nav["view_candidates"],"fact_candidates":nav["fact_candidates"]});
    }
    if coverage["lexical_match"] == "natural_question" {
        compact["lexical_match"] = coverage["lexical_match"].clone();
    }
    if let Some(attachments) = coverage.get("attachments") {
        compact["attachments"] = json!({"status":attachments["status"],"limited":attachments["limited"],"skipped":attachments["skipped"]});
    }
    value["coverage"] = compact;
}
pub(super) fn response(
    ranked: &[Match<'_>],
    args: &SearchArgs,
    offset: usize,
    binding: &str,
    coverage: Value,
) -> Result<Value> {
    if offset > ranked.len() {
        return Err("游标超出范围".into());
    }
    let available = &ranked[offset..];
    let mut items = Vec::new();
    // 先预留每条可用的最短真实片段，再分配剩余空间，避免第一条长文独占。
    for entry in available.iter().take(args.limit) {
        let candidate = item(entry, &args.query, 96, &args.lexical_match)?;
        items.push(candidate);
        if json_size(&envelope(
            items.clone(),
            ranked.len(),
            offset,
            binding,
            &coverage,
        ))? > args.budget_tokens
        {
            items.pop();
            break;
        }
    }
    if items.is_empty() && !available.is_empty() {
        let min = item(&available[0], &args.query, 96, &args.lexical_match)?;
        let required = json_size(&envelope(
            vec![min],
            ranked.len(),
            offset,
            binding,
            &coverage,
        ))?;
        let mut empty = envelope(items, ranked.len(), offset, binding, &coverage);
        empty["recommended_min_budget"] = json!(required);
        empty["max_budget_bytes"] = json!(32768);
        empty["hint"] = json!("按 recommended_min_budget 重试同一查询/游标；预算不足不是无匹配");
        if json_size(&empty)? > args.budget_tokens {
            compact_status(&mut empty);
            if json_size(&empty)? > args.budget_tokens {
                empty.as_object_mut().unwrap().remove("hint");
            }
        }
        if json_size(&empty)? > args.budget_tokens {
            return Err("预算不足以容纳搜索状态".into());
        }
        return Ok(empty);
    }
    let count = items.len();
    for index in 0..count {
        let used = json_size(&envelope(
            items.clone(),
            ranked.len(),
            offset,
            binding,
            &coverage,
        ))?;
        let share = (args.budget_tokens - used) / (count - index);
        let cap = if args.detail == "brief" { 240 } else { 2400 };
        let mut low = 96usize.min(cap);
        let mut high = (96 + share)
            .min(cap)
            .min(available[index].document.text.len().max(96));
        while low < high {
            let middle = low + (high - low).div_ceil(2);
            let candidate = item(&available[index], &args.query, middle, &args.lexical_match)?;
            let previous = std::mem::replace(&mut items[index], candidate);
            if json_size(&envelope(
                items.clone(),
                ranked.len(),
                offset,
                binding,
                &coverage,
            ))? <= args.budget_tokens
            {
                low = middle;
            } else {
                items[index] = previous;
                high = middle - 1;
            }
        }
    }
    let mut result = envelope(items, ranked.len(), offset, binding, &coverage);
    if count == 0 && json_size(&result)? > args.budget_tokens {
        compact_status(&mut result);
    }
    if json_size(&result)? > args.budget_tokens {
        return Err("预算不足以容纳搜索状态".into());
    }
    Ok(result)
}

pub(super) fn term_matches_mode(field: &str, term: &str, mode: &str) -> bool {
    if term_matches(field, term) {
        return true;
    }
    if mode != "identifier_prefix"
        || term.len() < 3
        || !term.bytes().all(|c| c.is_ascii_alphabetic())
    {
        return false;
    }
    let word = |c: char| (!han(c) && c.is_alphanumeric()) || c == '_';
    field
        .match_indices(term)
        .any(|(at, _)| !field[..at].chars().next_back().is_some_and(word))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn document(reference: &str, kind: &str, sources: &[&str]) -> Document {
        Document {
            navigation_scopes: vec![],
            navigation: vec![],
            role: None,
            on_current_path: None,
            reference: reference.into(),
            text: "合成命中".into(),
            scope: "personal".into(),
            kind: kind.into(),
            evidence_refs: sources.iter().map(|s| (*s).into()).collect(),
            session_ref: None,
            state: "tentative".into(),
            occurred_at: None,
            valid_from: None,
            valid_to: None,
            time_note: String::new(),
            evidence: "AssistantSuggestion".into(),
            labels: vec![],
            entities: vec![],
            protected: false,
        }
    }

    #[test]
    fn grouped_score_is_maximum_with_exact_deterministic_provenance() {
        let memory = document("memory:m", "memory", &["event:a", "event:b"]);
        let a = document("event:a", "event", &[]);
        let b = document("event:b", "event", &[]);
        let stronger = document("event:stronger", "event", &[]);
        let groups = group_sources(
            vec![(5.0, &stronger), (4.0, &b), (4.0, &a), (2.0, &memory)],
            "命中",
        );
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].document.reference, "event:stronger");
        assert_eq!(groups[1].score, 4.0, "不能把两个来源的得分相加");
        assert_eq!(groups[1].score_source_ref.as_deref(), Some("event:a"));
        let serialized = item(&groups[1], "命中", 96, "exact").unwrap();
        assert_eq!(serialized["score_source_ref"], "event:a");
        assert_eq!(serialized["evidence"], "AssistantSuggestion");
        assert_eq!(serialized["state"], "tentative");
        assert_eq!(serialized["text"], "合成命中");
    }

    #[test]
    fn memory_score_is_not_falsely_attributed_to_a_weaker_or_equal_source() {
        let memory = document("memory:m", "memory", &["event:a"]);
        let source = document("event:a", "event", &[]);
        for source_score in [2.0, 5.0] {
            let groups = group_sources(vec![(5.0, &memory), (source_score, &source)], "命中");
            assert_eq!(groups[0].score, 5.0);
            assert_eq!(groups[0].score_source_ref, None);
            assert!(item(&groups[0], "命中", 96, "exact")
                .unwrap()
                .get("score_source_ref")
                .is_none());
        }
    }

    #[test]
    fn absent_source_scores_and_nonmatching_memories_do_not_create_inheritance() {
        let mut memory = document("memory:m", "memory", &["event:a"]);
        let source = document("event:a", "event", &[]);
        let groups = group_sources(vec![(2.0, &memory)], "命中");
        assert_eq!(groups[0].score, 2.0);
        assert_eq!(groups[0].score_source_ref, None);
        memory.text = "另一段没有查询词的说明".into();
        let groups = group_sources(vec![(5.0, &source), (2.0, &memory)], "命中");
        assert_eq!(groups.len(), 2);
        assert!(groups
            .iter()
            .all(|g| g.score_source_ref.is_none() && g.related_refs.is_empty()));
    }
}
