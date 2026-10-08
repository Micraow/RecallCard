//! 合成资料验证长正文能被检索、定位并完整续读；没有真实账户或外部模型调用。
use recallcard::{
    application::connections::{self, ConnectionGrant},
    context::{Context, ReadPageArgs, SearchArgs},
    policy::Access,
    transport, EventInput, MemoryInput, Vault,
};
use serde_json::{json, Value};
fn setup() -> (tempfile::TempDir, Vault) {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(&d.path().join("vault")).unwrap();
    (d, v)
}
fn capture(v: &Vault, scope: &str, id: &str, text: &str) -> String {
    let input: EventInput = serde_json::from_value(json!({"occurred_at":"2026-10-06T10:00:00Z","role":"user","origin":"native","scope":scope,"content":text,"source":{"platform":"synthetic","conversation_id":"paging","message_id":id}})).unwrap();
    format!("event:{}", v.capture(input).unwrap().id)
}
fn context(v: &Vault) -> Context<'_> {
    Context::new(v, Access::new(vec!["personal".into()]).unwrap())
}
fn page(c: &Context<'_>, args: Value) -> Value {
    let response = c
        .read_page(serde_json::from_value::<ReadPageArgs>(args.clone()).unwrap())
        .unwrap();
    assert!(
        serde_json::to_vec(&response).unwrap().len()
            <= args["budget_bytes"].as_u64().unwrap_or(1500) as usize,
        "{response}"
    );
    response
}
#[test]
fn long_tail_hit_is_locatable_and_complete_text_is_recoverable() {
    let (_d, v) = setup();
    let text = format!(
        "{}\nORION_HANDOFF: TAIL_EVIDENCE_7843。最终核验完成。",
        "合成无关行🙂\\\"\n".repeat(9000)
    );
    assert!(text.len() > 32768);
    let reference = capture(&v, "personal", "long", &text);
    let c = context(&v);
    let result = c
        .search(
            serde_json::from_value::<SearchArgs>(
                json!({"query":"ORION_HANDOFF","budget_bytes":4000}),
            )
            .unwrap(),
        )
        .unwrap();
    let hit = &result["results"][0];
    assert_eq!(hit["ref"], reference);
    assert!(hit["text"].as_str().unwrap().contains("TAIL_EVIDENCE_7843"));
    assert_eq!(hit["text_truncated"], true);
    let start = hit["text_range"]["start_byte"].as_u64().unwrap() as usize;
    let end = hit["text_range"]["end_byte"].as_u64().unwrap() as usize;
    assert_eq!(hit["text"], &text[start..end]);
    let tail = page(
        &c,
        json!({"refs":[reference],"offset_bytes":start,"budget_bytes":4000}),
    );
    assert!(tail["results"][0]["text"]
        .as_str()
        .unwrap()
        .contains("TAIL_EVIDENCE_7843"));
    assert_eq!(tail["results"][0]["metadata"]["role"], "user");
    let mut args = json!({"refs":[reference],"budget_bytes":4096});
    let mut recovered = String::new();
    let mut snapshot = Value::Null;
    loop {
        let r = page(&c, args.clone());
        let fragment = &r["results"][0];
        assert_eq!(
            fragment["text_range"]["start_byte"].as_u64().unwrap() as usize,
            recovered.len()
        );
        if snapshot.is_null() {
            snapshot = fragment["snapshot"].clone();
        }
        assert_eq!(fragment["snapshot"], snapshot);
        recovered.push_str(fragment["text"].as_str().unwrap());
        assert_eq!(
            fragment["text_range"]["end_byte"].as_u64().unwrap() as usize,
            recovered.len()
        );
        if r["next_cursor"].is_null() {
            assert_eq!(r["truncated"], false);
            break;
        }
        assert_eq!(r["truncated"], true);
        args["cursor"] = r["next_cursor"].clone();
    }
    assert_eq!(recovered, text);
}
#[test]
fn budget_empty_is_different_from_no_match_and_brief_marks_clipping() {
    let (_d, v) = setup();
    let reference = capture(
        &v,
        "personal",
        "long",
        &format!("{} KEYWORD", "填充".repeat(20000)),
    );
    let c = context(&v);
    let query = |q, budget| {
        c.search(
            serde_json::from_value(json!({"query":q,"detail":"brief","budget_bytes":budget}))
                .unwrap(),
        )
        .unwrap()
    };
    assert_eq!(query("KEYWORD", 512)["status"], "budget_exhausted");
    assert_eq!(query("NOT_PRESENT", 512)["status"], "no_matches");
    let r = query("KEYWORD", 3000);
    assert_eq!(r["results"][0]["text_truncated"], true);
    assert!(r["results"][0]["text"]
        .as_str()
        .unwrap()
        .contains("KEYWORD"));
    let r = page(&c, json!({"refs":[reference],"budget_bytes":512}));
    assert_eq!(r["status"], "budget_exhausted");
    assert!(!r["pending_refs"].as_array().unwrap().is_empty());
}
#[test]
fn cursor_is_bound_to_ref_scope_and_snapshot_and_rechecks_suppression() {
    let (_d, v) = setup();
    let a = capture(&v, "personal", "a", &"页面中文🙂".repeat(10000));
    let b = capture(&v, "personal", "b", &"另一个正文".repeat(10000));
    let c = context(&v);
    let r = page(&c, json!({"refs":[a],"budget_bytes":2500}));
    let cursor = r["next_cursor"].clone();
    let args = |reference: &str| {
        serde_json::from_value(json!({"refs":[reference],"cursor":cursor,"budget_bytes":2500}))
            .unwrap()
    };
    assert!(c.read_page(args(&b)).is_err());
    let wider = Context::new(
        &v,
        Access::new(vec!["personal".into(), "project:other".into()]).unwrap(),
    );
    assert!(wider.read_page(args(&a)).is_err());
    v.suppress(a.strip_prefix("event:").unwrap(), "合成撤回".into())
        .unwrap();
    assert!(c.read_page(args(&a)).is_err());
    assert!(c
        .read_page(serde_json::from_value(json!({"refs":[b],"offset_bytes":1})).unwrap())
        .is_err());
    assert!(c
        .read_page(
            serde_json::from_value(json!({"refs":[b],"offset_bytes":0,"cursor":cursor})).unwrap()
        )
        .is_err());
}
#[test]
fn memory_revision_invalidates_cursor_and_sources_offer_readable_refs() {
    let (_d, v) = setup();
    let event = capture(&v, "personal", "source", &"很长的原始证据".repeat(10000));
    let input: MemoryInput = serde_json::from_value(json!({"content":"长记忆".repeat(5000),"source_refs":[event.strip_prefix("event:").unwrap()],"evidence":"user_explicit"})).unwrap();
    let memory = v.add_memory(input).unwrap();
    let reference = format!("memory:{}", memory.id);
    let c = context(&v);
    let r = page(&c, json!({"refs":[reference],"budget_bytes":3000}));
    let sources = c
        .sources_page(
            serde_json::from_value(json!({"refs":[reference],"budget_bytes":3000})).unwrap(),
        )
        .unwrap();
    assert_eq!(sources["results"][0]["source_refs"][0], event);
    assert_eq!(sources["status"], "source_refs");
    assert!(
        !page(&c, json!({"refs":[event],"budget_bytes":3000}))["results"][0]["text"]
            .as_str()
            .unwrap()
            .is_empty()
    );
    let mut changed = memory.data;
    changed.content.push_str("更新");
    v.update_memory(&memory.id, 1, changed).unwrap();
    assert!(c
        .read_page(
            serde_json::from_value(
                json!({"refs":[reference],"cursor":r["next_cursor"],"budget_bytes":3000})
            )
            .unwrap()
        )
        .is_err());
}
#[test]
fn managed_read_cursor_does_not_outlive_revocation_and_schema_names_bytes() {
    let (_d, v) = setup();
    let reference = capture(&v, "personal", "long", &"合成用户原话".repeat(10000));
    let grant = ConnectionGrant {
        client_kind: "chatgpt_mcp".into(),
        host_identity: "openai-chatgpt".into(),
        installation_id: None,
        platform: "chatgpt".into(),
        recall_scopes: vec!["personal".into()],
        capture_scopes: vec![],
        provider_disclosure: true,
        auto_recall: true,
        auto_capture: false,
    };
    let e = connections::configure(&v, grant, None).unwrap();
    let access = Access::new(vec!["personal".into()]).unwrap();
    let r = connections::invoke_agent(
        &v,
        &e.id,
        &access,
        "read",
        json!({"refs":[reference],"budget_bytes":3000}),
    )
    .unwrap();
    assert!(r["next_cursor"].is_string());
    connections::revoke(&v, &e.id, e.permission_revision).unwrap();
    assert!(connections::invoke_agent(
        &v,
        &e.id,
        &access,
        "read",
        json!({"refs":[reference],"cursor":r["next_cursor"],"budget_bytes":3000})
    )
    .is_err());
    for definition in transport::tool_definitions().as_array().unwrap() {
        assert!(definition["inputSchema"]["properties"]["budget_bytes"].is_object());
        assert_eq!(
            definition["inputSchema"]["properties"]["budget_tokens"]["deprecated"],
            true
        );
    }
    assert!(transport::invoke(
        &context(&v),
        "read",
        json!({"refs":[reference],"budget_tokens":3000})
    )
    .is_ok());
}

