# RecallCard

让属于你自己的记忆，跨越不同 AI 客户端。

RecallCard 是本地优先、文件原生的个人上下文层。Rust 主程序保存可追溯事件和长期记忆，向本地 Agent 提供只读检索。数据留在你选定的 Vault；初始化不会配置远端，也不调用任何云模型。

## 当前可运行能力

- JSONL 原始事件封段、重复导入去重、内容修订链
- Markdown + YAML frontmatter 长期记忆、原子替换、版本冲突保护
- 原始 Event 即时中英文 BM25 检索，尚未 Dream 的对话也可以查
- MCP stdio 的 `bootstrap / search / read / sources` 四个只读工具
- 按客户端绑定 scope；来源校验、反注入回声、秘密模式脱敏、遗忘抑制
- 主动导入手工 JSONL、ChatGPT 官方导出所选分支、Claude Code JSONL 日志
- 手动 Dream：有界导出、差异预览、按摘要批准、保护记录、幂等收据和崩溃恢复
- Chrome 扩展 + Native Messaging：ChatGPT，以及实验性 Qwen/Z.ai；只读查询、可见草稿、人工发送
- Claude Code SessionStart 只读 Hook：启动、恢复、压缩、清空后重新生成授权 Bootstrap
- 确定性人类视图、安全 Git 同步、离线健康检查
- 可选 Python 向量 worker：空间签名、增量复用、权限过滤与离线降级
- 可选 API Dream：固定前缀、请求预算、usage 记录，只生成待审核提议

这是开发中的分阶段实现，不代表整份设计已完成。独立 daemon/IPC 与可选 MCP 语义后端已通过三平台自动回归；真实宿主生命周期与登录态网页验收仍在继续，见[实施进度](docs/progress.md)。核心离线功能不需要 Python、API key 或向量数据库。

## 快速开始

拿到 Linux 开发包后，按 [Linux 首次使用 v0.3](docs/quickstart-linux-v0.3.md) 直接运行；核心不需要 Rust/Python/Node，先用包内合成资料完成离线检索。

从源码构建需要 Rust 1.89+ 和 Git：

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
- [手动 Dream](docs/dream.md)
- [浏览器桥安装与人工验收](docs/browser.md)
- [Qwen / Z.ai 手动适配与限制 v0.3](docs/browser-adapters-v0.3.md)
- [原生 DOM 回归与富文本保护 v0.3](docs/browser-engine-v0.3.md)
- [Agent 生命周期 Hook v0.3](docs/agent-hooks-v0.3.md)
- [安全 Git 同步](docs/sync.md)
- [可选向量 worker](docs/embedding.md)
- [本机 IPC 与桥接 v0.3](docs/ipc-v0.3.md)
- [可选语义检索 v0.3](docs/semantic-search-v0.3.md)
- [可选 API Dream](docs/dream-api.md)
- [跨平台 CI 修复记录](docs/ci.md)
- [具名 View、实体与有效期](docs/context-v0.3.md)
- [T01–T12 设计验收差距 v0.3](docs/acceptance-v0.3.md)
- [实施进度](docs/progress.md)

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
node --test extension/tests/*.test.js
PYTHONPATH=python RECALLCARD_TEST_BINARY="$PWD/target/debug/recallcard" python3 -m unittest discover -s python/tests -v
PYTHONPATH=python python3 python/tests/check_rust_embedding_contract.py "$PWD/target/debug/recallcard"
```

Linux 上验证了合成输入的 CLI 和 MCP 端到端回归。真实已登录网页、真实模型调用、Windows/macOS 安装链路仍需分别验收；不宣称缓存命中率、记忆准确率或账单节省。

许可证尚待项目维护者选择；当前未代替维护者授予开源许可证。
