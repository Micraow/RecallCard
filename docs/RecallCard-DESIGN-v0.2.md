# RecallCard：整体设计与开发规格 v0.2（架构复审稿）

> 日期：2026-10-06  
> 状态：保留已确认产品约束；新增内容是本轮审视后的**工程建议**，不是冒充用户已逐项确认的决策。  
> 适用对象：任何开发者或开发 Agent，不依赖 Dot、Codex 或 Claude Code。  
> 替代关系：本文整合并修订 `RecallCard-DESIGN.md` v0.1；旧文件保留作为讨论历史。  
> 验证边界：本轮完成文档与架构对照，**没有实现或实测端到端系统、模型记忆准确率、缓存命中率或实际账单**。

**Your memory, carried across every AI.**

## 阅读路线

- 产品与架构：§1–4。
- 数据与召回协议：§5–9。
- Prefix cache、Dream 与成本：§10–12。
- 接入、同步、安全：§13–16。
- 开发顺序、验收与决策日志：§17–19。
- 官方参考与证据边界：§20。

本文中的“必须”指推荐实现所需的不变量；不代表用户已批准每一个新增字段。协议示例是设计草案，应先用 fixtures 验证，再发布为稳定 API。

---

# 1. 结论：可以实现什么，不能承诺什么

RecallCard 的可实现目标是：**各个经过授权的客户端共享同一份可追溯历史、整理后的记忆和访问协议**。

目标包含三个不同能力：

| 能力 | 产品含义 | 主要实现路径 |
|---|---|---|
| 认识用户 | 新客户端知道稳定偏好、环境与限制 | 小型、受保护的 Bootstrap |
| 延续任务 | 刚在 Web 讨论，换到 Agent 能继续 | 已捕获 Events 的即时检索与显式交接 |
| 回顾长期历史 | 查询过去的决定、变化、原话 | Memory 检索、时间过滤、原始证据读取 |

**统一存储不等于统一行为。** 不同模型仍可能有不同的工具使用能力、理解能力、上下文窗口和安全约束；不同 Web 产品还可能注入自己的记忆。RecallCard 不承诺把所有产品变成同一个 ChatGPT，也不迁移供应商隐藏系统指令、隐藏推理、沙箱状态或账号权限。

“像认识我一样回答”的体验，需要整个链路都正常：

```text
正确捕获 → 不失真整理 → 知道何时检索 → 找到正确证据 → 正确使用
```

任何一环失效，文件和向量保存得再好也没有用。因此本轮重点不是增加记忆理论，而是补足这条链路的接口、证据和生命周期。

---

# 2. 保留的用户约束

| 已确认方向 | 本文处理 |
|---|---|
| 本地单体软件，Rust + Python | Rust 主程序；Python 是按需、可选 worker |
| 用户拥有文件，Git 管理与同步 | Events/Memory/必要控制元数据可版本化；无专属 SaaS |
| Web 和 Agent 都是输入端、输出端 | Capture 与 Context 两条独立通路 |
| Event + Memory 为知识对象 | 不增加独立 Episode；经历摘要仍可作为普通 Memory |
| Atomic records + human-readable views | 记录为正本；视图为可重建输出 |
| 不建立复杂 MemoryType | labels/entities 可选；不做认知科学分类树 |
| Dream 可手动、批量，也可交给 Web Chat | 与 API Provider 解耦 |
| 云 Embedding 可接受，本地可选 | 优先实现一个可配置云提供方，不强制下载模型 |
| 不需要实时向量化所有历史 | 先即时文本索引，向量批量更新 |
| Bootstrap + 模型主动渐进读取 | 不重新引入“输入几个字就猜 Topic”的前置路由器 |
| 四个只读能力 | bootstrap / search / read / sources |
| Web 最终 Send 必须由人触发 | 永不自动提交；不冒充 native tool-result 消息 |
| 不建设复杂 benchmark 平台 | 仍必须有少量正确性与集成回归用例 |

### 关于曾被误记为“已确认”的决策

此前讨论中，助手曾在没有收到对应用户选择的情况下，直接宣布 Dream 风险分级方案已锁定。这不能作为用户批准的证据。本文保留“受保护数据不能被自动覆盖”的方向；**详细自动批准规则属于实现建议**。

这也是 Dream 的一个真实测试案例：助手声称“用户已同意”，并不等于用户确实同意。

---

# 3. 相比 v0.1 必须补足的部分

| v0.1 的缺口或过强表述 | v0.2 建议 |
|---|---|
| 召回主要围绕整理好的 Memory | `search` 同时覆盖授权的 Events；未 Dream 也可查 |
| 渐进披露容易被实现为四步必经流水线 | 四个能力可直接访问、批量读取，不强制每层绕一遍 |
| 只有 MCP 工具，没有明确的首次/压缩后加载保证 | Adapter 负责 Bootstrap 生命周期，MCP 只是能力接口 |
| Profile 更新与会话前缀稳定没有协调 | 会话固定 Bootstrap 快照；变化追加为显式增量 |
| 注入内容可能被再次当作用户陈述学习 | 记录内容 origin、原始证据链与 synthetic 标记 |
| `.97 confidence` 容易被当作真实性保证 | 模型评分只是提示；接受规则依赖证据与保护策略 |
| 标记了时间，就仿佛能精确时态推理 | 未知/模糊时间必须保留；计划不能自动变成完成 |
| DreamResult 缺少基线/重复执行约束 | 来源哈希、目标版本、幂等 receipt、暂存提交 |
| 清空索引被描述成零成本完整恢复 | 文本索引可离线恢复；向量恢复可能需要模型/API |
| Git pull 在提交本地更改之前 | 先封段、校验、提交，再整合远端；冲突停止 |
| 处理过的 Memory 被当作更可信内容 | 总结不会自动消除注入风险或把错误变真 |
| 很多 Rust crates 与阶段先堆后连 | 先一个小纵切闭环，模块而非大量 crate |

不为这些修补引入新数据库、图谱服务、分布式锁服务或另一个 Agent Runtime。

---

# 4. 总体架构：两条读路径，一个整理过程

```text
             Web Chat                     Local Agent
                │                              │
         Browser Adapter               Hooks / Importers
                └───────────┬──────────────────┘
                            ▼
                  Capture + Normalize
                   Redact + Deduplicate
                            │
                            ▼
                    Events（原始正本）
                       │           │
            即时文本索引│           │ 手动/批量 Dream
                       ▼           ▼
                Raw event recall  Memory（整理正本）
                       │           │
                       │           ├── 确定性 Views / Bootstrap
                       │           └── 文本 + 向量索引
                       └──────┬────┘
                              ▼
                授权过滤 + 四个 Context 能力
                              │
              ┌───────────────┴─────────────────┐
              ▼                                 ▼
       MCP / 客户端加载适配               Web 手动发送桥
              │                                 │
       native tool calls                 可见 composer 内容
```

