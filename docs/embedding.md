# 可选云 Embedding worker

## 状态与边界

`python/recallcard_worker` 使用 Python 3.10 及以上标准库，不安装 SDK、不下载模型、不扫描 Vault，也不修改 Event、Memory 或 Dream receipt。Rust 的捕获、文本检索、Bootstrap、手动 Dream 与离线重建无需 Python、API key 或网络。

当前提供一个 OpenAI-compatible HTTPS Embeddings 适配器、增量派生缓存、余弦检索和 RRF 融合。Rust 提供 `embedding-export` 显式导出语料，并通过可信启动配置可选接入 `search`/MCP/daemon：受监督子进程查询已有兼容索引，重新对照正本后融合文本与向量候选。默认搜索仍完全离线且不启用 worker；查询文本外发需要与 corpus 分开的批准。配置、降级和进程边界见 [可选受监督语义检索](semantic-search-v0.3.md)。

本轮验证全部使用虚构材料、内存 fake transport 和离线子进程。没有向真实供应商发送任何数据，没有调用付费模型，没有验证某家中转服务的实际兼容性、召回质量或账单。

## 授权与本机配置

联网默认关闭。只有可信宿主或用户启动进程时同时提供以下参数，才可发送数据：

- `--allow-network`：明确开启发送
- `--approve-endpoint`：唯一接收者的完整 HTTPS 接口 URL，逐字匹配空间配置
- `--approve-scope`：允许发送的范围，可重复；不支持通配符
- `--approve-data corpus`：允许发送导出语料的文本
- `--approve-data query`：允许发送查询文本，与 corpus 分开授权

启动参数表达已经取得的授权，不会替用户作决定。必须先让用户了解接收服务、语料范围、文本类型及可能产生的费用。敏感材料需要针对具体数据和具体接收者的明确批准；现有本地读取许可不等于云发送许可。导出语料中的 `scope`、模型生成的请求字段或网页指令都不能授予联网权力。

API key 只从 `RECALLCARD_EMBEDDING_API_KEY` 环境变量读取，或通过 `--key-env` 指定另一个变量名。没有接收明文 key 的 CLI/JSON 字段。不要把 key 写入命令行、共享脚本、文档、语料、Vault 或 Git。配置 JSON、查询、导出语料、向量文件和 `.pending` 文件应放在用户控制的本机私有目录；不要把供应商配置保存进 Vault。代码不会自行保存配置。

网络请求验证 TLS，禁止 HTTP、重定向、URL 内嵌凭据、查询参数和 fragment，并关闭隐式环境代理。若供应商迁移地址，重新核对并批准目的地，不跟随返回的跳转。错误不会回显供应商错误正文、输入文本、key 或原始异常。

## 向量空间身份

空间配置必须包含全部字段，不接受未知字段：

```json
{
  "provider": "openai",
  "endpoint": "https://api.openai.com/v1/embeddings",
  "model": "text-embedding-3-small",
  "dimensions": 1536,
  "revision": "unknown",
  "document_instruction": "",
  "query_instruction": "",
  "normalization": "l2",
  "preprocessing_version": "utf8-v1"
}
```

`revision: "unknown"` 诚实表示供应商没有暴露可固定的 revision，不代表代码能保证模型别名永不变化。需要可复现性时选择供应商实际支持的固定 revision。这个字段是本地空间标识，不会被冒充为供应商支持的 API 参数。

`space_signature` 为上述完整对象的规范 JSON SHA-256：UTF-8，键名排序，无额外空白，保留非 ASCII 字符。endpoint、provider、model、dimensions、revision、两种 instruction、normalization 或预处理版本任何变化都建立不同空间。同维度不意味着空间相同。

`utf8-v1` 保留文本原样，不 trim、不折叠空格、不做 Unicode 正规化。有 instruction 时实际编码输入为 `instruction + "\n" + text`；无 instruction 时等于原文。只支持 `normalization: "l2"` 或 `"none"`；供应商内部 pooling 不可配置或观测，不作额外承诺。`input_hash` 是实际编码字符串的 UTF-8 SHA-256，与空间签名共同决定能否复用。

## 导出与文件模式

从 Rust 先导出用户选择的 scope，并审阅内容：

```sh
recallcard --vault /用户的本机路径/记忆库 embedding-export --scope project:demo > /用户的私有目录/corpus.json
```

请以 `recallcard --help` 和 `embedding-export --help` 为准确认当前构建的参数位置。导出本身不调用模型。corpus 合同：

```json
{
  "schema": "recallcard.embedding-corpus/1",
  "generation": "由 Rust 生成的语料版本哈希",
  "scope": ["project:demo"],
  "documents": [
    {
      "ref": "memory:mem_demo@1",
      "content_hash": "原始 text 的 UTF-8 SHA-256，小写 64 位十六进制",
      "text": "待发送的已选择文本",
      "scope": "project:demo"
    }
  ]
}
```

