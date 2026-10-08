//! ChatGPT 原生 MCP 下游的合成验证：仅运行 RecallCard；不建隧道、不配密钥、不调用模型。
use recallcard::{
    application::connections::{self, ConnectionGrant},
    desktop::DesktopSession,
    policy::Access,
    Vault,
};
use serde_json::{json, Value};
use std::io::Write;
use std::process::{Command, Stdio};
fn grant() -> ConnectionGrant {
    ConnectionGrant {
        client_kind: "chatgpt_mcp".into(),
        host_identity: "openai-chatgpt".into(),
        installation_id: None,
        platform: "chatgpt".into(),
        recall_scopes: vec!["personal".into()],
        capture_scopes: vec![],
        provider_disclosure: true,
        auto_capture: false,
        auto_recall: true,
    }
}
fn event(vault: &Vault, scope: &str, text: &str, message: &str) -> String {
    vault.capture(serde_json::from_value(json!({"scope":scope,"role":"user","origin":"user_input","content":text,"source":{"platform":"deepseek","conversation_id":"synthetic-transfer","message_id":message},"metadata":{"conversation_title":"合成接续任务"}})).unwrap()).unwrap().id
}
fn rpc(id: u64, name: &str, args: Value) -> Value {
    json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":name,"arguments":args}})
}
fn init() -> Vec<Value> {
    vec![
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25"}}),
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
    ]
}
fn run(vault: &Vault, id: &str, messages: Vec<Value>) -> Vec<Value> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_recallcard"))
        .arg("--vault")
        .arg(vault.root())
        .args(["mcp", "--scope", "personal", "--connection-id", id])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    {
        let mut input = child.stdin.take().unwrap();
        for message in messages {
            writeln!(input, "{message}").unwrap();
        }
    }
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}
#[test]
fn native_stdio_reads_existing_deepseek_context_without_web_draft_turns() {
    let root = tempfile::tempdir().unwrap();
    let vault = Vault::init(&root.path().join("vault")).unwrap();
    let allowed = event(
        &vault,
        "personal",
        "合成旧决定：先完成 EPUB 标注，暂不做手写批注。下一步核对来源。",
        "visible",
    );
    event(
        &vault,
        "project:private",
        "禁止泄漏的合成跨范围暗号 9317",
        "hidden",
    );
    let configured = connections::configure(&vault, grant(), None).unwrap();
    let mut input = init();
    input.push(json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}));
    input.push(rpc(3, "bootstrap", json!({})));
    input.push(rpc(
        4,
        "search",
        json!({"query":"合成旧决定","target":"events","budget_tokens":8192}),
    ));
    input.push(rpc(
        5,
        "read",
        json!({"refs":[format!("event:{allowed}")],"budget_tokens":8192}),
    ));
    input.push(rpc(
        6,
        "sources",
        json!({"refs":[format!("event:{allowed}")],"budget_tokens":8192}),
    ));
    let output = run(&vault, &configured.id, input);
    assert_eq!(output[1]["result"]["tools"].as_array().unwrap().len(), 4);
    assert!(output[1]["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .all(|tool| tool["annotations"]["readOnlyHint"] == true));
    for response in output.iter().skip(2) {
        assert_eq!(response["result"]["isError"], false, "{response}");
    }
    let search: Value =
        serde_json::from_str(output[3]["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert!(search["results"]
        .as_array()
        .unwrap()
        .iter()
        .any(|row| row["ref"] == format!("event:{allowed}")));
    let text = serde_json::to_string(&output).unwrap();
    assert!(text.contains("合成旧决定"));
    assert!(text.contains(&allowed));
    assert!(!text.contains("9317"));
    let observed = connections::get(&vault, &configured.id).unwrap().unwrap();
    assert!(observed.last_read_at.is_some());
    assert!(observed.last_bootstrap_at.is_some());
    assert!(observed.last_capture_at.is_none());
}
#[test]
fn chatgpt_grant_is_separate_readonly_and_revocation_has_no_broad_scope_fallback() {
    let root = tempfile::tempdir().unwrap();
    let vault = Vault::init(&root.path().join("vault")).unwrap();
    let mut invalid = grant();
    invalid.capture_scopes = vec!["personal".into()];
    assert!(connections::configure(&vault, invalid, None).is_err());
    let mut invalid = grant();
    invalid.host_identity = "claude-code".into();
    assert!(connections::configure(&vault, invalid, None).is_err());
    let entry = connections::configure(&vault, grant(), None).unwrap();
    assert_ne!(
        entry.id,
        connections::connection_key("claude_code", "claude-code", "claude_code", None)
    );
    let mut input = init();
    input.push(rpc(2, "capture_save", json!({})));
    input.push(rpc(
        3,
        "search",
        json!({"query":"合成","scope":"project:private"}),
    ));
    input.push(rpc(4, "read", json!({"refs":["/private/file"]})));
    for response in run(&vault, &entry.id, input).iter().skip(1) {
        assert_eq!(response["result"]["isError"], true);
    }
    connections::revoke(&vault, &entry.id, 1).unwrap();
    let mut input = init();
    input.push(rpc(2, "bootstrap", json!({})));
    assert_eq!(run(&vault, &entry.id, input)[1]["result"]["isError"], true);
    assert!(vault.events().unwrap().is_empty());
}
#[test]
fn local_plan_never_creates_a_grant_tunnel_credential_or_verified_connection() {
    let root = tempfile::tempdir().unwrap();
    let mut session = DesktopSession::default();
    let info = session
        .select_vault(&root.path().join("vault"), true)
        .unwrap();
    let binary = root.path().join("recallcard");
    let before = session
        .chatgpt_connection_plan(&info.session_id, "personal", &binary)
        .unwrap();
    assert_eq!(before["local_transport"], "stdio");
    assert_eq!(before["local_readiness"], "permission_required");
    assert_eq!(before["public_listener"], false);
    assert_eq!(before["upstream_verification"], "not_checked");
    assert!(before.get("server_url").is_none());
    assert!(session
        .connection_inventory(&info.session_id, "personal")
        .unwrap()["entries"]
        .as_array()
        .unwrap()
        .is_empty());
    let entry = session
        .connection_configure(&info.session_id, "personal", grant(), None)
        .unwrap();
    let vault = Vault::open(&root.path().join("vault")).unwrap();
    connections::invoke_agent(
        &vault,
        &entry.id,
        &Access::new(vec!["personal".into()]).unwrap(),
        "bootstrap",
        json!({}),
    )
    .unwrap();
    let after = session
        .chatgpt_connection_plan(&info.session_id, "personal", &binary)
        .unwrap();
    assert_eq!(after["local_readiness"], "ready");
    assert_eq!(after["upstream_verification"], "not_checked");
    assert_eq!(after["credential_status"], "not_inspected");
    assert!(!after["last_local_bootstrap_at"].is_null());
    assert!(session
        .chatgpt_connection_plan(&info.session_id, "project:other", &binary)
        .is_err());
    session
        .connection_revoke(&info.session_id, "personal", &entry.id, 1)
        .unwrap();
    assert_eq!(
        session
            .chatgpt_connection_plan(&info.session_id, "personal", &binary)
            .unwrap()["local_readiness"],
        "revoked"
    );
}
