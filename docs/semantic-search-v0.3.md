# 可选受监督语义检索

## 已接通的行为

Rust `Context::search` 可通过一个由可信启动配置固定的 Python worker，查询现有 Embedding 索引，并把向量候选与完整 BM25 文本结果进行 RRF 融合。没有进入 Memory、没有向量的原始 Events 仍参与文本检索。所有输出保留原本的 evidence、scope、ref 与时间字段；相似度和融合分数不提升来源可信度。

默认不配置 worker，仍是完全离线的文本检索。配置后也不自动建立索引、不扫描或上传 Vault、不重新执行 Dream。已有语料授权不等于查询授权；仅有 API key 不开启网络。索引的构建、增量复用与单独批准语料外发见 [Embedding worker](embedding.md)。

本轮使用合成 Vault、离线真实 Python 进程和内存 fake provider，未向真实供应商发出请求，未验证真实模型召回质量、费用或中转兼容性。

## 可信启动配置

配置是用户/可信宿主私有的本机 JSON 文件，不存进 Vault、Git 或模型消息。普通 search、MCP、native、IPC 请求都不能设置解释器、模块目录、缓存文件、endpoint、scope 或联网开关。未知请求字段直接拒绝。

下面是离线配置的字段示意；签名必须替换为现有索引 `space_signature` 中真实的 64 位小写 SHA-256，路径必须指向受用户控制的本机文件：

```json
{
  "python": "/usr/bin/python3",
  "python_path": "/用户本机/RecallCard/python",
  "index_path": "/用户私有目录/embedding-index.json",
  "expected_space_signature": "这里填写已经核对的空间签名",
  "query_cache_path": "/用户私有目录/query-vectors.json",
  "timeout_ms": 5000
}
```

- Python 3.10 及以上；`python` 可以是可信解释器路径或启动环境 PATH 中的命令，不经过 shell
- `python_path` 是包含 `recallcard_worker` 包的绝对目录；`index_path`、可选 `query_cache_path` 都须为绝对路径
- `expected_space_signature` 固定完整模型空间；缓存文件自己声明的签名不能改换可信查询空间
- `timeout_ms` 默认 5000，范围 10–120000；覆盖子进程启动后写入 stdin、flush 和等待完整 stdout 响应的整个异步交换
- 配置、索引与查询缓存均拒绝未知/重复字段、直接符号链接和非普通文件，读取上限为 16 MiB；父目录与解释器仍由本机维护者保护，此功能不是恶意程序沙箱

可信启动入口：

```sh
recallcard --vault /用户本机/记忆库 search '出行准备' \
  --scope personal --semantic-config /用户私有目录/semantic.json

recallcard --vault /用户本机/记忆库 mcp \
  --scope personal --semantic-config /用户私有目录/semantic.json

recallcard --vault /用户本机/记忆库 daemon \
  --scope personal --semantic-config /用户私有目录/semantic.json
```

MCP 的 `--semantic-config` 与 `--ipc-endpoint` 互斥。使用 IPC 时由 daemon 启动配置决定语义能力；客户端不能覆盖它。MCP 会话及 daemon 全生命周期复用同一个 backend，不为每个工具请求重启 Python。

Rust 宿主 API：

```rust
let semantic = recallcard::semantic::SemanticSearch::from_config_file(config_path)?;
let context = recallcard::context::Context::with_semantic(&vault, access, &semantic);
let response = context.search(search_args)?;
```

也可由可信代码构造严格类型 `SemanticConfig` 后调用 `SemanticSearch::new`。不要在请求处理时把不可信 JSON 反序列化为这份配置。

## 查询联网单独批准

只有用户已经明确批准具体 HTTPS 接收服务、query 文本类型、scope 与可能产生的费用之后，才在启动配置中加入 `cloud_query`。这个对象是宿主转交已有批准的方式，不是让程序自行获得同意：

