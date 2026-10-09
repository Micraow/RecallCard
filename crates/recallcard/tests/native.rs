use recallcard::{native, policy::Access, Vault};
use serde_json::{json, Value};
// 可执行副本 fixture 与启动必须串行，避免 fork/exec 与另一测试写文件重叠。
// 仅约束本测试二进制，不影响 Vault 的并发读写回归测试。
static NATIVE_FIXTURE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

const EXT: &str = "abcdefghijklmnopabcdefghijklmnop";
fn request(action: &str) -> Value {
    json!({"protocol":"recallcard.action/1","request_id":"r_demo","nonce":"a-long-synthetic-nonce","session_ref":"chatgpt:demo","action":action,"arguments":{}})
}
fn frame(v: &Value) -> Vec<u8> {
    let body = serde_json::to_vec(v).unwrap();
    let mut out = (body.len() as u32).to_ne_bytes().to_vec();
    out.extend(body);
    out
}
fn run(input: Vec<u8>, origin: &str) -> Result<Vec<Value>, String> {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(&d.path().join("vault")).unwrap();
    let mut out = Vec::new();
    native::serve_native_io(
        &v,
        Access::new(vec!["personal".into()]).unwrap(),
        EXT,
        origin,
        std::io::Cursor::new(input),
        &mut out,
    )?;
    let mut at = 0;
    let mut result = vec![];
    while at < out.len() {
        let len = u32::from_ne_bytes(out[at..at + 4].try_into().unwrap()) as usize;
        at += 4;
        result.push(serde_json::from_slice(&out[at..at + len]).unwrap());
        at += len;
    }
    Ok(result)
}
#[test]
fn allowed_browser_origin_can_only_read() {
    let _fixture_guard = NATIVE_FIXTURE_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let origin = native::extension_origin(EXT).unwrap();
    assert_eq!(
        run(frame(&request("bootstrap")), &origin).unwrap()[0]["ok"],
        true
    );
    assert_eq!(
        run(frame(&request("capture")), &origin).unwrap()[0]["ok"],
        false
    );
}
#[test]
fn foreign_browser_origin_and_invalid_id_are_rejected() {
    let _fixture_guard = NATIVE_FIXTURE_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    assert!(run(frame(&request("bootstrap")), "https://chatgpt.com/").is_err());
    assert!(native::extension_origin("../evil").is_err());
}
#[test]
fn native_messages_are_bounded_and_require_complete_frames() {
    let _fixture_guard = NATIVE_FIXTURE_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let origin = native::extension_origin(EXT).unwrap();
    assert!(run((262145u32).to_ne_bytes().to_vec(), &origin).is_err());
    assert!(run(vec![1, 2], &origin).is_err());
    assert!(run(vec![5, 0, 0, 0, b'{'], &origin).is_err());
}
#[test]
fn unknown_fields_and_invalid_nonce_are_rejected() {
    let _fixture_guard = NATIVE_FIXTURE_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let origin = native::extension_origin(EXT).unwrap();
    let mut r = request("bootstrap");
    r["shell"] = json!("false");
    assert_eq!(run(frame(&r), &origin).unwrap()[0]["ok"], false);
    r.as_object_mut().unwrap().remove("shell");
    r["nonce"] = json!("short");
    assert_eq!(run(frame(&r), &origin).unwrap()[0]["ok"], false);
}
#[test]
fn duplicate_id_with_changed_arguments_is_not_executed() {
    let _fixture_guard = NATIVE_FIXTURE_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let origin = native::extension_origin(EXT).unwrap();
    let r = request("bootstrap");
    let mut next = r.clone();
    next["arguments"] = json!({"budget_tokens":512});
    let mut input = frame(&r);
    input.extend(frame(&r));
    input.extend(frame(&next));
    let responses = run(input, &origin).unwrap();
    assert_eq!(responses[0], responses[1]);
    assert_eq!(responses[2]["ok"], false);
}
#[cfg(unix)]
#[test]
fn installer_generates_restricted_files_without_registering() {
    let _fixture_guard = NATIVE_FIXTURE_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(&d.path().join("vault")).unwrap();
    let out = d.path().join("install");
    let result = native::prepare_install(&v, vec!["personal".into()], EXT, &out).unwrap();
    assert_eq!(result["registered"], false);
    let manifest: Value =
        serde_json::from_slice(&std::fs::read(out.join("com.recallcard.host.json")).unwrap())
            .unwrap();
    assert_eq!(manifest["allowed_origins"].as_array().unwrap().len(), 1);
    assert!(native::prepare_install(&v, vec!["personal".into()], EXT, &out).is_err());
}

