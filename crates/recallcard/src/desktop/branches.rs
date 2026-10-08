//! 只用当前可读 Event 重建会话森林。导出节点编号仅用于内存索引，不暴露缺失来源。
use crate::{context::truncate_utf8, model::Result, Event};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

const MAX_BRANCHES: usize = 100;
const SUMMARY_BYTES: usize = 8000;
const INVALID_BRANCH: &str = "所选分支末端不可访问或已改变，请重新打开会话并选择末端";

pub(super) struct ConversationBranches {
    pub events: Vec<Event>,
    explicit_graph: bool,
    parents: Vec<Option<usize>>,
    child_counts: Vec<usize>,
    gaps: Vec<bool>,
    omitted: Vec<u64>,
    order: Vec<usize>,
    leaves: Vec<usize>,
    roots: usize,
}

impl ConversationBranches {
    pub fn new(events: Vec<Event>) -> Result<Self> {
        let explicit_graph = events.iter().any(|event| {
            event.data.metadata["deepseek"].is_object()
                || event.data.metadata.get("previous_message_id").is_some()
        });
        if !explicit_graph {
            // 旧资料只记录了片段集合，不把未知关系伪造成互斥分支，也不推断时间线。
            let count = events.len();
            return Ok(Self {
                events,
                explicit_graph,
                parents: vec![None; count],
                child_counts: vec![0; count],
                gaps: vec![false; count],
                omitted: vec![0; count],
                order: (0..count).collect(),
                leaves: Vec::new(),
                roots: 0,
            });
        }
        // 重复身份不猜测父节点；失去可见父节点时从此处断开，不能跨 scope 回查。
        let mut positions = BTreeMap::new();
        for (index, event) in events.iter().enumerate() {
            let id = event.data.metadata["deepseek"]["node_id"]
                .as_str()
                .unwrap_or(&event.data.source.message_id);
            positions
                .entry(id)
                .and_modify(|entry| *entry = None)
                .or_insert(Some(index));
        }
        let mut parents = vec![None; events.len()];
        let mut children = vec![Vec::new(); events.len()];
        let mut gaps = vec![false; events.len()];
        let mut omitted = vec![0; events.len()];
        for (index, event) in events.iter().enumerate() {
            let metadata = &event.data.metadata;
            let deepseek = &metadata["deepseek"];
            omitted[index] = deepseek["omitted_parent_nodes"].as_u64().unwrap_or(0);
            let parent = if let Some(parent) = deepseek.get("nearest_visible_parent_id") {
                parent.as_str()
            } else if deepseek["parent_is_structural_root"] == true {
                None
            } else {
                deepseek["parent_id"]
                    .as_str()
                    .or_else(|| metadata["previous_message_id"].as_str())
            };
            parents[index] = parent.and_then(|id| positions.get(id).copied().flatten());
            gaps[index] = omitted[index] > 0 || parent.is_some() && parents[index].is_none();
            if let Some(parent) = parents[index] {
                children[parent].push(index);
            }
        }
        let mut ready: BTreeSet<usize> = parents
            .iter()
            .enumerate()
            .filter_map(|(index, parent)| parent.is_none().then_some(index))
            .collect();
        let roots = ready.len();
        let mut order = Vec::with_capacity(events.len());
        while let Some(index) = ready.pop_first() {
            order.push(index);
            ready.extend(children[index].iter().copied());
        }
        if order.len() != events.len() {
            return Err("已保存的会话关系含有循环，请检查来源后重新导入".into());
        }
        let leaves = order
            .iter()
            .copied()
            .filter(|index| children[*index].is_empty())
            .collect();
        Ok(Self {
            events,
            explicit_graph,
            parents,
            child_counts: children.iter().map(Vec::len).collect(),
            gaps,
            omitted,
            order,
            leaves,
            roots,
        })
    }

    pub fn order(&self) -> &[usize] {
        &self.order
    }

    pub fn has_explicit_graph(&self) -> bool {
        self.explicit_graph
    }

    pub fn branch_count(&self) -> usize {
        self.leaves.len()
    }

    pub fn has_gaps(&self) -> bool {
        self.gaps.iter().any(|gap| *gap)
    }

    pub fn path_has_gaps(&self, indexes: &[usize]) -> bool {
        indexes.iter().any(|index| self.gaps[*index])
    }

    pub fn order_known(&self) -> bool {
        self.explicit_graph && self.branch_count() == 1 && !self.has_gaps()
    }

