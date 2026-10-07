# 手动浏览器适配 v0.3

本页补充 [浏览器安装与操作说明](browser.md)。本批扩展版本为 `0.2.0`，新增 **Qwen、Z.ai 实验性输入框适配**；DeepSeek 仍未启用。所有平台沿用本机只读 Native Messaging、可见预览、用户手动粘贴 action、人工最终发送，不抓取对话和流式输出，不自动登录。

## 1. 精确范围

| 平台 | 允许的 origin / 路径 | 输入框候选 | 验证范围 |
|---|---|---|---|
| ChatGPT | `https://chatgpt.com`；保留 `/`、`/c/<id>`、`/g/<id>`、`/g/<id>/c/<id>` | `#prompt-textarea` textarea 或其 ProseMirror contenteditable；`textarea#mobile-composer-prompt` | 合成测试；2026-10-07 guest 页仅确认 mobile DOM 存在，不是安装扩展 E2E |
| Qwen | `https://chat.qwen.ai`；仅 `/` 和 `/c/<UUID>` | `textarea.message-input-textarea` | 固定公开源码/DOM fixture 的结构证据及本项目合成测试；未做真实站点插入/撤销验收 |
| Z.ai | `https://chat.z.ai`；仅 `/` 和 `/c/<UUID>` | `textarea#chat-input` | 固定公开源码/DOM fixture 的结构证据及本项目合成测试；未做真实站点插入/撤销验收 |
| DeepSeek | 无启用 origin | 未启用 | 输入框有公开结构证据，但会话识别仍有未解决风险，不开放权限或插入 |

UUID 必须是 `8-4-4-4-12` 的十六进制格式。Qwen/Z.ai 暂不接受 query 或 hash；`/login`、`/auth`、`/share`、`/settings` 及未知路径一律拒绝。`qwen.ai`、`qwenlm.ai`、`z.ai` apex 和其他子域不在 allowlist；浏览器若正常重定向到已支持的精确 origin，按重定向后的地址判断。页面、sender origin、当前 tab URL、content URL 和声明的 route 必须匹配。

Manifest 只列 `chatgpt.com`、`chat.qwen.ai`、`chat.z.ai` 三个 HTTPS host，没有泛域权限。路径限制同时在 popup、broker、content 和 composer 层执行，不是只靠输入框 selector。扩展更新增加了两个站点的权限范围，使用者应在 Chrome 的扩展权限界面审阅后自行决定是否启用；项目代码和测试不会代为授予浏览器权限。

## 2. Qwen / Z.ai 使用顺序

1. 在自己的浏览器加载/更新扩展并刷新目标页面，进入上表允许的对话地址
2. 首次使用实验平台必须点击“重置 / 重新附上说明”，由自己确认当前准备操作的是哪一个对话；未确认前不能读取本机或插入
3. 点击准备 Bootstrap 或手动粘贴完整 `recallcard-action` 块，检查可见结果和来源
4. 检查 popup 中列出的当前网站及隐私提醒，再点击追加到草稿。插入已使该网站接触资料，即使还没有发送
5. 输入框必须是唯一可见、连接文档、可编辑的精确候选。多个候选、错误类型、禁用、只读、ARIA 禁用/只读、隐藏或结构变更均停止
6. 追加保留原草稿；撤销只移除完整、未被编辑或复制的自有块。扩展不点击网页按钮、不模拟 Enter、不调用提交表单
7. 最后由自己点击网站发送；扩展里的确认按钮只是记录用户声明，不能证明模型已经收到

Qwen 不用 placeholder 或任意 textarea 猜测输入框，隐藏/只读的代码编辑器 textarea 不会匹配所选 class。Z.ai 只使用限定为 textarea 的 `#chat-input`。不支持把陌生 contenteditable 当成替代输入框。找不到时停止，用户仍可自行检查预览后手动复制。

## 3. 会话身份与保守限制

- 状态按 tab、documentId、精确 origin/route、content 随机 token 和 nonce 绑定，session_ref 使用 `chatgpt:`、`qwen:` 或 `zai:` 前缀
- 路由变化、页面重载、输入框节点更换/消失/不可用都会使旧绑定失效；即使 URL 相同，换节点也不能沿用旧 capsule
- 绑定请求返回前若页面/输入框已改变，迟到结果不能恢复旧绑定
- 实验平台上述变化后必须重新点击重置；旧 action、预览和 nonce 不能直接跨站或跨绑定使用
- **同 URL 且复用同一个输入框节点，不足以证明仍是同一对话。** 扩展没有平台内部会话 API，也不扫描对话来猜测。Qwen/Z.ai 初次进入、主动切换对话或不确定时，用户必须先重置，再重新准备/审核资料；UI 明确提示这个限制，不宣称已经自动识别全部会话切换
- 插入前仍重新执行同一只读请求，核验本机内容/授权/版本的新鲜度。这个核验不能代替上面的网页会话判断，也不能收回已交给网页的文字

