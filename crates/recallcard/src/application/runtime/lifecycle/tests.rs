//! 原创合成状态与显式 channel 屏障；不靠调度速度复现竞态，不调用模型。
use super::*;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    mpsc, Arc,
};
use std::thread::{self, JoinHandle};

const DEADLINE: Duration = Duration::from_secs(4);
fn fixture() -> (tempfile::TempDir, Vault) {
    let dir = tempfile::tempdir().unwrap();
    let vault = Vault::init(&dir.path().join("vault")).unwrap();
    (dir, vault)
}
fn joined<T>(handle: JoinHandle<T>) -> T {
    let until = Instant::now() + DEADLINE;
    while !handle.is_finished() {
        assert!(Instant::now() < until, "合成线程没有在有界预算内退出");
        thread::sleep(Duration::from_millis(1));
    }
    handle.join().unwrap_or_else(|_| panic!("合成线程失败"))
}
fn owned(vault: &Vault) -> (ServiceLease, ServiceStatus) {
    match begin(vault).unwrap() {
        Start::Owned(lease, status) => (lease, *status),
        Start::Existing(_) => panic!("未取得预期的服务租约"),
    }
}
fn current(vault: &Vault) -> StopRequest {
    match read_stop(vault).unwrap().unwrap() {
        StopRecord::Current(request) => request,
        StopRecord::Legacy => panic!("停止请求尚未升级"),
    }
}

#[test]
fn old_cleanup_order_negative_control_deterministically_loses_the_stop() {
    let (_dir, vault) = fixture();
    let path = vault.root().to_path_buf();
    let (visible, published) = mpsc::channel();
    let (resume, stopped) = mpsc::channel();
    let old_start = thread::spawn(move || {
        let vault = Vault::open_existing(&path).unwrap();
        let _lease = try_lease(&directory(&vault).unwrap().join("service.lock"), false)
            .unwrap()
            .unwrap();
        save_status(
            &vault,
            &ServiceStatus {
                running: true,
                ..Default::default()
            },
        )
        .unwrap();
        visible.send(()).unwrap();
        stopped.recv_timeout(DEADLINE).unwrap();
        // 负对照精确重现旧实现：租约已可见之后，无条件清理 stop。
        fs::remove_file(directory(&vault).unwrap().join("stop.json")).unwrap();
        current_stop_requested(&vault).unwrap()
    });
    published.recv_timeout(DEADLINE).unwrap();
    request_service_stop(&vault).unwrap();
    assert!(current_stop_requested(&vault).unwrap());
    resume.send(()).unwrap();
    assert!(!joined(old_start), "负对照必须捕获旧顺序丢失停止请求");
}

#[test]
fn stop_during_lease_to_publication_window_is_bound_to_the_new_generation() {
    let (_dir, vault) = fixture();
    request_service_stop(&vault).unwrap(); // 已完成的旧停止，需要由新启动清理。
    let path = vault.root().to_path_buf();
    let (entered, window) = mpsc::channel();
    let (resume, publish) = mpsc::channel();
    let (started, ready) = mpsc::channel();
    let (finish, finish_rx) = mpsc::channel();
    let owner = thread::spawn(move || {
        let vault = Vault::open_existing(&path).unwrap();
        let result = begin_with(&vault, || {
            entered.send(()).unwrap();
            publish.recv_timeout(DEADLINE).unwrap();
        })
        .unwrap();
        let Start::Owned(_lease, status) = result else {
            panic!("意外的第二实例");
        };
        started.send(status.generation.clone()).unwrap();
        finish_rx.recv_timeout(DEADLINE).unwrap();
        for_generation(&vault, status.generation.as_deref()).unwrap()
    });
    window.recv_timeout(DEADLINE).unwrap();
    assert!(
        try_lease(&directory(&vault).unwrap().join("lifecycle.lock"), false)
            .unwrap()
            .is_none()
    );
    let path = vault.root().to_path_buf();
    let (attempt, attempted) = mpsc::channel();
    let stopper = thread::spawn(move || {
        let vault = Vault::open_existing(&path).unwrap();
        attempt.send(()).unwrap();
        request_service_stop(&vault).unwrap();
    });
    attempted.recv_timeout(DEADLINE).unwrap();
    resume.send(()).unwrap();
    let generation = ready.recv_timeout(DEADLINE).unwrap();
    joined(stopper);
    let request = current(&vault);
    assert_eq!(request.generation, generation);
    assert_eq!(request.phase, Phase::Applied);
    finish.send(()).unwrap();
    assert!(joined(owner));
}

#[test]
fn initializing_owner_and_second_instance_never_erase_a_live_stop() {
    let (_dir, vault) = fixture();
    let (_lease, status) = owned(&vault);
    assert!(!status.startup_published());
    request_service_stop(&vault).unwrap();
    let before = fs::read(directory(&vault).unwrap().join("stop.json")).unwrap();
    assert!(matches!(begin(&vault).unwrap(), Start::Existing(s) if s.running && s.stop_requested));
    assert_eq!(
        fs::read(directory(&vault).unwrap().join("stop.json")).unwrap(),
        before
    );
    assert!(for_generation(&vault, status.generation.as_deref()).unwrap());
}

