//! 共享进程宿主的合成验收；fake provider 验证调度，不验证真实模型或账单。
use chrono::Utc;
use recallcard::{
    application::{background_memory::*, runtime::*, AppResult, JobState},
    dream::DreamJob,
    Vault,
};
use serde_json::json;
use std::{
    process::Command,
    sync::mpsc,
    time::{Duration, Instant},
};
fn configuration() -> MemoryConfig {
    MemoryConfig {
        enabled: true,
        quiet_seconds: 0,
        provider: Some(MemoryProviderConfig {
            endpoint: "https://example.invalid/api".into(),
            model: "synthetic".into(),
        }),
        consent: Some(MemoryConsent {
            endpoint: "https://example.invalid/api".into(),
            model: "synthetic".into(),
            scope: "personal".into(),
            send_source_snapshots: true,
            send_memory_snapshots: true,
            auto_apply: true,
            accepted_at: Utc::now(),
        }),
        ..Default::default()
    }
}
fn source(vault: &Vault) {
    let input:recallcard::EventInput=serde_json::from_value(json!({"role":"user","origin":"native","content":"合成后台增量事实","source":{"platform":"synthetic","conversation_id":"service","message_id":"increment"}})).unwrap();
    for _ in 0..100 {
        if vault.capture(input.clone()).is_ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("合成来源未能捕获");
}
fn output(job: &DreamJob) -> ProviderOutput {
    ProviderOutput{result:serde_json::from_value(json!({"schema":"recallcard.dream-result/1","job_id":job.job_id,"input_hash":job.input_hash,"proposals":[{"operation":"add","scope":job.allowed_scope,"content":"合成后台增量事实","source_refs":[job.source_refs[0].reference],"evidence":"user_explicit","time_note":"发生时间未知"}]})).unwrap(),usage:ProviderUsage{input_tokens:Some(5),output_tokens:Some(5),request_bytes:100,network_calls:0},verification:"synthetic_protocol_only".into()}
}
struct Fake {
    completed: Option<mpsc::Sender<()>>,
}
impl MemoryProvider for Fake {
    fn execute(&mut self, _: &MemoryConfig, job: &DreamJob) -> AppResult<ProviderOutput> {
        if let Some(completed) = &self.completed {
            completed.send(()).unwrap();
        }
        Ok(output(job))
    }
}
fn wait_running(vault: &Vault) {
    let until = Instant::now() + Duration::from_secs(5);
    while Instant::now() < until {
        if service_status(vault).is_ok_and(|status| status.running) {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("服务未取得租约");
}
// 租约先于 worker 预检与完整程序 hash。需要凭据/调度就绪的测试等待
// 明确的启动发布，而不是在租约出现后给另一个任意的 5 秒窗口。
fn wait_started(
    vault: &Vault,
    pid: u32,
    not_before: chrono::DateTime<Utc>,
    mut child: Option<&mut std::process::Child>,
) -> ServiceStatus {
    let until = Instant::now() + SERVICE_STARTUP_TIMEOUT;
    loop {
        if let Some(process) = child.as_deref_mut() {
            assert!(
                process.try_wait().unwrap().is_none(),
                "服务在发布启动信息前退出"
            );
        }
        let status = service_status(vault).unwrap();
        if status.startup_published()
            && status.pid == Some(pid)
            && status.heartbeat_at.is_some_and(|at| at >= not_before)
        {
            return status;
        }
        assert!(
            Instant::now() < until,
            "服务启动信息未完整发布：leased={} pid_matches={} heartbeat={} hash_bytes={}",
            status.running,
            status.pid == Some(pid),
            status.heartbeat_at.is_some(),
            status.binary_hash.len()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}
struct TestChild(std::process::Child);
impl std::ops::Deref for TestChild {
    type Target = std::process::Child;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for TestChild {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
impl Drop for TestChild {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
        }
        let _ = self.0.wait();
    }
}
#[test]
fn a_running_lease_is_not_a_published_startup_or_a_model_approval() {
    let mut status = ServiceStatus {
        running: true,
        ..Default::default()
    };
    assert!(!status.startup_published());
    status.pid = Some(std::process::id());
    status.started_at = Some(Utc::now());
    status.heartbeat_at = status.started_at;
    assert!(!status.startup_published());
    status.binary_hash = "invalid".into();
    assert!(!status.startup_published());
    status.binary_hash = "a".repeat(64);
    assert!(status.startup_published());
    assert!(!status.credential.present);
    assert!(!status.provider_ready);
}
#[test]
fn service_keeps_discovering_new_evidence_without_a_gui_tick() {
    let root = tempfile::tempdir().unwrap();
    let vault = Vault::init(&root.path().join("vault")).unwrap();
    MemoryRuntime::new(&vault)
        .configure(configuration())
        .unwrap();
    let path = vault.root().to_path_buf();
    let (done, received) = mpsc::channel();
    let launched_at = Utc::now();
    let thread = std::thread::spawn(move || {
        let vault = Vault::open_existing(&path).unwrap();
        run_service(
            &vault,
            &mut Fake {
                completed: Some(done),
            },
            ServiceOptions {
                once: false,
                poll_interval_ms: 10,
            },
        )
        .unwrap()
    });
    wait_started(&vault, std::process::id(), launched_at, None);
    source(&vault);
    received.recv_timeout(Duration::from_secs(5)).unwrap();
    let until = Instant::now() + Duration::from_secs(5);
    while vault.memories().is_err() || vault.memories().unwrap().is_empty() {
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(10));
    }
    request_service_stop(&vault).unwrap();
    let status = thread.join().unwrap();
    assert!(!status.running);
    assert!(!service_status(&vault).unwrap().running);
    assert_eq!(vault.memories().unwrap().len(), 1);
}
#[test]
fn service_stop_during_provider_blocks_late_commit() {
    let root = tempfile::tempdir().unwrap();
    let vault = Vault::init(&root.path().join("vault")).unwrap();
    MemoryRuntime::new(&vault)
        .configure(configuration())
        .unwrap();
    source(&vault);
    struct Stopper<'a>(&'a Vault);
    impl MemoryProvider for Stopper<'_> {
        fn execute(&mut self, _: &MemoryConfig, job: &DreamJob) -> AppResult<ProviderOutput> {
            request_service_stop(self.0)?;
            Ok(output(job))
        }
    }
    let result = run_service(
        &vault,
        &mut Stopper(&vault),
        ServiceOptions {
            once: false,
            poll_interval_ms: 10,
        },
    )
    .unwrap();
    assert!(!result.running);
    assert!(result.stop_requested);
    assert_eq!(result.last_memory_job.unwrap().state, JobState::Paused);
    assert!(vault.memories().unwrap().is_empty());
    let stopped = service_status(&vault).unwrap();
    assert!(stopped.stop_requested);
    assert!(!stopped.running);
    assert!(MemoryRuntime::new(&vault).configuration().unwrap().paused);
}
#[test]
fn stop_waits_for_current_writer_without_overwriting_new_configuration() {
    use std::io::Write;
    let root = tempfile::tempdir().unwrap();
    let vault = Vault::init(&root.path().join("vault")).unwrap();
    MemoryRuntime::new(&vault)
        .configure(configuration())
        .unwrap();
    let directory = vault.state_dir().unwrap().join("background-memory");
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(directory.join("state.lock"))
        .unwrap();
    lock.lock().unwrap();
    let path = vault.root().to_path_buf();
    let (sent, received) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        let vault = Vault::open_existing(&path).unwrap();
        sent.send(request_service_stop(&vault)).unwrap();
    });
    let until = Instant::now() + Duration::from_secs(2);
    while !service_status(&vault).unwrap().stop_requested {
        assert!(Instant::now() < until, "停止请求尚未持久记录");
        std::thread::sleep(Duration::from_millis(10));
    }
    // 先收到停止请求，再等当前本地事务完成；不能因瞬态忙碌直接报错。
    assert!(matches!(
        received.try_recv(),
        Err(mpsc::TryRecvError::Empty)
    ));
    let config_path = directory.join("config.json");
    let mut record: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&config_path).unwrap()).unwrap();
    record["config"]["budget"]["max_calls_per_month"] = 37.into();
    let mut replacement = tempfile::NamedTempFile::new_in(&directory).unwrap();
    replacement
        .write_all(&serde_json::to_vec(&record).unwrap())
        .unwrap();
    replacement.persist(&config_path).unwrap();
    lock.unlock().unwrap();
    received
        .recv_timeout(Duration::from_secs(2))
        .unwrap()
        .unwrap();
    worker.join().unwrap();
    let config = MemoryRuntime::new(&vault).configuration().unwrap();
    assert!(config.paused);
    assert_eq!(config.budget.max_calls_per_month, 37);
}
#[test]
fn one_service_lease_prevents_duplicate_runtime_hosts() {
    let root = tempfile::tempdir().unwrap();
    let vault = Vault::init(&root.path().join("vault")).unwrap();
    let path = vault.root().to_path_buf();
    let thread = std::thread::spawn(move || {
        let vault = Vault::open_existing(&path).unwrap();
        run_service(
            &vault,
            &mut Fake { completed: None },
            ServiceOptions {
                once: false,
                poll_interval_ms: 10,
            },
        )
        .unwrap()
    });
    wait_running(&vault);
    let second = run_service(
        &vault,
        &mut Fake { completed: None },
        ServiceOptions {
            once: true,
            poll_interval_ms: 10,
        },
    )
    .unwrap();
    assert!(second.running);
    request_service_stop(&vault).unwrap();
    thread.join().unwrap();
}
#[test]
fn shared_status_probe_does_not_impersonate_or_abort_a_starting_service() {
    use std::{fs::OpenOptions, process::Stdio};
    let root = tempfile::tempdir().unwrap();
    let vault = Vault::init(&root.path().join("vault")).unwrap();
    assert!(!service_status(&vault).unwrap().running);
    let probe = OpenOptions::new()
        .read(true)
        .write(true)
        .open(vault.state_dir().unwrap().join("runtime/service.lock"))
        .unwrap();
    probe.lock_shared().unwrap();
    // 多个状态探测共享锁；只有真实服务独占持有才表示 running。
    for _ in 0..10 {
        assert!(!service_status(&vault).unwrap().running);
    }
    let launched_at = Utc::now();
    let mut child = TestChild(
        Command::new(env!("CARGO_BIN_EXE_recallcard"))
            .arg("--vault")
            .arg(vault.root())
            .args(["start", "--foreground", "--json"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    std::thread::sleep(Duration::from_millis(100));
    assert!(
        child.try_wait().unwrap().is_none(),
        "启动不能把共享探测误当已有服务而退出"
    );
    assert!(!service_status(&vault).unwrap().running);
    probe.unlock().unwrap();
    let pid = child.id();
    let started = wait_started(&vault, pid, launched_at, Some(&mut child));
    assert_eq!(started.pid, Some(pid));
    request_service_stop(&vault).unwrap();
    let until = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(exit) = child.try_wait().unwrap() {
            assert!(exit.success());
            break;
        }
        assert!(Instant::now() < until, "服务没有到达停稳边界");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(!service_status(&vault).unwrap().running);
}
#[cfg(unix)]
#[test]
fn initializing_owner_waits_for_identity_and_child_exit_has_its_own_error() {
    use recallcard::application::ErrorCode;
    use sha2::{Digest, Sha256};
    use std::{fs::OpenOptions, io::Write, os::unix::fs::PermissionsExt};
    let root = tempfile::tempdir().unwrap();
    let vault = Vault::init(&root.path().join("vault")).unwrap();
    let resources = root.path().join("resources");
    let binary = resources.join("说明与许可证/recallcard");
    std::fs::create_dir_all(binary.parent().unwrap()).unwrap();
    let synthetic_program = b"#!/bin/sh\nexit 7\n";
    std::fs::write(&binary, synthetic_program).unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
    let binary = register_packaged_cli(&binary, &resources).unwrap();
    let expected_hash = format!("{:x}", Sha256::digest(synthetic_program));
    service_status(&vault).unwrap();
    let directory = vault.state_dir().unwrap().join("runtime");
    let lease = OpenOptions::new()
        .read(true)
        .write(true)
        .open(directory.join("service.lock"))
        .unwrap();
    lease.lock().unwrap();
    let mut status = ServiceStatus {
        running: true,
        pid: Some(std::process::id()),
        started_at: Some(Utc::now()),
        heartbeat_at: Some(Utc::now()),
        ..Default::default()
    };
    let publish = |status: &ServiceStatus| {
        let mut file = tempfile::NamedTempFile::new_in(&directory).unwrap();
        file.write_all(&serde_json::to_vec(status).unwrap())
            .unwrap();
        file.persist(directory.join("status.json")).unwrap();
    };
    publish(&status);
    let path = vault.root().to_path_buf();
    let candidate = binary.clone();
    let (entered, waiting) = mpsc::channel();
    let (sent, received) = mpsc::channel();
    let starter = std::thread::spawn(move || {
        let vault = Vault::open_existing(&path).unwrap();
        entered.send(()).unwrap();
        sent.send(ensure_service(&vault, &candidate)).unwrap();
    });
    waiting.recv_timeout(Duration::from_secs(2)).unwrap();
    assert!(matches!(
        received.recv_timeout(Duration::from_millis(50)),
        Err(mpsc::RecvTimeoutError::Timeout)
    ));
    status.binary_hash = expected_hash.clone();
    publish(&status);
    let reused = received
        .recv_timeout(Duration::from_secs(2))
        .unwrap()
        .unwrap();
    starter.join().unwrap();
    assert_eq!(reused.binary_hash, expected_hash);
    assert_eq!(reused.pid, status.pid);
    // 真正的独占持有者公布不同程序时仍必须拒绝；不能抢占或偷偷替换。
    status.binary_hash = "0".repeat(64);
    publish(&status);
    let mismatch = ensure_service(&vault, &binary).unwrap_err();
    assert_eq!(mismatch.code, ErrorCode::Conflict);
    assert!(mismatch.message.contains("当前发行物不同"));
    let credential = recallcard::application::credentials::PreparedCredential::session(
        configuration().provider.unwrap(),
        "SYNTHETIC_DIFFERENT_BINARY_KEY".into(),
    )
    .unwrap();
    let mismatch = ensure_service_with_credential(&vault, &binary, credential).unwrap_err();
    assert_eq!(mismatch.code, ErrorCode::Conflict);
    assert!(!service_status(&vault).unwrap().stop_requested);
    assert!(service_status(&vault).unwrap().running);
    lease.unlock().unwrap();
    // 路径已经核验，进程确实启动后退出时应给生命周期错误，不能说找不到程序。
    let exited = ensure_service(&vault, &binary).unwrap_err();
    assert_eq!(exited.code, ErrorCode::ModelUnavailable);
    assert!(exited.message.contains("启动握手前退出"));
    assert!(exited.message.contains('7'));
    assert!(!exited.message.contains("未找到"));
}
#[cfg(unix)]
#[test]
fn another_owner_cannot_confirm_this_childs_session_credential_handoff() {
    use recallcard::application::{credentials::PreparedCredential, ErrorCode};
    use sha2::{Digest, Sha256};
    use std::{fs::OpenOptions, io::Write, os::unix::fs::PermissionsExt};
    let root = tempfile::tempdir().unwrap();
    let vault = Vault::init(&root.path().join("vault")).unwrap();
    let resources = root.path().join("resources");
    let binary = resources.join("说明与许可证/recallcard");
    let spawned = root.path().join("spawned");
    let finish = root.path().join("finish");
    std::fs::create_dir_all(binary.parent().unwrap()).unwrap();
    // 合成程序仅通知进程已启动并等待结束；不读取、输出或保存 stdin 凭据。
    let program = format!(
        "#!/bin/sh\n: > '{}'\nremaining=500\nwhile [ ! -f '{}' ] && [ \"$remaining\" -gt 0 ]; do remaining=$((remaining - 1)); sleep 0.01; done\nexit 7\n",
        spawned.display(),
        finish.display()
    );
    std::fs::write(&binary, &program).unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
    let binary = register_packaged_cli(&binary, &resources).unwrap();
    let expected_hash = format!("{:x}", Sha256::digest(program.as_bytes()));
    service_status(&vault).unwrap();
    let path = vault.root().to_path_buf();
    let (sent, received) = mpsc::channel();
    let starter = std::thread::spawn(move || {
        let vault = Vault::open_existing(&path).unwrap();
        let credential = PreparedCredential::session(
            configuration().provider.unwrap(),
            "SYNTHETIC_CONCURRENT_KEY_NOT_SENT".into(),
        )
        .unwrap();
        sent.send(ensure_service_with_credential(&vault, &binary, credential))
            .unwrap();
    });
    let until = Instant::now() + Duration::from_secs(2);
    while !spawned.exists() {
        assert!(Instant::now() < until, "合成子进程没有启动");
        std::thread::sleep(Duration::from_millis(10));
    }
    // 另一个同版本所有者抢先发布新鲜状态，不能替本次 child 确认密钥交接。
    let directory = vault.state_dir().unwrap().join("runtime");
    let lease = OpenOptions::new()
        .read(true)
        .write(true)
        .open(directory.join("service.lock"))
        .unwrap();
    lease.lock().unwrap();
    let status = ServiceStatus {
        running: true,
        pid: Some(std::process::id()),
        started_at: Some(Utc::now()),
        heartbeat_at: Some(Utc::now()),
        binary_hash: expected_hash,
        ..Default::default()
    };
    let mut file = tempfile::NamedTempFile::new_in(&directory).unwrap();
    file.write_all(&serde_json::to_vec(&status).unwrap())
        .unwrap();
    file.persist(directory.join("status.json")).unwrap();
    std::fs::write(&finish, b"done").unwrap();
    let result = received.recv_timeout(Duration::from_secs(2)).unwrap();
    starter.join().unwrap();
    lease.unlock().unwrap();
    let error = result.unwrap_err();
    assert_eq!(error.code, ErrorCode::ModelUnavailable);
    assert!(error.message.contains("启动握手前退出"));
    assert!(!serde_json::to_string(&error)
        .unwrap()
        .contains("SYNTHETIC_CONCURRENT_KEY"));
}
fn cli(vault: &Vault, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_recallcard"))
        .arg("--vault")
        .arg(vault.root())
        .args(args)
        .output()
        .unwrap()
}
fn cli_ok(vault: &Vault, args: &[&str]) -> serde_json::Value {
    let out = cli(vault, args);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}
#[test]
fn real_cli_detached_service_survives_launcher_and_stops_with_verified_lease() {
    let root = tempfile::tempdir().unwrap();
    let vault = Vault::init(&root.path().join("vault")).unwrap();
    let started = cli_ok(&vault, &["start", "--json"]);
    assert_eq!(started["result"]["running"], true);
    let status = cli_ok(&vault, &["service", "status", "--json"]);
    assert_eq!(status["result"]["running"], true);
    cli_ok(&vault, &["service", "stop", "--json"]);
    let until = Instant::now() + Duration::from_secs(5);
    while service_status(&vault).unwrap().running {
        assert!(Instant::now() < until, "停止没有生效");
        std::thread::sleep(Duration::from_millis(30));
    }
}
#[test]
fn positional_official_import_and_jobs_share_typed_cli_contract() {
    let root = tempfile::tempdir().unwrap();
    let vault = Vault::init(&root.path().join("vault")).unwrap();
    let path = root.path().join("official.json");
    std::fs::write(&path,serde_json::to_vec(&json!({"conversation_id":"synthetic","current_node":"u","title":"合成","mapping":{"root":{"id":"root","parent":null,"children":["u"],"message":null},"u":{"id":"u","parent":"root","children":[],"message":{"id":"u","author":{"role":"user"},"create_time":null,"content":{"content_type":"text","parts":["合成来源无需整理即可搜索"]}}}}})).unwrap()).unwrap();
    let imported = cli_ok(&vault, &["import", path.to_str().unwrap(), "--json"]);
    assert_eq!(imported["schema"], "recallcard.cli/1");
    assert_eq!(imported["result"]["job"]["state"], "completed");
    assert_eq!(imported["result"]["job"]["progress"]["events_added"], 1);
    let id = imported["result"]["job"]["job_id"].as_str().unwrap();
    let same = cli_ok(&vault, &["jobs", "status", id, "--json"]);
    assert_eq!(same["result"]["job_id"], id);
    let repeated = cli_ok(&vault, &["import", path.to_str().unwrap(), "--json"]);
    assert_eq!(repeated["result"]["job"]["progress"]["events_added"], 0);
    assert_eq!(
        repeated["result"]["job"]["progress"]["events_duplicates"],
        1
    );
    let wrong = cli(
        &vault,
        &["jobs", "status", id, "--scope", "project:other", "--json"],
    );
    assert_eq!(wrong.status.code(), Some(5));
    let error: serde_json::Value = serde_json::from_slice(&wrong.stderr).unwrap();
    assert_eq!(error["error"]["code"], "permission_denied");
    assert!(wrong.stdout.is_empty());
}

#[test]
fn real_cli_session_credential_pipe_is_not_persisted_and_restart_requests_input() {
    use recallcard::application::credentials::{
        write_handoff, CredentialStorage, PreparedCredential,
    };
    use std::process::Stdio;
    let root = tempfile::tempdir().unwrap();
    let vault = Vault::init(&root.path().join("vault")).unwrap();
    let mut config = configuration();
    config.credential_storage = Some(CredentialStorage::SessionOnly);
    MemoryRuntime::new(&vault)
        .configure(config.clone())
        .unwrap();
    let launched_at = Utc::now();
    let mut child = TestChild(
        Command::new(env!("CARGO_BIN_EXE_recallcard"))
            .arg("--vault")
            .arg(vault.root())
            .args(["start", "--foreground", "--credential-stdin", "--json"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .env_remove("RECALLCARD_DREAM_API_KEY")
            .spawn()
            .unwrap(),
    );
    let secret = "SYNTHETIC_PIPE_KEY_NEVER_SENT";
    let credential =
        PreparedCredential::session(config.provider.clone().unwrap(), secret.into()).unwrap();
    write_handoff(&mut child.stdin.take().unwrap(), &credential).unwrap();
    let pid = child.id();
    let running = wait_started(&vault, pid, launched_at, Some(&mut child));
    assert!(
        running.credential.present,
        "启动已发布但会话凭据没有接收成功"
    );
    assert_eq!(running.credential.storage, CredentialStorage::SessionOnly);
    assert_eq!(running.credential.lifetime, "background_service_exit");
    assert_eq!(running.build["version"], env!("CARGO_PKG_VERSION"));
    request_service_stop(&vault).unwrap();
    let until = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(20));
    }
    fn no_secret(path: &std::path::Path, secret: &str) {
        for entry in std::fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                no_secret(&path, secret);
            } else {
                assert!(!String::from_utf8_lossy(&std::fs::read(path).unwrap()).contains(secret));
            }
        }
    }
    no_secret(&vault.state_dir().unwrap(), secret);
    no_secret(vault.root(), secret);
    config.paused = false;
    MemoryRuntime::new(&vault).configure(config).unwrap();
    source(&vault);
    let restarted = cli_ok(&vault, &["start", "--once", "--json"]);
    assert_eq!(restarted["result"]["credential"]["present"], false);
    assert_eq!(
        restarted["result"]["last_memory_job"]["state"],
        "needs_input"
    );
    cli_ok(&vault, &["start", "--once", "--json"]);
    assert_eq!(MemoryRuntime::new(&vault).jobs().unwrap().len(), 1);
    assert_eq!(
        MemoryRuntime::new(&vault)
            .status()
            .unwrap()
            .usage
            .reserved_calls,
        0
    );
    assert_eq!(vault.events().unwrap().len(), 1);
}

#[test]
fn setup_help_and_status_expose_normal_flow_with_legacy_git_opt_in() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("new-vault");
    let output = Command::new(env!("CARGO_BIN_EXE_recallcard"))
        .arg("--vault")
        .arg(&path)
        .args(["setup", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        value["result"]["build"]["version"],
        env!("CARGO_PKG_VERSION")
    );
    let vault = Vault::open_existing(&path).unwrap();
    let status = cli_ok(&vault, &["status", "--json"]);
    assert_eq!(status["result"]["service"]["running"], false);
    assert_eq!(status["result"]["background"]["state"], "unconfigured");
    let git = cli(&vault, &["status", "--git"]);
    assert!(git.status.success());
    assert!(serde_json::from_slice::<serde_json::Value>(&git.stdout).is_ok());
    let help = Command::new(env!("CARGO_BIN_EXE_recallcard"))
        .arg("--help")
        .output()
        .unwrap();
    let text = String::from_utf8(help.stdout).unwrap();
    assert!(text.contains("setup"));
    assert!(text.contains("import"));
    assert!(!text.contains("dream"));
    assert!(!text.contains("native-host"));
    assert!(Command::new(env!("CARGO_BIN_EXE_recallcard"))
        .args(["dream", "--help"])
        .output()
        .unwrap()
        .status
        .success());
}