#[cfg(unix)]
#[test]
fn installer_resolves_selected_parent_alias_but_rejects_linked_root() {
    let _fixture_guard = NATIVE_FIXTURE_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    use std::os::unix::fs::symlink;
    let directory = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(directory.path()).unwrap();
    let vault = Vault::init(&root.join("vault")).unwrap();
    let real = root.join("actual");
    std::fs::create_dir(&real).unwrap();
    let alias = root.join("parent-alias");
    symlink(&real, &alias).unwrap();
    let result =
        native::prepare_install(&vault, vec!["personal".into()], EXT, &alias.join("install"))
            .unwrap();
    assert_eq!(
        result["manifest"],
        serde_json::json!(real.join("install/com.recallcard.host.json"))
    );
    let linked_root = root.join("linked-root");
    symlink(real.join("install"), &linked_root).unwrap();
    assert!(native::prepare_install(&vault, vec!["personal".into()], EXT, &linked_root).is_err());
}

fn decode_frames(output: &[u8]) -> Vec<Value> {
    let mut rest = output;
    let mut values = Vec::new();
    while !rest.is_empty() {
        assert!(rest.len() >= 4, "标准输出必须只有完整 Native 消息");
        let length = u32::from_ne_bytes(rest[..4].try_into().unwrap()) as usize;
        assert!(rest.len() >= 4 + length);
        values.push(serde_json::from_slice(&rest[4..4 + length]).unwrap());
        rest = &rest[4 + length..];
    }
    values
}