```json
{
  "endpoint": "https://api.openai.com/v1/embeddings",
  "scopes": ["personal"],
  "key_env": "RECALLCARD_EMBEDDING_API_KEY",
  "max_network_calls": 20,
  "max_transmitted_bytes": 65536
}
```

以上对象作为配置的 `cloud_query` 字段，不能单独作为完整配置。query 外发会带上该空间已固定的 query instruction；不会附带 Event/Memory 正文，也不会启用 corpus 授权。endpoint 必须逐字匹配已固定空间；本次参与语义检索的全部 scope 都必须获准。敏感查询仍需要针对具体数据和具体接收者的明确批准，现有本地读取许可不替代外发批准。

key 只经指定环境变量传入子进程，不接收明文 key 参数或 JSON 字段。默认变量名为 `RECALLCARD_EMBEDDING_API_KEY`。Rust 清除子进程继承环境，仅保留运行解释器所需的少量平台变量；有 `cloud_query` 时才转交指定 key 变量。Python 使用 `-I -S`，不采用当前工作目录、用户 site 或 `PYTHONPATH` 的隐式导入配置。

默认每个 worker 最多 20 次请求、累计 65536 UTF-8 输入字节；可配置上限分别为 1000 次与 4 MiB。这个 Rust 集成关闭自动网络重试。worker 的同空间、精确查询缓存最多 32 项；相同查询可复用向量，但每次都重新过滤当前 scope/ref。预算在同一进程生命周期累计；重启 CLI/daemon 会重新计数，不是跨进程月度限额或金额保证。超时并不意味着供应商没有处理或计费。

## 完全离线的精确查询向量缓存

未提供 `cloud_query` 时，只有 `query_cache_path` 中命中精确查询的已有向量才可运行 `query_vector`。没有缓存或未命中时自动使用文本检索。此文件由已持有兼容查询向量的可信宿主准备；当前没有额外 CLI 自动生成器，也不会伪造一个查询向量。

```text
schema = recallcard.embedding-query-cache/1
space_signature = 与启动配置、索引完全相同的真实空间签名
queries[] = {query_hash, input_hash, vector}
```

- 最多 32 项，`query_hash` 是精确查询原文 UTF-8 的 SHA-256，不能重复
- `input_hash` 是实际输入的 SHA-256：无 instruction 时等于原文；有时为 `query_instruction + "\n" + query`
- 不 trim、不折叠空格、不做 Unicode 正规化；`vector` 必须有正确维度、有限数值和非零范数
- 文件不含原文查询或 key，但向量与哈希仍可能泄露信息，必须当作私有派生缓存
- 坏缓存或空间不一致直接降级，不因为已有云批准而偷偷忽略一个损坏的离线缓存重新发送

## Rust 独立核验与并发边界

启动者指定的索引必须完整。Rust 在调用前独立核验完整空间签名、维度、范数、重复 ref、同输入哈希向量一致性、scope、正本精确 revision ref、正文 hash、带 instruction 的 input hash，以及当前有效 Memory 集合的 generation。索引中每个 ref 必须和同 scope 导出语料集合精确对应，不能只用一个看似正确的 generation 字段替代内容检查。

索引 scope 必须是当前客户端可读 scope 的子集。若缓存包含客户端无权读取的其他 scope，整体降级，而不是读取无权访问的正本去验证它。客户端可读取更多 scope 时，只为索引覆盖的那部分 scope 加入语义召回；其他材料仍通过文本检索返回。

worker 等待期间不持有 Vault 锁，允许正常捕获、修订、遗忘和同步。返回后 Rust 重新取得只读锁、重新读取正本，并重新计算当前时间与语料 generation；若等待期间 Memory 修订、范围变化、抑制或有效期结束，丢弃旧向量候选，用当前文本结果重新排序。锁保持到响应构造结束，防止校验与取正文之间再被受支持的 writer 修改。直接绕过 RecallCard 锁手工改文件不在此并发保证内。

