//! CI-only 合成资料：现有 LocalEvents → Python SQLite → recall_search/recall_fetch。
//! workflow 临时复制为 Cargo example；不改变库源码或发布入口。
use recallcard::{
    context::Context,
    local_events::{LocalEvents, Setup},
    policy::Access,
    transport, Vault,
};
use serde_json::{json, Value};
use std::{collections::BTreeMap, fs, path::Path, time::Instant};

fn run(config: Value) -> Result<Value, String> {
    if config["suite"] == "attachment-holdout" {
        return run_holdout(&config);
    }
    let mut all = Vec::new();
    for (set_number, dataset) in config["datasets"]
        .as_array()
        .ok_or("fixture_invalid")?
        .iter()
        .enumerate()
    {
        let root = Path::new(config["work"].as_str().ok_or("work_invalid")?)
            .join(format!("vault-{set_number}"));
        let vault = Vault::init(&root)?;
        let corpus = dataset["corpus"].as_array().ok_or("corpus_invalid")?;
        let queries = dataset["queries"].as_array().ok_or("queries_invalid")?;
        let mut references = BTreeMap::new();
        let mut originals = BTreeMap::new();
        for source in corpus {
            let event = vault.capture(serde_json::from_value(json!({
                "scope":"personal", "role":"user", "origin":"user_input",
                "occurred_at":"2026-10-13T00:00:00Z", "content":source["text"],
                "source":{"platform":"synthetic", "conversation_id":dataset["name"], "message_id":source["source_id"]}
            })).map_err(|_| "capture_fixture_invalid")?)?;
            let reference = format!("event:{}", event.id);
            references.insert(reference.clone(), source["source_id"].clone());
            originals.insert(reference, source["text"].clone());
        }
        if !vault.memories()?.is_empty() {
            return Err("fixture_must_not_have_memories".into());
        }
        let context = Context::new(&vault, Access::new(vec!["personal".into()])?);
        let mut baselines = BTreeMap::new();
        for policy in ["default_exact", "question_intent"] {
            let mut rows = Vec::new();
            for query in queries {
                let mode = if policy == "question_intent" && query["intent"] == "question" {
                    "natural_question"
                } else {
                    "exact"
                };
                let found = context.search(
                    serde_json::from_value(json!({"query":query["query"], "target":"events",
                    "limit":20, "budget_bytes":32768, "lexical_match":mode}))
                    .map_err(|_| "baseline_args_invalid")?,
                )?;
                if found["coverage"]["semantic_search"] != "unavailable"
                    || found["truncated"] == true
                {
                    return Err("baseline_must_be_complete_and_lexical_only".into());
                }
                let returned: Result<Vec<_>, _> = found["results"]
                    .as_array()
                    .ok_or("baseline_invalid")?
                    .iter()
                    .map(|row| {
                        references
                            .get(row["ref"].as_str().unwrap_or(""))
                            .cloned()
                            .ok_or("unknown_source")
                    })
                    .collect();
                rows.push(json!({"id":query["id"], "category":query["category"], "expected_sources":query["expected_sources"],
                    "obsolete_or_conflicting_sources":query.get("obsolete_or_conflicting_sources").cloned().unwrap_or(json!([])),
                    "returned_sources":returned?, "mode":mode}));
            }
            baselines.insert(policy, rows);
        }
        let index = LocalEvents::configure(
            &vault,
            "personal",
            Setup {
                endpoint: config["endpoint"]
                    .as_str()
                    .ok_or("endpoint_invalid")?
                    .into(),
                model: config["model"].as_str().ok_or("model_invalid")?.into(),
                revision: config["space_revision"]
                    .as_str()
                    .ok_or("revision_invalid")?
                    .into(),
                tokenizer: config["tokenizer_identity"]
                    .as_str()
                    .ok_or("tokenizer_invalid")?
                    .into(),
                dimensions: 512,
                query_instruction: config["query_instruction"]
                    .as_str()
                    .ok_or("prefix_invalid")?
                    .into(),
                document_instruction: String::new(),
                events_approved: true,
                attachments_approved: false,
                query_approved: true,
                chunk_bytes: 1024,
                disk_cap_mib: 16,
            },
            config["python"].as_str().ok_or("python_invalid")?.into(),
            config["python_path"]
                .as_str()
                .ok_or("python_path_invalid")?
                .into(),
        )?;
        let indexing = Instant::now();
        let mut complete = false;
        let mut steps = 0;
        for _ in 0..16 {
            steps += 1;
            if index.refresh_step()?["complete"] == true {
                complete = true;
                break;
            }
        }
        if !complete {
            return Err("bounded_indexing_incomplete".into());
        }
        let indexing_seconds = indexing.elapsed().as_secs_f64();
        let mut rows = Vec::new();
        let mut fetch_count = 0;
        for query in queries {
            let started = Instant::now();
            let found =
                transport::invoke(&context, "recall_search", json!({"query":query["query"]}))?;
            let elapsed = started.elapsed().as_secs_f64();
            if found["coverage"]["local_event_semantic"] != true
                || found["coverage"]["cloud_fallback"] != false
                || found["coverage"]["scopes"][0]["state"] != "ready"
            {
                return Err("real_semantic_route_unavailable".into());
            }
            let candidates = found["results"]
                .as_array()
                .ok_or("semantic_results_invalid")?;
            let mut returned = Vec::new();
            let mut scores = Vec::new();
            for row in candidates {
                let reference = row["ref"].as_str().ok_or("source_invalid")?;
                returned.push(
                    references
                        .get(reference)
                        .cloned()
                        .ok_or("unknown_semantic_source")?,
                );
                scores.push(row["score"].clone());
                let fetched = transport::invoke(&context, "recall_fetch", json!({"id":row["id"]}))?;
                if fetched["results"][0]["text"]
                    != *originals.get(reference).ok_or("missing_original")?
                    || fetched["results"][0]["snapshot"] != row["snapshot"]
                    || fetched["truncated"] != false
                {
                    return Err("fetch_original_mismatch".into());
                }
                fetch_count += 1;
            }
            rows.push(json!({"id":query["id"], "category":query["category"], "expected_sources":query["expected_sources"],
                "obsolete_or_conflicting_sources":query.get("obsolete_or_conflicting_sources").cloned().unwrap_or(json!([])),
                "returned_sources":returned, "cosine_scores":scores, "seconds":elapsed,
                "truncated_by_existing_facade_budget":found["truncated"], "status":found["status"],
                "query_encoding":found["coverage"]["scopes"][0]["query_encoding"]}));
        }
        let repeat = transport::invoke(
            &context,
            "recall_search",
            json!({"query":queries.last().ok_or("queries_empty")?["query"]}),
        )?;
        if repeat["coverage"]["scopes"][0]["query_encoding"] != "cache" {
            return Err("repeat_query_cache_missing".into());
        }
        all.push(json!({"name":dataset["name"], "sources":corpus.len(), "queries":queries.len(),
            "baselines":baselines, "semantic":rows, "indexing_seconds":indexing_seconds, "index_steps":steps,
            "verified_fetches":fetch_count, "repeated_query_cache_verified":true, "memory_count":vault.memories()?.len()}));
    }
    Ok(json!({"datasets":all}))
}