**即时文本索引不是实时 Embedding，也不是实时 Dream。** 它只是让已经落盘的内容尽快能被搜到。

增加这条路径不会增加第三种知识对象：最近事件、交接片段、会话摘要都只是 Event 的投影，或 Dream 产出的普通 Memory。

## 4.1 三个不同时间点

- `captured_through`：该来源成功捕获到哪里；来源时间不可靠时改用 source cursor/sequence。
- `dreamed_through`：哪些事件修订版本已被某次成功 Dream 处理。
- `indexed_generation`：当前索引反映哪一批正本数据。

这些属于运行/覆盖元数据，不属于用户知识，也不应每次塞进稳定 Prompt 前缀。

## 4.2 跨机器的边界

同机切换客户端：已捕获数据可即时使用。

跨机器切换：必须先完成 Git 同步及必要索引更新。系统只能声明“当前设备已同步到的历史”，不能假装已获得其他设备尚未推送的数据。

---

# 5. 文件布局与所有权

```text
vault/
├── events/YYYY/MM/<session>/
│   └── <device>-<segment>.jsonl
├── memories/
│   └── <memory-id>.md
├── control/                         # 可持久化的控制元数据，不是新记忆类型
│   ├── dream-receipts/
│   ├── suppressions/
│   └── schema-version.json
├── objects/                         # 按授权/体积策略保留的内容寻址对象
├── generated/
│   ├── views/
│   └── bootstrap/
└── .index/                          # 可重建的本机索引
```

程序配置、凭据、正在运行的任务锁、未完成事务位于平台标准配置/状态目录，默认不进入 Git。无秘密的共享策略可以显式版本化，但必须与 API keys 分离。

### 关键区分

- **知识正本**：Event、Memory。
- **控制正本**：如 suppression、成功 Dream receipt；用于阻止重复处理、重复复活，不应全部当缓存删除。
- **派生数据**：Views、Bootstrap、FTS、向量索引。
- **运行临时状态**：IPC、锁、暂存事务。

“只有两个知识对象”不等于“软件不能有配置和任务记录”。

## 5.1 Views 的可重建含义

v1 的 Views/Bootstrap 用确定性模板从 Memory 与用户保护设置构建：稳定排序、稳定格式、无每次构建时间戳。

如果需要更流畅的跨记录叙事摘要，可以在 Dream 中生成一条普通 Memory，再把它展示到 View。**不要每次启动、Git pull 或 rebuild 都重新调 LLM 写 Profile**。

直接编辑生成的 Markdown 不应悄悄成为正本：文件头标明 generated；通过 `edit/pin/import-edit` 将变更转成用户主导 Memory，再重建视图。

## 5.2 可读性与机器接口

人和可信本地 Agent 能直接读取文件；MCP 提供更高效的筛选和预算控制。

文件可读不意味着任意 Agent 都应获得整个 Vault 的路径访问。需要隔离时，只提供受限 MCP 或只读、已过滤的导出目录；MCP 权限无法阻止另有任意文件权限的进程。

---

# 6. Event：保存可获得的事实，不伪造完整性

## 6.1 核心模型

沿用：

```text
lifecycle / message / tool_call / tool_result /
file / citation / approval / custom
```

Trace 由 run、step、caused-by 等关联表达。不要记录每个 token delta；适配器合并流式片段。

### 推荐 Envelope 草案

```json
{
  "schema": "recallcard.event/1",
  "id": "evt_demo_01",
  "session_id": "ses_demo_01",
  "turn_id": null,
  "run_id": null,
  "step_id": null,
  "occurred_at": null,
  "ingested_at": "2026-10-06T10:00:00Z",
  "source": {
    "adapter": "chatgpt-web",
    "adapter_version": "0.1.0",
    "client": "web",
    "provider": "openai",
    "account_namespace": "local-account-alias",
    "external_session_id": "provider-session-id",
    "external_event_id": "provider-message-id"
  },
  "scope": "personal",
  "revision_of": null,
  "reply_to": null,
  "caused_by": null,
  "kind": "message",
  "payload": {
    "role": "user",
    "parts": [
      {
        "type": "text",
        "text": "这个项目决定采用 Rust + Python。",
        "origin": "user_input"
      }
    ],
    "completion": "complete"
  },
  "capture": {
    "completeness": "partial",
    "redacted": false,
    "reason": "only_visible_message_available"
  },
  "native_ref": null
}
```

### 字段约束

- `id`、`session_id`、`ingested_at`、来源、kind 不为空；缺失 provider session id 时生成局部 session，标明 synthetic binding。
- turn/run/step 不可观察时为 null，不能为了满足层级而编造执行细节。
- `occurred_at` 不知道就为空；不要用导入时间伪装历史发生时间。
- `reply_to` 表示对话关系；`caused_by` 表示执行因果；`revision_of` 表示修订，不混用一个万能 parent 字段。
- Branch 信息可以在能获取时写入来源扩展元数据；不能把 regenerate 的多个分支拼成同一条线性真相。
- Hash/模型名称/文件 diff 没实际读到时为空；不得凭命令名推断文件已经修改成功。

## 6.2 幂等捕获与修订

建议 source key：

```text
adapter + account_namespace + external_session_id + external_event_id
```

同一个 source key、同一规范化内容版本重复导入：NOOP。

同一个 source key 内容改变：追加 revision，不改写封闭事件段。

完全相同文本出现在两个不同消息中：仍是两条事件，**不能只靠内容哈希去重**。

没有可靠 source id 的 DOM adapter：用 session 内稳定位置/角色/内容指纹辅助，标明 weaker_identity；宁可报告可能重复，不要暗示绝对去重。

## 6.3 内容分块与来源标记

一个 `role=user` 消息可能同时含有用户输入与插件注入内容。必须在内容块级区分：

```text
user_input
assistant_output
tool_output
external_quote
recallcard_context
recallcard_dream_job
unknown
```

这是溯源标签，不是 MemoryType ontology，也不是仅凭标签就获得信任的授权机制。插件能确认的注入部分保留原始 refs、capsule id 和 hash；不能确认的内容标为 unknown，不假装它是用户亲口陈述。

### 防止记忆回声

```text
Memory A → 注入 Web → 被记录为 user message → 再 Dream
```

最后一步不能把注入部分当作新的独立证据，更不能因其被复述多次而提升“置信度”。外部模型复述 A 也不构成新的观察。

## 6.4 内容存储策略

