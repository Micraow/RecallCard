# 浏览器会话保存、导出与跨 AI 上下文 v0.3

扩展 `0.3.0` 的主入口是「保存 / 导出当前会话」与「为另一个 AI 准备上下文」。本机资料请求、Bootstrap、可撤销草稿和人工发送仍在「高级」区。扩展运行时没有第三方依赖。

## 使用顺序

1. 在自己的 Chrome 更新扩展并刷新 ChatGPT 或 DeepSeek 的会话页面，打开 RecallCard。顶部显示当前网站、网址和本机连接状态。
2. 点击「读取当前可见会话」。打开弹窗本身不会读取消息文字；首次点击才捕获普通可见文字。
3. 逐条检查原话、角色与时间，勾选要保留的消息。可展开每条全文。原时间不可得时显示「原始时间未知」，不会把读取时间当成消息时间。
4. 保存到资料库：点击「保存到 RecallCard：查看脱敏预览」，检查本机核心返回的样本，再单独点击「确认保存这些消息」。只有本机确认 `events_added / events_seen` 后才显示已保存。需要已安装本机桥，并在桌面连接设置中独立授权保存范围。
5. 导出文件：点击 JSON 或 Markdown，选择下载位置。两种格式都含完整的所选消息，无需连接本机桥。JSON 可在桌面导入；Markdown 把原文放在可安全容纳原有围栏的文字代码块内，避免脚本与 HTML 成为活动内容。
6. 跨 AI：点击「用所选消息准备上下文」，检查并复制，在目标网站粘贴。原话、来源、覆盖说明一起带过去。最终发送始终由用户完成。

导出与保存是两个独立结果。下载完成不表示已经入库；本机断线不阻止导出。下载取消或保存失败不会伪报成功，预览仍可重试。若保存请求已发出但最终确认断线，提示结果未确认，用户可在桌面核对；重试同一快照由核心幂等处理。

## 平台覆盖

| 平台 | 可见会话捕获 | 角色证据 | 草稿输入框 |
|---|---|---|---|
| ChatGPT | 支持；实际登录态完整兼容性待验收 | 明确 `data-message-author-role`，优先保留 `data-message-id` | `#prompt-textarea` textarea / ProseMirror；`textarea#mobile-composer-prompt` |
| DeepSeek | 支持；页面结构来自固定公开源码证据，实际登录态待验收 | `.ds-message` 中显式角色优先；普通 `.ds-markdown` 表示 AI 回答；无可靠角色标记必须由用户在预览确认 | `textarea.ds-scroll-area[name="search"]` |
| Qwen / Z.ai | 暂不读取，UI 明确说明 | 不按消息顺序或奇偶猜测角色 | 沿用实验性输入框适配，切换对话需显式重置 |

DeepSeek 仅允许 `https://chat.deepseek.com/` 和 `/a/chat/s/<id>`，不接受 query/hash、登录、共享和未知路径。不会仅凭低稳定性的哈希类（如 `.fbb737a4`）认定用户角色，也不会把思考区域或其内嵌 Markdown 当成最终回答。

本轮核对的结构证据：

