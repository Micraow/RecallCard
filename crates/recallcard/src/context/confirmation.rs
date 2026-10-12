//! Optional caller-assessment workflow. A hash is not a relevance proof or access grant.
use super::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceSpan {
    pub reference: String,
    pub snapshot: String,
    pub projection: String,
    pub start_byte: usize,
    pub end_byte: usize,
    pub text_hash: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfirmationBinding {
    pub version: String,
    pub query_hash: String,
    pub anchor_ref: String,
    pub anchor_snapshot: String,
    pub source_group_key: String,
    pub scopes_hash: String,
    pub event_filter: Option<EventFilter>,
    pub as_of: Option<DateTime<Utc>>,
    pub graph_generation: String,
    pub evidence: Vec<EvidenceSpan>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CallerAssessment {
    pub decision: String,
    pub selected_anchor_ref: Option<String>,
    pub evidence_indices: Vec<usize>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextConfirmation {
    pub binding: ConfirmationBinding,
    pub binding_hash: String,
    pub assessment: CallerAssessment,
}
pub(super) fn snapshot(event: &Event) -> String {
    hash(json!(event).to_string().as_bytes())
}
pub(super) fn filter_value(filter: &Option<EventFilter>) -> Value {
    if filter.as_ref().is_some_and(EventFilter::active) {
        json!(filter)
    } else {
        Value::Null
    }
}
pub(super) fn uncertain(reason: &str) -> Value {
    json!({"status":"uncertain","reason":reason,"results":[],"next_cursor":null,
        "relevance":"unresolved","deep_read_performed":false,"budget_unit":"utf8_bytes",
        "hint":"Assess bounded discovery evidence, or stop/reformulate. Do not assume top1 is relevant."})
}
impl ContextConfirmation {
    pub(super) fn new(binding: ConfirmationBinding) -> Self {
        Self {
            binding_hash: hash(json!(binding).to_string().as_bytes()),
            binding,
            assessment: CallerAssessment {
                decision: "uncertain".into(),
                selected_anchor_ref: None,
                evidence_indices: vec![],
            },
        }
    }
    pub(super) fn pending(&self) -> Option<&'static str> {
        if self.assessment.decision != "relevant" {
            Some("assessment_uncertain_or_irrelevant")
        } else if self.assessment.selected_anchor_ref.as_deref()
            != Some(self.binding.anchor_ref.as_str())
        {
            Some("selection_required")
        } else if self.assessment.evidence_indices.is_empty() {
            Some("evidence_required")
        } else {
            None
        }
    }
}
impl Context<'_> {
    pub(super) fn verify_confirmation(
        &self,
        args: &ReadPageArgs,
        events: &mut crate::vault::EventSnapshot<'_>,
        suppressed: &BTreeSet<String>,
    ) -> std::result::Result<(), &'static str> {
        let Some(confirmation) = &args.context_confirmation else {
            return Ok(());
        };
        let b = &confirmation.binding;
        let digest = |s: &str| {
            s.len() == 64
                && s.bytes()
                    .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        };
        if b.version != "recallcard.context-confirmation/1"
            || !digest(&b.query_hash)
            || b.evidence.is_empty()
            || b.evidence.len() > 3
            || confirmation.assessment.evidence_indices.len() > 3
            || confirmation.binding_hash != hash(json!(b).to_string().as_bytes())
        {
            return Err("binding_invalid");
        }
        let indices: BTreeSet<_> = confirmation
            .assessment
            .evidence_indices
            .iter()
            .copied()
            .collect();
        if indices.len() != confirmation.assessment.evidence_indices.len()
            || indices.iter().any(|i| *i >= b.evidence.len())
        {
            return Err("evidence_selection_invalid");
        }
        if args.context_anchor_ref.as_deref() != Some(&b.anchor_ref)
            || !args.include_adjacent
            || filter_value(&args.event_filter) != filter_value(&b.event_filter)
            || args.as_of != b.as_of
            || b.scopes_hash != hash(json!(self.access.scopes()).to_string().as_bytes())
        {
            return Err("boundary_mismatch");
        }
        let (kind, id, revision) = parse_ref(&b.anchor_ref).map_err(|_| "binding_invalid")?;
        if kind != "event" || revision.is_some() {
            return Err("binding_invalid");
        }
        let anchor = events
            .event(id)
            .map_err(|_| "source_unavailable_or_stale")?;
        if !self.access.permits(&anchor.data.scope)
            || suppressed.contains(id)
            || snapshot(&anchor) != b.anchor_snapshot
            || hash(session_context::graph_key(&anchor).as_bytes()) != b.source_group_key
        {
            return Err("source_unavailable_or_stale");
        }
        let mut anchor_args = args.clone();
        anchor_args.refs = vec![b.anchor_ref.clone()];
        anchor_args.cursor = None;
        anchor_args.offset_bytes = None;
        let (links, generation) = self
            .adjacent(&anchor, events, suppressed, &anchor_args)
            .map_err(|_| "source_unavailable_or_stale")?
            .ok_or("source_unavailable_or_stale")?;
        if generation != b.graph_generation {
            return Err("source_unavailable_or_stale");
        }
        if links["ambiguous_source_identity"] == true {
            return Err("source_ambiguous");
        }
        // Validate the complete reviewed packet, not just the caller-selected subset.
        for span in &b.evidence {
            let (kind, id, revision) =
                parse_ref(&span.reference).map_err(|_| "evidence_invalid")?;
            if kind != "event"
                || revision.is_some()
                || span.projection != "event.text"
                || span.start_byte >= span.end_byte
                || span.end_byte - span.start_byte > 512
            {
                return Err("evidence_invalid");
            }
            let event = events
                .event(id)
                .map_err(|_| "source_unavailable_or_stale")?;
            if suppressed.contains(id)
                || !self.access.permits(&event.data.scope)
                || snapshot(&event) != span.snapshot
                || session_context::graph_key(&event) != session_context::graph_key(&anchor)
            {
                return Err("source_unavailable_or_stale");
            }
            let text = event.data.text();
            let value = text
                .get(span.start_byte..span.end_byte)
                .ok_or("evidence_invalid")?;
            if hash(value.as_bytes()) != span.text_hash {
                return Err("evidence_invalid");
            }
            let mut evidence_args = anchor_args.clone();
            evidence_args.refs = vec![span.reference.clone()];
            self.adjacent(&event, events, suppressed, &evidence_args)
                .map_err(|_| "source_unavailable_or_stale")?
                .ok_or("source_unavailable_or_stale")?;
        }
        let (_, requested_id, _) = parse_ref(&args.refs[0]).map_err(|_| "binding_invalid")?;
        let requested = events
            .event(requested_id)
            .map_err(|_| "source_unavailable_or_stale")?;
        if !self.access.permits(&requested.data.scope) || suppressed.contains(requested_id) {
            return Err("source_unavailable_or_stale");
        }
        self.adjacent(&requested, events, suppressed, args)
            .map_err(|_| "source_unavailable_or_stale")?
            .ok_or("source_unavailable_or_stale")?;
        Ok(())
    }
}
