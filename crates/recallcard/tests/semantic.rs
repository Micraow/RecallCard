use recallcard::{
    context::{Context, SearchArgs},
    model::hash,
    policy::Access,
    semantic::{CloudQueryApproval, SemanticConfig, SemanticSearch},
    transport, EventInput, MemoryInput, Vault,
};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use tempfile::TempDir;

// 各用例同时冷启动 Python 会争抢 CI 进程/防病毒扫描资源，混淆协议与启动时限。
// 仅串行独立 fixture；同一测试内部的并发查询、撤权和真实超时仍完整执行。
fn fixture_process_guard() -> std::sync::MutexGuard<'static, ()> {
    static PROCESS_FIXTURES: std::sync::Mutex<()> = std::sync::Mutex::new(());
    PROCESS_FIXTURES
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

struct Fixture {
    directory: TempDir,
    vault: Vault,
    memory_ref: String,
    source_id: String,
}
impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let vault = Vault::init(&directory.path().join("vault")).unwrap();
        let event: EventInput = serde_json::from_value(json!({"role":"user","origin":"native","scope":"personal","content":"合成原始事件 lexical","source":{"platform":"manual-web","conversation_id":"semantic","message_id":"first"}})).unwrap();
        let source_id = vault.capture(event).unwrap().id;
        let memory: MemoryInput = serde_json::from_value(json!({"content":"远足装备整理清单","scope":"personal","source_refs":[source_id],"evidence":"user_explicit"})).unwrap();
        let memory_ref = format!("memory:{}@1", vault.add_memory(memory).unwrap().id);
        Self {
            directory,
            vault,
            memory_ref,
            source_id,
        }
    }
    fn context(&self) -> Context<'_> {
        Context::new(&self.vault, access())
    }
    fn path(&self, name: &str) -> PathBuf {
        self.directory.path().join(name)
    }
    fn config(&self, query: &str) -> SemanticConfig {
        let corpus = self.context().embedding_corpus().unwrap();
        let space = json!({"provider":"fixture","endpoint":"https://synthetic.invalid/v1/embeddings","model":"fixture-v1","dimensions":2,"revision":"fixture","document_instruction":"","query_instruction":"","normalization":"l2","preprocessing_version":"utf8-v1"});
        let signature = hash(&serde_json::to_vec(&space).unwrap());
        let documents = corpus["documents"].as_array().unwrap().iter().map(|document| json!({"ref":document["ref"],"scope":document["scope"],"content_hash":document["content_hash"],"input_hash":document["content_hash"],"vector":[1.0,0.0]})).collect::<Vec<_>>();
        write_json(
            &self.path("index.json"),
            &json!({"schema":"recallcard.embedding-index/1","space":space,"space_signature":signature,"generation":corpus["generation"],"scope":corpus["scope"],"complete":true,"documents":documents}),
        );
        write_json(
            &self.path("queries.json"),
            &json!({"schema":"recallcard.embedding-query-cache/1","space_signature":signature,"queries":[{"query_hash":hash(query.as_bytes()),"input_hash":hash(query.as_bytes()),"vector":[1.0,0.0]}]}),
        );
        SemanticConfig {
            python: if cfg!(windows) { "python" } else { "python3" }.into(),
            python_path: PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../python")
                .canonicalize()
                .unwrap(),
            index_path: self.path("index.json"),
            expected_space_signature: signature,
            query_cache_path: Some(self.path("queries.json")),
            cloud_query: None,
            timeout_ms: 10000,
        }
    }
    fn fake(&self, config: &mut SemanticConfig, body: &str) {
        let package = self.path("fake/recallcard_worker");
        fs::create_dir_all(&package).unwrap();
        fs::write(package.join("__init__.py"), "").unwrap();
        let body = body
            .lines()
            .map(|line| format!("    {line}\n"))
            .collect::<String>();
        fs::write(
            package.join("__main__.py"),
            format!("import json,sys,time,os\ndef main(argv=None):\n{body}"),
        )
        .unwrap();
        config.python_path = self.path("fake");
    }
    fn search(&self, config: SemanticConfig, query: &str) -> Value {
        let semantic = SemanticSearch::new(config).unwrap();
        Context::with_semantic(&self.vault, access(), &semantic)
            .search(args(query))
            .unwrap()
    }
    fn edit_index(&self, edit: impl FnOnce(&mut Value)) {
        let mut index: Value =
            serde_json::from_slice(&fs::read(self.path("index.json")).unwrap()).unwrap();
        edit(&mut index);
        write_json(&self.path("index.json"), &index);
    }
}
fn access() -> Access {
    Access::new(vec!["personal".into()]).unwrap()
}
fn args(query: &str) -> SearchArgs {
    serde_json::from_value(json!({"query":query,"budget_tokens":12000})).unwrap()
}
fn write_json(path: &Path, value: &Value) {
    fs::write(path, serde_json::to_vec(value).unwrap()).unwrap();
}
fn contains(response: &Value, reference: &str) -> bool {
    response["results"]
        .as_array()
        .unwrap()
        .iter()
        .any(|row| row["ref"] == reference)
}
fn fallback(response: &Value, code: &str) {
    assert_eq!(
        response["coverage"]["semantic_search"], "unavailable",
        "{response}"
    );
    assert_eq!(response["coverage"]["semantic_error"], code, "{response}");
}
const ECHO: &str = "for line in sys.stdin:\n    r=json.loads(line)\n    result={'results':[{'ref':ref,'score':0.9} for ref in r['allowed_refs']], 'ranking':'cosine','space_signature':r['index']['space_signature'],'generation':r['expected_generation']}\n    print(json.dumps({'id':r['id'],'ok':True,'result':result}),flush=True)";

