//! CLI 是独立入口：不用 GUI、模型凭据或网络完成导入、读取与受控MCP配置。
use serde_json::{json, Value};
use std::{path::Path, process::Command};
fn cli(vault: &Path, args: &[&str]) -> Value {
    let out = Command::new(env!("CARGO_BIN_EXE_recallcard"))
        .arg("--vault")
        .arg(vault)
        .arg("--json")
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8(out.stdout).unwrap();
    assert_eq!(text.lines().count(), 1, "--json 不混入进度或人类说明");
    serde_json::from_str(&text).unwrap()
}

#[test]
fn first_use_explains_setup_without_creating_or_replacing_a_vault() {
    let dir = tempfile::tempdir().unwrap();
    let vault = dir.path().join("new vault");
    for args in [
        vec!["status"],
        vec!["search", "合成内容"],
        vec!["mcp", "--scope", "personal"],
    ] {
        let out = Command::new(env!("CARGO_BIN_EXE_recallcard"))
            .arg("--vault")
            .arg(&vault)
            .args(args)
            .output()
            .unwrap();
        assert!(!out.status.success());
        let error = String::from_utf8(out.stderr).unwrap();
        assert!(
            error.contains("recallcard setup") && error.contains("--vault"),
            "{error}"
        );
        assert!(!vault.exists());
    }
    std::fs::create_dir(&vault).unwrap();
    let existing = vault.join("notes.txt");
    std::fs::write(&existing, "合成已有内容").unwrap();
    let setup = Command::new(env!("CARGO_BIN_EXE_recallcard"))
        .arg("--vault")
        .arg(&vault)
        .arg("setup")
        .output()
        .unwrap();
    assert!(!setup.status.success());
    assert_eq!(std::fs::read_to_string(existing).unwrap(), "合成已有内容");
    assert!(!vault.join("control/schema-version.json").exists());
}

#[test]
fn official_format_alias_and_auto_use_the_same_importer() {
    let dir = tempfile::tempdir().unwrap();
    let vault = dir.path().join("vault");
    cli(&vault, &["setup"]);
    let source = dir.path().join("official.json");
    std::fs::write(&source, json!({"id":"format-example","title":"合成格式样本","inserted_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-01T00:00:00Z","mapping":{"u":{"id":"u","parent":null,"children":[],"message":{"inserted_at":"2026-01-01T00:00:00Z","fragments":[{"type":"REQUEST","content":"合成的原始记录"}]}}}}).to_string()).unwrap();
    for (index, format) in ["deepseek", "deepseek-export", "auto"].iter().enumerate() {
        let imported = cli(
            &vault,
            &["import", source.to_str().unwrap(), "--format", format],
        );
        assert_eq!(imported["result"]["job"]["state"], "completed");
        assert_eq!(
            imported["result"]["job"]["progress"]["events_added"],
            if index == 0 { 1 } else { 0 }
        );
    }
    assert_eq!(cli(&vault, &["doctor"])["events"], 1);
    for format in ["other-format", "chatgpt"] {
        let output = Command::new(env!("CARGO_BIN_EXE_recallcard"))
            .arg("--vault")
            .arg(&vault)
            .args(["import", source.to_str().unwrap(), "--format", format])
            .output()
            .unwrap();
        assert!(!output.status.success(), "指定错误平台不能默默改用 auto");
    }
    let help = Command::new(env!("CARGO_BIN_EXE_recallcard"))
        .args(["import", "--help"])
        .output()
        .unwrap();
    let text = String::from_utf8(help.stdout).unwrap();
    assert!(text.contains("auto") && text.contains("deepseek") && text.contains("chatgpt"));
}
#[test]
fn cli_alone_imports_reads_sources_and_exposes_native_ai_entry() {
    let dir = tempfile::tempdir().unwrap();
    let vault = dir.path().join("vault");
    let setup = cli(&vault, &["setup"]);
    assert_eq!(setup["ok"], true);
    let source = dir.path().join("official.json");
    std::fs::write(&source,json!({"id":"cli-ai-source","current_node":"u","mapping":{"u":{"parent":null,"children":[],"message":{"id":"u","author":{"role":"user"},"create_time":1791241200,"content":{"content_type":"text","parts":["合成CLI事实：石榴项目使用可追溯出处。"]}}}}}).to_string()).unwrap();
    let imported = cli(&vault, &["import", source.to_str().unwrap()]);
    assert_eq!(imported["result"]["job"]["state"], "completed");
    let found = cli(
        &vault,
        &[
            "search",
            "石榴项目",
            "--scope",
            "personal",
            "--budget-bytes",
            "4000",
        ],
    );
    let reference = found["results"][0]["ref"].as_str().unwrap();
    let read = cli(&vault, &["read", reference]);
    assert!(read.to_string().contains("可追溯出处"));
    let memory = dir.path().join("memory.json");
    std::fs::write(&memory,json!({"content":"合成CLI事实：石榴项目使用可追溯出处。","source_refs":[reference.strip_prefix("event:").unwrap()],"evidence":"user_explicit","scope":"personal"}).to_string()).unwrap();
    let created = cli(
        &vault,
        &["memory", "add", "--file", memory.to_str().unwrap()],
    );
    let sources = cli(&vault, &["sources", created["id"].as_str().unwrap()]);
    assert!(sources.to_string().contains("cli-ai-source"));
    let status = cli(&vault, &["status"]);
    assert_eq!(status["result"]["service"]["running"], false);
    let connection = cli(
        &vault,
        &[
            "connect",
            "chatgpt-mcp",
            "--host-identity",
            "openai-chatgpt",
            "--platform",
            "chatgpt",
            "--recall-scope",
            "personal",
            "--provider-disclosure",
            "--auto-recall",
        ],
    );
    assert_eq!(connection["result"]["grant"]["client_kind"], "chatgpt_mcp");
    assert!(connection["result"]["last_read_at"].is_null());
    let help = Command::new(env!("CARGO_BIN_EXE_recallcard"))
        .arg("--help")
        .output()
        .unwrap();
    let help = String::from_utf8(help.stdout).unwrap();
    assert!(help.contains("\n  mcp ") && help.contains("\n  bootstrap "));
    let alias = cli(&vault, &["search", "石榴项目", "--budget-tokens", "4000"]);
    assert!(!alias["results"].as_array().unwrap().is_empty());
}

