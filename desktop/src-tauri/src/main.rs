#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use recallcard::desktop::DesktopSession;
use serde_json::Value;
use std::{
    fs::OpenOptions,
    io::Write,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};
use tauri::{AppHandle, Manager, State};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons};

type Service = Arc<Mutex<DesktopSession>>;
#[derive(Default)]
struct AppState {
    service: Service,
    dialog_open: Arc<AtomicBool>,
}
struct DialogGuard(Arc<AtomicBool>);
impl Drop for DialogGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}
fn dialog_guard(state: &AppState) -> Result<DialogGuard, String> {
    if state.dialog_open.swap(true, Ordering::SeqCst) {
        return Err("已有文件选择窗口，请先完成或取消".into());
    }
    Ok(DialogGuard(state.dialog_open.clone()))
}
async fn execute<T: serde::Serialize + Send + 'static>(
    service: Service,
    f: impl FnOnce(&mut DesktopSession) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut session = service.lock().map_err(|_| "资料库会话异常，请重启应用")?;
        f(&mut session)
    })
    .await
    .map_err(|_| "本地操作未完成，请重试；资料库仍保存在原目录".to_string())?
}
#[tauri::command]
fn build_info() -> recallcard::BuildInfo {
    recallcard::build_info()
}

// v0.6 GUI 与 CLI 共用应用任务，保留结构化错误，不吞掉文件/恢复诊断。
// 这里只在线程间传值，并不序列化。内部权限句柄不能为了通过此封装而导出为JSON；
// 真正的 #[tauri::command] 返回类型仍由 Tauri 检查可序列化的公开 DTO。
async fn execute_application<T: Send + 'static>(
    service: Service,
    f: impl FnOnce(&mut DesktopSession) -> recallcard::application::AppResult<T> + Send + 'static,
) -> recallcard::application::AppResult<T> {
    use recallcard::application::{AppError, ErrorCode};
    tauri::async_runtime::spawn_blocking(move || {
        let mut session = service.lock().map_err(|_| {
            AppError::new(
                ErrorCode::Internal,
                "本地资料服务暂不可用",
                "重新打开应用后重试",
            )
        })?;
        f(&mut session)
    })
    .await
    .map_err(|_| {
        AppError::new(
            ErrorCode::Internal,
            "本地操作未完成",
            "重新读取任务状态，确认已保存的内容后再继续",
        )
    })?
}

#[tauri::command]
async fn import_sources(
    app: AppHandle,
    state: State<'_, AppState>,
    session_id: String,
    scope: String,
    request_id: String,
) -> recallcard::application::AppResult<Option<recallcard::application::JobStatus>> {
    use recallcard::application::{AppError, ErrorCode};
    let guard = dialog_guard(&state)
        .map_err(|message| AppError::new(ErrorCode::Conflict, message, "先完成当前文件选择"))?;
    let selected = tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        app.dialog()
            .file()
            .set_title("添加来源：选择官方导出文件")
            .add_filter("会话导出", &["json", "zip"])
            .blocking_pick_files()
    })
    .await
    .map_err(|_| {
        AppError::new(
            ErrorCode::Internal,
            "文件选择未完成",
            "重新选择本机导出文件",
        )
    })?;
    let Some(selected) = selected else {
        return Ok(None);
    };
    let paths = selected
        .into_iter()
        .map(|path| {
            path.into_path().map_err(|_| {
                AppError::new(
                    ErrorCode::InvalidRequest,
                    "所选来源不是本机文件",
                    "下载导出文件后重新选择",
                )
            })
        })
        .collect::<recallcard::application::AppResult<Vec<_>>>()?;
    execute_application(state.service.clone(), move |service| {
        service
            .application_import_sources(&session_id, &scope, &request_id, &paths)
            .map(Some)
    })
    .await
}

#[tauri::command]
async fn application_jobs(
    state: State<'_, AppState>,
    session_id: String,
    scope: String,
) -> recallcard::application::AppResult<Vec<recallcard::application::JobStatus>> {
    execute_application(state.service.clone(), move |service| {
        service.application_jobs(&session_id, &scope)
    })
    .await
}
#[tauri::command]
async fn application_job(
    state: State<'_, AppState>,
    session_id: String,
    scope: String,
    job_id: String,
) -> recallcard::application::AppResult<recallcard::application::JobStatus> {
    execute_application(state.service.clone(), move |service| {
        service.application_job(&session_id, &scope, &job_id)
    })
    .await
}
#[tauri::command]
async fn application_import_result(
    state: State<'_, AppState>,
    session_id: String,
    scope: String,
    job_id: String,
) -> recallcard::application::AppResult<recallcard::application::ImportResult> {
    execute_application(state.service.clone(), move |service| {
        service.application_import_result(&session_id, &scope, &job_id)
    })
    .await
}

#[tauri::command]
async fn application_job_pause(
    state: State<'_, AppState>,
    session_id: String,
    scope: String,
    job_id: String,
) -> recallcard::application::AppResult<recallcard::application::JobStatus> {
    execute_application(state.service.clone(), move |service| {
        service.application_job_pause(&session_id, &scope, &job_id)
    })
    .await
}
#[tauri::command]
async fn application_job_resume(
    state: State<'_, AppState>,
    session_id: String,
    scope: String,
    job_id: String,
) -> recallcard::application::AppResult<recallcard::application::JobStatus> {
    execute_application(state.service.clone(), move |service| {
        service.application_job_resume(&session_id, &scope, &job_id)
    })
    .await
}
// 只在取得有身份绑定的句柄时持有资料会话锁；凭据访问和停机等待在锁外执行。
async fn execute_model<T: serde::Serialize + Send + 'static>(
    service: Service,
    session_id: String,
    scope: String,
    operation: impl FnOnce(recallcard::desktop::ModelOperation) -> recallcard::application::AppResult<T>
        + Send
        + 'static,
) -> recallcard::application::AppResult<T> {
    use recallcard::application::{AppError, ErrorCode};
    tauri::async_runtime::spawn_blocking(move || {
        let handle = {
            let selected = service.lock().map_err(|_| {
                AppError::new(
                    ErrorCode::Internal,
                    "本机资料会话暂不可用",
                    "重新打开空间后重试",
                )
            })?;
            selected.model_operation(&session_id, &scope)?
        };
        operation(handle)
    })
    .await
    .map_err(|_| {
        AppError::new(
            ErrorCode::Internal,
            "本机模型操作未完成",
            "先检查实际服务状态，勿重复提交密钥",
        )
    })?
}