`target`、`session_ref` 与 `as_of` 先筛选再交给 worker，最终融合再对照当前 ref 集合。语义索引只包含建立索引时当前有效的 Memory，不承诺历史快照的完整语义覆盖；历史或无向量材料仍有文本路径。RRF 常数为 60，每份候选去重，分数相同按 ref 排序；最多 100 个向量候选参与融合。分页游标绑定正本、查询、scope 以及最终排名；向量失败或排序改变后旧游标明确失效。

## 监督、降级与输出预算

一个 backend 同时最多执行一次 worker 查询；并发请求立即得到文本结果和 `worker_busy`，不会积累无界队列。私有监督线程运行 Tokio 子进程 I/O，因此同步 Context 可从 CLI、MCP 或其他异步宿主安全调用，不嵌套当前线程的运行时。

- 请求序列化后最多 32 MiB；响应一行最多 64 KiB
- stdin 写入、stdout 读取和 flush 受同一超时约束；写满管道也不会无限等待
- stderr 丢弃，不把供应商异常、路径、正文或 key 拼进客户端错误
- 返回 id、成功/失败封装、字段类型、空间、generation、候选 ref、有限分数及重复字段必须通过校验
- Python 不存在、进程退出、超时、管道失败、非法/过量输出自动降级；损坏进程会终止，当前 backend 不自动重启，避免重置网络预算
- backend 释放、协议失败或超时后尝试终止并回收直接 worker；不声称终止恶意可执行文件另外创建的所有后代进程，因此解释器和包目录必须可信
- 可恢复的供应商错误保留同一进程，累计预算继续生效；向量失败不回滚已提交 Memory

搜索成功融合时，`coverage.semantic_search` 为 `available`，`semantic_coverage` 为 `indexed_current_memories`，`semantic_candidates` 为本次参与融合的向量候选数量，`ranking` 为 `rrf`。这些字段不代表所有 scope、Events 或历史材料均已有向量。

默认关闭时 `semantic_search` 为 `unavailable`。已配置但降级时附加固定的 `semantic_error` 错误码，例如：

- `query_not_cached` / `query_not_approved`：没有兼容的精确查询缓存，或缺少本次外发批准
- `cache_unavailable` / `invalid_cache` / `index_incomplete`：缺失、损坏或尚未完成的派生缓存
- `signature_mismatch` / `generation_mismatch` / `content_mismatch` / `scope_mismatch`：空间、正本版本、内容或范围不兼容
- `worker_unavailable` / `worker_timeout` / `worker_exited` / `worker_io` / `worker_busy`：进程或 I/O 不可用
- `malformed_response` / `output_limit`：协议错误或响应越界
- `key_unavailable` / `network_budget_exhausted` / `provider_auth`：凭据缺失、已用尽预算或供应商拒绝认证

所有 coverage、错误码、结果和分页状态计入 `budget_tokens` 所表达的保守 UTF-8 总字节预算；小预算可能返回零条结果与 `budget_exhausted`。本集成不将 worker 的任意 diagnostics 或错误正文转发给模型，也不据此估算未验证费用。

## 验证

```sh
cargo test -p recallcard --test semantic --test context
cargo clippy --workspace --all-targets -- -D warnings
python3 -m unittest discover -s python/tests -v
```

专用测试覆盖真实 Python 离线召回、BM25/Events 保留、无授权不启动查询、进程复用、stdin 背压超时、stdout 越界、错误脱敏、重复 JSON、坏/过期缓存、scope/session/as_of、等待中的遗忘/Memory 修订/有效期结束、并发忙碌降级、分页失效和完整响应预算。CLI 验证包含直接 search 与真实 MCP stdio 会话；MCP 通过真实 Python worker 和内存 fake provider 验证同查询缓存、进程累计预算及模型配置注入拒绝。

当前已在 Linux 执行上述进程测试；实现使用跨平台 Tokio process API，不据此声称已在 macOS/Windows 运行验证。IPC 平台验证与部署边界另见对应文档。
