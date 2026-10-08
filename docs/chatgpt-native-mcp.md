# ChatGPT 原生接续：最小接口与待授权边界

状态：本机只读下游已实现并做合成验证；真实 ChatGPT 插件与隧道未接入。这里不把本机响应当作用户账号已经连通。

## 能去掉什么

ChatGPT 读取背景应走原生 MCP 工具结果，不经网页草稿转发。用户发送一次任务后，模型可按需调用 bootstrap/search/read/sources，不需要用户为每个读取结果再点 Send。

官方依据：[自定义 MCP 插件](https://developers.openai.com/api/docs/guides/custom-mcp-server)、[Secure MCP Tunnel](https://developers.openai.com/api/docs/guides/secure-mcp-tunnels)。后者允许官方客户端转发本机 stdio，无需为 RecallCard 自建 HTTP 服务或公网入口。Tunnel 不等于本地资料永不外传：被批准的工具结果会提供给 ChatGPT / OpenAI。

## 现有能力与最小补齐

已有：MCP stdio 生命周期、四个只读工具、readOnlyHint、固定启动范围、逐次读取当前授权、撤销、原文/记忆的最新抑制状态。

本轮仅新增：

- 独立授权类型 `chatgpt_mcp`，固定接收方身份 `openai-chatgpt`、平台 `chatgpt`；不能借用浏览器安装或 Claude Code 授权。
- 只能授予读取范围；捕获范围和自动捕获必须为空/关闭。来源保存仍属于独立流程。
- 桌面 `chatgpt_connection_plan(session_id, scope, packaged_binary)` 返回本机 stdio command/args、当前授权状态及官方入口。
- 该方法不创建授权、不启动隧道、不检查或保存凭据、不启动监听；本机存在调用回执时仍返回 `upstream_verification: not_checked`。

本机启动说明形态：

```text
recallcard --vault <固定本机目录> mcp --scope <当前空间> --connection-id <独立ChatGPT授权编号>
```

启动说明不是官方 tunnel-client 的成品配置文件，也不是 ChatGPT 可直接访问的本机 URL。实际设置时使用官方受支持的 stdio 配置形式；不拼接网页或模型传入的命令、路径、scope。

## GUI 只展示一个主要入口

“连接 ChatGPT”展示当前空间与接收方，在用户明确允许后保存该空间的只读授权。然后引导完成官方连接；内部 stdio、native messaging、scope、授予版本不作为多套必填概念暴露。详细启动说明仅用于诊断。

本机准备状态为 permission_required / ready / paused / revoked；实际 ChatGPT 连接另行验证。不能把工具调用来自本机测试，或模型自报 clientInfo，当作官方账号认证。

## 真实账号验收的最少前置

1. 可运行包安装到用户选定、会持续使用的电脑，资料也在该目标电脑。临时开发/验收环境不作为长期私人记忆服务，不为它建立用户的长期隧道。
2. 用户在一个清晰步骤中确认：仅把当前个人空间中命中的背景、原文和出处交给 ChatGPT / OpenAI。数据正本留在本机，检索结果仍会离机；两者不是同一个隐私承诺。
3. 目标账号确实能够添加私人 MCP 插件，并具备所选个人 Tunnel 所需权限；已有配置可复用，不能默认向整个团队或其它空间开放。
4. 在官方安全流程完成连接凭据与私人插件授权。应用负责本机启动/参数/状态，不要求用户编辑多份配置文件；外部账号授权尚未完成时明确停在待连接。
5. 先用一条合成记录验证真实工具读取、范围隔离和撤销，再用用户批准的实际资料验证接续。

## 还需用户明确批准的事项

- 允许把选定空间的背景、原文和来源，通过只读工具提供给 ChatGPT / OpenAI；不默认整个资料库。
- 为目标个人账号/空间创建或使用 Secure MCP Tunnel，确认关联的组织和 ChatGPT 空间，不把访问范围扩大到团队。
- 安装官方 tunnel-client、创建及安全配置其运行凭据、允许所需的持续访问。凭据不进入项目、日志或聊天；这里尚未执行。
- 在 [ChatGPT Plugins](https://chatgpt.com/plugins) 添加、安装该私人 MCP 插件并审阅官方风险/权限确认；具体可用性取决于用户账号与管理限制。[Tunnel 管理入口](https://platform.openai.com/settings/organization/tunnels)。

不为通过测试配置 No authentication、公网代理或隐藏的宽授权。实际认证和工作空间边界必须在官方设置时审阅。

## 一个明确的真实接续验收

先只用用户愿意共享的一段来源。在 ChatGPT 选择 RecallCard 插件并发送：“接着这段已有决定往下做，先找依据再回答。”

验收必须同时看到：真实插件调用找到该来源、读到正确原话和出处、无需搬运工具结果或额外 Send、其他空间不可读取；撤销授权后，同一个已连接会话的新读取被拒绝。

当前合成验证只能证明 RecallCard 的 stdio 下游能完成这条读取链，不能证明 Tunnel、ChatGPT 账号权限、模型选工具行为或真实接续体验已经成立。整理模型/API key 不应成为检索已有来源的前置条件。

DeepSeek 原网页版没有已核实的同等入口，保留明确受限路径；不能因它的限制把 ChatGPT 降级到同一草稿循环。已有旧草稿授权迁移时，应明确停用重复的自动准备，避免两条通道同时注入。