#[test]
fn reopened_vault_keeps_cursor_and_partial_branch_capture_metadata() {
    let (d, v) = setup();
    let input: EventInput = serde_json::from_value(json!({"role":"assistant","origin":"assistant_output","content":"合成分支正文🙂".repeat(10000),"metadata":{"chatgpt":{"on_current_path":false}},"capture":{"completeness":"visible_only","redacted":true,"redaction_count":2},"source":{"platform":"chatgpt","conversation_id":"synthetic","message_id":"branch"}})).unwrap();
    let reference = format!("event:{}", v.capture(input).unwrap().id);
    let first = page(
        &context(&v),
        json!({"refs":[reference],"budget_bytes":3000}),
    );
    let metadata = &first["results"][0]["metadata"];
    assert_eq!(metadata["chatgpt"]["on_current_path"], false);
    assert_eq!(metadata["capture"]["completeness"], "visible_only");
    assert_eq!(metadata["capture"]["redacted"], true);
    assert_eq!(metadata["capture"]["redaction_count"], 2);
    drop(v);
    let reopened = Vault::open_existing(&d.path().join("vault")).unwrap();
    let next = page(
        &context(&reopened),
        json!({"refs":[reference],"cursor":first["next_cursor"],"budget_bytes":3000}),
    );
    assert_eq!(
        next["results"][0]["text_range"]["start_byte"],
        first["results"][0]["text_range"]["end_byte"]
    );
    for budget in [0, 511, 32769] {
        assert!(transport::invoke(
            &context(&reopened),
            "read",
            json!({"refs":[reference],"budget_bytes":budget})
        )
        .is_err());
    }
    assert!(transport::invoke(
        &context(&reopened),
        "read",
        json!({"refs":[reference],"budget_bytes":3000,"budget_tokens":3000})
    )
    .is_err());
}
#[test]
fn source_reference_pages_are_bounded_complete_and_cannot_be_text_cursors() {
    let (_d, v) = setup();
    let refs = (0..25)
        .map(|i| capture(&v, "personal", &i.to_string(), "合成原始证据"))
        .collect::<Vec<_>>();
    let memory = v.add_memory(serde_json::from_value(json!({"content":"来源集合摘要","source_refs":refs.iter().map(|r|r.strip_prefix("event:").unwrap()).collect::<Vec<_>>(),"evidence":"user_explicit"})).unwrap()).unwrap();
    let reference = format!("memory:{}@1", memory.id);
    let c = context(&v);
    let mut args = json!({"refs":[reference],"budget_bytes":1200});
    let mut recovered = Vec::<String>::new();
    loop {
        let result = c
            .sources_page(serde_json::from_value(args.clone()).unwrap())
            .unwrap();
        assert!(serde_json::to_vec(&result).unwrap().len() <= 1200);
        recovered.extend(
            result["results"][0]["source_refs"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap().to_owned()),
        );
        if result["next_cursor"].is_null() {
            break;
        }
        args["cursor"] = result["next_cursor"].clone();
        assert!(c
            .read_page(serde_json::from_value(args.clone()).unwrap())
            .is_err());
    }
    assert_eq!(recovered, refs);
}
