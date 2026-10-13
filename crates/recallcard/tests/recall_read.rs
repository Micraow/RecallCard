//! 简洁读取入口的离线边界；这里只使用合成正本，不启用编码器。
use recallcard::{context::Context, policy::Access, transport, Vault};
use serde_json::{json, Value};
fn invoke(v: &Vault, name: &str, args: Value) -> Result<Value, String> {
    transport::invoke(
        &Context::new(v, Access::new(vec!["personal".into()]).unwrap()),
        name,
        args,
    )
}
fn event(v: &Vault) -> String {
    v.capture(serde_json::from_value(json!({"scope":"personal","role":"user","origin":"user_input","content":"合成来源","source":{"platform":"synthetic","conversation_id":"simple-read","message_id":"one"}})).unwrap()).unwrap().id
}
#[test]
fn unconfigured_is_honest_and_strict_arguments_cannot_select_provider_or_scope() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(&d.path().join("vault")).unwrap();
    event(&v);
    let r = invoke(&v, "recall_search", json!({"query":"合成"})).unwrap();
    assert_eq!(r["status"], "unavailable");
    assert_eq!(r["coverage"]["cloud_fallback"], false);
    assert!(r["results"].as_array().unwrap().is_empty());
    for extra in [
        json!({"scope":"personal"}),
        json!({"endpoint":"http://example.com"}),
        json!({"budget_bytes":9000}),
        json!({"target":"all"}),
    ] {
        let mut args = json!({"query":"合成"});
        args.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        assert!(invoke(&v, "recall_search", args).is_err());
    }
    for id in ["S1", "../../secret", "event:evt_fake", "rcs1|x"] {
        assert!(invoke(&v, "recall_fetch", json!({"id":id})).is_err());
    }
    let access = Access::new((0..5).map(|n| format!("project:p{n}")).collect()).unwrap();
    let r = transport::invoke(
        &Context::new(&v, access),
        "recall_search",
        json!({"query":"合成"}),
    )
    .unwrap();
    assert_eq!(r["status"], "scope_limit");
}
#[test]
fn memory_locator_full_snapshot_budget_and_sources_remain_bound() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(&d.path().join("vault")).unwrap();
    let source = event(&v);
    let m=v.add_memory(serde_json::from_value(json!({"content":"合成计划：".to_owned()+&"中文🙂引用\\\" ".repeat(1000),"source_refs":[source],"scope":"personal","evidence":"user_explicit"})).unwrap()).unwrap();
    let page = invoke(&v, "recall_search", json!({"query":"合成计划"})).unwrap();
    assert!(serde_json::to_vec(&page).unwrap().len() <= 8192, "{page}");
    let row = &page["results"][0];
    assert_eq!(row["kind"], "memory");
    assert_eq!(row["ref"], format!("memory:{}@{}", m.id, m.revision));
    let id = row["id"].as_str().unwrap();
    let got = invoke(&v, "recall_fetch", json!({"id":id})).unwrap();
    assert_eq!(got["results"][0]["snapshot"], row["snapshot"]);
    assert!(serde_json::to_vec(&got).unwrap().len() <= 8192);
    assert_eq!(got, invoke(&v, "recall_fetch", json!({"id":id})).unwrap());
    assert!(got["next_id"].is_string());
    let next = invoke(&v, "recall_fetch", json!({"id":got["next_id"]})).unwrap();
    assert_eq!(
        next["results"][0]["text_range"]["start_byte"],
        got["results"][0]["text_range"]["end_byte"]
    );
    let wide = Context::new(
        &v,
        Access::new(vec!["personal".into(), "project:x".into()]).unwrap(),
    );
    assert!(transport::invoke(&wide, "recall_fetch", json!({"id":id})).is_err());
    v.suppress(&source, "合成撤回".into()).unwrap();
    assert!(invoke(&v, "recall_fetch", json!({"id":id})).is_err());
}

#[test]
fn escaped_memory_metadata_budget_is_not_reported_as_unavailable() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(&d.path().join("vault")).unwrap();
    let source = event(&v);
    v.add_memory(serde_json::from_value(json!({"content":"合成预算测试","source_refs":[source],"scope":"personal","evidence":"user_explicit","time_note":"\u{0000}".repeat(256)})).unwrap()).unwrap();
    let context = Context::new(&v, Access::new(vec!["personal".into()]).unwrap());
    let page = context.recall_question("合成预算测试", "memories").unwrap();
    assert_eq!(page["status"], "budget_exhausted", "{page}");
    assert_eq!(page["coverage"]["memory_status"], "budget_exhausted");
    assert!(serde_json::to_vec(&page).unwrap().len() <= 8192);
}

#[test]
fn mcp_initialize_uses_shared_short_semantics_without_web_ticket_requirements() {
    let spec: Value =
        serde_json::from_str(include_str!("../src/recall-capabilities.json")).unwrap();
    let input = concat!(
        "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2025-11-25\"}}\n",
        "{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n",
        "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\"}\n"
    );
    let mut output = Vec::new();
    transport::serve_mcp_service_io(
        |_, _| panic!("握手和列工具不能读取用户资料"),
        input.as_bytes(),
        &mut output,
    )
    .unwrap();
    let messages: Vec<Value> = String::from_utf8(output)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(messages.len(), 2);
    let instructions = messages[0]["result"]["instructions"].as_str().unwrap();
    assert_eq!(instructions, transport::recall_instructions());
    assert!(instructions.contains(spec["common"].as_str().unwrap()));
    assert!(instructions.contains("bootstrap"));
    assert!(instructions.contains("recall_search(query)"));
    assert!(instructions.contains("recall_fetch(id)"));
    assert!(instructions.contains("当前 MCP 使用完整定位符"));
    assert!(!instructions.contains("RECALLCARD/2"));
    assert!(instructions.chars().count() < 500);
    let tools = messages[1]["result"]["tools"].as_array().unwrap();
    for name in [
        "bootstrap",
        "search",
        "read",
        "sources",
        "recall_search",
        "recall_fetch",
    ] {
        assert!(tools.iter().any(|tool| tool["name"] == name));
    }
}
