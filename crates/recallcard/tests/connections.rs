//! 仅合成本机协议；不安装浏览器、不启动外部 Agent、不调用模型。
use recallcard::{
    application::{
        connections::{self, ConnectionGrant},
        ErrorCode,
    },
    native,
    policy::Access,
    Vault,
};
use serde_json::{json, Value};
const EXT: &str = "abcdefghijklmnopabcdefghijklmnop";
fn setup() -> (tempfile::TempDir, Vault) {
    let root = tempfile::tempdir().unwrap();
    let v = Vault::init(&root.path().join("vault")).unwrap();
    (root, v)
}
fn grant() -> ConnectionGrant {
    ConnectionGrant {
        client_kind: "browser".into(),
        host_identity: EXT.into(),
        installation_id: Some("11111111-1111-4111-8111-111111111111".into()),
        platform: "chatgpt".into(),
        recall_scopes: vec!["personal".into()],
        capture_scopes: vec!["personal".into()],
        provider_disclosure: true,
        auto_capture: true,
        auto_recall: true,
    }
}
fn call(v: &Vault, action: &str, args: Value) -> Value {
    call_installation(
        v,
        action,
        args,
        Some("11111111-1111-4111-8111-111111111111"),
    )
}
fn call_installation(v: &Vault, action: &str, args: Value, installation: Option<&str>) -> Value {
    let body=serde_json::to_vec(&json!({"protocol":"recallcard.action/1","request_id":"synthetic-request","nonce":"synthetic-nonce-123456","session_ref":"chatgpt:synthetic","installation_id":installation,"action":action,"arguments":args})).unwrap();
    let mut framed = (body.len() as u32).to_ne_bytes().to_vec();
    framed.extend(body);
    let mut output = Vec::new();
    let served = native::serve_native_capture_io(
        v,
        Access::new(vec!["personal".into()]).unwrap(),
        Some("personal"),
        EXT,
        &native::extension_origin(EXT).unwrap(),
        framed.as_slice(),
        &mut output,
    );
    if let Err(error) = served {
        return json!({"ok":false,"error":error});
    }
    serde_json::from_slice(&output[4..]).unwrap()
}
fn conversation() -> Value {
    json!({"schema":"recallcard.conversation/1","capture_id":"synthetic-capture","captured_at":"2026-01-01T00:00:00Z","title":"合成会话","source":{"platform":"chatgpt","conversation_id":"synthetic","url":"https://chatgpt.com/c/synthetic"},"coverage":{"extent":"visible_only","complete":false,"reason":"合成稳定页面","warnings":[]},"messages":[{"id":"user-1","role":"user","text":"合成项目选择 Rust","occurred_at":null},{"id":"assistant-1","role":"assistant","text":"合成建议","occurred_at":null}],"metadata":{}})
}
#[test]
fn config_is_unverified_until_an_observed_host_operation() {
    let (_d, v) = setup();
    let e = connections::configure(&v, grant(), None).unwrap();
    assert_eq!(e.state, "configured_unverified");
    assert!(e.last_handshake_at.is_none());
    let result = call(&v, "connection", json!({}));
    assert_eq!(result["ok"], true);
    assert_eq!(result["result"]["automation"]["permission_revision"], 1);
    assert_eq!(result["result"]["account_identity"], "unverified");
    let e = connections::get(&v, &e.id).unwrap().unwrap();
    assert!(e.last_handshake_at.is_some());
    assert!(e.last_read_at.is_none());
}
#[test]
fn only_user_config_can_grant_automatic_capture_and_scopes_are_fixed() {
    let (_d, v) = setup();
    assert_eq!(
        call(
            &v,
            "automatic_capture",
            json!({"conversation":conversation(),"permission_revision":1})
        )["ok"],
        false
    );
    connections::configure(&v, grant(), None).unwrap();
    let saved = call(
        &v,
        "automatic_capture",
        json!({"conversation":conversation(),"permission_revision":1}),
    );
    assert_eq!(saved["ok"], true, "{saved}");
    assert_eq!(saved["result"]["events_added"], 2);
    assert_eq!(
        call(
            &v,
            "automatic_capture",
            json!({"conversation":conversation(),"permission_revision":1})
        )["result"]["events_added"],
        0
    );
    assert_eq!(
        call(
            &v,
            "automatic_capture",
            json!({"conversation":conversation(),"permission_revision":1,"scope":"secret"})
        )["ok"],
        false
    );
    assert!(v
        .events()
        .unwrap()
        .iter()
        .all(|e| e.data.scope == "personal"));
}
#[test]
fn revoke_blocks_capture_and_legacy_read_bypass_and_stale_grants() {
    let (_d, v) = setup();
    let e = connections::configure(&v, grant(), None).unwrap();
    connections::revoke(&v, &e.id, 1).unwrap();
    for (action, args) in [
        (
            "automatic_capture",
            json!({"conversation":conversation(),"permission_revision":1}),
        ),
        (
            "authorized_read",
            json!({"action":"bootstrap","arguments":{},"permission_revision":1}),
        ),
        ("bootstrap", json!({})),
    ] {
        assert_eq!(call(&v, action, args)["ok"], false, "{action}");
    }
    assert_eq!(
        connections::configure(&v, grant(), Some(1))
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    assert_eq!(
        connections::get(&v, &e.id).unwrap().unwrap().state,
        "revoked"
    );
}
#[test]
fn capture_does_not_grant_disclosure_or_model_write_tools() {
    let (_d, v) = setup();
    let mut g = grant();
    g.auto_recall = false;
    g.provider_disclosure = false;
    connections::configure(&v, g, None).unwrap();
    assert_eq!(
        call(
            &v,
            "automatic_capture",
            json!({"conversation":conversation(),"permission_revision":1})
        )["ok"],
        true
    );
    assert_eq!(
        call(
            &v,
            "authorized_read",
            json!({"action":"bootstrap","arguments":{},"permission_revision":1})
        )["ok"],
        false
    );
    assert_eq!(
        call(
            &v,
            "authorized_read",
            json!({"action":"capture_save","arguments":{},"permission_revision":1})
        )["ok"],
        false
    );
}
#[test]
fn weak_identity_and_platform_mismatch_cannot_be_automatically_captured() {
    let (_d, v) = setup();
    connections::configure(&v, grant(), None).unwrap();
    for mode in ["message", "conversation", "platform"] {
        let mut c = conversation();
        match mode {
            "message" => c["messages"][0]["metadata"] = json!({"weaker_identity":true}),
            "conversation" => c["metadata"] = json!({"weaker_conversation_identity":true}),
            _ => {
                c["source"]["platform"] = json!("deepseek");
                c["source"]["url"] = json!("https://chat.deepseek.com/a/chat/s/synthetic");
            }
        }
        assert_eq!(
            call(
                &v,
                "automatic_capture",
                json!({"conversation":c,"permission_revision":1})
            )["ok"],
            false,
            "{mode}"
        );
    }
    assert!(v.events().unwrap().is_empty());
}
#[test]
fn active_authorization_serializes_revocation_and_never_recreates_grants() {
    let (_d, v) = setup();
    let e = connections::configure(&v, grant(), None).unwrap();
    let auth = connections::authorize(&v, &e.id, 1).unwrap();
    assert_eq!(
        connections::revoke(&v, &e.id, 1).unwrap_err().code,
        ErrorCode::Conflict
    );
    drop(auth);
    connections::revoke(&v, &e.id, 1).unwrap();
    assert!(connections::authorize(&v, &e.id, 1).is_err());
    assert!(connections::observe(&v, &e.id, 1, "capture").is_err());
}
#[test]
fn lifecycle_output_observation_is_revocable_and_does_not_trust_hook_identity() {
    let (_d, v) = setup();
    let mut g = grant();
    g.client_kind = "claude_code".into();
    g.host_identity = "claude-code".into();
    g.installation_id = None;
    g.platform = "claude_code".into();
    g.capture_scopes.clear();
    g.auto_capture = false;
    let e = connections::configure(&v, g, None).unwrap();
    for source in ["startup", "resume", "compact", "clear"] {
        let input=json!({"hook_event_name":"SessionStart","source":source,"connection_id":"forged","scope":"secret","command":"not executed"}).to_string();
        let result = recallcard::agent_hook::session_start_connected(
            &v,
            &e.id,
            Access::new(vec!["personal".into()]).unwrap(),
            8192,
            input.as_bytes(),
        )
        .unwrap();
        assert_eq!(
            result["hookSpecificOutput"]["hookEventName"],
            "SessionStart"
        );
    }
    assert!(connections::get(&v, &e.id)
        .unwrap()
        .unwrap()
        .last_bootstrap_at
        .is_some());
    connections::revoke(&v, &e.id, 1).unwrap();
    assert!(recallcard::agent_hook::session_start_connected(
        &v,
        &e.id,
        Access::new(vec!["personal".into()]).unwrap(),
        8192,
        b"{\"hook_event_name\":\"SessionStart\",\"source\":\"startup\"}".as_slice()
    )
    .is_err());
}
#[test]
fn auto_recall_cannot_be_enabled_without_explicit_provider_destination_approval() {
    let (_d, v) = setup();
    let mut g = grant();
    g.provider_disclosure = false;
    assert_eq!(
        connections::configure(&v, g, None).unwrap_err().code,
        ErrorCode::ConsentRequired
    );
    assert!(connections::inventory(&v).unwrap()["entries"]
        .as_array()
        .unwrap()
        .is_empty());
}

#[test]
fn native_cache_does_not_replay_private_results_after_revocation() {
    use std::io::Write;
    struct RevokeAfterFirstReply<'a> {
        vault: &'a Vault,
        id: String,
        data: Vec<u8>,
        revoked: bool,
    }
    impl Write for RevokeAfterFirstReply<'_> {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.data.extend(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            if !self.revoked {
                connections::revoke(self.vault, &self.id, 1).unwrap();
                self.revoked = true;
            }
            Ok(())
        }
    }
    let (_d, v) = setup();
    let e = connections::configure(&v, grant(), None).unwrap();
    let body=serde_json::to_vec(&json!({"protocol":"recallcard.action/1","request_id":"same","nonce":"synthetic-nonce-123456","session_ref":"chatgpt:synthetic","installation_id":"11111111-1111-4111-8111-111111111111","action":"bootstrap","arguments":{}})).unwrap();
    let mut one = (body.len() as u32).to_ne_bytes().to_vec();
    one.extend(body);
    let mut input = one.clone();
    input.extend(one);
    let mut writer = RevokeAfterFirstReply {
        vault: &v,
        id: e.id,
        data: Vec::new(),
        revoked: false,
    };
    native::serve_native_capture_io(
        &v,
        Access::new(vec!["personal".into()]).unwrap(),
        Some("personal"),
        EXT,
        &native::extension_origin(EXT).unwrap(),
        input.as_slice(),
        &mut writer,
    )
    .unwrap();
    let len = u32::from_ne_bytes(writer.data[0..4].try_into().unwrap()) as usize;
    let first: Value = serde_json::from_slice(&writer.data[4..4 + len]).unwrap();
    let second: Value = serde_json::from_slice(&writer.data[8 + len..]).unwrap();
    assert_eq!(first["ok"], true);
    assert_eq!(second["ok"], false);
    assert!(second.get("result").is_none());
}
#[test]
fn vault_replacement_at_same_path_cannot_inherit_connection_grants() {
    let (root, v) = setup();
    let e = connections::configure(&v, grant(), None).unwrap();
    std::fs::rename(v.root(), root.path().join("previous-vault")).unwrap();
    let replaced = Vault::init(v.root()).unwrap();
    assert_eq!(
        connections::get(&replaced, &e.id).unwrap_err().code,
        ErrorCode::ConsentRequired
    );
    assert!(connections::authorize(&replaced, &e.id, 1).is_err());
    assert!(connections::revoke(&replaced, &e.id, 1).is_err());
    assert_eq!(
        call(
            &replaced,
            "automatic_capture",
            json!({"conversation":conversation(),"permission_revision":1})
        )["ok"],
        false
    );
    assert!(replaced.events().unwrap().is_empty());
}

