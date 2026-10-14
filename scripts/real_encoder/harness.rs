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
    let mut all = Vec::new();
    for (set_number, dataset) in config["datasets"].as_array().ok_or("fixture_invalid")?.iter().enumerate() {
        let root = Path::new(config["work"].as_str().ok_or("work_invalid")?).join(format!("vault-{set_number}"));
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
        if !vault.memories()?.is_empty() { return Err("fixture_must_not_have_memories".into()); }
        let context = Context::new(&vault, Access::new(vec!["personal".into()])?);
        let mut baselines = BTreeMap::new();
        for policy in ["default_exact", "question_intent"] {
            let mut rows = Vec::new();
            for query in queries {
                let mode = if policy == "question_intent" && query["intent"] == "question" {"natural_question"} else {"exact"};
                let found = context.search(serde_json::from_value(json!({"query":query["query"], "target":"events",
                    "limit":20, "budget_bytes":32768, "lexical_match":mode})).map_err(|_| "baseline_args_invalid")?)?;
                if found["coverage"]["semantic_search"] != "unavailable" || found["truncated"] == true {
                    return Err("baseline_must_be_complete_and_lexical_only".into());
                }
                let returned: Result<Vec<_>, _> = found["results"].as_array().ok_or("baseline_invalid")?.iter()
                    .map(|row| references.get(row["ref"].as_str().unwrap_or("")).cloned().ok_or("unknown_source")).collect();
                rows.push(json!({"id":query["id"], "category":query["category"], "expected_sources":query["expected_sources"],
                    "obsolete_or_conflicting_sources":query.get("obsolete_or_conflicting_sources").cloned().unwrap_or(json!([])),
                    "returned_sources":returned?, "mode":mode}));
            }
            baselines.insert(policy, rows);
        }
        let index = LocalEvents::configure(&vault, "personal", Setup {
            endpoint:config["endpoint"].as_str().ok_or("endpoint_invalid")?.into(),
            model:config["model"].as_str().ok_or("model_invalid")?.into(),
            revision:config["space_revision"].as_str().ok_or("revision_invalid")?.into(),
            tokenizer:config["tokenizer_identity"].as_str().ok_or("tokenizer_invalid")?.into(),
            dimensions:512, query_instruction:config["query_instruction"].as_str().ok_or("prefix_invalid")?.into(),
            document_instruction:String::new(), events_approved:true, attachments_approved:false, query_approved:true,
            chunk_bytes:1024, disk_cap_mib:16,
        }, config["python"].as_str().ok_or("python_invalid")?.into(),
        config["python_path"].as_str().ok_or("python_path_invalid")?.into())?;
        let indexing = Instant::now();
        let mut complete = false;
        let mut steps = 0;
        for _ in 0..16 {
            steps += 1;
            if index.refresh_step()?["complete"] == true { complete = true; break; }
        }
        if !complete { return Err("bounded_indexing_incomplete".into()); }
        let indexing_seconds = indexing.elapsed().as_secs_f64();
        let mut rows = Vec::new();
        let mut fetch_count = 0;
        for query in queries {
            let started = Instant::now();
            let found = transport::invoke(&context, "recall_search", json!({"query":query["query"]}))?;
            let elapsed = started.elapsed().as_secs_f64();
            if found["coverage"]["local_event_semantic"] != true || found["coverage"]["cloud_fallback"] != false
                || found["coverage"]["scopes"][0]["state"] != "ready" {
                return Err("real_semantic_route_unavailable".into());
            }
            let candidates = found["results"].as_array().ok_or("semantic_results_invalid")?;
            let mut returned = Vec::new();
            let mut scores = Vec::new();
            for row in candidates {
                let reference = row["ref"].as_str().ok_or("source_invalid")?;
                returned.push(references.get(reference).cloned().ok_or("unknown_semantic_source")?);
                scores.push(row["score"].clone());
                let fetched = transport::invoke(&context, "recall_fetch", json!({"id":row["id"]}))?;
                if fetched["results"][0]["text"] != *originals.get(reference).ok_or("missing_original")?
                    || fetched["results"][0]["snapshot"] != row["snapshot"] || fetched["truncated"] != false {
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
        let repeat = transport::invoke(&context, "recall_search", json!({"query":queries.last().ok_or("queries_empty")?["query"]}))?;
        if repeat["coverage"]["scopes"][0]["query_encoding"] != "cache" { return Err("repeat_query_cache_missing".into()); }
        all.push(json!({"name":dataset["name"], "sources":corpus.len(), "queries":queries.len(),
            "baselines":baselines, "semantic":rows, "indexing_seconds":indexing_seconds, "index_steps":steps,
            "verified_fetches":fetch_count, "repeated_query_cache_verified":true, "memory_count":vault.memories()?.len()}));
    }
    Ok(json!({"datasets":all}))
}

fn main() {
    let result = std::env::args().nth(1).ok_or("config_required".to_owned())
        .and_then(|path| fs::read(path).map_err(|_| "config_unreadable".into()))
        .and_then(|bytes| serde_json::from_slice(&bytes).map_err(|_| "config_invalid".into()))
        .and_then(run);
    match result {
        Ok(value) => println!("{}", serde_json::to_string(&value).expect("synthetic JSON serializes")),
        Err(_) => { eprintln!("真实编码链验收失败；未上传原文、配置或进程错误"); std::process::exit(1); }
    }
}
