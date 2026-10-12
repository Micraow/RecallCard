//! Original artificial 3-D vectors only. These tests verify bindings, never embedding quality.
use recallcard::{context::Context, event_vectors, model::hash, policy::Access, transport, Vault};
use serde_json::{json, Value};
use std::{fs, path::Path};

const LEGACY: &str = "legacy_raw_provider_preprocessing_unverified";
const QUERY: &str = "opaque query 😀\n";
const INSTRUCTION: &str = "Retrieve original passages relevant to this question.";

fn space() -> Value {
    json!({"schema":"private-vector-cache-v1","endpoint":"https://encoding-toy.invalid/v1/embeddings","model":"original-profile-toy","dimensions":3})
}
fn raw_profile() -> Value {
    json!({"kind":"raw/1"})
}
fn instruct_profile() -> Value {
    json!({"kind":"instruct_query/1","instruction":INSTRUCTION})
}
fn v2(query: &str, profile: Value, vector: [f64; 3]) -> Value {
    let encoded = if profile["kind"] == "raw/1" {
        query.to_owned()
    } else {
        format!(
            "Instruct: {}\nQuery: {query}",
            profile["instruction"].as_str().unwrap()
        )
    };
    json!({"schema":"recallcard.query-vector/2","space":space(),"query_sha256":hash(query.as_bytes()),
        "encoded_input_sha256":hash(encoded.as_bytes()),"encoding_profile":profile,"vector":vector})
}
fn v1(query: &str, vector: [f64; 3]) -> Value {
    json!({"schema":"recallcard.query-vector/1","space":space(),"query_sha256":hash(query.as_bytes()),"vector":vector})
}
fn import_query(root: &Path, query: &str, input: Value, cache: &Path) -> Result<Value, String> {
    let query_file = root.join("query.txt");
    let vector_file = root.join("query-vector.json");
    fs::write(&query_file, query).unwrap();
    fs::write(&vector_file, input.to_string()).unwrap();
    event_vectors::import_query(&query_file, &vector_file, cache)
}
fn legacy_vector(cache: &Path, text: &str, vector: [f64; 3]) {
    fs::create_dir_all(cache).unwrap();
    fs::write(cache.join("metadata.json"), space().to_string()).unwrap();
    let space_hash = hash(b"{\"dimensions\": 3, \"endpoint\": \"https://encoding-toy.invalid/v1/embeddings\", \"model\": \"original-profile-toy\", \"schema\": \"private-vector-cache-v1\"}");
    let text_hash = hash(text.as_bytes());
    let mut bytes = b"RCVEC001".to_vec();
    for digest in [&space_hash, &text_hash] {
        for at in (0..digest.len()).step_by(2) {
            bytes.push(u8::from_str_radix(&digest[at..at + 2], 16).unwrap());
        }
    }
    for value in vector {
        bytes.extend(value.to_le_bytes());
    }
    fs::write(cache.join(format!("{text_hash}.bin")), bytes).unwrap();
}
struct Fixture {
    root: tempfile::TempDir,
    vault: Vault,
    refs: Vec<String>,
}
impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let vault = Vault::init(&root.path().join("vault")).unwrap();
        let mut refs = Vec::new();
        let mut chunks = Vec::new();
        for (text, vector) in [
            ("cobalt prism", [1., 0., 0.]),
            ("velvet orbit", [0., 1., 0.]),
        ] {
            let event = vault
                .capture(
                    serde_json::from_value(
                        json!({"scope":"personal","role":"user","origin":"native","content":text,
                "source":{"platform":"manual","conversation_id":text,"message_id":"toy"}}),
                    )
                    .unwrap(),
                )
                .unwrap();
            let reference = format!("event:{}", event.id);
            refs.push(reference.clone());
            chunks.push(json!({"ref":reference,"start_byte":0,"end_byte":text.len(),"text_sha256":hash(text.as_bytes())}));
            legacy_vector(&root.path().join("source"), text, vector);
        }
        legacy_vector(&root.path().join("source"), QUERY, [1., 0., 0.]);
        let manifest = root.path().join("manifest.json");
        fs::write(
            &manifest,
            json!({"schema":"recallcard.event-vector-manifest/1","space":space(),"chunks":chunks})
                .to_string(),
        )
        .unwrap();
        event_vectors::import(
            &vault,
            Access::new(vec!["personal".into()]).unwrap(),
            &manifest,
            &root.path().join("source"),
            &root.path().join("index.json"),
        )
        .unwrap();
        Self { root, vault, refs }
    }
    fn cache(&self, name: &str) -> std::path::PathBuf {
        let path = self.root.path().join(name);
        fs::create_dir(&path).unwrap();
        path
    }
    fn index(&self, cache: Option<&Path>) -> event_vectors::EventVectorIndex {
        event_vectors::EventVectorIndex::open(&self.root.path().join("index.json"))
            .unwrap()
            .with_query_cache(cache)
            .unwrap()
            .with_ranking("semantic_only")
            .unwrap()
    }
    fn search(&self, index: &event_vectors::EventVectorIndex, query: &str) -> Value {
        transport::invoke(
            &Context::new(&self.vault, Access::new(vec!["personal".into()]).unwrap())
                .with_event_vectors(Some(index)),
            "search",
            json!({"query":query,"target":"events","budget_bytes":8192}),
        )
        .unwrap()
    }
    fn import(&self, cache: &Path, query: &str, input: Value) -> Result<Value, String> {
        import_query(self.root.path(), query, input, cache)
    }
}
fn read_json(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}
fn entry(cache: &Path, query: &str) -> std::path::PathBuf {
    cache.join(format!("{}.query-v2.json", hash(query.as_bytes())))
}

