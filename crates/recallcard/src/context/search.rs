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
    for (_, doc) in &ranked {
        if doc.kind == "event" {
            if let Some(owner) = owners.get(&doc.reference) {
                related
                    .entry(owner.clone())
                    .or_default()
                    .push(doc.reference.clone());
            }
        }
    }
    ranked
        .into_iter()
        .filter_map(|(score, doc)| {
            if doc.kind == "event" && owners.contains_key(&doc.reference) {
                return None;
            }
            Some(Match {
                score,
                document: doc,
                related_refs: related.remove(&doc.reference).unwrap_or_default(),
            })
        })
        .collect()
}
fn item(entry: &Match<'_>, query: &str, limit: usize) -> Result<Value> {
    let doc = entry.document;
    let mut value = serde_json::to_value(doc).map_err(|e| e.to_string())?;
    value["ref"] = value["reference"].take();
    value.as_object_mut().unwrap().remove("reference");
    value["score"] = json!(entry.score);
    let (start, end) = matching_window(&doc.text, query, limit);
    value["text"] = json!(&doc.text[start..end]);
    value["text_truncated"] = json!(start != 0 || end != doc.text.len());
    value["text_range"] = json!({"start_byte":start,"end_byte":end,"total_bytes":doc.text.len(),"projection":if doc.kind=="event"{"event.text"}else{"memory.content"}});
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
    json!({"results":results,"coverage":coverage,"truncated":next<total,"next_cursor":if next<total&&next>offset{Some(format!("{binding}:{next}"))}else{None},"budget_exhausted":next==offset&&offset<total,"budget_unit":"utf8_bytes","match_count":total,"status":if total==0{"no_matches"}else if next==offset&&offset<total{"budget_exhausted"}else if next==offset{"end_of_results"}else{"results"}})
}
fn compact_status(value: &mut Value) {
    let coverage = &value["coverage"];
    value["coverage"] = json!({"event_search":coverage["event_search"],"semantic_search":coverage["semantic_search"],"scope_filtered":true});
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
        let candidate = item(entry, &args.query, 96)?;
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
        let min = item(&available[0], &args.query, 96)?;
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
            let candidate = item(&available[index], &args.query, middle)?;
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
