//! Synthetic-only real desktop model setup -> independent service -> operator review.
//! No call to MemoryRuntime::tick and no fabricated provider replies.
use chrono::Utc;
use recallcard::{
    application::{background_memory::*, credentials::CredentialStorage},
    desktop::DesktopSession,
    Vault,
};
use serde_json::json;
use std::{
    fs,
    path::PathBuf,
    time::{Duration, Instant},
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let action = args.get(1).ok_or("expected start or approve")?;
    let root = PathBuf::from(args.get(2).ok_or("expected absolute synthetic root")?);
    if !root.is_absolute() {
        return Err("absolute root required".into());
    }
    let vault = if action == "start" {
        if root.join("vault").exists() {
            return Err("use a fresh synthetic vault".into());
        }
        fs::create_dir_all(&root)?;
        Vault::init(&root.join("vault"))?
    } else {
        Vault::open(&root.join("vault"))?
    };
    let mut desktop = DesktopSession::default();
    let session = desktop.select_vault(vault.root(), false)?;
    let operation = desktop
        .model_operation(&session.session_id, "project:atlas")
        .map_err(|e| format!("{e:?}"))?;
    if action == "resume" {
        let mut config = MemoryRuntime::new(&vault)
            .configuration()
            .map_err(|e| format!("{e:?}"))?;
        config.paused = false;
        let result = operation
            .configure(
                config,
                Some("synthetic-manual-gateway-only".into()),
                Some(CredentialStorage::SessionOnly),
            )
            .map_err(|e| format!("{e:?}"))?;
        println!("{}", serde_json::to_string_pretty(&result)?);
        return Ok(());
    }
    if action == "stop" {
        operation.stop_service().map_err(|e| format!("{e:?}"))?;
        let until = Instant::now() + Duration::from_secs(20);
        loop {
            let status = operation.status().map_err(|e| format!("{e:?}"))?;
            if !status.service.running {
                assert!(!status.credential.present);
                println!("{}", serde_json::to_string_pretty(&status.service)?);
                return Ok(());
            }
            if Instant::now() >= until {
                return Err("service did not stop".into());
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    if action == "approve" {
        let hash = args.get(3).ok_or("reviewed result hash required")?;
        let id = args.get(4).ok_or("job id required")?;
        let result = operation
            .approve(id, hash, false)
            .map_err(|e| format!("{e:?}"))?;
        println!("{}", serde_json::to_string_pretty(&result)?);
        return Ok(());
    }
    if action != "start" {
        return Err("unknown action".into());
    }
    let endpoint = args.get(3).ok_or("expected loopback HTTPS endpoint")?;
    if !endpoint.starts_with("https://127.0.0.1:") || !endpoint.ends_with("/v1/chat/completions") {
        return Err("synthetic loopback only".into());
    }
    let audit = PathBuf::from(args.get(6).ok_or("audit directory required")?);
    fs::create_dir_all(&audit)?;
    let target = MemoryProviderConfig {
        endpoint: endpoint.clone(),
        model: "manual-service-operator".into(),
    };
    let config = MemoryConfig {
        enabled: true,
        scope: "project:atlas".into(),
        quiet_seconds: 0,
        provider: Some(target.clone()),
        consent: Some(MemoryConsent {
            endpoint: target.endpoint,
            model: target.model,
            scope: "project:atlas".into(),
            send_source_snapshots: true,
            send_memory_snapshots: true,
            auto_apply: false,
            accepted_at: Utc::now(),
        }),
        timeouts: MemoryTimeouts {
            connect_seconds: 30,
            read_seconds: 1800,
            operation_seconds: 1800,
        },
        ..Default::default()
    };
    let setup = operation
        .configure(
            config,
            Some("synthetic-manual-gateway-only".into()),
            Some(CredentialStorage::SessionOnly),
        )
        .map_err(|e| format!("{e:?}"))?;
    fs::write(audit.join("setup.json"), serde_json::to_vec_pretty(&setup)?)?;
    if setup.service_error.is_some() {
        return Err("desktop setup could not start service; see receipt".into());
    }
    let service = setup.service.as_ref().ok_or("missing service receipt")?;
    assert!(service.running && service.provider_ready && service.credential.present);
    assert_ne!(service.pid, Some(std::process::id()));
    assert!(vault.events()?.is_empty());
    // Only now capture: automatic discovery must happen in the separate service.
    vault.capture(serde_json::from_value(json!({"role":"user","origin":"native","scope":"project:atlas","content":"合成 Atlas 服务验收：采用本地 Rust 核心，目录导航优先。独立 Python worker 只做本机离线向量编码，不使用云向量数据库。","source":{"platform":"synthetic","conversation_id":"service-atlas","message_id":"u1"}}))?)?;
    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
        let until = Instant::now() + Duration::from_secs(1800);
        loop {
            let status = operation.status().map_err(|e| format!("{e:?}"))?;
            if let Some(job) = status.runtime.jobs.first() {
                let state = serde_json::to_value(job)?
                    .get("state")
                    .cloned()
                    .unwrap_or_default();
                if state == "needs_input" || state == "failed" || state == "completed" {
                    fs::write(
                        audit.join("job-status.json"),
                        serde_json::to_vec_pretty(job)?,
                    )?;
                    fs::write(audit.join("job-id.txt"), &job.job_id)?;
                    let review = operation
                        .review(&job.job_id)
                        .map_err(|e| format!("{e:?}"))?;
                    fs::write(
                        audit.join("review.json"),
                        serde_json::to_vec_pretty(&review)?,
                    )?;
                    assert_eq!(
                        vault.memories()?.len(),
                        0,
                        "must not publish before human review"
                    );
                    if review.review.is_none() {
                        return Err("service produced no review; inspect job status".into());
                    }
                    println!(
                        "{}",
                        json!({"automatic_discovery":true,"separate_service_pid":service.pid,"job_id":job.job_id,"review_ready":true,"memories_before_review":0})
                    );
                    break;
                }
            }
            if Instant::now() >= until {
                return Err("service review timeout".into());
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        Ok(())
    })();
    operation.stop_service().map_err(|e| format!("{e:?}"))?;
    let until = Instant::now() + Duration::from_secs(20);
    loop {
        let status = operation.status().map_err(|e| format!("{e:?}"))?;
        if !status.service.running {
            assert!(!status.credential.present);
            fs::write(
                audit.join("stopped.json"),
                serde_json::to_vec_pretty(&status.service)?,
            )?;
            break;
        }
        if Instant::now() >= until {
            return Err("service did not stop".into());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    result
}