上面的哈希文字是字段说明，不是可运行 fixture。worker 会拒绝伪造或不匹配的哈希。Rust 当前导出允许范围内 active/tentative Memory；不自动向量化整段事件日志。worker 只接收这个显式文件或 stdin，不支持 Vault 路径扫描。

在已经批准具体发送动作、外部环境已安全提供 key 后，可以运行：

```sh
PYTHONPATH=python python3 -m recallcard_worker \
  --allow-network \
  --approve-endpoint https://api.openai.com/v1/embeddings \
  --approve-scope project:demo \
  --approve-data corpus \
  --max-network-calls 20 \
  --max-transmitted-bytes 131072 \
  index --space /用户的私有目录/space.json \
        --corpus /用户的私有目录/corpus.json \
        --cache /用户的私有目录/embedding-index.json
```

这是会联网并可能收费的使用示例，不属于项目测试命令。`--corpus -` 支持 stdin。`--cache` 的父目录必须已存在。写入使用同目录临时文件、fsync 和替换；新文件在 POSIX 上为 `0600`，拒绝直接覆盖符号链接缓存。

每个成功批次写入 `<cache>.pending`。全部成功后才替换活动索引并删除 pending。供应商失败时旧索引保持不变；相同 generation/空间的 pending 可在下次继续，已成功文本不再请求。空间改变时必须重新编码；不要用新 query encoder 查询旧空间。不同 generation 的 pending 当前不自动合并，重启可能重做尚未发布的批次。断电发生在供应商完成与本机检查点持久化之间，也可能造成一次重复计费；没有宣称跨服务 exactly-once。

## 索引合同

结果中的 `index` 结构：

```text
schema = recallcard.embedding-index/1
space = 完整空间对象
space_signature = 空间签名
generation = 对应语料 generation
scope = 显式范围列表
complete = true 或 false
documents[] = {ref, content_hash, input_hash, scope, vector}
```

不包含原始文本或 key。向量、ref、哈希和 instruction 仍可能透露私人信息，不能当作公开数据发布。索引是本机派生结果，删除不会丢失事实，但没有保留原空间向量时重建需要模型和费用。

索引载入时检查签名、scope、ref 唯一性、哈希格式、维度、有限数字、非零范数、声明的 L2 归一化，以及相同输入哈希的向量一致性。损坏缓存显式报错，不把它当作新事实。缓存没有密码学签名；宿主应只加载受本机访问控制保护的文件，不能信任第三方提供的任意向量文件。

## Rust 可选调用：有界 JSONL stdio

启动：

```sh
PYTHONPATH=python python3 -m recallcard_worker --stdio
```

每行一个请求和一个响应，无额外 stdout 日志。授权仍只通过进程启动参数传入；请求不得自行开启网络、携带 key 或指定文件路径。每个可信宿主独占进程，不直接把 stdin 暴露给网页或模型。

操作合同：

```text
{id, op:"space_signature", space}
  → result: {space, space_signature}

{id, op:"validate_index", index}
  → result: {space_signature, documents, complete}

{id, op:"index", space, corpus, previous_index?, batch_size?}
  → result: {index, diagnostics}

{id, op:"query", index, query, allowed_scopes, allowed_refs,
 limit?, lexical_refs?, expected_generation?}
  → result: {results:[{ref,score}], ranking, space_signature, generation,
             query_cache_hit, diagnostics}

{id, op:"query_vector", index, query_vector, space_signature,
 allowed_scopes, allowed_refs, limit?, lexical_refs?, expected_generation?}
  → result: {results:[{ref,score}], ranking, space_signature, generation}
```

成功封装为 `{id,ok:true,result}`；失败为 `{id,ok:false,error:{code,message}}`，可能附带安全诊断。`index` 的供应商/预算错误还可能返回 `partial_index`：不含原文的 `complete:false` 检查点，宿主可私下保存并在下次作为 `previous_index` 传入。stdio 不在每批主动发送进度帧；若进程被直接杀死，未回传的成功批次可能需重算。需要每批持久化时使用文件模式或 Python 的 `checkpoint` 回调。

`allowed_scopes` 与 `allowed_refs` 都由 Rust 当前权限和抑制/遗忘状态计算，不能接受模型自行宣称的授权。空列表返回空结果。两项必须同时允许一条记录，才参与排名。推荐每次传 `expected_generation`；generation 不符时拒绝查询。宿主仍负责重新对照当前事实的精确 revision ref、content_hash、时间过滤、保护状态与预算，再向最终客户端输出结果。Python 不读取 Event/Memory，因此不能独立核实这些业务状态。

