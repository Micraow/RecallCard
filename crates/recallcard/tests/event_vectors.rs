//! Artificial 3-D cache only. Verifies plumbing, not model quality.
use recallcard::{context::Context, event_vectors, model::hash, policy::Access, transport, Vault};
use serde_json::{json, Value};
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Stdio},
};

fn space() -> Value {
    json!({"schema":"private-vector-cache-v1","endpoint":"https://embedding.invalid/v1/embeddings","model":"independent-toy-v1","dimensions":3})
}
fn bytes(hex: &str) -> Vec<u8> {
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
        .collect()
}
fn vector(cache: &Path, text: &str, values: [f64; 3]) {
    fs::create_dir_all(cache).unwrap();
    fs::write(cache.join("metadata.json"), space().to_string()).unwrap();
    let digest = hash(text.as_bytes());
    let binding=hash(b"{\"dimensions\": 3, \"endpoint\": \"https://embedding.invalid/v1/embeddings\", \"model\": \"independent-toy-v1\", \"schema\": \"private-vector-cache-v1\"}");
    let mut raw = b"RCVEC001".to_vec();
    raw.extend(bytes(&binding));
    raw.extend(bytes(&digest));
    for v in values {
        raw.extend(v.to_le_bytes());
    }
    fs::write(cache.join(format!("{digest}.bin")), raw).unwrap();
}
fn capture(
    v: &Vault,
    id: &str,
    parent: Option<&str>,
    role: &str,
    text: &str,
    scope: &str,
) -> String {
    let input = json!({"scope":scope,"role":role,"origin":"native","content":text,"source":{"platform":"manual","conversation_id":"one","message_id":id},
        "metadata":{"qwen":{"node_id":id,"nearest_visible_parent_id":parent,"on_current_path":true}}});
    format!(
        "event:{}",
        v.capture(serde_json::from_value(input).unwrap())
            .unwrap()
            .id
    )
}
fn chunk(reference: &str, text: &str, start: usize, end: usize) -> Value {
    json!({"ref":reference,"start_byte":start,"end_byte":end,"text_sha256":hash(&text.as_bytes()[start..end])})
}
fn import(v: &Vault, root: &Path, chunks: Vec<Value>) -> Result<Value, String> {
    fs::write(
        root.join("manifest.json"),
        json!({"schema":"recallcard.event-vector-manifest/1","space":space(),"chunks":chunks})
            .to_string(),
    )
    .unwrap();
    event_vectors::import(
        v,
        Access::new(vec!["personal".into()]).unwrap(),
        &root.join("manifest.json"),
        &root.join("cache"),
        &root.join("index.json"),
    )
}
fn search(v: &Vault, index: &event_vectors::EventVectorIndex, query: &str, detail: &str) -> Value {
    transport::invoke(
        &Context::new(v, Access::new(vec!["personal".into()]).unwrap())
            .with_event_vectors(Some(index)),
        "search",
        json!({"query":query,"target":"events","detail":detail,"budget_bytes":8192}),
    )
    .unwrap()
}

#[test]
fn offline_vectors_retrieve_nonlexical_chunk_and_expand_actual_source_graph() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(&d.path().join("vault")).unwrap();
    let cache = d.path().join("cache");
    let prior = capture(&v, "prior", None, "user", "本地要求", "personal");
    let text = format!("{}实质方案在这里", "无关长前文。".repeat(900));
    let start = text.find("实质").unwrap();
    let anchor = capture(&v, "anchor", Some("prior"), "assistant", &text, "personal");
    let later = capture(
        &v,
        "later",
        Some("anchor"),
        "user",
        "仍然禁止上传",
        "personal",
    );
    vector(&cache, &text[start..], [1., 0., 0.]);
    vector(&cache, "unrelated-language-question", [1., 0., 0.]);
    let report = import(&v, d.path(), vec![chunk(&anchor, &text, start, text.len())]).unwrap();
    assert_eq!(report["network_requests"], 0);
    let index = event_vectors::EventVectorIndex::open(&d.path().join("index.json")).unwrap();
    let result = search(&v, &index, "unrelated-language-question", "context");
    assert_eq!(result["results"][0]["ref"], anchor);
    assert_eq!(result["results"][0]["text_range"]["start_byte"], start);
    assert_eq!(result["coverage"]["event_vectors"]["status"], "available");
    let candidate = search(
        &v,
        &index,
        "unrelated-language-question",
        "session_candidates",
    );
    assert_eq!(candidate["results"][0]["read_args"]["offset_bytes"], start);
    let selected_read = transport::invoke(
        &Context::new(&v, Access::new(vec!["personal".into()]).unwrap()),
        "read",
        candidate["results"][0]["read_args"].clone(),
    )
    .unwrap();
    assert_eq!(
        selected_read["results"][0]["text_range"]["start_byte"],
        start
    );
    assert!(selected_read["results"][0]["text"]
        .as_str()
        .unwrap()
        .contains("实质方案在这里"));
    let result = search(&v, &index, "unrelated-language-question", "session_context");
    assert_eq!(result["results"][0]["text_range"]["start_byte"], start);
    let refs: Vec<_> = result["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["ref"].as_str().unwrap())
        .collect();
    assert!(refs.contains(&prior.as_str()) && refs.contains(&later.as_str()));
    assert_eq!(result["coverage"]["event_vectors"]["network_requests"], 0);
}

