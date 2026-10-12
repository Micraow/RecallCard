//! Explicit, bounded source-graph expansion. No time-based relationship inference.
use super::*;
use std::collections::VecDeque;

const MAX_ANCHORS: usize = 2;
const MAX_HOPS: usize = 3;
const MAX_MESSAGES: usize = 12;
const TEXT_SLICE_BYTES: usize = 1024;

pub(super) fn graph_key(event: &Event) -> String {
    json!([
        event.data.scope,
        event.data.session_key(),
        event.data.source.platform,
        event.data.source.account_namespace,
        event.data.source.conversation_id
    ])
    .to_string()
}

struct Group<'a> {
    anchors: Vec<&'a str>,
    anchor_count: usize,
    score: f64,
    nodes: BTreeMap<&'a str, &'a Event>,
}
struct Message<'a> {
    document: &'a Document,
    snapshot: String,
    group: usize,
    anchor: &'a str,
    relation: &'static str,
    distance: usize,
    hit_range: Option<(usize, usize)>,
}

impl Context<'_> {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn session_context_response(
        &self,
        ranked: &[search::Match<'_>],
        current: &[Document],
        filtered: &[Document],
        args: &SearchArgs,
        search_binding: &str,
        guard: &crate::vault::WriteGuard,
        mut event_vector_coverage: Value,
        discovery_sources: Option<&BTreeMap<String, Value>>,
    ) -> Result<Value> {
        let began = std::time::Instant::now();
        let mut snapshot = crate::vault::EventSnapshot::new(self.vault, guard);
        let events = snapshot.all()?;
        let visible: BTreeSet<_> = current
            .iter()
            .filter(|doc| doc.kind == "event")
            .map(|doc| doc.reference.as_str())
            .collect();
        let allowed: BTreeMap<_, _> = filtered
            .iter()
            .map(|doc| (doc.reference.as_str(), doc))
            .collect();
        let mut groups: Vec<Group<'_>> = Vec::new();
        let mut indices = BTreeMap::new();
        for hit in ranked {
            let reference = hit.document.reference.as_str();
            let event = &events[reference.trim_start_matches("event:")];
            let key = graph_key(event);
            let index = *indices.entry(key).or_insert_with(|| {
                groups.push(Group {
                    anchors: vec![],
                    anchor_count: 0,
                    score: hit.score,
                    nodes: BTreeMap::new(),
                });
                groups.len() - 1
            });
            let group = &mut groups[index];
            group.anchor_count += 1;
            if group.anchors.len() < MAX_ANCHORS {
                group.anchors.push(reference);
            }
        }
        // One scan for all candidate sessions; never traverse hidden/revised/foreign records.
        for event in events.values() {
            if visible.contains(format!("event:{}", event.id).as_str()) {
                if let Some(index) = indices.get(&graph_key(event)) {
                    groups[*index].nodes.insert(event.id.as_str(), event);
                }
            }
        }
        // Bind graph metadata as well as ranking, filters, scope, mode and byte budget.
        // Parent/path changes can alter traversal even when projected text is unchanged.
        let graph_generation = hash(
            &serde_json::to_vec(&groups.iter().map(|g| &g.nodes).collect::<Vec<_>>())
                .map_err(|e| e.to_string())?,
        );
        let binding = hash(
            json!([
                search_binding,
                graph_generation,
                "session_context_v1",
                args.budget_tokens,
                args.limit
            ])
            .to_string()
            .as_bytes(),
        );
        let (mut position, mut start) = cursor_position(args.cursor.as_deref(), &binding)?;
        let mut messages = Vec::new();
        let mut summaries = Vec::new();
        for (index, group) in groups.iter().enumerate() {
            let mut reasons = BTreeSet::new();
            if group.anchor_count > MAX_ANCHORS {
                reasons.insert("anchor_limit");
            }
            let mut source_ids: BTreeMap<&str, Option<&str>> = BTreeMap::new();
            for event in group.nodes.values() {
                source_ids
                    .entry(adjacency::source_node_id(event))
                    .and_modify(|id| *id = None)
                    .or_insert(Some(&event.id));
            }
            let mut parents = BTreeMap::new();
            let mut children: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
            for (id, event) in &group.nodes {
                let parent = if let Some(reply) = event.data.reply_to.as_deref() {
                    group.nodes.contains_key(reply).then_some(reply)
                } else {
                    adjacency::source_parent(event)
                        .and_then(|source| source_ids.get(source).copied().flatten())
                };
                parents.insert(*id, parent);
                if let Some(parent) = parent {
                    children.entry(parent).or_default().push(*id);
                }
            }
            let cyclic = group.anchors.iter().any(|anchor| {
                let mut path = BTreeSet::new();
                let mut at = Some(anchor.trim_start_matches("event:"));
                while let Some(id) = at {
                    if !path.insert(id) {
                        return true;
                    }
                    at = parents[id];
                }
                false
            });
            if cyclic {
                if discovery_sources.is_none() {
                    return Err("来源关系包含循环，请核对导出后重新导入".into());
                }
                // Discovery sources are independent. A broken graph cannot erase
                // healthy sources or invalidate an independently retrieved original.
                reasons.insert("source_graph_cycle");
            }
            let mut queue = VecDeque::new();
            for anchor in &group.anchors {
                queue.push_back((
                    anchor.trim_start_matches("event:"),
                    *anchor,
                    "anchor",
                    0usize,
                ));
            }
            let mut seen = BTreeSet::new();
            let before = messages.len();
            while let Some((id, anchor, relation, distance)) = queue.pop_front() {
                if !seen.insert(id) {
                    continue;
                }
                let reference = format!("event:{id}");
                let Some(document) = allowed.get(reference.as_str()) else {
                    reasons.insert("request_filter_boundary");
                    continue;
                };
                if messages.len() - before == MAX_MESSAGES {
                    reasons.insert("message_limit");
                    break;
                }
                let event = group.nodes[id];
                messages.push(Message {
                    document,
                    snapshot: hash(json!(event).to_string().as_bytes()),
                    group: index,
                    anchor,
                    relation,
                    distance,
                    hit_range: ranked
                        .iter()
                        .find(|entry| entry.document.reference == reference)
                        .and_then(|entry| entry.hit_range),
                });
                if cyclic {
                    continue;
                }
                if source_ids[adjacency::source_node_id(event)].is_none() {
                    reasons.insert("ambiguous_source_identity");
                    continue;
                }
                if adjacency::graph(event).is_none()
                    && event.data.reply_to.is_none()
                    && event.data.metadata.get("previous_message_id").is_none()
                {
                    reasons.insert("source_graph_unavailable");
                }
                if adjacency::graph(event).is_some_and(|graph| {
                    graph["omitted_parent_nodes"]
                        .as_u64()
                        .is_some_and(|count| count > 0)
                }) {
                    reasons.insert("source_export_gap");
                }
                let previous = parents[id];
                if previous.is_none()
                    && (event.data.reply_to.is_some() || adjacency::source_parent(event).is_some())
                {
                    reasons.insert("unresolved_parent_boundary");
                }
                let next = children.get(id).cloned().unwrap_or_default();
                let selected: Vec<_> = next
                    .iter()
                    .copied()
                    .filter(|child| {
                        adjacency::current(event) != Some(true)
                            || adjacency::current(group.nodes[child]) == Some(true)
                    })
                    .collect();
                if selected.len() > 1 || selected.is_empty() && !next.is_empty() {
                    reasons.insert("branch_choice_required");
                }
                // An off-path anchor must never expand backwards and then into its sibling.
                // Each direction continues monotonically away from its own anchor.
                let previous = previous.filter(|_| relation != "next");
                let next = (selected.len() == 1 && relation != "previous").then(|| selected[0]);
                if distance == MAX_HOPS {
                    if previous.is_some() || next.is_some() {
                        reasons.insert("hop_limit");
                    }
                    continue;
                }
                if let Some(previous) = previous {
                    queue.push_back((previous, anchor, "previous", distance + 1));
                }
                if let Some(next) = next {
                    queue.push_back((next, anchor, "next", distance + 1));
                }
            }
            let mut frontiers = BTreeMap::new();
            if reasons.contains("hop_limit") || reasons.contains("message_limit") {
                for message in &messages[before..] {
                    let entry = frontiers
                        .entry((message.anchor, message.relation))
                        .or_insert(message);
                    if message.distance > entry.distance {
                        *entry = message;
                    }
                }
            }
            let continuation_candidates: Vec<_> = frontiers
                .values()
                .filter(|message| message.relation != "anchor")
                .map(|message| {
                    json!({"ref":message.document.reference,
                    "context_anchor_ref":message.anchor,"direction_from_anchor":message.relation})
                })
                .collect();
            let anchor = allowed[group.anchors[0]];
            let mut summary = json!({
                "group":index,"session_ref":anchor.session_ref,"scope":anchor.scope,
                "anchor_refs":group.anchors,"matching_anchor_count":group.anchor_count,
                "score":group.score,"selected_message_count":messages.len()-before,
                "window_limited":!reasons.is_empty(),"limit_reasons":reasons,
                "continuation_candidates":continuation_candidates
            });
            if let Some(sources) = discovery_sources {
                let event = &events[anchor.reference.trim_start_matches("event:")];
                let source = sources
                    .get(&graph_key(event))
                    .ok_or("discovery source identity changed")?;
                let object = summary.as_object_mut().unwrap();
                object.remove("score");
                object.extend(
                    source
                        .as_object()
                        .ok_or("invalid discovery source")?
                        .clone(),
                );
            }
            summaries.push(summary);
        }
        // First give every selected message one real excerpt, then round-robin its tails.
        // A giant assistant answer must not consume the budget before a short user correction.
        let mut by_group = Vec::new();
        for group in 0..groups.len() {
            let mut pieces = VecDeque::new();
            let mut remaining: Vec<_> = messages
                .iter()
                .enumerate()
                .filter(|(_, message)| message.group == group)
                .map(|(index, message)| {
                    (
                        index,
                        text_pieces(
                            &message.document.text,
                            &args.query,
                            message.relation == "anchor",
                            message.hit_range,
                        ),
                    )
                })
                .collect();
            loop {
                let before = pieces.len();
                for (index, ranges) in &mut remaining {
                    if let Some((from, until)) = ranges.pop_front() {
                        pieces.push_back((*index, from, until));
                    }
                }
                if before == pieces.len() {
                    break;
                }
            }
            by_group.push(pieces);
        }
        let mut pieces = Vec::new();
        if discovery_sources.is_some() {
            // Keep the existing candidate breadth; within each page-sized source
            // batch, give every source its next fair message slice before tails.
            for batch in by_group.chunks_mut(args.limit) {
                loop {
                    let before = pieces.len();
                    for group in batch.iter_mut() {
                        if let Some(piece) = group.pop_front() {
                            pieces.push(piece);
                        }
                    }
                    if before == pieces.len() {
                        break;
                    }
                }
            }
        } else {
            for group in by_group {
                pieces.extend(group);
            }
        }
        if position > pieces.len()
            || position == pieces.len() && start != 0
            || pieces.get(position).is_some_and(|(index, from, until)| {
                start > until - from
                    || !messages[*index]
                        .document
                        .text
                        .is_char_boundary(from + start)
            })
        {
            return Err("会话上下文游标偏移无效".into());
        }
        if event_vector_coverage["status"] == "available" {
            event_vector_coverage["timing_ms"]["graph_window"] = json!(began.elapsed().as_millis());
        }
        let mut coverage = json!({
            "source_grouping":"same_scope_session_and_official_source_graph",
            "ordering":"ranked_sessions_then_anchors_then_bidirectional_distance",
            "scope_filtered":true,"event_filter":args.event_filter,"as_of":args.as_of,
            "context_window":{"max_anchors":MAX_ANCHORS,"max_hops_each_direction":MAX_HOPS,"max_messages_per_session":MAX_MESSAGES},
            "completeness":"bounded_visible_source_graph_only",
            "visibility_note":"隐藏、修订或未授权节点不披露；没有返回邻居不证明没有其他消息。next_cursor只分页已选窗口，不能扩大它。",
            "continuation":{"tool":"read","include_adjacent":true,"detail":"compact",
                "event_filter":args.event_filter,"as_of":args.as_of,
                "refs":"copy a returned boundary result.ref into a one-element array",
                "context_anchor_ref":"copy that result.anchor_ref; keep it unchanged for every further read",
                "budget_bytes":"set an explicit per-call UTF-8 byte budget; count all calls and response bytes",
                "navigation":"follow returned previous_ref/next_refs monotonically; stop at branch/filter boundaries",
                "cursor_scope":"selected_window_text_only"},
            "event_vectors":event_vector_coverage,
            "selected_message_count":messages.len(),"text_order":"one_excerpt_per_message_then_round_robin_remaining_ranges"
        });
        if discovery_sources.is_some() {
            coverage["ordering"] =
                json!("independent_session_dedup_then_semantic_lexical_round_robin");
            coverage["text_order"] =
                json!("source_round_robin_within_limit_then_fair_message_slices");
            coverage["anchor_policy"] =
                json!("first_channel_then_distinct_other_channel_else_next_same_channel_event");
            coverage["relevance"] = json!("unassessed");
            coverage["selection"] = json!("bounded_evidence_without_single_source_commitment");
            coverage["score_units"] = session_discovery::score_units();
        }
        let mut items = Vec::new();
        let mut page_groups = BTreeSet::new();
        let mut required = 512;
        while let Some((message_index, from, until)) = pieces.get(position) {
            let message = &messages[*message_index];
            let absolute_start = from + start;
            if !page_groups.contains(&message.group) && page_groups.len() == args.limit {
                break;
            }
            let mut end = *until;
            while !message.document.text.is_char_boundary(end) {
                end -= 1;
            }
            loop {
                let doc = message.document;
                let item = json!({"ref":doc.reference,"group":message.group,"role":doc.role,
                    "occurred_at":doc.occurred_at,"on_current_path":doc.on_current_path,
                    "anchor_ref":message.anchor,"relation":message.relation,"distance":message.distance,
                    "text":&doc.text[absolute_start..end],"text_range":{"start_byte":absolute_start,"end_byte":end,"total_bytes":doc.text.len(),"projection":"event.text"},
                    "text_truncated":absolute_start!=0||end!=doc.text.len(),"snapshot":message.snapshot});
                let next_position = position + usize::from(end == *until);
                let next_start = if end == *until { 0 } else { end - from };
                let mut candidate_items = items.clone();
                candidate_items.push(item);
                let mut candidate_groups = page_groups.clone();
                candidate_groups.insert(message.group);
                let candidate = envelope(
                    candidate_items.clone(),
                    &summaries,
                    &candidate_groups,
                    &coverage,
                    pieces.len(),
                    (next_position, next_start),
                    &binding,
                );
                let size = json_size(&candidate)?;
                required = size;
                if size <= args.budget_tokens
                    && (end > absolute_start || absolute_start == doc.text.len())
                {
                    items = candidate_items;
                    page_groups = candidate_groups;
                    position = next_position;
                    start = next_start;
                    break;
                }
                if end == absolute_start || !items.is_empty() {
                    return page_or_empty(
                        items,
                        &summaries,
                        &page_groups,
                        &coverage,
                        pieces.len(),
                        position,
                        start,
                        &binding,
                        args.budget_tokens,
                        required,
                    );
                }
                end = end
                    .saturating_sub(size.saturating_sub(args.budget_tokens).max(1))
                    .max(absolute_start);
                while !doc.text.is_char_boundary(end) {
                    end -= 1;
                }
            }
        }
        page_or_empty(
            items,
            &summaries,
            &page_groups,
            &coverage,
            pieces.len(),
            position,
            start,
            &binding,
            args.budget_tokens,
            required,
        )
    }
}

