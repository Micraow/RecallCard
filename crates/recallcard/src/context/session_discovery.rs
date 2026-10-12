//! Independent retrieval channels, then bounded source context for caller assessment.
use super::*;
use confirmation::{CallerAssessment, ConfirmationBinding, EvidenceSpan};
const MAX_CHANNEL_SESSIONS: usize = 256;

#[derive(Default)]
struct Channels<'a> {
    semantic: Option<(usize, f64, &'a Document)>,
    lexical: Option<(usize, f64, &'a Document)>,
}

impl Context<'_> {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn session_discovery_response(
        &self,
        lexical: &[(f64, &Document)],
        vectors: Option<&crate::event_vectors::Hits>,
        current: &[Document],
        filtered: &[Document],
        args: &SearchArgs,
        search_binding: &str,
        guard: &crate::vault::WriteGuard,
        vector_coverage: Value,
    ) -> Result<Value> {
        let mut snapshot = crate::vault::EventSnapshot::new(self.vault, guard);
        let events = snapshot.all()?.clone();
        let docs: BTreeMap<_, _> = filtered.iter().map(|d| (d.reference.as_str(), d)).collect();
        let key = |doc: &Document| -> Result<String> {
            events
                .get(doc.reference.trim_start_matches("event:"))
                .map(session_context::graph_key)
                .ok_or("候选来源已变化".into())
        };
        let mut channels: BTreeMap<String, Channels<'_>> = BTreeMap::new();
        let mut semantic_groups = Vec::new();
        let mut lexical_groups = Vec::new();
        let mut semantic_seen = BTreeSet::new();
        let mut lexical_seen = BTreeSet::new();
        if let Some(vectors) = vectors {
            for reference in &vectors.references {
                let Some(doc) = docs.get(reference.as_str()).copied() else {
                    continue;
                };
                let k = key(doc)?;
                if semantic_seen.insert(k.clone()) && semantic_groups.len() < MAX_CHANNEL_SESSIONS {
                    let entry = channels.entry(k.clone()).or_default();
                    semantic_groups.push(k);
                    entry.semantic = Some((semantic_groups.len(), vectors.scores[reference], doc));
                }
            }
        }
        for (score, doc) in lexical {
            let k = key(doc)?;
            if lexical_seen.insert(k.clone()) && lexical_groups.len() < MAX_CHANNEL_SESSIONS {
                let entry = channels.entry(k.clone()).or_default();
                lexical_groups.push(k);
                entry.lexical = Some((lexical_groups.len(), *score, doc));
            }
        }
        // Source-session deduplication occurs independently, before fixed round-robin.
        // This is discovery order, not a relevance ranking or tuned weighted fusion.
        let mut seen = BTreeSet::new();
        let mut ordered = Vec::new();
        for i in 0..semantic_groups.len().max(lexical_groups.len()) {
            for (name, list) in [("semantic", &semantic_groups), ("lexical", &lexical_groups)] {
                if let Some(k) = list.get(i) {
                    if seen.insert(k.clone()) {
                        ordered.push((k.clone(), name));
                    }
                }
            }
        }
        let scope_hash = hash(json!(self.access.scopes()).to_string().as_bytes());
        let suppressed = self.vault.suppressed_ids()?;
        // Bind the full current source graph, not only a displayed or filtered excerpt.
        let visible_events = current
            .iter()
            .filter(|d| d.kind == "event")
            .filter_map(|d| events.get(d.reference.trim_start_matches("event:")))
            .collect::<Vec<_>>();
        let generation = hash(json!(visible_events).to_string().as_bytes());
        let source_ranks = channels
            .iter()
            .map(|(k, c)| {
                json!([
                    k,
                    c.semantic.map(|(r, s, d)| (r, s, &d.reference)),
                    c.lexical.map(|(r, s, d)| (r, s, &d.reference))
                ])
            })
            .collect::<Vec<_>>();
        let make_binding = |budget| {
            hash(
                json!([
                    "session_discovery_v1",
                    search_binding,
                    generation,
                    args.limit,
                    budget,
                    source_ranks,
                    vector_coverage["index_fingerprint"],
                    vector_coverage["query_encoding_profile"]
                ])
                .to_string()
                .as_bytes(),
            )
        };
        let binding = make_binding(args.budget_tokens);
        let offset = if let Some(cursor) = &args.cursor {
            let parts: Vec<_> = cursor.split(':').collect();
            if cursor.len() > 160 || parts.len() != 3 || parts[0] != "sd1" || parts[1] != binding {
                return Err(
                    "发现游标已失效；保持查询、范围、profile、模式、limit与字节预算".into(),
                );
            }
            parts[2].parse::<usize>().map_err(|_| "发现游标偏移无效")?
        } else {
            0
        };
        if offset > ordered.len() {
            return Err("发现游标超出范围".into());
        }
        let mut drafts = Vec::new();
        for (position, (k, first_channel)) in
            ordered.iter().enumerate().skip(offset).take(args.limit)
        {
            let c = &channels[k];
            let (_, _, doc) = if *first_channel == "semantic" {
                c.semantic.unwrap()
            } else {
                c.lexical.unwrap()
            };
            let event = &events[doc.reference.trim_start_matches("event:")];
            let mut read = ReadPageArgs {
                refs: vec![doc.reference.clone()],
                budget_tokens: 8192,
                cursor: None,
                offset_bytes: None,
                include_adjacent: true,
                event_filter: args.event_filter.clone(),
                as_of: args.as_of,
                context_anchor_ref: Some(doc.reference.clone()),
                context_confirmation: None,
                detail: "compact".into(),
            };
            let (links, graph_generation) =
                match self.adjacent(event, &mut snapshot, &suppressed, &read) {
                    Ok(Some(value)) => value,
                    _ => (
                        json!({"constraints":{"stop_reasons":["source_graph_unverifiable"]}}),
                        String::new(),
                    ),
                };
            let confirmation_available =
                !graph_generation.is_empty() && links["ambiguous_source_identity"] != true;
            let mut references = vec![(doc.reference.clone(), "anchor")];
            if let Some(r) = links["previous_ref"].as_str() {
                references.push((r.to_owned(), "previous"));
            }
            if let Some(r) = links["next_refs"]
                .as_array()
                .filter(|v| v.len() == 1)
                .and_then(|v| v[0].as_str())
            {
                references.push((r.to_owned(), "next"));
            }
            let mut context = Vec::new();
            let mut evidence = Vec::new();
            for (reference, relation) in references {
                let Some(d) = docs.get(reference.as_str()).copied() else {
                    continue;
                };
                let e = &events[reference.trim_start_matches("event:")];
                let (start, end) = if relation == "anchor" {
                    if *first_channel == "semantic" {
                        let (a, b) = vectors
                            .and_then(|v| v.ranges.get(&reference))
                            .copied()
                            .ok_or("向量命中范围缺失")?;
                        let mut end = (a + 512).min(b);
                        while !d.text.is_char_boundary(end) {
                            end -= 1;
                        }
                        (a, end)
                    } else {
                        matching_window(&d.text, &args.query, 512)
                    }
                } else {
                    // A short deterministic original slice, never a synthesized summary.
                    let mut end = 512.min(d.text.len());
                    while !d.text.is_char_boundary(end) {
                        end -= 1;
                    }
                    (0, end)
                };
                if relation == "anchor" {
                    read.offset_bytes = Some(start);
                }
                if start == end {
                    continue;
                }
                let snap = confirmation::snapshot(e);
                evidence.push(EvidenceSpan {
                    reference: reference.clone(),
                    snapshot: snap.clone(),
                    projection: "event.text".into(),
                    start_byte: start,
                    end_byte: end,
                    text_hash: hash(&d.text.as_bytes()[start..end]),
                });
                context.push(json!({"ref":reference,"snapshot":snap,"role":d.role,"occurred_at":d.occurred_at,
                    "relation":relation,"text":&d.text[start..end],"text_range":{"start_byte":start,"end_byte":end,"total_bytes":d.text.len(),"projection":"event.text"},"text_truncated":start>0||end<d.text.len()}));
            }
            let confirmation = ContextConfirmation::new(ConfirmationBinding {
                version: "recallcard.context-confirmation/1".into(),
                query_hash: hash(args.query.as_bytes()),
                anchor_ref: doc.reference.clone(),
                anchor_snapshot: confirmation::snapshot(event),
                source_group_key: hash(k.as_bytes()),
                scopes_hash: scope_hash.clone(),
                event_filter: args.event_filter.clone(),
                as_of: args.as_of,
                graph_generation,
                evidence,
            });
            read.context_confirmation = Some(confirmation);
            let source = &event.data.source;
            let channel_value = |value: Option<(usize, f64, &Document)>| {
                value.map(|(rank,score,doc)|json!({"session_rank":rank,"raw_score":score,"anchor_ref":doc.reference}))
            };
            drafts.push(json!({"discovery_position":position+1,"anchor_ref":doc.reference,"session_ref":doc.session_ref.as_deref().filter(|s|s.len()<=256),
                "source_identity":{"platform":truncate_utf8(&source.platform,128),"account_namespace":truncate_utf8(&source.account_namespace,128),
                "conversation_id":truncate_utf8(&source.conversation_id,256),"group_key":hash(k.as_bytes()),
                "truncated":source.platform.len()>128||source.account_namespace.len()>128||source.conversation_id.len()>256||doc.session_ref.as_ref().is_some_and(|s|s.len()>256)},
                "channels":{"semantic":channel_value(c.semantic),"lexical":channel_value(c.lexical)},
                "first_channel":first_channel,"score_units":{"semantic":"cosine","lexical":"existing_lexical_score"},
                "relevance":"unassessed","confirmation_available":confirmation_available,"context":context,"context_complete":false,
                "context_limits":{"max_messages":3,"max_each_utf8_bytes":512,"graph_stop_reasons":links["constraints"]["stop_reasons"],"further_context_requires_assessed_read":true},
                "read_args":read}));
        }
        let coverage = json!({"event_vectors":vector_coverage,"lexical_sessions":lexical_groups.len(),"semantic_sessions":semantic_groups.len(),
            "ordering":"independent_session_dedup_then_semantic_lexical_round_robin","scores_are_confidence":false,
            "per_channel_session_limit":MAX_CHANNEL_SESSIONS,"eligible_semantic_sessions":semantic_seen.len(),"eligible_lexical_sessions":lexical_seen.len(),
            "semantic_sessions_truncated":semantic_seen.len()>MAX_CHANNEL_SESSIONS,"lexical_sessions_truncated":lexical_seen.len()>MAX_CHANNEL_SESSIONS,
            "selection":"caller_assessment_required_not_necessarily_human","assessment_semantics":"binding verifies current original evidence, not semantic relevance or receipt issuance",
            "event_filter":args.event_filter,"as_of":args.as_of,"scope_filtered":true,"max_preview_messages":3,
            "continuation":"cursor lists more candidate sessions; it does not expand a graph or continue a text tail"});
        let envelope = |results: Vec<Value>| {
            let next = offset + results.len();
            let mut coverage = coverage.clone();
            coverage["unverifiable_previews_on_page"] = json!(results
                .iter()
                .filter(|r| r["confirmation_available"] == false)
                .count());
            json!({"results":results,"candidate_count":ordered.len(),
            "status":if ordered.is_empty(){"no_matches"}else if next<ordered.len(){"partial"}else{"complete"},"status_scope":"discovery_candidates",
            "relevance":"unassessed","selection_required":!ordered.is_empty(),"budget_unit":"utf8_bytes","coverage":coverage,
            "next_cursor":(next<ordered.len()&&next>offset).then(||format!("sd1:{binding}:{next}")),"truncated":next<ordered.len()})
        };
        let shrink = |original: &Value, bytes: usize| -> Result<Value> {
            let mut item = original.clone();
            let mut read: ReadPageArgs =
                serde_json::from_value(item["read_args"].clone()).map_err(|e| e.to_string())?;
            let proof = read.context_confirmation.as_mut().unwrap();
            for (part, span) in item["context"]
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .zip(proof.binding.evidence.iter_mut())
            {
                let text = &docs[span.reference.as_str()].text;
                let (start, end) =
                    if part["relation"] == "anchor" && original["first_channel"] == "lexical" {
                        matching_window(text, &args.query, bytes)
                    } else {
                        let start = span.start_byte;
                        let mut end = (start + bytes).min(span.end_byte);
                        while !text.is_char_boundary(end) {
                            end -= 1;
                        }
                        (start, end)
                    };
                part["text"] = json!(&text[start..end]);
                part["text_range"]["start_byte"] = json!(start);
                part["text_range"]["end_byte"] = json!(end);
                part["text_truncated"] = json!(start > 0 || end < text.len());
                if part["relation"] == "anchor" {
                    read.offset_bytes = Some(start);
                }
                span.start_byte = start;
                span.end_byte = end;
                span.text_hash = hash(&text.as_bytes()[start..end]);
            }
            item["context_limits"]["max_each_utf8_bytes"] = json!(bytes);
            proof.binding_hash = hash(json!(proof.binding).to_string().as_bytes());
            proof.assessment = CallerAssessment {
                decision: "uncertain".into(),
                selected_anchor_ref: None,
                evidence_indices: vec![],
            };
            item["read_args"] = json!(read);
            Ok(item)
        };
        if drafts.is_empty() {
            let page = envelope(vec![]);
            if json_size(&page)? <= args.budget_tokens {
                return Ok(page);
            }
        }
        let mut minimum = 512;
        for count in (1..=drafts.len()).rev() {
            for bytes in [512, 256, 128, 64] {
                let page = envelope(
                    drafts[..count]
                        .iter()
                        .map(|x| shrink(x, bytes))
                        .collect::<Result<Vec<_>>>()?,
                );
                let size = json_size(&page)?;
                minimum = size;
                if size <= args.budget_tokens {
                    return Ok(page);
                }
            }
        }
        let retry_budget = minimum.max(512);
        let result = json!({"results":[],"status":if ordered.is_empty(){"no_matches"}else if offset==ordered.len(){"complete"}else{"budget_exhausted"},
            "candidate_count":ordered.len(),"relevance":"unassessed","next_cursor":null,"budget_unit":"utf8_bytes",
            "recommended_min_budget":retry_budget,"retry_cursor":(offset<ordered.len()).then(||format!("sd1:{}:{offset}",make_binding(retry_budget))),
            "event_vector_status":coverage["event_vectors"]["status"],"event_vector_error":coverage["event_vectors"]["error"],
            "hint":"Use recommended_min_budget with retry_cursor; keep other search arguments."});
        if json_size(&result)? > args.budget_tokens {
            return Err("预算不足以输出发现状态".into());
        }
        Ok(result)
    }
}
