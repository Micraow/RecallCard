//! 真正的 Rust→Python 子进程/管道边界；fixture 不导入网络模块，也不发请求。
use chrono::Utc;
use recallcard::{
    application::{background_memory::*, ErrorCode},
    Vault,
};
use serde_json::json;
use std::{
    fs,
    time::{Duration, Instant},
};

#[test]
fn subprocess_eof_success_errors_stderr_limits_and_timeout_are_supervised() {
    let root = tempfile::tempdir().unwrap();
    let vault = Vault::init(&root.path().join("vault")).unwrap();
    let event=vault.capture(serde_json::from_value(json!({"role":"user","origin":"native","content":"完全合成的 stdio 资料","source":{"platform":"synthetic","conversation_id":"one","message_id":"one"}})).unwrap()).unwrap();
    let job = vault.dream_export(&[event.id], &[], "personal").unwrap();
    let config = MemoryConfig {
        enabled: true,
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
    };
    let previous = std::env::var_os("RECALLCARD_DREAM_API_KEY");
    std::env::set_var("RECALLCARD_DREAM_API_KEY", "synthetic-key-no-network");
    let cases=[
        ("success", "job=request['job']; print(json.dumps({'ok':True,'result':{'schema':'recallcard.dream-result/1','job_id':job['job_id'],'input_hash':job['input_hash'],'proposals':[{'operation':'noop','scope':job['allowed_scope']}]},'usage':{'input_tokens':None,'output_tokens':None,'request_bytes':1,'network_calls':0},'verification':'provider_reported'}))",None),
        ("empty", "pass",Some(ErrorCode::InvalidRequest)),
        ("malformed", "sys.stderr.write('PRIVATE_PROVIDER_BODY synthetic-key-no-network'); print('not json')",Some(ErrorCode::InvalidRequest)),
        ("stderr_success", "sys.stderr.write('PRIVATE_PROVIDER_BODY'); job=request['job']; print(json.dumps({'ok':True,'result':{'schema':'recallcard.dream-result/1','job_id':job['job_id'],'input_hash':job['input_hash'],'proposals':[{'operation':'noop','scope':job['allowed_scope']}]},'usage':{'input_tokens':0,'output_tokens':0,'request_bytes':0,'network_calls':0},'verification':'provider_reported'}))",None),
        ("oversize", "print('x'*(2*1024*1024+10))",Some(ErrorCode::InvalidRequest)),
        ("timeout", "import time; time.sleep(5)",Some(ErrorCode::ModelUnavailable)),
        ("exit_error", "print(json.dumps({'ok':False,'error':{'code':'provider_error','message':'PRIVATE_PROVIDER_BODY synthetic-key-no-network'}})); return 2",Some(ErrorCode::ModelUnavailable)),
    ];
    for (name, body, expected) in cases {
        let package = root.path().join(name);
        let module = package.join("recallcard_dream");
        fs::create_dir_all(&module).unwrap();
        fs::write(module.join("__init__.py"), "").unwrap();
        // read() 故意等 EOF；若 Rust 只 shutdown 而不 drop stdin，此测试会超时。
        let script=format!("import json,sys\ndef main(argv=None):\n    request=json.loads(sys.stdin.buffer.read())\n    {body}\n    return 0\n");
        fs::write(module.join("runtime.py"), script).unwrap();
        let mut provider = PythonMemoryProvider::new("python3".into(), package);
        provider.timeout_seconds = 1;
        let started = Instant::now();
        let result = provider.execute(&config, &job);
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "{name} 子进程没有有界终止"
        );
        if let Some(expected) = expected {
            let error = result.unwrap_err();
            assert_eq!(error.code, expected, "{name}");
            let encoded = serde_json::to_string(&error).unwrap();
            assert!(!encoded.contains("PRIVATE_PROVIDER_BODY"));
            assert!(!encoded.contains("synthetic-key-no-network"));
        } else {
            let result = result.unwrap();
            assert_eq!(result.result.job_id, job.job_id);
            assert_eq!(result.usage.network_calls, 0, "fixture 从未调用网络");
            assert!(
                started.elapsed() < Duration::from_secs(1),
                "{name} 应在 EOF 后立即完成"
            );
        }
    }
    if let Some(previous) = previous {
        std::env::set_var("RECALLCARD_DREAM_API_KEY", previous);
    } else {
        std::env::remove_var("RECALLCARD_DREAM_API_KEY");
    }
}