#[test]
fn pending_pause_keeps_status_readable_and_blocks_a_new_owner_until_applied() {
    let (_dir, vault) = fixture();
    let (lease, status) = owned(&vault);
    let memory = MemoryRuntime::new(&vault);
    memory.configure(Default::default()).unwrap();
    let state = OpenOptions::new()
        .read(true)
        .write(true)
        .open(
            vault
                .state_dir()
                .unwrap()
                .join("background-memory/state.lock"),
        )
        .unwrap();
    state.lock().unwrap();
    let path = vault.root().to_path_buf();
    let (persisted, pending) = mpsc::channel();
    let stopper = thread::spawn(move || {
        let vault = Vault::open_existing(&path).unwrap();
        stop_with(&vault, || {
            persisted.send(()).unwrap();
            MemoryRuntime::new(&vault).pause()?;
            ImportService::new(&vault)?.pause_all_pending()?;
            Ok(())
        })
        .unwrap();
    });
    pending.recv_timeout(DEADLINE).unwrap();
    assert!(service_status(&vault).unwrap().stop_requested);
    assert_eq!(current(&vault).phase, Phase::Pending);
    drop(lease); // 服务已退出也不能让尚未完成的慢暂停跨到下一代。
    assert_eq!(begin(&vault).err().unwrap().code, ErrorCode::Conflict);
    assert_eq!(read_status(&vault).unwrap().generation, status.generation);
    state.unlock().unwrap();
    joined(stopper);
    assert_eq!(current(&vault).phase, Phase::Applied);
    let (_next_lease, next) = owned(&vault);
    assert_ne!(next.generation, status.generation);
    assert!(!for_generation(&vault, next.generation.as_deref()).unwrap());
}

#[test]
fn concurrent_stops_cannot_overlap_side_effects_or_publish_applied_early() {
    let (_dir, vault) = fixture();
    let (lease, _) = owned(&vault);
    let active = Arc::new(AtomicUsize::new(0));
    let path = vault.root().to_path_buf();
    let first_active = active.clone();
    let (inside, entered) = mpsc::channel();
    let (release, released) = mpsc::channel();
    let first = thread::spawn(move || {
        let vault = Vault::open_existing(&path).unwrap();
        stop_with(&vault, || {
            assert_eq!(first_active.fetch_add(1, Ordering::SeqCst), 0);
            inside.send(()).unwrap();
            released.recv_timeout(DEADLINE).unwrap();
            MemoryRuntime::new(&vault).pause()?;
            first_active.fetch_sub(1, Ordering::SeqCst);
            Ok(())
        })
        .unwrap();
    });
    entered.recv_timeout(DEADLINE).unwrap();
    assert!(
        try_lease(&directory(&vault).unwrap().join("stop-apply.lock"), false)
            .unwrap()
            .is_none()
    );
    let path = vault.root().to_path_buf();
    let second_active = active.clone();
    let (attempt, attempted) = mpsc::channel();
    let second = thread::spawn(move || {
        let vault = Vault::open_existing(&path).unwrap();
        attempt.send(()).unwrap();
        stop_with(&vault, || {
            assert_eq!(second_active.fetch_add(1, Ordering::SeqCst), 0);
            MemoryRuntime::new(&vault).pause()?;
            second_active.fetch_sub(1, Ordering::SeqCst);
            Ok(())
        })
        .unwrap();
    });
    attempted.recv_timeout(DEADLINE).unwrap();
    assert_eq!(current(&vault).phase, Phase::Pending);
    drop(lease);
    assert!(begin(&vault).is_err());
    release.send(()).unwrap();
    joined(first);
    joined(second);
    assert_eq!(active.load(Ordering::SeqCst), 0);
    let (_lease, _) = owned(&vault);
}

#[test]
fn failed_pause_is_durable_pending_and_explicit_retry_finishes_the_same_request() {
    let (_dir, vault) = fixture();
    assert!(stop_with(&vault, || Err(storage())).is_err());
    let failed = current(&vault);
    assert_eq!(failed.phase, Phase::Pending);
    assert!(begin(&vault).is_err());
    request_service_stop(&vault).unwrap();
    let completed = current(&vault);
    assert_eq!(failed.request_id, completed.request_id);
    assert_eq!(completed.phase, Phase::Applied);
    let (_lease, _) = owned(&vault);
}

