//! 导航每次从当前已授权正本重算；旧派生文件不能授予读取权限。
use super::*;
use crate::navigation::{Index, Node, ROOT};
fn short(text: &str, bytes: usize) -> String {
    truncate_utf8(text, bytes)
}
fn overview(node: &Node) -> Value {
    json!({"ref":node.reference,"kind":"view","title":short(&node.title,128),
        "description":node.descriptions.first().map(|d|short(&d.text,160)),
        "description_memory_ref":node.descriptions.first().map(|d|&d.memory_ref),
        "description_provenance":node.descriptions.first().and_then(|d|node.contributors.get(&d.memory_ref)),
        "contributors":node.contributors.values().take(1).collect::<Vec<_>>(),
        "contributor_count":node.contributors.len(),
        "hints_are_not_certified_facts":true,
        "keywords":node.keywords.iter().take(5).map(|s|short(s,48)).collect::<Vec<_>>(),
        "aliases":node.aliases.iter().take(3).map(|s|short(s,48)).collect::<Vec<_>>(),
        "overview_only":true})
}
pub(super) fn index(docs: &[Document], scopes: &[String], now: DateTime<Utc>) -> Result<Index> {
    Index::build(docs, scopes, now)
}
impl Context<'_> {
    pub(super) fn navigation_page(&self, args: &ReadPageArgs, sources: bool) -> Result<Value> {
        if sources || args.include_adjacent || args.offset_bytes.is_some() {
            return Err(
                "目录不接受 sources、相邻事件或正文偏移；请先读取目录中的 Memory/Event 引用".into(),
            );
        }
        let _guard = self.vault.read_guard()?;
        let docs = self.memory_documents_locked()?;
        let scopes = self.access.scopes();
        let nav = index(&docs, &scopes, Utc::now())?;
        let reference = &args.refs[0];
        let node = nav
            .nodes
            .get(reference)
            .ok_or("导航入口不存在或当前授权下不可见")?;
        let binding = hash(
            &serde_json::to_vec(&(&nav.generation, reference, &scopes, &args.detail))
                .map_err(|e| e.to_string())?,
        );
        let start = if let Some(cursor) = &args.cursor {
            let parts = cursor.split(':').collect::<Vec<_>>();
            if parts.len() != 3 || parts[0] != "n1" || parts[1] != binding {
                return Err("导航游标已过期或不属于本次入口，请重新读取".into());
            }
            parts[2].parse::<usize>().map_err(|_| "无效导航游标")?
        } else {
            0
        };
        let by_ref = docs
            .iter()
            .map(|d| (d.reference.as_str(), d))
            .collect::<BTreeMap<_, _>>();
        let mut entries = Vec::new();
        for child in &node.children {
            entries.push(overview(&nav.nodes[child]));
        }
        for reference in &node.memories {
            let doc = by_ref
                .get(reference.as_str())
                .ok_or("导航成员已变化，请重新读取")?;
            entries.push(json!({"ref":reference,"kind":"memory","excerpt":short(&doc.text,384),"excerpt_truncated":doc.text.len()>384,"evidence":doc.evidence,"state":doc.state,"observed_at":doc.occurred_at,"time_note":short(&doc.time_note,96)}));
        }
        for related in &node.related {
            if let Some(related) = nav.nodes.get(related) {
                let mut item = overview(related);
                item["relation"] = json!("related");
                entries.push(item);
            }
        }
        // 完整导航提示可逐页读出；overview 不是完整目录元数据。
        for contributor in node.contributors.values() {
            entries.push(json!({"kind":"contributor","provenance":contributor}));
        }
        for description in &node.descriptions {
            entries.push(json!({"kind":"description","memory_ref":description.memory_ref,"text":description.text,"provenance":node.contributors.get(&description.memory_ref)}));
        }
        for (kind, values) in [("keyword", &node.keywords), ("alias", &node.aliases)] {
            for text in values {
                // 旧 labels/entities 可能比新 hint 字段长，切成明确的连续 UTF-8 片段。
                let mut at = 0;
                while at < text.len() {
                    let part = short(&text[at..], 384);
                    let end = at + part.len();
                    entries.push(json!({"kind":kind,"text":part,"start_byte":at,"end_byte":end,"total_bytes":text.len()}));
                    at = end;
                }
            }
        }
        for diagnostic in &node.diagnostics {
            entries.push(json!({"kind":"diagnostic","text":diagnostic}));
        }
        if start > entries.len() {
            return Err("导航游标偏移超过当前入口范围".into());
        }
        let metadata = overview(node);
        let mut response = json!({"results":[metadata],"entries":[],"status":"complete","truncated":false,"next_cursor":null,"snapshot":nav.generation,"reference_data":true,"budget_unit":"utf8_bytes","projection":"live_authorized_navigation","entry_count":entries.len(),"orphan_count":nav.orphan_refs.len(),"diagnostic_count":node.diagnostics.len()});
        if !node.diagnostics.is_empty() {
            response["diagnostics"] = json!(node
                .diagnostics
                .iter()
                .take(3)
                .map(|s| short(s, 128))
                .collect::<Vec<_>>());
        }
        let mut offset = start;
        while offset < entries.len() {
            response["entries"]
                .as_array_mut()
                .unwrap()
                .push(entries[offset].clone());
            let candidate_pending = offset + 1 < entries.len();
            response["truncated"] = json!(candidate_pending);
            response["status"] = json!(if candidate_pending {
                "partial"
            } else {
                "complete"
            });
            response["next_cursor"] = if candidate_pending {
                json!(format!("n1:{binding}:{}", offset + 1))
            } else {
                Value::Null
            };
            if json_size(&response)? > args.budget_tokens {
                response["entries"].as_array_mut().unwrap().pop();
                break;
            }
            offset += 1;
        }
        let pending = offset < entries.len();
        response["next_cursor"] = if pending {
            json!(format!("n1:{binding}:{offset}"))
        } else {
            Value::Null
        };
        response["truncated"] = json!(pending);
        response["status"] = json!(if pending { "partial" } else { "complete" });
        if (pending && offset == start) || json_size(&response)? > args.budget_tokens {
            // 不跳过放不下的条目；明确最小预算，原游标仍指向它。
            let mut required = response.clone();
            if offset < entries.len() {
                required["entries"] = json!([entries[offset].clone()]);
                let pending_after_one = offset + 1 < entries.len();
                required["truncated"] = json!(pending_after_one);
                required["status"] = json!(if pending_after_one {
                    "partial"
                } else {
                    "complete"
                });
                required["next_cursor"] = if pending_after_one {
                    json!(format!("n1:{binding}:{}", offset + 1))
                } else {
                    Value::Null
                };
            }
            return Ok(
                json!({"status":"budget_exhausted","ref":reference,"required_min_budget":json_size(&required)?,"recommended_min_budget":json_size(&required)?.max(args.budget_tokens+1),"budget_unit":"utf8_bytes","truncated":true}),
            );
        }
        Ok(response)
    }
}
/// 只补一个稳定根引用；旧 stable_text、profile 和平铺标签 ref 保持可用。
pub(super) fn add_bootstrap_root(
    response: &mut Value,
    docs: &[Document],
    scopes: &[String],
    budget: usize,
    now: DateTime<Utc>,
) -> Result<()> {
    let nav = match index(docs, scopes, now) {
        Ok(nav) => nav,
        Err(error) => {
            response["navigation"] =
                json!({"root_ref":ROOT,"status":"unavailable","diagnostic":short(&error,256)});
            if json_size(response)? > budget {
                response.as_object_mut().unwrap().remove("navigation");
            }
            return Ok(());
        }
    };
    let entries = nav.nodes[ROOT]
        .children
        .iter()
        .take(3)
        .map(|r| overview(&nav.nodes[r]))
        .collect::<Vec<_>>();
    response["navigation"] = json!({"root_ref":ROOT,"entries":entries,"active_memories":nav.active_memory_count,"orphan_count":nav.orphan_refs.len(),"more_entries":nav.nodes[ROOT].children.len()>3});
    while json_size(response)? > budget
        && !response["navigation"]["entries"]
            .as_array()
            .unwrap()
            .is_empty()
    {
        response["navigation"]["entries"]
            .as_array_mut()
            .unwrap()
            .pop();
        response["navigation"]["more_entries"] = json!(true);
    }
    if json_size(response)? > budget {
        response.as_object_mut().unwrap().remove("navigation");
    }
    Ok(())
}

