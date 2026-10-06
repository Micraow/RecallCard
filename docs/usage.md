# 本地使用与数据格式

## 1. 创建、捕获和查询

`recallcard --vault <目录> init` 创建本地 Git 资料库，不配置远端。先使用仓库合成 fixtures 验证安装。

```bash
recallcard --vault <目录> import --format manual-jsonl --file fixtures/manual-web.jsonl --scope project:demo
recallcard --vault <目录> import --format claude-code --file fixtures/claude-code.jsonl --scope project:demo
recallcard --vault <目录> search "Rust 主程序" --scope project:demo
recallcard --vault <目录> read <evt_编号>
recallcard --vault <目录> doctor
```

每个导入文件必须由使用者明确指定。程序不自动扫描任何 Agent 的私有会话目录。ChatGPT 导入使用用户主动下载的 `conversations.json`，只沿 `current_node` 的 parent 链选择当前分支；没有可靠分支信息就拒绝，不把重新生成的多条分支拼成同一段事实。

导入格式存在版本差异。当前只覆盖测试 fixture 所描述的字段，未知内容明确跳过或报错，不保证全量工具、文件、引用和分支捕获。

## 2. 事件输入

`capture --file event.json` 或 `capture --file -` 接收一条 JSON。JSONL 用 `import --format manual-jsonl`。

```json
{
  "occurred_at": null,
  "scope": "project:demo",
  "role": "user",
  "origin": "user_input",
  "content": "合成示例：这个项目采用 Rust。",
  "source": {
    "platform": "manual-web",
    "account_namespace": "local-alias",
    "conversation_id": "demo-session",
    "message_id": "demo-message"
  }
}
```

- `occurred_at` 未知就保留 null；捕获时间另记，不冒充历史时间
- 支持 `message/lifecycle/tool_call/tool_result/file/citation/approval/custom`
- session/turn/run/step/reply_to/caused_by/revision_of 可选，不编造上游未提供的信息
- `parts` 可标记 `user_input/assistant_output/tool_output/external_quote/unknown/context_injection/recallcard_dream_job`
- 整个 Event 作为 Memory 证据时，混合了注入内容的记录保守拒绝；首版没有部分证据批准捷径
- 相同来源标识和规范化内容重复导入不增加事件；同消息内容变化追加修订；不同消息的相同文本不合并
- 事件编号使用来源与规范化内容的 SHA-256。Memory 编号使用随机 UUID；这是预发布实现对 v0.1 ULID 草案的具体选择

## 3. 人工记忆

`memory add --file memory-input.json` 是用户主动审核后执行的本地写入，不提供给模型工具。

```json
{
  "content": "合成示例：项目采用 Rust。",
  "scope": "project:demo",
  "source_refs": ["请替换为上一步实际 evt_编号"],
  "evidence": "user_explicit",
  "labels": ["architecture"],
  "time_note": "只知道陈述时间，不推断更早生效日期。"
}
```

有效 `evidence` 为 `user_explicit/observed/assistant_suggestion`，它描述证据性质，不是记忆类型分类。助手建议保存为 tentative，不能冒充已批准事实。人工新增默认 `authority=user`、`protected=true`。只有明确标记 `labels: [bootstrap]` 的受保护 active Memory 进入个人启动资料。

更新要求 `memory update <id> --revision <刚读取的版本> --file <文件>`；旧版本不会覆盖新修改。时间字段允许 null，模糊时间保存在 `time_note`，不自动把计划当完成。

`views` 重新生成 `generated/views/memories.md`，不会调用模型。原始 Markdown 是正本，生成视图不是正本。

## 4. 只读 MCP

参考 `integrations/claude-code/mcp.example.json`，将程序与 Vault 改成实际绝对路径。标准输入/输出只传 JSON-RPC，诊断写标准错误；不开放本机 HTTP 端口。

四个工具可直接访问，并非四道必须经过的关卡。`search` 支持 target、session_ref、as_of、limit、detail、budget_tokens 和失效检查游标；`read/sources` 支持批量 refs。引用不能变成本机任意文件路径。

当前预算采用 UTF-8 字节的保守上界，通常比模型真实 token 数更严格；不伪装成精确模型分词。全文过长时分批读取或提高预算。索引统计只在已授权资料中计算。

## 5. 遗忘、恢复和状态目录

```bash
recallcard --vault <目录> forget <id> --reason "用户主动要求不再召回"
recallcard --vault <目录> restore <id>
```

抑制 Memory 同时抑制其来源 Event，以阻止下一次提炼重新出现。范围较保守，可能一并屏蔽同条原话的其他内容。恢复必须显式执行。Git 历史、其他设备 clone、已发送到模型的内容不会因此被擦除。

配置和凭据不要放入 Vault。受限容器若无法写标准状态目录，可设置 `RECALLCARD_STATE_DIR=/可写路径`。不同 Vault 使用不同摘要子目录和操作系统文件锁；崩溃释放锁，不依赖删除锁文件猜测进程存活。

## 6. 注入内容再次导入

Core 会识别自己的 `recallcard.context/1`、`recallcard.dream-job/1`、`recallcard.dream-result/1` 标记，重新捕获时标成 synthetic origin，不把复述当新的独立记忆证据，也不混入默认文本检索。正文仍留在原始事件，可按明确编号审计。含用户附言与上下文的混合消息在首版整条保守排除，避免来源被清洗；不会假装已实现精确块级证据批准。
