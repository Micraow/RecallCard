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
    pub reason: String,
    pub updated_at: chrono::DateTime<Utc>,
}
impl Vault {
    pub fn suppress(&self, id: &str, reason: String) -> Result<Suppression> {
        nonempty(&reason, "遗忘原因")?;
        let refs = if id.starts_with("mem_") {
            self.memory(id)?.data.source_refs
        } else {
            self.event(id)?;
            vec![id.to_owned()]
        };
        let _lock = self.lock()?;
        let s = Suppression {
            schema_version: 1,
            id: id.into(),
            active: true,
            source_refs: refs,
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
        validate_record_id(id)?;
        let _lock = self.lock()?;
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
        let mut ids = BTreeSet::new();
        for path in files_recursive(&self.root().join("control/suppressions"), "json")? {
            let s: Suppression = read_json(&path)?;
            validate_record_id(&s.id)?;
            if s.schema_version != 1 {
                return Err("不支持的 suppression 版本".into());
            }
            if s.active {
                ids.insert(s.id);
                for id in s.source_refs {
                    validate_id(&id, "evt_")?;
                    ids.insert(id);
                }
            }
        }
        Ok(ids)
    }
    pub fn is_suppressed(&self, id: &str) -> Result<bool> {
        Ok(self.suppressed_ids()?.contains(id))
    }
}
fn validate_record_id(id: &str) -> Result<()> {
    if id.starts_with("evt_") {
        validate_id(id, "evt_")
    } else {
        validate_id(id, "mem_")
    }
}