#[test]
fn uncached_queries_are_visible_lexical_fallback_even_with_no_matches_and_small_budget() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(&d.path().join("vault")).unwrap();
    let cache = d.path().join("cache");
    let r = capture(&v, "a", None, "user", "lighthouse", "personal");
    vector(&cache, "lighthouse", [1., 0., 0.]);
    import(&v, d.path(), vec![chunk(&r, "lighthouse", 0, 10)]).unwrap();
    let index = event_vectors::EventVectorIndex::open(&d.path().join("index.json")).unwrap();
    for detail in ["context", "session_context"] {
        let result = search(&v, &index, "unknown-query", detail);
        assert_eq!(result["status"], "no_matches");
        assert_eq!(
            result["coverage"]["event_vectors"]["error"],
            "event_query_not_cached_no_network_attempted"
        );
    }
    let result=transport::invoke(&Context::new(&v,Access::new(vec!["personal".into()]).unwrap()).with_event_vectors(Some(&index)),"search",
        json!({"query":"unknown-query","target":"events","detail":"session_context","budget_bytes":512})).unwrap();
    assert!(serde_json::to_vec(&result).unwrap().len() <= 512);
    assert_eq!(result["coverage"]["event_vectors"]["status"], "unavailable");
}

#[test]
fn import_rejects_wrong_model_hash_scope_range_and_corrupt_vectors_without_partial_output() {
    for failure in ["model", "hash", "scope", "range", "header", "norm"] {
        let d = tempfile::tempdir().unwrap();
        let v = Vault::init(&d.path().join("vault")).unwrap();
        let cache = d.path().join("cache");
        let r = capture(
            &v,
            "a",
            None,
            "user",
            "中文😀",
            if failure == "scope" {
                "project:secret"
            } else {
                "personal"
            },
        );
        vector(&cache, "中文😀", [1., 0., 0.]);
        let mut c = chunk(&r, "中文😀", 0, "中文😀".len());
        match failure {
            "model" => {
                let mut s = space();
                s["model"] = json!("different-model");
                fs::write(cache.join("metadata.json"), s.to_string()).unwrap();
            }
            "hash" => c["text_sha256"] = json!(hash(b"different")),
            "range" => c["start_byte"] = json!(1),
            "header" => {
                let p = cache.join(format!("{}.bin", hash("中文😀".as_bytes())));
                let mut b = fs::read(&p).unwrap();
                b[8] ^= 1;
                fs::write(p, b).unwrap();
            }
            "norm" => vector(&cache, "中文😀", [0., 0., 0.]),
            _ => {}
        }
        assert!(import(&v, d.path(), vec![c]).is_err(), "{failure}");
        assert!(!d.path().join("index.json").exists());
        assert!(!fs::read_dir(d.path()).unwrap().any(|entry| entry
            .unwrap()
            .path()
            .extension()
            .is_some_and(|ext| ext == "pack")));
    }
}