#[test]
fn real_python_offline_semantics_adds_nonlexical_memory_and_keeps_event() {
    let _fixture_guard = fixture_process_guard();
    let fixture = Fixture::new();
    let config = fixture.config("lexical");
    let response = fixture.search(config, "lexical");
    assert!(contains(&response, &fixture.memory_ref));
    assert!(contains(&response, &format!("event:{}", fixture.source_id)));
    assert_eq!(response["coverage"]["semantic_search"], "available");
    assert_eq!(response["coverage"]["ranking"], "rrf");
    assert_eq!(
        response["coverage"]["semantic_coverage"],
        "indexed_current_memories"
    );
}

#[test]
fn default_search_does_not_need_python_or_configuration() {
    let _fixture_guard = fixture_process_guard();
    let fixture = Fixture::new();
    let response = fixture.context().search(args("lexical")).unwrap();
    assert_eq!(response["coverage"]["semantic_search"], "unavailable");
    assert!(response["coverage"].get("semantic_error").is_none());
    assert!(!contains(&response, &fixture.memory_ref));
}

#[test]
fn absent_query_approval_never_launches_worker_even_with_index() {
    let _fixture_guard = fixture_process_guard();
    let fixture = Fixture::new();
    let mut config = fixture.config("different exact query");
    config.python = fixture.path("missing-python");
    fallback(&fixture.search(config, "lexical"), "query_not_cached");
}

#[test]
fn missing_python_is_safe_text_fallback() {
    let _fixture_guard = fixture_process_guard();
    let fixture = Fixture::new();
    let mut config = fixture.config("lexical");
    config.python = fixture.path("missing-python");
    let response = fixture.search(config, "lexical");
    fallback(&response, "worker_unavailable");
    assert!(contains(&response, &format!("event:{}", fixture.source_id)));
}