| 内容 | 默认建议 |
|---|---|
| 用户消息、助手回复 | inline，保留角色和 origin |
| 工具名、参数 | 经过脱敏的结构化内容 |
| 小工具结果 | inline |
| 大结果 | 有上限的 preview + blob，或明确 truncated/omitted |
| 文件访问 | path/size/hash（能获取时）；可选快照 |
| 文件修改 | 实际可得 diff/commit/ref；不是声称一定捕获 diff |
| 引用 | URL/title/可得摘录，与具体回复绑定 |
| 执行事件 | lifecycle/关联/结果状态 |
| 中断回复 | 保存 partial 记录，后续追加完成修订 |
| 隐藏推理 | 不获取、不作为能力前提 |
| API keys/秘密文件 | 写盘前过滤，native/raw/blob 同样过滤 |

仅有 file path/hash 的记录并不能恢复当时文件内容；`sources` 必须报告 `content_not_retained`。

## 6.5 Capture 能力声明

每个 adapter 输出覆盖报告：消息、工具、文件、引用、分支分别是 supported / partial / unsupported。

官方 Codex hooks 当前明确区分本地工具与 hosted WebSearch 的覆盖，并提醒 transcript 并非稳定 hook 接口；这说明不能把“装了 hooks”写成“全量捕获保证”。[R07]

---

# 7. Memory：通用内容 + 可核验元数据

## 7.1 最小记录

```markdown
---
id: mem_demo_01
revision: 1
status: active
scope: project:recallcard
observed_at: 2026-10-06T10:00:00Z
recorded_at: 2026-10-06T10:20:00Z
valid_from: null
valid_to: null
time_note: 该决定在本次对话中明确提出；不推定更早日期。
authority: user
protected: true
sources:
  - event: evt_demo_01
    part: 0
supersedes: []
labels: [architecture]
entities: [recallcard]
---

该项目采用 Rust 主程序与 Python worker。
```

`authority` 表示修改权限来源，不直接等于内容为真。`protected` 是程序执行的编辑保护，而不是对模型说一句“不要改”。手工创建/纠正可以通过 CLI 写一条有来源的用户更改 Event。

`confidence` 非必需；如果模型输出，保留为 `model_score` 或诊断字段，不能当作经校准概率，更不能因为 `0.97` 自动覆盖用户约束。

## 7.2 时间语义

- `observed_at`：有证据表明此陈述/观察出现的时间。
- `recorded_at`：本系统写入 Memory 的时间。
- `valid_from/to`：证据支持的现实有效区间；允许未知。
- `time_note`：保留“九月底”“之前”“计划”等原始精度，避免编造某一天。

`valid_to: null` 只表示没有记录终止边界，**不证明今天仍成立**。

“计划七月旅游”不能因为到了八月就自动变成“七月已经旅游”。现实事件是否发生需要新的证据。

Graphiti/Zep 对记录时间与事实有效时间的区分值得借鉴；RecallCard 不需要因此引入图数据库。[R10]

## 7.3 更新、替代、冲突

比较之前先检查 entity/subject 和 scope。用户笔记本用 Arch、服务器用 Debian，可以同时成立。

新事实明确替代旧事实时保留旧记录与 supersedes；如无法确定旧状态何时结束，不能强行制造精确 valid_to。

`status` 可保留少量状态：active / tentative / superseded / retracted。未决冲突记录为诊断/待审信息，不能靠向量相似度决定哪个为真。

## 7.4 单个记录不一定是单句话

“Atomic”表示可独立追溯、修改与检索的记录单位，不要求把每个复合决定拆成十条碎片。

一次重要讨论的结论和理由可以是一条 `labels: [session-summary]` 的 Memory。它不应替代原文，也不需要引入 canonical Episode。

---

# 8. 召回：四个工具，非四道关卡

## 8.1 入口必须同时考虑三类问题

| 问题 | 首选信息 | 不应强制做的事 |
|---|---|---|
| “我的电脑适合跑这个吗？” | Bootstrap / computing View | 每次全历史语义搜索 |
| “刚才在另一个客户端说到哪？” | 显式交接来源 + 最近 Events | 等待 Dream |
| “之前为什么选 Rust？” | Memory + 原始证据/相邻上下文 | 只给孤立事实 |

Bootstrap 应包含少量通用信息和有意义的目录标签，而不是只有 ID，也不是用户全部人生。LLM 按需调用；这与 Anthropic 说明的 upfront context + just-in-time retrieval 接近，但不保证所有模型同样可靠。[R01]

## 8.2 `bootstrap`

- 小型、受授权约束的 profile/index/access rules。
- 返回 `bootstrap_version`；稳定内容不放当前时刻、随机 request id。
- 动态 coverage、当前 session、交接来源等放单独尾部。
- 稳定访问规则与个人参考数据分开；记忆不是系统指令。
- 已知目标机器/项目等是任务专属信息时，不强行塞入通用 profile。

**首次加载由宿主/adapter 确保**。仅声明一个 bootstrap tool，模型未必会主动调用。

## 8.3 `search`

默认搜索当前授权范围内的 Memory 和可检索 Event 文本；View 可作为额外索引文档。

```json
{
  "query": "刚才关于编程语言的决定和理由",
  "target": "all",
  "session_ref": null,
  "as_of": null,
  "limit": 5,
  "detail": "context",
  "budget_tokens": 1500
}
```

`target`: all / memories / events。`detail`: brief / context。

`context` 可在预算内带少量完整短记忆、原句和相邻上下文；**不是让模型为每个 50 字结果再调用一次 read**。

示例返回：

```json
{
  "results": [
    {
      "ref": "memory:mem_demo_01@1",
      "text": "该项目采用 Rust 主程序与 Python worker。",
      "scope": "project:recallcard",
      "evidence_refs": ["event:evt_demo_01"],
      "state": "active",
      "time_note": "本次对话明确提出"
    }
  ],
  "coverage": {
    "event_search": "available",
    "semantic_search": "unavailable",
    "undreamed_events_included": true
  },
  "truncated": false,
  "next_cursor": null
}
```

没有命中 ≠ 从未发生；权限不足、未同步、未捕获、索引滞后必须区分。

## 8.4 `read`

```json
{
  "refs": ["view:computing", "event:evt_demo_01"],
  "budget_tokens": 1800
}
```

支持多个 refs、可选范围/分页；总输出预算由服务器执行。Ref 是不透明标识，不能变成任意本地路径读取接口。

## 8.5 `sources`

返回证据原文范围、关联事件、引用或文件版本位置。深层 blob 缺失要明确说明。

不要求每次回答都追到原文，但涉及有争议决定、确切原话、时间边界时应下钻。

## 8.6 授权、排序与预算

1. Server 根据客户端绑定的 scope/授权范围筛选；模型参数只能缩小范围，不能扩大权限。
2. 明确的 ID、session、实体、时间查询先结构化过滤。
3. 文本检索与兼容向量检索生成候选，去除重复和无关分支。
4. 在预算内返回足够作答的证据与来源；必要时提供下一页。

中文检索必须验证分词/Unicode 行为；不能把适合英文的默认 tokenizer 未测试地当作中文 BM25。IP、文件路径、型号等保留精确匹配路径。