DeepSeek 的一个第三方项目报告过切换对话而 URL 不变。我们没有复现它，也不把它当作所有当前版本的已证实行为，但它足以阻止在缺少稳定会话证据时开放自动追加。仅补一个 selector 或检测节点替换无法解决同 URL、同节点复用的问题。本批不启用 DeepSeek host 权限、会话绑定或 composer；后续须先获得可公开验证的会话边界，再独立验收，不能通过抓取私有状态或回复内容补足。

## 4. 公开证据与独立实现

核对日期：2026-10-07。以下来源用于确定 host、路径和元素属性，不代表站点对第三方扩展的许可或稳定接口承诺；使用者仍须遵守适用于自己账号与地区的条款。

- Qwen 官方仓库 README 链接 `chat.qwen.ai`：[QwenLM/Qwen3 固定版本](https://github.com/QwenLM/Qwen3/blob/7a2f61ffc7a20d47efcd2bf97f6f2bf52729042e/README.md#L8)
- Qwen `/c/<UUID>` 的公开问题报告：[Qwen issue 2300](https://github.com/QwenLM/Qwen/issues/2300)、[issue 2070](https://github.com/QwenLM/Qwen/issues/2070)。它们只提供已出现路径的证据，不是 UI 验收
- Z.ai 官方入口：[z.ai](https://z.ai/)；本次公开入口转向 `chat.z.ai`，只启用后者
- DeepSeek 官方入口：[deepseek.com](https://www.deepseek.com/) 链接其网页版 `chat.deepseek.com`，但本批仍不启用
- 第三方 ACE 固定 commit `cd63278cdecd3820ba8c2199500145e68648c0d5`：[输入框配置](https://github.com/Covai-Labs/ace/blob/cd63278cdecd3820ba8c2199500145e68648c0d5/content/transfer/platform-inputs.js#L21-L51)、[平台入口](https://github.com/Covai-Labs/ace/blob/cd63278cdecd3820ba8c2199500145e68648c0d5/content/transfer/targets.js#L9-L24)
- 同 commit 的 [Qwen DOM fixture](https://github.com/Covai-Labs/ace/blob/cd63278cdecd3820ba8c2199500145e68648c0d5/tests/fixtures/qwen-chat.html#L960-L966) 给出 `textarea.message-input-textarea`；[Z.ai DOM fixture](https://github.com/Covai-Labs/ace/blob/cd63278cdecd3820ba8c2199500145e68648c0d5/tests/fixtures/z-ai-chat.html) 给出 `textarea#chat-input`
- [DeepSeek DOM fixture](https://github.com/Covai-Labs/ace/blob/cd63278cdecd3820ba8c2199500145e68648c0d5/tests/fixtures/deepseek-chat.html#L740-L750) 中有 `textarea.ds-scroll-area[name="search"][placeholder="Message DeepSeek"]`，仅记录证据，不纳入当前 selector allowlist

没有复制第三方实现、完整 HTML、对话文本或媒体。该第三方 fixture 另有许可边界，不能因为仓库代码开放就复制其内容。本项目只独立实现最小 selector/校验逻辑，并用自建的少量 Element 对象和合成文本构建测试；没有新增 npm 依赖。

## 5. 本次测试与尚未完成项

2026-10-07 在 dot 云端 Node `v24.19.0` 运行：

```bash
node --test extension/tests/*.test.js
```

**74/74 通过**，从上一批 52 项新增 22 项。覆盖 host/路由限制、跨平台 sender 和 action 错配、DeepSeek 拒绝、唯一编辑器、不可写/不可见拒绝、草稿追加和撤销、实验平台显式重置、popup 可用性与接收网站说明、同 URL 节点替换失效、迟到绑定拒绝，以及本机错误/跨站导航不留下可插入预览。

这些是 Node 合成 fixture 与协议回归测试，不是 Qwen/Z.ai 登录态浏览器 E2E。公开研究本轮遇到 Qwen 设备不支持页面、Z.ai app shell、DeepSeek 403，均不能算 composer 实机验收。真实 ChatGPT guest 页只观察到了 mobile textarea，未安装扩展做插入/撤销。仍需用户授权的真实浏览器安装、合成草稿插入/撤销、登录态路由与同页会话切换检查；本批没有自动登录或自动发送测试消息。