fn text_pieces(
    text: &str,
    query: &str,
    anchor: bool,
    hit_range: Option<(usize, usize)>,
) -> VecDeque<(usize, usize)> {
    let (from, until) = if let Some((start, end)) = hit_range.filter(|_| anchor) {
        let mut until = (start + TEXT_SLICE_BYTES).min(end);
        while !text.is_char_boundary(until) {
            until -= 1;
        }
        (start, until)
    } else {
        matching_window(text, if anchor { query } else { "" }, TEXT_SLICE_BYTES)
    };
    let mut pieces = VecDeque::from([(from, until)]);
    for (mut start, stop) in [(0, from), (until, text.len())] {
        while start < stop {
            let mut end = (start + TEXT_SLICE_BYTES).min(stop);
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            pieces.push_back((start, end));
            start = end;
        }
    }
    pieces
}

fn cursor_position(cursor: Option<&str>, binding: &str) -> Result<(usize, usize)> {
    let Some(cursor) = cursor else {
        return Ok((0, 0));
    };
    let parts: Vec<_> = cursor.split(':').collect();
    if cursor.len() > 160 || parts.len() != 4 || parts[0] != "sc1" || parts[1] != binding {
        return Err(
            "会话上下文游标已失效；保持相同mode、预算、limit、查询和过滤条件，或重新搜索".into(),
        );
    }
    Ok((
        parts[2].parse().map_err(|_| "上下文消息偏移无效")?,
        parts[3].parse().map_err(|_| "上下文字节偏移无效")?,
    ))
}