#[test]
fn stale_incomplete_incompatible_and_bad_content_caches_fall_back() {
    let _fixture_guard = fixture_process_guard();
    let fixture = Fixture::new();
    for (field, value, code) in [
        ("generation", json!("a".repeat(64)), "generation_mismatch"),
        ("complete", json!(false), "index_incomplete"),
        (
            "space_signature",
            json!("f".repeat(64)),
            "signature_mismatch",
        ),
    ] {
        let config = fixture.config("lexical");
        fixture.edit_index(|index| index[field] = value);
        fallback(&fixture.search(config, "lexical"), code);
    }
    let config = fixture.config("lexical");
    fixture.edit_index(|index| index["documents"][0]["content_hash"] = json!("a".repeat(64)));
    fallback(&fixture.search(config, "lexical"), "content_mismatch");
    let config = fixture.config("lexical");
    fixture.edit_index(|index| index["documents"][0]["input_hash"] = json!("a".repeat(64)));
    fallback(&fixture.search(config, "lexical"), "content_mismatch");
}

#[test]
fn cache_scope_cannot_expand_client_access() {
    let _fixture_guard = fixture_process_guard();
    let fixture = Fixture::new();
    let config = fixture.config("lexical");
    fixture.edit_index(|index| index["scope"] = json!(["personal", "project:private"]));
    fallback(&fixture.search(config, "lexical"), "scope_mismatch");
}

#[test]
fn request_cannot_choose_startup_configuration() {
    let _fixture_guard = fixture_process_guard();
    let fixture = Fixture::new();
    let semantic = SemanticSearch::new(fixture.config("lexical")).unwrap();
    let context = Context::with_semantic(&fixture.vault, access(), &semantic);
    for (field, value) in [
        ("python", json!("evil")),
        ("endpoint", json!("https://evil.invalid")),
        ("scope", json!("private")),
        ("allow_network", json!(true)),
        ("query_vector", json!([1, 0])),
        ("semantic_config", json!("/tmp/anything")),
    ] {
        let mut request = json!({"query":"lexical"});
        request[field] = value;
        assert!(transport::invoke(&context, "search", request).is_err());
    }
}

#[test]
fn real_subprocess_timeout_is_bounded_and_does_not_restart() {
    let _fixture_guard = fixture_process_guard();
    let fixture = Fixture::new();
    let mut config = fixture.config("lexical");
    config.timeout_ms = 100;
    fixture.fake(&mut config, "time.sleep(60)");
    let semantic = SemanticSearch::new(config).unwrap();
    let context = Context::with_semantic(&fixture.vault, access(), &semantic);
    let start = Instant::now();
    fallback(&context.search(args("lexical")).unwrap(), "worker_timeout");
    assert!(start.elapsed() < Duration::from_secs(2));
    let start = Instant::now();
    fallback(&context.search(args("lexical")).unwrap(), "worker_timeout");
    assert!(start.elapsed() < Duration::from_millis(100));
}

#[test]
fn child_exit_malformed_and_oversized_stdout_are_bounded() {
    let _fixture_guard = fixture_process_guard();
    let fixture = Fixture::new();
    for (body, code) in [
        ("return 3", "worker_exited"),
        ("print('not-json',flush=True)", "malformed_response"),
        (
            "sys.stdout.write('x'*70000); sys.stdout.flush()",
            "output_limit",
        ),
    ] {
        let mut config = fixture.config("lexical");
        fixture.fake(&mut config, body);
        let response = fixture.search(config, "lexical");
        fallback(&response, code);
        assert!(contains(&response, &format!("event:{}", fixture.source_id)));
    }
}

#[test]
fn worker_error_message_is_not_exposed() {
    let _fixture_guard = fixture_process_guard();
    let fixture = Fixture::new();
    let mut config = fixture.config("lexical");
    fixture.fake(&mut config,"r=json.loads(sys.stdin.readline())\nprint(json.dumps({'id':r['id'],'ok':False,'error':{'code':'anything-secret','message':'DO-NOT-EXPOSE-QUERY-KEY'}}),flush=True)");
    let response = fixture.search(config, "lexical");
    fallback(&response, "worker_error");
    assert!(!response.to_string().contains("DO-NOT-EXPOSE"));
}

