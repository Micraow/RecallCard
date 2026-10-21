//! 本地界面服务：把一个资料库以 HTTP 交给浏览器。
//!
//! 只监听回环地址，首次访问必须带启动时打印的令牌，之后用 HttpOnly Cookie。
//! 这里没有后台轮询、没有自动联网：每个请求都由用户在页面上点出来。

use serde_json::{json, Value};
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use tiny_http::{Header, Method, Request, Response, Server};

use crate::desktop::DesktopSession;
use crate::model::Result;
use crate::policy::Access;
use crate::settings::Settings;
use crate::vault::Vault;
use crate::{dream_task, llm, semantic};

const MAX_BODY_BYTES: usize = 2 * 1024 * 1024;
const DREAM_SOURCE_LIMIT: usize = 64;

const DREAM_SYSTEM: &str = "你负责把用户与 AI 的历史对话整理成长期记忆。\
只输出一个 JSON 对象，不要任何解释、前后缀或代码块标记。\
可以写入的事实必须能在给定来源里找到依据；助手提出的建议要标为 assistant_suggestion。\
格式：{\"schema\":\"recallcard.dream-result/1\",\"scope\":\"<给定范围>\",\
\"memories\":[{\"content\":\"...\",\"evidence\":\"user_explicit|observed|assistant_suggestion\",\
\"source_refs\":[\"evt_...\"],\"time_note\":\"...\"}]}";

pub struct UiOptions {
    pub vault: PathBuf,
    pub scope: String,
    pub web_dir: PathBuf,
    pub port: u16,
    pub open_browser: bool,
}

struct Shared {
    state: Mutex<State>,
    token: String,
    web_dir: PathBuf,
    progress: Mutex<Progress>,
}

/// 索引重建的进度：界面轮询它画进度条，终端同时打日志。
#[derive(Default)]
struct Progress {
    running: bool,
    done: usize,
    total: usize,
    phase: String,
}

struct State {
    session: DesktopSession,
    session_id: String,
    scope: String,
}

/// 已经绑定但还没开始接收请求的服务。
pub struct Bound {
    server: Server,
    shared: Arc<Shared>,
    url: String,
    port: u16,
}

impl Bound {
    pub fn url(&self) -> &str {
        &self.url
    }
    pub fn port(&self) -> u16 {
        self.port
    }
    /// 访问令牌：只用于打印地址与测试，不参与判断分支。
    pub fn token(&self) -> &str {
        &self.shared.token
    }
}

/// 绑定端口并准备状态；与 `run` 分开，测试才能拿到真实端口。
pub fn bind(options: UiOptions) -> Result<Bound> {
    if !options.web_dir.is_dir() {
        return Err(format!(
            "界面文件不存在：{}；请先在 web 目录执行 pnpm install && pnpm build",
            options.web_dir.display()
        ));
    }
    let mut session = DesktopSession::default();
    let info = session.select_vault(&options.vault, false)?;
    let token = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    let shared = Arc::new(Shared {
        progress: Mutex::new(Progress::default()),
        state: Mutex::new(State {
            session,
            session_id: info.session_id,
            scope: options.scope,
        }),
        token: token.clone(),
        web_dir: options.web_dir,
    });
    let address = format!("127.0.0.1:{}", options.port);
    let server =
        Server::http(&address).map_err(|_| format!("无法监听 {address}；端口可能已被占用"))?;
    let port = server
        .server_addr()
        .to_ip()
        .map(|address| address.port())
        .ok_or_else(|| "无法确定监听端口".to_string())?;
    Ok(Bound {
        server,
        shared,
        url: format!("http://127.0.0.1:{port}/?token={token}"),
        port,
    })
}

/// 接收请求直到进程退出；每个请求独立线程，避免一次模型调用卡住整个界面。
pub fn run(bound: Bound) -> Result<()> {
    for request in bound.server.incoming_requests() {
        let shared = bound.shared.clone();
        thread::spawn(move || handle(shared, request));
    }
    Ok(())
}