fn envelope(
    items: Vec<Value>,
    summaries: &[Value],
    selected: &BTreeSet<usize>,
    coverage: &Value,
    total: usize,
    cursor: (usize, usize),
    binding: &str,
) -> Value {
    let (position, start) = cursor;
    let partial = position < total;
    json!({"results":items,"sessions":selected.iter().map(|index| &summaries[*index]).collect::<Vec<_>>(),
        "coverage":coverage,"status_scope":"selected_context_window","status":if total==0{"no_matches"}else if partial{"partial"}else{"complete"},
        "selected_slice_count":total,"session_count":summaries.len(),"truncated":partial,
        "next_cursor":partial.then(||format!("sc1:{binding}:{position}:{start}")),"budget_unit":"utf8_bytes"})
}

#[allow(clippy::too_many_arguments)]
fn page_or_empty(
    items: Vec<Value>,
    summaries: &[Value],
    selected: &BTreeSet<usize>,
    coverage: &Value,
    total: usize,
    position: usize,
    start: usize,
    binding: &str,
    budget: usize,
    required: usize,
) -> Result<Value> {
    if total == 0 {
        return Ok(
            json!({"results":[],"sessions":[],"status":"no_matches","truncated":false,"next_cursor":null,"budget_unit":"utf8_bytes","session_count":0,"selected_slice_count":0,"coverage":{"event_vectors":{"status":coverage["event_vectors"]["status"],"error":coverage["event_vectors"]["error"],"ranking_policy":coverage["event_vectors"]["ranking_policy"],"query_encoding":"existing_cache_only","network_requests":0}}}),
        );
    }
    if items.is_empty() && position < total {
        let empty = json!({"results":[],"sessions":[],"status":"budget_exhausted","budget_exhausted":true,
            "truncated":true,"next_cursor":null,"budget_unit":"utf8_bytes","recommended_min_budget":required+32,
            "max_budget_bytes":32768,"hint":"预算不足不是无匹配；增加budget_bytes后不带cursor重新开始。"});
        if json_size(&empty)? <= budget {
            return Ok(empty);
        }
        return Err("预算不足以输出上下文状态".into());
    }
    let result = envelope(
        items,
        summaries,
        selected,
        coverage,
        total,
        (position, start),
        binding,
    );
    if json_size(&result)? > budget {
        return Err("预算不足以输出上下文状态".into());
    }
    Ok(result)
}