#[tauri::command]
async fn model_setup_status(
    state: State<'_, AppState>,
    session_id: String,
    scope: String,
) -> recallcard::application::AppResult<recallcard::desktop::ModelSetupStatus> {
    execute_model(state.service.clone(), session_id, scope, |operation| {
        operation.status()
    })
    .await
}
#[tauri::command]
async fn inspect_model_credential(
    state: State<'_, AppState>,
    session_id: String,
    scope: String,
    target: recallcard::application::background_memory::MemoryProviderConfig,
) -> recallcard::application::AppResult<recallcard::application::credentials::CredentialStatus> {
    execute_model(state.service.clone(), session_id, scope, move |operation| {
        operation.inspect_credential(target)
    })
    .await
}
#[tauri::command]
async fn configure_memory_model(
    state: State<'_, AppState>,
    session_id: String,
    scope: String,
    config: recallcard::application::background_memory::MemoryConfig,
    api_key: Option<String>,
    credential_storage: Option<recallcard::application::credentials::CredentialStorage>,
) -> recallcard::application::AppResult<recallcard::application::runtime::ProviderSetupResult> {
    execute_model(state.service.clone(), session_id, scope, move |operation| {
        operation.configure(config, api_key, credential_storage)
    })
    .await
}
#[tauri::command]
async fn stop_local_service(
    state: State<'_, AppState>,
    session_id: String,
    scope: String,
) -> recallcard::application::AppResult<recallcard::application::runtime::ServiceStatus> {
    execute_model(state.service.clone(), session_id, scope, |operation| {
        operation.stop_service()
    })
    .await
}
#[tauri::command]
async fn memory_job_control(
    state: State<'_, AppState>,
    session_id: String,
    scope: String,
    job_id: String,
    action: recallcard::application::background_memory::MemoryJobControl,
) -> recallcard::application::AppResult<
    recallcard::application::JobStatus<recallcard::application::background_memory::MemoryProgress>,
> {
    execute_model(state.service.clone(), session_id, scope, move |operation| {
        operation.control(&job_id, action)
    })
    .await
}
#[tauri::command]
async fn memory_job_review(
    state: State<'_, AppState>,
    session_id: String,
    scope: String,
    job_id: String,
) -> recallcard::application::AppResult<recallcard::application::background_memory::MemoryJobReview>
{
    execute_model(state.service.clone(), session_id, scope, move |operation| {
        operation.review(&job_id)
    })
    .await
}

#[tauri::command]
async fn connection_inventory(
    state: State<'_, AppState>,
    session_id: String,
    scope: String,
) -> recallcard::application::AppResult<Value> {
    execute_application(state.service.clone(), move |service| {
        service.connection_inventory(&session_id, &scope)
    })
    .await
}
#[tauri::command]
async fn connection_configure(
    state: State<'_, AppState>,
    session_id: String,
    scope: String,
    grant: recallcard::application::connections::ConnectionGrant,
    expected_revision: Option<u64>,
) -> recallcard::application::AppResult<recallcard::application::connections::ConnectionEntry> {
    execute_application(state.service.clone(), move |service| {
        service.connection_configure(&session_id, &scope, grant, expected_revision)
    })
    .await
}
#[tauri::command]
async fn connection_approve_pairing(
    state: State<'_, AppState>,
    session_id: String,
    scope: String,
    request_id: String,
    grant: recallcard::application::connections::ConnectionGrant,
    expected_revision: Option<u64>,
) -> recallcard::application::AppResult<recallcard::application::connections::ConnectionEntry> {
    execute_application(state.service.clone(), move |service| {
        service.connection_approve_pairing(
            &session_id,
            &scope,
            &request_id,
            grant,
            expected_revision,
        )
    })
    .await
}
#[tauri::command]
async fn connection_revoke(
    state: State<'_, AppState>,
    session_id: String,
    scope: String,
    id: String,
    expected_revision: u64,
) -> recallcard::application::AppResult<recallcard::application::connections::ConnectionEntry> {
    execute_application(state.service.clone(), move |service| {
        service.connection_revoke(&session_id, &scope, &id, expected_revision)
    })
    .await
}

#[tauri::command]
async fn memory_runtime_status(
    state: State<'_, AppState>,
    session_id: String,
    scope: String,
) -> recallcard::application::AppResult<
    recallcard::application::background_memory::MemoryRuntimeStatus,
> {
    execute_application(state.service.clone(), move |service| {
        service.memory_runtime_status(&session_id, &scope)
    })
    .await
}
#[tauri::command]
async fn manage_memories_filtered(
    state: State<'_, AppState>,
    session_id: String,
    scope: String,
    filter: String,
    offset: usize,
) -> Result<Value, String> {
    execute(state.service.clone(), move |service| {
        service.manage_memories_filtered(&session_id, &scope, &filter, offset)
    })
    .await
}

#[tauri::command]
async fn managed_memory_view(
    state: State<'_, AppState>,
    session_id: String,
    scope: String,
    id: String,
) -> Result<Value, String> {
    execute(state.service.clone(), move |service| {
        service.managed_memory_view(&session_id, &scope, &id)
    })
    .await
}

/// 仅本地主窗口明确点击复制时写纯文本；不开放系统剪贴板读取。
#[tauri::command]
fn write_clipboard(
    app: AppHandle,
    window: tauri::WebviewWindow,
    text: String,
) -> Result<(), String> {
    if window.label() != "main" || text.is_empty() || text.len() > 1024 * 1024 {
        return Err("复制内容为空、超出上限或窗口无权操作".into());
    }
    app.clipboard()
        .write_text(text)
        .map_err(|_| "系统剪贴板写入失败，请重试".into())
}

