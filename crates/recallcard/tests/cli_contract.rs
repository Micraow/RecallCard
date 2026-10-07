//! CLI 与只读工具引用、过滤和有界输入的合成合同。
use recallcard::{EventInput, MemoryInput, Vault};
use serde_json::{json, Value};
use std::io::Write;
use std::process::{Command, Output, Stdio};

fn run(vault: &Vault, args: &[&str], input: Option<&str>) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_recallcard"))
        .arg("--vault")
        .arg(vault.root())
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    if let Some(input) = input {
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
    }
    child.wait_with_output().unwrap()
}
fn ok(output: Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}
fn setup() -> (tempfile::TempDir, Vault) {
    let dir = tempfile::tempdir().unwrap();
    let vault = Vault::init(&dir.path().join("vault")).unwrap();
    (dir, vault)
}
fn event(vault: &Vault, id: &str, conversation: &str) -> recallcard::Event {
    let input: EventInput = serde_json::from_value(json!({
        "role":"user", "origin":"native", "scope":"personal",
        "occurred_at":"2026-10-06T10:00:00Z", "content":"合成命令行合同 Rust",
        "source":{"platform":"manual-web","conversation_id":conversation,"message_id":id}
    }))
    .unwrap();
    vault.capture(input).unwrap()
}
#[test]
fn cli_accepts_tool_refs_checks_revision_and_respects_suppression() {
    let (_dir, vault) = setup();
    let event = event(&vault, "a", "first");
    let input: MemoryInput = serde_json::from_value(json!({
        "content":"合成记忆", "scope":"personal", "source_refs":[event.id],
        "evidence":"user_explicit"
    }))
    .unwrap();
    let memory = vault.add_memory(input).unwrap();
    let reference = format!("memory:{}@1", memory.id);
    assert_eq!(
        ok(run(&vault, &["read", &format!("event:{}", event.id)], None))["id"],
        event.id
    );
    assert_eq!(
        ok(run(&vault, &["read", &reference], None))["id"],
        memory.id
    );
    assert_eq!(
        ok(run(&vault, &["sources", &reference], None))[0]["id"],
        event.id
    );
    assert_eq!(
        ok(run(
            &vault,
            &["sources", &format!("event:{}", event.id)],
            None
        ))[0]["id"],
        event.id
    );
    assert!(!run(
        &vault,
        &["read", &reference, "--scope", "project:other"],
        None
    )
    .status
    .success());
    vault
        .update_memory(&memory.id, 1, memory.data.clone())
        .unwrap();
    assert!(!run(&vault, &["read", &reference], None).status.success());
    assert!(!run(&vault, &["sources", &reference], None).status.success());
    vault.suppress(&event.id, "合成撤权".into()).unwrap();
    assert!(
        !run(&vault, &["read", &format!("memory:{}@2", memory.id)], None)
            .status
            .success()
    );
    assert!(
        !run(&vault, &["sources", &format!("event:{}", event.id)], None)
            .status
            .success()
    );
}
#[test]
fn cli_search_exposes_session_time_detail_and_pagination() {
    let (_dir, vault) = setup();
    let first = event(&vault, "a", "first");
    event(&vault, "b", "second");
    let base = [
        "search",
        "Rust",
        "--scope",
        "personal",
        "--budget-tokens",
        "12000",
    ];
    let result = ok(run(
        &vault,
        &[
            &base[..],
            &[
                "--session-ref",
                &first.data.session_key(),
                "--detail",
                "brief",
            ],
        ]
        .concat(),
        None,
    ));
    assert_eq!(result["results"].as_array().unwrap().len(), 1);
    assert_eq!(result["results"][0]["ref"], format!("event:{}", first.id));
    let earlier = ok(run(
        &vault,
        &[&base[..], &["--as-of", "2020-01-01T00:00:00Z"]].concat(),
        None,
    ));
    assert!(earlier["results"].as_array().unwrap().is_empty());
    let page = ok(run(&vault, &[&base[..], &["--limit", "1"]].concat(), None));
    let cursor = page["next_cursor"].as_str().unwrap();
    let next = ok(run(
        &vault,
        &[&base[..], &["--limit", "1", "--cursor", cursor]].concat(),
        None,
    ));
    assert_ne!(page["results"][0]["ref"], next["results"][0]["ref"]);
}
#[test]
fn cli_import_accepts_stdin_and_rejects_oversized_files_before_writing() {
    let (dir, vault) = setup();
    let input = json!({"role":"user","origin":"native","content":"合成主动粘贴", "scope":"personal", "source":{"platform":"manual-web","conversation_id":"stdin","message_id":"one"}}).to_string()+"\n";
    let result = run(
        &vault,
        &[
            "import",
            "--format",
            "manual-jsonl",
            "--file",
            "-",
            "--scope",
            "personal",
        ],
        Some(&input),
    );
    ok(result);
    assert_eq!(vault.events().unwrap().len(), 1);
    let oversized = dir.path().join("oversized.jsonl");
    std::fs::write(&oversized, vec![b' '; 16 * 1024 * 1024 + 1]).unwrap();
    let result = run(
        &vault,
        &[
            "import",
            "--format",
            "manual-jsonl",
            "--file",
            oversized.to_str().unwrap(),
            "--scope",
            "personal",
        ],
        None,
    );
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("16 MiB"));
    assert_eq!(vault.events().unwrap().len(), 1);
}

#[test]
fn cli_read_and_sources_refuse_an_active_writer_instead_of_mixing_revisions() {
    let (_dir, vault) = setup();
    let event = event(&vault, "a", "first");
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(vault.state_dir().unwrap().join("write.lock"))
        .unwrap();
    file.try_lock().unwrap();
    for command in ["read", "sources"] {
        let output = run(&vault, &[command, &format!("event:{}", event.id)], None);
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("正在更新"));
        assert!(output.stdout.is_empty());
    }
    file.unlock().unwrap();
    assert_eq!(ok(run(&vault, &["read", &event.id], None))["id"], event.id);
}
