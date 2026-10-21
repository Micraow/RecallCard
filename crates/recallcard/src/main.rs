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
        /// 由本机维护者固定的可选语义索引配置；默认不启用
        #[arg(long)]
        semantic_config: Option<PathBuf>,
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
        #[arg(long)]
        semantic_config: Option<PathBuf>,
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
        let vault = Vault::init(&cli.vault)?;
        return Ok(json!({"ok": true, "vault": vault.root(), "schema_version": 1}));
    }
    let vault = Vault::open(&cli.vault)?;
    match cli.command {
        Command::Init => unreachable!(),
        Command::Capture { content, file } => {
            let event = match (content, file) {
                (Some(content), None) => EventInput::from_content(&content),
                (None, Some(file)) => input::<EventInput>(&file)?,
                _ => return Err("请给出 --content 或 --file 之一".into()),
            };
            value(vault.capture(event)?)
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
            semantic_config,
        } => {
            let index = recallcard::semantic::SemanticIndex::open_for(
                vault.root(),
                semantic_config.as_deref(),
            );
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

fn main() {
    let cli = Cli::parse();
    if let Command::Mcp {
        scope,
        semantic_config,
    } = &cli.command
    {
        let result = Vault::open(&cli.vault).and_then(|vault| {
            let access = Access::new(scope.clone())?;
            let index = recallcard::semantic::SemanticIndex::open_for(
                vault.root(),
                semantic_config.as_deref(),
            );
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