#[test]
fn different_installations_missing_identity_and_reinstall_do_not_inherit_grants() {
    let (_d, v) = setup();
    let first = connections::configure(&v, grant(), None).unwrap();
    let second = "22222222-2222-4222-8222-222222222222";
    for identity in [
        None,
        Some(second),
        Some("33333333-3333-4333-8333-333333333333"),
    ] {
        assert_eq!(
            call_installation(&v, "bootstrap", json!({}), identity)["ok"],
            false
        );
        assert_eq!(
            call_installation(
                &v,
                "automatic_capture",
                json!({"conversation":conversation(),"permission_revision":1}),
                identity
            )["ok"],
            false
        );
    }
    let mut another = grant();
    another.installation_id = Some(second.into());
    let next = connections::configure(&v, another, None).unwrap();
    assert_ne!(first.id, next.id);
    assert_eq!(
        call_installation(&v, "bootstrap", json!({}), Some(second))["ok"],
        true
    );
    connections::revoke(&v, &first.id, 1).unwrap();
    assert_eq!(call(&v, "bootstrap", json!({}))["ok"], false);
    assert_eq!(
        call_installation(&v, "bootstrap", json!({}), Some(second))["ok"],
        true
    );
    let mut missing = grant();
    missing.installation_id = None;
    assert_eq!(
        connections::configure(&v, missing, None).unwrap_err().code,
        ErrorCode::ConsentRequired
    );
}
#[test]
fn chunk_boundaries_preserve_predecessors_without_extra_revisions() {
    let (_d, v) = setup();
    connections::configure(&v, grant(), None).unwrap();
    let full = conversation();
    let mut first = full.clone();
    first["messages"] = json!([full["messages"][0]]);
    assert_eq!(
        call(
            &v,
            "automatic_capture",
            json!({"conversation":first,"permission_revision":1})
        )["result"]["events_added"],
        1
    );
    let mut second = full.clone();
    second["messages"] = json!([full["messages"][1]]);
    second["metadata"] = json!({"chunk_previous_message":{"platform":"chatgpt","conversation_id":"synthetic","message_id":"user-1"}});
    assert_eq!(
        call(
            &v,
            "automatic_capture",
            json!({"conversation":second,"permission_revision":1})
        )["result"]["events_added"],
        1
    );
    assert_eq!(
        call(
            &v,
            "automatic_capture",
            json!({"conversation":full,"permission_revision":1})
        )["result"]["events_added"],
        0
    );
    assert_eq!(v.events().unwrap().len(), 2);
    for event in v.events().unwrap() {
        if event.data.source.message_id == "assistant-1" {
            assert_eq!(event.data.metadata["previous_message_id"], "user-1");
        }
    }
    for value in [
        json!({"platform":"chatgpt","conversation_id":"other","message_id":"user-1"}),
        json!({"platform":"chatgpt","conversation_id":"synthetic","message_id":"../private"}),
        json!({"platform":"chatgpt","conversation_id":"synthetic","message_id":"unknown"}),
    ] {
        let mut bad = conversation();
        bad["messages"] = json!([bad["messages"][1]]);
        bad["metadata"] = json!({"chunk_previous_message":value});
        assert_eq!(
            call(
                &v,
                "automatic_capture",
                json!({"conversation":bad,"permission_revision":1})
            )["ok"],
            false
        );
    }
}
#[test]
fn mcp_each_call_observes_current_grant_and_cannot_escalate_scope_or_write() {
    let (_d, v) = setup();
    let mut g = grant();
    g.client_kind = "claude_code".into();
    g.platform = "claude_code".into();
    g.host_identity = "claude-code".into();
    g.installation_id = None;
    g.capture_scopes.clear();
    g.auto_capture = false;
    let e = connections::configure(&v, g, None).unwrap();
    let cap = Access::new(vec!["personal".into()]).unwrap();
    assert!(connections::invoke_agent(&v, &e.id, &cap, "bootstrap", json!({})).is_ok());
    assert!(connections::invoke_agent(&v, &e.id, &cap, "capture_save", json!({})).is_err());
    assert!(connections::invoke_agent(
        &v,
        &e.id,
        &cap,
        "search",
        json!({"query":"synthetic","scope":"secret"})
    )
    .is_err());
    connections::revoke(&v, &e.id, 1).unwrap();
    assert!(connections::invoke_agent(&v, &e.id, &cap, "bootstrap", json!({})).is_err());
}

