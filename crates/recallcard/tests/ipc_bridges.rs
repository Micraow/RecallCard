//! 真实 RecallCard daemon、MCP 与 Native 子进程的离线合同。
use recallcard::{ipc, policy::Access, EventInput, Vault};
use serde_json::{json, Value};
use std::{
    io::Write,
    path::{Path, PathBuf},
    process::{Child, Command, Output, Stdio},
    time::{Duration, Instant},
};
const EXTENSION: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
struct Daemon(Child);
impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn setup() -> (tempfile::TempDir, Vault, PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(&d.path().join("vault")).unwrap();
    #[cfg(unix)]
    let endpoint = d
        .path()
        .canonicalize()
        .unwrap()
        .join("ipc")
        .join("core.sock");
    #[cfg(windows)]
    let endpoint =
        ipc::default_endpoint(&v, &Access::new(vec!["personal".into()]).unwrap()).unwrap();
    for (scope, key, text) in [
        ("personal", "visible", "合成跨进程Rust决定"),
        ("project:secret", "hidden", "隐藏跨进程Rust决定"),
    ] {
        let input: EventInput = serde_json::from_value(
            json!({"role":"user","origin":"native","scope":scope,"content":text,
            "source":{"platform":"manual-web","conversation_id":"bridge","message_id":key}}),
        )
        .unwrap();
        v.capture(input).unwrap();
    }
    (d, v, endpoint)
}
fn start(v: &Vault, endpoint: &Path) -> Daemon {
    let child = Command::new(env!("CARGO_BIN_EXE_recallcard"))
        .arg("--vault")
        .arg(v.root())
        .args(["daemon", "--scope", "personal", "--endpoint"])
        .arg(endpoint)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let mut daemon = Daemon(child);
    let client = ipc::Client::new(
        v,
        &Access::new(vec!["personal".into()]).unwrap(),
        endpoint.into(),
        Duration::from_secs(1),
    )
    .unwrap();
    let end = Instant::now() + Duration::from_secs(15);
    loop {
        assert!(daemon.0.try_wait().unwrap().is_none(), "daemon 已退出");
        if client.invoke("bootstrap", json!({})).is_ok() {
            return daemon;
        }
        assert!(Instant::now() < end, "daemon 未就绪");
        std::thread::sleep(Duration::from_millis(15));
    }
}
fn run(v: &Vault, args: &[&str], bytes: &[u8]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_recallcard"))
        .arg("--vault")
        .arg(v.root())
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(bytes).unwrap();
    child.wait_with_output().unwrap()
}
fn mcp(v: &Vault, endpoint: &Path, scope: &str) -> Value {
    let mut input = String::new();
    for request in [
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25"}}),
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"search","arguments":{"query":"Rust","budget_tokens":12000}}}),
    ] {
        input.push_str(&request.to_string());
        input.push('\n');
    }
    let output = run(
        v,
        &[
            "mcp",
            "--scope",
            scope,
            "--ipc-endpoint",
            endpoint.to_str().unwrap(),
        ],
        input.as_bytes(),
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|s| serde_json::from_str::<Value>(s).unwrap())
        .find(|v| v["id"] == 2)
        .unwrap()["result"]
        .clone()
}
fn native_frame() -> Vec<u8> {
    let request = json!({"protocol":"recallcard.action/1","request_id":"bridge-test","nonce":"a".repeat(48),"action":"search","session_ref":"chatgpt:synthetic","arguments":{"query":"Rust","budget_tokens":12000}});
    let bytes = serde_json::to_vec(&request).unwrap();
    let mut frame = (bytes.len() as u32).to_ne_bytes().to_vec();
    frame.extend(bytes);
    frame
}
fn decode(output: Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.len() > 4);
    assert_eq!(
        u32::from_ne_bytes(output.stdout[..4].try_into().unwrap()) as usize,
        output.stdout.len() - 4
    );
    serde_json::from_slice(&output.stdout[4..]).unwrap()
}
#[test]
fn real_daemon_is_shared_by_mcp_and_native_without_scope_expansion() {
    let (_d, v, endpoint) = setup();
    let _daemon = start(&v, &endpoint);
    let result = mcp(&v, &endpoint, "personal");
    assert_eq!(result["isError"], false);
    let body: Value = serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(body["results"].as_array().unwrap().len(), 1);
    assert!(!body.to_string().contains("隐藏"));
    let result = decode(run(
        &v,
        &[
            "native-host",
            "--scope",
            "personal",
            "--allowed-extension",
            EXTENSION,
            "--ipc-endpoint",
            endpoint.to_str().unwrap(),
            "chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/",
        ],
        &native_frame(),
    ));
    assert_eq!(result["ok"], true);
    assert!(!result.to_string().contains("隐藏"));
    assert_eq!(mcp(&v, &endpoint, "project:secret")["isError"], true);
}
#[test]
fn stopped_daemon_does_not_fall_back_to_direct_vault_reads() {
    let (_d, v, endpoint) = setup();
    let daemon = start(&v, &endpoint);
    drop(daemon);
    let result = mcp(&v, &endpoint, "personal");
    assert_eq!(result["isError"], true);
    assert!(!result.to_string().contains("合成跨进程Rust决定"));
    let result = decode(run(
        &v,
        &[
            "native-host",
            "--scope",
            "personal",
            "--allowed-extension",
            EXTENSION,
            "--ipc-endpoint",
            endpoint.to_str().unwrap(),
            "chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/",
        ],
        &native_frame(),
    ));
    assert_eq!(result["ok"], false);
    assert!(!result.to_string().contains("合成跨进程Rust决定"));
}
#[test]
fn installed_launcher_uses_fixed_ipc_configuration() {
    let (d, v, endpoint) = setup();
    let _daemon = start(&v, &endpoint);
    let install = d.path().join("installed");
    let output = run(
        &v,
        &[
            "native-install",
            "--scope",
            "personal",
            "--extension-id",
            EXTENSION,
            "--output-dir",
            install.to_str().unwrap(),
            "--ipc-endpoint",
            endpoint.to_str().unwrap(),
        ],
        &[],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    let config: Value =
        serde_json::from_slice(&std::fs::read(value["config"].as_str().unwrap()).unwrap()).unwrap();
    assert_eq!(config["ipc_endpoint"], endpoint.to_str().unwrap());
    let mut child = Command::new(value["launcher"].as_str().unwrap())
        .arg("chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(&native_frame())
        .unwrap();
    let result = decode(child.wait_with_output().unwrap());
    assert_eq!(result["ok"], true);
    assert!(!result.to_string().contains("隐藏"));
}
