use recallcard::{
    ipc::{self, Client, Options, Server},
    policy::Access,
    EventInput, Vault,
};
use serde_json::json;
#[cfg(unix)]
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

fn access() -> Access {
    Access::new(vec!["personal".into()]).unwrap()
}
fn client(vault: &Vault, endpoint: &Path) -> Client {
    Client::new(vault, &access(), endpoint.into(), Duration::from_secs(2)).unwrap()
}
fn fixture() -> (tempfile::TempDir, Vault, PathBuf) {
    let directory = tempfile::tempdir().unwrap();
    let vault = Vault::init(&directory.path().join("vault")).unwrap();
    #[cfg(unix)]
    let endpoint = std::fs::canonicalize(directory.path())
        .unwrap()
        .join("private")
        .join("core.sock");
    #[cfg(windows)]
    let endpoint = ipc::default_endpoint(&vault, &access()).unwrap();
    (directory, vault, endpoint)
}
fn capture(vault: &Vault, scope: &str, key: &str, text: &str) -> String {
    let event: EventInput = serde_json::from_value(
        json!({"role":"user","origin":"native","scope":scope,"content":text,
        "source":{"platform":"manual-web","conversation_id":"ipc-test","message_id":key}}),
    )
    .unwrap();
    vault.capture(event).unwrap().id
}
struct Running(Child);
impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn start(vault: &Vault, endpoint: &Path) -> Running {
    let child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "daemon_process_fixture", "--nocapture"])
        .env("RECALLCARD_IPC_TEST_VAULT", vault.root())
        .env("RECALLCARD_IPC_TEST_ENDPOINT", endpoint)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let mut running = Running(child);
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        assert!(
            running.0.try_wait().unwrap().is_none(),
            "daemon 子进程启动失败"
        );
        if client(vault, endpoint)
            .invoke("bootstrap", json!({}))
            .is_ok()
        {
            return running;
        }
        assert!(Instant::now() < deadline, "daemon 未按时就绪");
        std::thread::sleep(Duration::from_millis(10));
    }
}
#[test]
fn daemon_process_fixture() {
    let Some(root) = std::env::var_os("RECALLCARD_IPC_TEST_VAULT") else {
        return;
    };
    let endpoint = std::env::var_os("RECALLCARD_IPC_TEST_ENDPOINT").unwrap();
    Server::bind(
        Vault::open(Path::new(&root)).unwrap(),
        access(),
        endpoint.into(),
        Options {
            request_timeout: Duration::from_millis(300),
            max_connections: 2,
        },
    )
    .unwrap()
    .run()
    .unwrap();
}