#[test]
fn permissions_role_filters_and_suppression_apply_before_loading_candidate_vectors() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(&d.path().join("vault")).unwrap();
    let cache = d.path().join("cache");
    let user = capture(&v, "user", None, "user", "copper", "personal");
    let assistant = capture(
        &v,
        "assistant",
        Some("user"),
        "assistant",
        "silver",
        "personal",
    );
    vector(&cache, "copper", [1., 0., 0.]);
    vector(&cache, "silver", [0., 1., 0.]);
    vector(&cache, "opaque-query", [0., 1., 0.]);
    import(
        &v,
        d.path(),
        vec![
            chunk(&user, "copper", 0, 6),
            chunk(&assistant, "silver", 0, 6),
        ],
    )
    .unwrap();
    let index = event_vectors::EventVectorIndex::open(&d.path().join("index.json")).unwrap();
    // Corrupt a now-excluded packed vector: filtered queries must not read it.
    let stored: Value =
        serde_json::from_slice(&fs::read(d.path().join("index.json")).unwrap()).unwrap();
    let pack_path = stored["pack"]["path"].as_str().unwrap();
    let offset = stored["pack"]["vectors"][hash(b"silver")]["offset"]
        .as_u64()
        .unwrap() as usize;
    let mut raw = fs::read(pack_path).unwrap();
    raw[offset + 72] ^= 1;
    fs::write(pack_path, raw).unwrap();
    let context = Context::new(&v, Access::new(vec!["personal".into()]).unwrap())
        .with_event_vectors(Some(&index));
    let result=transport::invoke(&context,"search",json!({"query":"opaque-query","target":"events","event_filter":{"role":"user"},"budget_bytes":8192})).unwrap();
    assert_eq!(result["results"][0]["ref"], user);
    assert_eq!(result["coverage"]["event_vectors"]["eligible_chunks"], 1);
    v.suppress(user.trim_start_matches("event:"), "toy suppression".into())
        .unwrap();
    let result=transport::invoke(&context,"search",json!({"query":"opaque-query","target":"events","event_filter":{"role":"user"},"budget_bytes":8192})).unwrap();
    assert!(result["results"].as_array().unwrap().is_empty());
    let denied = Context::new(&v, Access::new(vec!["project:elsewhere".into()]).unwrap())
        .with_event_vectors(Some(&index));
    let result = transport::invoke(
        &denied,
        "search",
        json!({"query":"opaque-query","target":"events","budget_bytes":8192}),
    )
    .unwrap();
    assert!(result["results"].as_array().unwrap().is_empty());
    for policy in ["semantic_only", "semantic_first"] {
        let index = event_vectors::EventVectorIndex::open(&d.path().join("index.json"))
            .unwrap()
            .with_ranking(policy)
            .unwrap();
        let denied = Context::new(&v, Access::new(vec!["project:other-user".into()]).unwrap())
            .with_event_vectors(Some(&index));
        let result = transport::invoke(
            &denied,
            "search",
            json!({"query":"opaque-query","target":"events","budget_bytes":8192}),
        )
        .unwrap();
        assert!(result["results"].as_array().unwrap().is_empty());
        assert_eq!(result["coverage"]["event_vectors"]["eligible_events"], 0);
    }
}

#[test]
fn duplicate_chunk_vectors_do_not_inflate_rank_and_mutated_index_fails_closed() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(&d.path().join("vault")).unwrap();
    let cache = d.path().join("cache");
    let text = "abcdef";
    let r = capture(&v, "a", None, "user", text, "personal");
    vector(&cache, "abc", [1., 0., 0.]);
    vector(&cache, "def", [0., 1., 0.]);
    vector(&cache, "opaque-query", [1., 0., 0.]);
    import(
        &v,
        d.path(),
        vec![chunk(&r, text, 0, 3), chunk(&r, text, 3, 6)],
    )
    .unwrap();
    let index = event_vectors::EventVectorIndex::open(&d.path().join("index.json")).unwrap();
    let page = search(&v, &index, "opaque-query", "context");
    assert_eq!(page["results"].as_array().unwrap().len(), 1);
    assert_eq!(page["results"][0]["matched_chunk_range"]["start_byte"], 0);
    let p = d.path().join("index.json");
    let mut value: Value = serde_json::from_slice(&fs::read(&p).unwrap()).unwrap();
    value["chunks"][0]["end_byte"] = json!(2);
    fs::write(p, value.to_string()).unwrap();
    assert_eq!(
        search(&v, &index, "opaque-query", "context")["coverage"]["event_vectors"]["error"],
        "event_vector_index_changed_restart_required"
    );
}

#[cfg(unix)]
#[test]
fn symlink_cache_vector_is_rejected_without_following_it() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(&d.path().join("vault")).unwrap();
    let cache = d.path().join("cache");
    let r = capture(&v, "a", None, "user", "copper", "personal");
    vector(&cache, "copper", [1., 0., 0.]);
    let p = cache.join(format!("{}.bin", hash(b"copper")));
    let other = d.path().join("vector-copy");
    fs::rename(&p, &other).unwrap();
    std::os::unix::fs::symlink(other, p).unwrap();
    assert!(import(&v, d.path(), vec![chunk(&r, "copper", 0, 6)])
        .unwrap_err()
        .contains("symlink"));
}