#[test]
fn same_query_profiles_are_isolated_and_preserve_original_space_and_cache() {
    let f = Fixture::new();
    let source = f.root.path().join("source");
    let original: Vec<_> = fs::read_dir(&source)
        .unwrap()
        .map(|e| {
            let p = e.unwrap().path();
            (p.clone(), fs::read(p).unwrap())
        })
        .collect();
    let raw_cache = f.cache("raw-query");
    let instruct_cache = f.cache("instruct-query");
    let raw = f
        .import(&raw_cache, QUERY, v2(QUERY, raw_profile(), [1., 0., 0.]))
        .unwrap();
    let instruct = f
        .import(
            &instruct_cache,
            QUERY,
            v2(QUERY, instruct_profile(), [0., 1., 0.]),
        )
        .unwrap();
    assert_eq!(raw["query_encoding_profile"], "raw/1");
    assert_eq!(instruct["query_encoding_profile"], "instruct_query/1");
    assert_ne!(
        raw["query_encoding_profile_sha256"],
        instruct["query_encoding_profile_sha256"]
    );
    for (cache, expected, kind) in [
        (&raw_cache, &f.refs[0], "raw/1"),
        (&instruct_cache, &f.refs[1], "instruct_query/1"),
    ] {
        let result = f.search(&f.index(Some(cache)), QUERY);
        assert_eq!(&result["results"][0]["ref"], expected);
        let coverage = &result["coverage"]["event_vectors"];
        assert_eq!(coverage["query_encoding_profile"], kind);
        assert_eq!(coverage["query_cache_selection"], "explicit_query_cache");
        assert_eq!(coverage["query_sha256"], hash(QUERY.as_bytes()));
        assert_eq!(coverage["network_requests"], 0);
        assert_eq!(
            coverage["query_input_binding"],
            "declared_input_hashes_validated_provider_preprocessing_unverified"
        );
        assert_eq!(read_json(&cache.join("profile.json"))["space"], space());
        assert!(!cache
            .join(format!("{}.bin", hash(QUERY.as_bytes())))
            .exists());
    }
    let stored = read_json(&entry(&instruct_cache, QUERY));
    assert_eq!(
        stored["encoded_input_sha256"],
        hash(format!("Instruct: {INSTRUCTION}\nQuery: {QUERY}").as_bytes())
    );
    assert_ne!(stored["encoded_input_sha256"], stored["query_sha256"]);
    assert_eq!(
        read_json(&entry(&raw_cache, QUERY))["encoded_input_sha256"],
        hash(QUERY.as_bytes())
    );
    assert_eq!(fs::read_dir(&source).unwrap().count(), original.len());
    for (path, bytes) in original {
        assert_eq!(fs::read(path).unwrap(), bytes);
    }
}

#[test]
fn legacy_query_import_and_source_fallback_stay_compatible_and_explicitly_unverified() {
    let f = Fixture::new();
    let cache = f.cache("legacy-query");
    let imported = f
        .import(&cache, "another query", v1("another query", [0., 1., 0.]))
        .unwrap();
    assert_eq!(imported["query_encoding_profile"], LEGACY);
    assert!(!cache.join("profile.json").exists());
    assert!(cache
        .join(format!("{}.bin", hash(b"another query")))
        .exists());
    let index = f.index(Some(&cache));
    for (query, expected, selection) in [
        ("another query", &f.refs[1], "explicit_query_cache"),
        (QUERY, &f.refs[0], "source_cache_legacy"),
    ] {
        let result = f.search(&index, query);
        assert_eq!(&result["results"][0]["ref"], expected);
        assert_eq!(
            result["coverage"]["event_vectors"]["query_encoding_profile"],
            LEGACY
        );
        assert_eq!(
            result["coverage"]["event_vectors"]["query_cache_selection"],
            selection
        );
        assert!(result["coverage"]["event_vectors"]["encoded_input_sha256"].is_null());
    }
    assert_eq!(
        f.search(&f.index(None), QUERY)["coverage"]["event_vectors"]["query_encoding_profile"],
        LEGACY
    );
}

