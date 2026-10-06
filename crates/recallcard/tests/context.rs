use recallcard::{
    context::{tokenize, BootstrapArgs, Context, ReadArgs, SearchArgs},
    policy::Access,
    transport, EventInput, MemoryInput, Vault,
};
use serde_json::{json, Value};
use tempfile::TempDir;
fn setup() -> (TempDir, Vault) {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(&d.path().join("vault")).unwrap();
    (d, v)
}
fn capture(v: &Vault, scope: &str, id: &str, text: &str) -> String {
    let input:EventInput=serde_json::from_value(json!({"occurred_at":"2026-10-06T10:00:00Z","role":"user","origin":"native","scope":scope,"content":text,"source":{"platform":"manual-web","conversation_id":"demo","message_id":id}})).unwrap();
    v.capture(input).unwrap().id
}
fn search(query: &str) -> SearchArgs {
    serde_json::from_value(json!({"query":query,"budget_tokens":12000})).unwrap()
}
fn context(v: &Vault) -> Context<'_> {
    Context::new(v, Access::new(vec!["personal".into()]).unwrap())
}
#[test]
fn undreamed_event_is_immediately_searchable() {
    let (_d, v) = setup();
    let id = capture(&v, "personal", "a", "编程语言决定使用 Rust，原因是内存安全");
    let r = context(&v).search(search("编程语言")).unwrap();
    assert_eq!(r["results"][0]["ref"], format!("event:{id}"));
    assert_eq!(r["coverage"]["undreamed_events_included"], true);
}
#[test]
fn chinese_tokenizer_keeps_bigrams_and_ascii_paths() {
    let terms = tokenize("中文检索 src/main.rs Qwen-3");
    assert!(terms.contains(&"中文".into()));
    assert!(terms.contains(&"检索".into()));
    assert!(terms.contains(&"src/main.rs".into()));
    assert!(terms.contains(&"qwen-3".into()));
}
#[test]
fn authorization_filters_search_read_sources_and_bootstrap() {
    let (_d, v) = setup();
    let hidden = capture(&v, "project:secret", "a", "隐藏专用内容");
    capture(&v, "personal", "b", "普通公开范围");
    let c = context(&v);
    assert!(c.search(search("隐藏")).unwrap()["results"]
        .as_array()
        .unwrap()
        .is_empty());
    assert!(c
        .read(ReadArgs {
            refs: vec![format!("event:{hidden}")],
            budget_tokens: 12000
        })
        .is_err());
    assert!(!c
        .bootstrap(BootstrapArgs {
            budget_tokens: 12000
        })
        .unwrap()
        .to_string()
        .contains("隐藏"));
}
#[test]
fn unknown_scope_parameter_cannot_expand_tool_access() {
    let (_d, v) = setup();
    capture(&v, "project:secret", "a", "隐藏专用内容");
    assert!(transport::invoke(
        &context(&v),
        "search",
        json!({"query":"隐藏","scope":"project:secret"})
    )
    .is_err());
}
#[test]
fn latest_revision_is_searchable_old_evidence_remains_readable() {
    let (_d, v) = setup();
    let old = capture(&v, "personal", "a", "决定使用旧方案");
    let new = capture(&v, "personal", "a", "决定使用新方案");
    let c = context(&v);
    let r = c.search(search("决定")).unwrap();
    assert_eq!(r["results"].as_array().unwrap().len(), 1);
    assert_eq!(r["results"][0]["ref"], format!("event:{new}"));
    assert!(c
        .read(ReadArgs {
            refs: vec![format!("event:{old}")],
            budget_tokens: 12000
        })
        .is_ok());
}
#[test]
fn suppression_prevents_recall_until_explicit_restore() {
    let (_d, v) = setup();
    let id = capture(&v, "personal", "a", "应该遗忘的合成内容");
    v.suppress(&id, "测试遗忘".into()).unwrap();
    assert!(context(&v).search(search("合成")).unwrap()["results"]
        .as_array()
        .unwrap()
        .is_empty());
    assert!(context(&v)
        .read(ReadArgs {
            refs: vec![id.clone()],
            budget_tokens: 12000
        })
        .is_err());
    v.restore(&id).unwrap();
    assert_eq!(
        context(&v).search(search("合成")).unwrap()["results"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}
#[test]
fn pinned_bootstrap_is_stable_and_does_not_promote_unpinned_text() {
    let (_d, v) = setup();
    let id = capture(&v, "personal", "a", "文档使用中文");
    let input:MemoryInput=serde_json::from_value(json!({"content":"文档使用中文","source_refs":[id],"evidence":"user_explicit","labels":["bootstrap"]})).unwrap();
    v.add_memory(input).unwrap();
    let c = context(&v);
    let a = c
        .bootstrap(BootstrapArgs {
            budget_tokens: 12000,
        })
        .unwrap();
    let b = c
        .bootstrap(BootstrapArgs {
            budget_tokens: 12000,
        })
        .unwrap();
    assert_eq!(a, b);
    assert!(a["stable_text"].as_str().unwrap().contains("文档使用中文"));
}
#[test]
fn suppression_of_memory_also_suppresses_its_sources() {
    let (_d, v) = setup();
    let id = capture(&v, "personal", "a", "文档使用中文");
    let input:MemoryInput=serde_json::from_value(json!({"content":"文档使用中文","source_refs":[id.clone()],"evidence":"user_explicit","labels":["bootstrap"]})).unwrap();
    let m = v.add_memory(input).unwrap();
    v.suppress(&m.id, "测试".into()).unwrap();
    assert!(context(&v).search(search("中文")).unwrap()["results"]
        .as_array()
        .unwrap()
        .is_empty());
    assert!(!context(&v)
        .bootstrap(BootstrapArgs {
            budget_tokens: 12000
        })
        .unwrap()["stable_text"]
        .as_str()
        .unwrap()
        .contains("文档使用中文"));
}
#[test]
fn cursor_is_invalidated_by_new_data_and_scope() {
    let (_d, v) = setup();
    for i in 0..3 {
        capture(&v, "personal", &i.to_string(), "相同查询内容");
    }
    let mut args = search("相同");
    args.limit = 1;
    let r = context(&v).search(args.clone()).unwrap();
    args.cursor = r["next_cursor"].as_str().map(str::to_owned);
    assert!(args.cursor.is_some());
    assert_eq!(
        context(&v).search(args.clone()).unwrap()["results"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    capture(&v, "personal", "next", "相同新内容");
    assert!(context(&v).search(args).is_err());
}
#[test]
fn temporal_filters_do_not_rewrite_unknown_time() {
    let (_d, v) = setup();
    capture(&v, "personal", "a", "未来记录");
    let mut args = search("未来");
    args.as_of = Some("2025-01-01T00:00:00Z".parse().unwrap());
    assert!(context(&v).search(args).unwrap()["results"]
        .as_array()
        .unwrap()
        .is_empty());
}
#[test]
fn mcp_has_exactly_four_readonly_tools_and_no_mutation_route() {
    let (_d, v) = setup();
    capture(&v, "personal", "a", "跨客户端原始决定");
    let requests = [
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"合成测试","version":"1"}}}),
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
        json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"search","arguments":{"query":"跨客户端","budget_tokens":12000}}}),
        json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"capture","arguments":{}}}),
    ];
    let data = requests
        .iter()
        .map(|v| format!("{v}\n"))
        .collect::<String>();
    let mut output = Vec::new();
    transport::serve_mcp_io(
        &v,
        Access::new(vec!["personal".into()]).unwrap(),
        std::io::Cursor::new(data),
        &mut output,
    )
    .unwrap();
    let responses = String::from_utf8(output)
        .unwrap()
        .lines()
        .map(|s| serde_json::from_str::<Value>(s).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(responses.len(), 4);
    assert_eq!(responses[1]["result"]["tools"].as_array().unwrap().len(), 4);
    assert_eq!(responses[2]["result"]["isError"], false);
    assert!(responses[2]["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("跨客户端"));
    assert_eq!(responses[3]["result"]["isError"], true);
    assert_eq!(v.events().unwrap().len(), 1);
}
#[test]
fn malformed_and_oversized_transport_fails_closed() {
    let (_d, v) = setup();
    let mut out = Vec::new();
    let input = "x".repeat(1024 * 1024 + 1) + "\n";
    assert!(transport::serve_mcp_io(
        &v,
        Access::new(vec!["personal".into()]).unwrap(),
        std::io::Cursor::new(input),
        &mut out
    )
    .is_err());
    assert!(out.is_empty());
}