- [DeepSeek-Exporter selectors.js，固定 commit dadfbc1](https://github.com/sati0121/DeepSeek-Exporter/blob/dadfbc13470b8c9af7885f59794ef722d565f482/public/content/selectors.js)：`.ds-message`、`.ds-markdown`、`.ds-think-content` 与可见生成状态选择器
- [ACE DeepSeek fixture，固定 commit cd63278](https://github.com/Covai-Labs/ace/blob/cd63278cdecd3820ba8c2199500145e68648c0d5/tests/fixtures/deepseek-chat.html)：输入框属性证据

只参考可公开验证的元素属性，未复制这些项目的实现、完整网页、真实会话或素材。测试全部使用本项目自建最小 HTML 和合成文字。

## 诚实的覆盖与身份

导出恒为 `coverage.extent = visible_only`、`complete = false`。这是当前网页已加载且可见的文字片段，不是厂商账户的完整历史备份。扩展不自动滚动、不读取私有 API、网络流、Cookie、令牌、隐藏状态、推理内容或附件原件。页面显示仍在生成时拒绝捕获；空文字记录不会生成文件。未识别的消息结构可能遗漏，覆盖声明不会因此升级为“完整”。

- 从已验证会话路径提取 `conversation_id`；根页或没有可靠 ID 时用 `capture:<capture_id>`，说明这是局部会话
- 可见、合法且不重复的网站 message ID 优先；否则用 `<capture_id>:<当前序号>`，附 `metadata.weaker_identity = true`
- 同一文件再次导入可幂等；缺少稳定 ID 的两次新捕获不保证跨次去重
- 只有可见 `time[datetime]` 的带时区 ISO 原时间才写入 `occurred_at`，否则为 `null`
- URL、文档、输入框节点、可见消息集合或消息内容发生变化会废弃旧预览和注入绑定；需要重新确认当前会话
- 每次导出、复制与保存前重新核验完整原始快照，不只比较头部或长度

## 交换文件

协议为 `recallcard.conversation/1`。合成验收样本在 [extension/tests/fixtures/conversation.json](../extension/tests/fixtures/conversation.json)。顶层字段：`schema`、`capture_id`、`captured_at`、`title`、`source`、`coverage`、`messages`、可选 `metadata`。每条消息只有 `id`、`role`、`text`、`occurred_at` 和可选 `metadata`；最终 `role` 仅为 `user / assistant`。

`metadata.snapshot_hash` 是删除该字段后整个 JSON 的 SHA-256：对象键递归排序，数组保持顺序，按 JSON.stringify 的紧凑表示编码 UTF-8。`metadata.hash_algorithm` 为 `SHA-256`。该摘要检测快照变化，不是来源签名或授权。它不允许网页选择 Vault、scope 或执行动作。

每份文件最多 16 MiB / 5000 条消息，单条正文最多 2 MiB，超限明确拒绝，不默默截断。Native 直接保存请求另限制约 200 KiB，以兼容本机通道帧限制；较大会话可导出 JSON 后在桌面导入。

## 本机与模型边界

`connection`、`capture_preview`、`capture_save` 仅由精确扩展 popup 的用户操作发起。后台校验活跃 tab、route、nonce、session_ref、documentId 与原快照。核心从固定本机配置决定 `capture_scope`；请求不携带用户自选 scope。保存预览绑定核心 `approval_hash`、资料库 `connection_id` 与完整所选快照摘要，显示目的资料库名和 scope。确认保存前重新查询 connection；更换资料库、保存范围或取消授权会清除旧确认，必须重新预览。缺少 connection_id 的旧本机桥只允许原只读功能和文件导出，不能保存。改变选择也必须重新预览。

模型 `recallcard-action` parser 仍仅允许 `bootstrap / search / read / sources`；模型不能通过粘贴 action 调用保存。网页没有消息桥或网络权限。新增 `downloads` 权限只用于用户点击导出文件；新增的站点权限仅为精确 `chat.deepseek.com`，需用户在自己的浏览器审阅。

[OpenAI 使用条款](https://openai.com/policies/terms-of-use/) 包含自动或程序化提取的限制；本功能的用户主导可见 DOM 范围不是法律许可保证。使用者应遵守适用于其账号、地区和合同的规则；[官方数据导出](https://help.openai.com/en/articles/7260999-exporting-your-chatgpt-history-and-data) 仍是获取账户历史的另一条路线。遇到网站或执行环境的实际访问拒绝，不绕过。

## 验证记录

2026-10-07 Node 回归 90/90 通过，保留原 76 项并补充摘要、角色、转义、大小、保存确认、模型读写边界、失效与失败路径。真实 Chromium 合成测试新增 ChatGPT/DeepSeek 可见捕获、隐藏内容排除、未知角色、流式拒绝、同 URL 变化、popup 下载取消与保存失败；脚本保持启用，测试引擎结果须以实际运行记录为准。

运行：

```sh
node --test extension/tests/*.test.js
cd extension
RECALLCARD_CHROMIUM_PATH=/已安装的/chromium npm run test:browser
```

`test:browser` 使用单个测试任务，不安装或加载真实浏览器扩展，不访问厂商账号。已知本机 socket 策略与云浏览器扩展安装限制没有绕过。Node 与合成页面通过不能替代真实 Chrome 安装、Native Messaging 注册和登录态站点的端到端验收。