#[tauri::command]
async fn choose_vault(
    app: AppHandle,
    state: State<'_, AppState>,
    create: bool,
) -> Result<Option<recallcard::desktop::VaultInfo>, String> {
    let guard = dialog_guard(&state)?;
    let selected = tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        let title = if create { "选择用于新资料库的空文件夹" } else { "打开已有 RecallCard 资料库" };
        let Some(path) = app.dialog().file().set_title(title).blocking_pick_folder() else { return Ok(None); };
        let path = path.into_path().map_err(|_| "请选择本机文件夹")?;
        if create && !app.dialog().message(format!("在此文件夹创建 RecallCard 资料库？\n\n{}\n\n将在此文件夹保存对话和记忆。现有文件不会被覆盖。", path.display())).title("创建资料库").buttons(MessageDialogButtons::OkCancelCustom("创建资料库".into(), "取消".into())).blocking_show() { return Ok(None); }
        Ok::<_, String>(Some(path))
    }).await.map_err(|_| "文件夹选择未完成")??;
    let Some(path) = selected else {
        return Ok(None);
    };
    execute(state.service.clone(), move |s| {
        s.select_vault(&path, create).map(Some)
    })
    .await
}
/// 新用户点击开始后使用应用私有目录；已有非资料库目录不覆盖、不迁移。
#[tauri::command]
async fn open_default_workspace(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<recallcard::desktop::VaultInfo, String> {
    let root = app
        .path()
        .app_data_dir()
        .map_err(|_| "无法确定本机保存位置")?
        .join("vault");
    let create = !root.exists();
    execute(state.service.clone(), move |s| {
        s.select_vault(&root, create)
    })
    .await
}

fn recent_workspace_file(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|directory| directory.join("recent-workspace.json"))
        .map_err(|_| "无法确定上次资料库设置的保存位置".into())
}

#[tauri::command]
async fn remember_workspace(
    app: AppHandle,
    state: State<'_, AppState>,
    session_id: String,
    scope: String,
) -> Result<(), String> {
    let path = recent_workspace_file(&app)?;
    execute(state.service.clone(), move |service| {
        service.remember_workspace(&session_id, &scope, &path)
    })
    .await
}

#[tauri::command]
async fn restore_workspace(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<recallcard::desktop::RestoredWorkspace>, String> {
    let path = recent_workspace_file(&app)?;
    execute(state.service.clone(), move |service| {
        service.restore_workspace(&path)
    })
    .await
}

/// 固定公开网址，不能由网页或导入文件传入地址、参数或任意系统命令。
#[tauri::command]
fn open_deepseek(window: tauri::WebviewWindow) -> Result<(), String> {
    if window.label() != "main" {
        return Err("此窗口不能打开外部网页".into());
    }
    #[cfg(target_os = "linux")]
    let mut command = std::process::Command::new("xdg-open");
    #[cfg(target_os = "macos")]
    let mut command = std::process::Command::new("open");
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut cmd = std::process::Command::new("rundll32.exe");
        cmd.arg("url.dll,FileProtocolHandler");
        cmd
    };
    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
    {
        let mut child = command
            .arg("https://chat.deepseek.com/")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|_| "无法请求浏览器打开，请手动打开 https://chat.deepseek.com/")?;
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        Ok(())
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    Err("请在浏览器打开 https://chat.deepseek.com/".into())
}

#[tauri::command]
async fn pick_import_files(
    app: AppHandle,
    state: State<'_, AppState>,
    session_id: String,
    scope: String,
    format: String,
) -> Result<Option<recallcard::desktop::ImportJobPreview>, String> {
    let guard = dialog_guard(&state)?;
    let paths = tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        app.dialog()
            .file()
            .set_title("选择导出文件（可多选）")
            .add_filter("会话导出", &["json", "jsonl", "zip"])
            .blocking_pick_files()
    })
    .await
    .map_err(|_| "文件选择未完成")?;
    let Some(paths) = paths else {
        return Ok(None);
    };
    let paths = paths
        .into_iter()
        .map(|p| p.into_path().map_err(|_| "请选择本机文件"))
        .collect::<Result<Vec<_>, _>>()?;
    execute(state.service.clone(), move |s| {
        s.prepare_import_job(&session_id, &format, &paths, &scope)
            .map(Some)
    })
    .await
}

#[tauri::command]
async fn start_import_job(
    state: State<'_, AppState>,
    session_id: String,
    preview_id: String,
    scope: String,
) -> Result<recallcard::desktop::ImportJobStatus, String> {
    execute(state.service.clone(), move |s| {
        s.start_import_job(&session_id, &preview_id, &scope)
    })
    .await
}
#[tauri::command]
async fn import_job_status(
    state: State<'_, AppState>,
    session_id: String,
    job_id: String,
    scope: String,
) -> Result<recallcard::desktop::ImportJobStatus, String> {
    execute(state.service.clone(), move |s| {
        s.import_job_status(&session_id, &job_id, &scope)
    })
    .await
}
#[tauri::command]
async fn cancel_import_job(
    state: State<'_, AppState>,
    session_id: String,
    job_id: String,
    scope: String,
) -> Result<recallcard::desktop::ImportJobStatus, String> {
    execute(state.service.clone(), move |s| {
        s.cancel_import_job(&session_id, &job_id, &scope)
    })
    .await
}
#[tauri::command]
async fn resume_import_job(
    state: State<'_, AppState>,
    session_id: String,
    job_id: String,
    scope: String,
) -> Result<recallcard::desktop::ImportJobStatus, String> {
    execute(state.service.clone(), move |s| {
        s.resume_import_job(&session_id, &job_id, &scope)
    })
    .await
}
#[tauri::command]
async fn list_import_jobs(
    state: State<'_, AppState>,
    session_id: String,
    scope: String,
) -> Result<Vec<recallcard::desktop::ImportJobStatus>, String> {
    execute(state.service.clone(), move |s| {
        s.list_import_jobs(&session_id, &scope)
    })
    .await
}

#[tauri::command]
async fn import_job_conversations(
    state: State<'_, AppState>,
    session_id: String,
    job_id: String,
    scope: String,
    offset: usize,
) -> Result<Value, String> {
    execute(state.service.clone(), move |s| {
        s.import_job_conversations(&session_id, &job_id, &scope, offset)
    })
    .await
}

