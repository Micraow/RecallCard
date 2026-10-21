# RecallCard

本地优先的个人 AI 上下文与记忆层。把跨 AI 的对话变成可追溯的本地事实，让下一个 AI 接着用。

没有账号、云同步或后台服务。资料以 Markdown 与 JSONL 存在你自己指定的目录里，`grep` 和 `git diff` 都能直接读。

## 它解决什么

换一个 AI 就要重讲一遍背景，讲完还不准。RecallCard 把你说过的话存成原始事件，把确认过的事实整理成长期记忆，再按需交给当前这个 AI——只交它需要的部分，每条都能追回原话。

## 三条使用路径

**本地 Agent（Claude Code、Codex、DSH 等）** — 支持 MCP 的客户端自己检索：

```sh
recallcard connect --vault ~/我的资料 --scope personal
```

把输出的配置粘贴进客户端，之后它自己 `bootstrap`、`search`、`read`。你不需要手动挑资料。

**网页 AI** — 把搜索结果复制出来贴进网页，或把网页回复粘贴回来存成事件。网页不能调用工具，所以这一步是人复制粘贴，不需要浏览器扩展。

**命令行** — 全部能力都可脚本化：捕获、检索、整理、索引、检查。

## 快速开始

```sh
recallcard init    --vault ~/我的资料
recallcard capture --vault ~/我的资料 --content "决定先核对来源，再继续实现"
recallcard search  --vault ~/我的资料 --scope personal "核对来源"
recallcard doctor  --vault ~/我的资料
```

整理记忆是人工确认的，不在后台自动发生：

```sh
recallcard dream export --vault ~/我的资料 --source event:xxxx > 任务.md
# 把任务贴给任意 AI，把结果存成 result.md
recallcard dream review --vault ~/我的资料 --input result.md
recallcard dream apply  --vault ~/我的资料 --input result.md
```

界面是一个本地网页，由 CLI 现场起服务，不常驻、不联网：

```sh
cd web && pnpm install && pnpm build   # 只需构建一次
recallcard ui --vault ~/我的资料 --scope personal
```

终端会打印一个带一次性令牌的地址，用浏览器打开即可。服务只监听 `127.0.0.1`，
令牌换成的 Cookie 只存在本次进程里；关掉进程，页面也随之失效。

## 设计

见 [设计文档](docs/design.md)：不变量、目录结构、三层披露、检索与索引、证据等级、Dream 流程、Agent 接入。

核心只有两条：事实源只有 `events/` 与 `memories/`；索引与视图随时可删可重建。

## 开发

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

界面构建与类型检查：

```sh
cd web && pnpm install --frozen-lockfile && pnpm build
```

约定见 [AGENTS.md](AGENTS.md)。
