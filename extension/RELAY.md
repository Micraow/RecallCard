# 网页轻悬浮接力（第一阶段）

## 使用路径

首次安装解压扩展后，从扩展弹窗复制 `recallcard-connect/1:扩展编号:安装编号`，在桌面安装本机桥，再点扩展“请求桌面连接”，回到桌面选择网站和范围并批准。扩展不能检测或代替完成浏览器商店安装；账号身份未核验。

已授权后，受支持网页出现轻悬浮窗：

1. 当前问题与 Bootstrap 使用说明留在网页草稿里，用户检查并点击网站发送。
2. AI 依问题提出 `search/read/sources` 请求，扩展检查会话、权限和消息完成凭据，准备完整资料。
3. 每轮由用户点击网站发送。AI 可按 `next_cursor` 续读长文，无需用户手挑记忆。
4. 本轮 AI 明确给出 `recallcard-final` 回执，并确认它位于当前资料发送后的完整助手消息时，悬浮窗显示“回答已就绪”。这只表示 AI 标记本轮回答完成，不验证回答正确或用户整个目标已完成。

暂停与关闭会停止该会话的自动捕获和资料读取。恢复会复用当前轮和已用 request_id，重新检查会话、安装身份、活动标签和权限修订，旧请求不会重新插入。

## 安全回退

- 没有唯一输入框，或输入框在资料读取期间被修改：不写入，保留完整结果；点击“复制本轮资料”，再自行粘贴到网站发送。
- 无法可靠读取网页：点击“准备使用说明”，复制到网站；将 AI 的完整 `recallcard-action` 请求粘贴至悬浮窗“网页改版或手动接力”。不能用任意网页文字代替完整请求。
- 网站渲染 `pre/code` 时，会重建 JSON 中明确声明的 RecallCard 协议围栏，再交严格解析器校验；无完成凭据、半截代码块、多块或杂糅说明均不自动执行。
- 仅 1.5 秒静默并不代表流式输出已完成。自动请求同时要求可识别的该条助手消息完成凭据。DOM 改版后可能需要手动复制完整请求。
- 草稿消失不是发送证据。自动确认要求当前 nonce/request_id 的资料标记出现在稳定用户消息中；也可点击“我已发送”。复制本身不算发送。
- 刷新、不同文档、SPA 路由或输入框身份变化会轮换会话；不跨标签、nonce 或安装绑定重放操作。浏览器会话存储保留暂停与轮次；刷新后的新文档必须重新建立关联，不盲目恢复旧草稿。
- 任何情况下都不调用网站 Send、模拟 Enter 或提交表单。

## 剪贴板边界

本阶段不声明“自动读写 OS 剪贴板”。能力接口只报告当前页面是否存在写入 API；写入须由用户显式点击，并检查当前页面可见且有焦点。失败时选中完整文本供用户手动复制。读取仅靠用户主动粘贴到悬浮窗，没有 `readText` 调用、后台监听、全局采集或权限申请。测试仅使用原创合成文本与模拟剪贴板。

## 能力矩阵

| 产品与限定站点 | 草稿 | 会话/请求读取 | 回退 | 验证边界 |
| --- | --- | --- | --- | --- |
| ChatGPT `chatgpt.com` | 精确输入框 + 输入快照保护 | 有稳定消息 ID/角色和明确完成凭据的可见文字 | 完整复制/粘贴 | 原创 DOM fixture；真实账号未验收 |
| DeepSeek `chat.deepseek.com` | 精确 textarea + 输入快照保护 | 同上；身份或角色不明的消息跳过 | 完整复制/粘贴 | 原创 DOM fixture；真实账号未验收 |
| Qwen `chat.qwen.ai` | 实验性精确 textarea | 手动接力，不自动捕获 | 完整复制/粘贴；切换后显式重新连接 | 不推广到 Qwen Studio 或其他通义产品 |
| Z.ai `chat.z.ai` | 实验性精确 textarea | 手动接力，不自动捕获 | 完整复制/粘贴；切换后显式重新连接 | 真实账号未验收 |
| Qwen Studio | 未确认站点 | 未实现 | 待明确站点 | 不列入支持主机 |

`RecallCardSites.forUrl().capabilities` 也提供同样的机器可读边界。可用的选择器并不等于真实网站已验证。

## 协议与状态合同

`publicState.relay`：`enabled`、`paused`、`phase`、`round`、`request_id`、`detail`、`manual_fallback`、`result_ready`。

`phase`：`idle` / `preparing` / `awaiting_user_send` / `waiting_reply` / `result_ready` / `paused` / `blocked`。

网页 UI 控制只接受同一扩展、顶层文档和当前活动标签的 `relay_control` 消息，不建立 webpage postMessage 桥。命令：`inspect`、`bootstrap`、`pause`、`resume`、`disable`、`enable`、`copy`、`delivered`、`execute`。`execute` 仍只接受 bootstrap/search/read/sources，不能设置权限、保存会话或执行写动作。捕获写入仍由既有、独立的 native 授权通路处理。

四工具都支持 `budget_bytes`（512–32768，UTF-8 JSON 字节）；`budget_tokens` 仅为互斥兼容别名。`read/sources` 支持单引用 `cursor` 或 `offset_bytes`，二者互斥；`search` 支持 `cursor`。结果保留完整 JSON 的 `status`、`text_range`、`snapshot`、`next_cursor`、`pending_refs`，不把 `partial`/`budget_exhausted` 伪装成完整或无结果。每页使用新的 `request_id`，按相同动作、引用和游标继续。

## 可重复验收

- `cd extension && npm test`：Node 单元、协议、生命周期、权限、轮次、剪贴板边界和输入保护回归。
- `cd extension && npm run test:browser`：Chromium 原创页面 fixture；新增 `browser-tests/relay.test.mjs` 完整跑 ChatGPT/DeepSeek 的三轮长文接力、渲染代码块、final、流式安全、异步编辑、剪贴板 fallback、暂停恢复与未知 DOM。
- 浏览器 fixture 拦截所有网站请求，使用合成本机桥；不安装扩展、不登录真实网站、不调用云模型、不接触真实剪贴板。
- 本环境的 Chromium 启动被限制时，只走已授权 CI。语法检查或 Node 通过不能表述为 Chromium 或真实网页验收通过。