fn main() {
    let result = std::env::args()
        .nth(1)
        .ok_or("config_required".to_owned())
        .and_then(|path| fs::read(path).map_err(|_| "config_unreadable".into()))
        .and_then(|bytes| serde_json::from_slice(&bytes).map_err(|_| "config_invalid".into()))
        .and_then(run);
    match result {
        Ok(value) => println!(
            "{}",
            serde_json::to_string(&value).expect("synthetic JSON serializes")
        ),
        Err(error) => {
            let code = if !error.is_empty()
                && error.len() <= 96
                && error.bytes().all(|c| c.is_ascii_lowercase() || c == b'_')
            {
                error.as_str()
            } else {
                "source_or_protocol_error"
            };
            eprintln!("recallcard_synthetic_error:{code}");
            std::process::exit(1);
        }
    }
}

fn holdout_scope(scope: &str) -> String {
    format!("project:synthetic-{scope}")
}
fn holdout_settings(config: &Value, attachments: bool) -> Result<Setup, String> {
    Ok(Setup {
        endpoint: config["endpoint"]
            .as_str()
            .ok_or("endpoint_invalid")?
            .into(),
        model: config["model"].as_str().ok_or("model_invalid")?.into(),
        revision: config["space_revision"]
            .as_str()
            .ok_or("revision_invalid")?
            .into(),
        tokenizer: config["tokenizer_identity"]
            .as_str()
            .ok_or("tokenizer_invalid")?
            .into(),
        dimensions: 512,
        query_instruction: config["query_instruction"]
            .as_str()
            .ok_or("prefix_invalid")?
            .into(),
        document_instruction: String::new(),
        events_approved: true,
        attachments_approved: attachments,
        query_approved: true,
        chunk_bytes: 1024,
        disk_cap_mib: 16,
    })
}
fn holdout_visible(query: &Value) -> Result<Vec<String>, String> {
    query["visible_scopes"]
        .as_array()
        .ok_or("query_scopes_invalid")?
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| "query_scopes_invalid".into())
        })
        .collect()
}
fn holdout_admission(query: &Value) -> Result<Option<&'static str>, String> {
    let text = query["query"].as_str().ok_or("query_invalid")?;
    if text.trim().is_empty() {
        return Ok(Some("input_rejected"));
    }
    if holdout_visible(query)?.len() > 4 {
        return Ok(Some("scope_limit"));
    }
    Ok(None)
}
fn holdout_row(query: &Value, corpus: &[Value], status: &str, executed: bool) -> Value {
    let expected = query["expected_sources"]
        .as_array()
        .expect("validated fixture qrels");
    let kinds: std::collections::BTreeSet<_> = corpus
        .iter()
        .filter(|source| expected.contains(&source["source_id"]))
        .map(|source| source["kind"].as_str().expect("validated source kind"))
        .collect();
    json!({"id":query["id"],"category":query["category"],"expected_sources":expected,
        "source_kind_expected":kinds,"visible_scopes":query["visible_scopes"],
        "returned_sources":[],"returned_kinds":[],"obsolete_or_conflicting_sources":[],
        "query_status":status,"route_executed":executed,"seconds":0.0})
}
fn holdout_source<'a>(
    row: &Value,
    sources: &'a BTreeMap<String, Value>,
    query: &Value,
) -> Result<&'a Value, String> {
    let source = sources
        .get(row["ref"].as_str().ok_or("candidate_ref_invalid")?)
        .ok_or("unknown_candidate")?;
    if source["revoked"] == true
        || !query["visible_scopes"]
            .as_array()
            .ok_or("query_scopes_invalid")?
            .contains(&source["scope"])
        || row["scope"] != holdout_scope(source["scope"].as_str().ok_or("source_scope_invalid")?)
    {
        return Err("candidate_scope_or_revocation_violation".into());
    }
    let kind = if source["kind"] == "attachment_parent" {
        "event"
    } else {
        source["kind"].as_str().ok_or("source_kind_invalid")?
    };
    if row["kind"] != kind || row["role"] != source["role"] {
        return Err("candidate_attribution_violation".into());
    }
    Ok(source)
}
fn run_holdout(config: &Value) -> Result<Value, String> {
    let datasets = config["datasets"].as_array().ok_or("fixture_invalid")?;
    if datasets.len() != 1 || datasets[0]["synthetic_only"] != true {
        return Err("synthetic_holdout_required".into());
    }
    let dataset = &datasets[0];
    let corpus = dataset["corpus"].as_array().ok_or("corpus_invalid")?;
    let queries = dataset["queries"].as_array().ok_or("queries_invalid")?;
    if corpus.len() != 24 || queries.len() != 64 {
        return Err("frozen_holdout_count_changed".into());
    }
    let root = Path::new(config["work"].as_str().ok_or("work_invalid")?).join("holdout-vault");
    let vault = Vault::init(&root)?;
    let mut sources = BTreeMap::new();
    let mut scopes = std::collections::BTreeSet::new();
    let mut attachment_parents = 0;
    for source in corpus {
        let scope = source["scope"].as_str().ok_or("source_scope_invalid")?;
        scopes.insert(scope.to_owned());
        let body = source["text"].as_str().ok_or("source_body_invalid")?;
        if body.len() > 1024 || body.trim().is_empty() {
            return Err("single_chunk_fixture_required".into());
        }
        let attachment = source["kind"] == "attachment";
        if !attachment && source["kind"] != "event" {
            return Err("source_kind_invalid".into());
        }
        let mut metadata = json!({"conversation_title":source["title"]});
        let object = recallcard::hash(body.as_bytes());
        if attachment {
            // fixture 本身仅提供 UTF-8 文本。保留原文件名（包括 .pdf），不假装解析真实 PDF。
            fs::write(vault.root().join("objects").join(&object), body)
                .map_err(|_| "synthetic_object_write_failed")?;
            metadata["source_assets"] = json!({"files":[{"name":source["filename"],"payload_status":"retained",
                "detected_media_type":"application/octet-stream","declared_media_type":"text/plain",
                "object_sha256":object,"byte_count":body.len()}]});
        }
        let event = vault.capture(serde_json::from_value(json!({
            "scope":holdout_scope(scope),"role":if attachment{json!("user")}else{source["role"].clone()},
            "origin":if !attachment && source["role"]=="assistant"{"assistant_output"}else{"user_input"},"content":if attachment{""}else{body},"metadata":metadata,
            "source":{"platform":"synthetic","conversation_id":dataset["name"],"message_id":source["source_id"]}
        })).map_err(|_|"capture_fixture_invalid")?)?;
        if source["revoked"] == true {
            vault.suppress(&event.id, "合成固定撤销标记".into())?;
        }
        let parent_ref = format!("event:{}", event.id);
        if attachment {
            attachment_parents += 1;
            let mut parent = source.clone();
            parent["source_id"] = json!(format!(
                "parent:{}",
                source["source_id"].as_str().ok_or("source_id_invalid")?
            ));
            parent["kind"] = json!("attachment_parent");
            parent["role"] = json!("user");
            parent["text"] = json!("");
            sources.insert(parent_ref, parent);
            sources.insert(
                format!("attachment:{}/{}", event.id, object),
                source.clone(),
            );
        } else {
            sources.insert(parent_ref, source.clone());
        }
    }
    if scopes.len() != 6 || attachment_parents != 12 || !vault.memories()?.is_empty() {
        return Err("holdout_shape_changed".into());
    }
    let mut baselines = BTreeMap::new();
    for policy in ["default_exact", "question_intent"] {
        let mut rows = Vec::new();
        for query in queries {
            let denied = holdout_admission(query)?;
            let mut row = holdout_row(
                query,
                corpus,
                denied.unwrap_or("lexical_only"),
                denied.is_none(),
            );
            if denied.is_none() {
                let context = Context::new(
                    &vault,
                    Access::new(
                        holdout_visible(query)?
                            .iter()
                            .map(|scope| holdout_scope(scope))
                            .collect(),
                    )?,
                );
                let mode = if policy == "question_intent" && query["lexical_intent"] == "question" {
                    "natural_question"
                } else {
                    "exact"
                };
                let page = context.search(
                    serde_json::from_value(
                        json!({"query":query["query"],"target":"events","limit":20,
                    "budget_bytes":32768,"lexical_match":mode}),
                    )
                    .map_err(|_| "baseline_args_invalid")?,
                )?;
                if page["coverage"]["semantic_search"] != "unavailable" || page["truncated"] == true
                {
                    return Err("baseline_must_be_complete_and_lexical_only".into());
                }
                let mut returned = vec![];
                let mut kinds = vec![];
                for found in page["results"]
                    .as_array()
                    .ok_or("baseline_results_invalid")?
                {
                    let source = holdout_source(found, &sources, query)?;
                    returned.push(source["source_id"].clone());
                    kinds.push(source["kind"].clone());
                }
                row["returned_sources"] = json!(returned);
                row["returned_kinds"] = json!(kinds);
                row["mode"] = json!(mode);
            }
            rows.push(row);
        }
        baselines.insert(policy, rows);
    }
    let mut all = vec![];
    for (variant, approved) in [
        ("events_semantic_attachment_lexical", false),
        ("events_and_attachments_semantic", true),
    ] {
        let indexing = Instant::now();
        let mut steps = 0;
        for scope in &scopes {
            let index = LocalEvents::configure(
                &vault,
                &holdout_scope(scope),
                holdout_settings(config, approved)?,
                config["python"].as_str().ok_or("python_invalid")?.into(),
                config["python_path"]
                    .as_str()
                    .ok_or("python_path_invalid")?
                    .into(),
            )?;
            let mut complete = false;
            for _ in 0..32 {
                steps += 1;
                if index.refresh_step()?["complete"] == true {
                    complete = true;
                    break;
                }
            }
            if !complete {
                return Err("bounded_indexing_incomplete".into());
            }
            let expected = corpus
                .iter()
                .filter(|source| {
                    source["scope"] == *scope
                        && source["revoked"] != true
                        && (source["kind"] == "event" || approved)
                })
                .count();
            if index.status()?["source_count"] != expected {
                return Err("approved_source_count_mismatch".into());
            }
        }
        let indexing_seconds = indexing.elapsed().as_secs_f64();
        let mut rows = vec![];
        let mut fetch_count = 0;
        for query in queries {
            let context = Context::new(
                &vault,
                Access::new(
                    holdout_visible(query)?
                        .iter()
                        .map(|scope| holdout_scope(scope))
                        .collect(),
                )?,
            );
            let denied = holdout_admission(query)?;
            let started = Instant::now();
            let attempt =
                transport::invoke(&context, "recall_search", json!({"query":query["query"]}));
            let mut row = holdout_row(
                query,
                corpus,
                denied.unwrap_or("semantic"),
                denied.is_none(),
            );
            if let Some(why) = denied {
                match (why, attempt) {
                    ("input_rejected", Err(_)) => {}
                    ("scope_limit", Ok(page))
                        if page["status"] == "scope_limit" && page["results"] == json!([]) => {}
                    _ => return Err("facade_admission_contract_changed".into()),
                }
            } else {
                let page = attempt?;
                let coverage = page["coverage"]["scopes"]
                    .as_array()
                    .ok_or("scope_coverage_missing")?;
                if page["coverage"]["cloud_fallback"] != false
                    || page["coverage"]["local_event_semantic"] != true
                    || coverage.len() != holdout_visible(query)?.len()
                    || coverage.iter().any(|scope| scope["state"] != "ready")
                {
                    return Err("real_semantic_route_unavailable".into());
                }
                if page["coverage"]["attachments"]["semantic"] != approved {
                    return Err("attachment_semantic_consent_mismatch".into());
                }
                if approved
                    && coverage
                        .iter()
                        .any(|scope| scope["query_encoding"] != "cache")
                {
                    return Err("paired_query_cache_missing".into());
                }
                let mut returned = vec![];
                let mut kinds = vec![];
                let mut scores = vec![];
                let mut score_kinds = vec![];
                for found in page["results"]
                    .as_array()
                    .ok_or("semantic_results_invalid")?
                {
                    let source = holdout_source(found, &sources, query)?;
                    let fetched =
                        transport::invoke(&context, "recall_fetch", json!({"id":found["id"]}))?;
                    let item = &fetched["results"][0];
                    let original = source["text"].as_str().ok_or("source_body_invalid")?;
                    let start = item["text_range"]["start_byte"]
                        .as_u64()
                        .ok_or("fetch_range_missing")? as usize;
                    let end = item["text_range"]["end_byte"]
                        .as_u64()
                        .ok_or("fetch_range_missing")? as usize;
                    if start > end
                        || end > original.len()
                        || !original.is_char_boundary(start)
                        || !original.is_char_boundary(end)
                        || item["text"] != original[start..end]
                        || item["snapshot"] != found["snapshot"]
                        || item["ref"] != found["ref"]
                        || item["text_range"]["total_bytes"] != original.len()
                        || fetched["truncated"] == true
                    {
                        return Err("fetch_original_mismatch".into());
                    }
                    returned.push(source["source_id"].clone());
                    kinds.push(source["kind"].clone());
                    scores.push(found["score"].clone());
                    score_kinds.push(
                        found["score_kind"]
                            .as_str()
                            .unwrap_or("local_lexical_score")
                            .to_owned(),
                    );
                    fetch_count += 1;
                }
                row["returned_sources"] = json!(returned);
                row["returned_kinds"] = json!(kinds);
                row["retrieval_scores"] = json!(scores);
                row["retrieval_score_kinds"] = json!(score_kinds);
                row["retrieval_score_note"] =
                    json!("保留生产排序和通道原分数；词法与cosine不可跨通道比较，不是相关概率");
                row["truncated_by_existing_facade_budget"] = page["truncated"].clone();
                row["status"] = page["status"].clone();
                row["query_encoding"] = json!(coverage
                    .iter()
                    .map(|scope| scope["query_encoding"].clone())
                    .collect::<Vec<_>>());
            }
            row["seconds"] = json!(started.elapsed().as_secs_f64());
            rows.push(row);
        }
        let last = queries
            .iter()
            .rev()
            .find(|query| holdout_admission(query).is_ok_and(|value| value.is_none()))
            .ok_or("queries_empty")?;
        let context = Context::new(
            &vault,
            Access::new(
                holdout_visible(last)?
                    .iter()
                    .map(|scope| holdout_scope(scope))
                    .collect(),
            )?,
        );
        let repeat = transport::invoke(&context, "recall_search", json!({"query":last["query"]}))?;
        if repeat["coverage"]["scopes"]
            .as_array()
            .ok_or("scope_coverage_missing")?
            .iter()
            .any(|scope| scope["query_encoding"] != "cache")
        {
            return Err("repeat_query_cache_missing".into());
        }
        all.push(json!({"name":dataset["name"],"variant":variant,"attachments_approved":approved,
            "sources":corpus.len(),"queries":queries.len(),"synthetic_attachment_parents":attachment_parents,
            "baselines":baselines,"semantic":rows,"indexing_seconds":indexing_seconds,"index_steps":steps,
            "verified_fetches":fetch_count,"repeated_query_cache_verified":true,"memory_count":vault.memories()?.len()}));
    }
    Ok(json!({"datasets":all,"sqlite_index_count":scopes.len()}))
}