#[test]
fn pairing_has_zero_authority_until_local_user_approval_and_retries_are_idempotent() {
    let (_d, v) = setup();
    let response = call(&v, "request_pairing", json!({}));
    assert_eq!(response["ok"], true);
    assert_eq!(response["result"]["data_access"], false);
    let unapproved = call(&v, "connection", json!({}));
    assert_eq!(unapproved["result"]["capture_enabled"], false);
    assert_eq!(unapproved["result"]["read_scopes"], json!([]));
    assert!(unapproved["result"]["capture_scope"].is_null());
    let id = response["result"]["request_id"].as_str().unwrap();
    assert_eq!(
        call(&v, "request_pairing", json!({}))["result"]["request_id"],
        id
    );
    assert_eq!(call(&v, "bootstrap", json!({}))["ok"], false);
    assert_eq!(
        call(
            &v,
            "automatic_capture",
            json!({"conversation":conversation(),"permission_revision":1})
        )["ok"],
        false
    );
    assert!(connections::inventory(&v).unwrap()["entries"]
        .as_array()
        .unwrap()
        .is_empty());
    assert!(v.events().unwrap().is_empty());
    assert_eq!(
        call(
            &v,
            "request_pairing",
            json!({"scope":"secret","auto_recall":true})
        )["ok"],
        false
    );
    let mut wrong = grant();
    wrong.recall_scopes = vec!["secret".into()];
    assert_eq!(
        connections::approve_pairing(&v, id, wrong, None)
            .unwrap_err()
            .code,
        ErrorCode::PermissionDenied
    );
    let mut wrong_install = grant();
    wrong_install.installation_id = Some("22222222-2222-4222-8222-222222222222".into());
    assert!(connections::approve_pairing(&v, id, wrong_install, None).is_err());
    let entry = connections::approve_pairing(&v, id, grant(), None).unwrap();
    assert_eq!(entry.state, "configured_unverified");
    assert_eq!(call(&v, "bootstrap", json!({}))["ok"], true);
    assert!(connections::approve_pairing(&v, id, grant(), None).is_err());
    assert!(connections::inventory(&v).unwrap()["pending_pairings"]
        .as_array()
        .unwrap()
        .is_empty());
}
#[test]
fn pairing_queue_is_bounded_expiring_and_cannot_be_used_from_model_tools() {
    let (_d, v) = setup();
    let mut first = None;
    for n in 0..16 {
        let install = format!("{n:08x}-1111-4111-8111-111111111111");
        let request = connections::request_pairing(
            &v,
            EXT,
            &install,
            "chatgpt",
            vec!["personal".into()],
            vec!["personal".into()],
        )
        .unwrap();
        if n == 0 {
            first = Some(request);
        }
    }
    assert_eq!(
        connections::request_pairing(
            &v,
            EXT,
            "ffffffff-1111-4111-8111-111111111111",
            "chatgpt",
            vec!["personal".into()],
            vec![]
        )
        .unwrap_err()
        .code,
        ErrorCode::ResourceLimit
    );
    let first = first.unwrap();
    let path = v.state_dir().unwrap().join("connections-v1.json");
    let mut stored: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    stored["pending_pairings"][0]["expires_at"] = json!("2000-01-01T00:00:00Z");
    std::fs::write(&path, serde_json::to_vec(&stored).unwrap()).unwrap();
    let mut expired = grant();
    expired.installation_id = Some(first.installation_id.clone());
    assert_eq!(
        connections::approve_pairing(&v, &first.request_id, expired, None)
            .unwrap_err()
            .code,
        ErrorCode::ConsentRequired
    );
    assert_eq!(
        connections::inventory(&v).unwrap()["pending_pairings"]
            .as_array()
            .unwrap()
            .len(),
        15
    );
    assert!(recallcard::transport::invoke(
        &recallcard::context::Context::new(&v, Access::new(vec!["personal".into()]).unwrap()),
        "request_pairing",
        json!({})
    )
    .is_err());
}

