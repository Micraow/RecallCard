//! 单个有界本机 worker 会话；配置变化换进程，空闲退出，超时杀进程。
use super::*;
use std::{
    process::Stdio,
    sync::{mpsc, Mutex, OnceLock},
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::{Child, ChildStdin, ChildStdout, Command},
};
struct Job {
    request: Value,
    deadline: Instant,
    reply: mpsc::SyncSender<Result<Value>>,
}
struct Handle {
    key: String,
    sender: mpsc::SyncSender<Job>,
}
static WORKER: OnceLock<Mutex<Option<Handle>>> = OnceLock::new();
pub(super) fn exchange(config: &Config, operation: Value) -> Result<Value> {
    exchange_before(config, operation, Instant::now() + Duration::from_secs(45))
}
pub(super) fn exchange_before(
    config: &Config,
    operation: Value,
    deadline: Instant,
) -> Result<Value> {
    if Instant::now() >= deadline {
        return Err("local_event_worker_timeout".into());
    }
    let key = hash(&serde_json::to_vec(config).map_err(|_| "local_event_config_invalid")?);
    let registry = WORKER.get_or_init(|| Mutex::new(None));
    let mut slot = registry
        .lock()
        .map_err(|_| "local_event_worker_unavailable")?;
    if slot.as_ref().is_none_or(|h| h.key != key) {
        *slot = None;
    }
    let (reply, receiver) = mpsc::sync_channel(1);
    let mut pending = Job {
        request: operation,
        deadline,
        reply,
    };
    for attempt in 0..2 {
        if slot.is_none() {
            let (sender, jobs) = mpsc::sync_channel::<Job>(4);
            let cfg = config.clone();
            std::thread::Builder::new()
                .name("recallcard-local-events".into())
                .spawn(move || supervise(cfg, jobs))
                .map_err(|_| "local_event_worker_unavailable")?;
            *slot = Some(Handle {
                key: key.clone(),
                sender,
            });
        }
        match slot.as_ref().unwrap().sender.try_send(pending) {
            Ok(()) => {
                drop(slot);
                return receiver
                    .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                    .map_err(|_| "local_event_worker_timeout".to_owned())?;
            }
            Err(mpsc::TrySendError::Full(_)) => return Err("local_event_worker_busy".into()),
            Err(mpsc::TrySendError::Disconnected(job)) => {
                *slot = None;
                pending = job;
                if attempt == 1 {
                    return Err("local_event_worker_unavailable".into());
                }
            }
        }
    }
    Err("local_event_worker_unavailable".into())
}

fn supervise(config: Config, receiver: mpsc::Receiver<Job>) {
    let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    else {
        return;
    };
    let mut process: Option<Process> = None;
    while let Ok(job) = receiver.recv_timeout(Duration::from_secs(30)) {
        let result = runtime.block_on(async {
            let remaining = job.deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err("local_event_worker_timeout".into());
            }
            if process.is_none() {
                process = Some(Process::start(&config)?);
            }
            let response = tokio::time::timeout(
                remaining.min(Duration::from_secs(40)),
                process.as_mut().unwrap().exchange(job.request),
            )
            .await
            .map_err(|_| "local_event_worker_timeout".to_owned())?;
            response
        });
        if result.is_err() {
            process = None;
        }
        let _ = job.reply.send(result);
    }
}
struct Process {
    child: Child,
    stdin: ChildStdin,
    stdout: ChildStdout,
}
impl Process {
    fn start(config: &Config) -> Result<Self> {
        let mut command = Command::new(&config.python);
        command.args(["-I","-S","-c","import sys; sys.path.insert(0, sys.argv[1]); from recallcard_worker.event_index import main; raise SystemExit(main(sys.argv[2:]))"])
            .arg(&config.python_path).arg("--config").arg(&config.worker_config).env_clear()
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).kill_on_drop(true);
        for key in ["PATH", "SystemRoot", "WINDIR", "TEMP", "TMP"] {
            if let Some(value) = std::env::var_os(key) {
                command.env(key, value);
            }
        }
        let mut child = command
            .spawn()
            .map_err(|_| "local_event_worker_unavailable")?;
        let stdin = child.stdin.take().ok_or("local_event_worker_unavailable")?;
        let stdout = child
            .stdout
            .take()
            .ok_or("local_event_worker_unavailable")?;
        Ok(Self {
            child,
            stdin,
            stdout,
        })
    }
    async fn exchange(&mut self, mut request: Value) -> Result<Value> {
        request["schema"] = json!("recallcard.local-event-index/1");
        request["id"] = json!(1);
        let mut bytes = serde_json::to_vec(&request).map_err(|_| "local_event_request_invalid")?;
        if bytes.len() > 256 * 1024 {
            return Err("local_event_request_limit".into());
        }
        bytes.push(b'\n');
        self.stdin
            .write_all(&bytes)
            .await
            .map_err(|_| "local_event_worker_io")?;
        self.stdin
            .flush()
            .await
            .map_err(|_| "local_event_worker_io")?;
        let mut response = Vec::new();
        loop {
            let mut buf = [0; 4096];
            let count = self
                .stdout
                .read(&mut buf)
                .await
                .map_err(|_| "local_event_worker_io")?;
            if count == 0 {
                return Err("local_event_worker_exited".into());
            }
            if response.len() + count > 128 * 1024 {
                return Err("local_event_response_limit".into());
            }
            response.extend_from_slice(&buf[..count]);
            if response.contains(&b'\n') {
                break;
            }
        }
        if response.last() != Some(&b'\n') || response[..response.len() - 1].contains(&b'\n') {
            return Err("local_event_response_invalid".into());
        }
        let envelope: Value =
            serde_json::from_slice(&response).map_err(|_| "local_event_response_invalid")?;
        if envelope["schema"] != "recallcard.local-event-index/1" || envelope["id"] != 1 {
            return Err("local_event_response_invalid".into());
        }
        if envelope["ok"] != true {
            let code = envelope["error"]["code"].as_str().unwrap_or("worker_error");
            if code.len() > 96 || !code.bytes().all(|c| c.is_ascii_lowercase() || c == b'_') {
                return Err("local_event_worker_error".into());
            }
            return Err(format!("local_event_{code}"));
        }
        Ok(envelope["result"].clone())
    }
}
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
    }
}