#[test]
fn actual_cli_import_and_mcp_reuse_vectors_without_provider_or_python_worker() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(&d.path().join("vault")).unwrap();
    let cache = d.path().join("cache");
    let r = capture(&v, "a", None, "user", "copper", "personal");
    vector(&cache, "copper", [1., 0., 0.]);
    vector(&cache, "opaque-query", [1., 0., 0.]);
    let manifest = d.path().join("manifest.json");
    let index = d.path().join("index.json");
    fs::write(&manifest,json!({"schema":"recallcard.event-vector-manifest/1","space":space(),"chunks":[chunk(&r,"copper",0,6)]}).to_string()).unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_recallcard"))
        .arg("--vault")
        .arg(v.root())
        .args(["event-vector-import", "--scope", "personal", "--manifest"])
        .arg(manifest)
        .arg("--cache-dir")
        .arg(cache)
        .arg("--output")
        .arg(&index)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let mut process = Command::new(env!("CARGO_BIN_EXE_recallcard"))
        .arg("--vault")
        .arg(v.root())
        .args(["mcp", "--scope", "personal", "--event-vector-index"])
        .arg(index)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = process.stdin.take().unwrap();
    for value in [
        json!({"jsonrpc":"2.0","id":1,"method":"initialize"}),
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"search","arguments":{"query":"opaque-query","target":"events","detail":"session_context","budget_bytes":8192}}}),
    ] {
        writeln!(input, "{value}").unwrap();
    }
    drop(input);
    let result = process.wait_with_output().unwrap();
    assert!(result.status.success());
    let reply: Value = serde_json::from_str(
        String::from_utf8(result.stdout)
            .unwrap()
            .lines()
            .last()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(reply["result"]["isError"], false);
    let page: Value =
        serde_json::from_str(reply["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(page["results"][0]["ref"], r);
    assert_eq!(page["coverage"]["event_vectors"]["status"], "available");
}

#[test]
fn explicit_new_query_vector_import_uses_separate_cache_and_exact_text_binding() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(&d.path().join("vault")).unwrap();
    let cache = d.path().join("cache");
    let r = capture(&v, "a", None, "user", "copper", "personal");
    vector(&cache, "copper", [1., 0., 0.]);
    import(&v, d.path(), vec![chunk(&r, "copper", 0, 6)]).unwrap();
    let query_cache = d.path().join("query-cache");
    fs::create_dir(&query_cache).unwrap();
    let query_file = d.path().join("query.txt");
    let vector_file = d.path().join("query-vector.json");
    fs::write(&query_file, "a-new-question").unwrap();
    fs::write(&vector_file,json!({"schema":"recallcard.query-vector/1","space":space(),"query_sha256":hash(b"a-new-question"),"vector":[1.,0.,0.]}).to_string()).unwrap();
    event_vectors::import_query(&query_file, &vector_file, &query_cache).unwrap();
    let index = event_vectors::EventVectorIndex::open(&d.path().join("index.json"))
        .unwrap()
        .with_query_cache(Some(&query_cache))
        .unwrap();
    assert_eq!(
        search(&v, &index, "a-new-question", "context")["results"][0]["ref"],
        r
    );
    assert!(!cache
        .join(format!("{}.bin", hash(b"a-new-question")))
        .exists());
    assert!(event_vectors::import_query(&query_file, &vector_file, &query_cache).is_err());
    fs::write(&query_file, "a-changed-question").unwrap();
    assert!(event_vectors::import_query(&query_file, &vector_file, &query_cache).is_err());
}

#[test]
fn managed_connection_revocation_also_blocks_local_event_vectors() {
    use recallcard::application::connections::{self, AgentReadService, ConnectionGrant};
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(&d.path().join("vault")).unwrap();
    let cache = d.path().join("cache");
    let r = capture(&v, "a", None, "user", "copper", "personal");
    vector(&cache, "copper", [1., 0., 0.]);
    vector(&cache, "opaque-query", [1., 0., 0.]);
    import(&v, d.path(), vec![chunk(&r, "copper", 0, 6)]).unwrap();
    let index = event_vectors::EventVectorIndex::open(&d.path().join("index.json")).unwrap();
    let grant = ConnectionGrant {
        client_kind: "chatgpt_mcp".into(),
        host_identity: "openai-chatgpt".into(),
        installation_id: None,
        platform: "chatgpt".into(),
        recall_scopes: vec!["personal".into()],
        capture_scopes: vec![],
        provider_disclosure: true,
        auto_capture: false,
        auto_recall: true,
    };
    let entry = connections::configure(&v, grant, None).unwrap();
    let access = Access::new(vec!["personal".into()]).unwrap();
    let service =
        AgentReadService::new(&v, &entry.id, &access, None).with_event_vectors(Some(&index));
    let args = json!({"query":"opaque-query","target":"events","detail":"session_context","budget_bytes":8192});
    assert_eq!(
        service.invoke("search", args.clone()).unwrap()["results"][0]["ref"],
        r
    );
    connections::revoke(&v, &entry.id, entry.permission_revision).unwrap();
    assert!(service.invoke("search", args).is_err());
}

#[test]
fn packed_and_original_cache_ranking_ranges_and_source_windows_are_identical() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(&d.path().join("vault")).unwrap();
    let cache = d.path().join("cache");
    let first = capture(&v, "a", None, "user", "copper", "personal");
    let second = capture(&v, "b", Some("a"), "assistant", "silver", "personal");
    vector(&cache, "copper", [1., 0., 0.]);
    vector(&cache, "silver", [0., 1., 0.]);
    vector(&cache, "opaque-query", [1., 0., 0.]);
    import(
        &v,
        d.path(),
        vec![
            chunk(&first, "copper", 0, 6),
            chunk(&second, "silver", 0, 6),
        ],
    )
    .unwrap();
    let packed = event_vectors::EventVectorIndex::open(&d.path().join("index.json")).unwrap();
    let mut legacy: Value =
        serde_json::from_slice(&fs::read(d.path().join("index.json")).unwrap()).unwrap();
    legacy.as_object_mut().unwrap().remove("pack");
    fs::write(d.path().join("legacy.json"), legacy.to_string()).unwrap();
    let legacy = event_vectors::EventVectorIndex::open(&d.path().join("legacy.json")).unwrap();
    for mode in ["brief", "context", "session_context"] {
        let a = search(&v, &packed, "opaque-query", mode);
        let b = search(&v, &legacy, "opaque-query", mode);
        for key in ["results", "sessions", "status", "next_cursor"] {
            assert_eq!(a[key], b[key], "{mode}: {key}");
        }
    }
}