/// 启动服务并阻塞。返回只在端口占用等启动失败时出现。
pub fn serve(options: UiOptions) -> Result<()> {
    let open = options.open_browser;
    let bound = bind(options)?;
    println!("{}", bound.url());
    if open {
        open_browser(bound.url());
    }
    run(bound)
}

/// 打开默认浏览器。打不开不算失败：终端里已经打印了地址。
pub fn open_browser(url: &str) {
    let candidates: [&[&str]; 3] = [&["xdg-open"], &["gio", "open"], &["sensible-browser"]];
    for candidate in candidates {
        let (program, arguments) = candidate.split_first().expect("候选命令至少有一个词");
        let started = std::process::Command::new(program)
            .args(arguments)
            .arg(url)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
        if started.is_ok() {
            return;
        }
    }
}

fn handle(shared: Arc<Shared>, mut request: Request) {
    let method = request.method().clone();
    let url = request.url().to_string();
    let (path, query) = match url.split_once('?') {
        Some((path, query)) => (path.to_string(), query.to_string()),
        None => (url.clone(), String::new()),
    };
    if !host_is_local(&request) {
        respond(request, 403, json!({"error": "只接受本机访问"}));
        return;
    }
    let cookie = cookie_token(&request);
    let query_token = query
        .split('&')
        .find_map(|pair| pair.strip_prefix("token="))
        .unwrap_or("");
    let authorized =
        cookie.as_deref() == Some(shared.token.as_str()) || query_token == shared.token;

    if !path.starts_with("/api/") {
        if !authorized {
            respond_text(
                request,
                401,
                "需要启动时打印的访问令牌；请用终端里给出的完整地址打开",
                None,
            );
            return;
        }
        let set_cookie = query_token == shared.token;
        serve_static(&shared, request, &path, set_cookie);
        return;
    }
    if !authorized {
        respond(
            request,
            401,
            json!({"error": "访问令牌无效，请用终端里打印的地址重新打开"}),
        );
        return;
    }
    let body = match read_body(&mut request) {
        Ok(body) => body,
        Err(error) => {
            respond(request, 413, json!({"error": error}));
            return;
        }
    };
    let readable = matches!(method, Method::Get | Method::Post);
    let started = std::time::Instant::now();
    let result = if readable {
        dispatch(&shared, &path, &body)
    } else {
        Err("不支持的请求方法".to_string())
    };
    match result {
        Ok(value) => {
            crate::log::done("界面", format!("{path} 成功"), started);
            respond(request, 200, value)
        }
        Err(error) => {
            crate::log::done("界面", format!("{path} 失败：{error}"), started);
            respond(request, 400, json!({ "error": error }))
        }
    }
}

fn dispatch(shared: &Shared, path: &str, body: &Value) -> Result<Value> {
    match path {
        "/api/state" => state(shared),
        "/api/scope" => set_scope(shared, body),
        "/api/search" => with_state(shared, |state| {
            let query = text(body, "query")?;
            let target = optional(body, "target", "all");
            let detail = optional(body, "detail", "context");
            let scope = state.scope.clone();
            let _ = detail;
            state
                .session
                .search(&state.session_id, &scope, &query, &target)
        }),
        "/api/browse" => with_state(shared, |state| {
            let target = optional(body, "target", "all");
            let scope = state.scope.clone();
            state.session.browse(&state.session_id, &scope, &target)
        }),
        "/api/read" => with_state(shared, |state| {
            let reference = text(body, "ref")?;
            let scope = state.scope.clone();
            state.session.read(&state.session_id, &scope, &reference)
        }),
        "/api/sources" => with_state(shared, |state| {
            let reference = text(body, "ref")?;
            let scope = state.scope.clone();
            state.session.sources(&state.session_id, &scope, &reference)
        }),
        "/api/memories" => with_state(shared, |state| {
            let offset = body.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize;
            let include_hidden = body.get("includeHidden") == Some(&Value::Bool(true));
            let scope = state.scope.clone();
            state
                .session
                .manage_memories(&state.session_id, &scope, include_hidden, offset)
        }),
        "/api/background" => with_state(shared, |state| {
            let scope = state.scope.clone();
            let page = state.session.read_background(&state.session_id, &scope)?;
            serde_json::to_value(page).map_err(|e| e.to_string())
        }),
        "/api/settings" => settings(shared, body),
        "/api/settings/reset" => reset_settings(shared),
        "/api/vault/open" => open_vault(shared, body),
        "/api/index/build" => build_index(shared),
        "/api/index/progress" => index_progress(shared),
        "/api/dream/sources" => dream_sources(shared, body),
        "/api/dream/preview" => dream_preview(shared, body),
        "/api/dream/apply" => dream_apply(shared, body),
        "/api/connect" => connect(shared),
        _ => Err("未知接口".to_string()),
    }
}

