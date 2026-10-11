//! 桌面表单参数→私有配置/空索引→真实后台→桌面/安装Native读取；合成数据与回环向量。
use recallcard::{
    application::{
        background_memory::MemoryRuntime,
        connections::{self, ConnectionGrant},
        runtime,
    },
    desktop::DesktopSession,
    semantic::LocalSemanticSetup,
    Vault,
};
use serde_json::{json, Value};
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    path::Path,
    time::{Duration, Instant},
};
const EXT: &str = "abcdefghijklmnopabcdefghijklmnop";
const INSTALL: &str = "11111111-1111-4111-8111-111111111111";
fn settings(endpoint: &str) -> LocalSemanticSetup {
    LocalSemanticSetup {
        endpoint: endpoint.into(),
        model: "synthetic-v1".into(),
        dimensions: 2,
        revision: "v1".into(),
        query_instruction: String::new(),
        document_instruction: String::new(),
        query_approved: true,
        memory_approved: true,
        resume_background: false,
    }
}
fn provider(
    count: usize,
    callback: impl Fn(usize) + Send + 'static,
) -> (String, std::thread::JoinHandle<Vec<Value>>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}/v1/embeddings", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        listener.set_nonblocking(true).unwrap();
        let mut calls = vec![];
        for n in 0..count {
            let deadline = Instant::now() + Duration::from_secs(20);
            let mut stream = loop {
                match listener.accept() {
                    Ok((s, _)) => break s,
                    Err(e)
                        if e.kind() == std::io::ErrorKind::WouldBlock
                            && Instant::now() < deadline =>
                    {
                        std::thread::sleep(Duration::from_millis(10))
                    }
                    Err(e) => panic!("provider was not called: {e}"),
                }
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            assert_eq!(line, "POST /v1/embeddings HTTP/1.1\r\n");
            let mut length = 0;
            loop {
                line.clear();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" {
                    break;
                }
                assert!(!line.to_ascii_lowercase().starts_with("authorization:"));
                if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = v.trim().parse().unwrap();
                }
            }
            assert!(length < 65536);
            let mut raw = vec![0; length];
            reader.read_exact(&mut raw).unwrap();
            let request: Value = serde_json::from_slice(&raw).unwrap();
            callback(n);
            let rows = request["input"]
                .as_array()
                .unwrap()
                .iter()
                .enumerate()
                .map(|(i, _)| json!({"index":i,"embedding":[1.,0.]}))
                .collect::<Vec<_>>();
            let body = serde_json::to_vec(&json!({"model":request["model"],"data":rows})).unwrap();
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            stream.write_all(&body).unwrap();
            calls.push(request);
        }
        calls
    });
    (endpoint, server)
}
struct Stop<'a>(&'a Vault);
impl Drop for Stop<'_> {
    fn drop(&mut self) {
        let _ = runtime::request_service_stop(self.0);
        let until = Instant::now() + Duration::from_secs(8);
        while runtime::service_status(self.0).is_ok_and(|s| s.running) && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(25));
        }
    }
}
struct Native(std::process::Child);
impl Native {
    fn read(&mut self, id: &str, action: &str, args: Value) -> Value {
        let request = json!({"protocol":"recallcard.action/1","request_id":id,"nonce":"desktop-setup-native-synthetic-nonce","session_ref":"chatgpt:desktop-setup","installation_id":INSTALL,"action":"authorized_read","arguments":{"action":action,"arguments":args,"permission_revision":1}});
        let raw = serde_json::to_vec(&request).unwrap();
        let input = self.0.stdin.as_mut().unwrap();
        input.write_all(&(raw.len() as u32).to_ne_bytes()).unwrap();
        input.write_all(&raw).unwrap();
        input.flush().unwrap();
        let out = self.0.stdout.as_mut().unwrap();
        let mut prefix = [0; 4];
        out.read_exact(&mut prefix).unwrap();
        let mut bytes = vec![0; u32::from_ne_bytes(prefix) as usize];
        out.read_exact(&mut bytes).unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }
}
impl Drop for Native {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
#[test]
fn desktop_form_builds_private_seed_starts_index_only_service_and_wires_search_and_native() {
    let dir = tempfile::tempdir().unwrap();
    let v = Vault::init(&dir.path().join("vault")).unwrap();
    let e=v.capture(serde_json::from_value(json!({"role":"user","scope":"personal","origin":"native","content":"合成这次远足仅半天，不是长期习惯","source":{"platform":"synthetic","conversation_id":"desktop","message_id":"one"}})).unwrap()).unwrap();
    let memory=v.add_memory(serde_json::from_value(json!({"content":"合成本次半日远足约束","scope":"personal","source_refs":[e.id],"evidence":"user_explicit","navigation":[{"path":"topics/hiking"}]})).unwrap()).unwrap();
    let mut desktop = DesktopSession::default();
    let selected = desktop.select_vault(v.root(), false).unwrap();
    let handle = desktop
        .model_operation(&selected.session_id, "personal")
        .unwrap();
    let source_root = v.root().to_owned();
    let source_id = e.id.clone();
    let (endpoint, server) = provider(3, move |n| {
        if n == 2 {
            Vault::open_existing(&source_root)
                .unwrap()
                .suppress(&source_id, "合成查询等待期间撤权".into())
                .unwrap();
        }
    });
    let _stop = Stop(&v);
    // 模拟安装包固定资源路径，并沿正常宿主注册/哈希核验，不放宽 CLI 校验。
    let resources = dir.path().join("resources");
    let component = resources.join("说明与许可证");
    fs::create_dir_all(&component).unwrap();
    let binary = component.join(if cfg!(windows) {
        "recallcard.exe"
    } else {
        "recallcard"
    });
    fs::copy(env!("CARGO_BIN_EXE_recallcard"), &binary).unwrap();
    let installed_worker = component.join("python/recallcard_worker");
    fs::create_dir_all(&installed_worker).unwrap();
    for entry in
        fs::read_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../python/recallcard_worker"))
            .unwrap()
    {
        let entry = entry.unwrap();
        if entry.path().extension().is_some_and(|v| v == "py") {
            fs::copy(entry.path(), installed_worker.join(entry.file_name())).unwrap();
        }
    }
    let binary = runtime::register_packaged_cli(&binary, &resources).unwrap();
    let saved = handle
        .configure_semantic_with_binary(settings(&endpoint), &binary)
        .unwrap();
    assert_eq!(saved.state, "configured");
    assert!(saved.service_error.is_none(), "{:?}", saved.service_error);
    assert!(saved.service.unwrap().running);
    let cfg = MemoryRuntime::new(&v).configuration().unwrap();
    assert!(!cfg.enabled, "语义设置不能顺手启用 LLM 整理");
    assert!(cfg.index_maintenance_enabled);
    assert!(cfg.provider.is_none());
    let path = cfg.candidate_semantic_config.unwrap();
    assert!(path.starts_with(v.state_dir().unwrap()));
    let trusted: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(trusted["python_path"], json!(component.join("python")));
    assert!(trusted["cloud_query"].is_null());
    assert_eq!(trusted["local_index"]["scopes"], json!(["personal"]));
    let until = Instant::now() + Duration::from_secs(15);
    while MemoryRuntime::new(&v)
        .status()
        .unwrap()
        .index_refresh
        .unwrap()
        .state
        != "ready"
    {
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(25));
    }
    let result = handle
        .search("synthetic new equipment wording", "memories")
        .unwrap();
    assert_eq!(
        result["coverage"]["semantic_query_encoding"],
        "local_encoded"
    );
    assert_eq!(
        result["results"][0]["ref"],
        format!("memory:{}@1", memory.id)
    );
    assert_eq!(
        result["results"][0]["source_summary"]["sources"][0]["role"],
        "user"
    );
    // 重复查询读取缓存；末尾断言不会多次调用提供器。
    assert_eq!(
        desktop
            .search(
                &selected.session_id,
                "personal",
                "synthetic new equipment wording",
                "memories"
            )
            .unwrap()["coverage"]["semantic_query_encoding"],
        "cached"
    );
    let installed = desktop
        .prepare_browser_connection(
            &selected.session_id,
            "personal",
            EXT,
            &dir.path().join("native"),
            Path::new(env!("CARGO_BIN_EXE_recallcard")),
            false,
        )
        .unwrap();
    let native_config: Value =
        serde_json::from_slice(&fs::read(installed["config"].as_str().unwrap()).unwrap()).unwrap();
    assert_eq!(native_config["semantic_config"], json!(path));
    assert_eq!(installed["registered"], false);
    connections::configure(
        &v,
        ConnectionGrant {
            client_kind: "browser".into(),
            host_identity: EXT.into(),
            installation_id: Some(INSTALL.into()),
            platform: "chatgpt".into(),
            recall_scopes: vec!["personal".into()],
            capture_scopes: vec![],
            provider_disclosure: true,
            auto_capture: false,
            auto_recall: true,
        },
        None,
    )
    .unwrap();
    let mut native = Native(
        std::process::Command::new(installed["launcher"].as_str().unwrap())
            .arg(format!("chrome-extension://{EXT}/"))
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap(),
    );
    let found = native.read(
        "find",
        "search",
        json!({"query":"synthetic new equipment wording","target":"views","budget_bytes":8192}),
    );
    assert_eq!(found["ok"], true, "{found}");
    assert_eq!(
        found["result"]["coverage"]["semantic_query_encoding"],
        "cached"
    );
    let view = found["result"]["results"][0]["ref"].clone();
    let page = native.read("dir", "read", json!({"refs":[view],"budget_bytes":8192}));
    let reference = page["result"]["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["kind"] == "memory")
        .unwrap()["ref"]
        .clone();
    let source = native.read(
        "source",
        "sources",
        json!({"refs":[reference],"budget_bytes":8192}),
    );
    assert_eq!(source["result"]["results"][0]["events"][0]["id"], e.id);
    // 新查询在编码等待期间撤权，最终展示必须重新核验，不能返回旧 Memory 正文。
    if let Ok(after_revoke) = handle.search("synthetic second unseen wording", "memories") {
        assert!(
            after_revoke["results"].as_array().unwrap().is_empty(),
            "不能显示已撤权的旧内容"
        );
    }
    let calls = server.join().unwrap();
    assert_eq!(calls.len(), 3);
    assert_eq!(
        calls[1]["input"],
        json!(["synthetic new equipment wording"])
    );
    assert_eq!(
        calls[2]["input"],
        json!(["synthetic second unseen wording"])
    );
    assert_eq!(
        native.read(
            "source",
            "sources",
            json!({"refs":[reference],"budget_bytes":8192})
        )["ok"],
        false
    );
    handle.disable_semantic().unwrap();
    assert!(MemoryRuntime::new(&v)
        .configuration()
        .unwrap()
        .candidate_semantic_config
        .is_none());
}
#[test]
fn semantic_setup_rejects_paths_external_destinations_missing_approval_and_foreign_active_scope() {
    let dir = tempfile::tempdir().unwrap();
    let v = Vault::init(&dir.path().join("vault")).unwrap();
    let mut desktop = DesktopSession::default();
    let selected = desktop.select_vault(v.root(), false).unwrap();
    let handle = desktop
        .model_operation(&selected.session_id, "personal")
        .unwrap();
    let good = settings("http://127.0.0.1:123/v1/embeddings");
    let mut injected = serde_json::to_value(&good).unwrap();
    injected["python"] = json!("untrusted");
    assert!(serde_json::from_value::<LocalSemanticSetup>(injected).is_err());
    for mode in ["external", "scope_credentials", "approval", "dimensions"] {
        let mut bad = good.clone();
        match mode {
            "external" => bad.endpoint = "https://external.invalid/v1/embeddings".into(),
            "scope_credentials" => bad.endpoint = "http://u:p@127.0.0.1:123/v1/embeddings".into(),
            "approval" => {
                bad.query_approved = false;
                bad.memory_approved = false
            }
            "dimensions" => bad.dimensions = 0,
            _ => unreachable!(),
        };
        assert!(handle
            .configure_semantic_with_binary(bad, &dir.path().join("missing-binary"))
            .is_err());
    }
    assert!(MemoryRuntime::new(&v)
        .configuration()
        .unwrap()
        .candidate_semantic_config
        .is_none());
    let first = handle
        .configure_semantic_with_binary(good.clone(), &dir.path().join("missing-binary"))
        .unwrap();
    assert_eq!(first.state, "configured");
    assert!(first.service_error.is_some());
    let cfg = MemoryRuntime::new(&v).configuration().unwrap();
    let first_path = cfg.candidate_semantic_config.unwrap();
    let cached: Value = serde_json::from_slice(&fs::read(&first_path).unwrap()).unwrap();
    let before = fs::read(cached["index_path"].as_str().unwrap()).unwrap();
    let other = desktop
        .model_operation(&selected.session_id, "project:other")
        .unwrap();
    assert!(other
        .configure_semantic_with_binary(good, &dir.path().join("missing-binary"))
        .is_err());
    assert_eq!(
        fs::read(cached["index_path"].as_str().unwrap()).unwrap(),
        before
    );
    let mut changed = settings("http://127.0.0.1:123/v1/embeddings");
    changed.revision = "v2".into();
    handle
        .configure_semantic_with_binary(changed, &dir.path().join("missing-binary"))
        .unwrap();
    let next = MemoryRuntime::new(&v)
        .configuration()
        .unwrap()
        .candidate_semantic_config
        .unwrap();
    assert_ne!(next, first_path);
    assert!(first_path.exists());
    assert_eq!(
        fs::read(cached["index_path"].as_str().unwrap()).unwrap(),
        before
    );
}
#[test]
fn local_semantic_setup_preserves_pause_and_llm_state_until_explicit_resume() {
    let dir = tempfile::tempdir().unwrap();
    let v = Vault::init(&dir.path().join("vault")).unwrap();
    let runtime = MemoryRuntime::new(&v);
    let mut cfg = runtime.configuration().unwrap();
    cfg.paused = true;
    runtime.configure(cfg).unwrap();
    let mut desktop = DesktopSession::default();
    let selected = desktop.select_vault(v.root(), false).unwrap();
    let handle = desktop
        .model_operation(&selected.session_id, "personal")
        .unwrap();
    let request = settings("http://127.0.0.1:123/v1/embeddings");
    let paused = handle
        .configure_semantic_with_binary(request.clone(), &dir.path().join("missing"))
        .unwrap();
    assert!(paused.paused);
    assert!(paused.service_error.is_none());
    assert!(!runtime.configuration().unwrap().enabled);
    let mut resumed = request;
    resumed.resume_background = true;
    let saved = handle
        .configure_semantic_with_binary(resumed, &dir.path().join("missing"))
        .unwrap();
    assert!(!saved.paused);
    assert!(saved.service_error.is_some());
    assert!(!runtime.configuration().unwrap().enabled);
    assert!(runtime.configuration().unwrap().index_maintenance_enabled);
}
#[test]
fn query_only_configuration_cannot_be_overwritten_or_disabled_from_another_scope() {
    let dir = tempfile::tempdir().unwrap();
    let vault = Vault::init(&dir.path().join("vault")).unwrap();
    let mut desktop = DesktopSession::default();
    let selected = desktop.select_vault(vault.root(), false).unwrap();
    let personal = desktop
        .model_operation(&selected.session_id, "personal")
        .unwrap();
    let mut request = settings("http://127.0.0.1:123/v1/embeddings");
    request.query_approved = true;
    request.memory_approved = false;
    personal
        .configure_semantic_with_binary(request.clone(), &dir.path().join("missing"))
        .unwrap();
    let runtime = MemoryRuntime::new(&vault);
    let before = runtime.configuration().unwrap();
    assert!(!before.enabled);
    assert!(!before.index_maintenance_enabled);
    let work = desktop
        .model_operation(&selected.session_id, "work")
        .unwrap();
    assert_eq!(work.semantic_status().unwrap().state, "scope_unavailable");
    assert!(work
        .configure_semantic_with_binary(request, &dir.path().join("missing"))
        .is_err());
    assert!(work.disable_semantic().is_err());
    assert_eq!(
        runtime.configuration().unwrap().candidate_semantic_config,
        before.candidate_semantic_config
    );
    personal.disable_semantic().unwrap();
    assert!(runtime
        .configuration()
        .unwrap()
        .candidate_semantic_config
        .is_none());
}
