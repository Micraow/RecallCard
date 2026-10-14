//! 原创合成端到端消融：生产分块、worker、RRF、正本校验及共享响应预算。
//! 不接收真实资料库；调用方提供的每次输出目录必须全新。
use recallcard::{
    application::{ImportRequest, ImportService, JobState},
    context::{Context, SearchArgs},
    local_events::{LocalEvents, Setup},
    policy::Access,
    transport, EventInput, Vault,
};
use serde_json::{json, Value};
use std::{collections::BTreeMap, fs, path::PathBuf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 7 {
        return Err(
            "需要合成fixture、全新目录、临时worker目录、回环endpoint、chunk字节和合成附件ZIP"
                .into(),
        );
    }
    let fixture: Value = serde_json::from_slice(&fs::read(&args[1])?)?;
    if fixture["schema"] != "recallcard.synthetic-lexical-pipeline/1"
        || fixture["private_data"] != false
    {
        return Err("仅接受明确的合成pipeline fixture".into());
    }
    let root = PathBuf::from(&args[2]);
    if !root.is_absolute() || root.exists() {
        return Err("使用全新绝对路径；不能覆盖已有资料库".into());
    }
    fs::create_dir_all(&root)?;
    let vault = Vault::init(&root.join("vault"))?;
    let docs = fixture["documents"].as_array().ok_or("缺少合成文档")?;
    let queries = fixture["queries"].as_array().ok_or("缺少合成问题")?;
    if docs.len() > 1000 || queries.len() > 100 {
        return Err("合成fixture超出本次有界评测范围".into());
    }
    let inputs: Vec<EventInput> = docs
        .iter()
        .map(|d| {
            serde_json::from_value(json!({"scope":d["scope"],"role":d.get("role").unwrap_or(&json!("user")),
                "origin":if d["role"]=="assistant"{"assistant_output"}else{"user_input"},"content":d["text"],
                "source":{"platform":"synthetic-pipeline","conversation_id":"fixed-synthetic-fixture",
                    "message_id":d.get("replaces").unwrap_or(&d["label"])}}))
        })
        .collect::<Result<_, _>>()?;
    let events = vault.capture_batch(inputs)?;
    let mut references = BTreeMap::new();
    for (d, e) in docs.iter().zip(&events) {
        references.insert(
            d["id"].as_str().ok_or("缺少合成标识")?,
            format!("event:{}", e.id),
        );
        if d["active"] == false {
            vault.suppress(&e.id, "合成撤销用例".into())?;
        }
    }
    let importer = ImportService::new(&vault).map_err(|e| format!("{e:?}"))?;
    let job = importer
        .submit(ImportRequest {
            request_id: "synthetic-attachment".into(),
            paths: vec![PathBuf::from(&args[6])],
            scope: "personal".into(),
            format: "auto".into(),
        })
        .map_err(|e| format!("{e:?}"))?;
    let imported = importer
        .run(&job.job_id, "personal")
        .map_err(|e| format!("{e:?}"))?;
    if imported.state != JobState::Completed {
        return Err("合成附件导入未完成".into());
    }
    let attachment = vault
        .events()?
        .into_iter()
        .find(|e| e.data.source.message_id == "pipeline-upload")
        .ok_or("合成附件事件缺失")?;
    let context = Context::new(&vault, Access::new(vec!["personal".into()])?);
    let search_args = |query: &str| -> Result<SearchArgs, serde_json::Error> {
        serde_json::from_value(json!({"query":query,"target":"events","budget_tokens":12000}))
    };
    let name_search = context.search(search_args("pipeline-attachment-note.txt")?)?;
    let body_search = context.search(search_args("ATTACHMENT_BODY_ONLY_834")?)?;
    let index = LocalEvents::configure(
        &vault,
        "personal",
        Setup {
            endpoint: args[4].clone(),
            model: "synthetic-pipeline-fixed-vectors".into(),
            revision: "contract-v1".into(),
            tokenizer: "no-real-model".into(),
            dimensions: 2,
            query_instruction: "synthetic-query".into(),
            document_instruction: "synthetic-corpus".into(),
            events_approved: true,
            attachments_approved: false,
            query_approved: true,
            chunk_bytes: args[5].parse()?,
            disk_cap_mib: 32,
        },
        "python3".into(),
        PathBuf::from(&args[3]).canonicalize()?,
    )?;
    let mut complete = false;
    for _ in 0..4096 {
        if index.refresh_step()?["complete"] == true {
            complete = true;
            break;
        }
    }
    if !complete {
        return Err("合成索引未在有界步骤内完成".into());
    }
    let status = index.status()?;
    let mut pages = Vec::new();
    for q in queries {
        let query = q["text"].as_str().ok_or("问题无效")?;
        let host = index.search(query, 20)?;
        let shared = context.recall_question(query, "events")?;
        if serde_json::to_vec(&shared)?.len() > 8192 {
            return Err("共享响应超过固定预算".into());
        }
        let mut fetch = Value::Null;
        if let Some(id) = shared["results"]
            .as_array()
            .and_then(|rows| rows.first())
            .and_then(|r| r["id"].as_str())
        {
            fetch = transport::invoke(&context, "recall_fetch", json!({"id":id}))?;
        }
        pages.push(json!({"query":q["id"],"host":host,"shared":shared,"first_fetch":fetch}));
    }
    let attachment_ref = format!("event:{}", attachment.id);
    let attachment_pipeline = context.recall_question("ATTACHMENT_BODY_ONLY_834", "events")?;
    let attachment_row = attachment_pipeline["results"]
        .as_array()
        .and_then(|rows| rows.iter().find(|r| r["kind"] == "attachment"))
        .ok_or("附件正文未进入共享词法候选")?;
    let attachment_id = attachment_row["id"]
        .as_str()
        .ok_or("附件缺少读取定位符")?
        .to_owned();
    let attachment_fetch =
        transport::invoke(&context, "recall_fetch", json!({"id":attachment_id}))?;
    let attachment_not_in_embedding = !index.search("ATTACHMENT_BODY_ONLY_834", 20)?["results"]
        .as_array()
        .ok_or("缺少结果")?
        .iter()
        .any(|r| r["ref"] == attachment_ref);
    let saved = pages
        .iter()
        .find_map(|p| {
            p["shared"]["results"]
                .as_array()
                .and_then(|r| r.first())
                .and_then(|r| r["id"].as_str())
        })
        .map(str::to_owned);
    LocalEvents::disable(&vault, "personal")?;
    let revoked_search = index.search("E_PIPE_71", 10).is_err();
    let revoked_fetch = saved
        .is_some_and(|id| transport::invoke(&context, "recall_fetch", json!({"id":id})).is_err());
    vault.suppress(&attachment.id, "合成附件撤回".into())?;
    let revoked_attachment_fetch =
        transport::invoke(&context, "recall_fetch", json!({"id":attachment_id})).is_err();
    let revoked_attachment_search = context
        .recall_question("ATTACHMENT_BODY_ONLY_834", "events")?["results"]
        .as_array()
        .ok_or("附件撤回结果无效")?
        .is_empty();
    let compiled_source_sha256 = json!({
        "crates/recallcard/src/context/attachments.rs": recallcard::hash(include_bytes!("../src/context/attachments.rs")),
        "crates/recallcard/src/context.rs": recallcard::hash(include_bytes!("../src/context.rs")),
        "crates/recallcard/src/context/search.rs": recallcard::hash(include_bytes!("../src/context/search.rs")),
        "crates/recallcard/src/context/paging.rs": recallcard::hash(include_bytes!("../src/context/paging.rs")),
        "crates/recallcard/src/local_events/snapshot.rs": recallcard::hash(include_bytes!("../src/local_events/snapshot.rs")),
        "crates/recallcard/src/local_events/ranking.rs": recallcard::hash(include_bytes!("../src/local_events/ranking.rs")),
        "crates/recallcard/src/local_events/lexical_context.rs": recallcard::hash(include_bytes!("../src/local_events/lexical_context.rs")),
        "crates/recallcard/src/local_events.rs": recallcard::hash(include_bytes!("../src/local_events.rs")),
        "crates/recallcard/src/context/recall.rs": recallcard::hash(include_bytes!("../src/context/recall.rs")),
    });
    let report = json!({"schema":"recallcard.synthetic-pipeline-host/1","compiled_source_sha256":compiled_source_sha256,"references":references,"status":status,
        "pages":pages,"revoked_search_rejected":revoked_search,"revoked_fetch_rejected":revoked_fetch,
        "attachment":{"reference":attachment_ref,"event_text":attachment.data.text(),"name_search":name_search,"body_search":body_search,"excluded_from_event_embeddings":attachment_not_in_embedding,"shared_body_search":attachment_pipeline,"body_fetch":attachment_fetch,"suppressed_fetch_rejected":revoked_attachment_fetch,"suppressed_search_empty":revoked_attachment_search}});
    fs::write(root.join("host.json"), serde_json::to_vec_pretty(&report)?)?;
    println!(
        "合成pipeline完成：{}个问题，{}字节分块",
        queries.len(),
        args[5]
    );
    Ok(())
}
