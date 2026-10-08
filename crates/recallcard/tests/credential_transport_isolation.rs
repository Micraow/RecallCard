//! 并发子进程凭据隔离：只有合成值，无模型网络请求，也不修改父进程环境。
use chrono::Utc;
use recallcard::{
    application::{background_memory::*, credentials::*},
    Vault,
};
use serde_json::json;
#[test]
fn parallel_provider_keys_are_child_local_and_parent_environment_is_unchanged() {
    let root = tempfile::tempdir().unwrap();
    let vault = Vault::init(&root.path().join("vault")).unwrap();
    let event=vault.capture(serde_json::from_value(json!({"role":"user","origin":"native","content":"完全合成的子进程隔离资料","source":{"platform":"synthetic","conversation_id":"isolation","message_id":"one"}})).unwrap()).unwrap();
    let job = vault.dream_export(&[event.id], &[], "personal").unwrap();
    let package = root.path().join("python");
    let module = package.join("recallcard_dream");
    std::fs::create_dir_all(&module).unwrap();
    std::fs::write(module.join("__init__.py"), "").unwrap();
    std::fs::write(module.join("runtime.py"),r#"import os,sys,json,time
def main(argv=None):
    request=json.loads(sys.stdin.buffer.read())
    assert os.environ.get('RECALLCARD_DREAM_API_KEY')=='SYNTHETIC_'+request['config']['provider']['model']
    time.sleep(0.05)
    job=request['job']
    print(json.dumps({'ok':True,'result':{'schema':'recallcard.dream-result/1','job_id':job['job_id'],'input_hash':job['input_hash'],'proposals':[{'operation':'noop','scope':job['allowed_scope']}]},'usage':{'input_tokens':0,'output_tokens':0,'request_bytes':0,'network_calls':0},'verification':'provider_reported'}))
    return 0
"#).unwrap();
    let before = std::env::var_os("RECALLCARD_DREAM_API_KEY");
    let threads: [_; 2] = ["PROVIDER_A", "PROVIDER_B"].map(|model| {
        let package = package.clone();
        let job = job.clone();
        std::thread::spawn(move || {
            let target = MemoryProviderConfig {
                endpoint: "https://example.invalid/api".into(),
                model: model.into(),
            };
            let config = MemoryConfig {
                enabled: true,
                provider: Some(target.clone()),
                credential_storage: Some(CredentialStorage::SessionOnly),
                consent: Some(MemoryConsent {
                    endpoint: target.endpoint.clone(),
                    model: target.model.clone(),
                    scope: "personal".into(),
                    send_source_snapshots: true,
                    send_memory_snapshots: true,
                    auto_apply: true,
                    accepted_at: Utc::now(),
                }),
                ..Default::default()
            };
            let credential =
                PreparedCredential::session(target, format!("SYNTHETIC_{model}")).unwrap();
            let mut provider = PythonMemoryProvider::new("python3".into(), package)
                .without_environment()
                .with_credential(credential);
            let result = provider.execute(&config, &job).unwrap();
            assert_eq!(result.usage.network_calls, 0);
        })
    });
    for thread in threads {
        thread.join().unwrap();
    }
    assert!(
        std::env::var_os("RECALLCARD_DREAM_API_KEY") == before,
        "父进程环境不得修改，也不在失败信息中显示旧值"
    );
}
