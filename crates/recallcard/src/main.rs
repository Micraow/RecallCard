use clap::{Parser, Subcommand, ValueEnum};
use recallcard::{
    context::{BootstrapArgs, Context, SearchArgs},
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
    /// 保存一条原始事件；文件为 - 时从标准输入读取 JSON
    Capture {
        #[arg(long)]
        file: String,
    },
    /// 导入主动提供的导出或日志文件
    Import {
        #[arg(long)]
        format: String,
        #[arg(long)]
        file: String,
        #[arg(long)]
        scope: String,
    },
    /// 手工 Dream：导出、预览、按摘要批准与恢复
    Dream {
        #[command(subcommand)]
        action: DreamCommand,
    },
    /// 以浏览器原生消息协议提供受限只读访问
    NativeHost {
        #[arg(long, required = true)]
        scope: Vec<String>,
        #[arg(long)]
        allowed_extension: String,
        origin: String,
        #[arg(long)]
        parent_window: Option<u64>,
    },
    /// 生成待人工检查/注册的 Native Messaging 文件
    NativeInstall {
        #[arg(long, required = true)]
        scope: Vec<String>,
        #[arg(long)]
        extension_id: String,
        #[arg(long)]
        output_dir: PathBuf,
    },
    /// 人工管理长期记忆
    Memory {
        #[command(subcommand)]
        action: MemoryCommand,
    },
    /// 通过编号读取原始事件或记忆
    Read { id: String },
    /// 返回记忆的原始证据
    Sources { id: String },
    /// 按当前授权/抑制状态导出可选Embedding输入；不会发送云端
    EmbeddingExport {
        #[arg(long, required = true)]
        scope: Vec<String>,
    },
    /// 校验并先提交本地正本，再整合与推送指定 Git 远端
    Sync {
        #[arg(long)]
        remote: String,
    },
    /// 查看资料库 Git 状态，不访问远端
    Status,
    /// 离线重建文本快照与视图，不重新调用模型
    Rebuild,
    /// 根据 canonical Memory 重新生成人类视图
    Views,
    /// 检查 schema、摘要与证据完整性
    Doctor,
    /// 生成授权范围内的稳定启动资料
    Bootstrap {
        #[arg(long, required = true)]
        scope: Vec<String>,
        #[arg(long, default_value_t = 1500)]
        budget_tokens: usize,
    },
    /// 不依赖 Dream 或云 API 的中英文文本检索
    Search {
        query: String,
        #[arg(long, required = true)]
        scope: Vec<String>,
        #[arg(long, default_value = "all")]
        target: String,
        #[arg(long, default_value_t = 5)]
        limit: usize,
        #[arg(long, default_value_t = 1500)]
        budget_tokens: usize,
    },
    /// 为本地 Agent 提供四个只读工具；范围由此处绑定
    Mcp {
        #[arg(long, required = true)]
        scope: Vec<String>,
    },
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

fn input<T: DeserializeOwned>(file: &str) -> Result<T> {
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
    serde_json::from_str(&text).map_err(|e| format!("输入 JSON 无效：{e}"))
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
        Command::Capture { file } => value(vault.capture(input::<EventInput>(&file)?)?),
        Command::Import {
            format,
            file,
            scope,
        } => {
            let text = std::fs::read_to_string(&file).map_err(|e| e.to_string())?;
            recallcard::import::import_text(&vault, &format, &text, &scope)
        }
        Command::NativeInstall {
            scope,
            extension_id,
            output_dir,
        } => recallcard::native::prepare_install(&vault, scope, &extension_id, &output_dir),
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
        Command::Read { id } => {
            ensure_visible(&vault, &id)?;
            vault.read(&id)
        }
        Command::Sources { id } => {
            ensure_visible(&vault, &id)?;
            value(vault.sources(&id)?)
        }
        Command::EmbeddingExport { scope } => {
            Context::new(&vault, Access::new(scope)?).embedding_corpus()
        }
        Command::Rebuild => vault.rebuild(),
        Command::Sync { remote } => vault.sync(&remote),
        Command::Status => vault.git_status(),
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
        } => Context::new(&vault, Access::new(scope)?).search(SearchArgs {
            query,
            target,
            session_ref: None,
            as_of: None,
            limit,
            detail: "context".into(),
            budget_tokens,
            cursor: None,
        }),
        Command::Mcp { .. } => Err("MCP 必须以 stdio 模式启动".into()),
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
        ..
    } = &cli.command
    {
        let result = Vault::open(&cli.vault).and_then(|vault| {
            recallcard::native::serve_native(
                &vault,
                Access::new(scope.clone())?,
                allowed_extension,
                origin,
            )
        });
        if let Err(error) = result {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return;
    }
    if let Command::Mcp { scope } = &cli.command {
        let result = Vault::open(&cli.vault).and_then(|vault| {
            recallcard::transport::serve_mcp(&vault, Access::new(scope.clone())?)
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