fn installed() -> (tempfile::TempDir, Vault, Value) {
    let directory = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(directory.path()).unwrap();
    let vault = Vault::init(&root.join("vault 合成")).unwrap();
    let source = root.join(if cfg!(windows) {
        "source-cli.exe"
    } else {
        "source-cli"
    });
    std::fs::copy(env!("CARGO_BIN_EXE_recallcard"), &source).unwrap();
    let output = std::process::Command::new(&source)
        .arg("--vault")
        .arg(vault.root())
        .args([
            "native-install",
            "--extension-id",
            EXT,
            "--scope",
            "personal",
            "--output-dir",
        ])
        .arg(root.join("native 合成 ' folder"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result = serde_json::from_slice(&output.stdout).unwrap();
    // 生成后删除源二进制，所有子进程验收均只依赖安装副本。
    std::fs::remove_file(source).unwrap();
    (directory, vault, result)
}

fn launch(
    executable: &str,
    directory: &std::path::Path,
    arguments: &[&str],
    input: &[u8],
) -> std::process::Output {
    use std::io::Write;
    let mut process = std::process::Command::new(executable)
        .args(arguments)
        .current_dir(directory)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = process.stdin.take().unwrap();
    // 预期拒绝的子进程可能在写入前退出。
    let _ = stdin.write_all(input);
    drop(stdin);
    process.wait_with_output().unwrap()
}

fn assert_launch_failure(output: std::process::Output) {
    assert!(!output.status.success());
    assert!(output.stdout.is_empty(), "错误不能污染 Native 标准输出");
    assert!(!output.stderr.is_empty());
}

fn config_value() -> Value {
    let vault = std::env::current_dir().unwrap().join("synthetic-vault");
    json!({
        "schema_version":1,
        "host":"com.recallcard.host",
        "vault":vault,
        "scopes":["personal","project:demo"],
        "extension_id":EXT
    })
}

#[test]
fn launcher_config_is_strict_bounded_and_pure() {
    let _fixture_guard = NATIVE_FIXTURE_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let valid = config_value();
    assert!(native::parse_launcher_config(&serde_json::to_vec(&valid).unwrap()).is_ok());
    for (field, value) in [
        ("schema_version", json!(2)),
        ("host", json!("com.other.host")),
        ("vault", json!("relative-vault")),
        (
            "vault",
            json!(std::env::current_dir().unwrap().join("../vault")),
        ),
        ("vault", json!("\u{0}")),
        ("scopes", json!([])),
        ("scopes", json!(["personal", "personal"])),
        ("scopes", json!(["*"])),
        ("scopes", json!(["--scope secret"])),
        (
            "scopes",
            json!((0..33).map(|n| format!("scope:{n}")).collect::<Vec<_>>()),
        ),
        ("extension_id", json!("https://chatgpt.com/")),
        ("extension_id", json!("ABCDEFGHIJKLMNOPABCDEFGHIJKLMNOP")),
        ("semantic_config", json!("relative.json")),
        (
            "semantic_config",
            json!(format!(
                "{}/../semantic.json",
                std::env::temp_dir().display()
            )),
        ),
        ("command", json!("shell")),
        ("api_key", json!("not-a-real-key")),
    ] {
        let mut invalid = valid.clone();
        invalid[field] = value;
        assert!(
            native::parse_launcher_config(&serde_json::to_vec(&invalid).unwrap()).is_err(),
            "{invalid}"
        );
    }
    let mut missing = valid.clone();
    missing.as_object_mut().unwrap().remove("extension_id");
    assert!(native::parse_launcher_config(&serde_json::to_vec(&missing).unwrap()).is_err());
    let duplicate =
        serde_json::to_string(&valid)
            .unwrap()
            .replacen('{', "{\"schema_version\":1,", 1);
    assert!(native::parse_launcher_config(duplicate.as_bytes()).is_err());
    assert!(native::parse_launcher_config(&vec![b' '; 16 * 1024 + 1]).is_err());
}

#[test]
fn copied_launcher_reads_fixed_config_ignores_working_directory_and_keeps_scope() {
    let _fixture_guard = NATIVE_FIXTURE_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let (directory, vault, install) = installed();
    let mut ids = Vec::new();
    for scope in ["personal", "project:secret"] {
        let event = serde_json::from_value(json!({
            "occurred_at":"2026-10-06T10:00:00Z","role":"user","origin":"native",
            "scope":scope,"content":format!("合成资料 {scope}"),
            "source":{"platform":"manual-web","conversation_id":"demo","message_id":scope}
        }))
        .unwrap();
        ids.push(vault.capture(event).unwrap().id);
    }
    let foreign_config = directory.path().join("com.recallcard.host.config.json");
    std::fs::write(&foreign_config, b"not a usable config").unwrap();
    let launcher = install["launcher"].as_str().unwrap();
    let mut input = Vec::new();
    for (number, id) in ids.iter().enumerate() {
        let mut read = request("read");
        read["request_id"] = json!(format!("read_{number}"));
        read["arguments"] = json!({"refs":[format!("event:{id}")]});
        input.extend(frame(&read));
    }
    let mut escalation = request("search");
    escalation["arguments"] = json!({"query":"合成", "scope":"project:secret"});
    input.extend(frame(&escalation));
    let result = launch(
        launcher,
        directory.path(),
        &[&native::extension_origin(EXT).unwrap(), "--parent-window=0"],
        &input,
    );
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(result.stderr.is_empty());
    let replies = decode_frames(&result.stdout);
    assert_eq!(replies.len(), 3);
    assert_eq!(replies[0]["ok"], true);
    assert_eq!(replies[1]["ok"], false);
    assert_eq!(replies[2]["ok"], false);
    assert!(!String::from_utf8_lossy(&result.stdout).contains("合成资料 project:secret"));
}

#[test]
fn copied_launcher_refuses_foreign_origins_and_cli_argument_overrides() {
    let _fixture_guard = NATIVE_FIXTURE_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let (directory, _vault, install) = installed();
    let launcher = install["launcher"].as_str().unwrap();
    let origin = native::extension_origin(EXT).unwrap();
    for arguments in [
        vec![],
        vec!["--help"],
        vec!["native-host"],
        vec!["https://chatgpt.com/"],
        vec!["chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/"],
        vec![origin.trim_end_matches('/')],
        vec![&origin, "--vault=/tmp/foreign"],
        vec![&origin, "--config=/tmp/foreign"],
        vec![&origin, "--scope=project:secret"],
        vec![&origin, "--parent-window=-1"],
        vec![&origin, "--parent-window=+1"],
        vec![&origin, "--parent-window=18446744073709551616"],
        vec![&origin, "--parent-window=0", "--scope=project:secret"],
    ] {
        assert_launch_failure(launch(launcher, directory.path(), &arguments, &[]));
    }
}

#[test]
fn copied_launcher_refuses_missing_invalid_oversized_or_directory_config() {
    let _fixture_guard = NATIVE_FIXTURE_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let (directory, _vault, install) = installed();
    let launcher = install["launcher"].as_str().unwrap();
    let config = std::path::Path::new(install["config"].as_str().unwrap());
    let origin = native::extension_origin(EXT).unwrap();
    let original = std::fs::read(config).unwrap();
    for bytes in [b"{invalid".to_vec(), vec![b' '; 16 * 1024 + 1]] {
        std::fs::write(config, bytes).unwrap();
        assert_launch_failure(launch(launcher, directory.path(), &[&origin], &[]));
    }
    let mut wrong: Value = serde_json::from_slice(&original).unwrap();
    wrong["extension_id"] = json!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    std::fs::write(config, serde_json::to_vec(&wrong).unwrap()).unwrap();
    assert_launch_failure(launch(launcher, directory.path(), &[&origin], &[]));
    std::fs::remove_file(config).unwrap();
    assert_launch_failure(launch(launcher, directory.path(), &[&origin], &[]));
    std::fs::create_dir(config).unwrap();
    assert_launch_failure(launch(launcher, directory.path(), &[&origin], &[]));
}

#[test]
fn copied_launcher_preserves_binary_frames_and_reports_truncation_on_stderr() {
    let _fixture_guard = NATIVE_FIXTURE_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let (directory, _vault, install) = installed();
    let launcher = install["launcher"].as_str().unwrap();
    let origin = native::extension_origin(EXT).unwrap();
    let mut request = request("bootstrap");
    // JSON 内含多字节字符和转义换行；帧前缀只按 UTF-8 字节数计数。
    request["session_ref"] = json!("合成\n会话🧪");
    let output = launch(launcher, directory.path(), &[&origin], &frame(&request));
    assert!(output.status.success());
    assert_eq!(decode_frames(&output.stdout)[0]["ok"], true);
    assert_launch_failure(launch(launcher, directory.path(), &[&origin], &[1, 2]));
    assert_launch_failure(launch(
        launcher,
        directory.path(),
        &[&origin],
        &262145u32.to_ne_bytes(),
    ));
}

#[test]
fn installer_copies_cli_and_publishes_matching_config_and_manifest_without_overwrite() {
    let _fixture_guard = NATIVE_FIXTURE_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let (directory, vault, install) = installed();
    assert_eq!(install["registered"], false);
    let launcher = std::path::Path::new(install["launcher"].as_str().unwrap());
    let config = std::path::Path::new(install["config"].as_str().unwrap());
    let manifest_path = std::path::Path::new(install["manifest"].as_str().unwrap());
    assert_eq!(
        std::fs::read(launcher).unwrap(),
        std::fs::read(env!("CARGO_BIN_EXE_recallcard")).unwrap()
    );
    let original_config = std::fs::read(config).unwrap();
    let parsed: Value = serde_json::from_slice(&original_config).unwrap();
    assert_eq!(parsed["vault"], json!(vault.root()));
    assert_eq!(parsed["extension_id"], EXT);
    assert_eq!(parsed["scopes"], json!(["personal"]));
    let manifest: Value = serde_json::from_slice(&std::fs::read(manifest_path).unwrap()).unwrap();
    assert_eq!(
        manifest["allowed_origins"],
        json!([native::extension_origin(EXT).unwrap()])
    );
    let host = manifest["path"].as_str().unwrap();
    let result = launch(
        host,
        directory.path(),
        &[&native::extension_origin(EXT).unwrap()],
        &frame(&request("bootstrap")),
    );
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(decode_frames(&result.stdout)[0]["ok"], true);
    assert!(native::prepare_install(
        &vault,
        vec!["project:secret".into()],
        EXT,
        launcher.parent().unwrap()
    )
    .is_err());
    assert_eq!(std::fs::read(config).unwrap(), original_config);
    #[cfg(windows)]
    {
        assert_eq!(manifest["path"], install["launcher"]);
        assert!(install["wrapper"].is_null());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(manifest["path"], install["wrapper"]);
        assert_eq!(
            std::fs::metadata(config).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::metadata(launcher).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }
}

#[cfg(unix)]
#[test]
fn installer_and_launcher_refuse_symlinks_and_shared_writable_configuration() {
    let _fixture_guard = NATIVE_FIXTURE_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    use std::os::unix::fs::{symlink, PermissionsExt};
    let (directory, vault, install) = installed();
    let launcher = install["launcher"].as_str().unwrap();
    let config = std::path::Path::new(install["config"].as_str().unwrap());
    let origin = native::extension_origin(EXT).unwrap();
    let original = std::fs::read(config).unwrap();
    std::fs::set_permissions(config, std::fs::Permissions::from_mode(0o666)).unwrap();
    assert_launch_failure(launch(launcher, directory.path(), &[&origin], &[]));
    std::fs::set_permissions(config, std::fs::Permissions::from_mode(0o600)).unwrap();
    let host_dir = config.parent().unwrap();
    std::fs::set_permissions(host_dir, std::fs::Permissions::from_mode(0o777)).unwrap();
    assert_launch_failure(launch(launcher, directory.path(), &[&origin], &[]));
    assert!(native::prepare_install(&vault, vec!["personal".into()], EXT, host_dir).is_err());
    std::fs::set_permissions(host_dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    let link_dir = directory.path().join("linked-install");
    symlink(host_dir, &link_dir).unwrap();
    assert!(native::prepare_install(&vault, vec!["personal".into()], EXT, &link_dir).is_err());
    let linked_vault = directory.path().join("linked-vault");
    symlink(vault.root(), &linked_vault).unwrap();
    let mut value: Value = serde_json::from_slice(&original).unwrap();
    value["vault"] = json!(linked_vault);
    std::fs::write(config, serde_json::to_vec(&value).unwrap()).unwrap();
    assert_launch_failure(launch(launcher, directory.path(), &[&origin], &[]));
    let destination = directory.path().join("safe-config.json");
    std::fs::write(&destination, &original).unwrap();
    std::fs::remove_file(config).unwrap();
    symlink(&destination, config).unwrap();
    assert_launch_failure(launch(launcher, directory.path(), &[&origin], &[]));
    assert_eq!(std::fs::read(&destination).unwrap(), original);
    let empty = directory.path().join("new-install");
    std::fs::create_dir(&empty).unwrap();
    symlink(
        directory.path().join("does-not-exist"),
        empty.join("com.recallcard.host.json"),
    )
    .unwrap();
    assert!(native::prepare_install(&vault, vec!["personal".into()], EXT, &empty).is_err());
    assert_eq!(std::fs::read_dir(empty).unwrap().count(), 1);
}

#[test]
fn ordinary_cli_still_displays_help_and_does_not_load_native_config() {
    let _fixture_guard = NATIVE_FIXTURE_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join("com.recallcard.host.config.json"),
        b"invalid",
    )
    .unwrap();
    let output = launch(
        env!("CARGO_BIN_EXE_recallcard"),
        directory.path(),
        &["--help"],
        &[],
    );
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("mcp"));
    let legacy = launch(
        env!("CARGO_BIN_EXE_recallcard"),
        directory.path(),
        &["native-install", "--help"],
        &[],
    );
    assert!(legacy.status.success());
}