#[tauri::command]
async fn vault_status(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<recallcard::desktop::VaultInfo, String> {
    execute(state.service.clone(), move |s| s.status(&session_id)).await
}
#[tauri::command]
async fn cancel_previews(state: State<'_, AppState>, session_id: String) -> Result<(), String> {
    execute(state.service.clone(), move |s| {
        s.cancel_previews(&session_id)
    })
    .await
}
#[tauri::command]
async fn browse_records(
    state: State<'_, AppState>,
    session_id: String,
    scope: String,
    target: String,
) -> Result<Value, String> {
    execute(state.service.clone(), move |s| {
        s.browse(&session_id, &scope, &target)
    })
    .await
}
#[tauri::command]
async fn search_records(
    state: State<'_, AppState>,
    session_id: String,
    scope: String,
    query: String,
    target: String,
) -> Result<Value, String> {
    execute(state.service.clone(), move |s| {
        s.search(&session_id, &scope, &query, &target)
    })
    .await
}
#[tauri::command]
async fn read_record(
    state: State<'_, AppState>,
    session_id: String,
    scope: String,
    reference: String,
) -> Result<Value, String> {
    execute(state.service.clone(), move |s| {
        s.read(&session_id, &scope, &reference)
    })
    .await
}
#[tauri::command]
async fn read_sources(
    state: State<'_, AppState>,
    session_id: String,
    scope: String,
    reference: String,
) -> Result<Value, String> {
    execute(state.service.clone(), move |s| {
        s.sources(&session_id, &scope, &reference)
    })
    .await
}
#[tauri::command]
async fn event_location(
    state: State<'_, AppState>,
    session_id: String,
    scope: String,
    reference: String,
) -> Result<Value, String> {
    execute(state.service.clone(), move |s| {
        s.event_location(&session_id, &scope, &reference)
    })
    .await
}
#[tauri::command]
async fn read_background(
    state: State<'_, AppState>,
    session_id: String,
    scope: String,
) -> Result<recallcard::desktop::BackgroundPage, String> {
    execute(state.service.clone(), move |s| {
        s.read_background(&session_id, &scope)
    })
    .await
}
#[tauri::command]
async fn read_background_page(
    state: State<'_, AppState>,
    session_id: String,
    scope: String,
    offset: usize,
) -> Result<recallcard::desktop::BackgroundPage, String> {
    execute(state.service.clone(), move |s| {
        s.read_background_page(&session_id, &scope, offset)
    })
    .await
}
#[tauri::command]
async fn background_memory(
    state: State<'_, AppState>,
    session_id: String,
    scope: String,
    id: String,
) -> Result<recallcard::model::Memory, String> {
    execute(state.service.clone(), move |s| {
        s.background_memory(&session_id, &scope, &id)
    })
    .await
}
#[tauri::command]
async fn background_memory_source(
    state: State<'_, AppState>,
    session_id: String,
    scope: String,
    memory_id: String,
    event_id: String,
) -> Result<recallcard::model::Event, String> {
    execute(state.service.clone(), move |s| {
        s.background_memory_source(&session_id, &scope, &memory_id, &event_id)
    })
    .await
}
#[tauri::command]
async fn review_background_change(
    state: State<'_, AppState>,
    session_id: String,
    scope: String,
    id: String,
    revision: u64,
    include: bool,
) -> Result<recallcard::desktop::BackgroundReview, String> {
    execute(state.service.clone(), move |s| {
        s.review_background_change(&session_id, &scope, &id, revision, include)
    })
    .await
}
#[tauri::command]
async fn confirm_background_change(
    state: State<'_, AppState>,
    session_id: String,
    scope: String,
    preview_id: String,
    approve_protected: bool,
) -> Result<Value, String> {
    execute(state.service.clone(), move |s| {
        s.confirm_background_change(&session_id, &scope, &preview_id, approve_protected)
    })
    .await
}
#[tauri::command]
async fn manage_memories(
    state: State<'_, AppState>,
    session_id: String,
    scope: String,
    include_hidden: bool,
    offset: usize,
) -> Result<Value, String> {
    execute(state.service.clone(), move |s| {
        s.manage_memories(&session_id, &scope, include_hidden, offset)
    })
    .await
}
#[tauri::command]
async fn managed_memory(
    state: State<'_, AppState>,
    session_id: String,
    scope: String,
    id: String,
) -> Result<recallcard::model::Memory, String> {
    execute(state.service.clone(), move |s| {
        s.managed_memory(&session_id, &scope, &id)
    })
    .await
}
#[tauri::command]
async fn managed_memory_source(
    state: State<'_, AppState>,
    session_id: String,
    scope: String,
    memory_id: String,
    event_id: String,
) -> Result<recallcard::model::Event, String> {
    execute(state.service.clone(), move |s| {
        s.managed_memory_source(&session_id, &scope, &memory_id, &event_id)
    })
    .await
}
#[tauri::command]
async fn review_memory_edit(
    state: State<'_, AppState>,
    session_id: String,
    scope: String,
    id: String,
    revision: u64,
    edit: recallcard::desktop::MemoryEdit,
) -> Result<recallcard::desktop::MemoryReview, String> {
    execute(state.service.clone(), move |s| {
        s.review_memory_edit(&session_id, &scope, &id, revision, edit)
    })
    .await
}
#[tauri::command]
async fn review_memory_visibility(
    state: State<'_, AppState>,
    session_id: String,
    scope: String,
    id: String,
    restore: bool,
    reason: String,
) -> Result<recallcard::desktop::MemoryReview, String> {
    execute(state.service.clone(), move |s| {
        s.review_memory_visibility(&session_id, &scope, &id, restore, &reason)
    })
    .await
}
#[tauri::command]
async fn confirm_memory_change(
    state: State<'_, AppState>,
    session_id: String,
    preview_id: String,
    approve_protected: bool,
) -> Result<Value, String> {
    execute(state.service.clone(), move |s| {
        s.confirm_memory_change(&session_id, &preview_id, approve_protected)
    })
    .await
}
#[tauri::command]
async fn list_conversations(
    state: State<'_, AppState>,
    session_id: String,
    scope: String,
    offset: usize,
) -> Result<Value, String> {
    execute(state.service.clone(), move |s| {
        s.conversations_page(&session_id, &scope, offset)
    })
    .await
}
#[tauri::command]
async fn conversation_messages(
    state: State<'_, AppState>,
    session_id: String,
    scope: String,
    conversation_ref: String,
    offset: usize,
) -> Result<Value, String> {
    execute(state.service.clone(), move |s| {
        s.conversation_messages(&session_id, &scope, &conversation_ref, offset)
    })
    .await
}
#[tauri::command]
async fn prepare_continuation(
    state: State<'_, AppState>,
    session_id: String,
    scope: String,
    conversation_ref: String,
    goal: String,
    branch_ref: Option<String>,
) -> Result<Value, String> {
    execute(state.service.clone(), move |s| {
        s.continuation_branch(
            &session_id,
            &scope,
            &conversation_ref,
            &goal,
            branch_ref.as_deref(),
        )
    })
    .await
}

#[tauri::command]
async fn prepare_selected_context(
    state: State<'_, AppState>,
    session_id: String,
    scope: String,
    references: Vec<String>,
    goal: String,
    budget_tokens: usize,
) -> Result<Value, String> {
    execute(state.service.clone(), move |service| {
        service.prepare_selected_context(&session_id, &scope, &references, &goal, budget_tokens)
    })
    .await
}

fn packaged_cli(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    let executable = std::env::current_exe().map_err(|_| "无法确定程序位置")?;
    let mut candidates = vec![executable
        .parent()
        .ok_or("程序目录无效")?
        .join(if cfg!(windows) {
            "recallcard.exe"
        } else {
            "recallcard"
        })];
    if let Ok(resources) = app.path().resource_dir() {
        candidates.push(resources.join("说明与许可证/recallcard"));
    }
    candidates
        .into_iter()
        .find(|p| p.is_file())
        .ok_or("当前安装包缺少连接组件，请使用包含 recallcard 的完整运行包或新版安装包".into())
}
fn prepare_client_binary(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    let bundled = packaged_cli(app)?;
    let bytes = std::fs::read(&bundled).map_err(|_| "无法读取随包连接组件")?;
    let directory = app
        .path()
        .app_local_data_dir()
        .map_err(|_| "无法确定本机应用目录")?
        .join("client-tools")
        .join(recallcard::hash(&bytes));
    for path in directory.ancestors() {
        if let Ok(metadata) = std::fs::symlink_metadata(path) {
            if metadata.is_symlink() {
                return Err("连接组件目录不能包含符号链接".into());
            }
        }
    }
    std::fs::create_dir_all(&directory).map_err(|_| "无法保存本机连接组件")?;
    let binary = directory.join(if cfg!(windows) {
        "recallcard.exe"
    } else {
        "recallcard"
    });
    if std::fs::symlink_metadata(&binary).is_ok_and(|m| m.is_symlink()) {
        return Err("连接组件不能为符号链接".into());
    }
    let valid_existing = std::fs::read(&binary).is_ok_and(|current| current == bytes);
    if !valid_existing {
        let mut file =
            tempfile::NamedTempFile::new_in(&directory).map_err(|_| "无法暂存连接组件")?;
        file.write_all(&bytes)
            .and_then(|_| file.as_file().sync_all())
            .map_err(|_| "连接组件暂存未完成")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.as_file()
                .set_permissions(std::fs::Permissions::from_mode(0o700))
                .map_err(|_| "无法设置组件执行权限")?;
        }
        file.persist(&binary)
            .map_err(|_| "连接组件发布未完成，请重试")?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let permissions = std::fs::metadata(&binary)
            .map_err(|_| "无法检查组件权限")?
            .permissions();
        if permissions.mode() & 0o100 == 0 {
            std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700))
                .map_err(|_| "无法恢复组件执行权限")?;
        }
    }
    Ok(binary)
}