#[test]
fn profiled_missing_queries_never_read_legacy_source_or_selected_bin_files() {
    let f = Fixture::new();
    for (name, profile) in [
        ("raw-query", raw_profile()),
        ("instruct-query", instruct_profile()),
    ] {
        let cache = f.cache(name);
        f.import(
            &cache,
            "different query",
            v2("different query", profile, [0., 1., 0.]),
        )
        .unwrap();
        // A planted legacy .bin is isolated; source also contains a valid vector for QUERY.
        legacy_vector(&cache, QUERY, [1., 0., 0.]);
        let result = f.search(&f.index(Some(&cache)), QUERY);
        assert_eq!(
            result["coverage"]["event_vectors"]["error"],
            "event_query_not_cached_no_network_attempted"
        );
        assert!(result["results"].as_array().unwrap().is_empty());
    }
}

#[test]
fn malformed_v2_input_fails_before_any_cache_writes() {
    for failure in [
        "query_hash",
        "encoded_hash",
        "missing_hash",
        "blank_instruction",
        "newline_instruction",
        "long_instruction",
        "unknown_profile",
        "raw_extra_field",
        "unknown_schema",
        "extra_field",
        "norm",
        "dimensions",
    ] {
        let root = tempfile::tempdir().unwrap();
        let cache = root.path().join("cache");
        fs::create_dir(&cache).unwrap();
        let mut input = v2(QUERY, instruct_profile(), [1., 0., 0.]);
        match failure {
            "query_hash" => input["query_sha256"] = json!(hash(b"wrong")),
            "encoded_hash" => input["encoded_input_sha256"] = json!(hash(QUERY.as_bytes())),
            "missing_hash" => {
                input
                    .as_object_mut()
                    .unwrap()
                    .remove("encoded_input_sha256");
            }
            "blank_instruction" => input["encoding_profile"]["instruction"] = json!("  "),
            "newline_instruction" => {
                input["encoding_profile"]["instruction"] = json!("first\nsecond")
            }
            "long_instruction" => {
                input["encoding_profile"]["instruction"] = json!("x".repeat(4097))
            }
            "unknown_profile" => input["encoding_profile"]["kind"] = json!("arbitrary-template/1"),
            "raw_extra_field" => input["encoding_profile"]["kind"] = json!("raw/1"),
            "unknown_schema" => input["schema"] = json!("recallcard.query-vector/3"),
            "extra_field" => input["template"] = json!("unbound"),
            "norm" => input["vector"] = json!([0., 0., 0.]),
            "dimensions" => input["space"]["dimensions"] = json!(4),
            _ => unreachable!(),
        }
        assert!(
            import_query(root.path(), QUERY, input, &cache).is_err(),
            "{failure}"
        );
        assert_eq!(fs::read_dir(&cache).unwrap().count(), 0, "{failure}");
    }
}

#[test]
fn cache_profiles_reject_mixing_relabeling_and_overwriting() {
    let f = Fixture::new();
    let cache = f.cache("profiled");
    let input = v2(QUERY, raw_profile(), [1., 0., 0.]);
    f.import(&cache, QUERY, input.clone()).unwrap();
    let before = fs::read(entry(&cache, QUERY)).unwrap();
    assert_eq!(
        f.import(&cache, QUERY, input).unwrap_err(),
        "event_query_vector_exists_or_failed"
    );
    for input in [
        v1(QUERY, [0., 1., 0.]),
        v2(QUERY, instruct_profile(), [0., 1., 0.]),
    ] {
        assert_eq!(
            f.import(&cache, QUERY, input).unwrap_err(),
            "event_query_cache_profile_mismatch"
        );
    }
    assert_eq!(fs::read(entry(&cache, QUERY)).unwrap(), before);
    let legacy = f.cache("legacy");
    f.import(&legacy, QUERY, v1(QUERY, [1., 0., 0.])).unwrap();
    assert_eq!(
        f.import(&legacy, QUERY, v2(QUERY, raw_profile(), [1., 0., 0.]))
            .unwrap_err(),
        "event_query_profile_requires_empty_cache"
    );
    assert!(!legacy.join("profile.json").exists());
    assert_eq!(
        f.import(
            &f.root.path().join("source"),
            QUERY,
            v2(QUERY, instruct_profile(), [1., 0., 0.])
        )
        .unwrap_err(),
        "event_query_profile_requires_empty_cache"
    );
}