Anthropic 的工具设计指南也强调结果应有语义价值、可受 token 预算约束。我们的四工具接口应该优化“材料能否支持下一步”，而不是只返回机器 ID；这不要求增加工具数量。[R18]

## 8.7 Web 的特殊成本是“人点 Send 的次数”

- Agent 能自动完成 search/read；Web 每轮往往需要用户发送工具结果。
- 一次 Context 请求允许批量 read 或有限的 search+证据展开。
- 不把每一层目录都变成一次人工点击。
- 不为减少点击无限扩展材料；仍受总预算和授权限制。

优先指标是：用户为一个正常问题额外点击几次、返回材料能否直接支持回答，而不是工具调用次数越多越“Agentic”。

---

# 9. Bootstrap 生命周期与跨端交接

## 9.1 稳定快照

对一个会话固定：

```text
bootstrap_version + authorized_scope + rendered_content_hash
```

Dream 更新全局 Memory 时，不改写已经发送的旧消息，也不不断替换会话开头。必要的新事实作为后续 Context update 追加。

新会话使用最新快照；明确纠正/权限撤销优先于缓存稳定，不得继续返回已知失效内容。

## 9.2 Agent 的加载

通过经过验证的启动/恢复/压缩相关 hook 或宿主配置，插入短指引与已授权 Bootstrap。

Claude Code 文档体现了“入口记忆有限、详情按需读取”的路径；Codex/Claude Code hooks 提供的注入位置各不相同，必须逐版本测试。特别不要假设名叫 PostCompact 的回调就能注入下一轮上下文。[R02][R06][R07]

MCP 负责提供能力；是否把工具、资源、启动内容交给模型属于宿主。不要依赖 MCP server 在背后监视所有模型消息。[R11]

## 9.3 Web 的加载

进入新会话时准备可见 Bootstrap，不覆盖用户已有草稿、不自动发送；该会话确认发送一次后不重复追加相同块。

扩展不知道厂商是否做过内部压缩时，不能假装能检测。提供“重新附上访问说明”操作；协议失效时明确回退到手动复制，不无限自动重试。

## 9.4 “继续另一端刚才的任务”

提供显式 continuation 来源选择，例如“带上这次对话继续”。这不是前置 Topic Router。

交接 Capsule 包含：

- 来源 session/branch/ref；
- 当前用户目标与最近关键消息的有界投影；
- 可继续 search/read 的入口；
- captured-through / 尚未 Dream 的提示。

它是派生传输包，不是新 canonical Memory 类型。默认不把“全局最近一个窗口”当成用户现在要继续的任务，避免多项目串线。

## 9.5 与客户端自带记忆共存

RecallCard 标记自己的来源与版本，不声称取代或已关闭宿主记忆。

内容冲突时应展示源证据和时间；不要用高优先级 Prompt 宣称自己的记忆永远正确。Codex 官方文档也区分本地 Codex memories 与 ChatGPT 的记忆存储。[R03]

---

# 10. Cache-aware Context Compiler

## 10.1 三种机制不要混淆

| 机制 | 存的是什么 | 是否影响未来“记住用户” |
|---|---|---|
| 本地派生结果缓存 | 构建输出、embedding 向量、投影结果 | 只是减少重复计算 |
| Provider prompt/KV cache | 相同前缀的模型计算状态 | 不是长期 Memory，也不是缓存最终答案 |
| 应用 Memory | 用户历史的可追溯解释 | 是本项目核心 |

v1 不做语义相似问题的整段答案缓存，避免陈旧答案和权限泄露。

## 10.2 Prefix cache 的真实边界

供应商缓存针对实际渲染的相同 token 前缀，不是句子语义相似。工具定义、隐式模板和模型设置可能影响可复用前缀；缓存边界、最低长度、保留期、计费随供应商和模型变化。[R04][R05]

因此以下不是可靠优化：

```text
把 job_id 放在最前面，然后说后面 10k 文本都一样
每次 Dream 重新措辞 Profile，但语义没变
每次调整 tools 排序，再期望长历史仍完全命中
给不同供应商发相同 Markdown，期待共享 KV cache
```

模型前缀缓存并不减少上下文窗口中占用的 token；不能用缓存命中为塞入无关材料辩护。

## 10.3 三类调用的控制权

| 调用 | RecallCard 能控制什么 | 不能承诺什么 |
|---|---|---|
| 自己发起 Dream API | 请求内容、序列、受支持缓存字段、usage 记录 | 供应商一定命中、一直不淘汰 |
| 外部 Codex/Claude Code | 自己的注入块、工具 schema 和返回内容稳定 | 接管宿主隐藏模板、缓存路由与计费 |
| 消费者 Web Chat | 可见文本与人工交接开销 | 可设置 KV TTL、跨端共享缓存、直接降低订阅账单 |

即使 API key 使用中转/兼容接口，也必须由实际 Provider 返回的能力与 usage 确认缓存；“OpenAI-compatible”不等于缓存字段完整兼容。

## 10.4 稳定前缀与动态后缀

下面是逻辑层顺序；实际 role 和 tool 格式交给 Provider Adapter，不能为追求顺序违反其消息协议。

```text
P0：固定协议、少量使用示例、固定工具 schema
P1：该客户端已授权的稳定 Bootstrap 快照
    [供应商支持且满足条件时，在合适稳定边界建立 cache]
P2：当前会话已有消息/观察，尽量 append-only
P3：本轮 Context 更新、检索证据、用户任务、动态运行元数据
```

静态指引可以放宿主授权的 instruction 区；Profile/检索文本仍是 reference data，不因位于前缀就变成高优先级指令。

### 不进入 P0/P1 的内容

- 当前精确时间、每次变化的 now；
- 随机 nonce、request id、Dream job id；
- 每次 refresh 的 Git HEAD、抓取计数、工具耗时；
- 每次重新措辞的摘要；
- 按当前分数重新排序的整个记忆清单。

确实需要这些信息时，放动态后缀；不是删除必要信息。

## 10.5 稳定的“缓存边界”同样重要

相同前缀只是必要条件之一。在需要显式写入缓存的 API 中，如果只在包含动态 suffix 的末尾设断点，之前那个稳定公共前缀不一定曾被写成可命中的条目。

Provider Adapter 必须知道实际支持的是 automatic、explicit breakpoint，还是不可观察/不支持。不能只实现一个“有共同前缀”的算法就宣布优化完成。[R05]

## 10.6 会话快照与变化发布

- 同一 bootstrap 输入、模板版本、授权范围必须产生同字节输出。
- 相同前缀不含每次构建时间；版本由实际内容决定。
- 会话开始固定快照；新事实追加 delta。
- 工具名称、顺序、schema 稳定；只有四个工具，不引入复杂动态 tool search。
- 不为了缓存保留错误事实；权限变化立即应用，必要时新建会话/重新提供纠正。
- 宿主 compaction 后按其生命周期重新注入；不为缓存阻止有益压缩。

