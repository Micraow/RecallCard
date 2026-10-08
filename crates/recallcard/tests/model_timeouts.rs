//! 超时配置与真实受监督Python进程；仅本地合成输入，绝不调用模型网络。
use recallcard::{
    application::{background_memory::*, credentials::PreparedCredential, ErrorCode},
    Vault,
};
use serde_json::json;
use std::{
    fs,
    time::{Duration, Instant},
};
#[test]
fn legacy_defaults_and_invalid_wait_limits_are_explicit() {
    let legacy = serde_json::to_value(MemoryConfig::default()).unwrap();
    assert!(legacy.get("timeouts").is_none());
    let parsed: MemoryConfig = serde_json::from_value(legacy.clone()).unwrap();
    assert_eq!(parsed.timeouts, MemoryTimeouts::default());
    assert_eq!(serde_json::to_value(parsed).unwrap(), legacy);
    for t in [
        MemoryTimeouts {
            connect_seconds: 121,
            read_seconds: 30,
            operation_seconds: 600,
        },
        MemoryTimeouts {
            connect_seconds: 1,
            read_seconds: 301,
            operation_seconds: 300,
        },
        MemoryTimeouts {
            connect_seconds: 1,
            read_seconds: 1,
            operation_seconds: 1801,
        },
    ] {
        assert_eq!(t.validate().unwrap_err().code, ErrorCode::InvalidRequest);
    }
}
#[test]
fn adjusting_only_timeouts_preserves_destination_permissions_and_budget() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(&d.path().join("vault")).unwrap();
    let runtime = MemoryRuntime::new(&v);
    let mut original = MemoryConfig::default();
    original.budget.max_calls_per_month = 17;
    original.scope = "project:synthetic".into();
    runtime.configure(original.clone()).unwrap();
    let t = MemoryTimeouts {
        connect_seconds: 10,
        read_seconds: 300,
        operation_seconds: 600,
    };
    let status = runtime.set_timeouts(t.clone()).unwrap();
    original.timeouts = t;
    assert_eq!(status.config, original);
    assert!(!status.config.enabled);
    assert!(status.config.consent.is_none());
    let reopened = Vault::open(v.root()).unwrap();
    assert_eq!(
        MemoryRuntime::new(&reopened).configuration().unwrap(),
        original
    );
}
#[test]
fn cancellation_and_supervisor_timeout_reap_the_real_worker_without_a_late_result() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(&d.path().join("vault")).unwrap();
    let e=v.capture(serde_json::from_value(json!({"role":"user","origin":"native","content":"合成超时样本","source":{"platform":"synthetic","conversation_id":"wait","message_id":"one"}})).unwrap()).unwrap();
    let job = v.dream_export(&[e.id], &[], "personal").unwrap();
    let target = MemoryProviderConfig {
        endpoint: "https://example.invalid/api".into(),
        model: "synthetic".into(),
    };
    let config = MemoryConfig {
        enabled: true,
        provider: Some(target.clone()),
        consent: Some(MemoryConsent {
            endpoint: target.endpoint.clone(),
            model: target.model.clone(),
            scope: "personal".into(),
            send_source_snapshots: true,
            send_memory_snapshots: true,
            auto_apply: true,
            accepted_at: chrono::Utc::now(),
        }),
        timeouts: MemoryTimeouts {
            connect_seconds: 10,
            read_seconds: 300,
            operation_seconds: 600,
        },
        ..Default::default()
    };
    for cancel in [true, false] {
        let package = d.path().join(if cancel { "cancel" } else { "timeout" });
        let module = package.join("recallcard_dream");
        fs::create_dir_all(&module).unwrap();
        fs::write(module.join("__init__.py"), "").unwrap();
        fs::write(module.join("runtime.py"),"import sys,os,time,json,pathlib\ndef main(argv=None):\n    r=json.loads(sys.stdin.buffer.read())\n    assert r['config']['timeouts']['operation_seconds']==600\n    pathlib.Path(__file__).with_name('pid').write_text(str(os.getpid()))\n    time.sleep(60)\n    pathlib.Path(__file__).with_name('late').write_text('should not survive')\n").unwrap();
        let mut provider = PythonMemoryProvider::new("python3".into(), package)
            .without_environment()
            .with_credential(
                PreparedCredential::session(target.clone(), "SYNTHETIC_NO_NETWORK".into()).unwrap(),
            );
        provider.timeout_seconds = if cancel { 10 } else { 1 };
        let began = Instant::now();
        let error = provider
            .execute_cancellable(&config, &job, &|| Ok(cancel && module.join("pid").exists()))
            .unwrap_err();
        assert_eq!(
            error.code,
            if cancel {
                ErrorCode::Cancelled
            } else {
                ErrorCode::ModelUnavailable
            }
        );
        assert!(began.elapsed() < Duration::from_secs(4));
        assert!(!module.join("late").exists());
        #[cfg(target_os = "linux")]
        {
            let pid = fs::read_to_string(module.join("pid")).unwrap();
            assert!(
                !std::path::Path::new("/proc").join(pid).exists(),
                "子进程必须被等待回收，不留僵尸"
            );
        }
    }
}