fn packaged_extension(app: &AppHandle) -> Option<std::path::PathBuf> {
    let executable = std::env::current_exe().ok()?;
    let mut candidates = vec![executable.parent()?.join("extension")];
    if let Ok(resources) = app.path().resource_dir() {
        candidates.push(resources.join("说明与许可证/extension"));
    }
    candidates.into_iter().find(|root| {
        if root
            .ancestors()
            .any(|p| std::fs::symlink_metadata(p).is_ok_and(|m| m.is_symlink()))
        {
            return false;
        }
        let manifest = root.join("manifest.json");
        if std::fs::symlink_metadata(&manifest).is_ok_and(|m| m.is_symlink()) {
            return false;
        }
        let Ok(bytes) = std::fs::read(manifest) else {
            return false;
        };
        if bytes.len() > 64 * 1024 {
            return false;
        }
        let Ok(value) = serde_json::from_slice::<Value>(&bytes) else {
            return false;
        };
        value["manifest_version"] == 3
            && value["name"] == "RecallCard 会话与上下文"
            && value["version_name"] == recallcard::build_info().version
            && ["background.js", "content.js", "popup.html", "protocol.js"]
                .iter()
                .all(|name| {
                    let p = root.join(name);
                    p.is_file() && !std::fs::symlink_metadata(p).is_ok_and(|m| m.is_symlink())
                })
    })
}
#[tauri::command]
async fn browser_setup_info(app: AppHandle) -> Value {
    let directory = packaged_extension(&app);
    serde_json::json!({"distribution":"unpublished_test_package","available":directory.is_some(),"open_directory_available":directory.is_some(),"extension_dir":directory,"store_url":null,"fixed_extension_id":null,"instructions":["这是尚未上架商店的测试扩展。","在浏览器扩展管理页启用开发者模式，选择加载已解压扩展并选择本包extension目录。","本机桥首次注册仍需核对扩展ID；注册后从扩展发起连接申请，再在这里确认范围。","配置文件写入不代表浏览器已连接；必须收到实际握手与读取回执。"]})
}
#[tauri::command]
async fn open_browser_extension_directory(app: AppHandle) -> Result<(), String> {
    let path = packaged_extension(&app).ok_or("当前安装包没有可核验的测试扩展目录")?;
    tauri::async_runtime::spawn_blocking(move || {
        let program = if cfg!(target_os = "windows") {
            "explorer"
        } else if cfg!(target_os = "macos") {
            "open"
        } else {
            "xdg-open"
        };
        let mut child = std::process::Command::new(program)
            .arg(path)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|_| "系统文件管理器未能启动，请复制显示的目录路径".to_string())?;
        // 文件管理器可独立保持打开；后台回收启动进程，避免留下僵尸进程。
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        Ok(())
    })
    .await
    .map_err(|_| "目录打开操作未完成".to_string())?
}