fn state(shared: &Shared) -> Result<Value> {
    with_state(shared, |state| {
        let info = state.session.status(&state.session_id)?;
        let vault = Vault::open(&PathBuf::from(&info.root))?;
        // 设置文件坏掉只影响设置页，不该让整个界面打不开。
        let (settings, settings_error) = match Settings::load(&vault) {
            Ok(settings) => (settings.public_view(), Value::Null),
            Err(error) => (Value::Null, Value::String(error)),
        };
        Ok(json!({
            "vault": info,
            "scope": state.scope,
            "index": semantic::index_status(&vault)?,
            "settings": settings,
            "settings_error": settings_error,
        }))
    })
}

/// 设置文件损坏时留一条出路：直接写回默认值，不需要用户去找文件。
fn reset_settings(shared: &Shared) -> Result<Value> {
    with_state(shared, |state| {
        let info = state.session.status(&state.session_id)?;
        let vault = Vault::open(&PathBuf::from(&info.root))?;
        let defaults = Settings::default();
        Settings::save(&vault, &defaults)?;
        Ok(defaults.public_view())
    })
}

fn set_scope(shared: &Shared, body: &Value) -> Result<Value> {
    let scope = text(body, "scope")?;
    with_state(shared, |state| {
        let info = state.session.status(&state.session_id)?;
        if !info.scopes.contains(&scope) {
            return Err("该范围不属于当前资料库".into());
        }
        state.scope = scope;
        Ok(json!({"ok": true}))
    })
}

fn open_vault(shared: &Shared, body: &Value) -> Result<Value> {
    let path = text(body, "path")?;
    let create = body.get("create") == Some(&Value::Bool(true));
    let mut guard = shared
        .state
        .lock()
        .map_err(|_| "服务状态异常".to_string())?;
    let info = guard.session.select_vault(Path::new(&path), create)?;
    guard.session_id = info.session_id.clone();
    guard.scope = info
        .scopes
        .first()
        .cloned()
        .unwrap_or_else(|| "personal".into());
    Ok(json!({"vault": info, "scope": guard.scope}))
}

fn index_progress(shared: &Shared) -> Result<Value> {
    let progress = shared.progress.lock().map_err(|_| "进度状态异常")?;
    Ok(json!({
        "running": progress.running,
        "done": progress.done,
        "total": progress.total,
        "phase": progress.phase,
    }))
}

fn set_progress(shared: &Shared, running: bool, done: usize, total: usize, phase: &str) {
    if let Ok(mut progress) = shared.progress.lock() {
        progress.running = running;
        progress.done = done;
        progress.total = total;
        progress.phase = phase.to_owned();
    }
}