#[test]
fn pack_out_of_bounds_size_hash_and_corruption_fail_closed() {
    for failure in ["offset", "size", "hash", "corrupt"] {
        let d = tempfile::tempdir().unwrap();
        let v = Vault::init(&d.path().join("vault")).unwrap();
        let cache = d.path().join("cache");
        let r = capture(&v, "a", None, "user", "copper", "personal");
        vector(&cache, "copper", [1., 0., 0.]);
        vector(&cache, "opaque-query", [1., 0., 0.]);
        import(&v, d.path(), vec![chunk(&r, "copper", 0, 6)]).unwrap();
        let path = d.path().join("index.json");
        let mut index: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        match failure {
            "offset" => index["pack"]["vectors"][hash(b"copper")]["offset"] = json!(u64::MAX),
            "size" => {
                let p = index["pack"]["path"].as_str().unwrap();
                fs::write(p, b"short").unwrap();
            }
            "hash" => index["pack"]["vectors"][hash(b"copper")]["sha256"] = json!(hash(b"wrong")),
            "corrupt" => {
                let p = index["pack"]["path"].as_str().unwrap();
                let mut raw = fs::read(p).unwrap();
                raw[72] ^= 1;
                fs::write(p, raw).unwrap();
            }
            _ => unreachable!(),
        }
        fs::write(&path, index.to_string()).unwrap();
        match event_vectors::EventVectorIndex::open(&path) {
            Err(_) => assert!(matches!(failure, "offset" | "size")),
            Ok(index) => {
                let page = search(&v, &index, "opaque-query", "context");
                assert_eq!(page["coverage"]["event_vectors"]["status"], "unavailable");
                assert!(page["results"].as_array().unwrap().is_empty());
            }
        }
    }
}