#[tauri::command]
async fn prepare_client_config(
    app: AppHandle,
    state: State<'_, AppState>,
    session_id: String,
    scope: String,
) -> Result<Value, String> {
    let binary = prepare_client_binary(&app)?;
    execute(state.service.clone(),move|s|{
        let info=s.status(&session_id)?;
        recallcard::policy::Access::new(vec![scope.clone()])?;
        Ok(serde_json::json!({"mcpServers":{"recallcard":{"command":binary,"args":["--vault",info.root,"mcp","--scope",scope]}}}))
    }).await
}
#[tauri::command]
async fn chatgpt_connection_plan(
    app: AppHandle,
    state: State<'_, AppState>,
    session_id: String,
    scope: String,
) -> recallcard::application::AppResult<Value> {
    use recallcard::application::{AppError, ErrorCode};
    let binary = prepare_client_binary(&app).map_err(|message| {
        AppError::new(
            ErrorCode::ModelUnavailable,
            message,
            "先安装包含本机读取组件的完整程序",
        )
    })?;
    execute_application(state.service.clone(), move |service| {
        service.chatgpt_connection_plan(&session_id, &scope, &binary)
    })
    .await
}

#[tauri::command]
async fn connection_agent_configs(
    app: AppHandle,
    state: State<'_, AppState>,
    session_id: String,
    scope: String,
    id: String,
) -> recallcard::application::AppResult<Value> {
    use recallcard::application::{AppError, ErrorCode};
    let binary = prepare_client_binary(&app).map_err(|message| {
        AppError::new(
            ErrorCode::ModelUnavailable,
            message,
            "请使用含本机连接组件的完整安装包",
        )
    })?;
    execute_application(state.service.clone(), move |service| {
        service.connection_agent_configs(&session_id, &scope, &id, &binary)
    })
    .await
}

#[tauri::command]
async fn choose_connection_project(
    app: AppHandle,
    state: State<'_, AppState>,
    session_id: String,
    scope: String,
) -> recallcard::application::AppResult<Option<String>> {
    use recallcard::application::{AppError, ErrorCode};
    execute_application(state.service.clone(), move |s| {
        s.connection_operation(&session_id, &scope)
    })
    .await?;
    let guard = dialog_guard(&state)
        .map_err(|message| AppError::new(ErrorCode::Conflict, message, "关闭已有选择窗口后重试"))?;
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        app.dialog()
            .file()
            .set_title("选择此 Agent 工作的项目目录")
            .blocking_pick_folder()
            .map(|p| {
                p.into_path()
                    .map(|path| path.to_string_lossy().into_owned())
                    .map_err(|_| {
                        AppError::new(ErrorCode::InvalidRequest, "请选择本机项目目录", "重新选择")
                    })
            })
            .transpose()
    })
    .await
    .map_err(|_| AppError::new(ErrorCode::Internal, "目录选择未完成", "重试选择项目目录"))?
}
#[tauri::command]
async fn connection_setup_plan(
    app: AppHandle,
    state: State<'_, AppState>,
    session_id: String,
    scope: String,
    client: String,
    project_dir: String,
) -> recallcard::application::AppResult<recallcard::application::agent_install::InstallPlan> {
    use recallcard::application::{AppError, ErrorCode};
    let binary = prepare_client_binary(&app).map_err(|message| {
        AppError::new(
            ErrorCode::Storage,
            message,
            "使用含本机读取组件的完整安装包",
        )
    })?;
    let operation = execute_application(state.service.clone(), move |s| {
        s.connection_operation(&session_id, &scope)
    })
    .await?;
    tauri::async_runtime::spawn_blocking(move || {
        operation.plan(&client, std::path::Path::new(&project_dir), &binary)
    })
    .await
    .map_err(|_| AppError::new(ErrorCode::Internal, "连接预览未完成", "重新预览后确认"))?
}
#[tauri::command]
async fn apply_connection_setup(
    state: State<'_, AppState>,
    session_id: String,
    scope: String,
    plan_id: String,
) -> recallcard::application::AppResult<Value> {
    use recallcard::application::{AppError, ErrorCode};
    let operation = execute_application(state.service.clone(), move |s| {
        s.connection_operation(&session_id, &scope)
    })
    .await?;
    tauri::async_runtime::spawn_blocking(move || operation.apply(&plan_id))
        .await
        .map_err(|_| {
            AppError::new(
                ErrorCode::Internal,
                "连接配置结果尚未确认",
                "先检查连接状态，不要重复提交",
            )
        })?
}
#[tauri::command]
async fn connection_health(
    state: State<'_, AppState>,
    session_id: String,
    scope: String,
    connection_id: String,
) -> recallcard::application::AppResult<recallcard::application::connection_setup::ConnectionHealth>
{
    use recallcard::application::{AppError, ErrorCode};
    let operation = execute_application(state.service.clone(), move |s| {
        s.connection_operation(&session_id, &scope)
    })
    .await?;
    tauri::async_runtime::spawn_blocking(move || operation.health(&connection_id))
        .await
        .map_err(|_| AppError::new(ErrorCode::Internal, "连接状态暂不可读取", "稍后刷新"))?
}
#[tauri::command]
async fn verify_connection(
    app: AppHandle,
    state: State<'_, AppState>,
    session_id: String,
    scope: String,
    connection_id: String,
) -> recallcard::application::AppResult<recallcard::application::connection_setup::ConnectionHealth>
{
    use recallcard::application::{AppError, ErrorCode};
    let binary = prepare_client_binary(&app)
        .map_err(|message| AppError::new(ErrorCode::Storage, message, "使用完整安装包"))?;
    let operation = execute_application(state.service.clone(), move |s| {
        s.connection_operation(&session_id, &scope)
    })
    .await?;
    tauri::async_runtime::spawn_blocking(move || operation.verify(&connection_id, &binary))
        .await
        .map_err(|_| AppError::new(ErrorCode::Internal, "本机试读暂未完成", "检查状态后重试"))?
}

