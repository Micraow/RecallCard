//! 合成原件的实际词法检索/分页/授权合同；不代表真实模型或用户验收。
use recallcard::{context::Context, policy::Access, transport, Event, Vault};
use serde_json::{json, Value};
fn call(v: &Vault, scope: &str, name: &str, args: Value) -> Result<Value, String> {
    transport::invoke(
        &Context::new(v, Access::new(vec![scope.into()]).unwrap()),
        name,
        args,
    )
}
fn fixture() -> (tempfile::TempDir, Vault) {
    let dir = tempfile::tempdir().unwrap();
    let v = Vault::init(&dir.path().join("vault")).unwrap();
    (dir, v)
}
fn attachment(v: &Vault, scope: &str, text: &[u8], message: &str, mime: &str) -> Event {
    let hash = recallcard::hash(text);
    std::fs::write(v.root().join("objects").join(&hash), text).unwrap();
    v.capture(serde_json::from_value(json!({"scope":scope,"role":"user","origin":"user_input","content":"消息原文保持不变","source":{"platform":"synthetic","conversation_id":"attachment-contract","message_id":message},"metadata":{"source_assets":{"files":[{"source_file_id":"file_SYNTHETIC","name":"synthetic-note.txt","payload_status":"retained","detected_media_type":"application/octet-stream","declared_media_type":mime,"object_sha256":hash,"byte_count":text.len()}]}}})).unwrap()).unwrap()
}
fn reference(event: &Event) -> String {
    format!(
        "attachment:{}/{}",
        event.id,
        event.data.metadata["source_assets"]["files"][0]["object_sha256"]
            .as_str()
            .unwrap()
    )
}
fn search(v: &Vault, q: &str) -> Value {
    call(v, "personal", "recall_search", json!({"query":q})).unwrap()
}
#[test]
fn body_only_keyword_reaches_normal_search_shared_search_fetch_and_sources() {
    let (_dir, v) = fixture();
    let content = "\u{feff}附件说明\r\nATTACHMENT_ONLY_421 中文🙂\n";
    let e = attachment(&v, "personal", content.as_bytes(), "one", "text/plain");
    for target in ["all", "events", "attachments"] {
        let page = call(
            &v,
            "personal",
            "search",
            json!({"query":"ATTACHMENT_ONLY_421","target":target,"budget_bytes":8192}),
        )
        .unwrap();
        assert_eq!(page["results"][0]["ref"], reference(&e), "{page}");
        assert_eq!(page["results"][0]["kind"], "attachment");
        assert_eq!(
            page["results"][0]["text_range"]["projection"],
            "attachment.utf8/1"
        );
        assert_eq!(page["coverage"]["attachments"]["semantic"], false);
    }
    let found = search(&v, "ATTACHMENT_ONLY_421");
    let row = &found["results"][0];
    assert_eq!(row["ref"], reference(&e), "{found}");
    assert_eq!(row["relevance"], "unassessed");
    let got = call(&v, "personal", "recall_fetch", json!({"id":row["id"]})).unwrap();
    assert_eq!(got["results"][0]["text"], content);
    assert_eq!(got["results"][0]["snapshot"], row["snapshot"]);
    assert_eq!(
        got["results"][0]["text_hash"],
        recallcard::hash(content.as_bytes())
    );
    let sources = call(
        &v,
        "personal",
        "sources",
        json!({"refs":[reference(&e)],"budget_bytes":2048}),
    )
    .unwrap();
    assert_eq!(
        sources["results"][0]["source_refs"],
        json!([format!("event:{}", e.id)])
    );
    let original = call(
        &v,
        "personal",
        "read",
        json!({"refs":[format!("event:{}",e.id)],"detail":"compact","budget_bytes":4096}),
    )
    .unwrap();
    assert_eq!(original["results"][0]["text"], "消息原文保持不变");
    assert!(serde_json::to_vec(&found).unwrap().len() <= 8192);
    assert!(serde_json::to_vec(&got).unwrap().len() <= 8192);
}
#[test]
fn utf8_pagination_reassembles_exact_original_and_scopes_bind_locators() {
    let (_dir, v) = fixture();
    let text = format!("ATTACHMENT_LONG_22\n{}", "中文🙂 \\\" \r\n".repeat(2000));
    let e = attachment(&v, "personal", text.as_bytes(), "long", "text/plain");
    let found = search(&v, "ATTACHMENT_LONG_22");
    let mut next = found["results"][0]["id"].clone();
    assert!(call(&v, "project:other", "recall_fetch", json!({"id":next})).is_err());
    let mut rebuilt = String::new();
    while !next.is_null() {
        let got = call(&v, "personal", "recall_fetch", json!({"id":next})).unwrap();
        assert!(serde_json::to_vec(&got).unwrap().len() <= 8192);
        assert_eq!(got["results"][0]["text_range"]["start_byte"], rebuilt.len());
        rebuilt.push_str(got["results"][0]["text"].as_str().unwrap());
        next = got["next_id"].clone();
    }
    assert_eq!(rebuilt, text);
    assert!(call(
        &v,
        "personal",
        "read",
        json!({"refs":[reference(&e)],"offset_bytes":21,"detail":"compact","budget_bytes":2048})
    )
    .is_err());
}
#[test]
fn suppression_revision_and_foreign_scope_remove_attachment_candidates_and_old_fetches() {
    for revise in [false, true] {
        let (_dir, v) = fixture();
        let e = attachment(&v, "personal", b"STALE_ATTACHMENT_87", "same", "text/plain");
        let foreign = attachment(
            &v,
            "project:other",
            b"FOREIGN_ATTACHMENT_99",
            "other",
            "text/plain",
        );
        let found = search(&v, "STALE_ATTACHMENT_87");
        let id = found["results"][0]["id"].clone();
        assert!(search(&v, "FOREIGN_ATTACHMENT_99")["results"]
            .as_array()
            .unwrap()
            .is_empty());
        assert!(call(
            &v,
            "personal",
            "read",
            json!({"refs":[reference(&foreign)],"budget_bytes":4096})
        )
        .is_err());
        if revise {
            let mut new = e.data.clone();
            new.content = "消息已修改，附件已移除".into();
            new.metadata = json!({});
            new.revision_of = Some(e.id.clone());
            v.capture(new).unwrap();
        } else {
            v.suppress(&e.id, "合成撤回".into()).unwrap();
        }
        assert!(search(&v, "STALE_ATTACHMENT_87")["results"]
            .as_array()
            .unwrap()
            .is_empty());
        assert!(call(&v, "personal", "recall_fetch", json!({"id":id})).is_err());
        assert!(call(
            &v,
            "personal",
            "read",
            json!({"refs":[reference(&e)],"budget_bytes":4096})
        )
        .is_err());
    }
}
#[test]
fn corrupt_missing_binary_oversized_or_unknown_payload_never_returns_body() {
    for mode in [
        "corrupt",
        "missing",
        "nul",
        "invalid_utf8",
        "pdf",
        "large",
        "unknown",
    ] {
        let (_dir, v) = fixture();
        let bytes = match mode {
            "nul" => b"BLOCKED_ATTACHMENT_56\0".to_vec(),
            "invalid_utf8" => b"BLOCKED_ATTACHMENT_56\xff".to_vec(),
            "pdf" => b"%PDF-1.7 BLOCKED_ATTACHMENT_56".to_vec(),
            "large" => vec![b'x'; 1024 * 1024 + 1],
            _ => b"BLOCKED_ATTACHMENT_56".to_vec(),
        };
        let e = attachment(
            &v,
            "personal",
            &bytes,
            "invalid",
            if mode == "unknown" {
                "application/octet-stream"
            } else {
                "text/plain"
            },
        );
        let object = e.data.metadata["source_assets"]["files"][0]["object_sha256"]
            .as_str()
            .unwrap();
        let path = v.root().join("objects").join(object);
        if mode == "corrupt" {
            std::fs::write(&path, b"TAMPERED_ATTACHMENT_5").unwrap();
        }
        if mode == "missing" {
            std::fs::remove_file(&path).unwrap();
        }
        let page = search(&v, "BLOCKED_ATTACHMENT_56");
        assert!(
            page["results"].as_array().unwrap().is_empty(),
            "{mode}: {page}"
        );
        assert_eq!(page["coverage"]["attachments"]["status"], "partial");
        assert!(call(
            &v,
            "personal",
            "read",
            json!({"refs":[reference(&e)],"budget_bytes":4096})
        )
        .is_err());
    }
}
#[cfg(unix)]
#[test]
fn symlink_object_and_parent_are_rejected_without_following_them() {
    for parent in [false, true] {
        let (dir, v) = fixture();
        let e = attachment(&v, "personal", b"SYMLINK_ONLY_72", "symlink", "text/plain");
        let object = e.data.metadata["source_assets"]["files"][0]["object_sha256"]
            .as_str()
            .unwrap();
        let objects = v.root().join("objects");
        if parent {
            std::fs::rename(&objects, dir.path().join("moved")).unwrap();
            std::os::unix::fs::symlink(dir.path().join("moved"), objects).unwrap();
        } else {
            std::fs::remove_file(objects.join(object)).unwrap();
            std::fs::write(dir.path().join("outside"), b"SYMLINK_ONLY_72").unwrap();
            std::os::unix::fs::symlink(dir.path().join("outside"), objects.join(object)).unwrap();
        }
        assert!(call(
            &v,
            "personal",
            "read",
            json!({"refs":[reference(&e)],"budget_bytes":4096})
        )
        .is_err());
    }
}
#[test]
fn revoked_connection_rejects_already_issued_attachment_locator() {
    use recallcard::application::connections::{self, AgentReadService, ConnectionGrant};
    let (_dir, v) = fixture();
    attachment(
        &v,
        "personal",
        b"GRANT_ATTACHMENT_12",
        "grant",
        "text/plain",
    );
    let entry = connections::configure(
        &v,
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
        },
        None,
    )
    .unwrap();
    let access = Access::new(vec!["personal".into()]).unwrap();
    let service = AgentReadService::new(&v, &entry.id, &access, None);
    let found = service
        .invoke("recall_search", json!({"query":"GRANT_ATTACHMENT_12"}))
        .unwrap();
    let id = found["results"][0]["id"].clone();
    assert!(id.is_string(), "{found}");
    service.invoke("recall_fetch", json!({"id":id})).unwrap();
    connections::revoke(&v, &entry.id, entry.permission_revision).unwrap();
    assert!(service.invoke("recall_fetch", json!({"id":id})).is_err());
    assert!(service
        .invoke("recall_search", json!({"query":"GRANT_ATTACHMENT_12"}))
        .is_err());
}

#[test]
fn shared_object_uses_first_verified_supported_association_in_search_and_fetch() {
    let (_dir, v) = fixture();
    let old = attachment(
        &v,
        "personal",
        b"SHARED_OBJECT_64",
        "shared-object",
        "text/plain",
    );
    let valid = old.data.metadata["source_assets"]["files"][0].clone();
    let mut unsupported = valid.clone();
    unsupported["declared_media_type"] = json!("application/pdf");
    let mut wrong_size = valid.clone();
    wrong_size["byte_count"] = json!(1);
    let mut input = old.data.clone();
    input.metadata["source_assets"]["files"] = json!([unsupported, wrong_size, valid]);
    input.revision_of = Some(old.id);
    let current = v.capture(input).unwrap();
    let found = search(&v, "SHARED_OBJECT_64");
    assert_eq!(found["results"].as_array().unwrap().len(), 1, "{found}");
    assert_eq!(found["results"][0]["ref"], reference(&current));
    let read = call(
        &v,
        "personal",
        "recall_fetch",
        json!({"id":found["results"][0]["id"]}),
    )
    .unwrap();
    assert_eq!(read["results"][0]["text"], "SHARED_OBJECT_64");
}
