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

## 界面

`recallcard ui` 在回环地址上起一个只服务本机的 HTTP 端点，前端是构建好的静态页面（`web/`，React + Vite）。
没有常驻服务，也没有原生外壳：进程随终端退出而消失，页面随之失效。

- 鉴权：启动时生成一次性令牌，首次访问带 `?token=` 换取 `HttpOnly` Cookie；随后清掉地址栏里的令牌。只接受 `127.0.0.1`/`localhost` 的 Host，避免 DNS 重绑定。
- 复用：所有读写都走 `DesktopSession`，与命令行共用同一套权限、抑制与审查规则；界面层不含业务判断。
- 渲染：正文按 Markdown 渲染并关闭原始 HTML（恶意文本只能成为纯文本），链接强制新窗口。
- 整理：按时间范围自动挑出尚未被任何记忆引用的原始记录，交给设置里的模型生成候选，再由人逐条确认；应用内调用只是把"复制粘贴"这一步本地化，人工确认没有减少。
- 设置：模型与编码服务的地址、模型名、密钥存在本机状态目录（权限 0600），界面只回报"是否已配置"，密钥不进 Vault、不进仓库、不写日志。

## 落地检查

```sh
cargo test --workspace          # 每个测试文件只守护一条不变量
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
recallcard doctor               # 库完整性
recallcard index status         # 索引与当前模型是否匹配
```