    pub fn order_kind(&self) -> &'static str {
        if !self.explicit_graph {
            "unverified"
        } else if self.branch_count() > 1 {
            "branch_forest"
        } else if self.has_gaps() {
            "unverified"
        } else {
            "single_branch"
        }
    }

    pub fn annotation(&self, index: usize) -> Value {
        json!({
            "parent_ref":self.parents[index].map(|parent| format!("event:{}", self.events[parent].id)),
            "is_branch_end":self.explicit_graph && self.child_counts[index] == 0,
            "relationship_known":self.explicit_graph,
            "child_count":self.child_counts[index],
            "omitted_parent_nodes":self.omitted[index],
            "gap_before":self.gaps[index],
            "on_current_path":self.events[index].data.metadata.pointer("/chatgpt/on_current_path")
        })
    }

    pub fn summary(&self) -> Result<Value> {
        let mut branches = Vec::new();
        let mut bytes = 0;
        for index in self.leaves.iter().take(MAX_BRANCHES) {
            let event = &self.events[*index];
            let row = json!({
                "branch_ref":format!("event:{}", event.id),
                "role":event.data.role,
                "text":truncate_utf8(&event.data.text(), 160),
                "occurred_at":event.data.occurred_at,
                "captured_at":event.captured_at
            });
            let size = serde_json::to_vec(&row).map_err(|e| e.to_string())?.len();
            if bytes + size > SUMMARY_BYTES {
                break;
            }
            bytes += size;
            branches.push(row);
        }
        Ok(json!({
            "branch_count":self.branch_count(),
            "root_count":self.roots,
            "selection_required":self.branch_count() > 1,
            "shown_branches":branches.len(),
            "branches_truncated":branches.len() < self.branch_count(),
            "branches":branches,
            "has_gaps":self.has_gaps(),
            "note":if self.explicit_graph {
                "只列出当前可访问消息的分支末端；并列分支与独立根不代表同一时间线。列表受数量和长度限制，更多末端可在消息分页中选择。"
            } else {
                "这些旧记录没有保存消息之间的关系，只能作为未知顺序的可见片段集合；不能据此推断分支或对话时间线。"
            }
        }))
    }

    /// 不按时间或保存顺序替用户选择。一次逆向遍历只包含所选末端的祖先。
    pub fn select(&self, branch_ref: Option<&str>) -> Result<Vec<usize>> {
        if !self.explicit_graph {
            return if branch_ref.is_none() {
                Ok(self.order.clone())
            } else {
                Err(INVALID_BRANCH.into())
            };
        }
        let leaf = if let Some(reference) = branch_ref {
            let id = reference.strip_prefix("event:").ok_or(INVALID_BRANCH)?;
            self.leaves
                .iter()
                .copied()
                .find(|index| self.events[*index].id == id)
                .ok_or(INVALID_BRANCH)?
        } else {
            if self.branch_count() > 1 {
                return Err("该会话有多个分支或独立片段，请明确选择一个分支末端后继续".into());
            }
            *self.leaves.first().ok_or("请先选择已保存的对话")?
        };
        let mut path = Vec::new();
        let mut current = Some(leaf);
        while let Some(index) = current {
            path.push(index);
            current = self.parents[index];
        }
        path.reverse();
        Ok(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hundred_thousand_node_chain_is_iterative_and_selects_only_its_ancestors() {
        let template: Event = serde_json::from_value(json!({
            "schema_version":1,"id":"synthetic","captured_at":"2026-10-07T00:00:00Z",
            "scope":"personal","role":"user","origin":"user_input","content":"合成压力测试",
            "source":{"platform":"deepseek","conversation_id":"synthetic-large","message_id":"node"}
        }))
        .unwrap();
        let mut events = Vec::with_capacity(100_000);
        for index in 0usize..100_000 {
            let mut event = template.clone();
            event.id = format!("synthetic-{index}");
            event.data.source.message_id = format!("node-{index}");
            event.data.metadata = json!({"previous_message_id":index.checked_sub(1).map(|parent| format!("node-{parent}"))});
            events.push(event);
        }
        events.reverse();
        let branches = ConversationBranches::new(events).unwrap();
        assert_eq!(branches.branch_count(), 1);
        assert!(branches.order_known());
        let selected = branches.select(Some("event:synthetic-99999")).unwrap();
        assert_eq!(selected.len(), 100_000);
        assert_eq!(branches.events[selected[0]].id, "synthetic-0");
        assert_eq!(
            branches.events[*selected.last().unwrap()].id,
            "synthetic-99999"
        );
    }
}