#[test]
fn reading_rechecks_query_input_profile_space_and_vector_bindings() {
    for failure in [
        "query",
        "encoded",
        "profile",
        "instruction",
        "model",
        "endpoint",
        "norm",
        "version",
    ] {
        let f = Fixture::new();
        let cache = f.cache("profiled");
        f.import(&cache, QUERY, v2(QUERY, instruct_profile(), [0., 1., 0.]))
            .unwrap();
        let index = f.index(Some(&cache));
        let path = entry(&cache, QUERY);
        let mut input = read_json(&path);
        match failure {
            "query" => input["query_sha256"] = json!(hash(b"different query")),
            "encoded" => input["encoded_input_sha256"] = json!(hash(b"different input")),
            "profile" => input = v2(QUERY, raw_profile(), [1., 0., 0.]),
            "instruction" => {
                input = v2(
                    QUERY,
                    json!({"kind":"instruct_query/1","instruction":"A different task."}),
                    [1., 0., 0.],
                )
            }
            "model" => input["space"]["model"] = json!("different-model"),
            "endpoint" => {
                input["space"]["endpoint"] = json!("https://other-toy.invalid/v1/embeddings")
            }
            "norm" => input["vector"] = json!([2., 0., 0.]),
            "version" => input = v1(QUERY, [1., 0., 0.]),
            _ => unreachable!(),
        }
        fs::write(path, input.to_string()).unwrap();
        let result = f.search(&index, QUERY);
        assert_eq!(
            result["coverage"]["event_vectors"]["status"], "unavailable",
            "{failure}"
        );
        assert!(
            result["results"].as_array().unwrap().is_empty(),
            "{failure}"
        );
    }
}

#[test]
fn profile_changes_and_missing_or_invalid_declarations_fail_closed() {
    for failure in ["changed", "deleted", "broken_json", "schema", "model"] {
        let f = Fixture::new();
        let cache = f.cache("profiled");
        f.import(&cache, QUERY, v2(QUERY, raw_profile(), [1., 0., 0.]))
            .unwrap();
        let index = f.index(Some(&cache));
        let path = cache.join("profile.json");
        let mut profile = read_json(&path);
        match failure {
            "changed" => {
                profile["encoding_profile"] = instruct_profile();
                fs::write(&path, profile.to_string()).unwrap();
            }
            "deleted" => fs::remove_file(&path).unwrap(),
            "broken_json" => fs::write(&path, "{").unwrap(),
            "schema" => {
                profile["schema"] = json!("recallcard.query-cache-profile/2");
                fs::write(&path, profile.to_string()).unwrap();
            }
            "model" => {
                profile["space"]["model"] = json!("other-model");
                fs::write(&path, profile.to_string()).unwrap();
            }
            _ => unreachable!(),
        }
        let result = f.search(&index, QUERY);
        assert_eq!(
            result["coverage"]["event_vectors"]["status"], "unavailable",
            "{failure}"
        );
        assert!(
            result["results"].as_array().unwrap().is_empty(),
            "{failure}"
        );
        if failure != "changed" {
            assert!(
                event_vectors::EventVectorIndex::open(&f.root.path().join("index.json"))
                    .unwrap()
                    .with_query_cache(Some(&cache))
                    .is_err(),
                "{failure}"
            );
        }
    }
}

#[test]
fn a_profile_for_a_different_source_space_cannot_be_selected() {
    let f = Fixture::new();
    let cache = f.cache("different-space");
    let mut input = v2(QUERY, raw_profile(), [1., 0., 0.]);
    input["space"]["model"] = json!("separate-toy-model");
    f.import(&cache, QUERY, input).unwrap();
    assert!(
        event_vectors::EventVectorIndex::open(&f.root.path().join("index.json"))
            .unwrap()
            .with_query_cache(Some(&cache))
            .is_err()
    );
}

#[cfg(unix)]
#[test]
fn profile_and_query_entry_symlinks_are_rejected() {
    for name in ["profile", "entry"] {
        let f = Fixture::new();
        let cache = f.cache("profiled");
        f.import(&cache, QUERY, v2(QUERY, raw_profile(), [1., 0., 0.]))
            .unwrap();
        let index = f.index(Some(&cache));
        let path = if name == "profile" {
            cache.join("profile.json")
        } else {
            entry(&cache, QUERY)
        };
        let other = f.root.path().join("other-file.json");
        fs::rename(&path, &other).unwrap();
        std::os::unix::fs::symlink(&other, path).unwrap();
        let result = f.search(&index, QUERY);
        assert!(result["coverage"]["event_vectors"]["error"]
            .as_str()
            .unwrap()
            .contains("symlink"));
        assert!(result["results"].as_array().unwrap().is_empty());
    }
}