#[test]
fn wrong_generation_wrong_id_and_hidden_refs_are_rejected() {
    let _fixture_guard = fixture_process_guard();
    let fixture = Fixture::new();
    for body in [
        ECHO.replace(
            "'generation':r['expected_generation']",
            "'generation':'stale'",
        ),
        ECHO.replace("'id':r['id']", "'id':999"),
        ECHO.replace(
            "for ref in r['allowed_refs']",
            "for ref in ['memory:mem_private@1']",
        ),
    ] {
        let mut config = fixture.config("lexical");
        fixture.fake(&mut config, &body);
        fallback(&fixture.search(config, "lexical"), "malformed_response");
    }
}

#[test]
fn cloud_query_approval_is_separate_scoped_and_only_startup_supplies_it() {
    let _fixture_guard = fixture_process_guard();
    let fixture = Fixture::new();
    let mut config = fixture.config("different");
    config.query_cache_path = None;
    config.cloud_query = Some(CloudQueryApproval {
        endpoint: "https://synthetic.invalid/v1/embeddings".into(),
        scopes: vec!["personal".into()],
        key_env: "RECALLCARD_SYNTHETIC_UNUSED_KEY".into(),
        max_network_calls: 2,
        max_transmitted_bytes: 100,
    });
    fixture.fake(&mut config,&format!("assert '--allow-network' in argv\nassert argv[argv.index('--approve-data')+1]=='query'\nassert 'corpus' not in argv\n{ECHO}"));
    let response = fixture.search(config.clone(), "lexical");
    assert_eq!(response["coverage"]["semantic_search"], "available");
    config.cloud_query.as_mut().unwrap().scopes = vec!["project:another".into()];
    fallback(
        &fixture.search(config.clone(), "lexical"),
        "query_not_approved",
    );
    config.cloud_query.as_mut().unwrap().scopes = vec!["personal".into()];
    config.cloud_query.as_mut().unwrap().endpoint = "https://other.invalid/v1/embeddings".into();
    fallback(&fixture.search(config, "lexical"), "query_not_approved");
}

#[test]
fn suppression_during_worker_wait_rechecks_text_and_semantic_refs() {
    let _fixture_guard = fixture_process_guard();
    let fixture = Fixture::new();
    let mut config = fixture.config("lexical");
    let marker = fixture.path("worker-ready");
    fixture.fake(
        &mut config,
        &format!(
            "open({},'w').write('ready')\ntime.sleep(0.4)\n{ECHO}",
            serde_json::to_string(&marker).unwrap()
        ),
    );
    let semantic = SemanticSearch::new(config).unwrap();
    let context = Context::with_semantic(&fixture.vault, access(), &semantic);
    std::thread::scope(|scope| {
        let searching = scope.spawn(|| context.search(args("lexical")).unwrap());
        let start = Instant::now();
        while !marker.exists() {
            assert!(start.elapsed() < Duration::from_secs(10));
            std::thread::sleep(Duration::from_millis(5));
        }
        fixture
            .vault
            .suppress(&fixture.source_id, "合成并发遗忘".into())
            .unwrap();
        let response = searching.join().unwrap();
        fallback(&response, "generation_mismatch");
        assert!(response["results"].as_array().unwrap().is_empty());
    });
}

#[test]
fn hybrid_response_respects_total_budget_including_coverage() {
    let _fixture_guard = fixture_process_guard();
    let fixture = Fixture::new();
    let semantic = SemanticSearch::new(fixture.config("lexical")).unwrap();
    let context = Context::with_semantic(&fixture.vault, access(), &semantic);
    for budget in [512, 700, 1000, 1500] {
        let mut query = args("lexical");
        query.budget_tokens = budget;
        let response = context.search(query).unwrap();
        assert!(
            serde_json::to_vec(&response).unwrap().len() <= budget,
            "{response}"
        );
        assert_eq!(response["coverage"]["semantic_search"], "available");
    }
}

