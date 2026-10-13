//! 简洁的共同本机读取入口；不访问旧云配置，不隐式建索引。
use super::*;
use crate::local_events::LocalEvents;
use std::time::{Duration, Instant};
const BUDGET: usize = 8192;
const LIMIT: usize = 10;
const SCOPES: usize = 4;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecallSearchArgs {
    pub query: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecallFetchArgs {
    pub id: String,
}
fn scopes_hash(access: &Access) -> String {
    hash(json!(access.scopes()).to_string().as_bytes())
}
fn code(error: &str) -> &str {
    if error.len() <= 96 && error.bytes().all(|c| c.is_ascii_lowercase() || c == b'_') {
        error
    } else {
        "local_read_unavailable"
    }
}
fn hash_valid(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}
// 这是可校验的定位符，不是权限或会话票据。Web text/2 后续由 host 映射会话内短编号。
struct Locator {
    scope: String,
    reference: String,
    snapshot: String,
    epoch: String,
    config: String,
    scopes: String,
    offset: usize,
}
impl Locator {
    fn id(&self) -> String {
        format!(
            "rcs1|{}|{}|{}|{}|{}|{}|{}",
            self.scope,
            self.reference,
            self.snapshot,
            self.epoch,
            self.config,
            self.scopes,
            self.offset
        )
    }
    fn parse(id: &str) -> Result<Self> {
        if id.len() > 768 {
            return Err("资料定位符过长".into());
        }
        let p: Vec<_> = id.split('|').collect();
        if p.len() != 8
            || p[0] != "rcs1"
            || !hash_valid(p[3])
            || !hash_valid(p[4])
            || !(p[5] == "-" || hash_valid(p[5]))
            || !hash_valid(p[6])
        {
            return Err("资料定位符无效；请重新检索".into());
        }
        validate_scope(p[1])?;
        let reference = p[2];
        if let Some(id) = reference.strip_prefix("event:") {
            validate_id(id, "evt_")?;
            if p[5] == "-" {
                return Err("事件定位符缺少索引绑定".into());
            }
        } else if let Some(id) = reference.strip_prefix("memory:") {
            let (id, revision) = id.split_once('@').ok_or("记忆定位符缺少版本")?;
            validate_id(id, "mem_")?;
            revision.parse::<u64>().map_err(|_| "记忆版本无效")?;
            if p[5] != "-" {
                return Err("记忆定位符无效".into());
            }
        } else {
            return Err("只能读取已返回的 Event/Memory 定位符".into());
        }
        let offset = p[7].parse::<usize>().map_err(|_| "资料字节起点无效")?;
        if offset > 4 * 1024 * 1024 {
            return Err("资料字节起点超限".into());
        }
        Ok(Self {
            scope: p[1].into(),
            reference: reference.into(),
            snapshot: p[3].into(),
            epoch: p[4].into(),
            config: p[5].into(),
            scopes: p[6].into(),
            offset,
        })
    }
}
impl Context<'_> {
    pub fn recall_search(&self, args: RecallSearchArgs) -> Result<Value> {
        self.recall_question(&args.query, "all")
    }
    /// 桌面固定 target；模型简洁参数不能改变范围、预算、提供器或批准。
    pub fn recall_question(&self, query: &str, target: &str) -> Result<Value> {
        if query.trim().is_empty() || query.len() > 4096 || query.chars().any(|c| c == '\0') {
            return Err("请输入1–4096字节的检索问题".into());
        }
        if !matches!(target, "all" | "events" | "memories") {
            return Err("简洁检索只支持普通对话和记忆".into());
        }
        let scopes = self.access.scopes();
        if scopes.len() > SCOPES {
            return Ok(
                json!({"status":"scope_limit","results":[],"truncated":true,"coverage":{"scope_limit":SCOPES,"selected_scopes":scopes.len(),"note":"请在宿主连接设置中明确缩小本次范围；没有静默选择或发送查询。","cloud_fallback":false}}),
            );
        }
        let deadline = Instant::now() + Duration::from_secs(15);
        let mut groups: Vec<Vec<Value>> = vec![];
        let mut bindings = vec![];
        let mut coverage = vec![];
        let mut partial = false;
        let scopes_digest = scopes_hash(&self.access);
        if target != "memories" {
            for scope in &scopes {
                if Instant::now() >= deadline {
                    coverage.push(json!({"scope":scope,"state":"deadline"}));
                    partial = true;
                    continue;
                }
                let loaded = LocalEvents::open(self.vault, scope);
                let index = match loaded {
                    Ok(Some(index)) => index,
                    Ok(None) => {
                        coverage.push(json!({"scope":scope,"state":"unconfigured"}));
                        partial = true;
                        continue;
                    }
                    Err(error) => {
                        coverage
                            .push(json!({"scope":scope,"state":"unavailable","code":code(&error)}));
                        partial = true;
                        continue;
                    }
                };
                match index.search_before(query, 20, deadline) {
                    Ok(page) => {
                        let epoch = page["coverage"]["event_index_snapshot"]
                            .as_str()
                            .ok_or("本机索引响应缺少版本")?
                            .to_owned();
                        let config = page["coverage"]["local_config_identity"]
                            .as_str()
                            .ok_or("本机索引响应缺少配置绑定")?
                            .to_owned();
                        let mut rows = page["results"]
                            .as_array()
                            .ok_or("本机索引响应无效")?
                            .clone();
                        for row in &mut rows {
                            let locator = Locator {
                                scope: scope.clone(),
                                reference: row["ref"].as_str().ok_or("本机来源无效")?.into(),
                                snapshot: row["snapshot"]
                                    .as_str()
                                    .ok_or("本机来源缺少快照")?
                                    .into(),
                                epoch: epoch.clone(),
                                config: config.clone(),
                                scopes: scopes_digest.clone(),
                                offset: row["text_range"]["start_byte"]
                                    .as_u64()
                                    .ok_or("本机来源缺少范围")?
                                    as usize,
                            };
                            row["id"] = json!(locator.id());
                            row["relevance"] = json!("unassessed");
                        }
                        partial |= page["truncated"] == true;
                        coverage.push(json!({"scope":scope,"state":"ready","candidates":rows.len(),"truncated":page["truncated"],"query_encoding":page["coverage"]["semantic_query_encoding"]}));
                        bindings.push((index, epoch, config));
                        groups.push(rows);
                    }
                    Err(error) => {
                        coverage
                            .push(json!({"scope":scope,"state":"unavailable","code":code(&error)}));
                        partial = true;
                    }
                }
            }
        }
        // 所有提供器等待已结束；同一最终读锁下重验早先 scope 的来源/配置并构造最终包。
        let _guard = self.vault.read_guard()?;
        for (index, epoch, config) in &bindings {
            index.verify_binding(epoch, config)?;
        }
        if target != "events" {
            let local = Context::new(self.vault, self.access.clone());
            let args=serde_json::from_value(json!({"query":query,"target":"memories","lexical_match":"natural_question","limit":10,"budget_bytes":8192})).map_err(|_|"内部检索参数无效")?;
            let page = local.search(args)?;
            partial |= page["truncated"] == true;
            let epoch = crate::local_events::source_epoch(self.vault)?;
            let mut rows = vec![];
            for found in page["results"].as_array().ok_or("记忆候选响应无效")? {
                let reference = found["ref"].as_str().ok_or("记忆候选缺少引用")?;
                let read:ReadPageArgs=serde_json::from_value(json!({"refs":[reference],"detail":"compact","offset_bytes":found["text_range"]["start_byte"].as_u64().unwrap_or(0),"budget_bytes":2048})).map_err(|_|"内部读取参数无效")?;
                let resolved = local.read_page(read)?;
                let Some(row) = resolved["results"].as_array().and_then(|r| r.first()) else {
                    partial = true;
                    continue;
                };
                let mut value = found.clone();
                for key in ["text", "snapshot", "text_range"] {
                    value[key] = row[key].clone();
                }
                value["text_hash"] =
                    json!(hash(row["text"].as_str().ok_or("记忆正文无效")?.as_bytes()));
                value["text_truncated"] = json!(
                    row["text_range"]["start_byte"] != 0
                        || row["text_range"]["end_byte"] != row["text_range"]["total_bytes"]
                );
                let locator = Locator {
                    scope: found["scope"].as_str().ok_or("记忆缺少范围")?.into(),
                    reference: reference.into(),
                    snapshot: row["snapshot"].as_str().ok_or("记忆缺少快照")?.into(),
                    epoch: epoch.clone(),
                    config: "-".into(),
                    scopes: scopes_digest.clone(),
                    offset: row["text_range"]["start_byte"]
                        .as_u64()
                        .ok_or("记忆范围无效")? as usize,
                };
                value["id"] = json!(locator.id());
                value["relevance"] = json!("unassessed");
                rows.push(value);
            }
            groups.push(rows);
        }
        let candidate_count: usize = groups.iter().map(Vec::len).sum();
        let mut rows = vec![];
        // 不比较不同模型空间的 cosine；按每路原始排名稳定轮询。
        for i in 0..20 {
            for group in &groups {
                if let Some(row) = group.get(i) {
                    if rows.len() < LIMIT {
                        rows.push(row.clone());
                    }
                }
            }
        }
        partial |= rows.len() < candidate_count;
        let mut result = json!({"results":rows,"match_count":rows.len(),"status":if partial {"partial"}else if rows.is_empty(){"no_verified_candidates"}else{"ok"},"truncated":partial,"budget_unit":"utf8_bytes","coverage":{"event_search":"local_only","memory_search":"lexical_only_no_cloud_fallback","local_event_semantic":!bindings.is_empty(),"scopes":coverage,"cloud_fallback":false,"relevance":"unassessed","merge":"stable_round_robin_not_cross_model_score","note":"候选不是答案；未配置或未覆盖不表示没有历史。旧search仍可显式进行关键词检索。"}});
        if result["results"].as_array().unwrap().is_empty() && partial {
            result["status"] = json!("unavailable");
        }
        while serde_json::to_vec(&result)
            .map_err(|_| "检索结果无法序列化")?
            .len()
            > BUDGET
        {
            result["truncated"] = json!(true);
            result["status"] = json!("partial");
            if result["results"].as_array_mut().unwrap().pop().is_none() {
                return Err("检索元数据超过固定预算".into());
            }
            result["match_count"] = json!(result["results"].as_array().unwrap().len());
            if result["match_count"] == 0 {
                result["status"] = json!("budget_exhausted");
            }
        }
        Ok(result)
    }
    pub fn recall_fetch(&self, args: RecallFetchArgs) -> Result<Value> {
        let mut locator = Locator::parse(&args.id)?;
        if locator.scopes != scopes_hash(&self.access) || !self.access.permits(&locator.scope) {
            return Err("资料范围已变化，请重新检索".into());
        }
        let _guard = self.vault.read_guard()?;
        if crate::local_events::source_epoch(self.vault)? != locator.epoch {
            return Err("资料来源已变化，请重新检索".into());
        }
        if locator.config != "-" {
            LocalEvents::open(self.vault, &locator.scope)?
                .ok_or("本机索引已停用，请重新检索")?
                .verify_source(&locator.epoch, &locator.config, &locator.reference)?;
        }
        let read=serde_json::from_value(json!({"refs":[locator.reference],"detail":"compact","offset_bytes":locator.offset,"budget_bytes":6144})).map_err(|_|"内部读取参数无效")?;
        let page = self.read_page(read)?;
        let row = page["results"]
            .as_array()
            .and_then(|r| r.first())
            .ok_or("当前读取预算不足，请使用原始读取入口")?;
        if row["ref"] != locator.reference
            || row["snapshot"] != locator.snapshot
            || row["metadata"]["scope"] != locator.scope
            || row["text_range"]["start_byte"] != locator.offset
            || row["text_range"]["projection"]
                != if locator.config == "-" {
                    "memory.content"
                } else {
                    "event.text"
                }
        {
            return Err("资料版本或范围已变化，请重新检索".into());
        }
        let mut row = row.clone();
        row["id"] = json!(args.id);
        row["text_hash"] = json!(hash(row["text"].as_str().ok_or("读取正文无效")?.as_bytes()));
        let end = row["text_range"]["end_byte"]
            .as_u64()
            .ok_or("读取范围无效")? as usize;
        let next = if page["truncated"] == true && end > locator.offset {
            locator.offset = end;
            Some(locator.id())
        } else {
            None
        };
        let result = json!({"results":[row],"next_id":next,"status":page["status"],"truncated":page["truncated"],"budget_unit":"utf8_bytes","reference_data":true});
        if serde_json::to_vec(&result)
            .map_err(|_| "读取结果无法序列化")?
            .len()
            > BUDGET
        {
            return Err("读取结果超过固定预算".into());
        }
        Ok(result)
    }
}
