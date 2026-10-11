//! 显式请求的有界来源导航；不按时间猜关系，不合并并行回答。
use super::*;

pub(super) fn graph(event: &Event) -> Option<&Value> {
    ["qwen", "deepseek", "chatgpt"].into_iter().find_map(|key| {
        event
            .data
            .metadata
            .get(key)
            .filter(|value| value.is_object())
    })
}
pub(super) fn source_node_id(event: &Event) -> &str {
    graph(event)
        .and_then(|graph| graph.get("node_id"))
        .and_then(Value::as_str)
        .unwrap_or(&event.data.source.message_id)
}
pub(super) fn source_parent(event: &Event) -> Option<&str> {
    if let Some(graph) = graph(event) {
        // 显式 null 是已知无可见父节点，不能退回另一条猜测关系。
        if let Some(parent) = graph.get("nearest_visible_parent_id") {
            return parent.as_str();
        }
        if graph["parent_is_structural_root"] == true {
            return None;
        }
        if let Some(parent) = graph.get("parent_id") {
            return parent.as_str();
        }
    }
    event
        .data
        .metadata
        .get("previous_message_id")
        .and_then(Value::as_str)
}
pub(super) fn current(event: &Event) -> Option<bool> {
    graph(event)
        .and_then(|value| value.get("on_current_path"))
        .and_then(Value::as_bool)
}

