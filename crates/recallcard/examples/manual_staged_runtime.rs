//! Synthetic operator gateway exercising the production staged runtime and Python provider.
//! No model replies are generated here; approval requires a separate explicit command.
use chrono::Utc;
use recallcard::application::background_memory::{
    MemoryConfig, MemoryConsent, MemoryProviderConfig, MemoryRuntime, MemoryTimeouts,
    PythonMemoryProvider,
};
use recallcard::Vault;
use serde_json::json;
use std::{fs, path::PathBuf};

fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let action = args
        .get(1)
        .ok_or("expected start, inspect, approve or revoke")?;
    let root = PathBuf::from(args.get(2).ok_or("expected synthetic run directory")?);
    if !root.is_absolute() {
        return Err("run directory must be absolute".into());
    }
    let scope = "project:atlas";
    if action == "start" || action == "continue" {
        let endpoint = args.get(3).ok_or("expected loopback HTTPS endpoint")?;
        if !endpoint.starts_with("https://127.0.0.1:")
            || !endpoint.ends_with("/v1/chat/completions")
        {
            return Err("only synthetic loopback HTTPS is permitted".into());
        }
        if action == "start" && root.join("vault").exists() {
            return Err("use a new synthetic vault; never overwrite evidence".into());
        }
        fs::create_dir_all(&root)?;
        let vault = if action == "start" {
            Vault::init(&root.join("vault"))?
        } else {
            Vault::open(&root.join("vault"))?
        };
        if action == "start" {
            for (id, role, content) in [
            ("u1", "user", "合成 Atlas 项目：当前使用本地 Rust 核心，目录导航优先。现在不启用独立 Python worker；只在确有需要时再评估。"),
            ("a1", "assistant", "合成助手建议：可以考虑云向量数据库，但用户尚未采纳，也没有实施。"),
        ] {
            vault.capture(serde_json::from_value(json!({"role":role,"origin":"native","scope":scope,"content":content,"source":{"platform":"synthetic","conversation_id":"staged-atlas","message_id":id}}))?)?;
        }
            assert!(vault.memories()?.is_empty());
        } else {
            assert_eq!(
                vault.memories()?.len(),
                1,
                "continue requires the previously reviewed synthetic Memory"
            );
            vault.capture(serde_json::from_value(json!({"role":"user","origin":"native","scope":scope,"content":"合成 Atlas 用户改口：现在启用独立 Python worker，但只用于本机离线向量编码。Rust 核心与目录导航优先不变，不启用云向量数据库。此前暂不启用 worker 的决定已经过时。","source":{"platform":"synthetic","conversation_id":"staged-atlas","message_id":"u2-correction"}}))?)?;
        }
        let before_memories = vault.memories()?.len();
        let audit = args
            .get(6)
            .map(PathBuf::from)
            .unwrap_or_else(|| root.clone());
        fs::create_dir_all(&audit)?;
        let target = MemoryProviderConfig {
            endpoint: endpoint.clone(),
            model: "manual-staged-operator".into(),
        };
        let config = MemoryConfig {
            enabled: true,
            scope: scope.into(),
            quiet_seconds: 0,
            provider: Some(target.clone()),
            consent: Some(MemoryConsent {
                endpoint: target.endpoint,
                model: target.model,
                scope: scope.into(),
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
        let runtime = MemoryRuntime::new(&vault);
        runtime.configure(config).map_err(|e| format!("{e:?}"))?;
        let mut provider = PythonMemoryProvider::new(
            PathBuf::from(
                args.get(4)
                    .ok_or("expected test TLS-trust Python launcher")?,
            ),
            PathBuf::from(args.get(5).ok_or("expected production Python modules")?),
        );
        let job = runtime
            .tick(&mut provider)
            .map_err(|e| format!("{e:?}"))?
            .ok_or("runtime did not schedule source events")?;
        fs::write(
            audit.join("job-status.json"),
            serde_json::to_vec_pretty(&job)?,
        )?;
        fs::write(audit.join("job-id.txt"), &job.job_id)?;
        let review = runtime
            .review(&job.job_id, scope)
            .map_err(|e| format!("{e:?}"))?;
        fs::write(
            audit.join("review.json"),
            serde_json::to_vec_pretty(&review)?,
        )?;
        if review.review.is_none() {
            return Err(
                "runtime did not produce a reviewable result; inspect preserved job status".into(),
            );
        }
        assert_eq!(vault.memories()?.len(), before_memories);
        println!(
            "{}",
            json!({"state":job.state,"job_id":job.job_id,"memories":vault.memories()?.len(),"review_ready":review.review.is_some(),"automatic_publish":false})
        );
    } else {
        let vault = Vault::open(&root.join("vault"))?;
        let runtime = MemoryRuntime::new(&vault);
        let id = if action == "revoke" {
            String::new()
        } else {
            match args.get(4) {
                Some(id) => id.clone(),
                None => fs::read_to_string(root.join("job-id.txt"))?,
            }
        };
        match action.as_str() {
            "inspect" => println!(
                "{}",
                serde_json::to_string_pretty(
                    &runtime.review(&id, scope).map_err(|e| format!("{e:?}"))?
                )?
            ),
            "approve" => {
                let hash = args
                    .get(3)
                    .ok_or("explicit reviewed result hash required")?;
                let result = runtime
                    .approve(&id, scope, hash, false)
                    .map_err(|e| format!("{e:?}"))?;
                println!("{}", serde_json::to_string_pretty(&result)?);
            }
            "revoke" => {
                let mut config = runtime.configuration().map_err(|e| format!("{e:?}"))?;
                config.enabled = false;
                config.consent = None;
                runtime.configure(config).map_err(|e| format!("{e:?}"))?;
                println!("{{\"revoked\":true}}");
            }
            _ => return Err("unknown operator action".into()),
        }
    }
    Ok(())
}
