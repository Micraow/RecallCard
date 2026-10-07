# RecallCard Linux 首次使用 v0.3

这是能直接运行的开发版，优先供 Arch Linux x86_64 使用。核心功能不用安装 Rust、Python、Node 或模型，也不需要 API key。需要系统已有 Git；Arch 通常可通过系统包管理器安装 Git。可选 Python worker 才需要 Python 3.10+。

当前代码构建基线写在包内 `build-info.json`。这是命令行工具和 Chrome 扩展，没有独立桌面窗口。所有示例均为合成资料，下面的步骤不会调用任何模型、上传记忆或配置远端。

## 1 解压并确认程序能运行

把 Linux 包解压到自己方便的位置，在该文件夹打开终端：

```bash
./recallcard --version
./recallcard --help
```

若解压工具没有保留执行位，可对当前目录的这个程序运行 `chmod u+x ./recallcard`。正常应显示 `recallcard 0.1.0` 和中文命令说明。

## 2 建立自己的私人资料库

程序目录与资料库分开。首次创建一个只有自己能访问的目录：

```bash
mkdir -m 700 "$HOME/RecallCardVault"
./recallcard --vault "$HOME/RecallCardVault" init
```

若目录已存在，先确认它是否就是你要继续使用的资料库，不要删除或覆盖内容。`init` 可以继续初始化已有受支持目录。不要把真实资料库放进公开 RecallCard 代码仓库，不要把公开仓库地址当作私人 Vault remote。

## 3 先用合成资料验证离线闭环

```bash
./recallcard --vault "$HOME/RecallCardVault" import \
  --format manual-jsonl --file fixtures/manual-web.jsonl --scope project:demo

./recallcard --vault "$HOME/RecallCardVault" search "Rust" \
  --scope project:demo --budget-tokens 8000

./recallcard --vault "$HOME/RecallCardVault" doctor
```

第二步的 `results` 应非空，并带 `event:...` 来源；第三步应有 `ok: true`。重复导入同一个示例不会重复保存同一条来源。到这里，不做 Dream、没有云模型，也已经可以检索保存的原始对话。

替换为真实资料时，只导入自己明确选择的原文。支持手动 JSONL、ChatGPT 官方导出与 Claude Code JSONL，格式和命令见 `docs/usage.md`。不要把助手建议标成用户已经决定的事实。

## 4 让本地 Agent 按需读取

在自己选定的客户端里添加一个 MCP stdio server：

- command：解压后 `recallcard` 的绝对路径
- args：`--vault`、资料库绝对路径、`mcp`、`--scope`、`project:demo`

例如：

```json
{
  "mcpServers": {
    "recallcard": {
      "command": "/你的绝对路径/recallcard",
      "args": ["--vault", "/你的绝对路径/RecallCardVault", "mcp", "--scope", "project:demo"]
    }
  }
}
```

示例配置也在 `integrations/claude-code/mcp.example.json`。只给该客户端你允许读取的 scope；不要为了省事开放全部个人资料。首次会话显式调用 `bootstrap`，再调用 `search`。上下文注入后，所选客户端可能随模型请求把资料发送到其服务，需要你决定是否批准该接收者与范围。

如果要使用启动/恢复/压缩后的 Hook，先阅读 `docs/agent-hooks-v0.3.md` 并手工合并示例配置，保留原有设置。本包没有替你安装 Hook，也没有运行 Claude、Codex 或任何外部 Agent。

## 5 可选：浏览器可见草稿桥

1. 在 Chrome 扩展页开启开发者模式，加载包内 `extension` 文件夹，记录 Chrome 显示的扩展 ID
2. 生成固定范围的本机桥文件；将下面的 ID 替换成自己的真实扩展 ID

```bash
./recallcard --vault "$HOME/RecallCardVault" native-install \
  --scope project:demo --extension-id '这里替换成32位扩展ID' \
  --output-dir "$HOME/.local/share/recallcard-native-demo"
```

3. 按 `docs/browser.md` 的 Chrome/Linux 注册步骤放置生成的 manifest；不要覆盖现有注册或配置
4. 打开受支持的 ChatGPT 页面；Qwen/Z.ai 为实验性适配，先显式重置绑定。准备预览，检查后追加到可见草稿，最后由自己点击网页发送

扩展不读取/抓取回复，不自动发送。插入草稿本身已经让当前网站接触这段资料，点击插入前先核对网站和内容。真实登录态网站与用户电脑上的注册链路尚未完成实机验收；合成 Chromium 和 Native 进程测试不是这个验收的替代品。

如果暂时不安装扩展，仍可使用 CLI/MCP 的离线检索；也可以自行检查结果后手动复制所需片段。

## 6 暂时不用配置的部分

- Dream：以后需要整理长期记忆时，再按 `docs/dream.md` 导出、审核并批准
- Embedding/API Dream：默认关闭。只在你批准具体服务、数据与范围后配置，详见相关中文文档
- daemon/IPC：MCP 直接模式已可使用。多个入口希望共用固定核心时再看 `docs/ipc-v0.3.md`
- Git 同步：先使用本机库；确定私人远端、Git 身份与冲突处理方式后再看 `docs/sync.md`

## 排错与安全退出

- 命令报错时检查 stderr 和非零退出码，不要因为生成了空 JSON 文件就当作成功
- `doctor` 报损坏或 Git 冲突时先保留原文件，不要删除资料库“重试”
- 未启动 daemon 时，显式 IPC 模式会报连接错误；直接 MCP 模式不需要 daemon
- 工具进程可用 Ctrl-C 停止；数据在 Vault 文件中，退出程序不会删除资料
- 若系统提示缺少某个 GLIBC 版本，以包内 `build-info.json` 的构建信息为准；不要替换系统 libc。可以在目标系统从源码构建

本包未配置开机启动、系统服务或远端，没有保存任何 key。原始设计、边界与各项测试说明保留在 `docs/`。
