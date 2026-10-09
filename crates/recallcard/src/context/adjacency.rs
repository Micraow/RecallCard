//! 显式请求的有界来源导航；不按时间猜关系，不合并并行回答。
use super::*;

fn graph(event: &Event) -> Option<&Value> {
    ["qwen", "deepseek", "chatgpt"].into_iter().find_map(|key| {
        event
            .data
            .metadata
            .get(key)
            .filter(|value| value.is_object())
    })
}
fn source_parent(event: &Event) -> Option<&str> {
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
fn current(event: &Event) -> Option<bool> {
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
    ) -> Result<Option<(Value, String)>> {
        if graph(event).is_none()
            && event.data.reply_to.is_none()
            && event.data.metadata.get("previous_message_id").is_none()
        {
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
            return Ok(Some((
                json!({"historical_or_noncontext_source":true,"previous_ref":null,"next_refs":[],"next_user_ref":null}),
                generation,
            )));
        }
        // 同一会话重复导出节点编号不能猜测父节点。canonical Event id 仍由底层强制唯一。
        let mut source_ids: BTreeMap<&str, Option<&str>> = BTreeMap::new();
        for candidate in visible.values() {
            source_ids
                .entry(&candidate.data.source.message_id)
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
        let children_of = |id: &str| -> Vec<&str> {
            parents
                .iter()
                .filter_map(|(child, parent)| (*parent == Some(id)).then_some(*child))
                .collect()
        };
        let selected_children = |id: &str| -> Vec<&str> {
            children_of(id)
                .into_iter()
                .filter(|child| {
                    current(visible[id]) != Some(true) || current(visible[child]) == Some(true)
                })
                .collect()
        };
        let children = children_of(&event.id);
        let selected = selected_children(&event.id);
        let mut next_user = None;
        let mut seen = BTreeSet::from([event.id.as_str()]);
        let mut cursor = event.id.as_str();
        let mut traversal_limited = false;
        for hop in 0..64 {
            let next = selected_children(cursor);
            if next.len() != 1 {
                break;
            }
            cursor = next[0];
            if !seen.insert(cursor) {
                return Err("来源关系包含循环，请核对导出后重新导入".into());
            }
            if visible[cursor].data.role == Role::User {
                next_user = Some(format!("event:{cursor}"));
                break;
            }
            if hop == 63 {
                traversal_limited = true;
            }
        }
        let choices = children
            .iter()
            .filter(|child| !selected.contains(child))
            .take(3)
            .map(|id| format!("event:{id}"))
            .collect::<Vec<_>>();
        Ok(Some((
            json!({
                "previous_ref":parents[event.id.as_str()].map(|id|format!("event:{id}")),
                "next_refs":selected.iter().take(3).map(|id|format!("event:{id}")).collect::<Vec<_>>(),
                "next_ref_count":selected.len(),"next_user_ref":next_user,"user_traversal_limited":traversal_limited,
                "branch_choices":choices,"other_branch_count":children.len()-selected.len(),
                "branch_choice_required":selected.len()>1 || selected.is_empty()&&!children.is_empty(),
                "ambiguous_source_identity":source_ids[&event.data.source.message_id.as_str()].is_none(),
                "relation":"authorized_source_graph_not_temporal_override"
            }),
            generation,
        )))
    }
}
