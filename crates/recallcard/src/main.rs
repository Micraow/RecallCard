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
    /// 本地 Vault 目录（不会自动上传到任何服务）；省略时用上次打开过的库
    #[arg(long, global = true)]
    vault: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// 创建本地事实源目录
    Init,
    /// 保存一条原始事件：--content 直接给正文，或用 --file 指定 JSON（- 为标准输入）
    Capture {
        #[arg(long)]
        content: Option<String>,
        #[arg(long)]
        file: Option<String>,
    },
    /// 启动本地界面服务：只在回环地址监听，首次访问需要终端里打印的令牌
    Ui {
        /// 授权范围；省略时用该库的第一个范围
        #[arg(long)]
        scope: Option<String>,
        /// 监听端口；省略时沿用上次的，被占用会自动换一个
        #[arg(long)]
        port: Option<u16>,
        /// 已构建的前端目录；省略时依次找程序旁的 web 与仓库里的 web/dist
        #[arg(long)]
        web_dir: Option<PathBuf>,
        /// 不尝试自动打开浏览器
        #[arg(long)]
        no_open: bool,
    },
    /// 打印一段提示词，把整个库交给外部 agent 整理；它写完后用 doctor 验收
    Brief {
        /// 授权范围；省略时用库里的第一个范围
        #[arg(long)]
        scope: Option<String>,
    },
    /// 打印 Agent 接入配置片段；不修改任何文件
    Connect {
        #[arg(long, required = true)]
        scope: Vec<String>,
        #[arg(long, default_value = "claude-code")]
        client: String,
    },
    /// 手工 Dream：导出、审阅、按摘要批准与恢复
    Dream {
        #[command(subcommand)]
        action: DreamCommand,
    },
    /// 人工管理长期记忆
    Memory {
        #[command(subcommand)]
        action: MemoryCommand,
    },
    /// 通过编号读取原始事件或记忆
    Read {
        id: String,
        /// 读取具名 View 时必须显式给出授权范围
        #[arg(long)]
        scope: Vec<String>,
    },
    /// 返回记忆的原始证据
    Sources { id: String },
    /// 生成授权范围内的稳定启动资料
    Bootstrap {
        #[arg(long, required = true)]
        scope: Vec<String>,
        #[arg(long, default_value_t = 1500)]
        budget_bytes: usize,
    },
    /// 中英文文本检索；不依赖 Dream 或云 API
    Search {
        query: String,
        #[arg(long, required = true)]
        scope: Vec<String>,
        #[arg(long, default_value = "all")]
        target: String,
        #[arg(long, default_value_t = 5)]
        limit: usize,
        #[arg(long, default_value_t = 1500)]
        budget_bytes: usize,
        #[arg(long)]
        session_ref: Option<String>,
        #[arg(long)]
        as_of: Option<chrono::DateTime<chrono::Utc>>,
        #[arg(long, default_value = "context")]
        detail: String,
        #[arg(long)]
        cursor: Option<String>,
    },
    /// 重建语义索引；只读正本，不外发
    Index {
        #[command(subcommand)]
        action: IndexCommand,
    },
    /// 为本地 Agent 提供 MCP 工具；范围由此处绑定
    Mcp {
        #[arg(long, required = true)]
        scope: Vec<String>,
    },
    /// 离线重建文本快照与视图，不重新调用模型
    Rebuild,
    /// 根据 canonical Memory 重新生成人类视图
    Views,
    /// 检查 schema、摘要与证据完整性
    Doctor,
    /// 抑制记录及其原始证据，防止检索与下一次 Dream 再次提炼
    Forget {
        id: String,
        #[arg(long)]
        reason: String,
    },
    /// 显式撤销一条抑制规则
    Restore { id: String },
}

#[derive(Subcommand)]
enum IndexCommand {
    /// 导出当前授权/抑制状态下的编码输入
    Export {
        #[arg(long, required = true)]
        scope: Vec<String>,
    },
    /// 用本机模型重建索引
    Build {
        #[arg(long, required = true)]
        scope: Vec<String>,
        #[arg(long)]
        config: Option<PathBuf>,
    },
    /// 显示索引覆盖与模型空间
    Status,
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
    let mut source: Box<dyn Read> = if file == "-" {
        Box::new(std::io::stdin().take(MAX_INPUT_BYTES as u64 + 1))
    } else {
        Box::new(
            std::fs::File::open(file)
                .map_err(|e| e.to_string())?
                .take(MAX_INPUT_BYTES as u64 + 1),
        )
    };
    source
        .read_to_string(&mut text)
        .map_err(|e| e.to_string())?;
    if text.len() > MAX_INPUT_BYTES {
        return Err("输入不能超过 16 MiB".into());
    }
    Ok(text)
}