#[test]
fn real_daemon_reads_current_events_and_rechecks_suppression() {
    let (_directory, vault, endpoint) = fixture();
    let _running = start(&vault, &endpoint);
    let id = capture(&vault, "personal", "visible", "daemon 刚刚决定使用 Rust");
    let hidden = capture(&vault, "project:secret", "hidden", "daemon 绝密哨兵");
    let client = client(&vault, &endpoint);
    let result = client
        .invoke("search", json!({"query":"daemon", "budget_tokens":12000}))
        .unwrap();
    assert!(result.to_string().contains(&id));
    assert!(!result.to_string().contains(&hidden));
    assert!(client
        .invoke("read", json!({"refs":[format!("event:{hidden}")]}))
        .is_err());
    assert!(client
        .invoke("search", json!({"query":"绝密", "scope":"project:secret"}))
        .is_err());
    assert!(client
        .invoke("read", json!({"refs":["../../etc/passwd"]}))
        .is_err());
    assert!(client.invoke("capture", json!({})).is_err());
    vault.suppress(&id, "IPC 合成遗忘验证".into()).unwrap();
    let result = client
        .invoke("search", json!({"query":"daemon", "budget_tokens":12000}))
        .unwrap();
    assert!(!result.to_string().contains(&id));
    assert!(client
        .invoke("read", json!({"refs":[format!("event:{id}")]}))
        .is_err());
}
#[test]
fn vault_and_scope_binding_prevents_wrong_daemon_use() {
    let (_directory, vault, endpoint) = fixture();
    let _running = start(&vault, &endpoint);
    let other_access = Access::new(vec!["project:secret".into()]).unwrap();
    let wrong_scope = Client::new(
        &vault,
        &other_access,
        endpoint.clone(),
        Duration::from_secs(2),
    )
    .unwrap();
    assert!(wrong_scope
        .invoke("bootstrap", json!({}))
        .unwrap_err()
        .contains("绑定"));
    let (_other_directory, other_vault, _) = fixture();
    assert!(client(&other_vault, &endpoint)
        .invoke("bootstrap", json!({}))
        .unwrap_err()
        .contains("绑定"));
    assert_ne!(
        ipc::default_endpoint(&vault, &access()).unwrap(),
        ipc::default_endpoint(&vault, &other_access).unwrap()
    );
}
#[test]
fn real_daemon_has_one_instance_and_recovers_after_process_death() {
    let (_directory, vault, endpoint) = fixture();
    let mut running = start(&vault, &endpoint);
    assert!(Server::bind(
        Vault::open(vault.root()).unwrap(),
        access(),
        endpoint.clone(),
        Options::default()
    )
    .is_err());
    assert!(client(&vault, &endpoint)
        .invoke("bootstrap", json!({}))
        .is_ok());
    running.0.kill().unwrap();
    running.0.wait().unwrap();
    let _replacement = start(&vault, &endpoint);
    assert!(client(&vault, &endpoint)
        .invoke("bootstrap", json!({}))
        .is_ok());
}
#[test]
fn controlled_shutdown_releases_endpoint_for_next_instance() {
    let (_directory, vault, endpoint) = fixture();
    let server = Server::bind(
        Vault::open(vault.root()).unwrap(),
        access(),
        endpoint.clone(),
        Options::default(),
    )
    .unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let shutdown = stop.clone();
    let worker = std::thread::spawn(move || server.run_until(shutdown));
    assert!(client(&vault, &endpoint)
        .invoke("bootstrap", json!({}))
        .is_ok());
    stop.store(true, Ordering::Release);
    worker.join().unwrap().unwrap();
    let _next = Server::bind(
        Vault::open(vault.root()).unwrap(),
        access(),
        endpoint,
        Options::default(),
    )
    .unwrap();
}
#[test]
fn client_rejects_oversized_payload_before_connecting() {
    let (_directory, vault, endpoint) = fixture();
    let error = client(&vault, &endpoint)
        .invoke("search", json!({"query":"x".repeat(ipc::MAX_FRAME_BYTES)}))
        .unwrap_err();
    assert!(error.contains("大小上限"));
}
#[test]
fn configuration_has_explicit_limits() {
    let (_directory, vault, endpoint) = fixture();
    for timeout in [Duration::ZERO, Duration::from_secs(121)] {
        assert!(Client::new(&vault, &access(), endpoint.clone(), timeout).is_err());
    }
    for max_connections in [0, 65] {
        assert!(Server::bind(
            Vault::open(vault.root()).unwrap(),
            access(),
            endpoint.clone(),
            Options {
                max_connections,
                ..Options::default()
            }
        )
        .is_err());
    }
    #[cfg(unix)]
    assert!(Client::new(
        &vault,
        &access(),
        "relative.sock".into(),
        Duration::from_secs(1)
    )
    .is_err());
    #[cfg(windows)]
    assert!(Client::new(
        &vault,
        &access(),
        r"\\server\pipe\recallcard-test".into(),
        Duration::from_secs(1)
    )
    .is_err());
}

