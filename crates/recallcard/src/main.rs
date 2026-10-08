use clap::{Parser, Subcommand, ValueEnum};
use recallcard::{
    context::{parse_ref, BootstrapArgs, Context, ReadArgs, SearchArgs},
    policy::Access,
    EventInput, MemoryInput, MemoryState, Result, Vault,
};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use std::{
    io::{Read, Write},
    path::PathBuf,
};

#[derive(Parser)]
#[command(
    name = "recallcard",
    version,
    about = "本地优先、可追溯的个人 AI 上下文与记忆层"
)]
struct Cli {
    /// 本地 Vault 目录（不会自动上传到任何服务）
    #[arg(long, default_value = "vault", global = true)]
    vault: PathBuf,
    /// 新应用命令输出版本化 JSON，stdout 不混入进度日志
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    /// 首次准备本地资料库；模型与凭据可在桌面的一次设置中连接
    Setup,

    /// 创建本地事实源目录
    #[command(hide = true)]
    Init,
    /// 保存一条原始事件；文件为 - 时从标准输入读取 JSON
    #[command(hide = true)]
    Capture {
        #[arg(long)]
        file: String,
    },
    /// 导入完整官方导出；旧 --file / manual-jsonl 入口继续兼容
    Import {
        #[arg(value_name = "PATH", conflicts_with = "file")]
        paths: Vec<PathBuf>,
        #[arg(long, default_value = "auto")]
        format: String,
        #[arg(long)]
        file: Option<String>,
        #[arg(long, default_value = "personal")]
        scope: String,
        #[arg(long)]
        request_id: Option<String>,
    },
    /// 启动共同后台服务；默认独立进程，不依赖 GUI
    Start {
        #[arg(long, conflicts_with = "once")]
        foreground: bool,
        #[arg(long)]
        once: bool,
        #[arg(long, hide = true, requires = "foreground")]
        credential_stdin: bool,
    },
    /// 查看或停止共同后台服务
    Service {
        #[command(subcommand)]
        action: ServiceCommand,
    },
    /// 查看与控制导入及记忆整理任务
    Jobs {
        #[command(subcommand)]
        action: Option<JobsCommand>,
    },
    /// 一次配置后台模型、目的地授权与预算；不接收明文密钥
    Background {
        #[command(subcommand)]
        action: BackgroundCommand,
    },
    /// 查看真实连接配置、授权和最近握手状态
    Connections,
    /// 预览并确认项目内 Agent 入口；不执行外部 Agent、不修改全局配置
    ConnectionSetup {
        #[command(subcommand)]
        action: ConnectionSetupCommand,
    },
    /// 自动刷新按连接过滤的文件背景；供 Agent 入口直接调用
    ConnectionContext {
        id: String,
        #[arg(long, required = true)]
        scope: Vec<String>,
        #[arg(long, default_value_t = 4096)]
        budget_bytes: usize,
    },
    /// 检查配置及实际本机读取，不能代替宿主账号验证
    ConnectionCheck {
        id: String,
        #[arg(long)]
        scope: String,
        #[arg(long)]
        verify: bool,
    },
    /// 明确配置一个客户端的读取和捕获范围，不自动授予网页发送权
    Connect {
        #[arg(value_parser=["browser","claude-code","codex","chatgpt-mcp"])]
        client: String,
        #[arg(long)]
        host_identity: String,
        #[arg(long)]
        platform: String,
        #[arg(long)]
        installation_id: Option<String>,
        #[arg(long)]
        recall_scope: Vec<String>,
        #[arg(long)]
        capture_scope: Vec<String>,
        #[arg(long)]
        provider_disclosure: bool,
        #[arg(long)]
        auto_capture: bool,
        #[arg(long)]
        auto_recall: bool,
        #[arg(long)]
        expected_revision: Option<u64>,
    },
    /// 撤销指定连接的精确权限版本
    ConnectionRevoke {
        id: String,
        #[arg(long)]
        expected_revision: u64,
    },
    /// 手工 Dream：导出、预览、按摘要批准与恢复
    #[command(hide = true)]
    Dream {
        #[command(subcommand)]
        action: DreamCommand,
    },
    /// 以浏览器原生消息协议提供受限只读访问
    #[command(hide = true)]
    NativeHost {
        #[arg(long, required = true)]
        scope: Vec<String>,
        #[arg(long)]
        allowed_extension: String,
        origin: String,
        #[arg(long)]
        parent_window: Option<u64>,
        #[arg(long)]
        ipc_endpoint: Option<PathBuf>,
    },
    /// 生成待人工检查/注册的 Native Messaging 文件
    #[command(hide = true)]
    NativeInstall {
        /// 明确允许扩展在此范围内保存用户预览确认的可见对话；默认不允许写入
        #[arg(long)]
        capture_scope: Option<String>,
        #[arg(long, required = true)]
        scope: Vec<String>,
        #[arg(long)]
        extension_id: String,
        #[arg(long)]
        output_dir: PathBuf,
        #[arg(long)]
        ipc_endpoint: Option<PathBuf>,
    },
    /// 人工管理长期记忆
    Memory {
        #[command(subcommand)]
        action: MemoryCommand,
    },
    /// 通过编号读取原始事件或记忆
    Read {
        id: String,
        /// 具名 View 或有界续读需要明确授权范围
        #[arg(long)]
        scope: Vec<String>,
        /// 输出 UTF-8 字节预算；与 --scope 一起使用有界读取
        #[arg(long, alias = "budget-tokens")]
        budget_bytes: Option<usize>,
        /// 按 search 返回的 text_range.start_byte 定位正文
        #[arg(long)]
        offset_bytes: Option<usize>,
        /// 使用上一页返回的不透明 next_cursor 继续读取
        #[arg(long)]
        cursor: Option<String>,
    },
    /// 返回原始证据；大记录可按固定范围有界续读
    Sources {
        id: String,
        #[arg(long)]
        scope: Vec<String>,
        #[arg(long, alias = "budget-tokens")]
        budget_bytes: Option<usize>,
        #[arg(long)]
        offset_bytes: Option<usize>,
        #[arg(long)]
        cursor: Option<String>,
    },
    /// 按当前授权/抑制状态导出可选Embedding输入；不会发送云端
    #[command(hide = true)]
    EmbeddingExport {
        #[arg(long, required = true)]
        scope: Vec<String>,
    },
    /// 校验并先提交本地正本，再整合与推送指定 Git 远端
    #[command(hide = true)]
    Sync {
        #[arg(long)]
        remote: String,
    },
    /// 查看共同服务、后台配置和版本；--git 保留旧 Git 状态入口
    Status {
        #[arg(long)]
        git: bool,
    },
    /// 离线重建文本快照与视图，不重新调用模型
    #[command(hide = true)]
    Rebuild,
    /// 根据 canonical Memory 重新生成人类视图
    #[command(hide = true)]
    Views,
    /// 检查 schema、摘要与证据完整性
    Doctor,
    /// 为 AI 读取固定范围的启动背景；新信息仍可通过 search 查找
    Bootstrap {
        #[arg(long, required = true)]
        scope: Vec<String>,
        /// 返回内容的 UTF-8 字节上限；不是模型 token 计数
        #[arg(long = "budget-bytes", alias = "budget-tokens", default_value_t = 1500)]
        budget_tokens: usize,
    },
    /// 不依赖 Dream 或云 API 的中英文文本检索
    Search {
        query: String,
        #[arg(long, default_value = "personal")]
        scope: Vec<String>,
        #[arg(long, default_value = "all")]
        target: String,
        #[arg(long, default_value_t = 5)]
        limit: usize,
        /// 返回内容的 UTF-8 字节上限；不是模型 token 计数
        #[arg(long = "budget-bytes", alias = "budget-tokens", default_value_t = 1500)]
        budget_tokens: usize,
        #[arg(long)]
        session_ref: Option<String>,
        #[arg(long)]
        as_of: Option<chrono::DateTime<chrono::Utc>>,
        #[arg(long, default_value = "context")]
        detail: String,
        #[arg(long)]
        cursor: Option<String>,
        /// 由本机维护者固定的可选语义 worker 配置；默认不启用
        #[arg(long)]
        semantic_config: Option<PathBuf>,
    },
    /// 启动 AI 只读 MCP 入口；资料库和授权范围在启动时固定
    Mcp {
        #[arg(long)]
        connection_id: Option<String>,
        #[arg(long, required = true)]
        scope: Vec<String>,
        #[arg(long)]
        ipc_endpoint: Option<PathBuf>,
        #[arg(long, conflicts_with = "ipc_endpoint")]
        semantic_config: Option<PathBuf>,
    },
    /// 启动固定资料库和范围的只读本机 IPC 服务
    #[command(hide = true)]
    Daemon {
        #[arg(long, required = true)]
        scope: Vec<String>,
        #[arg(long)]
        endpoint: Option<PathBuf>,
        #[arg(long, default_value_t = 5000)]
        timeout_ms: u64,
        #[arg(long, default_value_t = 16)]
        max_connections: usize,
        #[arg(long)]
        semantic_config: Option<PathBuf>,
    },
    /// 只读响应 Claude Code 启动/恢复/压缩生命周期，不运行外部 Agent
    #[command(hide = true)]
    AgentHook {
        #[arg(long)]
        connection_id: Option<String>,
        #[arg(long, required = true)]
        scope: Vec<String>,
        /// 返回内容的 UTF-8 字节上限；不是模型 token 计数
        #[arg(long = "budget-bytes", alias = "budget-tokens", default_value_t = 1500)]
        budget_tokens: usize,
    },
    /// 抑制记录及其原始证据，防止检索与下一次 Dream 再次提炼
    #[command(hide = true)]
    Forget {
        id: String,
        #[arg(long)]
        reason: String,
    },
    /// 显式撤销一条抑制规则
    #[command(hide = true)]
    Restore { id: String },
}
#[derive(Subcommand)]
enum ServiceCommand {
    Status,
    Stop,
}
#[derive(Subcommand)]
enum ConnectionSetupCommand {
    /// 只预览将合并的项目文件与一次授权范围
    Plan {
        #[arg(value_parser = ["codex", "claude_code"])]
        client: String,
        #[arg(long)]
        scope: String,
        #[arg(long)]
        project_dir: PathBuf,
    },
    /// 明确确认预览中的文件变更与读取授权；必须使用仍有效的计划编号
    Apply { plan_id: String },
}
#[derive(Subcommand)]
enum JobsCommand {
    List {
        #[arg(long, default_value = "personal")]
        scope: String,
    },
    Status {
        id: String,
        #[arg(long, default_value = "personal")]
        scope: String,
    },
    Pause {
        id: String,
        #[arg(long, default_value = "personal")]
        scope: String,
    },
    Resume {
        id: String,
        #[arg(long, default_value = "personal")]
        scope: String,
    },
    Retry {
        id: String,
        #[arg(long, default_value = "personal")]
        scope: String,
    },
    Cancel {
        id: String,
        #[arg(long, default_value = "personal")]
        scope: String,
    },
}
#[derive(Subcommand)]
enum BackgroundCommand {
    Status {
        #[arg(long, default_value = "personal")]
        scope: String,
    },
    Configure {
        #[arg(long)]
        file: String,
    },
    Pause,
    Resume,
}

