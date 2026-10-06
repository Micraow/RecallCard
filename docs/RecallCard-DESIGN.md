# RecallCard — 统一个人 AI Context 层设计与开发文档

> **状态**：Architecture Baseline / 可直接交给任意开发 Agent 或开发者执行  
> **工作名称**：RecallCard  
> **仓库名建议**：`recallcard`  
> **核心目标**：让属于用户自己的长期 Context / Memory 在 ChatGPT、Claude、DeepSeek、Qwen、Codex、Claude Code 等不同 AI Surface 之间统一、可迁移、可审计、可渐进披露，并始终由用户自己拥有。  
> **文档版本**：v0.1  
> **设计日期**：2026-10-06

---

## 目录

1. [项目一句话定义](#1-项目一句话定义)
2. [问题背景](#2-问题背景)
3. [设计原则](#3-设计原则)
4. [已经确定的关键决策](#4-已经确定的关键决策)
5. [总体架构](#5-总体架构)
6. [Canonical 数据模型：Event 与 Memory](#6-canonical-数据模型event-与-memory)
7. [Raw Event Capture Policy](#7-raw-event-capture-policy)
8. [文件存储与 Git Vault](#8-文件存储与-git-vault)
9. [Dream：从历史到长期 Memory](#9-dream从历史到长期-memory)
10. [Memory 数据模型与时间语义](#10-memory-数据模型与时间语义)
11. [Bootstrap、View 与渐进式披露](#11-bootstrapview-与渐进式披露)
12. [Retrieval 与 Embedding](#12-retrieval-与-embedding)
13. [Agent 接入：MCP + Adapter / Hook](#13-agent-接入mcp--adapter--hook)
14. [Web Chat 接入：Browser Extension + Native Messaging](#14-web-chat-接入browser-extension--native-messaging)
15. [Web Chat 的 Human-in-the-loop 交互](#15-web-chat-的-human-in-the-loop-交互)
16. [统一 Context Action Protocol](#16-统一-context-action-protocol)
17. [Rust + Python 技术架构](#17-rust--python-技术架构)
18. [Git 同步](#18-git-同步)
19. [安全、隐私与平台边界](#19-安全隐私与平台边界)
20. [CLI / 本地接口建议](#20-cli--本地接口建议)
21. [开发阶段规划](#21-开发阶段规划)
22. [验收标准](#22-验收标准)
23. [明确的非目标](#23-明确的非目标)
24. [被否决或暂缓的设计](#24-被否决或暂缓的设计)
25. [仍可后续调整的问题](#25-仍可后续调整的问题)
26. [参考项目与借鉴点](#26-参考项目与借鉴点)
27. [项目哲学与 README 摘要](#27-项目哲学与-readme-摘要)

---

# 1. 项目一句话定义

**RecallCard 是一个 local-first、Git-native、vendor-neutral 的个人 AI Context Runtime：它从不同 Web Chat 和本地 Agent 捕获统一的历史事件，通过可选的 Dream 过程整理成长期 Memory，再通过 MCP 或浏览器扩展把恰当的 Context 渐进式提供给任意 AI。**

三个最重要的句子：

> **Events record what happened.**  
> **Dream decides what is worth remembering.**  
> **Progressive disclosure decides what the model needs to know now.**

中文：

> **Event 记录发生过什么。**  
> **Dream 决定什么值得记住。**  
> **渐进式披露决定此刻应该让模型知道什么。**

RecallCard **不是新的聊天客户端，也不是新的 Agent**。

它位于各种 AI 产品之下：

```text
 ChatGPT Web ─────┐
 Claude Web ──────┤
 Gemini / Qwen ───┤
 DeepSeek Web ────┤
                  │
 Codex ───────────┤
 Claude Code ─────┤
 Cursor / Others ─┤
                  ▼
            ┌────────────┐
            │ RecallCard │
            │ Context OS │
            └────────────┘
```

模型、客户端和订阅都可以更换，但用户自己的 Context 不换。

---

# 2. 问题背景

当前 AI 使用存在几个明显断层：

1. **Memory 被锁在单一产品中**  
   ChatGPT 认识用户，但换到 Claude / DeepSeek / Codex 后，这些上下文通常消失。

2. **Web Chat 和本地 Agent 是两套世界**  
   Web Chat 擅长搜索、文件处理和自然对话；Codex / Claude Code 擅长本地工程执行，但它们的长期历史通常无法自然共享。

3. **导出聊天 ≠ 拥有长期 Context**  
   保存几 GB 历史聊天只能说明“数据还在”，不能让新模型在需要时自然想起相关信息。

4. **向量数据库 ≠ Memory**  
   单纯将全部历史切 chunk、做 embedding，会产生大量噪声，无法很好表达：
   - 当前状态；
   - 旧事实何时失效；
   - 用户长期偏好；
   - 项目曾做出的决策；
   - 一个事实到底来自哪次原始对话。

5. **Agent Trace 与长期 Memory 不应该混为一谈**  
   Shell 输出、文件读取、tool call 等应被记录为历史证据，但不应全部进入长期 Memory。

因此 RecallCard 的目标不是“多存一点”，而是建立：

```text
统一事件历史
      +
长期整理后的 Memory
      +
面向模型的渐进式 Context 访问协议
```

---

# 3. 设计原则

## 3.1 User-owned

用户的 Context 必须首先属于用户自己，而不是 RecallCard 服务端、OpenAI、Anthropic 或其他厂商。

默认不要求任何 RecallCard Cloud。

---

## 3.2 Local-first

核心数据保存在本地。

允许调用云端：

- LLM；
- Embedding；
- Reranker；

但**存储本身不依赖云服务**。

---

## 3.3 File-native

数据库可以存在，但只允许作为可重建索引。

Canonical data 应能被：

- 人；
- Git；
- Codex；
- Claude Code；
- 普通编辑器；

直接查看。

---

## 3.4 Git-native

同步与版本历史使用 Git。

不重新发明账号同步系统。

Git remote 默认不配置，由用户主动选择私有 GitHub/GitLab、自托管 Git 或其他远端。

---

## 3.5 Vendor-neutral

Core 不知道 ChatGPT DOM 长什么样，也不依赖 Codex 内部格式。

所有厂商差异均通过 Adapter 消化。

---

## 3.6 Progressive Disclosure

不要把所有历史塞入 Prompt。

模型只先看到一个很小的 Bootstrap，并知道如何继续访问：

```text
Bootstrap
   ↓
Search / View
   ↓
Memory
   ↓
Sources
   ↓
Raw Events
```

---

## 3.7 Minimal canonical abstractions

不要模拟完整认知科学。

**Canonical knowledge primitive 只保留两个：**

```text
Event
Memory
```

以下都是 derived artifact：

```text
Profile
Topic View
Bootstrap
Embedding
Search Index
Summary View
```

---

## 3.8 Capture 与 Memory 解耦

```text
Raw Event Store = recorder
Dream           = editor
Memory          = edited knowledge
```

捕获到一个事件，不意味着它一定成为 Memory。

---

## 3.9 Transport 与 Action 解耦

同一个 Context Action：

```text
Search(query)
```

在 Codex 中可以表现为 MCP tool call；

在 Web Chat 中可以表现为模型输出的一段结构化文本。

Core 不应依赖具体 transport。

---

## 3.10 Human controls Web Send

Web Chat 中：

- 抓取可自动；
- Context search 可自动；
- Context prepare 可自动；
- Composer injection 可自动；
- Tool-like request 解析可自动；

但：

> **最终 Send 必须由人点击。**

RecallCard v1 不把 consumer Web UI 当成无人值守 API。

---

# 4. 已经确定的关键决策

| 项目 | 决策 |
|---|---|
| 工作名称 | **RecallCard** |
| Core | **Rust** |
| ML / 实验组件 | **Python worker，可选** |
| 部署方式 | **本地单体软件** |
| 必需基础设施 | 不要求 Docker / PostgreSQL / Neo4j / Redis / Qdrant Server |
| Canonical primitives | **Event + Memory** |
| Raw Event | JSONL / append-oriented |
| Memory | Markdown + YAML frontmatter |
| Episode primitive | **不建立** |
| Episode summary | 如果值得长期保存，Dream 后作为普通 Memory |
| Memory 类型 | **不建立严格 enum ontology** |
| Memory 分类 | 可选 `labels[]` / `entities[]` |
| 人类可读视图 | 自动生成 Views |
| Profile / Topic | Derived View，不是独立 canonical object |
| Bootstrap | 自动生成 + 用户可编辑/保护策略 |
| Memory 时间模型 | temporal-aware |
| Memory source | 必须具有 provenance |
| Dream | 可手动触发，不要求实时 |
| Dream executor | API / Web Chat / Local，均可替换 |
| Embedding | 云端可接受，本地可选 |
| Corpus embedding | Dream / rebuild 时批量执行即可 |
| Query embedding | 仅语义搜索时按需实时执行 |
| Retrieval | Bootstrap → structured → BM25/vector → read → sources |
| Agent 输出接口 | MCP |
| Agent 输入 Capture | Adapter / Hook / Transcript importer，不依赖模型自己 `remember()` |
| MCP v1 tools | `bootstrap / search / read / sources` |
| Web 接入 | Browser Extension |
| Browser ↔ local | **Native Messaging** |
| Core IPC | Unix Domain Socket / Windows Named Pipe |
| HTTP API | v1 不作为默认本地接口 |
| Web 最终 Send | **必须人为触发** |
| Git | 同步和历史机制 |
| `.index` | 可删除、可重建、不进 Git |
| Hidden CoT | 不作为标准采集内容 |
| Benchmark 多模型 | v1 不做 |
| Mem0 | 参考其 extraction / consolidation 思路，不直接作为产品底座 |

---

# 5. 总体架构

```text
                 ┌──────────────────────────┐
                 │        AI Surfaces       │
                 │                          │
                 │ ChatGPT  Claude  Qwen    │
                 │ DeepSeek  Codex  CC      │
                 └─────────────┬────────────┘
                               │
             ┌─────────────────┴─────────────────┐
             │                                   │
        Web Adapters                        Agent Adapters
   Browser Extension                    Hooks / Importers / MCP
             │                                   │
       Native Messaging                           │
             └─────────────────┬─────────────────┘
                               ▼
                      ┌─────────────────┐
                      │ RecallCard Core │
                      │      Rust       │
                      └────────┬────────┘
                               │
             ┌─────────────────┼──────────────────┐
             │                 │                  │
             ▼                 ▼                  ▼
         Raw Events         Memories            Views
           JSONL            Markdown          generated
             │                 │                  │
             └─────────────────┼──────────────────┘
                               │
                          Git-managed
                               │
                 ┌─────────────┴─────────────┐
                 ▼                           ▼
            Search Index                 Dream Jobs
          disposable cache                   │
                 │                 ┌─────────┼─────────┐
                 │                 ▼         ▼         ▼
                 │                API     Web Chat   Local
                 │                           │
                 └──────────────┬────────────┘
                                ▼
                         Memory Candidates
                                │
                        Temporal Resolver
                                │
                                ▼
                              Memory
```

---

# 6. Canonical 数据模型：Event 与 Memory

## 6.1 为什么只保留两个

调研成熟项目后，不应额外制造：

```text
Fact
Preference
Decision
Goal
Procedure
Relationship
Episode
Narrative
...
```

这些可以是 Memory 的语义内容或 label，而不应全部成为数据库级类型。

真正稳定的抽象只有：

### Event

> “发生了什么？”

### Memory

> “未来值得知道什么？”

---

# 6.2 Event Envelope

建议所有平台事件先归一化为统一 Envelope：

```json
{
  "schema": 1,

  "id": "evt_01K...",
  "session_id": "ses_01K...",

  "turn_id": "turn_01K...",
  "run_id": "run_01K...",
  "step_id": null,

  "parent_event_id": null,

  "occurred_at": "2026-10-06T17:42:13+08:00",
  "ingested_at": "2026-10-06T17:42:14+08:00",

  "source": {
    "surface": "agent",
    "adapter": "codex",
    "provider": "openai",
    "client": "codex-cli",
    "device_id": "dev_...",
    "external_session_id": "...",
    "external_event_id": "..."
  },

  "actor": {
    "kind": "assistant"
  },

  "kind": "message",
  "payload": {},

  "native_ref": null
}
```

---

## 6.3 两种时间必须分开

### `occurred_at`

事件实际发生时间。

### `ingested_at`

RecallCard 获取它的时间。

例如 2027 年导入 2026 年 ChatGPT 历史：

```text
occurred_at = 2026
ingested_at = 2027
```

---

## 6.4 Session / Turn / Run / Step

它们不是同一个概念。

```text
Session
└── 一个长期 Chat / Agent session

Turn
└── 用户一次输入引发的一轮互动

Run
└── 一次 Agent execution

Step
└── Run 中的一步
```

Web Chat 通常只有：

```text
session
turn
```

Agent 可能完整拥有：

```text
session
turn
run
step
```

所有字段都允许为空。

---

## 6.5 第一版 Event Kind

尽量保持少：

```text
lifecycle
message
tool_call
tool_result
file
citation
approval
custom
```

不建立几十种厂商专属 Event。

---

## 6.6 Agent Trace 不需要单独的 `agent_trace` kind

Agent Trace 由：

```text
run_id
step_id
parent_event_id
lifecycle
```

组合表达。

例如：

```json
{
  "kind": "lifecycle",
  "run_id": "run_01...",
  "payload": {
    "scope": "run",
    "phase": "started"
  }
}
```

结束：

```json
{
  "kind": "lifecycle",
  "run_id": "run_01...",
  "payload": {
    "scope": "run",
    "phase": "finished",
    "status": "success"
  }
}
```

---

## 6.7 Message Event

使用 content parts，而不是无限扩充 Message 类型：

```json
{
  "kind": "message",
  "payload": {
    "role": "assistant",
    "content": [
      {
        "type": "text",
        "text": "..."
      },
      {
        "type": "image_ref",
        "ref": "obj_sha256..."
      }
    ]
  }
}
```

可扩展：

```text
text
image_ref
file_ref
audio_ref
structured
```

---

## 6.8 Tool Call

```json
{
  "kind": "tool_call",
  "run_id": "run_...",
  "step_id": "step_...",
  "payload": {
    "tool_call_id": "call_...",
    "name": "shell",
    "arguments": {
      "command": "rg TODO ."
    }
  }
}
```

---

## 6.9 Tool Result

小内容 inline：

```json
{
  "kind": "tool_result",
  "payload": {
    "tool_call_id": "call_...",
    "status": "success",
    "content": {
      "storage": "inline",
      "text": "..."
    }
  }
}
```

大内容降级：

```json
{
  "kind": "tool_result",
  "payload": {
    "tool_call_id": "call_...",
    "status": "success",
    "content": {
      "storage": "blob",
      "ref": "sha256:...",
      "size": 23818292,
      "preview": "..."
    }
  }
}
```

---

## 6.10 File Event

默认不复制整个文件。

读取：

```json
{
  "kind": "file",
  "payload": {
    "operation": "read",
    "path": "src/main.rs",
    "sha256": "...",
    "size": 18291
  }
}
```

写入：

```json
{
  "kind": "file",
  "payload": {
    "operation": "write",
    "path": "src/main.rs",
    "before_sha256": "...",
    "after_sha256": "...",
    "diff_ref": "sha256:..."
  }
}
```

---

## 6.11 Citation Event

Web citation 应是一等 Event：

```json
{
  "kind": "citation",
  "parent_event_id": "evt_assistant_message",
  "payload": {
    "url": "https://...",
    "title": "...",
    "snippet": "...",
    "accessed_at": "2026-10-06T..."
  }
}
```

因此未来可以形成：

```text
Memory
  ↓
Assistant Message
  ↓
Citation
  ↓
Original Web Source
```

---

## 6.12 Approval Event

对 Agent 中的人类批准保留统一结构：

```json
{
  "kind": "approval",
  "payload": {
    "action_ref": "call_...",
    "decision": "approved"
  }
}
```

---

## 6.13 Provider-native Raw Event

Canonical schema 无法覆盖所有未来厂商细节。

因此允许：

```json
"native_ref": "sha256:..."
```

保存可选的原始 provider payload。

Canonical Event 不应塞进大量 provider-specific 字段。

---

# 7. Raw Event Capture Policy

## 7.1 设计原则

采取：

> **结构尽可能完整，内容按价值/体积选择存储。**

不是简单的“这个事件存，那个不存”。

每个 payload 可以是：

```text
inline
blob
metadata-only
ignored
```

---

## 7.2 默认策略

| 内容 | 默认策略 |
|---|---|
| User message | Inline |
| Assistant final answer | Inline |
| Tool call | Inline |
| 小型 tool result | Inline |
| 大型 shell/tool output | Preview + Blob / truncate |
| File read | Metadata |
| File write | Metadata + diff |
| Generated file | Metadata + optional object |
| Web citation | Inline metadata |
| Fetched webpage | Ref / Blob |
| Agent run/step | Structured metadata |
| Explicit reasoning summary | Optional inline |
| Hidden chain-of-thought | **不抓** |
| Credentials / secrets | Redact / ignore |

---

## 7.3 Streaming Event 不作为 Canonical Event

例如上游可能产生：

```text
TEXT_START
TEXT_DELTA
TEXT_DELTA
TEXT_DELTA
TEXT_END
```

Canonical Vault 中只保存：

```text
message(final text)
```

同理，tool args 的 streaming delta 也在 adapter 层归并。

> Transport events ≠ Historical events.

---

## 7.4 Event Capture 与 Dream 输入不同

一次 Codex Run 可能有 2000 个 Event。

Dream 不应读取全部 2000 个。

Dream 前需要 deterministic projection：

```text
所有 user messages
最终 assistant outputs
关键 tool outcomes
errors
important diffs
citations
approvals
```

大量重复 shell output / file read 不进入 Dream prompt。

---

# 8. 文件存储与 Git Vault

## 8.1 Conceptual Vault

```text
vault/
├── events/
│   └── ...
│
├── memories/
│   ├── mem_01K....md
│   └── ...
│
├── objects/
│   └── sha256/
│
├── native/
│   └── sha256/
│
├── generated/
│   ├── views/
│   └── bootstrap/
│
└── .index/
```

配置和 API keys 不放在 Git Vault。

---

## 8.2 为什么 Event 用 JSONL

Raw Event 具有：

- 流式写入；
- append-heavy；
- 结构化；
- 大量；
- 机器处理为主；

JSONL 最合适。

---

## 8.3 为什么 Memory 用 Markdown

Memory 需要：

- 人类查看；
- Agent 直接 `cat/rg`；
- Git diff；
- 手工修正；
- YAML 元数据；

因此 Markdown + YAML frontmatter 最合适。

---

## 8.4 Git 冲突优化：Event Segment

不建议多设备直接 append 同一个 session 文件。

使用不可变 Segment：

```text
events/
└── 2026/
    └── 10/
        └── ses_01K.../
            ├── seg_devA_01K....jsonl
            └── seg_devB_01K....jsonl
```

每个 capture instance 写自己的 segment。

同步前：

1. rotate 当前 open segment；
2. 已关闭 segment 不再修改；
3. Git 只新增文件。

这样大幅减少跨设备 merge conflict。

---

## 8.5 Git tracking

建议默认追踪：

```text
events/
memories/
```

默认忽略：

```text
.index/
generated/
runtime state
temporary Dream jobs
```

`objects/` 根据大小和隐私单独配置：

- 小对象可 Git；
- 大对象可 local-only；
- 后续支持 Git LFS。

---

# 9. Dream：从历史到长期 Memory

## 9.1 Dream 的定位

Dream 不是必需实时服务。

它是：

> **把过去的 Raw Events 重新整理为未来仍有价值的 Working Context。**

没有 Dream 时：

- Events 继续正常捕获；
- 不丢数据；
- 只是长期 Memory 不更新。

---

## 9.2 Dream 触发方式

v1 首选：

```bash
recallcard dream
```

手动执行。

未来可增加：

```text
manual
idle
scheduled
```

但定时任务不是 v1 必需。

---

## 9.3 Dream Pipeline

```text
Raw Events
    │
    ▼
1. Projection
    │
    ▼
2. Extract
    │
    ▼
3. Consolidate
    │
    ▼
4. Resolve
    │
    ▼
5. Materialize
```

---

## 9.4 Step 1 — Projection

完全 deterministic。

目标：

> 将高噪声 Trace 变成 LLM 能有效处理的 DreamInput。

保留：

- User intent；
- Assistant final result；
- 决策；
- errors；
- tool outcome；
- file diff summary；
- citations；
- approval。

过滤：

- 重复 shell log；
- 巨量 build output；
- 无意义 token streaming；
- 大量普通 file reads。

---

## 9.5 Step 2 — Extract

LLM 从 DreamInput 产生：

```text
MemoryCandidate[]
```

Candidate 不直接落盘。

建议字段：

```json
{
  "content": "...",
  "confidence": 0.92,
  "valid_from": "...",
  "valid_to": null,
  "source_events": ["evt_..."],
  "labels": ["project"],
  "entities": ["recallcard"]
}
```

labels/entities 均为 optional。

---

## 9.6 Step 3 — Consolidate

每个 Candidate 检索相关已有 Memory。

让模型判断关系：

```text
ADD
SUPERSEDE
MERGE
NOOP
CONFLICT
```

这里借鉴 Mem0 等系统的 consolidation 思路，但我们的时间语义与文件存储自行实现。

---

## 9.7 Step 4 — Deterministic Resolver

LLM 提建议，但最终落盘应经过确定性规则。

### Risk-based governance

#### 明确事实

例如用户明确说：

> 我现在主要使用 Arch Linux。

可自动写入。

#### 明确 temporal update

例如：

> 我已经不用 Ubuntu 了。

可关闭旧 memory 的 `valid_to`，建立新 memory。

#### 推断

例如多次行为推断：

> 用户偏好 Rust。

可以：

```text
status: tentative
confidence: lower
```

不直接进入高信任 Bootstrap。

#### Conflict

两个可信 source 明显矛盾：

```text
status: conflict
```

进入 review queue。

#### Identity / sensitive / high-impact change

不直接覆写用户核心 identity。

只能提出建议，由用户确认。

---

## 9.8 Step 5 — Materialize

成功 resolve 后：

```text
memories/*.md
   ↓
generated views
   ↓
embedding batch
   ↓
search index
   ↓
Git diff
```

---

## 9.9 Dream Executor 必须可替换

```text
DreamExecutor
├── ManualWebExecutor
├── ApiExecutor
└── LocalExecutor
```

### Manual Web

即使用户没有 API 预算，也能：

```bash
recallcard dream --export
```

Browser Extension 将 Dream Job 注入：

- ChatGPT；
- Claude；
- DeepSeek；
- Qwen；

用户点击 Send。

模型输出结构化 DreamResult 后：

```bash
recallcard dream --import result.json
```

或由浏览器扩展检测并准备导入。

### API

适合无人值守、便宜模型。

兼容：

- OpenAI-style endpoint；
- Anthropic-style endpoint；
- provider adapter。

### Local

Python worker 可以接本地模型。

v1 不要求实现完整本地 LLM。

---

## 9.10 Dream 成本估算

成本不是核心约束，因为 Manual Web 是正式支持路径。

但廉价 API 本身已经足够实用。

以 **500 万 input tokens + 20 万 output tokens / 月** 为例：

### Qwen3.7-Flash（2026-10 价格快照）

在单请求输入不超过 32K token 的定价档：

```text
input  ¥0.2 / 1M
output ¥0.8 / 1M
```

则：

```text
5M × 0.2 + 0.2M × 0.8
≈ ¥1.16 / 月
```

Batch 半价时约：

```text
¥0.58 / 月
```

因此 Dream Job 应尽量 chunk 到廉价区间。

> 价格会变化，实现时必须重新读取 provider 官方价格。

参考：
https://help.aliyun.com/zh/model-studio/model-pricing

### DeepSeek Flash

同类廉价模型也可作为 Dream Executor。

不要在 Core 中硬编码某个推荐模型。

参考：
https://api-docs.deepseek.com/quick_start/pricing/

---

# 10. Memory 数据模型与时间语义

## 10.1 不建立严格 MemoryType

不定义：

```rust
enum MemoryType {
    Fact,
    Preference,
    Decision,
    Goal,
    Procedure,
    ...
}
```

原因：

- 真实信息通常跨多个类别；
- 分类不会显著改善核心功能；
- 会给 Dream 造成额外负担；
- 成熟项目通常也没有统一 ontology。

使用开放 metadata：

```yaml
labels:
  - project
  - decision

entities:
  - recallcard
```

即可。

---

## 10.2 Memory Markdown 示例

```markdown
---
id: mem_01K...

status: active
confidence: 0.97
authority: dream

valid_from: 2026-10-06
valid_to: null

observed_at: 2026-10-06T15:20:00+08:00
recorded_at: 2026-10-06T17:30:00+08:00

sources:
  - event: evt_01K...
  - event: evt_01K...

supersedes: []

labels:
  - project
  - architecture

entities:
  - recallcard
---

RecallCard 的核心采用 Rust，Python 仅作为可选 ML/实验 worker。
```

---

## 10.3 Memory 的时间字段

### `valid_from`

现实世界中何时开始成立。

### `valid_to`

现实世界中何时不再成立。

### `observed_at`

用户/系统何时观察到、得知该信息。

### `recorded_at`

RecallCard 何时把它写成 Memory。

例如用户 10 月告诉系统：

> 我九月底换成 Arch 了。

则：

```text
valid_from  ≈ 2026-09
observed_at = 2026-10
recorded_at = 2026-10
```

---

## 10.4 不覆盖旧事实

错误方式：

```text
UPDATE Ubuntu → Arch
```

正确方式：

```text
mem_A:
Ubuntu
valid_to = 2026-09

mem_B:
Arch
valid_from = 2026-09
supersedes = [mem_A]
```

因此系统能回答：

> 去年用什么？

也能回答：

> 现在用什么？

---

## 10.5 Authority

为了支持用户保护核心 Context，可保留一个非常小的 authority 字段：

```text
user
dream
import
```

`authority: user` 的 Memory 不应被 Dream 自动删除或重写。

---

# 11. Bootstrap、View 与渐进式披露

## 11.1 Views 不是 canonical

Views 从 Memories 自动生成：

```text
generated/views/
├── profile.md
├── computing.md
├── projects.md
├── current-state.md
└── topics/
```

删除后可以重建。

---

## 11.2 Bootstrap

每个新 AI Surface 不需要先猜用户问题属于哪个 Topic。

只注入很小的 Bootstrap：

```text
Stable Profile
Current State
Context Index
Context Access Protocol
```

例如：

```text
User context:
- CS student.
- Primary environment: Arch Linux + KDE + Wayland.
- ...

Available context areas:
- computing
- networking
- projects
- courses
- ...

If additional personal context would materially improve the answer,
request it through RecallCard.
```

---

## 11.3 不做前置 Topic Router

原则：

> 不需要 RecallCard 在用户刚输入几个字时抢先猜 Topic。

模型本身通常比一个额外 classifier 更理解当前任务。

RecallCard 应告诉模型：

> “有更多 Context 可查。”

然后让模型自己逐层调用。

---

## 11.4 Progressive Disclosure Levels

```text
L0 Bootstrap
│
├── 最稳定的核心 Profile
├── Current State
└── 可访问 Context 的目录
        │
        ▼
L1 Generated View
│
│ computing / project / academic ...
        │
        ▼
L2 Search Result
│
│ 轻量候选 + summary
        │
        ▼
L3 Full Memory
        │
        ▼
L4 Source / Raw Event
```

越靠近原始外部数据，越晚披露。

---

## 11.5 Bootstrap 自动生成 + 用户保护

采取：

> 自动生成 + 用户可编辑 + protected behavior。

不要把“用户手工编辑的 identity”变成独立复杂数据系统。

推荐做法：

- 用户修改核心身份时生成 `authority: user` 的 Memory；
- Dream 只能建议新的高影响修改；
- Bootstrap 仍然由 Memory 生成。

这样 Canonical knowledge 仍然只有：

```text
Event + Memory
```

---

# 12. Retrieval 与 Embedding

## 12.1 Embedding 的定位

Embedding 只是：

> semantic candidate retriever

不是 Memory，也不是 Truth Resolver。

---

## 12.2 Retrieval Pipeline

```text
Model request
     │
     ▼
Bootstrap / Named View
     │
     ▼
Structured filtering
(time / entity / labels / source)
     │
     ▼
Hybrid Search
BM25 + vector
     │
     ▼
Optional rerank
     │
     ▼
read(memory)
     │
     ▼
sources(memory)
```

---

## 12.3 Corpus Embedding 不要求实时

历史 Memories 在：

```text
dream
rebuild
```

阶段批量 embedding。

---

## 12.4 Query Embedding

只有发生 semantic search 时才实时 embedding 当前 query。

通常只有几十 token，成本很低。

---

## 12.5 不依赖 Embedding 的路径

以下情况无需 embedding：

```text
bootstrap()
read(view:computing)
read(memory:id)
sources(memory:id)
```

因此 Embedding Provider 挂掉时，RecallCard 仍然能工作。

---

## 12.6 Cloud-first，但 Local 可选

用户不特别介意云 embedding，因此默认实现应优先保证：

- 便宜；
- 跨平台；
- 零模型下载；
- 高质量多语言。

建议 Provider interface：

```text
EmbeddingProvider
├── OpenAICompatible
├── DashScope
├── CustomHTTP
└── LocalPython
```

不要把具体服务写死。

---

## 12.7 参考云 Embedding 成本

2026-10 阿里云 `qwen3.7-text-embedding`：

```text
¥0.5 / 1M input tokens
Batch: ¥0.25 / 1M
```

因此即使对 **1 亿 token** 历史做一次完整 embedding：

```text
实时：约 ¥50
Batch：约 ¥25
```

个人场景通常远低于此规模。

参考：
https://help.aliyun.com/en/model-studio/embedding

---

## 12.8 Local Embedding

可选 Python worker：

```text
Qwen3-Embedding-0.6B
OpenVINO / Transformers
```

本地路径用于：

- 隐私敏感用户；
- 完全离线；
- 无 API 时。

不应让本地 embedding 环境成为安装 RecallCard 的前置条件。

---

## 12.9 Search Index 是 disposable

Canonical Memory 不放向量数据库。

索引：

```text
.index/
```

允许：

```bash
rm -rf .index
recallcard rebuild
```

完全恢复。

FTS / BM25 / vector backend 可以以后替换，不泄漏进 Vault schema。

---

# 13. Agent 接入：MCP + Adapter / Hook

## 13.1 两条不同的数据平面

Agent 接入应分成：

### Capture plane

自动获取：

- message；
- tool call；
- tool result；
- file event；
- run / step；

来源：

- Hooks；
- Session transcript；
- Provider logs；
- 插件提供的事件；
- post-run importer。

### Context plane

Agent 主动查询 RecallCard。

使用 MCP。

---

## 13.2 不让模型负责 Capture

不依赖：

```text
LLM decides:
store_memory(...)
```

否则容易：

- 漏记；
- 重复；
- Agent 兴奋时写几十条垃圾 Memory。

Capture 应尽可能由 Adapter / Hook 在模型之外完成。

---

## 13.3 MCP v1 只暴露四个工具

### `bootstrap`

返回：

- small core profile；
- current state；
- context index；
- progressive disclosure 指引。

### `search`

输入：

```json
{
  "query": "azurehk ipv6",
  "limit": 5
}
```

返回轻量候选：

```text
ref
title/summary
time
relevance
source hint
```

### `read`

读取一个 Ref：

```text
memory:...
view:...
```

### `sources`

沿 provenance 下钻：

```text
Memory
  ↓
source events
  ↓
conversation / citation / tool result
```

---

## 13.4 v1 不暴露 `remember`

正常长期 Memory 通过：

```text
Capture → Dream
```

产生。

以后如果真的需要显式用户 pin，可以新增：

```text
pin
```

但不是 v1 所需能力。

---

## 13.5 Plugin / Skill 的角色

MCP 负责：

> 提供能力。

Plugin / Skill 负责：

> 教模型何时使用能力。

例如 Codex Plugin 可以包含：

```text
Skill:
You have access to a user-owned context system.
Start with bootstrap.
Use search/read/sources progressively when prior context is useful.

MCP:
recallcard mcp
```

不同 Agent 的 Plugin/Skill 格式可以不同。

真正统一的是 MCP 和 RecallCard Context Protocol。

---

# 14. Web Chat 接入：Browser Extension + Native Messaging

## 14.1 为什么需要 Browser Extension

Web Chat 没有标准 MCP。

需要 Adapter 负责：

- 捕获 Web Chat message/citation；
- 插入 Bootstrap；
- 解析模型提出的 Context request；
- 从 RecallCard 获取结果；
- 写入 composer；
- 永远不自动 Send。

---

## 14.2 为什么使用 Native Messaging

Browser Extension 不能直接读本地 Vault。

Native Messaging 提供标准桥：

```text
Web page
   │
Content Script
   │
Extension Service Worker
   │
runtime.connectNative()
   │
recallcard native-host
   │
local IPC
   │
RecallCard daemon
```

好处：

- 不监听 localhost TCP port；
- 不需要 CORS；
- 减少 CSRF / localhost exposure；
- 浏览器通过 Extension ID 限制可连接 native host；
- 适合密码管理器等敏感本地应用的典型架构。

---

## 14.3 v1 不默认开放 localhost HTTP

未来如果需要：

- 手机；
- LAN；
- remote MCP；
- Web Dashboard；

可以增加 HTTP。

但不是本地 Browser Extension 必需条件。

---

# 15. Web Chat 的 Human-in-the-loop 交互

## 15.1 核心约束

RecallCard 可以：

```text
自动 capture
自动 retrieve
自动 prepare
自动 inject
```

但：

```text
Send = human click
```

---

## 15.2 ArcShift 式 UX

目标体验：

1. 用户切到 ChatGPT / Claude；
2. RecallCard Extension 已准备好 Bootstrap；
3. composer 中出现可见 Context；
4. 光标在 Context 后；
5. 用户继续输入；
6. 用户检查内容；
7. 用户自己点击 Send。

不是后台偷偷改 Prompt。

---

## 15.3 Context Capsule

内部不要只产生一坨 string。

先产生：

```text
ContextCapsule
├── reason
├── sections
├── token estimate
├── references
└── trust metadata
```

Web Adapter 再序列化成对应 Web Chat 的文本格式。

UI 可以显示：

```text
┌ Personal Context · 900 tokens ┐
│ Stable profile                │
│ Current project state         │
│ Context access instructions   │
│                               │
│ [Preview] [Remove]            │
└───────────────────────────────┘
```

---

## 15.4 不自动塞大量 Topic Context

新 Chat 只注入最小 Bootstrap。

不要因为页面刚打开就做复杂 Topic inference。

更多 Context 由模型主动请求。

---

# 16. 统一 Context Action Protocol

## 16.1 Internal Action

Core 内部使用稳定的 action enum：

```rust
enum ContextAction {
    Bootstrap,
    Search { query: String, limit: usize },
    Read { reference: String },
    Sources { reference: String },
}
```

---

## 16.2 MCP Transport

Agent 中：

```text
ContextAction
   ↕
native MCP tool call
```

---

## 16.3 Web Transport

Web Chat 无 native tool call 时：

```text
ContextAction
   ↕
in-band structured action block
```

建议使用严格 JSON + 代码块，而不是脆弱的自由文本 marker：

````text
```recallcard-action
{
  "protocol": "recallcard/1",
  "nonce": "...",
  "action": "search",
  "query": "azurehk ipv6 return path",
  "limit": 5
}
```
````

Extension 解析后调用 Context Core。

---

## 16.4 Nonce

每个 Web Chat 注入 Bootstrap 时生成短期 nonce。

模型必须在 action 中带回。

作用：

- 减少页面里普通文字误触发 action；
- 避免历史文章中恰好包含 RecallCard 标记；
- 为 adapter 提供最基本的 request binding。

---

## 16.5 Web Tool Loop

```text
User asks
   ↓
Web LLM
   ↓
recallcard-action
   ↓
Extension parses
   ↓
Core search/read
   ↓
Extension puts result in composer
   ↓
USER REVIEWS
   ↓
USER CLICKS SEND
   ↓
Web LLM continues
```

RecallCard 不自动点击 Send。

---

# 17. Rust + Python 技术架构

## 17.1 Rust 掌握系统主权

Rust Core 负责：

```text
Vault
Event schema
Memory schema
Temporal resolver
Git
Index orchestration
MCP
Native Messaging bridge
Local IPC
CLI
Context Compiler
Adapter protocol
Validation
```

---

## 17.2 Python 是可选 worker

Python worker 负责适合 Python ML 生态的部分：

```text
local embedding
local reranker
local LLM experiment
OpenVINO
Transformers
memory extraction experiments
```

Core 在 Python 不存在时仍然工作。

---

## 17.3 避免 Rust ↔ Python FFI 强耦合

推荐：

```text
Rust
  │
  │ JSON-RPC / framed JSON
  ▼
Python worker
```

通过：

- stdio；
- Unix socket；
- Named Pipe；

之一通信。

第一版优先 stdio，简单且容易管理生命周期。

---

## 17.4 一个 Binary，多种模式

目标：

```bash
recallcard daemon
recallcard mcp
recallcard native-host
recallcard dream
recallcard rebuild
recallcard import
recallcard sync
recallcard doctor
```

可以是同一 Rust executable 的不同 subcommand。

---

## 17.5 Local IPC

Daemon 运行时：

Linux / macOS：

```text
Unix Domain Socket
```

Windows：

```text
Named Pipe
```

`mcp` / `native-host` 可以只是桥，连接 daemon。

如果 daemon 不在运行，可按策略自动启动。

---

# 18. Git 同步

## 18.1 Git 是正式同步机制

RecallCard v1 不做账号式 Cloud Sync。

```text
Laptop
   ↕
 Git remote
   ↕
Desktop / Server
```

---

## 18.2 默认没有 remote

初始化：

```bash
recallcard init
```

只建立本地 Git repo。

用户主动：

```bash
git remote add origin ...
```

或：

```bash
recallcard sync setup ...
```

---

## 18.3 Remote 必须被视为敏感数据存储

即使 private GitHub repo，也可能包含：

- 私人聊天；
- 项目资料；
-机器信息；
-长期习惯；
-课程内容。

文档应明确提醒。

未来可选 E2EE Git workflow，但 v1 不强制实现。

---

## 18.4 Sync 流程

建议：

```bash
recallcard sync
```

做：

1. rotate open event segment；
2. validate Vault；
3. `git pull --rebase`；
4. 如有冲突则停止；
5. rebuild derived state if necessary；
6. commit local pending canonical changes；
7. push。

不要静默解决语义 Memory conflict。

---

## 18.5 Generated Files 不同步

以下应重建：

```text
.index/
generated/views/
generated/bootstrap/
```

从而减少 merge conflict。

---

# 19. 安全、隐私与平台边界

## 19.1 不采集 Hidden CoT

不同 provider：

- 不一定提供；
- 可能只是 reasoning summary；
- 语义不稳定；
- 不是可移植 API。

RecallCard 标准不依赖 hidden reasoning。

---

## 19.2 Prompt Injection

外部网页 / tool result 属于低信任 source。

RecallCard 应记录 trust/provenance。

不要把未经整理的：

```text
网页原文
tool output
外部文件
```

自动放进 Bootstrap。

它们只能在更深层 progressive disclosure 中出现。

---

## 19.3 Secret Redaction

写入 Canonical Vault 前，至少支持：

- known secret file patterns；
- environment variable filters；
- API key regex；
- user ignore rules；
- configurable path exclude。

发生 redaction 时，Event 应记录：

```text
redacted: true
redaction_count: N
```

但不记录被删的秘密内容。

---

## 19.4 Browser Security

Extension 必须：

- 校验 origin；
- 不信任 content script 输入；
- 校验 nonce；
- 使用严格 action schema；
- Native Messaging host 只允许指定 extension ID；
- 不向任意网页暴露本地 Context API。

---

## 19.5 Consumer Web Chat 条款

即使 Send 由人点击，不同服务也可能限制：

- 自动抓取 DOM；
- 自动保存 Output；
- 自动交互。

因此：

1. 每个 Web Adapter 是独立可维护组件；
2. 不绕过反自动化措施；
3. 不模拟无头批量访问；
4. 不把 Web Chat 作为无人值守 API；
5. 实现/发布适配器前重新检查对应平台当前条款。

---

# 20. CLI / 本地接口建议

## 20.1 Core Commands

```bash
recallcard init
recallcard status
recallcard doctor

recallcard import <source>
recallcard capture ...

recallcard dream
recallcard dream --export
recallcard dream --import result.json

recallcard search "..."
recallcard read <ref>
recallcard sources <ref>

recallcard rebuild
recallcard validate

recallcard sync
```

---

## 20.2 Internal IPC Methods

内部 IPC 可定义：

```text
event.ingest
event.query

context.bootstrap
context.search
context.read
context.sources

dream.prepare
dream.apply

index.rebuild
vault.validate
git.status
```

这些不是都需要暴露给模型。

---

## 20.3 MCP Surface

严格保持：

```text
bootstrap
search
read
sources
```

---

# 21. 开发阶段规划

## Phase 0 — Contract First

目标：

> 在写真实 Adapter 前固定 schema 和 fixtures。

完成：

- Rust workspace；
- Event JSON Schema；
- Memory frontmatter schema；
- ULID ID policy；
- path layout；
- sample fixtures；
- schema versioning；
- validator。

验收：

```text
sample event → parse → serialize → identical semantics
memory markdown → parse → validate
```

---

## Phase 1 — Vault + Event Core

完成：

- Event segment writer；
- Event reader；
- Session/turn/run/step relations；
- Blob/object abstraction；
- file/native refs；
- redaction pipeline；
- CLI import/export；
- Git init。

暂时不做 Dream。

验收：

> 能从 fixture 写入完整 Raw Event history，并通过 `rg` / 普通编辑器查看。

---

## Phase 2 — First Agent Integration

先选一个 Agent：

> Codex 或 Claude Code。

实现：

- Session importer / hook adapter；
- Capture → Event normalization；
- MCP bridge；
- `bootstrap/search/read/sources`；
- basic BM25 fallback。

这一步应该证明：

> 一个 Agent 的历史可以被另一个新 session 查询。

---

## Phase 3 — Memory + Manual Dream

实现：

- Dream projection；
- DreamJob schema；
- DreamResult schema；
- manual export/import；
- MemoryCandidate validator；
- temporal resolver；
- risk governance；
- Memory Markdown writer；
- generated views。

暂时无需 API 自动 Dream。

验收：

> 把一段真实历史导出到任意 Web LLM，导回 DreamResult，可以产生可追溯 Memory。

---

## Phase 4 — Search + Embedding

实现：

- BM25；
- EmbeddingProvider interface；
- Cloud provider；
- batch corpus embedding；
- query embedding；
- hybrid fusion；
- temporal filter；
- index rebuild。

本地 embedding worker 可以放到 Phase 4.5。

验收：

> 模糊语义 query 能找到正确 Memory；删掉 `.index` 后可以完全重建。

---

## Phase 5 — Browser Extension

先只支持一个平台：

> ChatGPT Web。

实现：

- message capture；
- citation capture；
- Context Capsule；
- Bootstrap injection；
- RecallCard action parser；
- Native Messaging；
- composer injection；
- **no auto-send**。

随后扩展：

```text
Claude
DeepSeek
Qwen
Gemini
...
```

所有 DOM selector 均必须位于独立 adapter。

---

## Phase 6 — Web Dream UX

实现：

```text
recallcard dream
   ↓
Open/prepare Web Chat
   ↓
inject DreamJob
   ↓
user Send
   ↓
detect DreamResult
   ↓
preview/import
```

---

## Phase 7 — Git Sync

实现：

- segment rotation；
- pull/rebase；
- validation；
- commit；
- push；
- conflict detection；
- rebuild after sync。

---

## Phase 8 — Cross-platform / Hardening

CI：

```text
Linux
Windows
macOS
```

完成：

- Native Messaging installers；
- Unix socket / Named Pipe；
- browser extension packaging；
- credential storage；
- crash recovery；
- migration；
- schema upgrade。

---

# 22. 验收标准

## 22.1 Portability

场景：

1. 在 ChatGPT Web 谈论一个新项目；
2. Capture；
3. Dream；
4. 打开 Codex；
5. Codex `bootstrap/search/read`；
6. 能自然使用相关历史。

反向也必须成立。

---

## 22.2 Provenance

任意 Memory：

```bash
recallcard sources mem_xxx
```

必须能追溯到：

```text
Raw Event
Conversation
Citation / Tool Result
```

---

## 22.3 Temporal Correctness

历史：

```text
2025 Ubuntu
2026 Arch
```

Query：

```text
现在用什么？
→ Arch

去年用什么？
→ Ubuntu
```

---

## 22.4 File-native

即使 RecallCard executable 暂时不可用，用户仍然可以：

```bash
cat memories/...
rg "keyword" memories/
git log -p
```

读懂自己的长期 Memory。

---

## 22.5 Disposable Index

```bash
rm -rf .index
recallcard rebuild
```

必须恢复完整检索能力。

---

## 22.6 No Auto-send

Browser Adapter 的任何测试都不得出现：

```text
extension automatically clicks Send
```

---

## 22.7 Degraded Mode

没有：

- Python；
- Embedding API；
- Dream API；

时，系统仍然支持：

```text
Capture
Vault
Git
Bootstrap
Read
Sources
BM25/basic search
Manual Dream export
```

---

## 22.8 Git Clone Recovery

新设备：

```bash
git clone <vault>
recallcard rebuild
```

可以恢复：

- Memories；
- Events；
- Bootstrap/Views；
- Search index。

---

# 23. 明确的非目标

v1 **不做**：

- 新的 ChatGPT/Claude 替代前端；
- 自己的 LLM；
- Agent orchestration framework；
- SaaS 用户账号系统；
- RecallCard Cloud；
- Neo4j knowledge graph；
- 复杂 cognitive memory ontology；
- 多模型 Dream benchmark 平台；
- 自动 Web Send；
- headless Web Chat API 替代；
- hidden chain-of-thought archive；
- mandatory local model；
- mandatory Docker；
- mandatory vector database server；
- 自动 E2EE cloud sync；
- 手机客户端。

---

# 24. 被否决或暂缓的设计

## 24.1 直接以 Mem0 为产品底座

否决原因：

- 数据模型不是 file-native；
- temporal semantics 不完全符合目标；
- 我们不需要它的完整 storage/server 结构；
- fork 后会长期承担 upstream merge 成本。

处理方式：

> 阅读、参考、必要时复用 Apache-2.0 允许的算法/代码，但 RecallCard Core 独立设计。

---

## 24.2 Canonical Episode

选择：

```text
Events → Memory
```

而不是：

```text
Events → Episode → Memory
```

某次讨论若值得总结：

```text
labels: [session-summary]
```

作为普通 Memory 即可。

---

## 24.3 MemoryType ontology

不强制：

```text
fact/preference/decision/goal/procedure...
```

开放 labels 即可。

---

## 24.4 前置 Topic Router

不需要在用户刚输入几个字时猜 Topic。

Bootstrap 告诉 LLM 有什么 Context，LLM 自己决定是否下钻。

---

## 24.5 强制本地 Embedding

不需要。

云 embedding 足够便宜且质量通常更高、部署更简单。

保留 Local Provider 即可。

---

## 24.6 每条消息实时 Dream

不需要。

Raw Event 可以先积累，Dream 手动/批量执行。

---

## 24.7 v1 本地 HTTP Server

不需要。

采用：

```text
Agent → MCP stdio
Browser → Native Messaging
Internal → Unix Socket / Named Pipe
```

---

# 25. 仍可后续调整的问题

这些问题不阻碍开发启动：

1. 最终 License；
2. 默认 BM25/FTS backend；
3. 默认 vector index 实现；
4. Browser Extension 第二维平台支持顺序；
5. objects 是否默认进入 Git；
6. Git LFS 是否内置；
7. 自动 scheduled Dream 是否进入 v1.x；
8. Review UI 是 CLI/TUI 还是 Desktop；
9. 本地 embedding 默认模型；
10. Remote MCP 是否支持；
11. 最终仓库名称是否保持 `recallcard`。

---

# 26. 参考项目与借鉴点

> 以下是设计研究参考，不代表 RecallCard 依赖这些项目。实现时应重新检查其最新版本、协议和许可证。

## Mem0

https://github.com/mem0ai/mem0

借鉴：

- conversation → durable memory；
- extraction；
- dedup / consolidation；
- memory search。

不照搬：

- canonical storage；
- server-first deployment；
- Memory 数据模型。

---

## Letta

https://github.com/letta-ai/letta

借鉴：

- stateful context；
- progressive disclosure；
- context repository；
- file/Git 作为 Agent context primitive；
- sleep-time/background reflection 思想。

---

## Zep / Graphiti

https://github.com/getzep/graphiti

借鉴：

- temporal facts；
- provenance；
- source → derived knowledge；
- 事实失效而非简单覆盖。

RecallCard 的 Event 已承担很多 Graphiti Episode 的职责。

---

## LangMem

https://github.com/langchain-ai/langmem

借鉴：

- background memory manager；
- memory extraction 与 agent runtime 解耦。

不绑定 LangGraph。

---

## TiMEM

https://github.com/TiMEM-AI/TiMEM

借鉴：

- 不同时间尺度的 consolidation；
- profile 是长期整理结果，而不是碎片事实堆。

不照搬复杂层级为 canonical schema。

---

## Supermemory

https://github.com/supermemoryai/supermemory

借鉴：

- profile + dynamic context；
- cross-agent context；
- user-centric memory。

---

## AG-UI

https://github.com/ag-ui-protocol/ag-ui

借鉴：

- Run / Step / Message / Tool Call 的事件建模；
- Agent ↔ UI event stream；
- native/raw provider event 的兼容思路。

RecallCard 不存所有 streaming delta。

---

## OpenTelemetry GenAI Semantic Conventions

https://opentelemetry.io/docs/specs/semconv/gen-ai/

借鉴：

- input/output messages；
- tool calls/results；
- GenAI trace correlation；
- structured event semantics。

RecallCard 是历史/context 系统，不是 observability backend，因此只借语义设计。

---

## Model Context Protocol

https://modelcontextprotocol.io/

Agent Context Plane 的主要协议。

RecallCard v1 MCP Surface：

```text
bootstrap
search
read
sources
```

---

## Chrome Native Messaging

https://developer.chrome.com/docs/extensions/develop/concepts/native-messaging

Browser Extension ↔ RecallCard local core 的推荐通信方式。

---

## ArcRift

https://github.com/Eshaan-Nair/ArcRift

借鉴：

- Browser Chat + local memory + MCP；
- 多 Web Chat adapter；
- cross-surface memory。

---

## Sovseal

https://github.com/sovseal/core

借鉴：

- Browser Extension；
- Native Messaging；
- MCP；
- local-first memory engine。

---

## ArcShift 交互参考

https://www.youtube.com/watch?v=58zbSxzQ94U

重点借鉴的是用户观察到的交互：

> 在切换到 Web Chat 后，Context 已经被准备/注入到 composer，用户可以看到并最终自己 Send。

---

# 27. 项目哲学与 README 摘要

## 项目名称

# RecallCard

### Tagline

> **Your memory, carried across every AI.**

或者：

> **One memory. Every AI.**

---

## README 开头建议

```markdown
# RecallCard

**Your memory, carried across every AI.**

RecallCard is a local-first personal context layer that lets
ChatGPT, Claude, DeepSeek, Codex, Claude Code, and other AI tools
share the same user-owned memory.

It does not replace your AI clients.

Instead, it sits underneath them.

- Events record what happened.
- Dream decides what is worth remembering.
- Progressive disclosure decides what the model needs to know now.

Your conversations remain yours.
Your memories are plain files.
Your history is versioned with Git.
Your AI provider can change without resetting who the AI knows you to be.
```

---

# Appendix A — 推荐的 Repository 初始布局

```text
recallcard/
├── Cargo.toml
├── crates/
│   ├── recallcard-core/
│   ├── recallcard-schema/
│   ├── recallcard-vault/
│   ├── recallcard-memory/
│   ├── recallcard-search/
│   ├── recallcard-mcp/
│   ├── recallcard-native/
│   ├── recallcard-git/
│   └── recallcard-cli/
│
├── python/
│   └── recallcard_ml/
│       ├── embeddings/
│       ├── rerank/
│       └── experiments/
│
├── extension/
│   ├── src/
│   │   ├── core/
│   │   └── adapters/
│   │       ├── chatgpt/
│   │       ├── claude/
│   │       ├── deepseek/
│   │       └── qwen/
│   └── manifest.json
│
├── integrations/
│   ├── codex/
│   └── claude-code/
│
├── schemas/
│   ├── event.schema.json
│   ├── dream-job.schema.json
│   └── dream-result.schema.json
│
├── fixtures/
│   ├── chatgpt/
│   ├── codex/
│   └── claude-code/
│
├── docs/
│   ├── architecture.md
│   ├── event-model.md
│   ├── dream.md
│   ├── adapter-guide.md
│   ├── security.md
│   └── decisions/
│
└── README.md
```

---

# Appendix B — Adapter Contract 草案

每个 Adapter 至少声明 capability：

```json
{
  "capture": {
    "messages": true,
    "tool_calls": false,
    "tool_results": false,
    "files": false,
    "citations": true,
    "runs": false
  },

  "context": {
    "bootstrap": true,
    "structured_tool_call": false,
    "text_action_protocol": true,
    "composer_injection": true,
    "automatic_send": false
  }
}
```

Agent Adapter 示例：

```json
{
  "capture": {
    "messages": true,
    "tool_calls": true,
    "tool_results": true,
    "files": true,
    "citations": true,
    "runs": true
  },

  "context": {
    "bootstrap": true,
    "structured_tool_call": true,
    "text_action_protocol": false,
    "composer_injection": false,
    "automatic_send": null
  }
}
```

---

# Appendix C — DreamJob 草案

```json
{
  "schema": 1,
  "job_id": "dream_01K...",
  "created_at": "...",

  "window": {
    "from": "...",
    "to": "..."
  },

  "projection": {
    "sessions": 12,
    "events": 428,
    "text": "..."
  },

  "instructions": {
    "extract_durable_context": true,
    "preserve_provenance": true,
    "infer_temporal_validity": true,
    "avoid_trivia": true,
    "avoid_hidden_reasoning": true
  },

  "expected_result_schema": "recallcard/dream-result/1"
}
```

---

# Appendix D — DreamResult 草案

```json
{
  "schema": 1,
  "job_id": "dream_01K...",

  "candidates": [
    {
      "content": "...",
      "confidence": 0.91,
      "valid_from": null,
      "valid_to": null,
      "source_events": [
        "evt_..."
      ],
      "labels": [],
      "entities": []
    }
  ]
}
```

Consolidation 可以由第二阶段 Dream call 或 Core 内单独 pipeline 完成。

---

# Appendix E — 最小 Context Search Result

```json
{
  "ref": "memory:mem_01K...",
  "summary": "用户当前主要使用 Arch Linux + KDE + Wayland。",
  "valid_from": "2026-09-29",
  "valid_to": null,
  "confidence": 0.98,
  "source_count": 3
}
```

search 结果保持轻量。

完整内容必须通过：

```text
read(ref)
```

继续下钻。

---

# Appendix F — 开发者实施优先级

如果交给一个新的开发 Agent，从以下顺序开始，**不要直接先写 Browser Extension**：

1. 阅读本文件；
2. 固定 `Event` JSON Schema；
3. 固定 `Memory` Markdown schema；
4. 实现 Vault writer / reader / validator；
5. 用 fixture 模拟 ChatGPT + Codex；
6. 实现最小 MCP；
7. 实现 Manual Dream；
8. 实现 Search；
9. 再做第一个真实 Agent Adapter；
10. 最后做第一个 Web Adapter。

最早的技术验证应该是：

```text
Codex fixture
    ↓ capture
Event Vault
    ↓ manual Dream
Memory
    ↓ MCP
Second Agent
```

而不是：

```text
先花两周维护 ChatGPT DOM selector
```

Web Adapter 是必要产品体验，但不应该成为 Context Core 正确性的前置条件。

---

# Appendix G — 一条最终约束

如果未来实现过程中出现“要不要再加一个 abstraction”的争论，使用下面这条规则：

> **如果当前 Event + Memory + Derived View + Index 无法解决一个已经存在的实际问题，才允许增加新的 canonical abstraction。**

不要为了理论完整性添加：

- 新 Memory 类型；
- 新层级；
- 新数据库；
- 新服务；
- 新 Agent runtime。

RecallCard 的价值不在于拥有最复杂的 Memory Architecture，而在于：

> **让一份真正属于用户自己的 Context，以最小摩擦跨越所有 AI Surface。**
