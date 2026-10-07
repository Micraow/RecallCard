# 浏览器手动上下文桥

本页对应 `extension/`，以设计 v0.2 为准。扩展是无第三方运行依赖的 Chrome Manifest V3 实现，使用固定的 `com.recallcard.host` 本机桥，不启动 HTTP 服务，不调用付费 API。

**验证状态：已运行 Node 内建单元、模拟 DOM 和生命周期测试，以及 Linux 上的 Native 启动器子进程、二进制分帧和配置拒绝测试；尚未在真实登录的 ChatGPT 页面、Chrome Native Messaging 安装环境、macOS 或 Windows 上完成端到端验证。** 请先用合成资料验收；不要把“测试通过”理解为当前 ChatGPT DOM 已实测兼容。

## 1. 本阶段支持什么

- 在 `https://chatgpt.com/`、`/c/<id>`、`/g/<id>`、`/g/<id>/c/<id>` 打开扩展弹窗
- 用户点击后准备最小 Bootstrap，预览范围内的资料，再显式追加到可见输入框
- 用户手动复制模型输出的完整 `recallcard-action` 块，粘贴进扩展后执行只读检索
- `bootstrap/search/read/sources`；read/sources 支持批量引用
- 原草稿不覆盖；撤销只处理扩展拥有、内容仍可完整识别的块；用户修改导致不能安全识别时停止
- 当前标签页、主框架、浏览器文档、URL、会话 nonce、request_id 校验和重复请求拒绝
- 手动发送确认；“已准备”和“已插入草稿”不表示模型已经收到

不支持自动读取/保存聊天输出、监听流式回复、隐藏推理、工具输出抓取、官方导出下载自动化、自动发送、无人值守 Web Dream。网页账号记忆和内部压缩不可观察，不假装能够检测。

### 为什么使用手动复制