impl Context<'_> {
    pub(super) fn adjacent(
        &self,
        event: &Event,
        snapshot: &mut crate::vault::EventSnapshot<'_>,
        suppressed: &BTreeSet<String>,
        args: &ReadPageArgs,
    ) -> Result<Option<(Value, String)>> {
        let constrained = args.event_filter.is_some()
            || args.as_of.is_some()
            || args.context_anchor_ref.is_some();
        let graph_unavailable = graph(event).is_none()
            && event.data.reply_to.is_none()
            && event.data.metadata.get("previous_message_id").is_none();
        if graph_unavailable && !constrained {
            return Ok(None);
        }
        let all = snapshot.all()?;
        let superseded: BTreeSet<&str> = all
            .values()
            .filter_map(|candidate| {
                candidate.data.revision_of.as_deref().filter(|id| {
                    all.get(*id)
                        .is_some_and(|old| old.data.revision_key() == candidate.data.revision_key())
                })
            })
            .collect();
        let session = event.data.session_key();
        let visible: BTreeMap<&str, &Event> = all
            .values()
            .filter(|candidate| {
                candidate.data.scope == event.data.scope
                    && self.access.permits(&candidate.data.scope)
                    && candidate.data.session_key() == session
                    // session_id 可由调用方命名；它不能把不同官方来源图拼成一条边。
                    && candidate.data.source.platform == event.data.source.platform
                    && candidate.data.source.account_namespace == event.data.source.account_namespace
                    && candidate.data.source.conversation_id == event.data.source.conversation_id
                    && !suppressed.contains(&candidate.id)
                    && !superseded.contains(candidate.id.as_str())
                    && !matches!(
                        candidate.data.origin,
                        Origin::ContextInjection | Origin::RecallcardDreamJob
                    )
                    && !candidate.data.parts.iter().any(|part| {
                        matches!(
                            part.origin,
                            Origin::ContextInjection | Origin::RecallcardDreamJob
                        )
                    })
            })
            .map(|candidate| (candidate.id.as_str(), candidate))
            .collect();
        let generation = hash(&serde_json::to_vec(&visible).map_err(|error| error.to_string())?);
        if !visible.contains_key(event.id.as_str()) {
            if constrained {
                return Err("导航来源不可访问、已修订或不属于当前上下文".into());
            }
            return Ok(Some((
                json!({"historical_or_noncontext_source":true,"previous_ref":null,"next_refs":[],"next_user_ref":null}),
                generation,
            )));
        }
        let allowed = |id: &str| {
            let candidate = visible[id];
            let path = candidate
                .data
                .metadata
                .pointer("/qwen/on_current_path")
                .or_else(|| candidate.data.metadata.pointer("/chatgpt/on_current_path"))
                .and_then(Value::as_bool);
            args.event_filter.as_ref().is_none_or(|filter| {
                filter.permits_fields(Some(&candidate.data.role), candidate.data.occurred_at, path)
            }) && args.as_of.is_none_or(|time| {
                candidate
                    .data
                    .occurred_at
                    .is_none_or(|occurred| occurred <= time)
            })
        };
        if !allowed(&event.id) {
            return Err("引用不满足本次 Event 过滤或 as_of 边界".into());
        }
        // 同一会话重复导出节点编号不能猜测父节点。canonical Event id 仍由底层强制唯一。
        let mut source_ids: BTreeMap<&str, Option<&str>> = BTreeMap::new();
        for candidate in visible.values() {
            source_ids
                .entry(source_node_id(candidate))
                .and_modify(|value| *value = None)
                .or_insert(Some(&candidate.id));
        }
        let mut parents: BTreeMap<&str, Option<&str>> = BTreeMap::new();
        for (id, candidate) in &visible {
            let parent = if let Some(reply) = candidate.data.reply_to.as_deref() {
                visible.contains_key(reply).then_some(reply)
            } else {
                source_parent(candidate).and_then(|id| source_ids.get(id).copied().flatten())
            };
            parents.insert(id, parent);
        }
        let mut visited = BTreeSet::new();
        let mut at = Some(event.id.as_str());
        while let Some(id) = at {
            if !visited.insert(id) {
                return Err("来源关系包含循环，请核对导出后重新导入".into());
            }
            at = parents.get(id).copied().flatten();
        }
        let mut children_by_parent: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for (child, parent) in &parents {
            if let Some(parent) = parent {
                children_by_parent.entry(parent).or_default().push(child);
            }
        }
        let children_of =
            |id: &str| -> Vec<&str> { children_by_parent.get(id).cloned().unwrap_or_default() };
        let selected_children = |id: &str| -> Vec<&str> {
            children_of(id)
                .into_iter()
                .filter(|child| {
                    current(visible[id]) != Some(true) || current(visible[child]) == Some(true)
                })
                .collect()
        };
        // Fix the original search anchor across explicit reads. A backwards step
        // cannot be rebased into a forwards step along a sibling branch.
        let mut direction = "anchor";
        if let Some(reference) = &args.context_anchor_ref {
            let (kind, anchor, _) = parse_ref(reference)?;
            if kind != "event" || !visible.contains_key(anchor) || !allowed(anchor) {
                return Err("上下文锚点不可访问、已修订或不在同一来源/过滤范围".into());
            }
            let path_to = |from: &str, to: &str| -> Option<Vec<String>> {
                let mut path = Vec::new();
                let mut seen = BTreeSet::new();
                let mut at = Some(from);
                while let Some(id) = at {
                    if !seen.insert(id)
                        || !allowed(id)
                        || source_ids[source_node_id(visible[id])].is_none()
                    {
                        return None;
                    }
                    path.push(id.to_owned());
                    if id == to {
                        return Some(path);
                    }
                    at = parents.get(id).copied().flatten();
                }
                None
            };
            if event.id == anchor {
                if path_to(anchor, anchor).is_none() {
                    return Err("来源标识不唯一，无法进行锚点导航".into());
                }
            } else if path_to(anchor, &event.id).is_some() {
                direction = "previous";
            } else if let Some(path) = path_to(&event.id, anchor) {
                if !path.windows(2).all(|pair| {
                    let selected = selected_children(&pair[1]);
                    selected.len() == 1 && selected[0] == pair[0]
                }) {
                    return Err("上下文路径需要显式分支选择；不能自动跨分支".into());
                }
                direction = "next";
            } else {
                return Err("引用不在锚点的可验证单调路径，或路径已被隐藏/过滤/修订".into());
            }
        }
        let children = children_of(&event.id);
        // Determine branch ambiguity BEFORE filtering. A filtered fork is still a fork.
        let selected = selected_children(&event.id);
        let mut stops = BTreeSet::new();
        if graph_unavailable {
            stops.insert("source_graph_unavailable");
        }
        if graph(event).is_some_and(|g| g["omitted_parent_nodes"].as_u64().is_some_and(|n| n > 0)) {
            stops.insert("source_export_gap");
        }
        if parents[event.id.as_str()].is_none()
            && (event.data.reply_to.is_some() || source_parent(event).is_some())
        {
            stops.insert("unresolved_parent_boundary");
        }
        if selected.len() > 1 || selected.is_empty() && !children.is_empty() {
            stops.insert("branch_choice_required");
        }
        if source_ids[source_node_id(event)].is_none() {
            stops.insert("ambiguous_source_identity");
        }
        let mut previous = parents[event.id.as_str()];
        if previous.is_some_and(|id| !allowed(id)) {
            stops.insert("request_filter_boundary");
            previous = None;
        }
        let unique = |id: &str| source_ids[source_node_id(visible[id])].is_some();
        if constrained && previous.is_some_and(|id| !unique(id)) {
            stops.insert("ambiguous_source_identity");
            previous = None;
        }
        if direction == "next" || constrained && !unique(&event.id) {
            previous = None;
        }
        let mut next_refs = Vec::new();
        for id in &selected {
            if !allowed(id) {
                stops.insert("request_filter_boundary");
            } else if constrained && (!unique(id) || !unique(&event.id)) {
                stops.insert("ambiguous_source_identity");
            } else {
                next_refs.push(*id);
            }
        }
        if args.context_anchor_ref.is_some() && (direction == "previous" || selected.len() != 1) {
            next_refs.clear();
        }
        let mut next_user = None;
        let mut seen = BTreeSet::from([event.id.as_str()]);
        let mut cursor = event.id.as_str();
        let mut traversal_limited = false;
        for hop in 0..64 {
            if args.context_anchor_ref.is_some() && direction == "previous"
                || constrained && !unique(&event.id)
            {
                break;
            }
            let next = selected_children(cursor);
            if next.len() != 1 {
                if next.len() > 1 || !children_of(cursor).is_empty() {
                    stops.insert("branch_choice_required");
                }
                break;
            }
            cursor = next[0];
            if !allowed(cursor) {
                stops.insert("request_filter_boundary");
                break;
            }
            if source_ids[source_node_id(visible[cursor])].is_none() {
                stops.insert("ambiguous_source_identity");
                break;
            }
            if !seen.insert(cursor) {
                return Err("来源关系包含循环，请核对导出后重新导入".into());
            }
            if visible[cursor].data.role == Role::User {
                next_user = Some(format!("event:{cursor}"));
                break;
            }
            if hop == 63 {
                traversal_limited = true;
                stops.insert("user_traversal_limit");
            }
        }
        let choices = children
            .iter()
            .filter(|child| {
                !selected.contains(child)
                    && allowed(child)
                    && args.context_anchor_ref.is_none()
                    && (!constrained || unique(child) && unique(&event.id))
            })
            .take(3)
            .map(|id| format!("event:{id}"))
            .collect::<Vec<_>>();
        let other_count = children
            .iter()
            .filter(|child| {
                !selected.contains(child)
                    && allowed(child)
                    && args.context_anchor_ref.is_none()
                    && (!constrained || unique(child) && unique(&event.id))
            })
            .count();
        let mut links = json!({
            "previous_ref":previous.map(|id|format!("event:{id}")),
            "next_refs":next_refs.iter().take(3).map(|id|format!("event:{id}")).collect::<Vec<_>>(),
            "next_ref_count":next_refs.len(),"next_user_ref":next_user,"user_traversal_limited":traversal_limited,
            "branch_choices":choices,"other_branch_count":other_count,
            "branch_choice_required":selected.len()>1 || selected.is_empty()&&!children.is_empty(),
            "ambiguous_source_identity":source_ids[source_node_id(event)].is_none(),
            "relation":"authorized_source_graph_not_temporal_override"
        });
        if constrained {
            links["constraints"] = json!({"event_filter":args.event_filter,"as_of":args.as_of,
                "context_anchor_ref":args.context_anchor_ref,"direction_from_anchor":direction,
                "stop_reasons":stops,"may_continue":previous.is_some() || !next_refs.is_empty(),
                "cursor_scope":"current_event_text_only","navigation":"explicit_read_per_ref_keep_same_constraints"});
        }
        Ok(Some((links, generation)))
    }
}