#[test]
fn changed_ranking_mode_invalidates_pagination_cursor() {
    let _fixture_guard = fixture_process_guard();
    let fixture = Fixture::new();
    let config = fixture.config("lexical");
    let semantic = SemanticSearch::new(config).unwrap();
    let context = Context::with_semantic(&fixture.vault, access(), &semantic);
    let mut query = args("lexical");
    query.limit = 1;
    let first = context.search(query.clone()).unwrap();
    assert!(first["next_cursor"].is_string());
    fixture.edit_index(|index| index["complete"] = json!(false));
    query.cursor = first["next_cursor"].as_str().map(str::to_owned);
    assert!(context.search(query).unwrap_err().contains("游标已失效"));
}

#[test]
fn memory_revision_during_worker_wait_uses_only_current_text() {
    let _fixture_guard = fixture_process_guard();
    let fixture = Fixture::new();
    let mut config = fixture.config("lexical");
    let marker = fixture.path("revision-ready");
    fixture.fake(
        &mut config,
        &format!(
            "open({},'w').write('ready')\ntime.sleep(0.4)\n{ECHO}",
            serde_json::to_string(&marker).unwrap()
        ),
    );
    let semantic = SemanticSearch::new(config).unwrap();
    let context = Context::with_semantic(&fixture.vault, access(), &semantic);
    std::thread::scope(|scope| {
        let searching = scope.spawn(|| context.search(args("lexical")).unwrap());
        let start = Instant::now();
        while !marker.exists() {
            assert!(start.elapsed() < Duration::from_secs(10));
            std::thread::sleep(Duration::from_millis(5));
        }
        let id = fixture
            .memory_ref
            .strip_prefix("memory:")
            .unwrap()
            .split('@')
            .next()
            .unwrap();
        let mut input = fixture.vault.memory(id).unwrap().data;
        input.content = "lexical 当前修订内容".into();
        let updated = fixture.vault.update_memory(id, 1, input).unwrap();
        let response = searching.join().unwrap();
        fallback(&response, "generation_mismatch");
        assert!(!contains(&response, &fixture.memory_ref));
        assert!(contains(&response, &format!("memory:{}@2", updated.id)));
        assert!(!response.to_string().contains("远足装备"));
    });
}

#[test]
fn validity_expiry_during_worker_wait_is_rechecked_at_response_time() {
    let _fixture_guard = fixture_process_guard();
    let fixture = Fixture::new();
    let id = fixture
        .memory_ref
        .strip_prefix("memory:")
        .unwrap()
        .split('@')
        .next()
        .unwrap();
    let mut input = fixture.vault.memory(id).unwrap().data;
    input.valid_to = Some(chrono::Utc::now() + chrono::Duration::milliseconds(700));
    fixture.vault.update_memory(id, 1, input).unwrap();
    let mut config = fixture.config("lexical");
    fixture.fake(&mut config, &format!("time.sleep(1.0)\n{ECHO}"));
    let response = fixture.search(config, "lexical");
    fallback(&response, "generation_mismatch");
    assert!(response["results"]
        .as_array()
        .unwrap()
        .iter()
        .all(|item| item["kind"] != "memory"));
}

#[test]
fn session_target_and_as_of_filters_apply_before_semantic_ranking() {
    let _fixture_guard = fixture_process_guard();
    let fixture = Fixture::new();
    let id = fixture
        .memory_ref
        .strip_prefix("memory:")
        .unwrap()
        .split('@')
        .next()
        .unwrap();
    let mut input = fixture.vault.memory(id).unwrap().data;
    input.valid_from = Some(chrono::Utc::now() - chrono::Duration::days(1));
    fixture.vault.update_memory(id, 1, input).unwrap();
    let semantic = SemanticSearch::new(fixture.config("lexical")).unwrap();
    let context = Context::with_semantic(&fixture.vault, access(), &semantic);
    for field in ["target", "session_ref", "as_of"] {
        let mut query = args("lexical");
        match field {
            "target" => query.target = "events".into(),
            "session_ref" => query.session_ref = Some("does-not-match".into()),
            _ => query.as_of = Some(chrono::Utc::now() - chrono::Duration::days(2)),
        }
        let response = context.search(query).unwrap();
        fallback(&response, "no_eligible_memories");
        assert!(response["results"]
            .as_array()
            .unwrap()
            .iter()
            .all(|item| item["kind"] != "memory"));
    }
}