#[derive(Subcommand)]
enum DreamCommand {
    /// 导出明确选择的有界来源；不自动发送任何云端
    Export {
        #[arg(long, required = true)]
        source: Vec<String>,
        #[arg(long)]
        memory: Vec<String>,
        #[arg(long)]
        scope: String,
    },
    /// 校验结果并显示逐条差异，不发布
    Review {
        #[arg(long)]
        file: String,
    },
    /// 按 review 返回的摘要显式批准发布
    Apply {
        #[arg(long)]
        file: String,
        #[arg(long)]
        approve: String,
        #[arg(long)]
        approve_protected: bool,
    },
    /// 恢复中断的本机多文件事务，冲突时停止
    Recover,
}
#[derive(Subcommand)]
enum MemoryCommand {
    /// 从已审核的 JSON 添加一条记忆
    Add {
        #[arg(long)]
        file: String,
    },
    /// 乐观锁更新，要求指定刚刚读取的版本
    Update {
        id: String,
        #[arg(long)]
        revision: u64,
        #[arg(long)]
        file: String,
    },
    /// 标记失效、撤回或显式恢复记忆
    State {
        id: String,
        #[arg(long)]
        revision: u64,
        #[arg(value_enum)]
        state: State,
    },
}
#[derive(Clone, ValueEnum)]
enum State {
    Active,
    Tentative,
    Superseded,
    Retracted,
}