pub(super) fn append_documents(
    docs: &mut Vec<Document>,
    scopes: &[String],
    time: DateTime<Utc>,
) -> Result<()> {
    let nav = index(docs, scopes, time)?;
    for node in nav.nodes.values() {
        let text = std::iter::once(node.title.clone())
            .chain(node.descriptions.iter().map(|d| d.text.clone()))
            .chain(node.keywords.iter().cloned())
            .chain(node.aliases.iter().cloned())
            .collect::<Vec<_>>()
            .join("\n");
        docs.push(Document {
            reference: node.reference.clone(),
            text,
            scope: "navigation".into(),
            navigation_scopes: node.scopes.iter().cloned().collect(),
            navigation: vec![],
            kind: "view".into(),
            role: None,
            on_current_path: None,
            evidence_refs: node
                .memories
                .iter()
                .cloned()
                .chain(node.descriptions.iter().map(|d| d.memory_ref.clone()))
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect(),
            session_ref: None,
            state: "generated".into(),
            occurred_at: None,
            valid_from: None,
            valid_to: None,
            time_note: "目录是当前授权记忆的派生入口，不表示事实发生时间".into(),
            evidence: "DerivedNavigation".into(),
            labels: node.keywords.iter().cloned().collect(),
            entities: node.aliases.iter().cloned().collect(),
            protected: false,
        });
    }
    Ok(())
}
/// 原事实候选优先获得前两个位置；View 走独立保留通道，不靠分数占满前屏。
pub(super) fn preserve_channels(ranked: Vec<search::Match<'_>>) -> Vec<search::Match<'_>> {
    let (mut views, mut facts): (std::collections::VecDeque<_>, std::collections::VecDeque<_>) =
        ranked.into_iter().partition(|m| m.document.kind == "view");
    let mut out = Vec::new();
    while !facts.is_empty() || !views.is_empty() {
        for _ in 0..2 {
            if let Some(m) = facts.pop_front() {
                out.push(m);
            }
        }
        if let Some(m) = views.pop_front() {
            out.push(m);
        }
    }
    out
}
