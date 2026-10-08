# 自动连接上下文（0.7 开发中）

本轮让一次授权后的 Agent 自己读取背景，不再要求用户每次选上下文。代码与定向合成验证不代表真实 Codex、Claude Code 或浏览器账号已接入；尚未通过本轮原生整体验收时，不作为候选发行版。

## 项目接入

CLI 与桌面共用 `connection-setup` 计划与确认服务。选择客户端、资料范围和项目目录后，先展示修改路径和 RecallCard 管理的增量；确认后才合并项目文件并授予这一个项目读取权限。不会修改全局用户配置，不会执行 Codex 或 Claude。

```sh
recallcard --vault /path/to/vault connection-setup plan codex --scope personal --project-dir /path/to/project
recallcard --vault /path/to/vault connection-setup apply <plan_id>
recallcard --vault /path/to/vault connection-check <connection_id> --scope personal --verify
```

Claude Code 的客户端参数是 `claude_code`。计划过期、配置被修改、读取程序变更、路径含符号链接或授权被撤销时，必须重新预览。其他 MCP 服务、指令及设置保留。配置摘要按整个文件核验，因此无关设置变化也会保守地提示重新核对。

Codex 项目设置仅在其受信任项目中生效；Claude 项目 MCP 也可能要求宿主确认。这里的本机试读不会替用户接受宿主权限，也不证明模型已理解这些资料。

## 文件正本与自动入口

```text
vault/
├── events/                       原始事件正本
├── memories/                     有引用、证据性质和状态的记忆正本
├── control/                      schema、抑制规则和整理收据
├── objects/                      保留的对象
├── generated/
│   ├── bootstrap/conn_<id>/context.json  按连接和当前范围再生的文件快照
│   └── views/                    旧人类视图；不作为自动授权入口
└── .index/                       可重建检索索引
```

项目 AGENTS.md / CLAUDE.md 只含接入指令和固定连接入口，不复制私密全文。入口会让宿主自动刷新 `connection-context`，再读取它返回的文件路径，或通过只读 MCP 检索。资料库、范围与连接编号由本机配置固定，模型不能扩大它们。

文件快照含有限的有效记忆、原始会话目录及原文引用。没有整理出的记忆时仍能从目录继续检索来源；不会把原始对话或助手建议伪装成确认结论。更多内容使用相同的 bootstrap / search / read / sources；长文本通过游标和 UTF-8 字节位置继续读取。

所有正本写者在获得写锁后先使旧连接快照失效；授权变化或撤销也会删除旧快照。下一次入口调用从正本重新生成并原子替换，因此重启不会依赖旧缓存。手工改动磁盘正本绕过程序写入的情况不具备即时失效通知，入口刷新仍重新校验。不要让宿主永久加载旧快照。已经被宿主读入会话的文字无法收回；同一系统用户直接访问整个 Vault 的权限也不是 RecallCard 连接授权能够限制的沙箱。

旧 `generated/views/memories.md` 没有连接范围合同，不能直接塞进所有 Agent 的启动指令。

## 状态说清楚

- 配置已写：文件摘要与本次计划一致，不等于宿主已连接。
- 本机试读通过：真正启动 RecallCard CLI 读到了当前授权资料；不运行外部 Agent，不制造宿主读取时间。
- 收到客户端读取：带此连接编号的请求执行过只读接口；不能证明模型采用了答案。
- 写回未启用：本轮 Codex / Claude 项目接入只读。浏览器捕获需要单独授权。
- 浏览器测试版：包中有可加载扩展，但尚未上架，未提供固定商店链接。首次 Native bridge 需准确绑定本机扩展ID，之后可通过配对申请确认范围。Qwen Studio 不列为已验证自动接入。

## 官方宿主合同

配置依据 [Codex 的 AGENTS.md](https://learn.chatgpt.com/docs/agent-configuration/agents-md)、[Codex MCP](https://learn.chatgpt.com/docs/extend/mcp?surface=cli)、[Claude Code 项目记忆](https://code.claude.com/docs/en/memory)及[Claude Code hooks](https://code.claude.com/docs/en/hooks)。实际账号、宿主版本与宿主审批仍须单独验收。