fn build_index(shared: &Shared) -> Result<Value> {
    let (vault, config, scope) = with_state(shared, |state| {
        let info = state.session.status(&state.session_id)?;
        let vault = Vault::open(&PathBuf::from(&info.root))?;
        let settings = Settings::load(&vault)?;
        let config = provider_config(&vault, &settings)?;
        Ok((vault, config, state.scope.clone()))
    })?;
    let access = Access::new(vec![scope])?;
    let started = std::time::Instant::now();
    set_progress(shared, true, 0, 0, "读取语料");
    crate::log::note("索引", "开始重建");
    let mut on_progress = |done: usize, total: usize| {
        set_progress(shared, true, done, total, "编码");
    };
    let result =
        semantic::build_index_from_config_with_progress(&vault, access, &config, &mut on_progress);
    set_progress(shared, false, 0, 0, "");
    let result = result?;
    crate::log::done(
        "索引",
        format!(
            "重建完成：{} 条文档，新编码 {} 条",
            result["documents"], result["encoded"]
        ),
        started,
    );
    with_state(shared, |state| {
        let id = state.session_id.clone();
        state.session.reload_semantic_index(&id);
        Ok(())
    })?;
    Ok(result)
}

fn dream_sources(shared: &Shared, body: &Value) -> Result<Value> {
    let (since, until) = window(body)?;
    with_state(shared, |state| {
        let info = state.session.status(&state.session_id)?;
        let vault = Vault::open(&PathBuf::from(&info.root))?;
        let picked =
            dream_task::select_sources(&vault, &state.scope, since, until, DREAM_SOURCE_LIMIT)?;
        Ok(json!({
            "count": picked.len(),
            "total_bytes": picked.iter().map(|e| e.data.text().len()).sum::<usize>(),
            "samples": picked.iter().take(5).map(|e| json!({
                "ref": format!("event:{}", e.id),
                "role": e.data.role,
                "occurred_at": e.data.occurred_at,
                "text": crate::context::truncate_utf8(&e.data.text(), 160),
            })).collect::<Vec<_>>(),
            "llm_ready": {
                "base_url": !Settings::load(&vault)?.llm.base_url.is_empty(),
            },
        }))
    })
}

fn dream_preview(shared: &Shared, body: &Value) -> Result<Value> {
    let (since, until) = window(body)?;
    // 取来源与任务文本在锁内，模型调用在锁外，避免一次生成把界面卡满三分钟。
    let (task, scope, vault) = with_state(shared, |state| {
        let info = state.session.status(&state.session_id)?;
        let vault = Vault::open(&PathBuf::from(&info.root))?;
        let scope = state.scope.clone();
        let picked = dream_task::select_sources(&vault, &scope, since, until, DREAM_SOURCE_LIMIT)?;
        if picked.is_empty() {
            return Err("这个时间范围内没有尚未整理过的原始记录".into());
        }
        let refs: Vec<String> = picked
            .iter()
            .map(|event| format!("event:{}", event.id))
            .collect();
        let task = state
            .session
            .prepare_dream_task(&state.session_id, &scope, &refs, &[])?;
        Ok((task, scope, vault))
    })?;
    let settings = Settings::load(&vault)?;
    let prompt = format!(
        "授权范围：{scope}\n\n以下是完整整理任务，请按其中的规则输出 JSON：\n\n{}",
        task["text"].as_str().unwrap_or_default()
    );
    let reply = llm::complete(&settings.llm, DREAM_SYSTEM, &prompt)?;
    with_state(shared, |state| {
        let preview = state
            .session
            .review_dream_text(&state.session_id, &scope, &reply)?;
        Ok(json!({
            "preview": preview,
            "source_count": task["source_count"],
            "memory_count": task["memory_count"],
        }))
    })
}