const MAX_INPUT_BYTES: usize = 16 * 1024 * 1024;

fn input<T: DeserializeOwned>(file: &str) -> Result<T> {
    serde_json::from_str(&input_text(file)?).map_err(|e| format!("输入 JSON 无效：{e}"))
}

fn value<T: serde::Serialize>(v: T) -> Result<Value> {
    serde_json::to_value(v).map_err(|e| e.to_string())
}

fn run(cli: Cli) -> Result<Value> {
    if let Command::Init = cli.command {
        let vault = Vault::init(&vault_path(&cli)?)?;
        recallcard::ui_state::remember_vault(vault.root()).ok();
        return Ok(json!({"ok": true, "vault": vault.root(), "schema_version": 1}));
    }
    let vault = Vault::open(&vault_path(&cli)?)?;
    recallcard::ui_state::remember_vault(vault.root()).ok();
    match cli.command {
        Command::Init => unreachable!(),
        // 已在 main 里提前处理，这里不可达。
        Command::Ui { .. } | Command::Brief { .. } => unreachable!(),
        Command::Capture { content, file } => {
            let event = match (content, file) {
                (Some(content), None) => EventInput::from_content(&content),
                (None, Some(file)) => input::<EventInput>(&file)?,
                _ => return Err("请给出 --content 或 --file 之一".into()),
            };
            let event = vault.capture(event)?;
            // 索引是派生物：已有索引就把这条补进去，没有就等用户显式建立。
            if let Ok(access) = Access::new(vec![event.data.scope.clone()]) {
                if let Some(Err(error)) = recallcard::semantic::refresh_if_present(&vault, access) {
                    eprintln!("索引补建失败（不影响已保存的记录）：{error}");
                }
            }
            value(event)
        }
        Command::Connect { scope, client } => {
            let binary = std::env::current_exe().map_err(|e| e.to_string())?;
            recallcard::transport::connect_config(&binary, vault.root(), &scope, &client)
        }
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
            MemoryCommand::Add { file } => value(vault.add_memory(input::<MemoryInput>(&file)?)?),
            MemoryCommand::Update { id, revision, file } => {
                value(vault.update_memory(&id, revision, input::<MemoryInput>(&file)?)?)
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
        Command::Read { id, scope } => {
            let _read_guard = vault.read_guard()?;
            let (kind, record_id, revision) = parse_ref(&id)?;
            if kind == "view" {
                return Context::new(&vault, Access::new(scope)?).read(ReadArgs {
                    refs: vec![id],
                    budget_bytes: 32768,
                });
            }
            ensure_current_visible(&vault, record_id, revision)?;
            if !scope.is_empty() {
                Context::new(&vault, Access::new(scope)?).read(ReadArgs {
                    refs: vec![id.clone()],
                    budget_bytes: 32768,
                })?;
            }
            vault.read(record_id)
        }
        Command::Sources { id } => {
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
        Command::Index { action } => match action {
            IndexCommand::Export { scope } => {
                Context::new(&vault, Access::new(scope)?).embedding_corpus()
            }
            IndexCommand::Build { scope, config } => {
                recallcard::semantic::build_index(&vault, Access::new(scope)?, config.as_deref())
            }
            IndexCommand::Status => recallcard::semantic::index_status(&vault),
        },
        Command::Rebuild => vault.rebuild(),
        Command::Views => Ok(json!({"ok": true, "memories": vault.rebuild_views()?})),
        Command::Doctor => vault.doctor(),
        Command::Bootstrap {
            scope,
            budget_bytes,
        } => Context::new(&vault, Access::new(scope)?).bootstrap(BootstrapArgs { budget_bytes }),
        Command::Search {
            query,
            scope,
            target,
            limit,
            budget_bytes,
            session_ref,
            as_of,
            detail,
            cursor,
        } => {
            let index = recallcard::semantic::SemanticIndex::open_for_vault(&vault);
            let access = Access::new(scope)?;
            let context = match index.as_ref() {
                Some(index) => Context::with_semantic(&vault, access, index),
                None => Context::new(&vault, access),
            };
            context.search(SearchArgs {
                query,
                target,
                session_ref,
                as_of,
                limit,
                detail,
                budget_bytes,
                cursor,
            })
        }
        Command::Mcp { .. } => Err("MCP 必须以 stdio 模式启动".into()),
        Command::Forget { id, reason } => value(vault.suppress(&id, reason)?),
        Command::Restore { id } => value(vault.restore(&id)?),
    }
}

/// 生成给外部 agent 的提示词：库路径、待整理条数都由真实数据算出来。
fn build_brief(vault: Option<std::path::PathBuf>, scope: Option<String>) -> Result<String> {
    let cli = Cli {
        vault,
        command: Command::Brief { scope: None },
    };
    let vault = Vault::open(&vault_path(&cli)?)?;
    recallcard::ui_state::remember_vault(vault.root()).ok();
    let scope = match scope {
        Some(scope) => scope,
        None => vault
            .events()?
            .first()
            .map(|event| event.data.scope.clone())
            .ok_or("库里还没有记录；用 --scope 指定范围")?,
    };
    let pending = recallcard::dream_task::pending_count(&vault, &scope, None, None)?;
    let sample = vault
        .events()?
        .into_iter()
        .rev()
        .find(|event| event.data.scope == scope)
        .map(|event| event.id)
        .ok_or_else(|| {
            format!(
                "库里还没有原始记录（{}）；先用 capture 或导入，再运行 brief",
                vault.root().display()
            )
        })?;
    Ok(recallcard::brief::prompt(
        vault.root(),
        &scope,
        pending,
        &sample,
    ))
}

fn main() {
    let cli = Cli::parse();
    if let Command::Brief { scope } = &cli.command {
        // 提示词要能整段复制，所以直接写纯文本，不套 JSON。
        match build_brief(cli.vault.clone(), scope.clone()) {
            Ok(text) => {
                println!("{text}");
                return;
            }
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(1);
            }
        }
    }
    if let Command::Ui {
        scope,
        port,
        web_dir,
        no_open,
    } = &cli.command
    {
        if let Err(error) = start_ui(
            cli.vault.clone(),
            scope.clone(),
            *port,
            web_dir.clone(),
            *no_open,
        ) {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return;
    }
    if let Command::Mcp { scope } = &cli.command {
        let result = vault_path(&cli)
            .and_then(|path| Vault::open(&path))
            .and_then(|vault| {
                let access = Access::new(scope.clone())?;
                let index = recallcard::semantic::SemanticIndex::open_for_vault(&vault);
                recallcard::transport::serve_mcp_io(
                    &vault,
                    access,
                    index.as_ref(),
                    std::io::stdin().lock(),
                    std::io::stdout().lock(),
                )
            });
        if let Err(error) = result {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return;
    }
    match run(cli) {
        Ok(result) => {
            let result = serde_json::to_string_pretty(&result).expect("可序列化的输出");
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

/// 前端目录：显式参数 > 程序旁的 web > 仓库里的 web/dist。
fn resolve_web_dir(explicit: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(path) = explicit {
        return Ok(path);
    }
    let mut candidates = Vec::new();
    if let Ok(executable) = std::env::current_exe() {
        if let Some(directory) = executable.parent() {
            candidates.push(directory.join("web"));
        }
    }
    candidates.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../web/dist"));
    candidates
        .into_iter()
        .find(|path| path.is_dir())
        .ok_or_else(|| {
            "找不到前端构建产物；请先在 web 目录执行 pnpm install && pnpm build".to_string()
        })
}

/// 零参数也能起来：找库、定范围、挑端口、开浏览器。
fn start_ui(
    vault: Option<PathBuf>,
    scope: Option<String>,
    port: Option<u16>,
    web_dir: Option<PathBuf>,
    no_open: bool,
) -> Result<()> {
    let root = recallcard::ui_state::resolve_vault(vault)?;
    let scope = recallcard::ui_state::resolve_scope(&root, scope)?;
    let web = resolve_web_dir(web_dir)?;
    let preferred = port.unwrap_or_else(|| recallcard::ui_state::remembered_port(7878));
    let bound = match recallcard::http::bind(recallcard::http::UiOptions {
        vault: root.clone(),
        scope: scope.clone(),
        web_dir: web.clone(),
        port: preferred,
        open_browser: false,
    }) {
        Ok(bound) => bound,
        // 端口被占就换一个，不让人去查是谁占的
        Err(_) => recallcard::http::bind(recallcard::http::UiOptions {
            vault: root.clone(),
            scope: scope.clone(),
            web_dir: web,
            port: 0,
            open_browser: false,
        })?,
    };
    let url = bound.url().to_owned();
    recallcard::ui_state::remember_session(&root, &scope, bound.port()).ok();
    println!(
        "资料库 {}（范围 {}）",
        recallcard::ui_state::describe(&root),
        scope
    );
    println!("{url}");
    if !no_open {
        recallcard::http::open_browser(&url);
    }
    recallcard::http::run(bound)
}

/// 没给 --vault 时沿用 clap 之外的既有行为：当前目录下的 vault/。
/// 没有 --vault 时沿用上次打开过的库；都没有就说清楚该怎么做，
/// 而不是抛一个 "No such file or directory"。
fn vault_path(cli: &Cli) -> Result<PathBuf> {
    recallcard::ui_state::resolve_vault(cli.vault.clone())
}