#[test]
fn stale_generation_and_stale_completion_cannot_stop_or_acknowledge_a_new_owner() {
    let (_dir, vault) = fixture();
    let (lease, old) = owned(&vault);
    request_service_stop(&vault).unwrap();
    let mut stale = current(&vault);
    drop(lease);
    let (_lease, new) = owned(&vault);
    assert_ne!(old.generation, new.generation);
    save_stop(&vault, &stale).unwrap(); // 合成重放旧的已完成记录。
    assert!(!for_generation(&vault, new.generation.as_deref()).unwrap());
    assert!(!current_stop_requested(&vault).unwrap());
    request_service_stop(&vault).unwrap();
    let current_request = current(&vault);
    assert_eq!(current_request.generation, new.generation);
    stale.phase = Phase::Pending;
    assert!(finish_stop(&vault, &stale).is_err());
    assert_eq!(current(&vault), current_request);
}

#[test]
fn legacy_stop_requires_recovery_and_bad_markers_fail_closed() {
    let (_dir, vault) = fixture();
    let path = directory(&vault).unwrap().join("stop.json");
    vault
        .write_replace(&path, &serde_json::json!({"requested_at": Utc::now()}))
        .unwrap();
    assert!(current_stop_requested(&vault).unwrap());
    assert!(begin(&vault).is_err());
    request_service_stop(&vault).unwrap();
    assert_eq!(current(&vault).phase, Phase::Applied);
    let (lease, _) = owned(&vault);
    drop(lease);
    for bytes in [b"{}".to_vec(), vec![b'x'; 4097], b"not json".to_vec()] {
        fs::write(&path, bytes).unwrap();
        assert!(begin(&vault).is_err());
        assert!(current_stop_requested(&vault).is_err());
        assert!(request_service_stop(&vault).is_err());
    }
}

#[cfg(unix)]
#[test]
fn stop_marker_symlink_is_not_followed_or_replaced() {
    let (dir, vault) = fixture();
    let outside = dir.path().join("outside");
    fs::write(&outside, b"SYNTHETIC_OUTSIDE").unwrap();
    let marker = directory(&vault).unwrap().join("stop.json");
    std::os::unix::fs::symlink(&outside, marker).unwrap();
    assert!(begin(&vault).is_err());
    assert!(request_service_stop(&vault).is_err());
    assert!(service_status(&vault).is_err());
    assert_eq!(fs::read(outside).unwrap(), b"SYNTHETIC_OUTSIDE");
}

#[test]
fn delayed_stop_call_for_old_generation_never_pauses_the_replacement() {
    let (_dir, vault) = fixture();
    let (lease, old) = owned(&vault);
    let path = vault.root().to_path_buf();
    let (bound, bound_rx) = mpsc::channel();
    let (resume, resumed) = mpsc::channel();
    let delayed = thread::spawn(move || {
        let vault = Vault::open_existing(&path).unwrap();
        let target = {
            let _guard = lock(&vault).unwrap();
            read_status(&vault).unwrap().generation
        };
        bound.send(target.clone()).unwrap();
        resumed.recv_timeout(DEADLINE).unwrap();
        stop_for_target(&vault, target.as_deref(), || {
            panic!("旧代次停止不应执行新代次的暂停副作用")
        })
    });
    assert_eq!(bound_rx.recv_timeout(DEADLINE).unwrap(), old.generation);
    request_service_stop(&vault).unwrap();
    drop(lease);
    let (_lease, new) = owned(&vault);
    assert_ne!(old.generation, new.generation);
    MemoryRuntime::new(&vault)
        .configure(Default::default())
        .unwrap();
    resume.send(()).unwrap();
    assert_eq!(joined(delayed).unwrap_err().code, ErrorCode::Conflict);
    assert!(!MemoryRuntime::new(&vault).configuration().unwrap().paused);
    assert!(read_stop(&vault).unwrap().is_none());
}

#[test]
fn failed_generation_save_keeps_the_previous_applied_stop() {
    let (_dir, vault) = fixture();
    request_service_stop(&vault).unwrap();
    let previous = current(&vault);
    fs::create_dir(directory(&vault).unwrap().join("status.json")).unwrap();
    assert!(begin(&vault).is_err());
    assert_eq!(current(&vault), previous);
}

#[cfg(unix)]
#[test]
fn lifecycle_state_rejects_fifo_and_directory_without_waiting_for_a_writer() {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let (_dir, vault) = fixture();
    for name in ["stop.json", "status.json"] {
        let path = directory(&vault).unwrap().join(name);
        let cpath = CString::new(path.as_os_str().as_bytes()).unwrap();
        // 合成 FIFO 从不附加 writer；阻塞 open 的负实现将无法通过有界完成检查。
        assert_eq!(unsafe { libc::mkfifo(cpath.as_ptr(), 0o600) }, 0);
        let root = vault.root().to_path_buf();
        let reader = thread::spawn(move || {
            let vault = Vault::open_existing(&root).unwrap();
            assert!(service_status(&vault).is_err());
            assert!(request_service_stop(&vault).is_err());
        });
        joined(reader);
        fs::remove_file(&path).unwrap();
        fs::create_dir(&path).unwrap();
        assert!(service_status(&vault).is_err());
        assert!(request_service_stop(&vault).is_err());
        fs::remove_dir(&path).unwrap();
    }
}
