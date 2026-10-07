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
use tauri::{AppHandle, State};
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
        if create && !app.dialog().message(format!("在此文件夹创建 RecallCard 资料库？\n\n{}\n\n将创建 Event、Memory 等资料目录。现有文件不会被覆盖。", path.display())).title("创建资料库").buttons(MessageDialogButtons::OkCancelCustom("创建资料库".into(), "取消".into())).blocking_show() { return Ok(None); }
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
async fn pick_import(
    app: AppHandle,
    state: State<'_, AppState>,
    session_id: String,
    scope: String,
    format: String,
) -> Result<Option<recallcard::desktop::ImportPreview>, String> {
    let guard = dialog_guard(&state)?;
    let path = tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        app.dialog()
            .file()
            .set_title("选择要导入的对话文件")
            .add_filter("对话导出", &["json", "jsonl"])
            .blocking_pick_file()
    })
    .await
    .map_err(|_| "文件选择未完成")?;
    let Some(path) = path else {
        return Ok(None);
    };
    let path = path.into_path().map_err(|_| "请选择本机文件")?;
    execute(state.service.clone(), move |s| {
        s.preview_import(&session_id, &format, &path, &scope)
            .map(Some)
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
            .set_title("选择 Dream 整理结果")
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
            .set_title("保存本次整理的来源包")
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
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            choose_vault,
            cancel_previews,
            vault_status,
            browse_records,
            search_records,
            read_record,
            read_sources,
            pick_import,
            confirm_import,
            pick_dream,
            apply_dream,
            export_dream
        ])
        .run(tauri::generate_context!())
        .expect("RecallCard 桌面应用无法启动");
}