#[test]
fn switching_profile_invalidates_cursor_even_when_ranking_is_identical() {
    let f = Fixture::new();
    let raw = f.cache("raw");
    let instruct = f.cache("instruct");
    f.import(&raw, QUERY, v2(QUERY, raw_profile(), [1., 0., 0.]))
        .unwrap();
    f.import(
        &instruct,
        QUERY,
        v2(QUERY, instruct_profile(), [1., 0., 0.]),
    )
    .unwrap();
    let raw_index = f.index(Some(&raw));
    let instruct_index = f.index(Some(&instruct));
    let args = json!({"query":QUERY,"target":"events","limit":1,"budget_bytes":8192});
    let context = |index| {
        Context::new(&f.vault, Access::new(vec!["personal".into()]).unwrap())
            .with_event_vectors(Some(index))
    };
    let first = transport::invoke(&context(&raw_index), "search", args.clone()).unwrap();
    let second = transport::invoke(&context(&instruct_index), "search", args.clone()).unwrap();
    assert_eq!(first["results"], second["results"]);
    assert!(first["next_cursor"].is_string());
    let mut continuation = args;
    continuation["cursor"] = first["next_cursor"].clone();
    assert!(transport::invoke(&context(&instruct_index), "search", continuation).is_err());
}

#[test]
fn concurrent_legacy_and_profiled_imports_never_commit_mixed_cache_contracts() {
    for different_space in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let cache = root.path().join("cache");
        fs::create_dir(&cache).unwrap();
        let mut old = v1("first toy query", [1., 0., 0.]);
        if different_space {
            old["space"]["model"] = json!("different-space-toy");
        }
        let inputs = [
            ("first toy query", old),
            (
                "second toy query",
                v2("second toy query", raw_profile(), [0., 1., 0.]),
            ),
        ];
        let barrier = std::sync::Barrier::new(2);
        let results = std::thread::scope(|scope| {
            let handles = inputs
                .into_iter()
                .enumerate()
                .map(|(i, (query, entry))| {
                    let work = root.path().join(format!("writer-{i}"));
                    fs::create_dir(&work).unwrap();
                    let q = work.join("query.txt");
                    let v = work.join("vector.json");
                    fs::write(&q, query).unwrap();
                    fs::write(&v, entry.to_string()).unwrap();
                    let b = &barrier;
                    let c = &cache;
                    scope.spawn(move || {
                        b.wait();
                        event_vectors::import_query(&q, &v, c)
                    })
                })
                .collect::<Vec<_>>();
            handles
                .into_iter()
                .map(|h| h.join().unwrap())
                .collect::<Vec<_>>()
        });
        assert_eq!(
            results.iter().filter(|r| r.is_ok()).count(),
            1,
            "{results:?}"
        );
        let metadata: Value =
            serde_json::from_slice(&fs::read(cache.join("metadata.json")).unwrap()).unwrap();
        if cache.join("profile.json").exists() {
            let profile: Value =
                serde_json::from_slice(&fs::read(cache.join("profile.json")).unwrap()).unwrap();
            assert_eq!(profile["space"], metadata);
            assert!(!cache
                .join(format!("{}.bin", hash(b"first toy query")))
                .exists());
        } else {
            assert!(!cache
                .join(format!("{}.query-v2.json", hash(b"second toy query")))
                .exists());
        }
    }
}
#[test]
fn held_import_lock_reports_busy_without_publishing_any_declaration() {
    let root = tempfile::tempdir().unwrap();
    let cache = root.path().join("cache");
    fs::create_dir(&cache).unwrap();
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(cache.join(".query-import.lock"))
        .unwrap();
    lock.try_lock().unwrap();
    assert_eq!(
        import_query(
            root.path(),
            QUERY,
            v2(QUERY, raw_profile(), [1., 0., 0.]),
            &cache
        )
        .unwrap_err(),
        "event_query_import_busy"
    );
    assert_eq!(fs::read_dir(&cache).unwrap().count(), 1);
    lock.unlock().unwrap();
    drop(lock);
    import_query(
        root.path(),
        QUERY,
        v2(QUERY, raw_profile(), [1., 0., 0.]),
        &cache,
    )
    .unwrap();
}