fn input_text(file: &str) -> Result<String> {
    let mut text = String::new();
    if file == "-" {
        std::io::stdin()
            .take(16 * 1024 * 1024 + 1)
            .read_to_string(&mut text)
            .map_err(|e| e.to_string())?;
    } else {
        std::fs::File::open(file)
            .map_err(|e| e.to_string())?
            .take(16 * 1024 * 1024 + 1)
            .read_to_string(&mut text)
            .map_err(|e| e.to_string())?;
    }
    if text.len() > 16 * 1024 * 1024 {
        return Err("输入不能超过 16 MiB".into());
    }
    Ok(text)
}
fn input<T: DeserializeOwned>(file: &str) -> Result<T> {
    serde_json::from_str(&input_text(file)?).map_err(|e| format!("输入 JSON 无效：{e}"))
}
fn value<T: serde::Serialize>(v: T) -> Result<Value> {
    serde_json::to_value(v).map_err(|e| e.to_string())
}
fn run(cli: Cli) -> Result<Value> {
    if matches!(cli.command, Command::Init) {
        let vault = Vault::init(&cli.vault)?;
        return Ok(json!({"ok":true, "vault":vault.root(), "schema_version":1}));
    }
    let vault = Vault::open(&cli.vault)?;
    match cli.command {
        Command::Init => unreachable!(),
        Command::Setup
        | Command::Start { .. }
        | Command::Service { .. }
        | Command::Jobs { .. }
        | Command::Background { .. }
        | Command::Connections
        | Command::ConnectionSetup { .. }
        | Command::ConnectionContext { .. }
        | Command::ConnectionCheck { .. }
        | Command::Connect { .. }
        | Command::ConnectionRevoke { .. } => Err("请通过共同应用服务调用此命令".into()),
        Command::Capture { file } => value(vault.capture(input::<EventInput>(&file)?)?),
        Command::Import {
            format,
            file,
            scope,
            ..
        } => {
            let file = file.ok_or("请提供导出路径或 --file；官方导出可使用 import <path>")?;
            let parsed = if file == "-" {
                let text = input_text(&file)?;
                recallcard::import_bundle::parse_import_bytes(&format, text.as_bytes(), &scope)?
            } else {
                recallcard::import_bundle::read_import_file(
                    std::path::Path::new(&file),
                    &format,
                    &scope,
                )?
            };
            let before = vault.events()?.len();
            let refs: Vec<_> = vault
                .capture_batch(parsed.events)?
                .into_iter()
                .map(|event| format!("event:{}", event.id))
                .collect();
            let after = vault.events()?.len();
            Ok(
                json!({"ok":true,"events_added":after.saturating_sub(before),"events_seen":refs.len(),"refs":refs,"coverage":parsed.coverage,
                "note":"仅导入显式提供的文件；导入中断可安全重复运行，已写原始事件不回滚"}),
            )
        }
        Command::NativeInstall {
            capture_scope,
            scope,
            extension_id,
            output_dir,
            ipc_endpoint,
        } => recallcard::native::prepare_install_from_binary(
            &vault,
            scope,
            &extension_id,
            &output_dir,
            ipc_endpoint,
            capture_scope,
            &std::env::current_exe().map_err(|e| e.to_string())?,
        ),
        Command::NativeHost { .. } => Err("Native host 必须使用 framed stdio 模式".into()),
        Command::Dream { action } => match action {
            DreamCommand::Export {
                source,
                memory,
                scope,
            } => value(vault.dream_export(&source, &memory, &scope)?),
            DreamCommand::Review { file } => {
                value(vault.dream_review(&input::<recallcard::dream::DreamResult>(&file)?)?)
            }
            DreamCommand::Apply {
                file,
                approve,
                approve_protected,
            } => value(vault.dream_apply(
                &input::<recallcard::dream::DreamResult>(&file)?,
                &approve,
                approve_protected,
            )?),
            DreamCommand::Recover => vault.dream_recover(),
        },
        Command::Memory { action } => match action {
            MemoryCommand::Add { file } => {
                let mut memory = input::<MemoryInput>(&file)?;
                memory.authority = "user".into();
                value(vault.add_memory(memory)?)
            }
            MemoryCommand::Update { id, revision, file } => {
                let mut memory = input::<MemoryInput>(&file)?;
                memory.authority = "user".into();
                value(vault.update_memory(&id, revision, memory)?)
            }
            MemoryCommand::State {
                id,
                revision,
                state,
            } => value(vault.set_state(
                &id,
                revision,
                match state {
                    State::Active => MemoryState::Active,
                    State::Tentative => MemoryState::Tentative,
                    State::Superseded => MemoryState::Superseded,
                    State::Retracted => MemoryState::Retracted,
                },
            )?),
        },
        Command::Read {
            id,
            scope,
            budget_bytes,
            offset_bytes,
            cursor,
        } => {
            if budget_bytes.is_some() || offset_bytes.is_some() || cursor.is_some() {
                if scope.is_empty() {
                    return Err("有界读取需要明确 --scope；请使用授权范围".into());
                }
                return Context::new(&vault, Access::new(scope)?).read_page(
                    recallcard::context::ReadPageArgs {
                        refs: vec![id],
                        budget_tokens: budget_bytes.unwrap_or(1500),
                        offset_bytes,
                        cursor,
                    },
                );
            }
            let _read_guard = vault.read_guard()?;
            let (kind, record_id, revision) = parse_ref(&id)?;
            if kind == "view" {
                return Context::new(&vault, Access::new(scope)?).read(ReadArgs {
                    refs: vec![id],
                    budget_tokens: 32768,
                });
            }
            ensure_current_visible(&vault, record_id, revision)?;
            if !scope.is_empty() {
                Context::new(&vault, Access::new(scope)?).read(ReadArgs {
                    refs: vec![id.clone()],
                    budget_tokens: 32768,
                })?;
            }
            vault.read(record_id)
        }
        Command::Sources {
            id,
            scope,
            budget_bytes,
            offset_bytes,
            cursor,
        } => {
            if budget_bytes.is_some()
                || offset_bytes.is_some()
                || cursor.is_some()
                || !scope.is_empty()
            {
                if scope.is_empty() {
                    return Err("有界来源读取需要明确 --scope；请使用授权范围".into());
                }
                return Context::new(&vault, Access::new(scope)?).sources_page(
                    recallcard::context::ReadPageArgs {
                        refs: vec![id],
                        budget_tokens: budget_bytes.unwrap_or(1500),
                        offset_bytes,
                        cursor,
                    },
                );
            }
            let _read_guard = vault.read_guard()?;
            let (kind, record_id, revision) = parse_ref(&id)?;
            if kind == "view" {
                return Err("sources 接受 Event/Memory 引用；View 请使用 read --scope".into());
            }
            ensure_current_visible(&vault, record_id, revision)?;
            if kind == "event" {
                value(vec![vault.event(record_id)?])
            } else {
                value(vault.sources(record_id)?)
            }
        }
        Command::EmbeddingExport { scope } => {
            Context::new(&vault, Access::new(scope)?).embedding_corpus()
        }
        Command::Rebuild => vault.rebuild(),
        Command::Sync { remote } => vault.sync(&remote),
        Command::Status { .. } => vault.git_status(),
        Command::Views => Ok(json!({"ok":true,"memories":vault.rebuild_views()?})),
        Command::Doctor => vault.doctor(),
        Command::Bootstrap {
            scope,
            budget_tokens,
        } => Context::new(&vault, Access::new(scope)?).bootstrap(BootstrapArgs { budget_tokens }),
        Command::Search {
            query,
            scope,
            target,
            limit,
            budget_tokens,
            session_ref,
            as_of,
            detail,
            cursor,
            semantic_config,
        } => {
            let semantic = semantic_config
                .as_ref()
                .map(|path| recallcard::semantic::SemanticSearch::from_config_file(path))
                .transpose()?;
            let access = Access::new(scope)?;
            let context = match semantic.as_ref() {
                Some(semantic) => Context::with_semantic(&vault, access, semantic),
                None => Context::new(&vault, access),
            };
            context.search(SearchArgs {
                query,
                target,
                session_ref,
                as_of,
                limit,
                detail,
                budget_tokens,
                cursor,
            })
        }
        Command::Mcp { .. } => Err("MCP 必须以 stdio 模式启动".into()),
        Command::Daemon {
            scope,
            endpoint,
            timeout_ms,
            max_connections,
            semantic_config,
        } => {
            let access = Access::new(scope)?;
            let endpoint = match endpoint {
                Some(endpoint) => endpoint,
                None => recallcard::ipc::default_endpoint(&vault, &access)?,
            };
            let mut server = recallcard::ipc::Server::bind(
                vault,
                access,
                endpoint.clone(),
                recallcard::ipc::Options {
                    request_timeout: std::time::Duration::from_millis(timeout_ms),
                    max_connections,
                },
            )?;
            if let Some(path) = semantic_config {
                server = server.with_semantic(std::sync::Arc::new(
                    recallcard::semantic::SemanticSearch::from_config_file(&path)?,
                ));
            }
            eprintln!("RecallCard 本机只读服务：{}", endpoint.display());
            server.run()?;
            Ok(json!({"ok":true}))
        }
        Command::AgentHook {
            scope,
            budget_tokens,
            connection_id,
        } => {
            let access = Access::new(scope)?;
            if let Some(id) = connection_id {
                recallcard::agent_hook::session_start_connected(
                    &vault,
                    &id,
                    access,
                    budget_tokens,
                    std::io::stdin().lock(),
                )
            } else {
                recallcard::agent_hook::session_start(
                    &vault,
                    access,
                    budget_tokens,
                    std::io::stdin().lock(),
                )
            }
        }
        Command::Forget { id, reason } => value(vault.suppress(&id, reason)?),
        Command::Restore { id } => value(vault.restore(&id)?),
    }
}
fn main() {
    if let Some(result) = recallcard::native::dispatch_launcher() {
        if let Err(error) = result {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return;
    }
    let cli = Cli::parse();
    if let Command::NativeHost {
        scope,
        allowed_extension,
        origin,
        ipc_endpoint,
        ..
    } = &cli.command
    {
        let result = Vault::open(&cli.vault).and_then(|vault| {
            let access = Access::new(scope.clone())?;
            if let Some(endpoint) = ipc_endpoint {
                let client = recallcard::ipc::Client::new(
                    &vault,
                    &access,
                    endpoint.clone(),
                    std::time::Duration::from_secs(5),
                )?;
                return recallcard::native::serve_native_read_service_io(
                    &vault,
                    access,
                    allowed_extension,
                    origin,
                    |name, args| client.invoke(name, args),
                    std::io::stdin().lock(),
                    std::io::stdout().lock(),
                );
            }
            recallcard::native::serve_native(&vault, access, allowed_extension, origin)
        });
        if let Err(error) = result {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return;
    }
    if let Command::Mcp {
        scope,
        ipc_endpoint,
        semantic_config,
        connection_id,
    } = &cli.command
    {
        let result = Vault::open(&cli.vault).and_then(|vault| {
            let access = Access::new(scope.clone())?;
            if let Some(id)=connection_id {
                if ipc_endpoint.is_some()||semantic_config.is_some(){return Err("当前受连接授权管理的 MCP 尚不支持同时配置 IPC/语义代理；请移除该组合，不会忽略撤权".into());}
                return recallcard::transport::serve_mcp_service_io(|name,args|recallcard::application::connections::invoke_agent(&vault,id,&access,name,args),std::io::stdin().lock(),std::io::stdout().lock());
            }
            if let Some(endpoint) = ipc_endpoint {
                let client = recallcard::ipc::Client::new(
                    &vault,
                    &access,
                    endpoint.clone(),
                    std::time::Duration::from_secs(5),
                )?;
                return recallcard::transport::serve_mcp_service_io(
                    |name, args| client.invoke(name, args),
                    std::io::stdin().lock(),
                    std::io::stdout().lock(),
                );
            }
            if let Some(path) = semantic_config {
                let semantic = recallcard::semantic::SemanticSearch::from_config_file(path)?;
                let context = Context::with_semantic(&vault, access, &semantic);
                return recallcard::transport::serve_mcp_service_io(
                    |name, args| recallcard::transport::invoke(&context, name, args),
                    std::io::stdin().lock(),
                    std::io::stdout().lock(),
                );
            }
            recallcard::transport::serve_mcp(&vault, access)
        });
        if let Err(error) = result {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return;
    }
    if let Some(result) = run_application(&cli) {
        match result {
            Ok(value) => {
                let output = if cli.json {
                    serde_json::to_string(
                        &json!({"schema":"recallcard.cli/1","ok":true,"result":value}),
                    )
                    .expect("可序列化结果")
                } else {
                    human_application_result(&value)
                };
                if let Err(error) = writeln!(std::io::stdout().lock(), "{output}") {
                    if error.kind() != std::io::ErrorKind::BrokenPipe {
                        std::process::exit(1);
                    }
                }
            }
            Err(error) => {
                let code = error.exit_code();
                eprintln!(
                    "{}",
                    json!({"schema":"recallcard.cli/1","ok":false,"error":error})
                );
                std::process::exit(code);
            }
        }
        return;
    }
    let machine_output = cli.json;
    match run(cli) {
        Ok(result) => {
            let result = if machine_output {
                serde_json::to_string(&result)
            } else {
                serde_json::to_string_pretty(&result)
            }
            .expect("可序列化的输出");
            if let Err(error) = writeln!(std::io::stdout().lock(), "{result}") {
                if error.kind() != std::io::ErrorKind::BrokenPipe {
                    eprintln!("输出失败：{error}");
                    std::process::exit(1);
                }
            }
        }
        Err(error) => {
            eprintln!("{}", json!({"ok":false,"error":error}));
            std::process::exit(1);
        }
    }
}

fn run_application(cli: &Cli) -> Option<recallcard::application::AppResult<Value>> {
    use recallcard::application::{
        background_memory::{MemoryConfig, MemoryJobControl, MemoryRuntime},
        runtime, AppError, AppResult, ErrorCode, ImportRequest, ImportService, JobState,
    };
    // 位置参数是完整归档的新应用入口；显式 --file 保留既有 JSON/标准输入合同。
    let modern_import = matches!(&cli.command, Command::Import { paths, .. } if !paths.is_empty());
    if !modern_import
        && !matches!(
            cli.command,
            Command::Setup
                | Command::Status { git: false }
                | Command::Start { .. }
                | Command::Service { .. }
                | Command::Jobs { .. }
                | Command::Background { .. }
                | Command::Connections
                | Command::ConnectionSetup { .. }
                | Command::ConnectionContext { .. }
                | Command::ConnectionCheck { .. }
                | Command::Connect { .. }
                | Command::ConnectionRevoke { .. }
        )
    {
        return None;
    }
    Some((|| -> AppResult<Value> {
        let opened = if matches!(cli.command, Command::Setup)
            && !cli.vault.join("control/schema-version.json").exists()
        {
            if cli.vault.exists()
                && std::fs::read_dir(&cli.vault)
                    .map(|mut entries| entries.next().is_some())
                    .unwrap_or(true)
            {
                return Err(AppError::new(
                    ErrorCode::InvalidRequest,
                    "目标目录已有内容，不会自动改成资料库",
                    "选择新的空目录，或打开完整的已有资料库",
                ));
            }
            Vault::init(&cli.vault)
        } else {
            Vault::open_existing(&cli.vault)
        };
        let vault = opened.map_err(|_| {
            AppError::new(
                ErrorCode::Storage,
                "无法打开已有资料库",
                "先运行 recallcard init 或选择完整资料库",
            )
        })?;
        let service = ImportService::new(&vault)?;
        let memory = MemoryRuntime::new(&vault);
        let encode = |value: Value| Ok(value);
        match &cli.command {
            Command::Setup => Ok(
                json!({"message":"资料库已准备。可以直接导入和搜索；在桌面「连接模型」中一次设置目的地、密钥、范围与预算后启用后台整理。","build":recallcard::build_info(),"runtime":memory.status()?,"next_steps":["recallcard import <path>","recallcard search <query>","recallcard status"]}),
            ),
            Command::Status { git: false } => {
                let service_status = runtime::service_status(&vault)?;
                let background = memory.status()?;
                Ok(
                    json!({"build":recallcard::build_info(),"message":format!("{}；{}",if service_status.running{"后台服务正在运行"}else{"后台服务未运行"},background.message),"service":service_status,"background":background,"compatibility":"旧 Git 状态入口为 recallcard status --git"}),
                )
            }
            Command::Connections => recallcard::application::connections::inventory(&vault),
            Command::ConnectionContext {
                id,
                scope,
                budget_bytes,
            } => Ok(json!(recallcard::application::agent_entry::refresh(
                &vault,
                id,
                scope.clone(),
                *budget_bytes
            )?)),
            Command::ConnectionCheck { id, scope, verify } => {
                let health = if *verify {
                    let binary = std::env::current_exe().map_err(|_| {
                        AppError::new(
                            ErrorCode::Storage,
                            "无法定位当前读取程序",
                            "使用完整RecallCard安装包",
                        )
                    })?;
                    recallcard::application::connection_setup::verify(&vault, id, scope, &binary)?
                } else {
                    recallcard::application::connection_setup::health(&vault, id, scope)?
                };
                Ok(json!(health))
            }
            Command::ConnectionSetup { action } => match action {
                ConnectionSetupCommand::Plan {
                    client,
                    scope,
                    project_dir,
                } => {
                    let binary_path = std::env::current_exe().map_err(|_| {
                        AppError::new(
                            ErrorCode::Storage,
                            "无法定位当前读取程序",
                            "使用完整RecallCard安装包",
                        )
                    })?;
                    Ok(json!(recallcard::application::agent_install::plan(
                        &vault,
                        &recallcard::application::agent_install::InstallRequest {
                            client: client.clone(),
                            connection_id: String::new(),
                            scope: scope.clone(),
                            project_dir: project_dir.clone(),
                            binary_path
                        }
                    )?))
                }
                ConnectionSetupCommand::Apply { plan_id } => {
                    recallcard::application::connection_setup::apply(&vault, plan_id)
                }
            },
            Command::Connect {
                client,
                host_identity,
                platform,
                installation_id,
                recall_scope,
                capture_scope,
                provider_disclosure,
                auto_capture,
                auto_recall,
                expected_revision,
            } => Ok(json!(recallcard::application::connections::configure(
                &vault,
                recallcard::application::connections::ConnectionGrant {
                    client_kind: client.replace('-', "_"),
                    host_identity: host_identity.clone(),
                    platform: platform.clone(),
                    installation_id: installation_id.clone(),
                    recall_scopes: recall_scope.clone(),
                    capture_scopes: capture_scope.clone(),
                    provider_disclosure: *provider_disclosure,
                    auto_capture: *auto_capture,
                    auto_recall: *auto_recall
                },
                *expected_revision
            )?)),
            Command::ConnectionRevoke {
                id,
                expected_revision,
            } => Ok(json!(recallcard::application::connections::revoke(
                &vault,
                id,
                *expected_revision
            )?)),
            Command::Start {
                foreground,
                once,
                credential_stdin,
            } => {
                if *foreground || *once {
                    let mut provider = if *credential_stdin {
                        runtime::default_python_provider()
                            .with_credential(recallcard::application::credentials::read_handoff(
                                std::io::stdin().lock(),
                            )?)
                            .without_environment()
                    } else {
                        runtime::default_python_provider_for(&vault)?
                    };
                    Ok(json!(runtime::run_service(
                        &vault,
                        &mut provider,
                        runtime::ServiceOptions {
                            once: *once,
                            ..Default::default()
                        }
                    )?))
                } else {
                    let binary = std::env::current_exe().map_err(|_| {
                        AppError::new(
                            ErrorCode::Storage,
                            "当前发行程序不可定位",
                            "运行 start --foreground",
                        )
                    })?;
                    Ok(json!(runtime::ensure_service(&vault, &binary)?))
                }
            }
            Command::Service { action } => match action {
                ServiceCommand::Status => Ok(json!(runtime::service_status(&vault)?)),
                ServiceCommand::Stop => {
                    runtime::request_service_stop(&vault)?;
                    Ok(
                        json!({"message":"已请求服务停止；已提交资料保留","service":runtime::service_status(&vault)?}),
                    )
                }
            },
            Command::Background { action } => match action {
                BackgroundCommand::Status { scope } => Ok(json!(memory.status_for_scope(scope)?)),
                BackgroundCommand::Configure { file } => {
                    let config: MemoryConfig = input(file).map_err(|_| {
                        AppError::new(
                            ErrorCode::InvalidRequest,
                            "后台配置不是有效 JSON 或含有不允许字段",
                            "使用版本化 MemoryConfig；密钥只能通过受保护启动环境提供",
                        )
                    })?;
                    let status = memory.configure(config)?;
                    let mut result = json!(status);
                    if status.config.enabled
                        && !status.config.paused
                        && status.config.consent.is_some()
                    {
                        match std::env::current_exe()
                            .ok()
                            .and_then(|binary| runtime::ensure_service(&vault, &binary).ok())
                        {
                            Some(service) => result["service"] = json!(service),
                            None => {
                                result["service_error"] = json!({"code":"model_unavailable","message":"配置已保存；服务启动未确认，请运行 start --foreground 检查"})
                            }
                        }
                    }
                    Ok(result)
                }
                BackgroundCommand::Pause | BackgroundCommand::Resume => {
                    let mut config = memory.status()?.config;
                    config.paused = matches!(action, BackgroundCommand::Pause);
                    let status = memory.configure(config)?;
                    let mut result = json!(status);
                    if status.config.enabled
                        && !status.config.paused
                        && status.config.consent.is_some()
                    {
                        match std::env::current_exe()
                            .ok()
                            .and_then(|binary| runtime::ensure_service(&vault, &binary).ok())
                        {
                            Some(service) => result["service"] = json!(service),
                            None => {
                                result["service_error"] = json!({"code":"model_unavailable","message":"配置已保存；服务启动未确认，请运行 start --foreground 检查"})
                            }
                        }
                    }
                    Ok(result)
                }
            },
            Command::Import {
                paths,
                file,
                scope,
                format,
                request_id,
            } => {
                let mut paths = paths.clone();
                if let Some(file) = file {
                    paths.push(PathBuf::from(file));
                }
                let request_id = request_id
                    .clone()
                    .unwrap_or_else(|| format!("cli_{}", uuid::Uuid::new_v4().simple()));
                let job = service.submit(ImportRequest {
                    request_id,
                    paths,
                    scope: scope.clone(),
                    format: format.clone(),
                })?;
                let status = service.run(&job.job_id, scope)?;
                if matches!(status.state, JobState::Failed | JobState::NeedsInput) {
                    return Err(status.error.unwrap_or_else(|| {
                        AppError::new(ErrorCode::Internal, "导入未完成", "查看 jobs 状态后恢复")
                    }));
                }
                Ok(json!(service.result(&job.job_id, scope)?))
            }
            Command::Jobs { action } => {
                let (scope, id, control) = match action {
                    None => ("personal", None, None),
                    Some(JobsCommand::List { scope }) => (scope.as_str(), None, None),
                    Some(JobsCommand::Status { id, scope }) => {
                        (scope.as_str(), Some(id.as_str()), None)
                    }
                    Some(JobsCommand::Pause { id, scope }) => (
                        scope.as_str(),
                        Some(id.as_str()),
                        Some(MemoryJobControl::Pause),
                    ),
                    Some(JobsCommand::Resume { id, scope }) => (
                        scope.as_str(),
                        Some(id.as_str()),
                        Some(MemoryJobControl::Resume),
                    ),
                    Some(JobsCommand::Retry { id, scope }) => (
                        scope.as_str(),
                        Some(id.as_str()),
                        Some(MemoryJobControl::Retry),
                    ),
                    Some(JobsCommand::Cancel { id, scope }) => (
                        scope.as_str(),
                        Some(id.as_str()),
                        Some(MemoryJobControl::Cancel),
                    ),
                };
                if let Some(id) = id {
                    if id.starts_with("memory_") {
                        let status = memory
                            .status_for_scope(scope)?
                            .jobs
                            .into_iter()
                            .find(|job| job.job_id == id)
                            .ok_or_else(|| {
                                AppError::new(
                                    ErrorCode::PermissionDenied,
                                    "当前范围没有此整理任务",
                                    "从当前任务列表选择任务",
                                )
                            })?;
                        if let Some(control) = control {
                            Ok(json!(memory.control(id, control)?))
                        } else {
                            Ok(json!(status))
                        }
                    } else {
                        match control {
                            None => Ok(json!(service.status(id, scope)?)),
                            Some(MemoryJobControl::Pause) => Ok(json!(service.pause(id, scope)?)),
                            Some(MemoryJobControl::Resume | MemoryJobControl::Retry) => {
                                service.resume(id, scope)?;
                                Ok(json!(service.run(id, scope)?))
                            }
                            Some(MemoryJobControl::Cancel) => Err(AppError::new(
                                ErrorCode::InvalidRequest,
                                "此导入阶段请使用 pause 保留恢复点",
                                "停止后已提交来源仍保留；使用 jobs pause",
                            )),
                        }
                    }
                } else {
                    let mut jobs = service
                        .list(scope)?
                        .into_iter()
                        .map(|job| json!(job))
                        .collect::<Vec<_>>();
                    jobs.extend(
                        memory
                            .status_for_scope(scope)?
                            .jobs
                            .into_iter()
                            .map(|job| json!(job)),
                    );
                    Ok(json!({"jobs":jobs}))
                }
            }
            _ => encode(Value::Null),
        }
    })())
}
fn human_application_result(value: &Value) -> String {
    if let Some(job) = value.get("job") {
        return human_application_result(job);
    }
    if let Some(jobs) = value.get("jobs").and_then(Value::as_array) {
        if jobs.is_empty() {
            return "暂无后台任务".into();
        }
        return jobs
            .iter()
            .map(human_application_result)
            .collect::<Vec<_>>()
            .join("\n");
    }
    if let Some(id) = value.get("job_id").and_then(Value::as_str) {
        let state = value
            .get("state")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let phase = value
            .get("phase")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let count = value
            .pointer("/progress/events_added")
            .or_else(|| value.pointer("/progress/memories_committed"))
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let state = match state {
            "queued" => "等待处理",
            "running" => "处理中",
            "paused" => "已暂停",
            "needs_input" => "需要处理",
            "failed" => "失败",
            "completed" => "已完成",
            "cancelled" => "已取消",
            _ => "状态未知",
        };
        let phase = match phase {
            "preflight" => "预检",
            "parsing" => "解析",
            "preparing" => "准备",
            "executing" => "提取与整合",
            "validating" => "校验",
            "committing" => "提交",
            "indexing" => "更新索引",
            "finished" => "结束",
            _ => "阶段未知",
        };
        return format!("任务 {id}：{state} / {phase}；本次提交 {count}");
    }
    if let Some(message) = value.get("message").and_then(Value::as_str) {
        return message.into();
    }
    if let Some(running) = value.get("running").and_then(Value::as_bool) {
        return if running {
            "后台服务正在运行".into()
        } else {
            "后台服务未运行".into()
        };
    }
    "操作已完成；使用 --json 查看版本化详细结果".into()
}

fn ensure_visible(vault: &Vault, id: &str) -> Result<()> {
    let suppressed = vault.suppressed_ids()?;
    if suppressed.contains(id) {
        return Err("记录已被抑制；需要恢复时请显式执行 restore".into());
    }
    if id.starts_with("mem_")
        && vault
            .memory(id)?
            .data
            .source_refs
            .iter()
            .any(|id| suppressed.contains(id))
    {
        return Err("记忆的来源已被抑制".into());
    }
    Ok(())
}

fn ensure_current_visible(vault: &Vault, id: &str, revision: Option<u64>) -> Result<()> {
    ensure_visible(vault, id)?;
    if let Some(revision) = revision {
        if vault.memory(id)?.revision != revision {
            return Err("引用的记忆版本已变化，请重新搜索".into());
        }
    }
    Ok(())
}