#[test]
fn busy_worker_falls_back_without_queueing_unbounded_work() {
    let _fixture_guard = fixture_process_guard();
    let fixture = Fixture::new();
    let mut config = fixture.config("lexical");
    let marker = fixture.path("busy-ready");
    fixture.fake(
        &mut config,
        &format!(
            "open({},'w').write('ready')\ntime.sleep(0.4)\n{ECHO}",
            serde_json::to_string(&marker).unwrap()
        ),
    );
    let semantic = SemanticSearch::new(config).unwrap();
    let context = Context::with_semantic(&fixture.vault, access(), &semantic);
    std::thread::scope(|scope| {
        let searching = scope.spawn(|| context.search(args("lexical")).unwrap());
        let start = Instant::now();
        while !marker.exists() {
            assert!(start.elapsed() < Duration::from_secs(10));
            std::thread::sleep(Duration::from_millis(5));
        }
        let start = Instant::now();
        fallback(&context.search(args("lexical")).unwrap(), "worker_busy");
        assert!(start.elapsed() < Duration::from_millis(200));
        assert_eq!(
            searching.join().unwrap()["coverage"]["semantic_search"],
            "available"
        );
    });
}

#[test]
fn stdin_backpressure_is_covered_by_the_same_timeout() {
    let _fixture_guard = fixture_process_guard();
    let fixture = Fixture::new();
    for number in 0..3 {
        let input: MemoryInput = serde_json::from_value(json!({"content":format!("另一份合成记忆 {number}"),"scope":"personal","source_refs":[fixture.source_id],"evidence":"user_explicit"})).unwrap();
        fixture.vault.add_memory(input).unwrap();
    }
    let mut config = fixture.config("lexical");
    let mut signature = String::new();
    fixture.edit_index(|index| {
        index["space"]["dimensions"] = json!(8192);
        signature = hash(&serde_json::to_vec(&index["space"]).unwrap());
        index["space_signature"] = json!(signature);
        let mut vector = vec![0.0; 8192];
        vector[0] = 1.0;
        for document in index["documents"].as_array_mut().unwrap() {
            document["vector"] = json!(vector);
        }
    });
    config.expected_space_signature = signature.clone();
    let mut vector = vec![0.0; 8192];
    vector[0] = 1.0;
    write_json(
        &fixture.path("queries.json"),
        &json!({"schema":"recallcard.embedding-query-cache/1","space_signature":signature,"queries":[{"query_hash":hash(b"lexical"),"input_hash":hash(b"lexical"),"vector":vector}]}),
    );
    config.timeout_ms = 100;
    fixture.fake(&mut config, "time.sleep(60)");
    let start = Instant::now();
    fallback(&fixture.search(config, "lexical"), "worker_timeout");
    assert!(start.elapsed() < Duration::from_secs(2));
}

#[test]
fn duplicate_nested_reply_fields_and_duplicate_cache_fields_are_rejected() {
    let _fixture_guard = fixture_process_guard();
    let fixture = Fixture::new();
    let mut config = fixture.config("lexical");
    fixture.fake(&mut config, &ECHO.replace("print(json.dumps({'id':r['id'],'ok':True,'result':result}),flush=True)","encoded=json.dumps({'id':r['id'],'ok':True,'result':result}); print(encoded.replace('\\\"ranking\\\": \\\"cosine\\\"','\\\"ranking\\\": \\\"cosine\\\", \\\"ranking\\\": \\\"cosine\\\"'),flush=True)"));
    fallback(&fixture.search(config, "lexical"), "malformed_response");
    let config = fixture.config("lexical");
    let bytes = fs::read_to_string(fixture.path("index.json")).unwrap();
    fs::write(
        fixture.path("index.json"),
        bytes.replacen(
            "\"complete\":true",
            "\"complete\":true,\"complete\":true",
            1,
        ),
    )
    .unwrap();
    fallback(&fixture.search(config, "lexical"), "invalid_cache");
}