#[test]
fn cli_can_locate_and_continue_long_evidence_with_explicit_scope() {
    let dir = tempfile::tempdir().unwrap();
    let vault = dir.path().join("vault");
    cli(&vault, &["setup"]);
    let body = format!(
        "{}\nCLI_PAGING_ANCHOR：合成尾部证据 AA42。",
        "合成普通观察记录。\n".repeat(4000)
    );
    let source = dir.path().join("long.json");
    std::fs::write(&source,json!({"id":"cli-long","current_node":"u","mapping":{"u":{"parent":null,"children":[],"message":{"id":"u","author":{"role":"user"},"create_time":null,"content":{"content_type":"text","parts":[body]}}}}}).to_string()).unwrap();
    cli(&vault, &["import", source.to_str().unwrap()]);
    let found = cli(
        &vault,
        &[
            "search",
            "CLI_PAGING_ANCHOR",
            "--scope",
            "personal",
            "--budget-bytes",
            "4000",
        ],
    );
    let hit = &found["results"][0];
    let reference = hit["ref"].as_str().unwrap();
    assert_eq!(hit["text_truncated"], true);
    let offset = hit["text_range"]["start_byte"]
        .as_u64()
        .unwrap()
        .to_string();
    let tail = cli(
        &vault,
        &[
            "read",
            reference,
            "--scope",
            "personal",
            "--offset-bytes",
            &offset,
            "--budget-bytes",
            "4000",
        ],
    );
    assert!(tail["results"][0]["text"]
        .as_str()
        .unwrap()
        .contains("AA42"));
    let first = cli(
        &vault,
        &[
            "read",
            reference,
            "--scope",
            "personal",
            "--offset-bytes",
            "0",
            "--budget-bytes",
            "2000",
        ],
    );
    let cursor = first["next_cursor"].as_str().unwrap();
    let second = cli(
        &vault,
        &[
            "read",
            reference,
            "--scope",
            "personal",
            "--cursor",
            cursor,
            "--budget-bytes",
            "2000",
        ],
    );
    assert_eq!(
        first["results"][0]["text_range"]["end_byte"],
        second["results"][0]["text_range"]["start_byte"]
    );
    let source_page = cli(
        &vault,
        &[
            "sources",
            reference,
            "--scope",
            "personal",
            "--offset-bytes",
            &offset,
            "--budget-bytes",
            "4000",
        ],
    );
    assert!(source_page["results"][0]["text"]
        .as_str()
        .unwrap()
        .contains("AA42"));
}