#[cfg(test)]
mod holdout_contracts {
    use super::*;
    #[test]
    fn admission_uses_exact_input_and_never_silently_trims_scopes() {
        assert_eq!(
            holdout_admission(
                &json!({"query":"\t \n\u{3000}","visible_scopes":["a","b","c","d","e"]})
            )
            .unwrap(),
            Some("input_rejected")
        );
        assert_eq!(
            holdout_admission(&json!({"query":"？！","visible_scopes":["a","b","c","d","e"]}))
                .unwrap(),
            Some("scope_limit")
        );
        assert_eq!(
            holdout_admission(&json!({"query":"原样问题","visible_scopes":["a","b"]})).unwrap(),
            None
        );
    }
    #[test]
    fn attribution_scope_and_revocation_are_checked_before_counting_candidates() {
        let reference = "attachment:synthetic";
        let mut sources = BTreeMap::from([(
            reference.to_owned(),
            json!({"source_id":"ATT-X","scope":"home","kind":"attachment","role":null,"revoked":false}),
        )]);
        let query = json!({"visible_scopes":["home"]});
        let mut row = json!({"ref":reference,"kind":"attachment","role":null,"scope":"project:synthetic-home"});
        assert!(holdout_source(&row, &sources, &query).is_ok());
        row["role"] = json!("user");
        assert!(holdout_source(&row, &sources, &query).is_err());
        row["role"] = Value::Null;
        row["scope"] = json!("project:synthetic-other");
        assert!(holdout_source(&row, &sources, &query).is_err());
        row["scope"] = json!("project:synthetic-home");
        sources.get_mut(reference).unwrap()["revoked"] = json!(true);
        assert!(holdout_source(&row, &sources, &query).is_err());
    }
    #[test]
    fn unanswered_and_mixed_source_annotations_are_not_rewritten_by_variant() {
        let corpus = vec![
            json!({"source_id":"A","kind":"attachment"}),
            json!({"source_id":"E","kind":"event"}),
        ];
        let query = json!({"id":"Q","category":"synthetic","expected_sources":["E","A"],"visible_scopes":["home"]});
        let row = holdout_row(&query, &corpus, "semantic", true);
        assert_eq!(row["expected_sources"], query["expected_sources"]);
        assert_eq!(row["source_kind_expected"], json!(["attachment", "event"]));
        let mut negative = query;
        negative["expected_sources"] = json!([]);
        let row = holdout_row(&negative, &corpus, "scope_limit", false);
        assert_eq!(row["expected_sources"], json!([]));
        assert_eq!(row["route_executed"], false);
    }
}