fn dream_apply(shared: &Shared, body: &Value) -> Result<Value> {
    let preview_id = text(body, "previewId")?;
    let approve = body.get("approveProtected") == Some(&Value::Bool(true));
    let (receipt, scope) = with_state(shared, |state| {
        let receipt = state
            .session
            .apply_dream(&state.session_id, &preview_id, approve)?;
        Ok((receipt, state.scope.clone()))
    })?;
    // 新记忆会让旧索引过期；已有索引时就地补上，失败也不影响已经写入的结果。
    if let Ok(vault) = current_vault(shared) {
        if let Ok(access) = Access::new(vec![scope]) {
            if let Some(Err(error)) = semantic::refresh_if_present(&vault, access) {
                eprintln!("索引补建失败（不影响已写入的记忆）：{error}");
            }
        }
    }
    with_state(shared, |state| {
        let id = state.session_id.clone();
        state.session.reload_semantic_index(&id);
        Ok(())
    })?;
    Ok(json!({"receipt": receipt}))
}

fn connect(shared: &Shared) -> Result<Value> {
    let (root, scope) = with_state(shared, |state| {
        let info = state.session.status(&state.session_id)?;
        Ok((PathBuf::from(info.root), state.scope.clone()))
    })?;
    let binary = std::env::current_exe().map_err(|_| "无法确定程序位置".to_string())?;
    let config = crate::transport::connect_config(&binary, &root, &[scope], "claude-code")?;
    Ok(json!({ "config": config, "binary": binary }))
}

fn settings(shared: &Shared, body: &Value) -> Result<Value> {
    let vault = current_vault(shared)?;
    let current = Settings::load(&vault)?;
    if body.is_null() || body.as_object().is_none_or(|object| object.is_empty()) {
        return Ok(current.public_view());
    }
    let next = current.apply_patch(body)?;
    Settings::save(&vault, &next)?;
    Ok(next.public_view())
}

/// 把本机设置翻译成索引配置：云端要地址、模型、维度与密钥环境变量之外的信息。
/// 单次云端重建允许外发的正文字节上限（JSON 封装后的实际发送量）。
const CLOUD_INDEX_BUDGET: usize = 16 * 1024 * 1024;

fn provider_config(vault: &Vault, settings: &Settings) -> Result<semantic::SemanticConfig> {
    let index_path = semantic::vault_index_path(vault);
    if settings.embedding.provider == "cloud" {
        if settings.embedding.key.is_empty() {
            return Err("云端编码需要密钥；请到设置页填写".into());
        }
        return Ok(semantic::SemanticConfig {
            provider: semantic::Provider::Cloud {
                endpoint: settings.embedding.endpoint.clone(),
                model: settings.embedding.model.clone(),
                dimensions: settings.embedding.dimensions,
                key_env: "RECALLCARD_EMBEDDING_KEY".into(),
                timeout_ms: 30_000,
                // 一次重建是整库外发，上限按"够用但不放任"给：超了会报出实际字节数，
                // 不会静默只发一半。
                max_transmitted_bytes: CLOUD_INDEX_BUDGET,
                key: Some(settings.embedding.key.clone()),
            },
            index_path,
        });
    }
    Ok(semantic::SemanticConfig {
        provider: semantic::Provider::Local,
        index_path,
    })
}

fn current_vault(shared: &Shared) -> Result<Vault> {
    with_state(shared, |state| {
        let info = state.session.status(&state.session_id)?;
        Vault::open(&PathBuf::from(&info.root))
    })
}

fn with_state<T>(shared: &Shared, action: impl FnOnce(&mut State) -> Result<T>) -> Result<T> {
    let mut guard = shared
        .state
        .lock()
        .map_err(|_| "服务状态异常".to_string())?;
    action(&mut guard)
}

fn text(body: &Value, key: &str) -> Result<String> {
    body.get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| format!("缺少参数 {key}"))
}

fn optional(body: &Value, key: &str, fallback: &str) -> String {
    body.get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .unwrap_or(fallback)
        .to_owned()
}

/// 整理时间窗：两端都可以省略，缺省表示不限。
type Window = (
    Option<chrono::DateTime<chrono::Utc>>,
    Option<chrono::DateTime<chrono::Utc>>,
);