#[cfg(unix)]
mod unix {
    use super::*;
    use std::{
        fs,
        io::{Read, Write},
        os::unix::{
            fs::{symlink, MetadataExt, PermissionsExt},
            net::{UnixListener, UnixStream},
        },
    };
    fn raw(endpoint: &Path, body: &[u8]) -> std::io::Result<Value> {
        let mut stream = UnixStream::connect(endpoint)?;
        stream.set_read_timeout(Some(Duration::from_secs(2)))?;
        stream.set_write_timeout(Some(Duration::from_secs(2)))?;
        stream.write_all(&(body.len() as u32).to_le_bytes())?;
        stream.write_all(body)?;
        let mut header = [0; 4];
        stream.read_exact(&mut header)?;
        let length = u32::from_le_bytes(header) as usize;
        assert!(length <= ipc::MAX_FRAME_BYTES);
        let mut bytes = vec![0; length];
        stream.read_exact(&mut bytes)?;
        stream.write_all(&[0x06])?;
        Ok(serde_json::from_slice(&bytes).unwrap())
    }
    fn request(vault: &Vault) -> Value {
        let mut bytes = vault.root().as_os_str().as_encoded_bytes().to_vec();
        bytes.push(0);
        bytes.extend(json!(access().scopes()).to_string().as_bytes());
        let binding = recallcard::model::hash(&bytes);
        json!({"protocol":ipc::PROTOCOL,"binding":binding,"request_id":"test1","method":"bootstrap","arguments":{}})
    }
    #[test]
    fn daemon_rejects_unknown_fields_versions_and_mutation_routes() {
        let (_directory, vault, endpoint) = fixture();
        let _running = start(&vault, &endpoint);
        for extra in ["scope", "vault", "endpoint", "shell"] {
            let mut message = request(&vault);
            message[extra] = json!("untrusted");
            assert!(
                raw(&endpoint, &serde_json::to_vec(&message).unwrap()).unwrap()["error"]
                    .is_string()
            );
        }
        for (field, value) in [
            ("protocol", "recallcard.ipc/9"),
            ("method", "capture"),
            ("request_id", "../../evil"),
        ] {
            let mut message = request(&vault);
            message[field] = json!(value);
            assert!(
                raw(&endpoint, &serde_json::to_vec(&message).unwrap()).unwrap()["error"]
                    .is_string()
            );
        }
        assert!(raw(&endpoint, b"{").unwrap()["error"].is_string());
        assert!(client(&vault, &endpoint)
            .invoke("bootstrap", json!({}))
            .is_ok());
    }
    #[test]
    fn bad_frames_and_slow_clients_do_not_stop_daemon() {
        let (_directory, vault, endpoint) = fixture();
        let _running = start(&vault, &endpoint);
        for header in [0u32, ipc::MAX_FRAME_BYTES as u32 + 1] {
            let mut stream = UnixStream::connect(&endpoint).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            stream.write_all(&header.to_le_bytes()).unwrap();
            let mut byte = [0];
            assert!(matches!(stream.read(&mut byte), Ok(0) | Err(_)));
        }
        let mut slow = UnixStream::connect(&endpoint).unwrap();
        slow.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        slow.write_all(&[42, 0]).unwrap();
        let begin = Instant::now();
        assert!(client(&vault, &endpoint)
            .invoke("bootstrap", json!({}))
            .is_ok());
        assert!(begin.elapsed() < Duration::from_secs(1));
        let mut byte = [0];
        assert!(matches!(slow.read(&mut byte), Ok(0) | Err(_)));
        assert!(begin.elapsed() < Duration::from_secs(2));
        assert!(client(&vault, &endpoint)
            .invoke("bootstrap", json!({}))
            .is_ok());
    }
    #[test]
    fn partial_body_and_slow_drip_have_a_total_deadline() {
        let (_directory, vault, endpoint) = fixture();
        let _running = start(&vault, &endpoint);
        let mut stream = UnixStream::connect(&endpoint).unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        stream.write_all(&100u32.to_le_bytes()).unwrap();
        let begin = Instant::now();
        loop {
            if stream.write_all(b" ").is_err() {
                break;
            }
            assert!(
                begin.elapsed() < Duration::from_secs(2),
                "慢速逐字节客户端不能无限续期"
            );
            std::thread::sleep(Duration::from_millis(40));
        }
        assert!(client(&vault, &endpoint)
            .invoke("bootstrap", json!({}))
            .is_ok());
    }
    #[test]
    fn private_socket_permissions_and_non_socket_files_are_preserved() {
        let (_directory, vault, endpoint) = fixture();
        {
            let _running = start(&vault, &endpoint);
            assert_eq!(
                fs::metadata(endpoint.parent().unwrap()).unwrap().mode() & 0o777,
                0o700
            );
            assert_eq!(fs::metadata(&endpoint).unwrap().mode() & 0o777, 0o600);
        }
        fs::remove_file(&endpoint).unwrap();
        fs::write(&endpoint, b"must not remove").unwrap();
        fs::set_permissions(&endpoint, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(Server::bind(
            Vault::open(vault.root()).unwrap(),
            access(),
            endpoint.clone(),
            Options::default()
        )
        .is_err());
        assert_eq!(fs::read(&endpoint).unwrap(), b"must not remove");
    }
    #[test]
    fn symlink_endpoint_parent_and_lock_are_never_followed() {
        let (directory, vault, endpoint) = fixture();
        fs::create_dir(endpoint.parent().unwrap()).unwrap();
        fs::set_permissions(
            endpoint.parent().unwrap(),
            fs::Permissions::from_mode(0o700),
        )
        .unwrap();
        let victim = directory.path().join("victim");
        fs::write(&victim, b"unchanged").unwrap();
        symlink(&victim, &endpoint).unwrap();
        assert!(Server::bind(
            Vault::open(vault.root()).unwrap(),
            access(),
            endpoint.clone(),
            Options::default()
        )
        .is_err());
        fs::remove_file(&endpoint).unwrap();
        fs::remove_file(endpoint.with_extension("lock")).unwrap();
        symlink(&victim, endpoint.with_extension("lock")).unwrap();
        assert!(Server::bind(
            Vault::open(vault.root()).unwrap(),
            access(),
            endpoint.clone(),
            Options::default()
        )
        .is_err());
        let alias = directory.path().join("alias");
        symlink(endpoint.parent().unwrap(), &alias).unwrap();
        assert!(Server::bind(
            Vault::open(vault.root()).unwrap(),
            access(),
            alias.join("core.sock"),
            Options::default()
        )
        .is_err());
        assert_eq!(fs::read(victim).unwrap(), b"unchanged");
    }
    #[test]
    fn shared_parent_and_live_foreign_socket_are_rejected() {
        let (_directory, vault, endpoint) = fixture();
        fs::create_dir(endpoint.parent().unwrap()).unwrap();
        fs::set_permissions(
            endpoint.parent().unwrap(),
            fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        assert!(Server::bind(
            Vault::open(vault.root()).unwrap(),
            access(),
            endpoint.clone(),
            Options::default()
        )
        .is_err());
        fs::set_permissions(
            endpoint.parent().unwrap(),
            fs::Permissions::from_mode(0o700),
        )
        .unwrap();
        let _foreign = UnixListener::bind(&endpoint).unwrap();
        fs::set_permissions(&endpoint, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(Server::bind(
            Vault::open(vault.root()).unwrap(),
            access(),
            endpoint.clone(),
            Options::default()
        )
        .is_err());
        assert!(endpoint.exists());
    }
    #[test]
    fn cleanup_does_not_remove_replaced_endpoint() {
        let (_directory, vault, endpoint) = fixture();
        let server = Server::bind(
            Vault::open(vault.root()).unwrap(),
            access(),
            endpoint.clone(),
            Options::default(),
        )
        .unwrap();
        fs::remove_file(&endpoint).unwrap();
        fs::write(&endpoint, b"replacement").unwrap();
        drop(server);
        assert_eq!(fs::read(&endpoint).unwrap(), b"replacement");
    }
    #[test]
    fn client_has_a_total_timeout_against_unresponsive_peer() {
        let (_directory, vault, endpoint) = fixture();
        fs::create_dir(endpoint.parent().unwrap()).unwrap();
        fs::set_permissions(
            endpoint.parent().unwrap(),
            fs::Permissions::from_mode(0o700),
        )
        .unwrap();
        let _listener = UnixListener::bind(&endpoint).unwrap();
        fs::set_permissions(&endpoint, fs::Permissions::from_mode(0o600)).unwrap();
        let client = Client::new(&vault, &access(), endpoint, Duration::from_millis(80)).unwrap();
        let begin = Instant::now();
        assert!(client
            .invoke("bootstrap", json!({}))
            .unwrap_err()
            .contains("超时"));
        assert!(begin.elapsed() < Duration::from_secs(2));
    }
}
