//! Explicit candidate selection before graph expansion. Scores are not confidence.
use super::*;

const EXCERPT_BYTES: usize = 512;
const MIN_EXCERPT_BYTES: usize = 128;

impl Context<'_> {
    pub(super) fn session_candidates_response(
        &self,
        ranked: &[search::Match<'_>],
        filtered: &[Document],
        args: &SearchArgs,
        search_binding: &str,
        guard: &crate::vault::WriteGuard,
        vector_coverage: Value,
    ) -> Result<Value> {
        let mut snapshot = crate::vault::EventSnapshot::new(self.vault, guard);
        let events = snapshot.all()?;
        let mut seen = BTreeSet::new();
        let mut candidates = Vec::new();
        for entry in ranked {
            let event = events
                .get(entry.document.reference.trim_start_matches("event:"))
                .ok_or("候选来源已变化，请重新搜索")?;
            if seen.insert(session_context::graph_key(event)) {
                candidates.push((entry, event));
            }
        }
        let generation = hash(
            &serde_json::to_vec(
                &candidates
                    .iter()
                    .map(|(_, event)| event)
                    .collect::<Vec<_>>(),
            )
            .map_err(|e| e.to_string())?,
        );
        let binding = hash(
            json!([
                search_binding,
                generation,
                "session_candidates_v1",
                args.limit,
                args.budget_tokens
            ])
            .to_string()
            .as_bytes(),
        );
        let offset = if let Some(cursor) = &args.cursor {
            let parts: Vec<_> = cursor.split(':').collect();
            if cursor.len() > 160 || parts.len() != 3 || parts[0] != "cs1" || parts[1] != binding {
                return Err("候选游标已失效；保持mode、预算、limit、查询、范围和过滤条件".into());
            }
            parts[2].parse::<usize>().map_err(|_| "候选游标偏移无效")?
        } else {
            0
        };
        if offset > candidates.len() {
            return Err("候选游标超出范围".into());
        }
        let score_kind = vector_coverage["score_kind"]
            .as_str()
            .unwrap_or("existing_ranking_score");
        let mut coverage = json!({"relevance":"unassessed","scores_are_confidence":false,
            "source_grouping":"same_scope_session_and_official_source_graph",
            "ordering":"best_ranked_anchor_per_distinct_source_session",
            "scope_filtered":true,"event_filter":args.event_filter,"as_of":args.as_of,
            "eligible_event_count":filtered.len(),"retrieved_event_candidates":ranked.len(),
            "indexed_eligible_event_count":vector_coverage["eligible_events"],
            "event_vectors":vector_coverage,"max_excerpt_utf8_bytes":EXCERPT_BYTES,
            "continuation":"next_cursor lists more candidate sessions, never text tails or graph neighbors",
            "selection":"Choose an evidenced candidate before read; unresolved relevance is not no_matches. Keep its anchor and filters when expanding."});
        search::add_question_coverage(&mut coverage, args);
        let envelope = |results: Vec<Value>| {
            let next = offset + results.len();
            json!({"results":results,"candidate_count":candidates.len(),"status_scope":"ranked_candidate_sessions",
                "status":if candidates.is_empty(){"no_matches"}else if next<candidates.len(){"partial"}else{"complete"},
                "selection_required":!candidates.is_empty(),"relevance":"unassessed",
                "next_cursor":(next<candidates.len() && next>offset).then(||format!("cs1:{binding}:{next}")),
                "truncated":next<candidates.len(),"budget_unit":"utf8_bytes","coverage":coverage})
        };
        let make_item = |index: usize, bytes: usize| {
            let (entry, event) = candidates[index];
            let doc = entry.document;
            let (start, end) = if let Some((start, until)) = entry.hit_range {
                let mut end = (start + bytes).min(until);
                while !doc.text.is_char_boundary(end) {
                    end -= 1;
                }
                (start, end)
            } else {
                matching_window(&doc.text, &args.query, bytes, &args.lexical_match)
            };
            let session_ref = doc.session_ref.as_ref().filter(|value| value.len() <= 256);
            let source = &event.data.source;
            let mut item = json!({"rank":index+1,"ref":doc.reference,"anchor_ref":doc.reference,"session_ref":session_ref,
                "scope":doc.scope,"source_identity":{"platform":truncate_utf8(&source.platform,128),
                    "account_namespace":truncate_utf8(&source.account_namespace,128),"conversation_id":truncate_utf8(&source.conversation_id,256),
                    "truncated":source.platform.len()>128 || source.account_namespace.len()>128 || source.conversation_id.len()>256,
                    "group_key":hash(session_context::graph_key(event).as_bytes())},
                "role":doc.role,"occurred_at":doc.occurred_at,"on_current_path":doc.on_current_path,
                "score":entry.score,"score_kind":score_kind,"relevance":"unassessed",
                "snapshot":hash(json!(event).to_string().as_bytes()),"text":&doc.text[start..end],
                "text_truncated":start!=0 || end!=doc.text.len(),
                "text_range":{"start_byte":start,"end_byte":end,"total_bytes":doc.text.len(),"projection":"event.text"},
                "read_args":{"refs":[doc.reference],"context_anchor_ref":doc.reference,"include_adjacent":true,
                    "detail":"compact","event_filter":args.event_filter,"as_of":args.as_of,"offset_bytes":start,"budget_bytes":8192}});
            if session_ref.is_none() && doc.session_ref.is_some() {
                item["session_ref_truncated"] = json!(true);
                item["session_label"] =
                    json!(truncate_utf8(doc.session_ref.as_deref().unwrap(), 256));
            }
            item
        };
        if offset == candidates.len() {
            let full = envelope(vec![]);
            if json_size(&full)? <= args.budget_tokens {
                return Ok(full);
            }
            return compact_status(
                candidates.len(),
                filtered.len(),
                &vector_coverage,
                None,
                &coverage,
                args.budget_tokens,
            );
        }
        let mut count = args.limit.min(candidates.len() - offset);
        let mut required = 512;
        // Share excerpt space across distinct sessions before considering another
        // excerpt from the first session. Long top-ranked bodies cannot starve peers.
        while count > 0 {
            let mut bytes = EXCERPT_BYTES;
            loop {
                let page = envelope(
                    (offset..offset + count)
                        .map(|index| make_item(index, bytes))
                        .collect(),
                );
                let size = json_size(&page)?;
                required = size;
                if size <= args.budget_tokens {
                    return Ok(page);
                }
                if bytes == MIN_EXCERPT_BYTES {
                    break;
                }
                bytes = bytes
                    .saturating_sub((size - args.budget_tokens).div_ceil(count))
                    .max(MIN_EXCERPT_BYTES);
            }
            count -= 1;
        }
        compact_status(
            candidates.len(),
            filtered.len(),
            &vector_coverage,
            Some(required),
            &coverage,
            args.budget_tokens,
        )
    }
}

fn compact_status(
    total: usize,
    eligible: usize,
    vectors: &Value,
    required: Option<usize>,
    coverage: &Value,
    budget: usize,
) -> Result<Value> {
    let mut page = json!({"results":[],"candidate_count":total,"status":if required.is_some(){"budget_exhausted"}else if total==0{"no_matches"}else{"complete"},
        "next_cursor":null,"budget_unit":"utf8_bytes","relevance":"unassessed",
        "coverage":{"eligible_event_count":eligible,"event_vectors":{"status":vectors["status"],"error":vectors["error"]}}});
    if let Some(size) = required {
        page["recommended_min_budget"] = json!(size);
        page["max_budget_bytes"] = json!(32768);
    }
    search::preserve_question_mode(&mut page, coverage);
    if json_size(&page)? > budget {
        return Err("预算不足以输出候选状态".into());
    }
    Ok(page)
}