#[test]
fn persistent_worker_is_reused_and_specific_failure_codes_are_safe() {
    let _fixture_guard = fixture_process_guard();
    let fixture = Fixture::new();
    let mut config = fixture.config("lexical");
    let starts = fixture.path("starts");
    fixture.fake(
        &mut config,
        &format!(
            "open({},'a').write('start\\n')\n{ECHO}",
            serde_json::to_string(&starts).unwrap()
        ),
    );
    let semantic = SemanticSearch::new(config).unwrap();
    let context = Context::with_semantic(&fixture.vault, access(), &semantic);
    for _ in 0..2 {
        assert_eq!(
            context.search(args("lexical")).unwrap()["coverage"]["semantic_search"],
            "available"
        );
    }
    assert_eq!(
        fs::read_to_string(&starts)
            .unwrap()
            .lines()
            .collect::<Vec<_>>(),
        vec!["start"]
    );
    for (worker_code, expected) in [
        ("missing_key", "key_unavailable"),
        ("network_budget_exhausted", "network_budget_exhausted"),
        ("provider_auth", "provider_auth"),
    ] {
        let mut config = fixture.config("lexical");
        fixture.fake(&mut config, &format!("r=json.loads(sys.stdin.readline())\nprint(json.dumps({{'id':r['id'],'ok':False,'error':{{'code':'{worker_code}','message':'隐藏供应商正文'}}}}),flush=True)"));
        let response = fixture.search(config, "lexical");
        fallback(&response, expected);
        assert!(!response.to_string().contains("隐藏供应商正文"));
    }
}

