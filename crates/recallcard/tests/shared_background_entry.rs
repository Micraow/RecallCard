//! 只用合成正本，核对桌面、真实 MCP/Native 帧与文件入口的同一背景选择。
use recallcard::{
    application::{
        agent_entry,
        connections::{self, ConnectionGrant},
    },
    context::Context,
    desktop::DesktopSession,
    native,
    policy::Access,
    transport, Memory, MemoryState, Vault,
};
use serde_json::{json, Value};
const EXT: &str = "abcdefghijklmnopabcdefghijklmnop";
const INSTALL: &str = "11111111-1111-4111-8111-111111111111";
fn access() -> Access {
    Access::new(vec!["personal".into()]).unwrap()
}
fn source(v: &Vault) -> String {
    v.capture(serde_json::from_value(json!({"role":"user","origin":"user_input","scope":"personal","content":"合成原话，供核对背景选择","source":{"platform":"synthetic","conversation_id":"shared-background","message_id":"one"}})).unwrap()).unwrap().id
}
fn memory(v: &Vault, event: &str, text: &str, extra: Value) -> Memory {
    let mut input =
        json!({"content":text,"source_refs":[event],"scope":"personal","evidence":"user_explicit"});
    input
        .as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    v.add_memory(serde_json::from_value(input).unwrap())
        .unwrap()
}
fn grant(v: &Vault) -> connections::ConnectionEntry {
    connections::configure(
        v,
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
    .unwrap()
}
fn mcp(v: &Vault) -> Value {
    let input=[json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26"}}),json!({"jsonrpc":"2.0","method":"notifications/initialized"}),json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"bootstrap","arguments":{}}})].into_iter().map(|v|v.to_string()+"\n").collect::<String>();
    let mut out = Vec::new();
    transport::serve_mcp_io(v, access(), input.as_bytes(), &mut out).unwrap();
    let result: Value =
        serde_json::from_str(String::from_utf8(out).unwrap().lines().last().unwrap()).unwrap();
    assert_eq!(result["result"]["isError"], false);
    serde_json::from_str(result["result"]["content"][0]["text"].as_str().unwrap()).unwrap()
}
fn browser(v: &Vault) -> Value {
    let body=serde_json::to_vec(&json!({"protocol":"recallcard.action/1","request_id":"background-check","nonce":"synthetic-background-nonce","session_ref":"chatgpt:shared-background","installation_id":INSTALL,"action":"authorized_read","arguments":{"action":"bootstrap","arguments":{},"permission_revision":1}})).unwrap();
    let mut frame = (body.len() as u32).to_ne_bytes().to_vec();
    frame.extend(body);
    let mut out = Vec::new();
    native::serve_native_capture_io(
        v,
        access(),
        None,
        EXT,
        &native::extension_origin(EXT).unwrap(),
        frame.as_slice(),
        &mut out,
    )
    .unwrap();
    serde_json::from_slice(&out[4..]).unwrap()
}
fn same(v: &Vault, id: &str, expected: &Value) -> agent_entry::EntrySnapshot {
    let file = agent_entry::refresh(v, id, vec!["personal".into()], 4096).unwrap();
    assert_eq!(file.schema, "recallcard.agent-entry/2");
    assert_eq!(file.bootstrap_version, expected["bootstrap_version"]);
    assert_eq!(
        file.profile
            .iter()
            .map(|r| r["ref"].clone())
            .collect::<Vec<_>>(),
        *expected["refs"].as_array().unwrap()
    );
    let actual = mcp(v);
    assert_eq!(actual["refs"], expected["refs"]);
    assert_eq!(actual["stable_text"], expected["stable_text"]);
    let native = browser(v);
    assert_eq!(native["ok"], true, "{native}");
    assert_eq!(native["result"]["refs"], expected["refs"]);
    assert_eq!(
        native["result"]["bootstrap_version"],
        expected["bootstrap_version"]
    );
    file
}
#[test]
fn desktop_selection_is_shared_by_file_mcp_and_native_without_erasing_static_copies() {
    let d = tempfile::tempdir().unwrap();
    let mut ui = DesktopSession::default();
    let info = ui.select_vault(&d.path().join("vault"), true).unwrap();
    let v = Vault::open_existing(std::path::Path::new(&info.root)).unwrap();
    let event = source(&v);
    let chosen = memory(&v, &event, "合成默认背景甲", json!({"protected":false}));
    memory(&v, &event, "仅保护不自动携带乙", json!({"protected":true}));
    memory(
        &v,
        &event,
        "仅标签不自动携带丙",
        json!({"protected":false,"labels":["bootstrap"]}),
    );
    memory(
        &v,
        &event,
        "未来背景不携带",
        json!({"labels":["bootstrap"],"valid_from":"2100-01-01T00:00:00Z"}),
    );
    memory(
        &v,
        &event,
        "过期背景不携带",
        json!({"labels":["bootstrap"],"valid_to":"2020-01-01T00:00:00Z"}),
    );
    let connection = grant(&v);
    let empty = ui
        .read_background(&info.session_id, "personal")
        .unwrap()
        .background;
    assert!(empty["refs"].as_array().unwrap().is_empty());
    let old = same(&v, &connection.id, &empty);
    let review = ui
        .review_background_change(
            &info.session_id,
            "personal",
            &chosen.id,
            chosen.revision,
            true,
        )
        .unwrap();
    let confirmed = ui
        .confirm_background_change(&info.session_id, "personal", &review.preview_id, false)
        .unwrap();
    assert!(!old.file_path.exists());
    let included = same(&v, &connection.id, &confirmed["background"]);
    assert_eq!(included.profile.len(), 1);
    assert_eq!(included.profile[0]["text"], chosen.data.content);
    assert_eq!(included.profile[0]["text_truncated"], false);
    let copy = d.path().join("用户另存并编辑的旧副本.json");
    let saved = std::fs::read(&included.file_path).unwrap();
    std::fs::write(&copy, &saved).unwrap();
    let current = v.memory(&chosen.id).unwrap();
    let review = ui
        .review_background_change(
            &info.session_id,
            "personal",
            &chosen.id,
            current.revision,
            false,
        )
        .unwrap();
    ui.confirm_background_change(&info.session_id, "personal", &review.preview_id, true)
        .unwrap();
    assert!(!included.file_path.exists());
    assert_eq!(std::fs::read(&copy).unwrap(), saved);
    let removed = ui
        .read_background(&info.session_id, "personal")
        .unwrap()
        .background;
    let refreshed = same(&v, &connection.id, &removed);
    assert!(refreshed.profile.is_empty());
    assert_ne!(included.bootstrap_version, refreshed.bootstrap_version);
    let text = serde_json::to_string(&refreshed).unwrap();
    assert!(!text.contains(&chosen.data.content));
    assert!(!refreshed.directory.is_empty());
    let search = transport::invoke(
        &Context::new(&v, access()),
        "search",
        json!({"query":"合成默认背景甲"}),
    )
    .unwrap();
    assert!(search.to_string().contains(&chosen.id));
    connections::revoke(&v, &connection.id, 1).unwrap();
    assert!(!refreshed.file_path.exists());
    assert!(agent_entry::refresh(&v, &connection.id, vec!["personal".into()], 4096).is_err());
    assert_eq!(browser(&v)["ok"], false);
    assert_eq!(std::fs::read(copy).unwrap(), saved);
    assert!(ui.read_background(&info.session_id, "personal").is_ok());
}
#[test]
fn explicit_active_suggestion_keeps_its_evidence_and_large_background_is_not_refilled() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(&d.path().join("vault")).unwrap();
    let event = source(&v);
    let connection = grant(&v);
    let suggestion = memory(
        &v,
        &event,
        "合成建议，不是用户决定",
        json!({"evidence":"assistant_suggestion","labels":["bootstrap"]}),
    );
    let tentative = mcp(&v);
    assert!(tentative["refs"].as_array().unwrap().is_empty());
    v.set_state(&suggestion.id, 1, MemoryState::Active).unwrap();
    let projected = mcp(&v);
    let file = same(&v, &connection.id, &projected);
    assert_eq!(file.profile.len(), 1);
    assert_eq!(file.profile[0]["evidence"], "AssistantSuggestion");
    assert!(projected["stable_text"]
        .as_str()
        .unwrap()
        .contains("AI 建议，非用户事实"));
    v.set_state(&suggestion.id, 2, MemoryState::Superseded)
        .unwrap();
    memory(
        &v,
        &event,
        &"合成长背景".repeat(500),
        json!({"labels":["bootstrap"]}),
    );
    memory(&v, &event, "未选择短句不能补位", json!({}));
    let default = mcp(&v);
    assert!(default["refs"].as_array().unwrap().is_empty());
    assert_eq!(default["truncated"], true);
    for budget in [2048, 4096, 32768] {
        let file =
            agent_entry::refresh(&v, &connection.id, vec!["personal".into()], budget).unwrap();
        assert!(file.profile.is_empty());
        assert!(file.truncated);
        assert!(serde_json::to_vec(&file).unwrap().len() <= budget);
    }
}

#[test]
fn outer_file_budget_removes_complete_profile_rows_without_claiming_delivery() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(&d.path().join("vault")).unwrap();
    let event = source(&v);
    let connection = grant(&v);
    let mut labels = vec!["bootstrap".to_string()];
    labels.extend((0..40).map(|i| format!("合成标签{i}{}", "界".repeat(25))));
    let item = memory(&v, &event, "合成短背景元数据较多", json!({"labels":labels}));
    let projected = mcp(&v);
    assert_eq!(projected["refs"].as_array().unwrap().len(), 1);
    let full = agent_entry::refresh(&v, &connection.id, vec!["personal".into()], 32768).unwrap();
    assert_eq!(full.profile.len(), 1);
    assert_eq!(full.profile[0]["text"], item.data.content);
    let small = agent_entry::refresh(&v, &connection.id, vec!["personal".into()], 2048).unwrap();
    assert!(small.profile.is_empty());
    assert!(small.truncated);
    assert_eq!(small.coverage["default_background_items"], 1);
    assert_eq!(small.bootstrap_version, full.bootstrap_version);
    assert!(serde_json::to_vec(&small).unwrap().len() <= 2048);
}