fn window(body: &Value) -> Result<Window> {
    let parse = |key: &str| -> Result<Option<chrono::DateTime<chrono::Utc>>> {
        match body
            .get(key)
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty())
        {
            Some(value) => chrono::DateTime::parse_from_rfc3339(value)
                .map(|time| Some(time.with_timezone(&chrono::Utc)))
                .map_err(|_| format!("{key} 不是有效时间")),
            None => Ok(None),
        }
    };
    Ok((parse("since")?, parse("until")?))
}

fn read_body(request: &mut Request) -> Result<Value> {
    let mut buffer = Vec::new();
    request
        .as_reader()
        .take(MAX_BODY_BYTES as u64 + 1)
        .read_to_end(&mut buffer)
        .map_err(|_| "读取请求失败".to_string())?;
    if buffer.len() > MAX_BODY_BYTES {
        return Err("请求体过大".into());
    }
    if buffer.is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_slice(&buffer).map_err(|_| "请求体不是合法 JSON".to_string())
}

fn host_is_local(request: &Request) -> bool {
    request
        .headers()
        .iter()
        .find(|header| header.field.equiv("Host"))
        .map(|header| {
            let value = header.value.as_str();
            value.starts_with("127.0.0.1:") || value.starts_with("localhost:")
        })
        .unwrap_or(false)
}

fn cookie_token(request: &Request) -> Option<String> {
    request
        .headers()
        .iter()
        .filter(|header| header.field.equiv("Cookie"))
        .find_map(|header| {
            header.value.as_str().split(';').find_map(|pair| {
                let pair = pair.trim();
                pair.strip_prefix("rc_token=").map(str::to_owned)
            })
        })
}

fn serve_static(shared: &Shared, request: Request, path: &str, set_cookie: bool) {
    let relative = path.trim_start_matches('/');
    let relative = if relative.is_empty() {
        "index.html"
    } else {
        relative
    };
    let candidate = match safe_join(&shared.web_dir, relative) {
        Some(path) => path,
        None => {
            respond_text(request, 400, "路径无效", None);
            return;
        }
    };
    let file = if candidate.is_file() {
        candidate
    } else {
        shared.web_dir.join("index.html")
    };
    let bytes = match std::fs::read(&file) {
        Ok(bytes) => bytes,
        Err(_) => {
            respond_text(request, 404, "界面文件缺失", None);
            return;
        }
    };
    let content_type = match file.extension().and_then(|value| value.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("json") => "application/json",
        Some("woff2") => "font/woff2",
        Some("png") => "image/png",
        _ => "application/octet-stream",
    };
    let mut response = Response::from_data(bytes).with_header(header("Content-Type", content_type));
    if set_cookie {
        response = response.with_header(header(
            "Set-Cookie",
            &format!(
                "rc_token={}; HttpOnly; SameSite=Strict; Path=/",
                shared.token
            ),
        ));
    }
    let _ = request.respond(response.with_header(header("Cache-Control", "no-store")));
}

fn safe_join(root: &Path, relative: &str) -> Option<PathBuf> {
    let path = Path::new(relative);
    if path
        .components()
        .any(|part| !matches!(part, Component::Normal(_)))
    {
        return None;
    }
    Some(root.join(path))
}

fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name.as_bytes(), value.as_bytes()).expect("静态响应头必须合法")
}

fn respond(request: Request, status: u16, body: Value) {
    let text = body.to_string();
    let response = Response::from_string(text)
        .with_status_code(status)
        .with_header(header("Content-Type", "application/json; charset=utf-8"))
        .with_header(header("Cache-Control", "no-store"));
    let _ = request.respond(response);
}

fn respond_text(request: Request, status: u16, body: &str, cookie: Option<&str>) {
    let mut response = Response::from_string(body.to_owned())
        .with_status_code(status)
        .with_header(header("Content-Type", "text/plain; charset=utf-8"));
    if let Some(cookie) = cookie {
        response = response.with_header(header("Set-Cookie", cookie));
    }
    let _ = request.respond(response);
}