#[test]
fn cli_search_startup_config_recalls_nonlexical_memory() {
    let _fixture_guard = fixture_process_guard();
    let fixture = Fixture::new();
    let config = fixture.config("lexical");
    write_json(
        &fixture.path("startup.json"),
        &serde_json::to_value(config).unwrap(),
    );
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_recallcard"))
        .arg("--vault")
        .arg(fixture.vault.root())
        .args([
            "search",
            "lexical",
            "--scope",
            "personal",
            "--budget-tokens",
            "12000",
            "--semantic-config",
        ])
        .arg(fixture.path("startup.json"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let response: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(response["coverage"]["semantic_search"], "available");
    assert!(contains(&response, &fixture.memory_ref));
}

#[test]
fn cli_mcp_reuses_real_worker_query_cache_and_budget_with_fake_provider() {
    let _fixture_guard = fixture_process_guard();
    use std::io::Write;
    let fixture = Fixture::new();
    let mut config = fixture.config("unused");
    let original_python = config.python_path.clone();
    let starts = fixture.path("mcp-starts");
    let network_calls = fixture.path("fake-calls");
    // 使用真实 worker/EmbeddingClient 的缓存、批准和预算逻辑；唯一 provider transport 为内存 fake。
    fixture.fake(&mut config, &format!(
        "from . import protocol, embedding\nopen({},'a').write('start\\n')\nos.environ['RECALLCARD_SYNTHETIC_UNUSED_KEY']='synthetic-never-sent'\ndef fake_transport(endpoint, body, key, timeout, response_limit):\n    open({},'a').write('call\\n')\n    payload=json.loads(body)\n    return embedding.TransportResponse(200,embedding.canonical({{'model':payload['model'],'data':[{{'index':i,'embedding':[1.0,0.0]}} for i in range(len(payload['input']))],'usage':{{'prompt_tokens':1,'total_tokens':1}}}}),{{}})\noriginal=embedding.EmbeddingClient\nprotocol.EmbeddingClient=lambda *args,**kwargs: original(*args,transport=fake_transport,**kwargs)\nreturn protocol.main(argv)",
        serde_json::to_string(&starts).unwrap(), serde_json::to_string(&network_calls).unwrap()));
    fs::copy(
        original_python.join("recallcard_worker/embedding.py"),
        config.python_path.join("recallcard_worker/embedding.py"),
    )
    .unwrap();
    fs::copy(
        original_python.join("recallcard_worker/__main__.py"),
        config.python_path.join("recallcard_worker/protocol.py"),
    )
    .unwrap();
    config.query_cache_path = None;
    config.cloud_query = Some(CloudQueryApproval {
        endpoint: "https://synthetic.invalid/v1/embeddings".into(),
        scopes: vec!["personal".into()],
        key_env: "RECALLCARD_SYNTHETIC_UNUSED_KEY".into(),
        max_network_calls: 1,
        max_transmitted_bytes: 100,
    });
    write_json(
        &fixture.path("startup.json"),
        &serde_json::to_value(config).unwrap(),
    );
    let requests = [
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25"}}),
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"search","arguments":{"query":"lexical","budget_tokens":12000}}}),
        json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"search","arguments":{"query":"lexical","budget_tokens":12000}}}),
        json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"search","arguments":{"query":"lexical different","budget_tokens":12000}}}),
        json!({"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"search","arguments":{"query":"lexical","semantic_config":"/tmp/untrusted"}}}),
    ];
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_recallcard"))
        .arg("--vault")
        .arg(fixture.vault.root())
        .args(["mcp", "--scope", "personal", "--semantic-config"])
        .arg(fixture.path("startup.json"))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    for request in requests {
        serde_json::to_writer(&mut input, &request).unwrap();
        input.write_all(b"\n").unwrap();
    }
    drop(input);
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let responses = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(responses.len(), 5);
    for response in &responses[1..3] {
        assert_eq!(response["result"]["isError"], false, "{response}");
        let content: Value =
            serde_json::from_str(response["result"]["content"][0]["text"].as_str().unwrap())
                .unwrap();
        assert_eq!(
            content["coverage"]["semantic_search"], "available",
            "{content}"
        );
        assert!(contains(&content, &fixture.memory_ref));
    }
    let budget_response: Value = serde_json::from_str(
        responses[3]["result"]["content"][0]["text"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    fallback(&budget_response, "network_budget_exhausted");
    assert_eq!(responses[4]["result"]["isError"], true);
    assert_eq!(
        fs::read_to_string(starts)
            .unwrap()
            .lines()
            .collect::<Vec<_>>(),
        vec!["start"]
    );
    assert_eq!(
        fs::read_to_string(network_calls)
            .unwrap()
            .lines()
            .collect::<Vec<_>>(),
        vec!["call"]
    );
}

#[test]
fn memory_semantics_routes_only_current_authorized_navigation() {
    let _fixture_guard = fixture_process_guard();
    let fixture = Fixture::new();
    let query = "synthetic_nonlexical_route";
    let semantic = SemanticSearch::new(fixture.config(query)).unwrap();
    let context = Context::with_semantic(&fixture.vault, access(), &semantic);
    let mut request = args(query);
    request.target = "views".into();
    let response = context.search(request.clone()).unwrap();
    assert!(contains(&response, "view:nav/_unfiled"));
    assert_eq!(response["coverage"]["navigation"]["semantic_views"], false);
    assert_eq!(
        response["coverage"]["navigation"]["semantic_routing"],
        "via_current_memory_embeddings"
    );
    assert_eq!(response["coverage"]["semantic_search"], "available");
    assert!(response["results"]
        .as_array()
        .unwrap()
        .iter()
        .all(|row| row["kind"] == "view"));
    fixture
        .vault
        .suppress(&fixture.source_id, "合成来源撤回".into())
        .unwrap();
    let hidden = context.search(request).unwrap();
    assert!(!contains(&hidden, "view:nav/_unfiled"));
    assert_eq!(hidden["coverage"]["semantic_search"], "unavailable");
}
