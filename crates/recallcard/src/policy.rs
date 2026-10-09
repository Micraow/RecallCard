//! 客户端范围由启动者绑定；模型参数没有扩大权限的能力。
use crate::{
    model::*,
    vault::{files_recursive, read_json},
    Vault,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone)]
pub struct Access {
    scopes: BTreeSet<String>,
}
impl Access {
    pub fn new(scopes: Vec<String>) -> Result<Self> {
        if scopes.is_empty() {
            return Err("必须显式配置客户端可读取的 --scope".into());
        }
        for scope in &scopes {
            validate_scope(scope)?;
        }
        Ok(Self {
            scopes: scopes.into_iter().collect(),
        })
    }
    pub fn permits(&self, scope: &str) -> bool {
        self.scopes.contains(scope)
    }
    pub fn scopes(&self) -> Vec<String> {
        self.scopes.iter().cloned().collect()
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Suppression {
    pub schema_version: u32,
    pub id: String,
    pub active: bool,
    pub source_refs: Vec<String>,
    /// 同一 scope 内来源身份的摘要；不依赖来源当前内容或修订编号。
    #[serde(default)]
    pub source_hashes: Vec<String>,
    pub reason: String,
    pub updated_at: chrono::DateTime<Utc>,
}
impl Vault {
    pub fn suppress(&self, id: &str, reason: String) -> Result<Suppression> {
        let _lock = self.lock()?;
        self.suppress_locked(id, reason)
    }
    /// 调用方持有写锁；供桌面在同一快照中确认影响和写入规则。
    pub(crate) fn suppress_locked(&self, id: &str, reason: String) -> Result<Suppression> {
        nonempty(&reason, "遗忘原因")?;
        if reason.len() > 4096 {
            return Err("遗忘原因不能超过 4096 字节".into());
        }
        let refs = if id.starts_with("mem_") {
            self.memory(id)?.data.source_refs
        } else {
            self.event(id)?;
            vec![id.to_owned()]
        };
        let source_hashes = refs
            .iter()
            .map(|id| self.event(id).map(|event| suppression_source_hash(&event)))
            .collect::<Result<BTreeSet<_>>>()?
            .into_iter()
            .collect();
        let s = Suppression {
            schema_version: 1,
            id: id.into(),
            active: true,
            source_refs: refs,
            source_hashes,
            reason,
            updated_at: Utc::now(),
        };
        self.write_replace(
            &self
                .root()
                .join("control/suppressions")
                .join(format!("{id}.json")),
            &s,
        )?;
        Ok(s)
    }
    pub fn restore(&self, id: &str) -> Result<Suppression> {
        let _lock = self.lock()?;
        self.restore_locked(id)
    }
    /// 调用方持有写锁；不意味着其他规则同时撤销。
    pub(crate) fn restore_locked(&self, id: &str) -> Result<Suppression> {
        validate_record_id(id)?;
        let path = self
            .root()
            .join("control/suppressions")
            .join(format!("{id}.json"));
        let mut s: Suppression = read_json(&path)?;
        s.active = false;
        s.updated_at = Utc::now();
        self.write_replace(&path, &s)?;
        Ok(s)
    }
    pub fn suppressed_ids(&self) -> Result<BTreeSet<String>> {
        self.ensure_no_pending_dream()?;
        self.suppressed_ids_impl()
    }
    /// 仅限持有 Vault 写锁的 Dream 事务验证；绝不用于普通检索或模型读取。
    pub(crate) fn suppressed_ids_for_dream_recovery<'a>(
        &self,
        events: impl Iterator<Item = &'a Event> + Clone,
    ) -> Result<BTreeSet<String>> {
        self.check_dream_recovery_barrier()?;
        self.suppressed_ids_from_event_iter_unchecked(events)
    }
    pub(crate) fn check_dream_recovery_barrier(&self) -> Result<()> {
        if self.state_dir()?.join("event-transaction.json").exists() {
            return Err("存在未恢复的事件事务，不能恢复 Dream".into());
        }
        Ok(())
    }
    fn suppression_rules(&self) -> Result<Vec<Suppression>> {
        let mut rules = Vec::new();
        for path in files_recursive(&self.root().join("control/suppressions"), "json")? {
            let s: Suppression = read_json(&path)?;
            validate_record_id(&s.id)?;
            if s.schema_version != 1 {
                return Err("不支持的 suppression 版本".into());
            }
            if path.file_stem().and_then(|name| name.to_str()) != Some(s.id.as_str()) {
                return Err("抑制规则文件名与编号不一致".into());
            }
            for id in &s.source_refs {
                validate_id(id, "evt_")?;
            }
            for digest in &s.source_hashes {
                if digest.len() != 64 || !digest.bytes().all(|b| b.is_ascii_hexdigit()) {
                    return Err("抑制规则来源摘要无效".into());
                }
            }
            rules.push(s);
        }
        Ok(rules)
    }
    pub(crate) fn directly_suppressed_ids(&self) -> Result<BTreeSet<String>> {
        let mut ids = BTreeSet::new();
        for rule in self
            .suppression_rules()?
            .into_iter()
            .filter(|rule| rule.active)
        {
            ids.insert(rule.id);
            ids.extend(rule.source_refs);
        }
        Ok(ids)
    }
    /// 只扩展本次确实要读的来源；规则每次重读，身份来自已验证的当前 Event，非缓存字段。
    pub(crate) fn suppressed_ids_for_sources(
        &self,
        sources: &BTreeSet<String>,
        events: &mut crate::vault::EventSnapshot<'_>,
    ) -> Result<BTreeSet<String>> {
        self.ensure_no_pending_dream()?;
        let mut ids = BTreeSet::new();
        let mut hashes = BTreeSet::new();
        for rule in self
            .suppression_rules()?
            .into_iter()
            .filter(|rule| rule.active)
        {
            ids.insert(rule.id);
            ids.extend(rule.source_refs);
            hashes.extend(rule.source_hashes);
        }
        for id in ids.iter().filter(|id| id.starts_with("evt_")) {
            match events.event(id) {
                Ok(event) => {
                    hashes.insert(suppression_source_hash(&event));
                }
                Err(error) if error == format!("找不到原始事件 {id}") => {}
                Err(error) => return Err(error),
            }
        }
        if !hashes.is_empty() {
            for id in sources {
                if ids.contains(id) {
                    continue;
                }
                let event = events.event(id)?;
                if hashes.contains(&suppression_source_hash(&event)) {
                    ids.insert(id.clone());
                }
            }
        }
        Ok(ids)
    }
    /// 调用方已经在同一锁内读取并验证了完整 Event 快照；不重复扫描，也不跨请求保存身份。
    pub(crate) fn suppressed_ids_from_events(&self, events: &[Event]) -> Result<BTreeSet<String>> {
        self.suppressed_ids_from_event_iter(events.iter())
    }
    pub(crate) fn suppressed_ids_from_event_iter<'a>(
        &self,
        events: impl Iterator<Item = &'a Event> + Clone,
    ) -> Result<BTreeSet<String>> {
        self.ensure_no_pending_dream()?;
        self.suppressed_ids_from_event_iter_unchecked(events)
    }
    // 仅供已检查相应事务屏障的同锁快照调用，不用于绕过公共读取屏障。
    fn suppressed_ids_from_event_iter_unchecked<'a>(
        &self,
        events: impl Iterator<Item = &'a Event> + Clone,
    ) -> Result<BTreeSet<String>> {
        let mut ids = BTreeSet::new();
        let mut source_hashes = BTreeSet::new();
        for rule in self
            .suppression_rules()?
            .into_iter()
            .filter(|rule| rule.active)
        {
            ids.insert(rule.id);
            ids.extend(rule.source_refs);
            source_hashes.extend(rule.source_hashes);
        }
        if !ids.is_empty() {
            for event in events.clone() {
                if ids.contains(&event.id) {
                    source_hashes.insert(suppression_source_hash(event));
                }
            }
            for event in events {
                if source_hashes.contains(&suppression_source_hash(event)) {
                    ids.insert(event.id.clone());
                }
            }
        }
        Ok(ids)
    }
    fn suppressed_ids_impl(&self) -> Result<BTreeSet<String>> {
        let mut ids = BTreeSet::new();
        let mut source_hashes = BTreeSet::new();
        for s in self.suppression_rules()? {
            if s.active {
                ids.insert(s.id);
                source_hashes.extend(s.source_hashes);
                for id in s.source_refs {
                    ids.insert(id);
                }
            }
        }
        // 兼容尚无 source_hashes 的旧规则，从仍保留的不可变 Event 补出身份。
        // 仅扩散到同 scope、同平台/账户/会话/消息，绝不按内容相似度遗忘。
        if !ids.is_empty() {
            let collect_hashes = |event: Event| {
                if ids.contains(&event.id) {
                    source_hashes.insert(suppression_source_hash(&event));
                }
                Ok(())
            };
            self.visit_events(collect_hashes)?;
            let expand_revisions = |event: Event| {
                if source_hashes.contains(&suppression_source_hash(&event)) {
                    ids.insert(event.id);
                }
                Ok(())
            };
            self.visit_events(expand_revisions)?;
        }
        Ok(ids)
    }
    pub fn is_suppressed(&self, id: &str) -> Result<bool> {
        Ok(self.suppressed_ids()?.contains(id))
    }
}
fn suppression_source_hash(event: &Event) -> String {
    let source = &event.data.source;
    hash(
        serde_json::json!([
            event.data.scope,
            source.platform,
            source.account_namespace,
            source.conversation_id,
            source.message_id
        ])
        .to_string()
        .as_bytes(),
    )
}
fn validate_record_id(id: &str) -> Result<()> {
    if id.starts_with("evt_") {
        validate_id(id, "evt_")
    } else {
        validate_id(id, "mem_")
    }
}