#[tauri::command]
async fn install_browser_connection(
    app: AppHandle,
    state: State<'_, AppState>,
    session_id: String,
    scope: String,
    extension_id: String,
    browser: String,
    allow_capture: bool,
) -> Result<Option<Value>, String> {
    recallcard::native::extension_origin(&extension_id)?;
    let browser_dir = match browser.as_str() {
        "chromium" => "chromium",
        "chrome" => "google-chrome",
        "brave" => "BraveSoftware/Brave-Browser",
        _ => return Err("请选择 Chromium、Chrome 或 Brave".into()),
    };
    if !cfg!(target_os = "linux") {
        return Err("当前图形化注册先支持 Linux，其他平台仍可使用随包说明".into());
    }
    let binary = packaged_cli(&app)?;
    let config_dir = app
        .path()
        .config_dir()
        .map_err(|_| "无法确定本机配置目录")?;
    let target = config_dir
        .join(browser_dir)
        .join("NativeMessagingHosts/com.recallcard.host.json");
    let previous = match std::fs::symlink_metadata(&target) {
        Ok(metadata) => {
            if !metadata.is_file() || metadata.is_symlink() || metadata.len() > 16 * 1024 {
                return Err("已有连接文件不是可更新的普通配置，本次停止".into());
            }
            let bytes = std::fs::read(&target).map_err(|_| "无法检查已有连接")?;
            let value: Value =
                serde_json::from_slice(&bytes).map_err(|_| "已有连接配置损坏，请先修复")?;
            if value["name"] != "com.recallcard.host" {
                return Err("已有文件不属于 RecallCard，本次不会覆盖".into());
            }
            Some(bytes)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => return Err("无法检查已有浏览器连接".into()),
    };
    let replacing = previous.is_some();
    let data_dir = app
        .path()
        .app_local_data_dir()
        .map_err(|_| "无法确定应用数据目录")?;
    let output = data_dir.join("browser-connections").join(format!(
        "{}-{}",
        browser,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| "系统时间无效")?
            .as_nanos()
    ));
    let guard = dialog_guard(&state)?;
    let confirm_path = target.clone();
    let confirm_scope = scope.clone();
    let confirm_extension = extension_id.clone();
    let confirmed=tauri::async_runtime::spawn_blocking(move||{
        let _guard=guard;
        app.dialog().message(format!("允许此浏览器扩展连接当前资料库？\n\n扩展 ID：{}\n资料范围：{}\n允许保存预览后的对话：{}\n\n将注册：{}\n{}\n请仅填写你安装的 RecallCard 扩展 ID。",confirm_extension,confirm_scope,if allow_capture{"是"}else{"否"},confirm_path.display(),if replacing{"已有 RecallCard 连接会先备份，再更新到本次资料库和权限。"}else{"新建本机连接，不修改其他扩展。"})).title("连接浏览器扩展").buttons(MessageDialogButtons::OkCancelCustom("允许并连接".into(),"取消".into())).blocking_show()
    }).await.map_err(|_|"连接确认未完成")?;
    if !confirmed {
        return Ok(None);
    }
    execute(state.service.clone(),move|s|{
        let parent=target.parent().ok_or("注册目录无效")?;
        for path in parent.ancestors(){
            if let Ok(m)=std::fs::symlink_metadata(path){if m.is_symlink(){return Err("浏览器配置目录包含链接，本次未注册".into());}}
        }
        std::fs::create_dir_all(parent).map_err(|_|"无法建立此浏览器的本机连接目录")?;
        let result=s.prepare_browser_connection(&session_id,&scope,&extension_id,&output,&binary,allow_capture)?;
        let manifest=std::fs::read(output.join("com.recallcard.host.json")).map_err(|_|"无法读取生成的连接文件")?;
        let current=std::fs::read(&target).ok();
        if current!=previous || std::fs::symlink_metadata(&target).is_ok_and(|m|m.is_symlink()) {return Err("确认后原连接发生变化，请重新检查再连接".into());}
        if let Some(bytes)=previous {
            let backup=parent.join(format!("com.recallcard.host.{}.backup",&recallcard::hash(&bytes)[..16]));
            if backup.exists(){if std::fs::symlink_metadata(&backup).map_err(|_|"无法检查备份")?.is_symlink() || std::fs::read(&backup).map_err(|_|"无法读取备份")?!=bytes{return Err("连接备份文件发生冲突，原连接保持不变".into());}}
            else{let mut file=OpenOptions::new().write(true).create_new(true).open(backup).map_err(|_|"无法备份原连接")?;file.write_all(&bytes).and_then(|_|file.sync_all()).map_err(|_|"原连接备份未完成")?;}
        }
        let mut temporary=tempfile::NamedTempFile::new_in(parent).map_err(|_|"无法暂存本机连接")?;
        temporary.write_all(&manifest).and_then(|_|temporary.as_file().sync_all()).map_err(|_|"连接暂存未完成")?;
        temporary.persist(&target).map_err(|_|"连接替换失败，原配置备份已保留")?;
        Ok(Some(serde_json::json!({"registered":true,"registration":target,"capture_enabled":allow_capture,"capture_scope":scope,"files":result,"note":"本机文件已注册；请回到扩展点击检查连接，才能确认浏览器实际连通"})))
    }).await
}

