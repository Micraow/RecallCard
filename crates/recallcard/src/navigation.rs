//! 可重建的多入口导航。这里只处理数据，不执行 hints 中的任何文本。
use crate::{
    context::Document,
    model::{hash, Result},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const ROOT: &str = "view:nav/_root";
pub const PREFIX: &str = "view:nav/";
const MAX_DERIVED_TEXT_BYTES: usize = 16 * 1024 * 1024;
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Hint {
    pub path: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub keywords: Vec<String>,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub related_paths: Vec<String>,
}
pub fn valid_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 256
        && path.split('/').count() <= 6
        && path.split('/').all(|part| {
            !part.is_empty()
                && part.len() <= 48
                && part.as_bytes()[0].is_ascii_alphanumeric()
                && part.bytes().all(|b| {
                    b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'-' | b'_')
                })
        })
}
pub fn valid_reference(reference: &str) -> bool {
    reference
        .strip_prefix(PREFIX)
        .is_some_and(|p| matches!(p, "_root" | "_unfiled") || valid_path(p))
}
pub fn validate(hints: &[Hint]) -> Result<()> {
    if hints.len() > 8 || serde_json::to_vec(hints).map_err(|e| e.to_string())?.len() > 8192 {
        return Err("导航 hints 最多 8 个入口、整体最多 8 KiB".into());
    }
    let mut paths = BTreeSet::new();
    for h in hints {
        if !valid_path(&h.path)
            || h.path.split('/').next() == Some("labels")
            || !paths.insert(&h.path)
        {
            return Err(
                "导航路径须为唯一的小写 ASCII 分段，最多 6 层；不接受保留名称或路径转义".into(),
            );
        }
        if h.title.len() > 128
            || h.description.len() > 512
            || h.keywords.len() > 16
            || h.aliases.len() > 8
            || h.related_paths.len() > 8
            || h.keywords
                .iter()
                .chain(&h.aliases)
                .any(|s| s.trim().is_empty() || s.len() > 96 || s.chars().any(char::is_control))
            || h.related_paths.iter().any(|p| !valid_path(p))
            || h.title.chars().any(char::is_control)
            || h.description.contains('\0')
        {
            return Err("导航简介、别名、关键词或关联超出有界合同".into());
        }
    }
    Ok(())
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Description {
    pub memory_ref: String,
    pub text: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Node {
    pub reference: String,
    pub title: String,
    pub descriptions: Vec<Description>,
    pub keywords: BTreeSet<String>,
    pub aliases: BTreeSet<String>,
    pub children: BTreeSet<String>,
    pub related: BTreeSet<String>,
    pub memories: BTreeSet<String>,
    pub scopes: BTreeSet<String>,
    pub diagnostics: BTreeSet<String>,
}
impl Node {
    fn new(reference: String, title: String) -> Self {
        Self {
            reference,
            title,
            descriptions: vec![],
            keywords: BTreeSet::new(),
            aliases: BTreeSet::new(),
            children: BTreeSet::new(),
            related: BTreeSet::new(),
            memories: BTreeSet::new(),
            scopes: BTreeSet::new(),
            diagnostics: BTreeSet::new(),
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Index {
    pub schema: String,
    pub generation: String,
    pub nodes: BTreeMap<String, Node>,
    pub active_memory_count: usize,
    pub orphan_refs: Vec<String>,
}
impl Index {
    /// docs 必须是当前请求已通过 scope、来源抑制等校验的正本投影。
    pub fn build(docs: &[Document], scopes: &[String], now: DateTime<Utc>) -> Result<Self> {
        let mut current: Vec<_> = docs.iter().filter(|d| d.current_memory_at(now)).collect();
        current.sort_by(|a, b| a.reference.cmp(&b.reference));
        let mut nodes = BTreeMap::from([(ROOT.into(), Node::new(ROOT.into(), "记忆目录".into()))]);
        let mut memberships = 0usize;
        let mut copied_text_bytes = 0usize;
        let mut named_nodes = BTreeSet::new();
        for doc in &current {
            validate(&doc.navigation)?;
            let fallback;
            let hints = if doc.navigation.is_empty() {
                fallback = if doc.labels.iter().any(|l| l != "bootstrap") {
                    doc.labels
                        .iter()
                        .filter(|l| *l != "bootstrap")
                        .map(|l| Hint {
                            path: format!("labels/{}", &hash(l.as_bytes())[..24]),
                            title: l.clone(),
                            keywords: vec![],
                            aliases: vec![],
                            description: String::new(),
                            related_paths: vec![],
                        })
                        .collect::<Vec<_>>()
                } else {
                    vec![Hint {
                        path: "_unfiled".into(),
                        title: "未分类记忆".into(),
                        ..Hint::default()
                    }]
                };
                &fallback
            } else {
                &doc.navigation
            };
            for hint in hints {
                memberships += 1;
                if memberships > 65536 {
                    return Err("导航入口关联超过 65536，请拆分资料范围".into());
                }
                let parts = hint.path.split('/').collect::<Vec<_>>();
                // 在复制到多个祖先前限制最坏文本体量；不依赖最后一次 JSON 分配才发现超限。
                let hint_bytes = serde_json::to_vec(hint).map_err(|e| e.to_string())?.len()
                    + doc.entities.iter().map(String::len).sum::<usize>()
                    + doc.reference.len();
                copied_text_bytes =
                    copied_text_bytes.saturating_add(hint_bytes.saturating_mul(parts.len() + 1));
                if copied_text_bytes > MAX_DERIVED_TEXT_BYTES {
                    return Err("导航派生文本超过 16 MiB，请缩小资料范围；正本未改变".into());
                }
                let mut parent = ROOT.to_string();
                for depth in 1..=parts.len() {
                    let path = parts[..depth].join("/");
                    let reference = format!("{PREFIX}{path}");
                    let fallback_title = if path == "labels" {
                        "标签"
                    } else {
                        parts[depth - 1]
                    };
                    let title = if depth == parts.len() && !hint.title.trim().is_empty() {
                        hint.title.as_str()
                    } else {
                        fallback_title
                    };
                    if !nodes.contains_key(&reference) && nodes.len() >= 4096 {
                        return Err("导航节点超过 4096，请拆分资料范围".into());
                    }
                    nodes
                        .entry(reference.clone())
                        .or_insert_with(|| Node::new(reference.clone(), title.to_string()));
                    let node = nodes.get_mut(&reference).unwrap();
                    if depth == parts.len()
                        && !hint.title.is_empty()
                        && named_nodes.insert(reference.clone())
                    {
                        node.title = hint.title.clone();
                    } else if depth == parts.len()
                        && !hint.title.is_empty()
                        && node.title != hint.title
                    {
                        node.diagnostics
                            .insert("同一路径有不同展示名，保留各标题为别名".into());
                        node.aliases.insert(hint.title.clone());
                    }
                    node.scopes.insert(doc.scope.clone());
                    // 祖先也携带有来源的描述线索，模型不必猜一个空目录名的含义。
                    node.keywords.extend(hint.keywords.iter().cloned());
                    node.keywords.extend(doc.entities.iter().cloned());
                    node.aliases.extend(hint.aliases.iter().cloned());
                    if !hint.description.trim().is_empty() {
                        node.descriptions.push(Description {
                            memory_ref: doc.reference.clone(),
                            text: hint.description.clone(),
                        });
                    }
                    let parent_node = nodes.get_mut(&parent).unwrap();
                    parent_node.children.insert(reference.clone());
                    parent_node.scopes.insert(doc.scope.clone());
                    parent = reference;
                }
                let node = nodes.get_mut(&parent).unwrap();
                node.memories.insert(doc.reference.clone());
                if !hint.description.trim().is_empty() {
                    node.descriptions.push(Description {
                        memory_ref: doc.reference.clone(),
                        text: hint.description.clone(),
                    });
                }
                node.keywords.extend(hint.keywords.iter().cloned());
                node.keywords.extend(doc.entities.iter().cloned());
                node.aliases.extend(hint.aliases.iter().cloned());
                node.related
                    .extend(hint.related_paths.iter().map(|p| format!("{PREFIX}{p}")));
            }
        }
        let existing = nodes.keys().cloned().collect::<BTreeSet<_>>();
        for node in nodes.values_mut() {
            node.descriptions
                .sort_by(|a, b| a.memory_ref.cmp(&b.memory_ref).then(a.text.cmp(&b.text)));
            node.descriptions.dedup();
            for missing in node.related.difference(&existing) {
                node.diagnostics
                    .insert(format!("关联入口当前不可用：{missing}"));
            }
        }
        let mut reached = BTreeSet::new();
        let mut pending = vec![ROOT.to_string()];
        let mut memories = BTreeSet::new();
        while let Some(reference) = pending.pop() {
            if !reached.insert(reference.clone()) {
                continue;
            }
            let node = &nodes[&reference];
            memories.extend(node.memories.iter().cloned());
            pending.extend(node.children.iter().cloned());
        }
        let orphan_refs = current
            .iter()
            .filter(|d| !memories.contains(&d.reference))
            .map(|d| d.reference.clone())
            .collect();
        let generation =
            hash(&serde_json::to_vec(&(scopes, &nodes, &current)).map_err(|e| e.to_string())?);
        Ok(Self {
            schema: "recallcard.navigation/1".into(),
            generation,
            nodes,
            active_memory_count: current.len(),
            orphan_refs,
        })
    }
}

/// Escape all user-derived text, including raw HTML and Markdown link syntax.
fn markdown_text(text: &str) -> String {
    let mut out = String::new();
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\n' => out.push_str("\n\n"),
            '\\' | '`' | '*' | '_' | '{' | '}' | '[' | ']' | '(' | ')' | '#' | '+' | '-' | '!'
            | '|' => {
                out.push('\\');
                out.push(ch);
            }
            _ => out.push(ch),
        }
    }
    out
}
fn page_path(reference: &str) -> String {
    if reference == ROOT {
        "INDEX.md".into()
    } else {
        format!("{}/INDEX.md", reference.strip_prefix(PREFIX).unwrap())
    }
}
impl Index {
    /// Snapshot pages contain references to canonical Memory, never copied facts.
    /// The caller must supply the same authorized projection used by Index::build.
    pub fn markdown_pages(&self) -> BTreeMap<String, String> {
        let mut pages = BTreeMap::new();
        for node in self.nodes.values() {
            let path = page_path(&node.reference);
            let depth = path.matches('/').count();
            let to_root = "../".repeat(depth);
            let mut text = format!(
                "# {}\n\n派生导航快照，可重新生成；不是新的事实源。来源隐藏或记忆变更后须重新导出，旧快照不会自动更新。不要把整个 Vault 分享给受限读者。\n\n- 逻辑引用：{}\n- 代次：{}\n\n[目录首页]({}INDEX.md)\n\n",
                markdown_text(&node.title), markdown_text(&node.reference), self.generation, to_root
            );
            for (heading, refs) in [("子目录", &node.children), ("关联目录", &node.related)]
            {
                text.push_str(&format!("## {heading}\n\n"));
                for reference in refs {
                    if let Some(target) = self.nodes.get(reference) {
                        text.push_str(&format!(
                            "- [{}]({}{})\n",
                            markdown_text(&target.title),
                            to_root,
                            page_path(reference)
                        ));
                    }
                }
                text.push('\n');
            }
            text.push_str("## 正本记忆\n\n");
            for reference in &node.memories {
                let id = reference
                    .strip_prefix("memory:")
                    .unwrap()
                    .split('@')
                    .next()
                    .unwrap();
                text.push_str(&format!(
                    "- [{}]({}../../../memories/{}.md)\n",
                    markdown_text(reference),
                    to_root,
                    id
                ));
            }
            text.push_str("\n## 有来源的导航简介\n\n");
            for description in &node.descriptions {
                text.push_str(&format!(
                    "- {}：{}\n",
                    markdown_text(&description.memory_ref),
                    markdown_text(&description.text)
                ));
            }
            for (heading, values) in [
                ("关键词", &node.keywords),
                ("别名", &node.aliases),
                ("诊断", &node.diagnostics),
            ] {
                text.push_str(&format!("\n## {heading}\n\n"));
                for value in values {
                    text.push_str(&format!("- {}\n", markdown_text(value)));
                }
            }
            pages.insert(path, text);
        }
        pages
    }
}
