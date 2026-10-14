//! 启停代次只在短锁内变更；慢暂停操作另行串行化，失败保留 pending。
use super::*;

const STOP_SCHEMA: &str = "recallcard.service-stop/2";
const LOCK_WAIT: Duration = Duration::from_secs(5);
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Phase {
    Pending,
    Applied,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StopRequest {
    schema: String,
    request_id: String,
    generation: Option<String>,
    requested_at: DateTime<Utc>,
    phase: Phase,
}
#[derive(Clone)]
enum StopRecord {
    Legacy,
    Current(StopRequest),
}
impl StopRecord {
    fn pending(&self) -> bool {
        matches!(self, Self::Legacy)
            || matches!(self, Self::Current(r) if r.phase == Phase::Pending)
    }
    fn matches(&self, generation: Option<&str>) -> bool {
        match self {
            Self::Legacy => true,
            Self::Current(request) => request.generation.as_deref() == generation,
        }
    }
}
pub(super) fn valid_id(id: &str) -> bool {
    id.len() == 32
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn conflict(message: &str) -> AppError {
    let mut error = AppError::new(
        ErrorCode::Conflict,
        message,
        "等待当前启停操作完成；若停止操作中断，请重试停止后再启动",
    );
    error.retryable = true;
    error
}
fn bounded_lock(vault: &Vault, name: &str) -> AppResult<ServiceLease> {
    let path = directory(vault)?.join(name);
    let until = Instant::now() + LOCK_WAIT;
    loop {
        if let Some(guard) = try_lease(&path, false)? {
            return Ok(guard);
        }
        if Instant::now() >= until {
            return Err(conflict("服务启停状态正在更新，尚未取得操作锁"));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
pub(super) fn lock(vault: &Vault) -> AppResult<ServiceLease> {
    bounded_lock(vault, "lifecycle.lock")
}
fn read_stop(vault: &Vault) -> AppResult<Option<StopRecord>> {
    let path = directory(vault)?.join("stop.json");
    let Some(value): Option<serde_json::Value> = read_state_json(&path, 4096)? else {
        return Ok(None);
    };
    // 旧标记无法证明慢暂停已经结束，保守要求重试停止；不能直接丢弃。
    if value
        .as_object()
        .is_some_and(|map| map.len() == 1 && map.contains_key("requested_at"))
    {
        serde_json::from_value::<DateTime<Utc>>(value["requested_at"].clone())
            .map_err(|_| storage())?;
        return Ok(Some(StopRecord::Legacy));
    }
    let request: StopRequest = serde_json::from_value(value).map_err(|_| storage())?;
    if request.schema != STOP_SCHEMA
        || !valid_id(&request.request_id)
        || request
            .generation
            .as_deref()
            .is_some_and(|id| !valid_id(id))
    {
        return Err(storage());
    }
    Ok(Some(StopRecord::Current(request)))
}
fn save_stop(vault: &Vault, request: &StopRequest) -> AppResult<()> {
    vault
        .write_replace(&directory(vault)?.join("stop.json"), request)
        .map_err(|_| storage())
}
pub(super) fn for_generation(vault: &Vault, generation: Option<&str>) -> AppResult<bool> {
    Ok(read_stop(vault)?.is_some_and(|record| record.matches(generation)))
}
// 调用方已持 lifecycle；与代次及实际租约一起读取，不能拿旧身份解释新租约。
pub(super) fn status_locked(vault: &Vault) -> AppResult<ServiceStatus> {
    let mut status = read_status(vault)?;
    status.running = try_lease(&directory(vault)?.join("service.lock"), true)?.is_none();
    status.stop_requested = read_stop(vault)?.is_some_and(|record| {
        record.matches(status.generation.as_deref()) || (!status.running && record.matches(None))
    });
    Ok(status)
}
pub(super) enum Start {
    Owned(ServiceLease, Box<ServiceStatus>),
    Existing(Box<ServiceStatus>),
}
pub(super) fn begin(vault: &Vault) -> AppResult<Start> {
    begin_with(vault, || {})
}
// 私有同步边界供单元测试使用；没有环境变量或模型/外部调用入口。
fn begin_with(vault: &Vault, after_lease: impl FnOnce()) -> AppResult<Start> {
    let _guard = lock(vault)?;
    let stop = read_stop(vault)?;
    if stop.as_ref().is_some_and(StopRecord::pending) {
        return Err(conflict("上次停止尚未完成，未创建新的服务代次"));
    }
    let Some(lease) = try_lease(&directory(vault)?.join("service.lock"), false)? else {
        // 第二实例不得清除现有所有者的停止请求。
        return Ok(Start::Existing(Box::new(status_locked(vault)?)));
    };
    after_lease();
    let started = Utc::now();
    let status = ServiceStatus {
        generation: Some(uuid::Uuid::new_v4().simple().to_string()),
        running: true,
        pid: Some(std::process::id()),
        started_at: Some(started),
        heartbeat_at: Some(started),
        ..Default::default()
    };
    // 先持久化新身份，保存失败时保留旧停止记录。清理与发布共用短锁：
    // 外部启停操作只能在解锁后观察新代次，旧 applied 即使因崩溃残留也不匹配它。
    save_status(vault, &status)?;
    if stop.is_some() {
        fs::remove_file(directory(vault)?.join("stop.json")).map_err(|_| storage())?;
    }
    Ok(Start::Owned(lease, Box::new(status)))
}
fn prepare_stop(vault: &Vault, expected_generation: Option<&str>) -> AppResult<StopRequest> {
    let _guard = lock(vault)?;
    let status = status_locked(vault)?;
    if status.generation.as_deref() != expected_generation {
        return Err(conflict("等待停止锁期间服务代次已改变，未暂停新的实例"));
    }
    let generation = status.running.then_some(status.generation).flatten();
    if let Some(StopRecord::Current(request)) = read_stop(vault)? {
        if request.phase == Phase::Pending {
            if status.running && request.generation != generation {
                return Err(conflict("未完成停止请求与当前代次不一致，未重放暂停操作"));
            }
            return Ok(request);
        }
    }
    let request = StopRequest {
        schema: STOP_SCHEMA.into(),
        request_id: uuid::Uuid::new_v4().simple().to_string(),
        generation,
        requested_at: Utc::now(),
        phase: Phase::Pending,
    };
    save_stop(vault, &request)?;
    Ok(request)
}
fn finish_stop(vault: &Vault, request: &StopRequest) -> AppResult<()> {
    let _guard = lock(vault)?;
    if !matches!(read_stop(vault)?, Some(StopRecord::Current(ref current)) if current == request) {
        return Err(conflict("停止状态已变化，未覆盖新的请求"));
    }
    let mut applied = request.clone();
    applied.phase = Phase::Applied;
    save_stop(vault, &applied)
}
pub(super) fn request_stop(vault: &Vault) -> AppResult<()> {
    stop_with(vault, || {
        MemoryRuntime::new(vault).pause()?;
        ImportService::new(vault)?.pause_all_pending()?;
        Ok(())
    })
}
fn stop_with(vault: &Vault, apply: impl FnOnce() -> AppResult<()>) -> AppResult<()> {
    let expected_generation = {
        let _guard = lock(vault)?;
        read_status(vault)?.generation
    };
    stop_for_target(vault, expected_generation.as_deref(), apply)
}
fn stop_for_target(
    vault: &Vault,
    expected_generation: Option<&str>,
    apply: impl FnOnce() -> AppResult<()>,
) -> AppResult<()> {
    // 防止两个慢暂停乱序：旧请求仍在执行时，另一个请求不能先标 applied 放行新代次。
    let _apply = bounded_lock(vault, "stop-apply.lock")?;
    let request = prepare_stop(vault, expected_generation)?;
    // 绝不持 lifecycle 等 state/import 锁；checkpoint 可以反向查询 lifecycle。
    apply()?;
    finish_stop(vault, &request)
}

#[cfg(test)]
mod tests;