#[test]
fn cli_ipc_native_path_cannot_fall_back_around_revoked_installation_grants() {
    let (_d, v) = setup();
    let e = connections::configure(&v, grant(), None).unwrap();
    connections::revoke(&v, &e.id, 1).unwrap();
    let body=serde_json::to_vec(&json!({"protocol":"recallcard.action/1","request_id":"ipc-read","nonce":"synthetic-nonce-123456","session_ref":"chatgpt:synthetic","installation_id":"11111111-1111-4111-8111-111111111111","action":"bootstrap","arguments":{}})).unwrap();
    let mut frame = (body.len() as u32).to_ne_bytes().to_vec();
    frame.extend(body);
    let mut output = Vec::new();
    native::serve_native_read_service_io(
        &v,
        Access::new(vec!["personal".into()]).unwrap(),
        EXT,
        &native::extension_origin(EXT).unwrap(),
        |_, _| panic!("不得绕过撤权调用旧IPC读取"),
        frame.as_slice(),
        &mut output,
    )
    .unwrap();
    let result: Value = serde_json::from_slice(&output[4..]).unwrap();
    assert_eq!(result["ok"], false);
    assert!(result.get("result").is_none());
}

#[test]
fn real_cli_connected_hook_and_mcp_use_the_fixed_grant_without_external_agents() {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let (_d, v) = setup();
    let mut g = grant();
    g.client_kind = "claude_code".into();
    g.platform = "claude_code".into();
    g.host_identity = "claude-code".into();
    g.installation_id = None;
    g.capture_scopes.clear();
    g.auto_capture = false;
    let e = connections::configure(&v, g, None).unwrap();
    let run = |subcommand: &str, input: &str| {
        let mut child = Command::new(env!("CARGO_BIN_EXE_recallcard"))
            .arg("--vault")
            .arg(v.root())
            .args([subcommand, "--scope", "personal", "--connection-id", &e.id])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    };
    let hook = run(
        "agent-hook",
        "{\"hook_event_name\":\"SessionStart\",\"source\":\"compact\"}",
    );
    assert!(
        hook.status.success(),
        "{}",
        String::from_utf8_lossy(&hook.stderr)
    );
    let parsed: Value = serde_json::from_slice(&hook.stdout).unwrap();
    assert_eq!(
        parsed["hookSpecificOutput"]["hookEventName"],
        "SessionStart"
    );
    let rpc=concat!("{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2025-11-25\"}}\n","{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n","{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"bootstrap\",\"arguments\":{}}}\n");
    let mcp = run("mcp", rpc);
    assert!(
        mcp.status.success(),
        "{}",
        String::from_utf8_lossy(&mcp.stderr)
    );
    let lines: String = String::from_utf8(mcp.stdout).unwrap();
    let response: Value = serde_json::from_str(lines.lines().last().unwrap()).unwrap();
    assert_eq!(response["result"]["isError"], false);
    connections::revoke(&v, &e.id, 1).unwrap();
    let denied = run(
        "agent-hook",
        "{\"hook_event_name\":\"SessionStart\",\"source\":\"resume\"}",
    );
    assert!(!denied.status.success());
    assert!(denied.stdout.is_empty());
    let denied = run("mcp", rpc);
    let lines = String::from_utf8(denied.stdout).unwrap();
    let response: Value = serde_json::from_str(lines.lines().last().unwrap()).unwrap();
    assert_eq!(response["result"]["isError"], true);
}