Manus 的公开工程经验同样强调稳定前缀、追加式上下文和稳定工具定义；RecallCard 借鉴这个约束，不需要复制其完整运行时。[R08]

## 10.7 Dream API 的缓存布局

```text
相同 extraction/consolidation 指引
相同输出 schema 与少量固定示例
真正复用时才附相同、授权且冻结的旧记忆子集
[稳定缓存边界]
不同 source chunk + job metadata + 待判断候选
```

不能为了命中缓存给每一小批输入塞全库旧 Memory；额外 token、注意力和写缓存费用可能得不偿失。

手动一周一次 Dream，不能假设上周的 Provider cache 仍然存在。可靠的第一省钱手段是只处理增量、避免失败重跑与重复向量化，而不是依赖无限 TTL。

Aider 将 system prompt、只读文件、repo map 等不同内容组织到缓存中，并提供 map 刷新策略。对本项目的启发是控制派生目录的刷新，而不是每轮改写；不照搬其保温 ping，以免为很少复用的内容额外花钱。[R17]

## 10.8 Provider cache 能力模型

这是内部能力信息，不是统一向 API 发送的 JSON：

```text
cache_control: automatic | explicit | unavailable | unknown
min_prefix_tokens: 已知值或 unknown
retention_options: 已知列表或 unknown
usage_fields: 可读取的 read/write/input/output 指标
pricing_snapshot: model + region + date + unit rates
```

不硬编码一套对所有模型通用的缓存字段。模型/API 升级后重新验证，未知时正常运行但不宣称有缓存收益。

---

# 11. 成本：先避免重算，再优化前缀

## 11.1 正确估价范围

Dream 成本至少包含：

```text
投影后的输入
+ 检索旧记忆后 consolidation 的输入
+ 输出（包括提供方计费的 reasoning token）
+ retry / schema repair
+ embedding / reranker
+ cache 写入/命中计费
```

Raw Events 的 token 总数不能直接当成实际 billed input；经过多阶段操作后，既可能减少，也可能因重复读取增加。

旧版固定模型的月费示例不再作为默认承诺。配置中只保留可更新 price snapshot；使用时按实际模型、区域、长度档、Batch 资格核对官方价格。

## 11.2 缓存的示意算例，不是实测账单

假设某服务按普通输入单价计：5 分钟 cache 写入 1.25 倍，读取 0.1 倍。Anthropic 当前价目对部分模型采用这组倍率，但并非所有模型相同。[R12]

设 10 次请求，每次稳定前缀 2,000 token，变量部分 500 token；全部满足缓存长度、边界和存活条件：

```text
无缓存输入成本当量：10 × (2000 + 500) = 25000
有缓存输入成本当量：2000 × 1.25 + 9 × 2000 × 0.1 + 10 × 500 = 9300
理想输入费用减少：(25000 - 9300) / 25000 = 62.8%
```

这个百分比不含输出、工具、缓存 miss、重试和其他收费，更不是 RecallCard 真实节省比例。低频不复用时，写缓存甚至可能不划算。

## 11.3 优先级

1. 新 source revisions 才进入 Dream。
2. 不重复处理召回注入块、无意义日志、已完成 job。
3. 未变的 Memory 内容不重复 embedding。
4. 相同输入的确定性 Views 不重复调 LLM。
5. 真实重复的 Prompt 前缀才走供应商缓存。

## 11.4 最小用量记录

本机 JSONL/SQLite 诊断记录即可，无需另建观测平台：

```text
job_id / attempt_id
provider / model / region
input_tokens / output_tokens
cache_read_tokens / cache_write_tokens（不可得则 null）
latency_ms / retries
estimated_cost + pricing_date
bootstrap_hash / prompt_template_hash
```

不要为诊断默认再次保存完整敏感 Prompt。没有供应商命中指标时只能说“前缀稳定”，不能说“缓存命中”。

---

# 12. Dream：有界任务、证据约束、可安全重试

## 12.1 五步保持不变

```text
Project → Extract → Retrieve/Consolidate → Validate/Resolve → Publish
```

Dream 不是主 Agent 的实时执行循环；手动 Web、API、本地模型输出同一种受验证的结果。

Codex 官方本地 memories 说明、Letta 的后台整理设计、LangGraph 的后台记忆处理都提供了“持久知识与正在运行的会话分开处理”的参考；它们不是 RecallCard 完整实现的依赖。[R03][R09][R13]

## 12.2 Projection 不能伪装成理解一切的 deterministic 算法

规则能稳定选择用户消息、完整回复、失败状态、显式批准、短 diff/结果；但“真正重要的决定在哪里”不总能由日志格式判断。

v1 采用保守有界投影：优先完整用户输入和结论，保留失败/纠正与对应 source ids；大结果抽取明确摘要或 preview，并允许下一阶段按 ref 补读。

不要只保留成功的最终答案；失败尝试可能正是以后避免重犯的证据。

## 12.3 Dream Job 草案

```json
{
  "schema": "recallcard.dream-job/1",
  "job_id": "dream_demo_01",
  "operation": "extract",
  "prompt_version": "extract-v1",
  "projection_version": "projection-v1",
  "source_refs": [
    {"ref": "event:evt_demo_01", "content_hash": "demo-source-hash"}
  ],
  "memory_read_set": [
    {"ref": "memory:mem_existing@3", "content_hash": "demo-memory-hash"}
  ],
  "allowed_scope": "project:recallcard",
  "input_hash": "demo-job-hash",
  "output_schema": "recallcard.dream-result/1"
}
```

Hash 根据 source revisions、选定旧记忆、projection、prompt/schema 与必要模型配置生成。job id 仅在动态任务段，不塞进共享前缀。

## 12.4 Result 只是提议

```json
{
  "schema": "recallcard.dream-result/1",
  "job_id": "dream_demo_01",
  "input_hash": "demo-job-hash",
  "proposals": [
    {
      "operation": "add",
      "content": "该项目采用 Rust 主程序与 Python worker。",
      "source_refs": ["event:evt_demo_01"],
      "scope": "project:recallcard",
      "valid_from": null,
      "valid_to": null,
      "time_note": "来源没有给出独立于陈述时间的生效日期。",
      "target_ref": null,
      "expected_revision": null
    }
  ]
}
```

其他 operation 可包括 supersede / noop / conflict；是否保留 merge 为独立操作由实现用例决定，不要求每个后端都做复杂合并。

## 12.5 导入必须检查

