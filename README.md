# RecallCard

让属于你自己的记忆，跨越不同 AI 客户端。

RecallCard 是本地优先、文件原生的个人上下文层。Rust 主程序保存可追溯事件和长期记忆，向本地 Agent 提供只读检索。数据留在你选定的 Vault；初始化不会配置远端，也不调用任何云模型。

## 当前可运行的第一段

- JSONL 原始事件封段、重复导入去重、内容修订链
- Markdown + YAML frontmatter 长期记忆、原子替换、版本冲突保护
- 原始 Event 即时中英文 BM25 检索，尚未 Dream 的对话也可以查
- MCP stdio 的 `bootstrap / search / read / sources` 四个只读工具
- 按客户端绑定 scope；来源校验、反注入回声、秘密模式脱敏、遗忘抑制
- 主动导入手工 JSONL、ChatGPT 官方导出所选分支、Claude Code JSONL 日志
- 确定性人类视图、Git 初始化、离线健康检查

这是开发中的首个纵切，不代表整份设计已完成。手动 Dream、浏览器桥、可选云向量、同步与跨平台硬化分段交付，见[实施进度](docs/progress.md)。目前不需要 Python、API key 或向量数据库。

## 快速开始

需要 Rust 1.89+ 和 Git：

```bash
cargo build --release
# Linux/macOS 使用 target/release/recallcard；Windows 使用 .exe
cargo run -- --vault /你选择的路径/recallcard-vault init
cargo run -- --vault /你选择的路径/recallcard-vault import \
  --format manual-jsonl --file fixtures/manual-web.jsonl --scope project:demo
cargo run -- --vault /你选择的路径/recallcard-vault search "Rust" --scope project:demo
cargo run -- --vault /你选择的路径/recallcard-vault doctor
```

仓库中的示例全部为合成数据。不要把自己的真实资料库放进此公开代码仓库。

## 接入本地 Agent

```bash
recallcard --vault /资料库绝对路径 mcp --scope personal --scope project:demo
```

MCP 的读取权限由启动参数固定，模型不能用请求参数扩大范围。首次会话应由宿主显式调用 `bootstrap`；只声明工具并不保证宿主自动加载。示例配置见 [Claude Code 配置](integrations/claude-code/mcp.example.json)，具体步骤见[本地使用](docs/usage.md)。

## 数据布局

```text
vault/
├── events/YYYY/MM/会话摘要/evt_内容摘要.jsonl
├── memories/mem_编号.md
├── objects/
├── control/schema-version.json
├── control/suppressions/
├── control/dream-receipts/
├── generated/views/
└── .index/
```

第一版每次写入立即形成单事件不可变封段，以优先保证崩溃安全和跨设备不共写文件。Memory 正文可直接用编辑器、`cat`、`rg` 查看。运行锁与临时状态放系统状态目录，可通过 `RECALLCARD_STATE_DIR` 指定；不会进入 Vault Git。

## 设计和验证

- [整体设计 v0.2](docs/RecallCard-DESIGN-v0.2.md) 为实施依据；[v0.1](docs/RecallCard-DESIGN.md) 保留历史
- [本地使用与数据格式](docs/usage.md)
- [安全边界](docs/security.md)
- [实施进度与验收](docs/progress.md)

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Linux 上验证了合成输入的 CLI 和 MCP 端到端回归。真实已登录网页、真实模型调用、Windows/macOS 安装链路仍需分别验收；不宣称缓存命中率、记忆准确率或账单节省。

许可证尚待项目维护者选择；当前未代替维护者授予开源许可证。