#[test]
fn explicit_semantic_policies_preserve_paraphrase_rank_without_rrf_agreement_bonus() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(&d.path().join("vault")).unwrap();
    let cache = d.path().join("cache");
    let paraphrase = capture(&v, "a", None, "user", "coastal bearings", "personal");
    let lexical = capture(&v, "b", None, "user", "telescope catalog", "personal");
    let extra = capture(&v, "c", None, "user", "telescope backup", "personal");
    vector(&cache, "coastal bearings", [1., 0., 0.]);
    vector(&cache, "telescope catalog", [0.6, 0.8, 0.]);
    vector(&cache, "telescope", [1., 0., 0.]);
    import(
        &v,
        d.path(),
        vec![
            chunk(&paraphrase, "coastal bearings", 0, 16),
            chunk(&lexical, "telescope catalog", 0, 17),
        ],
    )
    .unwrap();
    for policy in ["hybrid", "semantic_only", "semantic_first"] {
        let index = event_vectors::EventVectorIndex::open(&d.path().join("index.json"))
            .unwrap()
            .with_ranking(policy)
            .unwrap();
        let page = search(&v, &index, "telescope", "context");
        assert_eq!(page["coverage"]["event_vectors"]["ranking_policy"], policy);
        if policy == "hybrid" {
            assert_eq!(page["results"][0]["ref"], lexical);
        } else {
            assert_eq!(page["results"][0]["ref"], paraphrase);
            assert_eq!(page["results"][0]["score"], 1.0);
        }
        let extras = page["results"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["ref"] == extra);
        assert_eq!(extras, policy != "semantic_only");
    }
}

#[test]
fn rare_identifier_priority_requires_explicit_override_with_semantic_first() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(&d.path().join("vault")).unwrap();
    let cache = d.path().join("cache");
    let semantic = capture(&v, "a", None, "user", "museum index", "personal");
    let literal = capture(
        &v,
        "b",
        None,
        "user",
        "ZXQ-714 maintenance receipt",
        "personal",
    );
    vector(&cache, "museum index", [1., 0., 0.]);
    vector(&cache, "ZXQ-714", [1., 0., 0.]);
    import(&v, d.path(), vec![chunk(&semantic, "museum index", 0, 12)]).unwrap();
    let index = event_vectors::EventVectorIndex::open(&d.path().join("index.json"))
        .unwrap()
        .with_ranking("semantic_first")
        .unwrap();
    assert_eq!(
        search(&v, &index, "ZXQ-714", "context")["results"][0]["ref"],
        semantic
    );
    let ctx = Context::new(&v, Access::new(vec!["personal".into()]).unwrap())
        .with_event_vectors(Some(&index));
    let page = transport::invoke(&ctx, "search", json!({"query":"ZXQ-714","target":"events","lexical_match":"identifier_prefix","budget_bytes":8192})).unwrap();
    assert_eq!(page["results"][0]["ref"], literal);
    assert_eq!(
        page["coverage"]["event_vectors"]["lexical_identifier_override"],
        true
    );
    let index = index.with_ranking("semantic_only").unwrap();
    let ctx = Context::new(&v, Access::new(vec!["personal".into()]).unwrap())
        .with_event_vectors(Some(&index));
    assert!(transport::invoke(
        &ctx,
        "search",
        json!({"query":"ZXQ-714","target":"events","lexical_match":"identifier_prefix"})
    )
    .is_err());
    assert!(transport::invoke(&ctx, "search", json!({"query":"ZXQ-714","target":"all"})).is_err());
}

#[test]
fn semantic_policy_preserves_negation_and_explicit_user_role_without_claiming_entailment() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(&d.path().join("vault")).unwrap();
    let cache = d.path().join("cache");
    let user = capture(
        &v,
        "a",
        None,
        "user",
        "Do not publish the draft.",
        "personal",
    );
    let assistant = capture(
        &v,
        "b",
        Some("a"),
        "assistant",
        "Publish the draft.",
        "personal",
    );
    vector(&cache, "Do not publish the draft.", [0.8, 0.6, 0.]);
    vector(&cache, "Publish the draft.", [1., 0., 0.]);
    vector(&cache, "deployment permission", [1., 0., 0.]);
    import(
        &v,
        d.path(),
        vec![
            chunk(&user, "Do not publish the draft.", 0, 25),
            chunk(&assistant, "Publish the draft.", 0, 18),
        ],
    )
    .unwrap();
    for policy in ["semantic_only", "semantic_first"] {
        let index = event_vectors::EventVectorIndex::open(&d.path().join("index.json"))
            .unwrap()
            .with_ranking(policy)
            .unwrap();
        assert_eq!(
            search(&v, &index, "deployment permission", "context")["results"][0]["ref"],
            assistant
        );
        let ctx = Context::new(&v, Access::new(vec!["personal".into()]).unwrap())
            .with_event_vectors(Some(&index));
        let page=transport::invoke(&ctx,"search",json!({"query":"deployment permission","target":"events","detail":"session_context","event_filter":{"role":"user"},"budget_bytes":8192})).unwrap();
        assert_eq!(page["results"][0]["ref"], user);
        assert_eq!(page["results"][0]["role"], "user");
        assert_eq!(page["results"][0]["text"], "Do not publish the draft.");
        assert_eq!(page["coverage"]["event_vectors"]["eligible_events"], 1);
    }
}

