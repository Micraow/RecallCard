//! Compact, caller-owned assessment workflow; hashes establish consistency, not issuance.
use super::*;
use confirmation::{CallerAssessment, ConfirmationBinding, EvidenceSpan};
const VERSION: &str = "recallcard.candidate-review/1";
const RADIUS: usize = 2;
const WINDOW: usize = 2048;
const EXCERPT: usize = 512;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewTemplate {
    pub version: String,
    pub query_hash: String,
    pub scopes_hash: String,
    pub event_filter: Option<EventFilter>,
    pub as_of: Option<DateTime<Utc>>,
    pub max_graph_hops: usize,
    pub window_bytes: usize,
    pub excerpt_bytes: usize,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewEvidence {
    pub reference: String,
    pub snapshot: String,
    pub start_byte: usize,
    pub end_byte: usize,
    pub total_bytes: usize,
    pub text: String,
    pub text_truncated: bool,
    pub role: Role,
    pub occurred_at: Option<DateTime<Utc>>,
    pub relation: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewCandidate {
    pub candidate_id: String,
    pub anchor_ref: String,
    pub anchor_snapshot: String,
    pub anchor_start_byte: usize,
    pub source_group_key: String,
    pub graph_generation: String,
    pub evidence: Vec<ReviewEvidence>,
    pub packet_hash: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextReview {
    pub stage: String,
    pub template: ReviewTemplate,
    pub candidate: ReviewCandidate,
    pub assessment: CallerAssessment,
}
impl ReviewCandidate {
    fn id(&self) -> String {
        hash(
            json!([
                "recallcard.candidate-id/1",
                self.source_group_key,
                self.anchor_ref,
                self.anchor_snapshot
            ])
            .to_string()
            .as_bytes(),
        )[..32]
            .to_owned()
    }
    fn digest(&self, template: &ReviewTemplate) -> String {
        let mut candidate = self.clone();
        candidate.packet_hash.clear();
        hash(json!([template, candidate]).to_string().as_bytes())
    }
}
pub(super) fn compact_page(page: &mut Value, args: &SearchArgs, scopes_hash: &str) {
    let template = ReviewTemplate {
        version: VERSION.into(),
        query_hash: hash(args.query.as_bytes()),
        scopes_hash: scopes_hash.into(),
        event_filter: args.event_filter.clone().filter(EventFilter::active),
        as_of: args.as_of,
        max_graph_hops: RADIUS,
        window_bytes: WINDOW,
        excerpt_bytes: EXCERPT,
    };
    for item in page["results"].as_array_mut().unwrap() {
        // The legacy intermediate representation is internal only. Serialize each span once.
        let read: ReadPageArgs = serde_json::from_value(item["read_args"].clone()).unwrap();
        let b = read.context_confirmation.unwrap().binding;
        let mut candidate = ReviewCandidate {
            candidate_id: String::new(),
            anchor_ref: b.anchor_ref,
            anchor_snapshot: b.anchor_snapshot,
            anchor_start_byte: read.offset_bytes.unwrap_or(0),
            source_group_key: b.source_group_key,
            graph_generation: b.graph_generation,
            packet_hash: String::new(),
            evidence: item["context"]
                .as_array()
                .unwrap()
                .iter()
                .map(|p| ReviewEvidence {
                    reference: p["ref"].as_str().unwrap().into(),
                    snapshot: p["snapshot"].as_str().unwrap().into(),
                    start_byte: p["text_range"]["start_byte"].as_u64().unwrap() as usize,
                    end_byte: p["text_range"]["end_byte"].as_u64().unwrap() as usize,
                    total_bytes: p["text_range"]["total_bytes"].as_u64().unwrap() as usize,
                    text: p["text"].as_str().unwrap().into(),
                    text_truncated: p["text_truncated"].as_bool().unwrap(),
                    role: serde_json::from_value(p["role"].clone()).unwrap(),
                    occurred_at: serde_json::from_value(p["occurred_at"].clone()).unwrap(),
                    relation: p["relation"].as_str().unwrap().into(),
                })
                .collect(),
        };
        candidate.candidate_id = candidate.id();
        candidate.packet_hash = candidate.digest(&template);
        let stop_reasons = item["context_limits"]["graph_stop_reasons"].clone();
        let object = item.as_object_mut().unwrap();
        for key in [
            "read_args",
            "context",
            "context_limits",
            "anchor_ref",
            "score_units",
            "relevance",
        ] {
            object.remove(key);
        }
        object.insert("context_stop_reasons".into(), stop_reasons);
        object.insert("review_candidate".into(), json!(candidate));
    }
    page["score_units"] = session_discovery::score_units();
    page["review_template"] = json!(template);
    page["review_instructions"] = json!({"read_argument":"context_review","stages":["inspect","read"],
        "copy":"template=review_template; candidate=one review_candidate; assessment defaults uncertain with null selected_anchor_ref and [] evidence_indices",
        "inspection":"refs=[target]; include_adjacent=true; context_anchor_ref=original anchor; retain event_filter/as_of. Inspect stays unresolved. Copy updated_candidate for later explicit assessment.",
        "deep_read":"stage=read requires decision=relevant, selected_anchor_ref=original anchor and nonempty evidence_indices. No automatic relevance decision.",
        "workflow_budget":"Caller must cap cumulative calls/bytes across candidates; unsigned bindings do not enforce a global quota."});
    page["coverage"]["preview_min_bytes"] = json!(args.preview_min_bytes.unwrap_or(512));
    page["coverage"]["preview_priority"] = json!("preserve_excerpt_cap_before_candidate_count; short originals and UTF-8 boundaries may be smaller");
}
impl ContextReview {
    fn confirmation(&self) -> ContextConfirmation {
        let t = &self.template;
        let c = &self.candidate;
        let mut proof = ContextConfirmation::new(ConfirmationBinding {
            version: "recallcard.context-confirmation/1".into(),
            query_hash: t.query_hash.clone(),
            anchor_ref: c.anchor_ref.clone(),
            anchor_snapshot: c.anchor_snapshot.clone(),
            source_group_key: c.source_group_key.clone(),
            scopes_hash: t.scopes_hash.clone(),
            event_filter: t.event_filter.clone(),
            as_of: t.as_of,
            graph_generation: c.graph_generation.clone(),
            evidence: c
                .evidence
                .iter()
                .map(|e| EvidenceSpan {
                    reference: e.reference.clone(),
                    snapshot: e.snapshot.clone(),
                    projection: "event.text".into(),
                    start_byte: e.start_byte,
                    end_byte: e.end_byte,
                    text_hash: hash(e.text.as_bytes()),
                })
                .collect(),
        });
        proof.assessment = self.assessment.clone();
        proof
    }
}
impl Context<'_> {
    /// Build only permitted monotonic graph nodes. No timestamps, skipped intermediates or rebasing.
    fn inspection_nodes(
        &self,
        args: &ReadPageArgs,
        events: &mut crate::vault::EventSnapshot<'_>,
        suppressed: &BTreeSet<String>,
    ) -> Result<BTreeMap<String, (usize, String, Value)>> {
        let anchor = args
            .context_anchor_ref
            .as_ref()
            .ok_or("inspection requires original anchor")?;
        let mut nodes = BTreeMap::new();
        let mut queue = std::collections::VecDeque::from([(anchor.clone(), 0)]);
        while let Some((reference, distance)) = queue.pop_front() {
            if nodes.contains_key(&reference) {
                continue;
            }
            let (_, id, _) = parse_ref(&reference)?;
            let event = events.event(id)?;
            let mut a = args.clone();
            a.refs = vec![reference.clone()];
            a.cursor = None;
            a.offset_bytes = None;
            let (links, _) = self
                .adjacent(&event, events, suppressed, &a)?
                .ok_or("source graph unavailable")?;
            if links["ambiguous_source_identity"] == true {
                return Err("source identity ambiguous".into());
            }
            if distance < RADIUS {
                if let Some(previous) = links["previous_ref"].as_str() {
                    queue.push_back((previous.into(), distance + 1));
                }
                if let Some(next) = links["next_refs"]
                    .as_array()
                    .filter(|a| a.len() == 1)
                    .and_then(|a| a[0].as_str())
                {
                    queue.push_back((next.into(), distance + 1));
                }
            }
            let direction = links["constraints"]["direction_from_anchor"]
                .as_str()
                .unwrap_or("anchor")
                .to_owned();
            nodes.insert(reference, (distance, direction, links));
        }
        Ok(nodes)
    }
    pub(super) fn candidate_review(&self, mut args: ReadPageArgs) -> Result<Value> {
        if args.detail != "compact"
            || args.context_confirmation.is_some()
            || args.refs.len() != 1
            || !args.include_adjacent
            || args.cursor.is_some() && args.offset_bytes.is_some()
        {
            return Err("context_review requires one Event, include_adjacent and no conflicting confirmation/cursor/offset".into());
        }
        let review = args.context_review.take().unwrap();
        let t = &review.template;
        let c = &review.candidate;
        if !["inspect", "read"].contains(&review.stage.as_str()) {
            return Err("unknown review stage".into());
        }
        if t.version != VERSION
            || t.max_graph_hops != RADIUS
            || t.window_bytes != WINDOW
            || t.excerpt_bytes != EXCERPT
            || c.candidate_id != c.id()
            || c.packet_hash != c.digest(t)
            || c.evidence.is_empty()
            || c.evidence.len() > 3
        {
            return Ok(confirmation::uncertain("review_binding_invalid"));
        }
        if !["uncertain", "relevant", "irrelevant"].contains(&review.assessment.decision.as_str()) {
            return Err("unknown assessment decision".into());
        }
        let (kind, _, revision) = parse_ref(&args.refs[0])?;
        if kind != "event" || revision.is_some() {
            return Err("context_review only supports current Event refs".into());
        }
        args.context_confirmation = Some(review.confirmation());
        if review.stage == "read" {
            if let Some(reason) = args.context_confirmation.as_ref().unwrap().pending() {
                return Ok(confirmation::uncertain(reason));
            }
        }
        let guard = self.vault.read_guard()?;
        let mut events = crate::vault::EventSnapshot::new(self.vault, &guard);
        let suppressed = self.vault.suppressed_ids()?;
        if let Err(reason) = self.verify_confirmation(&args, &mut events, &suppressed) {
            return Ok(confirmation::uncertain(reason));
        }
        let nodes = match self.inspection_nodes(&args, &mut events, &suppressed) {
            Ok(nodes) => nodes,
            Err(_) => return Ok(confirmation::uncertain("inspection_graph_unavailable")),
        };
        for e in &c.evidence {
            let (_, id, _) = parse_ref(&e.reference)?;
            let event = events.event(id)?;
            let text = event.data.text();
            if e.text_truncated != (e.start_byte > 0 || e.end_byte < text.len())
                || text.len() != e.total_bytes
                || text.get(e.start_byte..e.end_byte) != Some(e.text.as_str())
                || event.data.role != e.role
                || event.data.occurred_at != e.occurred_at
                || nodes
                    .get(&e.reference)
                    .is_none_or(|(_, relation, _)| relation != &e.relation)
            {
                return Ok(confirmation::uncertain("review_evidence_invalid"));
            }
        }
        let (_, anchor_id, _) = parse_ref(&c.anchor_ref)?;
        let anchor = events.event(anchor_id)?;
        if !anchor.data.text().is_char_boundary(c.anchor_start_byte) {
            return Ok(confirmation::uncertain("review_anchor_offset_invalid"));
        }
        if review.stage == "read" {
            // Revalidate under the normal reader's snapshot. No synthetic relevant assessment.
            drop(events);
            drop(guard);
            return self.read_page(args);
        }
        let Some((distance, relation, links)) = nodes.get(&args.refs[0]) else {
            return Ok(confirmation::uncertain("inspection_radius_exceeded"));
        };
        let (_, id, _) = parse_ref(&args.refs[0])?;
        let event = events.event(id)?;
        let text = event.data.text();
        let start = if args.refs[0] == c.anchor_ref {
            c.anchor_start_byte
        } else {
            0
        };
        let mut window_end = start.saturating_add(WINDOW).min(text.len());
        while !text.is_char_boundary(window_end) {
            window_end -= 1;
        }
        let binding = hash(
            json!([
                "recallcard.inspection-cursor/1",
                t,
                c,
                args.refs[0],
                confirmation::snapshot(&event),
                args.budget_tokens,
                args.detail,
                start,
                window_end
            ])
            .to_string()
            .as_bytes(),
        );
        let offset = if let Some(cursor) = &args.cursor {
            let parts: Vec<_> = cursor.split(':').collect();
            if cursor.len() > 160 || parts.len() != 3 || parts[0] != "ri1" || parts[1] != binding {
                return Err("inspection cursor expired or boundary changed".into());
            }
            parts[2]
                .parse::<usize>()
                .map_err(|_| "invalid inspection cursor offset")?
        } else {
            args.offset_bytes.unwrap_or(start)
        };
        if offset < start
            || offset > window_end
            || (offset == window_end && start < window_end)
            || !text.is_char_boundary(offset)
        {
            return Err("inspection offset outside finite window or UTF-8 boundary".into());
        }
        let mut end = offset.saturating_add(EXCERPT).min(window_end);
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        let mut updated = c.clone();
        if end > offset {
            updated.evidence = vec![ReviewEvidence {
                reference: args.refs[0].clone(),
                snapshot: confirmation::snapshot(&event),
                start_byte: offset,
                end_byte: end,
                total_bytes: text.len(),
                text: text[offset..end].into(),
                text_truncated: offset > 0 || end < text.len(),
                role: event.data.role.clone(),
                occurred_at: event.data.occurred_at,
                relation: relation.clone(),
            }];
            updated.packet_hash = updated.digest(t);
        }
        let permitted = |v: &Value| {
            v.as_str()
                .filter(|s| nodes.contains_key(*s))
                .map(str::to_owned)
        };
        let previous = permitted(&links["previous_ref"]);
        let next = links["next_refs"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(permitted)
            .collect::<Vec<_>>();
        let result = json!({"status":if end<window_end{"partial"}else{"complete"},"status_scope":"bounded_inspection_window",
            "results":[],"updated_candidate":(end>offset).then_some(updated),"relevance":"unresolved","deep_read_performed":false,"budget_unit":"utf8_bytes",
            "text_bytes":end-offset,"context_complete":false,"navigation":{"previous_ref":previous,"next_refs":next,"stop_reasons":links["constraints"]["stop_reasons"],"distance_from_anchor":distance,"max_graph_hops":RADIUS},
            "window":{"start_byte":start,"end_byte":window_end,"more_original_text_outside_window":start>0||window_end<text.len(),"max_excerpt_bytes":EXCERPT},
            "next_cursor":(end<window_end&&end>offset).then(||format!("ri1:{binding}:{end}")),
            "cursor_contract":"Continue with the same original context_review; updated_candidate is for a new review/read, not this cursor.",
            "workflow_budget":"Caller accounts for cumulative calls and bytes; inspection does not establish relevance."});
        let size = json_size(&result)?;
        if size > args.budget_tokens {
            return Ok(
                json!({"status":"budget_exhausted","results":[],"next_cursor":null,"relevance":"unresolved","deep_read_performed":false,
                "budget_unit":"utf8_bytes","recommended_min_budget":size,"hint":"Retry the same finite inspection without advancing; changed budget requires offset_bytes, not the old cursor."}),
            );
        }
        Ok(result)
    }
}