宿主应设进程超时、限制 stdout 字节、校验 schema 和错误，并在 Python 缺失、失败、pending、空间或 generation 不一致时继续文本检索。向量失败不能回滚已正确提交的 Memory。stdout 返回的分数也不提升 source authority。

`query_vector` 完全离线，但必须带生成此 query vector 的真实 `space_signature`，不接受仅维度一致的猜测。`query` 需要单独批准 query 数据外发；精确文本和同空间编码结果最多缓存 32 项，只存哈希和向量，不缓存最终答案。每次请求重新过滤当前 scope/ref。

传 `lexical_refs` 时执行 RRF，默认 `k=60`，每份列表去重并先过滤权限，再用 `1/(k+rank)` 累计，分数相同按 ref 排序。当前融合仅覆盖本索引中可见的向量记录；Rust 可另外保留无向量的文本结果，不要因此丢掉未 Dream 的 Events。未传该字段时返回余弦分数；相似度不是事实可信度。

直接 Python API 位于 `recallcard_worker.embedding`：`EmbeddingSpace.from_dict`、`EmbeddingClient`、`NetworkApproval`、`index_corpus(..., checkpoint=...)`、`search_index(...)` 与 `reciprocal_rank_fusion(...)`。这些函数不读取额外语料文件。测试通过 `EmbeddingClient(transport=...)` 注入内存响应，生产路径默认使用验证 TLS 的标准库 transport。

## 限额、重试与用量

默认或硬上限：

- 一行 JSON / 语料文件 / 索引文件最多 32 MiB，重复 JSON 字段与 NaN/Infinity 拒绝
- 最多 10,000 文档；维度 1–8192；文档数乘维度最多 500,000
- 原始文本与 instruction 拼接后的每项输入最多 8192 UTF-8 字节，空白输入拒绝
- 一份语料的实际输入总量最多 4 MiB
- 每批最多 64 项、131,072 UTF-8 字节，超限提前分批；不偷偷截断文本
- 一进程默认最多 100 次网络请求、累计发送文本最多 1 MiB，重试也计入预算
- 每批默认最多重试 2 次，最多配置 5 次；单请求默认超时 30 秒，最高 120 秒
- 只对连接错误、408、429、500、502、503、504 有界重试；Retry-After 最多等待 5 秒；401/403、重定向、非法参数和坏向量不重试
- 查询返回最多 100 项，默认 10 项

UTF-8 字节预算是保守的输入限制，不是通用 tokenizer，也不是金额上限。重试可能被供应商计费，tokenizer/分词和单价必须按实际服务核对。请求次数与字节预算在本进程累计，重启会重新计算；不跨进程实施月度费用限制。

诊断包含请求次数、重试、累计发送文本字节、成功响应的 `prompt_tokens`、延迟、复用文档数及新增唯一输入数。不保存原始 Prompt。没有供应商 usage 时 token 与费用标为 `null`；provider prompt-cache 指标始终保持 `null`，不能把本机向量复用误报为供应商缓存命中。

可用 `--price-per-million-tokens` 、`--pricing-date YYYY-MM-DD` 和 `--pricing-currency USD` 提供用户核实的价格快照。`estimated_cost` 只按成功响应实际报告的 token 估算，货币通过 `pricing_currency` 明确返回；不是实时官方价格，也不包含未知失败请求收费。没有快照则为 `null`。项目不硬编码月费或“节省百分比”。

## 验证

```sh
python3 -m unittest discover -s python/tests -v

# 已有本机编译产物时，验证真实 Rust 导出与 Python 协议；仍不联网
python3 python/tests/check_rust_embedding_contract.py target/debug/recallcard
```

纯 Python 测试共 48 项；跨语言检查另建临时合成 Vault，设置临时本机状态目录，核对 SHA-256、fake 构建、零请求复用、离线 stdio 查询、撤权与过期 generation。

覆盖空间身份、哈希、变更与删除、同文跨 ref 复用、混合 scope 预检、无授权不联网、NaN/维度/零向量、乱序和重复供应商索引、429 与连接故障重试、预算、失败检查点继续、权限撤销、查询缓存隔离、RRF、坏 JSON 与超长行恢复、子进程 stdio、缓存原子写入以及敏感错误脱敏。所有 key、文本、endpoint、向量均为合成测试数据；测试没有调用真实 provider。

官方参考（2026-10-06 查阅）：

- [OpenAI Embeddings API](https://developers.openai.com/api/reference/resources/embeddings/methods/create)：`POST /embeddings`、字符串批次输入、float 输出、响应 index/usage；`dimensions` 支持 text-embedding-3 及后续模型。本适配器始终发送 dimensions，因此并不宣称兼容不支持该参数的旧模型或中转接口
- [OpenAI 向量指南](https://developers.openai.com/api/docs/guides/embeddings)：模型选择、维度与检索背景；本项目没有把指南中的模型性能当作自身测量结果