#[test]
fn ranking_policy_binds_cursors_even_when_uncached_query_uses_identical_lexical_fallback() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(&d.path().join("vault")).unwrap();
    let cache = d.path().join("cache");
    let first = capture(&v, "a", None, "user", "metal bronze", "personal");
    let second = capture(&v, "b", None, "user", "metal tin", "personal");
    vector(&cache, "metal bronze", [1., 0., 0.]);
    vector(&cache, "metal tin", [0., 1., 0.]);
    import(
        &v,
        d.path(),
        vec![
            chunk(&first, "metal bronze", 0, 12),
            chunk(&second, "metal tin", 0, 9),
        ],
    )
    .unwrap();
    let index = event_vectors::EventVectorIndex::open(&d.path().join("index.json")).unwrap();
    let args = json!({"query":"metal","target":"events","limit":1,"budget_bytes":4096});
    let page = transport::invoke(
        &Context::new(&v, Access::new(vec!["personal".into()]).unwrap())
            .with_event_vectors(Some(&index)),
        "search",
        args.clone(),
    )
    .unwrap();
    assert!(page["next_cursor"].is_string());
    let index = index.with_ranking("semantic_only").unwrap();
    let ctx = Context::new(&v, Access::new(vec!["personal".into()]).unwrap())
        .with_event_vectors(Some(&index));
    let next = transport::invoke(&ctx, "search", args.clone()).unwrap();
    assert_eq!(page["results"], next["results"]);
    assert_eq!(
        next["coverage"]["event_vectors"]["ranking_policy"],
        "semantic_only"
    );
    assert_eq!(
        next["coverage"]["event_vectors"]["error"],
        "event_query_not_cached_no_network_attempted"
    );
    let mut continuation = args;
    continuation["cursor"] = page["next_cursor"].clone();
    assert!(transport::invoke(&ctx, "search", continuation).is_err());
}

#[test]
fn session_candidates_disclose_uncached_fallback_and_do_not_turn_cosine_into_confidence() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(&d.path().join("vault")).unwrap();
    let cache = d.path().join("cache");
    let text = "beacon original unrelated toy event";
    let anchor = capture(&v, "a", None, "user", text, "personal");
    vector(&cache, text, [1., 0., 0.]);
    vector(&cache, "opposite toy query", [-1., 0., 0.]);
    import(&v, d.path(), vec![chunk(&anchor, text, 0, text.len())]).unwrap();
    let index = event_vectors::EventVectorIndex::open(&d.path().join("index.json"))
        .unwrap()
        .with_ranking("semantic_only")
        .unwrap();
    let page = search(&v, &index, "opposite toy query", "session_candidates");
    assert_eq!(page["results"][0]["score"], -1.0);
    assert_eq!(page["results"][0]["score_kind"], "cosine");
    assert_eq!(page["results"][0]["relevance"], "unassessed");
    assert_eq!(page["coverage"]["indexed_eligible_event_count"], 1);
    let page = transport::invoke(&Context::new(&v, Access::new(vec!["personal".into()]).unwrap()).with_event_vectors(Some(&index)), "search", json!({"query":"uncached-missing-identifier","target":"events","detail":"session_candidates","budget_bytes":512})).unwrap();
    assert_eq!(page["status"], "no_matches");
    assert_eq!(
        page["coverage"]["event_vectors"]["error"],
        "event_query_not_cached_no_network_attempted"
    );
    assert_eq!(page["coverage"]["eligible_event_count"], 1);
    assert!(serde_json::to_vec(&page).unwrap().len() <= 512);
}
