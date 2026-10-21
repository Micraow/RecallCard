# RecallCard 设计

把跨 AI 的对话变成可追溯的本地事实，让下一个 AI 接着用。

## 不变量

1. **只有两个事实源**：`events/`（原始事件，只追加）和 `memories/`（人工确认的长期记忆）。其余一切都是派生物。
2. **派生物必须可重建**：语义索引、`generated/views/` 随时可删，`recallcard rebuild` / `index build` 能整份重做。
3. **模型侧只读**：MCP 客户端只能 `knowledge_write` 追加一条 Event，不能改 Memory、不能删任何东西。
4. **联网必须由人按下**：没有任何后台常驻服务，没有自动上传。云 embedding 只在显式配置后、由人触发的命令使用。
5. **助手建议不等于用户决定**：AI 生成的记忆一律 `assistant_suggestion`，未经人确认不得升格。

## 目录结构

```
vault/
  events/<年>/<月>/<会话>/<事件>.jsonl   # 原始事件，一条一个文件，只追加
  memories/<id>.md                     # 长期记忆，YAML front matter + 正文
  .index/semantic/index.json + vectors.bin
  generated/views/                     # 人读视图，可重建
```

Event 与 Memory 都是纯文本，`grep`、`git diff`、手工编辑全部可行。没有数据库，没有二进制黑箱。

## 三层披露

模型不需要预先知道资料里有什么，只需要知道怎么一层层问。

| 层 | 工具 | 代价 | 得到什么 |
|---|---|---|---|
| L0 | `bootstrap` | ~240 B | 目录、访问说明、稳定个人参考 |
| L1 | `search` | `brief` 240 B / `context` 2400 B | 摘录或片段 + 引用 |
| L2 | `read` / `sources` | 按需分批 | 原文；记忆的原始证据 |

预算用 `budget_bytes`（保守的 UTF-8 字节数）而非 token：字节数是本地可精确计算的，token 依赖具体模型分词器。预算不够时返回 `pending_refs`，客户端继续 `read`，而不是静默截断。

## 检索

词法（BM25 类）与语义双通道各自排序，再用 RRF（k=60）融合。两条通道互不依赖：任一条失效时另一条仍能给出可用结果。

语义向量在进程内计算，不启子进程、不起服务：

- 本地：`fastembed` + ONNX，`bge-small-zh-v1.5`，512 维，CLS pooling，L2 归一化，查询侧加指令前缀。94.8 MB，单次查询约 1.8 ms。
- 云端：OpenAI 兼容的 `/v1/embeddings`，仅 https，密钥从环境变量读，按字节预算计费。

选 `bge-small-zh` 而非更大的模型：中文短文本够用；本地查询延迟是交互体验的硬指标；大模型体积会破坏"装完就能用"。需要更强表示时走云端通道，而不是把 600 MB 塞进安装包。

索引写入先落 `complete=false`，全部写完再翻成 `true`，中途崩溃不会留下看似完整的坏索引。索引里存 provider 与维度，换模型时自动判定失效并提示重建。

## 证据等级

每条 Memory 标注证据性质：

- `user_explicit` — 用户明确说过
- `observed` — 对话中可观察到，但未明说
- `assistant_suggestion` — AI 提出，尚未成为用户事实

这是 Memory 唯一允许的"分类"维度。不做主题树、不做认知类型学，因为主题推断会错，而错的主题会让整个检索偏掉。

## Dream：离线整理

记忆不是自动抽取的，是人工确认的。

```
dream export  →  外部 AI 整理  →  dream review  →  dream apply
```

1. `dream export` 选定来源，导出带完整规则与输出格式的任务文本。
2. 人把任务贴给任意 AI，拿回结果。
3. `dream review` 本地校验，逐条展示变更与影响范围。
4. `dream apply` 在人工批准后写入。

关键点：整理这一步可以发生在任何 AI 上，因为它是纯文本进、纯文本出；校验和写入始终在本地。受保护记忆的变更需要独立的二次批准。

## Agent 接入

`recallcard connect` 打印一段客户端配置 JSON，包含固定的库路径与授权范围。人自己粘贴进客户端，程序不修改任何外部配置。

MCP 走 stdio JSON-RPC 2.0，五个工具：`bootstrap`、`search`、`read`、`sources`、`knowledge_write`。工具描述总长约 250 字符，`instructions` 一句话 110 字符——这些内容每一轮都在上下文里，臃肿的说明是持续性成本。

`knowledge_write` 只写一次、只写 Event、从不写 Memory。不提供 `knowledge_delete`：有删除权的 agent 在一次提示注入里就能销毁证据，这与"助手建议不等于用户决定"直接冲突。删除始终是用户侧动作（`forget` / `restore`）。

## 网页 AI

网页版不能调用工具，所以接力靠人复制粘贴：把检索结果复制出来贴进网页，或把网页的回复粘贴回来存成 Event。这条路径不需要程序去驱动浏览器，因此不做浏览器扩展、不做协议注入。

## 桌面应用

Tauri 外壳，前端是原生 ES 模块（无构建步骤、无框架）。职责只有：选库、搜索与阅读、随身背景、记忆管理、Dream 审阅、生成连接配置。

桌面端不复制核心逻辑，全部经 `recallcard` 的命令与 `transport::connect_config` 复用同一套规则。

## 落地检查

```sh
cargo test --workspace          # 每个测试文件只守护一条不变量
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
recallcard doctor               # 库完整性
recallcard index status         # 索引与当前模型是否匹配
```