2026-10-06 核对的 [OpenAI 使用条款](https://openai.com/policies/terms-of-use/) 限制自动或程序化提取数据与输出。扩展因此不扫描对话 DOM，也不把用户手动点击发送当作自动抓取的许可。此处是保守的工程边界，不是针对所有地区、账号合同的法律结论。使用者仍须遵守适用于自己账号的条款。

对话保存使用用户主动复制或 [ChatGPT 官方数据导出](https://help.openai.com/en/articles/7260999-exporting-your-chatgpt-history-and-data)，再通过本地 CLI 导入。官方页面介绍的导出得到 ZIP；用户解压后仅在其中确实包含 `conversations.json` 时使用对应导入器。导出能力受账号与工作区限制，文件格式也可能改变。不要把原始导出提交到这个代码仓库。

## 2. 构建与扩展加载

前提：Chrome 116 或更高版本、已经构建的 RecallCard 二进制。测试需要 Node 20 或以上；没有 npm 依赖，不需要 `npm install`。

```bash
# 在仓库根目录
cargo build --release
node --test extension/tests/*.test.js
```

1. 在自己的 Chrome 打开 `chrome://extensions`，开启开发者模式
2. 点击“加载已解压的扩展程序”，选择本仓库的 `extension/` 目录，不是仓库根目录
3. 记下扩展卡片上的 32 个字符 ID，用于下面的本机授权
4. 固定工具栏图标；加载或更新扩展后刷新已有 ChatGPT 标签页
5. 可在扩展卡片进入 service worker 检查页面；扩展不会把 Vault 内容写入调试日志

开发阶段改变扩展路径可能改变 ID；ID 变化后必须重新生成并注册本机 manifest。Chrome Web Store 发布、签名和自动更新尚未实现。

## 3. 生成本机桥并手动注册

先按 [本地使用说明](usage.md) 创建测试 Vault。必须选择自己的二进制绝对路径、Vault 绝对路径、准确扩展 ID，以及允许读取的 scope。不要把所有 Vault 默认授权给网页。

```bash
# 修改这些路径和 ID；不要把尖括号占位符原样执行
BIN=/绝对路径/RecallCard/target/release/recallcard
VAULT=/绝对路径/recallcard-vault
HOST_DIR="$HOME/.local/share/recallcard/native"
EXTENSION_ID=这里替换为Chrome显示的32字符扩展ID

"$BIN" --vault "$VAULT" native-install \
  --extension-id "$EXTENSION_ID" \
  --scope personal \
  --output-dir "$HOST_DIR"
```

`native-install` 只生成待检查文件，返回 `registered: false`，不注册浏览器、不修改注册表：

- `recallcard-native-launcher`（Windows 为 `.exe`）：当前平台 RecallCard 可执行文件的副本，不依赖原来的构建目录
- `com.recallcard.host.config.json`：版本固定为 1，绑定 host 名、一个 Vault 绝对路径、显式 scope 列表和准确扩展 ID；没有密钥
- `com.recallcard.host.json`：给 Chrome 的 Native Messaging manifest
- Linux/macOS 另有 `recallcard-native-host` shell 包装器，只执行上述相邻副本并原样转交参数；Windows manifest 直接指向 `.exe`

专用副本只从自身目录读取固定名字的配置；工作目录、浏览器参数和消息均不能另选配置、Vault、scope 或命令。它只接受准确的 `chrome-extension://<ID>/`，以及可选的 `--parent-window=<十进制非负整数>`。普通 `recallcard` 命令仍保持原 CLI 行为，人工诊断也可显式使用 `native-host` 子命令。不要改文件名、改用 `eval`，或把模型参数接到命令行。

安装拒绝已有目标文件、安装根及文件的符号链接和 Windows 重解析点；用户明确选定路径中的 `/var` 等父目录别名会先解析为真实安装路径，再把真实绝对路径写入配置和 manifest。文件通过临时文件无覆盖发布，manifest 最后生成。失败时可能留下本次已完成的副本，但不会自动删文件或注册半成品，请检查后改用新的输出目录。Unix 新目录/可执行文件权限为 `0700`、配置/manifest 为 `0600`，启动时拒绝可被组或其他用户写入的配置和安装目录。Windows 不会自动调整或核验 ACL，须由使用者选择自己控制、其他用户不可写的目录。以上检查不抵御已控制同一本机账号的恶意进程。

配置最多 16 KiB、1–32 个不重复的有效 scope；拒绝未知/重复字段、错误版本、相对 Vault 路径、父目录跳转及不匹配的扩展来源。配置和安装目录应放在代码仓库、同步 Vault 之外，不要提交本机路径与权限配置。Host 的标准输出只有 Native Messaging 二进制分帧，诊断写标准错误。

本机 manifest 的形状可参考 `extension/native-host-manifest.example.json`。`allowed_origins` 必须是准确的 `chrome-extension://<ID>/`，不能用通配符；`path` 指向可执行包装器或 Windows `.exe` 的绝对路径。[Chrome 官方 Native Messaging 文档](https://developer.chrome.com/docs/extensions/develop/concepts/native-messaging)

### Linux：Google Chrome 用户级注册

```bash
mkdir -p "$HOME/.config/google-chrome/NativeMessagingHosts"
cp "$HOST_DIR/com.recallcard.host.json" \
  "$HOME/.config/google-chrome/NativeMessagingHosts/com.recallcard.host.json"
```

Chromium 的用户级目录通常为 `~/.config/chromium/NativeMessagingHosts/`。自定义用户数据目录和 Chrome for Testing 应按当前官方文档核对，不要照抄其他浏览器的路径。

### macOS：Google Chrome 用户级注册

```bash
mkdir -p "$HOME/Library/Application Support/Google/Chrome/NativeMessagingHosts"
cp "$HOST_DIR/com.recallcard.host.json" \
  "$HOME/Library/Application Support/Google/Chrome/NativeMessagingHosts/com.recallcard.host.json"
```

### Windows：生成 `.exe` 并由用户注册到 Google Chrome

先在 Windows 构建 `recallcard.exe`；其他平台的二进制不能直接换扩展名使用。下面是供用户自行审阅、执行的 PowerShell 示例，路径和扩展 ID 必须替换：

```powershell
$Bin = 'C:\RecallCard\target\release\recallcard.exe'
$Vault = 'C:\Users\你的用户名\recallcard-vault'
$HostDir = Join-Path $env:LOCALAPPDATA 'RecallCard\native-v1'
$ExtensionId = '这里替换为Chrome显示的32字符扩展ID'

& $Bin --vault $Vault native-install --extension-id $ExtensionId `
  --scope personal --output-dir $HostDir
if ($LASTEXITCODE -ne 0) { throw '生成失败，请检查错误，不要继续注册' }

$Manifest = (Resolve-Path -LiteralPath (Join-Path $HostDir 'com.recallcard.host.json')).Path
Get-Content -LiteralPath $Manifest
Get-Content -LiteralPath (Join-Path $HostDir 'com.recallcard.host.config.json')
```

先检查 manifest 的 `path` 指向本目录内的 `recallcard-native-launcher.exe`，`allowed_origins` 只有你的准确扩展来源；配置内 Vault、scope、扩展 ID 也必须正确。不要移动或只复制 `.exe`，配置必须相邻。

Chrome 官方要求该 host 注册表项的默认 `REG_SZ` 值是 manifest **完整路径**，不是 `.exe` 路径。它先查 32 位、再查 64 位注册表视图。以下采用当前用户 HKCU 的 32 位视图，不写全机 HKLM；先查询已有值，发现旧配置时先核对归属并保留备份，别直接覆盖。[Chrome 官方注册说明](https://developer.chrome.com/docs/extensions/develop/concepts/native-messaging)

```powershell
$Key = 'HKCU\Software\Google\Chrome\NativeMessagingHosts\com.recallcard.host'
reg.exe query $Key /ve /reg:32
# 64 位 Windows 还应检查另一个视图，避免旧配置造成混淆
if ([Environment]::Is64BitOperatingSystem) { reg.exe query $Key /ve /reg:64 }
```

“找不到项”只有在该项确实未创建时才是正常情况。确认路径、权限与旧项归属后，**由用户执行**注册和复查；这里没有 `/f` 强制覆盖选项：

```powershell
reg.exe add $Key /ve /t REG_SZ /d $Manifest /reg:32
if ($LASTEXITCODE -ne 0) { throw '注册未完成，请检查错误' }
reg.exe query $Key /ve /reg:32
```

`/ve` 表示默认值，`/reg:32` 指定注册表视图；相关命令选项见 [Microsoft reg add](https://learn.microsoft.com/en-us/windows-server/administration/windows-commands/reg-add) 和 [reg query](https://learn.microsoft.com/en-us/windows-server/administration/windows-commands/reg-query)。注册完成后用合成 Vault 检查扩展连接、分帧和来源拒绝，不要直接输入私人资料。上述官方注册文档核对日期为 2026-10-06；部署时仍应核对当前浏览器版本。

### 升级、迁移与撤销

- 更新程序、Vault 位置、scope 或扩展 ID：用新的输出目录重新运行 `native-install`，检查后由用户将浏览器注册位置切换到新 manifest；旧副本不会跟随源二进制自动更新
- 不要直接搬走已注册的输出目录：manifest 和 Unix wrapper 含绝对路径，须在目标位置重新生成并重新注册
- 停用时先在 Chrome 禁用扩展；用户自行移除确认属于这次安装的 manifest 注册文件或 Windows 对应注册项。不要删除整个 `NativeMessagingHosts` 父目录/父项，也不要删除 Vault
- 原始构建目录移走不影响已生成的副本；仍须满足当前操作系统的运行时要求，这不是跨操作系统二进制打包

以上命令只作为用户自行安装的说明，本阶段未在用户电脑执行。Windows 注册表、Windows ACL/重解析点、Windows Chrome 的真实 `.exe` 启动及管道 I/O、macOS 平台执行均待实机验证。Linux 子进程测试只覆盖共用逻辑，不代表 Windows 已验收。Firefox、Safari、Edge 不在本轮验证范围。

## 4. 第一次使用

1. 打开一个 ChatGPT 对话，在网页输入框先打几个字，例如“我的原有草稿”，但不要发送
2. 打开扩展，点击“准备 Bootstrap”。本机返回后，资料只在扩展预览里
3. 检查资料、来源、scope 和隐私。点击“已检查，追加到草稿”会让 ChatGPT 网页接触这些资料；即使未点击发送，也不要把这一步当作绝对离线
4. 确认原有草稿仍在，新增块带有 RecallCard 标记。需要撤销时点击“仅移除我的上下文”
5. 想继续时，由自己点击 ChatGPT 网页的发送按钮。扩展不点击按钮、不模拟 Enter、不提交表单
6. 若页面未切换，再回扩展点击“我已亲自点击网页发送”。它记录用户确认，不声称平台已接收；草稿中的完整标记还在时拒绝假确认
7. 模型需要检索时，请自己复制完整请求块到扩展，点击“验证并读取本机资料”，再检查返回预览并决定是否插入

每个会话保留已审核的 Bootstrap 快照，但纠正、遗忘和权限变化优先于缓存。点击“准备 Bootstrap”重用旧快照前会再次联系本机，核验当前 bootstrap_version 和稳定资料指纹；仅动态 coverage 改变时保留原快照及其当时的覆盖信息。稳定内容或来源变化、授权拒绝、版本缺失或本机断线都会废弃旧预览，要求重新准备和检查，不会悄悄替换后直接插入。已确认发送的同一块不会重复注入。若模型再次请求 bootstrap，扩展会提示用按钮检查已有快照。需要补回访问说明时可点击“重置 / 重新附上说明”。扩展不能发现模型内部压缩，也不会修改已发送的历史消息。

任何 Bootstrap、search、read 或 sources 预览在点击插入时，都会以当前 nonce/session_ref 和全新的 request_id 再次执行同一只读请求。search/read/sources 比较完整结构化结果，忽略对象键的排列差异；Bootstrap 比较版本与稳定字段。核验后才允许插入已经审核的原文，结果有变就清空预览并提示重新请求、审核。重新打开弹窗只是恢复暂存预览，并不证明资料仍有效；若采用手动复制回退，请先重新请求并检查。

核验是插入前的检查，不是持续监控。插入后再发生权限变更时，网页草稿仍须由用户检查、移除。若重用 Bootstrap 时发现草稿中的旧块失效，扩展会尝试只移除未被改动的自有块；无法安全识别时要求人工删除，不覆盖用户编辑。已经交给网页或已经发送的资料无法通过缓存清理收回。

打开新对话、浏览器重载、SPA 路由切换、手工重置都会使旧绑定失效。新对话从 `/` 变成 `/c/<id>` 也视作新绑定，需重新获取 nonce；这是保守边界，不能沿用旧请求。切到另一个标签页不会把结果注入过去。

## 5. 请求格式

使用扩展“当前会话关联信息”中的真实值。下面的占位符不能直接执行；Bootstrap 预览会提供带当前值的示例。

````text
```recallcard-action
{
  "protocol": "recallcard.action/1",
  "request_id": "r_search_001",
  "nonce": "替换为当前nonce",
  "session_ref": "替换为当前session_ref",
  "action": "search",
  "arguments": {
    "query": "为什么选 Rust 和 Python",
    "target": "all",
    "limit": 5,
    "detail": "context",
    "budget_tokens": 1500
  }
}
```
````

- 只能粘贴一个完整的指定围栏，围栏外不要附加说明；未闭合、多个块、错误 JSON、重复键、未知字段和原型字段都会拒绝
- request_id 为 1–96 个英文字母、数字、下划线或连字符，首位为字母或数字；同一会话每次使用全新值
- 顶层 session_ref 用于浏览器会话关联；`arguments.session_ref` 是可选的 Vault 检索过滤器，两者不是同一个用途
- `bootstrap`: `{ "budget_tokens": 1800 }`
- `read`: `{ "refs": ["view:profile", "event:evt_实际编号", "memory:mem_实际编号@1"], "budget_tokens": 1800 }`
- `sources`: 同样使用 refs 数组，只接受 Event/Memory，不接受 View
- 最多 32 个引用；不允许路径或 URL；请求最多 16 KiB，草稿胶囊最多 64 KiB
- 参数 token 预算范围 256–16000，search limit 为 1–20；本机仍可执行更严格限制
- 每会话最多 128 次请求，包含插入前和 Bootstrap 重用时的新鲜度核验；相邻请求间隔至少 1 秒，必要的核验会短暂等待。失败也保留 request_id，以避免断线后的无声重复；用户检查原因后可以换新 ID 手工重试
- 15 秒未响应会报告超时，不自动重试，不会迟到后偷偷插入

nonce 是误触防护和关联字段，模型及网页可以看到，不能代替授权。只读、scope、引用、路径、预算和脱敏边界必须由本机核心再次执行。

返回资料以 `recallcard.context/1` 胶囊展示，保留 `origin: recallcard_context`、`synthetic: true`、request_id 和原始结果中的来源。它是参考资料，不是假装成平台原生 tool-result 的消息，也不能当作用户新的独立陈述。

## 6. 手动捕获与覆盖边界

```bash
# 手动复制后按输入格式保存 JSONL；参见 docs/usage.md
"$BIN" --vault "$VAULT" import --format manual-jsonl \
  --file /用户明确选定的/manual.jsonl --scope personal

# 用户主动下载并解压官方导出之后
"$BIN" --vault "$VAULT" import --format chatgpt-export \
  --file /用户明确选定的/conversations.json --scope personal
```

首次测试可把文件替换为仓库的合成 fixture `fixtures/manual-web.jsonl`，同时使用其对应的 scope。真实导出和私人资料留在自己的 Vault，不进入代码仓库。

扩展的 Native Messaging 接口始终只有四个读操作；上述写入只由用户在本地明确选择文件后执行。手工材料不能证明对话完整性：没有提供的文件、工具、引用、分支和时间不补造。原始消息 ID 可得时保留，否则使用明确的局部标识并承认身份较弱。

复制含 RecallCard 注入块的用户消息时，必须保留块级来源标记；不能把整个组合文本一概写成新的 `user_input`，不能把同一上下文在不同网页的复述当作新增独立证据。详见 [本地数据格式](usage.md)。

## 7. 安全和状态说明

- MV3 权限只有 `nativeMessaging`、`storage`，站点范围只有 `https://chatgpt.com/*`
- 页面适配在 Chrome 隔离环境执行；无 MAIN-world 桥、window.postMessage 接口、外部扩展消息接口、web-accessible resources、剪贴板自动读取或网络请求
- 只有扩展自己的准确 popup URL 可发起读动作；content script 仅绑定当前会话和执行经过检查的输入框操作
- 每次 native 操作前后都校验活跃标签、URL、顶层 documentId、当前绑定；后台重启从 `chrome.storage.session` 恢复去重记录
- session storage 暂存 nonce、已处理 ID、Bootstrap、最后预览及其原只读请求和结果指纹，不保存到磁盘 Vault、Git 或 storage.local。关闭标签会清理；浏览器重启、禁用、重载或更新扩展也会清空。Chrome 的会话存储不是抗本机恶意软件的安全边界。[Chrome Storage 文档](https://developer.chrome.com/docs/extensions/reference/api/storage)
- 导航使旧请求失效；能完整识别的旧草稿块会移除。用户改过的内容不强行删除，必须人工检查
- 附加的 DOM 操作依赖未稳定承诺的输入框结构。支持唯一可见的 `#prompt-textarea` textarea、带此 ID 的 ProseMirror contenteditable，以及 `textarea#mobile-composer-prompt`；从这些明确候选中要求只有一个可见且仍连接文档的输入框，禁用或只读则拒绝。多个候选同时可见、结构不明或找不到输入框时停止，不猜测任意 textarea/contenteditable
- 清理扩展状态或本机 Vault 不会撤回已交给网页、已发送或已经同步到其他设备的数据

Chrome service worker 可能被停止，因此不能只用内存 Set 去重；实现将请求预留记录先存入 session storage，再调用本机桥。[MV3 生命周期](https://developer.chrome.com/docs/extensions/develop/concepts/service-workers/lifecycle)

## 8. 精确的人工验收清单

这些是待在真实浏览器执行的检查步骤，不是已经完成的测试记录。请仅使用合成内容。

| 场景 | 操作 | 期望 |
|---|---|---|
| 新对话 | 准备 Bootstrap，暂不插入 | 只在扩展预览出现；页面没有消息发送 |
| 已有草稿 | 先写两行中英文和 emoji，再插入 | 原内容保留；上下文只追加一次 |
| 撤销 | 在块外增删文字后移除 | 块外修改保留；仅删除未改动的上下文 |
| 注入块被编辑 | 改块内一字后移除 | 明确拒绝自动移除，不覆盖草稿 |
| 重复 action | 同一 request_id 连续提交两次 | 第二次拒绝，本机不再次读取 |
| 中断回复 | 粘贴缺少结尾围栏的 JSON | 不发本机请求，不写入输入框 |
| 错误命令 | 将 action 改成 shell/capture/remember | schema 拒绝 |
| 跨会话 | 保存旧请求，切换另一对话再粘贴 | nonce/session 校验失败 |
| 重生成/消息编辑 | 只编辑或重生成网页回复 | 扩展不采集变化；手工复制新块才可能请求 |
| 标签切换 | 准备结果后切到另一 ChatGPT 标签 | 不跨标签注入；重新打开时使用另一绑定 |
| 本机缺失 | 准备预览后移走测试 host 注册文件，再尝试插入 | 核验失败，旧预览清空；没有自动重试或发送 |
| 遗忘与撤权 | 准备 search/read 预览，在本机抑制来源或撤销 scope，再插入 | 重新读取发现变化或拒绝授权，废弃旧预览，不写草稿 |
| Bootstrap 更新 | 准备 Bootstrap，修改受保护资料或授权，再点击重用 | 核验版本/稳定资料变化，清空旧预览，要求重新准备和审核 |
| 稳定快照 | 仅新增不改变 Bootstrap 的 Event，再检查 Bootstrap | 核验后继续用同一已审核文本，不重复注入 |
| 编辑器变更 | 测试环境移除已知输入框 ID | 安全停止，仍能选中预览手动复制 |
| 人工发送 | 自己点击网页 Send，再显式确认 | 只记录用户确认，不检测隐藏平台状态 |
| 后台重启 | 停止 service worker 后重复旧请求 | 从 session storage 恢复去重，拒绝重放 |

自动化回归命令：`node --test extension/tests/*.test.js`。测试覆盖实际协议/后台代码、内容脚本生命周期和模拟 DOM，不把模拟 DOM 当作真实 ChatGPT 页面验收。

## 9. 常见问题

- **本机桥找不到：** 检查 host 名称、注册目录、manifest 的 path、包装器执行权限和扩展 ID 是否精确匹配。不要改成通配授权
- **安装后没有输入框：** 刷新 ChatGPT 页面，确认在支持的对话路由；仍失败就手动复制预览，不自动探索未知页面结构
- **插入前提示资料已变化：** 旧预览已废弃。重新请求并检查新的资料，不要从其他窗口复制旧缓存；本机断线时同样不会继续使用旧预览
- **旧 request_id 不可重试：** 这是预留去重行为。确认未得到结果、修复 host 后使用新 ID，或显式重置会话
- **上下文仍留在旧草稿：** 用户改过的块不会被强删。先人工移除旧内容，再重置扩展。扩展刷新后丢失 DOM 所有权信息时同样需要人工清理
- **为什么没自动抓到刚才的聊天：** 本阶段只通过明确选定的复制/导出文件捕获，不扫描网页输出

进一步的 IPC、作用域和源码事实源约束见 [安全说明](security.md)；平台行为依照当前 [Chrome 消息文档](https://developer.chrome.com/docs/extensions/develop/concepts/messaging) 和 [Content Scripts 文档](https://developer.chrome.com/docs/extensions/develop/concepts/content-scripts) 复核。
