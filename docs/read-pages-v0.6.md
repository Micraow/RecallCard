# 模型读取：命中片段与有界续读

四个只读工具、授权范围与遗忘/撤权规则不变；CLI `mcp` 无需启动 GUI。

- 新参数 `budget_bytes` 是工具结果 JSON 的 UTF-8 字节上限，包含正文、出处、元数据、转义、游标及状态，不包含外层 MCP JSON-RPC 信封。默认 1500，允许 512–32768。它不是供应商 token 数。
- 旧 `budget_tokens` 仍作为已弃用别名接受，计量方式不变；两个名字不能同时传入。
- `search` 返回命中附近的连续原文、`text_truncated` 和 `text_range`。`status=no_matches` 是本次词法查询无命中；`budget_exhausted` 表示存在资料但当前预算不足。`next_cursor` 分页更多命中，不是正文续读游标。词法查询可能需要换关键字、旧称或原文语言，不保证同义词召回。
- `read` 对放得下的记录保留 `record`；大记录返回明确标记的 `text` 片段、`metadata`、`snapshot`、`record_complete:false` 和正文范围。`event.text` 是 content，或存在 parts 时各 part.text 以换行连接；`memory.content` 是记忆正文。范围使用 UTF-8 字节、左闭右开，不是字符位置。
- 使用相同的单个 ref 和 `next_cursor` 续读；或把 `search.text_range.start_byte` 传给 `offset_bytes` 直接定位命中。两者不能同时使用。`truncated=false` 表示没有后续正文，不代表本次提供了起点以前的内容或完整正本。引用 ref、snapshot 和字节范围用于稳定定位。
- 部分正文保留角色、来源、时间、修订关系、ChatGPT 当前分支标记（未知为 null）及 capture 的完整性/脱敏状态。过长的来源字符串和 time_note 有明确截断标记；完整记录中的其他任意 metadata 不自动转发。AI 不能把非当前分支、助手建议或旧 active 记忆直接当成当前用户决定。
- `sources(memory)` 的小集合继续返回完整 events；大集合返回有界 `source_refs` 和来源列表的 `next_cursor`。逐个 `read` event 得到原始证据。文件没有保留正文时明确报告，不能当作已有正文。
- 批量 refs 或 view 放不下时返回 `pending_refs`，改为逐个 read；view 不支持正文 offset/cursor。

游标不是能力令牌。每次读取重新核验当前范围、抑制状态和受管连接许可；绑定引用、正文快照、工具种类和授权范围。记忆更新或改变引用/范围会拒绝旧游标。不可变的旧 Event 依既有合同仍可作为历史证据读取，新修订 Event 是另一个引用。重启不影响未变化且仍授权的快照游标。

## 实际读取

先 search，再用命中返回的真实 ref 和 start_byte：

```json
{"refs":["event:evt_实际编号"],"offset_bytes":实际起点,"budget_bytes":8192}
```

若响应包含 next_cursor，以同一 ref 调用：

```json
{"refs":["event:evt_实际编号"],"cursor":"返回的正文游标","budget_bytes":8192}
```

这验证的是本地可读合同。真实宿主是否自动调用、是否依据原话作答，需要独立的实际 AI 接续验收；不能用协议测试声称已经连接 ChatGPT 或其他账号。
