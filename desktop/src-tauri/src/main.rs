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
) -> Result<Value, String> {
    execute(state.service.clone(), move |s| {
        s.continuation(&session_id, &scope, &conversation_ref, &goal)
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
#[tauri::command]
async fn prepare_client_config(
    app: AppHandle,
    state: State<'_, AppState>,
    session_id: String,
    scope: String,
) -> Result<Value, String> {
    let bundled = packaged_cli(&app)?;
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
    execute(state.service.clone(),move|s|{
        let info=s.status(&session_id)?;
        recallcard::policy::Access::new(vec![scope.clone()])?;
        Ok(serde_json::json!({"mcpServers":{"recallcard":{"command":binary,"args":["--vault",info.root,"mcp","--scope",scope]}}}))
    }).await
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
            choose_vault,
            write_clipboard,
            cancel_previews,
            vault_status,
            browse_records,
            list_conversations,
            conversation_messages,
            prepare_continuation,
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