- job 存在，input_hash 一致；
- 引用的来源在 job 授权/显式补读的集合中；
- 来源真实存在，不能接受编造 event id；
- 修改目标仍为期望 revision；过期结果进入重新整合，而非盲写；
- 有效区间格式/范围、scope、保护权限合法；
- 数量、字节、路径和输出长度限额；
- 相同 job/result 已成功应用则返回 already_applied。

**确定性校验器能检查结构和权限，不能证明自然语言结论一定为真。** 语义冲突和高影响修改仍须人工判断或明确保留不确定性。

## 12.6 保守的默认治理建议

- 有明确用户原话且不修改保护记录的低风险新增，可按用户配置自动接受。
- 助手提议/推断先 tentative，不能直接成为“用户已决定”。
- 用户保护内容的修改要求确认。
- 不因重复复述、embedding 相似度或模型自评分升高就判定更可信。
- 从外部网页总结出的指令不会获得系统指令权限。

这些是本次复审建议，详细开关不是历史上已逐项确认的产品决定。

## 12.7 暂存、提交与收据

```text
prepare job
  → executor result
  → validate against current read-set
  → stage memory changes + receipt
  → 单写者锁/事务日志下提交
  → publish new memory generation
  → update text/views
  → optional embedding
```

成功 receipt 保留 job hash、source coverage、目标记录版本与结果摘要；不放入可随意删除的 `.index`。

多文件原子性不能靠“git commit”本身保证。使用本机事务日志/暂存目录与恢复逻辑；崩溃后完整提交或回滚，不能一半改了 Memory、一半仍显示未处理。

向量服务失败不回滚已经正确提交的 Memory；标记 semantic index pending，保留文本检索。再次运行只补向量，不重新付费 Dream。

## 12.8 Manual Web Dream

- 输入分为可独立处理的有界 job；不假设每家 Web 能接收几十万 token。
- 用户可预览/删除敏感 source，再生成新 input hash。
- 由人发送；结果导入先校验、展示 diff，不执行结果中的命令。
- 手动聊天中的自由回复不能被误当作完整 DreamResult。
- Dream 本身产生的聊天若被 capture，标为 dream_job/result，防止再次当原创用户经历学习。
- Web 输出不完整：不推进该 job 的成功 watermark。

## 12.9 可以复用函数，不必引入完整框架

之前把 LangMem 描述成几乎必须绑定 LangGraph，不够准确。其官方概念文档区分：无存储副作用的 core memory API，以及依赖 LangGraph Store 的集成层；前者能用于其他存储。[R19]

因此 Python worker 可以先参考或试用类似 `create_memory_manager` 的候选生成方式，但输出只能进入我们的 Job/Result 校验、权限和文件事务流程。不能把某个库生成的 update 操作直接写进 Vault。

Mem0 的 extraction/consolidation 同样是参考，不是必须复制完整源码或 API。具体复用还是自行实现由最小闭环结果决定，不因架构独立就拒绝成熟函数。

LangMem 的 delayed processing 文档还指出，等待一段对话安静后再处理，可以减少重复工作和处理中途缺失上下文的问题。首版手动 Dream 已符合该方向；将来自动模式只需 debounce，不需要常驻多 Agent 编排。[R20]

---

# 13. Adapter、IPC 与客户端边界

## 13.1 Capture plane ≠ Context plane

MCP server 通常只看到对自己发起的调用，不能因此获取所有宿主对话或其他工具输出。完整输入仍靠 hooks、日志/导出适配器与用户授权。[R11]

每个 Agent adapter 至少测试：首次启动、恢复会话、压缩后、工具失败、被中断、重复导入。

Web adapter 至少测试：新对话、已有草稿、消息编辑、regenerate、tab 切换、流式中断、页面结构失效。

## 13.2 通信形态不改变

```text
Agent → MCP stdio bridge → local core IPC
Web → extension service worker → native host → local core IPC
Python worker → supervised stdio process
```

Native Messaging 用浏览器允许的本机桥；具体 host 注册路径与安装步骤按 OS/browser 实现。消息大小有限，大文本/附件必须分块或按 ref 读取，不能把整个 Vault 一次返回。[R14]

默认无需 HTTP Server。未来官方 Web 接入需要远程 MCP 时，可以独立增加有认证的 HTTP 适配，不为 v1 提前部署公网服务。

## 13.3 Web 并非一概没有官方工具接口

将 v0.1 “Web Chat 没有标准 MCP”改为：**不同 Web 产品与账号能力不同，应优先评估当前官方集成；不可用时才使用人操作的文本桥**。

不要把需要公网 MCP 的官方能力，与“扩展访问本机 private IPC”混为一谈；是否支持取决于具体接入方式。

## 13.4 文本 action 协议

```json
{
  "protocol": "recallcard.action/1",
  "request_id": "r_demo_01",
  "nonce": "session-correlation-token",
  "action": "read",
  "arguments": {
    "refs": ["view:computing", "memory:mem_demo_01@1"],
    "budget_tokens": 1800
  }
}
```

实际以 `recallcard-action` fenced block 输出。只处理当前会话的完整、符合 schema 的块；禁止 eval、任意命令和任意本地路径。

nonce 是防误触与请求关联，**不是防 Prompt Injection 的安全认证**；模型和网页可见的值无法证明动作具有用户授权。范围、速率、读写权限必须由 native host/core 执行。

## 13.5 发送边界

检索结果放进可见、可撤销的 composer draft；保留用户原有草稿。每个 request id 的结果至多准备一次，不重复注入。

human send 之后才记录“已交付”；仅填入草稿不意味着模型已读到。

自动读取/保存网页内容是否允许仍要看平台条款，人点 Send 并不自动使抓取合规。官方导出、手动复制等降级路径必须保留。[R15]

---

# 14. 索引与 Embedding

## 14.1 最小可用顺序

1. Events/Memory 的精确词与文本检索；
2. 时间、session、scope 过滤；
3. 只为合适 Memory/选定文档做向量索引；
4. 必要时增加融合和重排。

一条 20MB build log 没有必要整体向量化。旧记录不值得入 Memory，仍可通过原文文本搜索找到。

## 14.2 向量空间身份

索引必须记录：

```text
provider / model / 可获得的 model revision
dimensions
query/document task instruction
pooling / normalization（可控时）
text preprocessing version
```

两个模型输出同为 1024 维，不等于可放在同一检索空间比较。切模型、维度或关键 instruction 后建立新 generation；完成前旧索引继续使用原 query encoder，或退回文本检索。

## 14.3 增量 Embedding

按 embedding input hash + embedding-space signature 复用结果。改了显示标题但未改向量输入时无需重算；改变有效检索文本时必须重算。

query embedding 可按精确查询内容和同一 signature 小规模缓存，但不可绕过当前授权/时间过滤缓存最终答案。

## 14.4 重建不是零成本魔法

删除 `.index` 后：

