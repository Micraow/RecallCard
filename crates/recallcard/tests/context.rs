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

#[test]
fn complete_responses_respect_server_byte_budgets() {
    let (_d, v) = setup();
    for i in 0..8 {
        let id = capture(
            &v,
            "personal",
            &i.to_string(),
            &"合成中文预算测试。".repeat(150),
        );
        let input:MemoryInput=serde_json::from_value(json!({"content":"合成中文预算测试。".repeat(150),"source_refs":[id],"evidence":"user_explicit","labels":["bootstrap"]})).unwrap();
        v.add_memory(input).unwrap();
    }
    let c = context(&v);
    for budget in [512, 1024, 1500, 4096] {
        let mut args = search("中文");
        args.budget_tokens = budget;
        let r = c.search(args).unwrap();
        assert!(serde_json::to_vec(&r).unwrap().len() <= budget);
        let r = c
            .bootstrap(BootstrapArgs {
                budget_tokens: budget,
            })
            .unwrap();
        assert!(serde_json::to_vec(&r).unwrap().len() <= budget);
        let refs = v
            .events()
            .unwrap()
            .iter()
            .map(|e| format!("event:{}", e.id))
            .collect();
        let r = c
            .read(ReadArgs {
                refs,
                budget_tokens: budget,
            })
            .unwrap();
        assert!(serde_json::to_vec(&r).unwrap().len() <= budget);
    }
}