#[tauri::command]
async fn preview_note(
    state: State<'_, AppState>,
    session_id: String,
    scope: String,
    content: String,
) -> Result<recallcard::desktop::NotePreview, String> {
    execute(state.service.clone(), move |s| {
        s.preview_note(&session_id, &scope, &content)
    })
    .await
}
#[tauri::command]
async fn confirm_note(
    state: State<'_, AppState>,
    session_id: String,
    preview_id: String,
) -> Result<Value, String> {
    execute(state.service.clone(), move |s| {
        s.confirm_note(&session_id, &preview_id)
    })
    .await
}
#[tauri::command]
async fn pick_import(
    app: AppHandle,
    state: State<'_, AppState>,
    session_id: String,
    scope: String,
    format: String,
) -> Result<Option<recallcard::desktop::ImportFilePreview>, String> {
    let guard = dialog_guard(&state)?;
    let path = tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        app.dialog()
            .file()
            .set_title("选择要导入的对话文件")
            .add_filter("对话导出或备份", &["json", "jsonl", "zip"])
            .blocking_pick_file()
    })
    .await
    .map_err(|_| "文件选择未完成")?;
    let Some(path) = path else {
        return Ok(None);
    };
    let path = path.into_path().map_err(|_| "请选择本机文件")?;
    execute(state.service.clone(), move |s| {
        s.select_import_file(&session_id, &format, &path, &scope)
            .map(Some)
    })
    .await
}
#[tauri::command]
async fn preview_import_selection(
    state: State<'_, AppState>,
    session_id: String,
    selection_id: String,
    source_ids: Vec<String>,
) -> Result<recallcard::desktop::ImportPreview, String> {
    execute(state.service.clone(), move |s| {
        s.preview_import_selection(&session_id, &selection_id, &source_ids)
    })
    .await
}
#[tauri::command]
async fn return_import_selection(
    state: State<'_, AppState>,
    session_id: String,
    selection_id: String,
) -> Result<(), String> {
    execute(state.service.clone(), move |s| {
        s.return_import_selection(&session_id, &selection_id)
    })
    .await
}
#[tauri::command]
async fn confirm_import(
    state: State<'_, AppState>,
    session_id: String,
    preview_id: String,
) -> Result<Value, String> {
    execute(state.service.clone(), move |s| {
        s.confirm_import(&session_id, &preview_id)
    })
    .await
}
#[tauri::command]
async fn prepare_dream_task(
    state: State<'_, AppState>,
    session_id: String,
    scope: String,
    source_refs: Vec<String>,
    memory_refs: Vec<String>,
) -> Result<Value, String> {
    execute(state.service.clone(), move |s| {
        s.prepare_dream_task(&session_id, &scope, &source_refs, &memory_refs)
    })
    .await
}
#[tauri::command]
async fn review_dream_text(
    state: State<'_, AppState>,
    session_id: String,
    scope: String,
    text: String,
) -> Result<recallcard::desktop::DreamPreview, String> {
    execute(state.service.clone(), move |s| {
        s.review_dream_text(&session_id, &scope, &text)
    })
    .await
}

#[tauri::command]
async fn pick_dream(
    app: AppHandle,
    state: State<'_, AppState>,
    session_id: String,
    scope: String,
) -> Result<Option<recallcard::desktop::DreamPreview>, String> {
    let guard = dialog_guard(&state)?;
    let path = tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        app.dialog()
            .file()
            .set_title("选择整理结果文件")
            .add_filter("Dream JSON", &["json"])
            .blocking_pick_file()
    })
    .await
    .map_err(|_| "文件选择未完成")?;
    let Some(path) = path else {
        return Ok(None);
    };
    let path = path.into_path().map_err(|_| "请选择本机文件")?;
    execute(state.service.clone(), move |s| {
        s.review_dream(&session_id, &path, &scope).map(Some)
    })
    .await
}
#[tauri::command]
async fn apply_dream(
    state: State<'_, AppState>,
    session_id: String,
    preview_id: String,
    approve_protected: bool,
) -> Result<recallcard::dream::DreamReceipt, String> {
    execute(state.service.clone(), move |s| {
        s.apply_dream(&session_id, &preview_id, approve_protected)
    })
    .await
}
#[tauri::command]
async fn export_dream(
    app: AppHandle,
    state: State<'_, AppState>,
    session_id: String,
    scope: String,
    source_refs: Vec<String>,
    memory_refs: Vec<String>,
) -> Result<Option<String>, String> {
    let guard = dialog_guard(&state)?;
    let path = tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        app.dialog()
            .file()
            .set_title("保存整理包")
            .set_file_name("recallcard-dream-job.json")
            .add_filter("Dream JSON", &["json"])
            .blocking_save_file()
    })
    .await
    .map_err(|_| "保存位置选择未完成")?;
    let Some(path) = path else {
        return Ok(None);
    };
    let path = path.into_path().map_err(|_| "请选择本机保存位置")?;
    execute(state.service.clone(), move |s| {
        // 不覆盖已有文件；保存对话框取消时也不创建本地 Dream job。
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|_| "无法创建文件；请选择不存在的新文件名")?;
        let result = (|| {
            let job = s.export_dream(&session_id, &scope, &source_refs, &memory_refs)?;
            let bytes = serde_json::to_vec_pretty(&job).map_err(|_| "无法编码来源包")?;
            file.write_all(&bytes).map_err(|_| "来源包写入未完成")?;
            file.sync_all().map_err(|_| "来源包保存未完成")?;
            Ok(Some(path.display().to_string()))
        })();
        if result.is_err() {
            drop(file);
            let _ = std::fs::remove_file(&path);
        }
        result
    })
    .await
}
fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            build_info,
            import_sources,
            application_jobs,
            application_job,
            application_import_result,
            application_job_pause,
            application_job_resume,
            memory_runtime_status,
            model_setup_status,
            inspect_model_credential,
            configure_memory_model,
            stop_local_service,
            memory_job_control,
            memory_job_review,
            connection_inventory,
            connection_configure,
            connection_agent_configs,
            choose_connection_project,
            connection_setup_plan,
            apply_connection_setup,
            connection_health,
            verify_connection,
            browser_setup_info,
            open_browser_extension_directory,
            chatgpt_connection_plan,
            connection_approve_pairing,
            connection_revoke,
            managed_memory_view,
            manage_memories_filtered,
            choose_vault,
            open_default_workspace,
            remember_workspace,
            restore_workspace,
            open_deepseek,
            pick_import_files,
            start_import_job,
            import_job_status,
            cancel_import_job,
            resume_import_job,
            list_import_jobs,
            import_job_conversations,
            write_clipboard,
            cancel_previews,
            vault_status,
            browse_records,
            list_conversations,
            conversation_messages,
            prepare_continuation,
            prepare_selected_context,
            prepare_client_config,
            install_browser_connection,
            search_records,
            read_record,
            read_sources,
            event_location,
            read_background,
            read_background_page,
            background_memory,
            background_memory_source,
            review_background_change,
            confirm_background_change,
            manage_memories,
            managed_memory,
            managed_memory_source,
            review_memory_edit,
            review_memory_visibility,
            confirm_memory_change,
            pick_import,
            preview_import_selection,
            return_import_selection,
            preview_note,
            confirm_note,
            confirm_import,
            pick_dream,
            prepare_dream_task,
            review_dream_text,
            apply_dream,
            export_dream
        ])
        .run(tauri::generate_context!())
        .expect("RecallCard 桌面应用无法启动");
}