- 解析、结构化过滤、文本索引、确定性 Views 可以离线重建；
- 向量索引如果没有保留可复用 embedding，就需要模型或 API；
- 原模型不可用时必须选择新模型并重新编码，而不是编造一致性。

因此“索引可丢弃”意味着数据不会丢，不意味着原始语义检索性能/费用/字节结果必定不变。

---

# 15. Git 同步、冲突与删除

## 15.1 建议流程

1. 短暂冻结本机 writer，封闭当前事件段。
2. 校验并将本机已完成事务提交，确保工作区状态可整合。
3. fetch 远端；使用项目明确选择的 merge/rebase 策略整合。
4. 文本冲突时停止，保留两边，不自动 force push。
5. 校验结构、引用和语义一致性；同一事实出现矛盾时待审，不以 Git merge 成功代表内容正确。
6. 发布新的 Vault generation，更新文本索引与 Views。
7. 正常 push；远端已变化则重试 fetch/整合，不丢弃他端更新。

原 v0.1 的“先 pull --rebase，再提交本地变化”顺序已撤回。

## 15.2 并发策略

- 本机一个核心 writer 管理事务；MCP/native/CLI 都走该入口。
- Events 使用设备独立、封闭后不可变的段。
- 首版建议指定一台主要 Dream 发布设备，其他设备先 capture/sync；这是简化政策，不是假装 Git 提供分布式互斥锁。
- 允许离线双端编辑时，必须接受同步后的冲突审查；不要承诺自动强一致。

## 15.3 忘记与抑制

删除 Memory 文件不够：原始 Events 仍可能被下一次 Dream 重新提炼。

因此保存 suppression/撤回规则，作用于 read/search/bootstrap/dream，并提供用户显式恢复流程。这是控制元数据，不是新 Memory 类别。

Git 保留旧版本，删除当前文件不等于擦除所有历史、远端 clone 或已经发给模型的文本。需要彻底移除时必须明确操作范围；不能宣传“一键遗忘”超出实际能力。

---

# 16. 安全与权限：统一不代表全部广播

## 16.1 至少分开四项开关

```text
允许捕获本会话
允许它参与 Dream
允许本客户端读取哪些 scope
允许哪些内容发送给云 embedding/LLM
```

默认个人全局范围也必须有明确授权，不应自动把学校项目、私人聊天、第三方机密全部混发。

## 16.2 权限不可只写在 Prompt 里

模型输入中的 labels、nonce、自然语言声明都不授予额外权限。native host 根据实际扩展/客户端绑定、用户设置和允许操作执行检查。

未知 action/ref、越界请求、目录穿越、符号链接逃逸、超大 payload 必须由程序拒绝。

## 16.3 总结不改变信任来源

网页恶意内容被 Dream 总结后仍然是来自网页的内容。authority、source origin、root evidence 不应在 consolidation 中丢失。

将规则指令和参考资料分离；不要把“从用户文档提取的指令”默认写成 system/developer 指令。

## 16.4 数据最少化

- 凭据不进入 Vault、Git、native raw 或调试日志。
- 敏感 source 可以只允许本地文本索引，不参与云 Dream/Embedding。
- 全部输出必须受预算、范围与脱敏限制。
- 导出到 Web Dream 前有可读预览；跨服务发送个人上下文是用户授权行为。

---

# 17. 实现结构与开发计划

## 17.1 不照搬大型 Harness

首版建议一个 Rust package/少量 crate，内部模块清晰即可：

```text
recallcard/
├── src/
│   ├── model/
│   ├── vault/
│   ├── capture/
│   ├── context/
│   ├── search/
│   ├── dream/
│   ├── transports/
│   ├── sync/
│   └── main.rs
├── extension/
├── python/recallcard_worker/
├── integrations/
├── schemas/
├── fixtures/
└── docs/
```

仍然支持 `daemon / mcp / native-host / dream / search / read / sources / rebuild / sync / doctor` 等子命令，但不要求一开始全部完善。

Rust 控制写入、权限、数据模型和协议；Python 按需提供 embedding/实验能力。没有 Python 或 API 时，Capture、文本搜索、Bootstrap、手动 Dream 导出仍可用。

## 17.2 先验证一个真实纵切

### Milestone A：不用 Dream 也能跨端继续

- 一个 Web 来源（手动导出/复制适配也可以作为第一步）。
- 一个本地 Agent。
- 去重、Event 文件、文本检索、四工具最小实现。
- Bootstrap 自动或显式一次性加载。
- 读到刚才另一端的决定及原话，不依赖 embedding。

这是最早产品验收，不要先做九个 crate、五个 Web adapter 再验证用途。

### Milestone B：Dream 与受保护 Memory

- Manual job export/import。
- 来源校验、幂等 receipt、冲突和 protected 记录。
- Memory Markdown、确定性 Views。
- 原始搜索与 Memory 搜索共同可用。

### Milestone C：Web 交互闭环

- Native Messaging、composer 注入、来源标记。
- request/result 去重，不自动发送，不覆盖草稿。
- 搜索给足上下文；必要时一次批量 read。
- 页面变化时安全停止并回退。

### Milestone D：云 Embedding 与 API Dream 优化

- embedding-space signature 与增量更新。
- 一个 Provider Adapter；缓存边界和 usage 可观测。
- 稳定前缀测试、失败重试、费用上限。

### Milestone E：同步与平台硬化

- Git 封段、提交、整合、冲突恢复。
- 三平台 IPC/安装/路径测试；至少早期 CI 检查可编译性。
- 再增加第二个 Web 和第二个 Agent adapter。

---

# 18. 最小验收：不是大型 benchmark，但不能不验证

下列可以用少量脱敏 fixture 与实际选定的一个模型人工检查，不需要多模型排行榜。

| 编号 | 场景 | 必须满足 |
|---|---|---|
| T01 | Web 刚作决定，未 Dream，Agent 立即查询 | 返回已捕获 Event 与真实来源 |
| T02 | 没预算做 Dream，积累一周历史 | 原文搜索仍可用，不假装不存在 |
| T03 | 助手提议 C，用户没回答 | 不存成用户已批准 C |
| T04 | 已有 Memory 注入两家 Web 又被捕获 | 不生成两份独立新证据、不提高信任 |
| T05 | 笔记本 Arch，服务器 Debian | 不视为互相替代 |
| T06 | “上个月换了系统”，无具体日 | 不造具体 valid_from 日期 |
| T07 | 同一 source/result/job 重复导入 | 幂等；Memory 与费用不重复增长 |
| T08 | DreamResult 导回前目标记录被修改 | 拒绝过期覆盖，进入重新整合 |
| T09 | 多轮上下文或宿主 compact 后 | 已验证的加载钩子或显式恢复可继续使用工具 |
| T10 | 同快照构建两次 Bootstrap | 输出同字节；动态 now/nonce 不进入稳定前缀 |
| T11 | 记录权限撤销/遗忘后再次 Dream | 不召回、不重新提炼被抑制内容 |
| T12 | Git 冲突/崩溃/Embedding 故障 | 正本一致、错误可见、可退回文本检索 |