#[test]
fn recaptured_capsule_is_not_new_evidence_or_search_noise() {
    let (_d, v) = setup();
    let id = capture(
        &v,
        "personal",
        "echo",
        r#"用户附言加上上下文 {"schema":"recallcard.context/1","result":"合成被复述事实"}"#,
    );
    assert_eq!(
        v.event(&id).unwrap().data.origin,
        recallcard::Origin::ContextInjection
    );
    assert!(
        context(&v).search(search("合成被复述事实")).unwrap()["results"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let input: MemoryInput = serde_json::from_value(
        json!({"content":"合成被复述事实","source_refs":[id],"evidence":"user_explicit"}),
    )
    .unwrap();
    assert!(v.add_memory(input).is_err());
}

#[test]
fn metadata_only_file_sources_report_missing_body() {
    let (_d, v) = setup();
    let input:EventInput=serde_json::from_value(json!({"occurred_at":null,"role":"tool","origin":"tool_output","kind":"file","metadata":{"path":"synthetic.txt","size":12},"source":{"platform":"demo","conversation_id":"demo","message_id":"file"}})).unwrap();
    let e = v.capture(input).unwrap();
    let r = context(&v)
        .read(ReadArgs {
            refs: vec![format!("event:{}", e.id)],
            budget_tokens: 12000,
        })
        .unwrap();
    assert_eq!(r["results"][0]["retention"], "content_not_retained");
}
#[test]
fn disposable_index_and_empty_directories_are_rebuilt_offline() {
    let (_d, v) = setup();
    capture(&v, "personal", "a", "离线恢复合成内容");
    std::fs::remove_dir_all(v.root().join(".index")).unwrap();
    std::fs::remove_dir_all(v.root().join("generated")).unwrap();
    std::fs::remove_dir_all(v.root().join("memories")).unwrap();
    std::fs::remove_dir_all(v.root().join("objects")).unwrap();
    let reopened = Vault::open(v.root()).unwrap();
    reopened.rebuild().unwrap();
    assert!(reopened.root().join(".index/text.json").exists());
    assert_eq!(
        context(&reopened).search(search("离线恢复")).unwrap()["results"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

fn memory_with(
    vault: &Vault,
    scope: &str,
    source_key: &str,
    content: &str,
    extra: Value,
) -> recallcard::Memory {
    let source = capture(vault, scope, source_key, content);
    let mut input =
        json!({"scope":scope,"content":content,"source_refs":[source],"evidence":"user_explicit"});
    input
        .as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    vault
        .add_memory(serde_json::from_value(input).unwrap())
        .unwrap()
}

fn memory_search(query: &str) -> SearchArgs {
    let mut args = search(query);
    args.target = "memories".into();
    args
}

fn view(context: &Context<'_>, label: &str) -> Value {
    context
        .read(ReadArgs {
            refs: vec![format!("view:{label}")],
            budget_tokens: 12000,
        })
        .unwrap()["results"][0]
        .clone()
}

#[test]
fn same_scope_arch_laptop_and_debian_server_are_independent_entities() {
    let (_dir, vault) = setup();
    let laptop = memory_with(
        &vault,
        "personal",
        "laptop",
        "操作系统为 Arch Linux。",
        json!({"entities":["笔记本","laptop-synthetic"],"labels":["computing"]}),
    );
    let server = memory_with(
        &vault,
        "personal",
        "server",
        "操作系统为 Debian。",
        json!({"entities":["服务器","server-synthetic"],"labels":["computing"]}),
    );
    let context = context(&vault);
    for (query, expected) in [
        ("笔记本", &laptop),
        ("LAPTOP-SYNTHETIC", &laptop),
        ("服务器", &server),
        ("server-synthetic", &server),
    ] {
        let response = context.search(memory_search(query)).unwrap();
        assert_eq!(response["results"].as_array().unwrap().len(), 1);
        assert_eq!(
            response["results"][0]["ref"],
            format!("memory:{}@1", expected.id)
        );
        assert_eq!(response["results"][0]["text"], expected.data.content);
        assert_eq!(
            response["results"][0]["entities"],
            json!(expected.data.entities)
        );
    }
    let combined = context.search(memory_search("操作系统")).unwrap();
    assert_eq!(combined["results"].as_array().unwrap().len(), 2);
    assert_eq!(
        view(&context, "computing")["records"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(vault.memory(&laptop.id).unwrap(), laptop);
    assert_eq!(vault.memory(&server.id).unwrap(), server);
}

#[test]
fn labels_are_searchable_without_rewriting_memory_content() {
    let (_dir, vault) = setup();
    let memory = memory_with(
        &vault,
        "personal",
        "labels",
        "说明使用简洁中文。",
        json!({"labels":["writing-preference","写作习惯"]}),
    );
    for query in ["writing-preference", "写作习惯"] {
        let response = context(&vault).search(memory_search(query)).unwrap();
        assert_eq!(response["results"].as_array().unwrap().len(), 1);
        assert_eq!(response["results"][0]["text"], memory.data.content);
        assert_eq!(response["results"][0]["labels"], json!(memory.data.tags));
    }
    let docs = context(&vault).documents().unwrap();
    assert!(docs
        .iter()
        .filter(|doc| doc.kind == "event")
        .all(|doc| doc.entities.is_empty()));
}

#[test]
fn named_view_is_dynamic_and_never_reads_generated_files() {
    let (_dir, vault) = setup();
    let memory = memory_with(
        &vault,
        "personal",
        "view",
        "当前资料来自正本。",
        json!({"labels":["computing","计算环境"],"entities":["合成电脑"]}),
    );
    let generated = vault.root().join("generated/views/computing.md");
    std::fs::write(&generated, "合成伪造视图：不应该被读取").unwrap();
    let context = context(&vault);
    let first = view(&context, "computing");
    assert_eq!(
        first["records"][0]["ref"],
        format!("memory:{}@1", memory.id)
    );
    assert_eq!(first["records"][0]["text"], "当前资料来自正本。");
    assert!(!first.to_string().contains("伪造视图"));
    assert_eq!(first["records"], view(&context, "计算环境")["records"]);
    let mut updated = memory.data.clone();
    updated.content = "正本更新会立即反映到具名视图。".into();
    vault.update_memory(&memory.id, 1, updated).unwrap();
    let latest = view(&context, "computing");
    assert_eq!(
        latest["records"][0]["ref"],
        format!("memory:{}@2", memory.id)
    );
    assert!(latest["records"][0]["text"]
        .as_str()
        .unwrap()
        .contains("立即反映"));
    assert_eq!(
        std::fs::read_to_string(&generated).unwrap(),
        "合成伪造视图：不应该被读取"
    );
}

#[test]
fn named_view_rejects_paths_encoded_paths_versions_and_controls() {
    let (_dir, vault) = setup();
    for reference in [
        "view:",
        "view:.",
        "view:..",
        "view:../secret",
        "view:..\\secret",
        "view:/tmp/private",
        "view:%2e%2e",
        "view:%2fetc",
        "view:computing@1",
        "view:scope:secret",
        "view:计算\n环境",
        "view: has-space",
        "file:/tmp/private",
    ] {
        assert!(
            context(&vault)
                .read(ReadArgs {
                    refs: vec![reference.into()],
                    budget_tokens: 12000
                })
                .is_err(),
            "{reference}"
        );
    }
    assert!(context(&vault)
        .read(ReadArgs {
            refs: vec![format!("view:{}", "a".repeat(129))],
            budget_tokens: 12000,
        })
        .is_err());
    for reference in [
        "view:computing",
        "view:计算环境",
        "view:work-station.v1",
        "view:profile",
    ] {
        assert!(recallcard::context::parse_ref(reference).is_ok());
    }
}

#[test]
fn named_views_and_entity_search_cannot_reveal_other_scopes() {
    let (_dir, vault) = setup();
    let visible = memory_with(
        &vault,
        "personal",
        "visible",
        "公开读取范围的合成内容。",
        json!({"labels":["computing"],"entities":["visible-machine"]}),
    );
    let hidden = memory_with(
        &vault,
        "project:secret",
        "hidden",
        "保密范围的合成内容。",
        json!({"labels":["computing","secret-only-label"],"entities":["secret-machine"]}),
    );
    let context = context(&vault);
    assert_eq!(
        view(&context, "computing")["records"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        view(&context, "computing")["records"][0]["ref"],
        format!("memory:{}@1", visible.id)
    );
    for query in ["secret-only-label", "secret-machine"] {
        assert!(context.search(memory_search(query)).unwrap()["results"]
            .as_array()
            .unwrap()
            .is_empty());
    }
    for label in ["secret-only-label", "not-present-label"] {
        let response = view(&context, label);
        assert!(response["records"].as_array().unwrap().is_empty());
        assert_eq!(response["truncated"], false);
        assert!(response["pending_refs"].as_array().unwrap().is_empty());
    }
    let bootstrap = context
        .bootstrap(BootstrapArgs {
            budget_tokens: 12000,
        })
        .unwrap();
    assert!(!bootstrap.to_string().contains("secret-only-label"));
    assert!(!bootstrap.to_string().contains("secret-machine"));
    for source in [false, true] {
        let args = ReadArgs {
            refs: vec![format!("memory:{}@1", hidden.id)],
            budget_tokens: 12000,
        };
        assert!(if source {
            context.sources(args)
        } else {
            context.read(args)
        }
        .is_err());
    }
}

#[test]
fn suppression_is_rechecked_for_named_views_and_metadata_search() {
    let (_dir, vault) = setup();
    let memory = memory_with(
        &vault,
        "personal",
        "suppressed-view",
        "合成可抑制记忆。",
        json!({"labels":["computing","bootstrap"],"entities":["suppressed-machine"]}),
    );
    let context = context(&vault);
    assert_eq!(
        view(&context, "computing")["records"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    for id in [&memory.id, &memory.data.source_refs[0]] {
        vault.suppress(id, "合成撤权".into()).unwrap();
        assert!(view(&context, "computing")["records"]
            .as_array()
            .unwrap()
            .is_empty());
        assert!(
            context.search(memory_search("suppressed-machine")).unwrap()["results"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert!(!context
            .bootstrap(BootstrapArgs {
                budget_tokens: 12000
            })
            .unwrap()["stable_text"]
            .as_str()
            .unwrap()
            .contains("view:computing"));
        vault.restore(id).unwrap();
        assert_eq!(
            view(&context, "computing")["records"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }
}

#[test]
fn default_current_recall_excludes_future_and_expired_memories() {
    use chrono::{Duration, Utc};
    let (_dir, vault) = setup();
    let now = Utc::now();
    let current = memory_with(
        &vault,
        "personal",
        "current",
        "时态合成记录：现在有效。",
        json!({"labels":["bootstrap","computing"],"valid_from":now-Duration::days(2),"valid_to":now+Duration::days(2)}),
    );
    let unknown = memory_with(
        &vault,
        "personal",
        "unknown",
        "时态合成记录：没有独立生效日期。",
        json!({"labels":["computing"],"time_note":"生效时间未知"}),
    );
    let future = memory_with(
        &vault,
        "personal",
        "future",
        "时态合成记录：未来才生效。",
        json!({"labels":["bootstrap","computing","future-only"],"valid_from":now+Duration::days(10)}),
    );
    let expired = memory_with(
        &vault,
        "personal",
        "expired",
        "时态合成记录：已经过期。",
        json!({"labels":["bootstrap","computing","expired-only"],"valid_to":now-Duration::days(10)}),
    );
    let context = context(&vault);
    let response = context.search(memory_search("时态合成记录")).unwrap();
    let refs: Vec<&str> = response["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["ref"].as_str().unwrap())
        .collect();
    assert_eq!(refs.len(), 2);
    assert!(refs.contains(&format!("memory:{}@1", current.id).as_str()));
    assert!(refs.contains(&format!("memory:{}@1", unknown.id).as_str()));
    let records = view(&context, "computing");
    assert_eq!(records["records"].as_array().unwrap().len(), 2);
    assert!(!records.to_string().contains(&future.id));
    assert!(!records.to_string().contains(&expired.id));
    let bootstrap = context
        .bootstrap(BootstrapArgs {
            budget_tokens: 12000,
        })
        .unwrap();
    assert!(bootstrap["stable_text"]
        .as_str()
        .unwrap()
        .contains("现在有效"));
    for excluded in ["未来才生效", "已经过期", "future-only", "expired-only"] {
        assert!(!bootstrap.to_string().contains(excluded));
    }
    let corpus = context.embedding_corpus().unwrap();
    assert_eq!(corpus["documents"].as_array().unwrap().len(), 2);
    assert!(!corpus.to_string().contains(&future.id));
    assert!(!corpus.to_string().contains(&expired.id));
    // 默认筛选不删除历史，也不伪造未知的生效日期。
    assert_eq!(
        context
            .documents()
            .unwrap()
            .iter()
            .filter(|doc| doc.kind == "memory")
            .count(),
        4
    );
    assert_eq!(vault.memory(&unknown.id).unwrap().data.valid_from, None);
    for memory in [&future, &expired] {
        let response = context
            .read(ReadArgs {
                refs: vec![format!("memory:{}@1", memory.id)],
                budget_tokens: 12000,
            })
            .unwrap();
        assert_eq!(response["results"][0]["record"]["id"], memory.id);
    }
}

#[test]
fn explicit_as_of_keeps_history_and_uses_half_open_valid_intervals() {
    let (_dir, vault) = setup();
    let memory = memory_with(
        &vault,
        "personal",
        "interval",
        "历史窗口合成记录。",
        json!({"labels":["computing"],"valid_from":"2020-01-01T00:00:00Z","valid_to":"2021-01-01T00:00:00Z"}),
    );
    let memory = vault
        .set_state(&memory.id, 1, recallcard::MemoryState::Superseded)
        .unwrap();
    let context = context(&vault);
    assert!(
        context.search(memory_search("历史窗口")).unwrap()["results"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    for (time, count) in [
        ("2019-12-31T23:59:59Z", 0),
        ("2020-01-01T00:00:00Z", 1),
        ("2020-12-31T23:59:59Z", 1),
        ("2021-01-01T00:00:00Z", 0),
    ] {
        let mut args = memory_search("历史窗口");
        args.as_of = Some(time.parse().unwrap());
        let response = context.search(args).unwrap();
        assert_eq!(response["results"].as_array().unwrap().len(), count);
        if count == 1 {
            assert_eq!(
                response["results"][0]["ref"],
                format!("memory:{}@2", memory.id)
            );
            assert_eq!(response["results"][0]["state"], "superseded");
        }
    }
    assert!(view(&context, "computing")["records"]
        .as_array()
        .unwrap()
        .is_empty());
}

#[test]
fn historical_as_of_does_not_bypass_suppression_or_scope() {
    let (_dir, vault) = setup();
    let memory = memory_with(
        &vault,
        "personal",
        "historical-suppressed",
        "过去合成事实。",
        json!({"valid_to":"2021-01-01T00:00:00Z"}),
    );
    memory_with(
        &vault,
        "project:secret",
        "historical-private",
        "过去合成事实。",
        json!({"valid_to":"2021-01-01T00:00:00Z"}),
    );
    vault.suppress(&memory.id, "历史也不再召回".into()).unwrap();
    let mut args = memory_search("过去合成事实");
    args.as_of = Some("2020-06-01T00:00:00Z".parse().unwrap());
    assert!(context(&vault).search(args).unwrap()["results"]
        .as_array()
        .unwrap()
        .is_empty());
}

#[test]
fn bootstrap_view_directory_is_stable_and_only_current_safe_labels_are_advertised() {
    let (_dir, vault) = setup();
    memory_with(
        &vault,
        "personal",
        "directory",
        "明确受保护的简洁说明。",
        json!({"labels":["bootstrap","computing","计算环境","../invalid","bad label"]}),
    );
    memory_with(
        &vault,
        "personal",
        "tentative",
        "尚待确认的助手建议。",
        json!({"evidence":"assistant_suggestion","labels":["bootstrap","tentative-topic"]}),
    );
    let context = context(&vault);
    let first = context
        .bootstrap(BootstrapArgs {
            budget_tokens: 12000,
        })
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(2));
    let second = context
        .bootstrap(BootstrapArgs {
            budget_tokens: 12000,
        })
        .unwrap();
    assert_eq!(first, second);
    let stable = first["stable_text"].as_str().unwrap();
    assert!(stable.contains("view:computing"));
    assert!(stable.contains("view:计算环境"));
    assert!(stable.contains("view:tentative-topic"));
    assert!(stable.contains("明确受保护的简洁说明"));
    assert!(!stable.contains("尚待确认的助手建议"));
    assert!(!stable.contains("../invalid"));
    assert!(!stable.contains("bad label"));
    assert!(!stable.contains("now"));
    assert_eq!(
        first["bootstrap_version"],
        recallcard::model::hash(stable.as_bytes())
    );
    let profile = view(&context, "profile");
    assert_eq!(profile["stable_text"], first["stable_text"]);
    assert_eq!(profile["bootstrap_version"], first["bootstrap_version"]);
    assert_eq!(
        view(&context, "tentative-topic")["records"][0]["state"],
        "tentative"
    );
}

#[test]
fn old_memory_refs_are_rejected_after_update_from_a_named_view() {
    let (_dir, vault) = setup();
    let memory = memory_with(
        &vault,
        "personal",
        "old-ref",
        "原有合成说明。",
        json!({"labels":["computing"]}),
    );
    let context = context(&vault);
    let reference = view(&context, "computing")["records"][0]["ref"]
        .as_str()
        .unwrap()
        .to_owned();
    let mut update = memory.data.clone();
    update.content = "更新后的合成说明。".into();
    vault.update_memory(&memory.id, 1, update).unwrap();
    for source in [false, true] {
        let args = ReadArgs {
            refs: vec![reference.clone()],
            budget_tokens: 12000,
        };
        assert!(if source {
            context.sources(args)
        } else {
            context.read(args)
        }
        .is_err());
    }
    assert_eq!(
        view(&context, "computing")["records"][0]["ref"],
        format!("memory:{}@2", memory.id)
    );
}

#[test]
fn named_views_and_mixed_reads_respect_total_byte_budget() {
    let (_dir, vault) = setup();
    let mut memories = Vec::new();
    for i in 0..12 {
        memories.push(memory_with(
            &vault,
            "personal",
            &format!("budget-view-{i}"),
            &"合成较长记录。".repeat(100),
            json!({"labels":["computing","bootstrap"],"entities":[format!("machine-{i}")]}),
        ));
    }
    let context = context(&vault);
    for budget in [512, 1024, 1500, 4096, 12000] {
        for sources in [false, true] {
            let args = ReadArgs {
                refs: vec![
                    "view:computing".into(),
                    "view:profile".into(),
                    format!("memory:{}@1", memories[0].id),
                ],
                budget_tokens: budget,
            };
            let response = if sources {
                context.sources(args)
            } else {
                context.read(args)
            }
            .unwrap();
            assert!(serde_json::to_vec(&response).unwrap().len() <= budget);
            assert_eq!(response["truncated"], true);
            assert!(response["results"]
                .as_array()
                .unwrap()
                .iter()
                .all(|result| result["ref"].is_string()));
        }
    }
    let response = context
        .read(ReadArgs {
            refs: vec!["view:computing".into()],
            budget_tokens: 4096,
        })
        .unwrap();
    assert_eq!(response["truncated"], true);
    assert_eq!(response["results"][0]["truncated"], true);
    assert!(!response["results"][0]["pending_refs"]
        .as_array()
        .unwrap()
        .is_empty());
}

#[test]
fn metadata_changes_invalidate_search_cursors_without_hidden_scope_leakage() {
    let (_dir, vault) = setup();
    let a = memory_with(
        &vault,
        "personal",
        "cursor-a",
        "合成查询记录甲。",
        json!({"labels":["computing"],"entities":["common-machine"]}),
    );
    memory_with(
        &vault,
        "personal",
        "cursor-b",
        "合成查询记录乙。",
        json!({"labels":["computing"],"entities":["common-machine"]}),
    );
    let context = context(&vault);
    let mut args = memory_search("common-machine");
    args.limit = 1;
    let initial = context.search(args.clone()).unwrap();
    args.cursor = initial["next_cursor"].as_str().map(str::to_owned);
    assert!(args.cursor.is_some());
    memory_with(
        &vault,
        "project:secret",
        "cursor-private",
        "范围外合成查询记录。",
        json!({"labels":["computing"],"entities":["common-machine"]}),
    );
    assert!(context.search(args.clone()).is_ok());
    let mut update = a.data.clone();
    update.entities.push("renamed-machine".into());
    vault.update_memory(&a.id, 1, update).unwrap();
    assert!(context.search(args).is_err());
}