另外记录一项 UX 观察：正常个人上下文问题，需要多少次额外手动 Send 才获得足够材料。没有实测前不写“和 ChatGPT 一样自然”。

---

# 19. 决策日志与明确暂缓

## 19.1 已保留

- Event + Memory；非强制 Episode/ontology。
- Rust + Python、本机单体、文件/Git、四只读工具。
- Dream 可手工 Web、Embedding 云可选。
- Bootstrap + 模型主动检索，不恢复前置 Topic Router。
- Web human Send。

## 19.2 本轮建议

- Events 即时可搜索；显式交接而非猜全局最近任务。
- Bootstrap 快照与增量发布、稳定缓存边界。
- search 返回足量证据、read 批量；不是层层强制点击。
- 模型建议与用户事实分离、回声去重。
- Dream 任务幂等/基线校验/事务收据。
- 明确 scope/权限、覆盖缺口、原文未保留状态。
- 确定性 Views 和向量重建的实际条件。
- Git 同步顺序修正及首版并发简化。

## 19.3 不增加

- 复杂知识图数据库。
- 自建缓存服务或自己管理供应商 KV。
- 多 Agent Dream committee。
- 全平台首发。
- 大规模多模型 benchmark。
- 自动 Web Send 或绕过反自动化。
- 用动态 routing 猜每一个用户输入主题。

---

# 20. 调研依据与证据边界

以下页面在 2026-10-06 对照。它们支持相应设计原则，不证明 RecallCard 已实测成功；条款、功能、字段与价格发布前需复核。

| 编号 | 官方/项目来源 | 本轮实际借鉴的边界 |
|---|---|---|
| R01 | Anthropic：Effective context engineering | 小入口 + 按需读取；少量清楚工具；探索有延迟和漏检风险 |
| R02 | Claude Code：Memory | 入口文件与按需详情；持久指令不等于安全强制 |
| R03 | OpenAI：Codex Memories | 后台整理、本地状态；ChatGPT 与 Codex 的存储区别；读取/贡献分开 |
| R04 | OpenAI：Prompt caching | 实际前缀与模型/请求条件；按 Provider 适配 |
| R05 | Anthropic：Prompt caching | 稳定段、缓存断点；存在公共前缀不代表一定已写入可命中边界 |
| R06 | Claude Code：Hooks | 加载/恢复/工具事件需要宿主适配；回调语义不可凭名称猜 |
| R07 | OpenAI：Hooks | 事件覆盖差异、transcript 稳定性边界 |
| R08 | Manus：Context Engineering lessons | 前缀稳定、追加式上下文、工具定义稳定；不搬完整运行时 |
| R09 | Letta：Context Repositories | 文件/Git 上下文、后台整理隔离；借鉴而非必需依赖 |
| R10 | Zep：Searching the Graph | 记录/有效时间与过滤；不把相似度等同当前真实状态 |
| R11 | MCP Architecture | Host 管上下文与编排；Server 提供能力，不是全量会话 recorder |
| R12 | Anthropic Pricing | 缓存写/读倍率的示意计算来源，不硬编码成通用价目 |
| R13 | LangGraph Memory overview | 当前线程状态与跨线程长期记忆区分；后台写入有延迟 |
| R14 | Chrome Native Messaging | 本机桥、允许来源、消息限额与安装约束 |
| R15 | OpenAI Terms of Use | 人工发送不自动消除程序化提取条款风险 |
| R16 | Mem0：Add memories | 消息到提炼记忆的接口参考；不兼任全量跨客户端 recorder |
| R17 | Aider：Prompt caching / Options | 稳定内容分组与 repo map 刷新，避免频繁重写入口 |
| R18 | Anthropic：Writing tools for agents | 结果语义、预算与工具调用效率 |
| R19 | LangMem：Core concepts | 核心函数可独立存储使用，不必接管主 Runtime |
| R20 | LangMem：Delayed processing | 对话闲置后整理，减少重复/过早提炼 |

- R01: https://www.anthropic.com/engineering/effective-context-engineering-for-ai-agents
- R02: https://code.claude.com/docs/en/memory
- R03: https://learn.chatgpt.com/docs/customization/memories
- R04: https://developers.openai.com/api/docs/guides/prompt-caching
- R05: https://platform.claude.com/docs/en/build-with-claude/prompt-caching
- R06: https://code.claude.com/docs/en/hooks
- R07: https://learn.chatgpt.com/docs/hooks
- R08: https://manus.im/blog/Context-Engineering-for-AI-Agents-Lessons-from-Building-Manus
- R09: https://www.letta.com/blog/context-repositories/
- R10: https://help.getzep.com/searching-the-graph
- R11: https://modelcontextprotocol.io/docs/2026-07-28/learn/architecture
- R12: https://platform.claude.com/docs/en/about-claude/pricing
- R13: https://docs.langchain.com/oss/python/concepts/memory
- R14: https://developer.chrome.com/docs/extensions/develop/concepts/native-messaging
- R15: https://openai.com/policies/terms-of-use/
- R16: https://docs.mem0.ai/core-concepts/memory-operations/add
- R17: https://aider.chat/docs/usage/caching.html ; https://aider.chat/docs/config/options.html
- R18: https://www.anthropic.com/engineering/writing-tools-for-agents
- R19: https://langchain-ai.github.io/langmem/concepts/conceptual_guide/
- R20: https://langchain-ai.github.io/langmem/guides/delayed_processing/

### 没有据此作出的断言

- 没有把任何框架营销 benchmark 当成本项目准确率。
- 没有声称已经审阅或测试了无法获取的 Codex 内部 memory 源文件。
- 没有从产品公开体验反推 ChatGPT 必定使用我们的内部数据结构。
- 没有因为两个项目都采用文件，就断言 Git 已经解决语义冲突。
- 没有因为用户允许 cloud embedding，就默认所有来源都允许发送第三方。

---

# 附：给实现者的执行摘要

1. 不推翻已选产品方向，不扩大 canonical Memory 类型。
2. 第一优先级是无 Dream 的跨端原文召回，以及一次真实 Web→Agent 的闭环。
3. 第二优先级是证据正确、反回声、可纠正，而不是给每条 Memory 打精确概率。
4. 第三优先级是 bounded Dream、幂等提交与可读 diff。
5. 同时固定 Bootstrap/工具序列化，动态内容后置；只在能观察的 API 路径承诺缓存测量。
6. 再加入云向量搜索和更多 adapter。

**Record faithfully. Consolidate cautiously. Retrieve purposefully. Cache what is stable.**

记录要忠实，整理要谨慎，读取要有目的，缓存只复用稳定内容。
