# 可选 Dream API：显式发送，结果仍须审查

`python/recallcard_dream` 是独立的标准库执行器：读取现有 Rust 导出的完整 `DreamJob`，向用户选定的 OpenAI-compatible HTTPS Chat Completions 接口发送一次请求，保存一个未信任的 `DreamResult`。

它不读取 Vault、不捕获对话、不发布 Memory、不接受批准摘要、不调用 `dream apply`，也不操作网页的 Send。没有 Python 或 API 时，Rust 的捕获、文本检索、Bootstrap 与[手动 Dream](dream.md)仍可独立使用。

## 授权边界

默认不联网。执行 API 必须同时满足：

1. `--allow-network` 明确启用本次调用
2. `--approve-endpoint` 与 `--endpoint` 的完整 HTTPS 地址逐字相等
3. `--approve-scope` 包含 Job 唯一的 `allowed_scope`
4. `--approve-dream-data` 明确批准发送 Job 中全部选中 Event 和旧 Memory 快照

不会从 Job、来源正文、环境变量、模型输出或 prompt 中读取这些授权。配置 endpoint/model 或存在 API key 都不等于允许上传。联网前检查整个 Job 的所有来源和整个旧记忆 read-set：每份快照的 scope、引用、版本与 Job 必须一致；混入最后一条越界资料也会整体拒绝，不能先发送合法部分。

此 CLI 是可信用户/本机宿主入口，不应直接交给不可信模型或网页调用。输出只能用于审查，任何模型提议都不授予发布权限。

### 导出文件与当前资料库之间的边界

执行器接受的是用户检查过的 Rust 导出文件，不是实时 Vault 视图。`content_hash`、`input_hash` 按 Rust 生成的不透明摘要保留；Python 检查摘要格式与 `job_id = dream_<input_hash>`，不声称独立重算了 Rust 的序列化摘要，也无法确认文件未被编辑。

Python 不知道导出后发生的撤权、suppression、来源编辑或删除。因此，发送前应重新从当前 Vault 导出并检查所选内容；旧文件不会因 Vault 后续变化自动擦除。不要把 `--validate-only` 当作允许上传、来源真实性、摘要完整性或当前权限证明。

结果回到 Rust 后，仍必须检查原始 Job、所有摘要、最新 read-set、当前 suppression、证据来源、保护规则及批准摘要。Rust 是最终校验和发布权威；Python 的前置检查不能证明自然语言结论正确。

## 最小命令行流程

要求 Python 3.10 或更新版本，无第三方依赖。在项目根目录运行：

```sh
recallcard --vault ./vault dream export \
  --source "$EVENT_ID" --memory "$MEMORY_ID" --scope project:recallcard \
  > dream-job.json

PYTHONPATH=python python3 -m recallcard_dream \
  --job dream-job.json --validate-only
```

`--memory` 是可选项。不要照抄示意编号；选择实际希望发给该供应商的 Event/Memory。检查完整 Job，移除不应外发的来源时应重新导出，而不是手改摘要。

密钥只从进程环境变量 `RECALLCARD_DREAM_API_KEY` 读取。请由本机秘密管理工具或可信启动器提供；不把真实 key 写进命令参数、文件、Vault、Git、诊断日志或 shell 历史。执行器不保存 key，不支持 `--api-key` 或凭据配置文件。

确认选定供应商、模型、价格和这份 Job 的外发范围后，才运行下面的模板。`ENDPOINT` 必须是实际完整接口地址，不会自动追加 `/v1/chat/completions`；示例没有默认供应商和默认模型：

```sh
PYTHONPATH=python python3 -m recallcard_dream \
  --job dream-job.json --output dream-result.json \
  --endpoint "$ENDPOINT" --model "$MODEL" \
  --allow-network --approve-endpoint "$ENDPOINT" \
  --approve-scope project:recallcard --approve-dream-data \
  --max-output-tokens 4096 --max-request-bytes 262144 --timeout 30
```

正常返回退出码 0，stdout 是不含 prompt 的状态/用量 JSON。`dream-result.json` 只包含原始协议结果，不混入 usage、批准字段或执行器元数据。失败返回退出码 2，stderr 是已去除私密内容的中文错误 JSON。

接着人工检查差异：

```sh
recallcard --vault ./vault dream review --file dream-result.json
```

只有人工确认该份 review 后，才使用 review 返回的完整 `result_hash` 批准：

```sh
recallcard --vault ./vault dream apply \
  --file dream-result.json --approve '<已审查的完整 result_hash>'
```

受保护记录仍需要 Rust 的额外明确批准。不要把模型输出自动管道到 `apply`；`conflict` 是有效提议，但 Rust 不会把它当成可发布结果。

## 请求布局与兼容范围

当前只支持已落地的 Job 策略：

- `schema = recallcard.dream-job/1`
- `operation = extract`
- `prompt_version = manual-extract-v1`
- `projection_version = bounded-full-v1`
- `output_schema = recallcard.dream-result/1`

旧 Memory 非空时，在同一个有界请求内做提取与整合，不增加自动检索/模型循环，不发送整个 Vault。

Chat Completions 请求固定使用：

- 两条固定中文 system 消息：提取/整合与信任边界指引，随后是完整固定输出 schema
- 最后一条 user 消息：本次完整 Job JSON，所有来源内容、job_id 和动态元数据只在这里
- `response_format: {"type":"json_object"}`、`n: 1`、`stream: false`
- 模块参数 `max_output_tokens` / CLI `--max-output-tokens` 映射到供应商字段 `max_completion_tokens`

该版本不发送 `tools`、`cache_control`、推测的缓存 TTL 或未经验证的特殊供应商参数。兼容接口必须支持上述字段和双 system 消息；不支持时明确失败，不改用其他模型、地址、字段或隐式 repair 请求。不是所有标为 OpenAI-compatible 的服务都支持这组字段。

输出必须是一个完整 JSON 对象，`finish_reason = stop`，恰好一个 assistant 结果。拒绝工具调用、refusal、截断响应、代码围栏、多个 JSON、重复字段、NaN/Infinity、未知 Result 字段、空/过多提议，以及错误的 Job/schema/input_hash。提议不能越出当前 Job 的 scope、来源或旧记忆 read-set；修改版本必须一致，同一目标不能重复修改。

## 稳定前缀与实际用量

固定指引和 schema 编码为 `STABLE_PREFIX_BYTES`，模板摘要为 `PROMPT_TEMPLATE_HASH`。测试直接比较两份不同 Job 的稳定消息字节和实际请求动态 user 内容前的字节，要求完全一致。不会把 Job ID、当前时间、来源正文或每轮重新措辞的摘要插进这段前缀。

这只验证前缀稳定，不证明实际 token 前缀、供应商路由、最低长度、TTL 或缓存命中。没有真实供应商测试，不声明缓存收益或费用节省；`cache_control` 为 `unknown`，费用与价格快照字段为 `null`。

成功响应的完整数值型 `usage` 会保留，包括嵌套 token 明细；未知数值字段不丢弃。映射诊断字段：

| 诊断字段 | 供应商依据 |
|---|---|
| `input_tokens` | `usage.prompt_tokens` |
| `output_tokens` | `usage.completion_tokens` |
| `cache_read_tokens` | `usage.prompt_tokens_details.cached_tokens`，否则 `usage.cache_read_input_tokens` |
| `cache_write_tokens` | `usage.cache_creation_input_tokens` |

未提供的数据为 `null`，不把缺失视为零，也不以 `prompt_tokens - completion_tokens` 等方式推测缓存。`usage` 只接受有界的非负整数/null/嵌套对象；字符串或无界异常结构会导致响应被拒绝，避免把伪装为诊断的数据打印出来。输入/输出计数与缓存细节仅是供应商报告，不是独立核验账单。

## 有界执行和故障行为

| 限额 | 默认值 | 可配置范围或硬上限 |
|---|---|---|
| Job | 1 MiB | 可调低，最多 64 Event / 32 旧 Memory |
| API 请求 | 4 MiB | 可调低，含固定 prompt、schema 和 JSON 转义开销 |
| 模型生成 | 4096 token | 1–131072；实际供应商上限可能更低 |
| Result 文件 | 1 MiB | 可调低，最多 32 提议，单条 content 最多 64 KiB |
| 完整供应商响应 | 2 MiB | 可调低，含外层包装与 usage |
| 请求超时 | 30 秒 | 大于 0 且最多 120 秒 |
| CLI 请求次数 | 1 | 固定单次，不自动重试 |
| 模块实例请求次数 | 1 | 显式可设 1–100，总请求字节预算也累计 |

这些是请求/字节/token 预算，不是货币硬上限。供应商可能计算额外 reasoning、失败请求或其他项目；本项目没有硬编码模型价格。正式使用仍须核对供应商价格并配置其账户预算。供应商是否遵守 `max_completion_tokens` 由供应商负责，不能把客户端参数当作实际账单保证。

网络使用标准库 `http.client.HTTPSConnection`，开启正常证书与 hostname 验证，不使用环境代理、不跟随 HTTP 重定向。错误状态不读取供应商错误正文，避免复制 prompt 或 key。读取按有界块进行，并根据剩余时间更新 socket 超时；系统 DNS 解析由操作系统完成，Python 标准库不能保证强制中止卡住的系统解析器，因此这里不是独立进程级的绝对墙钟截止保证。

所有错误，包括 429/5xx、断网、超时、schema 错误和修复需求，都立即停止。没有重试、退避、schema repair、换模型或追加请求。失败也消耗当前实例已经保留的请求预算；重启 CLI 或明确调用新的实例属于新请求，可能再次计费。请先检查原因和现有结果，不要将“可重试”理解成免费。

输出在同目录私有临时文件中写入、同步，再原子提交。默认排他创建，不覆盖已存在结果；覆盖需要 `--overwrite`，但仍拒绝覆盖输入 Job 或符号链接路径。临时文件使用私有权限，失败清理临时文件。请使用本机私有目录；不将此路径检查当作针对恶意并发修改共享目录的完整文件系统沙箱。

API 成功后若本机写盘失败，调用可能已经计费；命令会报错，不自动重跑。诊断不默认额外落盘或保存完整 prompt。输出状态中的调用/字节计数为当前实例累计，token usage 为这次成功响应。

## 模块 API

```python
from recallcard_dream import DreamClient, NetworkApproval, load_json

# job_bytes 应由可信调用方有界读取；来源是经人工检查的 Rust 导出文件。
job = load_json(job_bytes)
approval = NetworkApproval(
    enabled=True,
    endpoint=approved_endpoint,
    scopes=("project:recallcard",),
    dream_data=True,
)
client = DreamClient(
    approved_endpoint,
    chosen_model,
    approval,
    max_output_tokens=4096,
    max_request_bytes=262144,
    max_total_request_bytes=262144,
    max_requests=1,
)
completed = client.execute(job)
# completed["result"] 仍必须由 Rust review，不能直接发布。
# completed["diagnostics"] 是本次供应商用量和本机计数，不含 prompt 或 key。
```

实例面向可信单线程宿主，每次 `execute` 一条有界请求。单独调用 `build_request` 或 `validate_job` 不联网。测试可注入 `transport(endpoint, payload_bytes, key, timeout, response_limit)`，返回 `TransportResponse(status, body_bytes)`；注入不取消显式授权、预算和结果校验。

## 离线验证

所有测试使用合成资料与只在进程中存在的假 key。fake transport 不访问真实供应商；CLI 子进程通过启动补丁禁止 socket 连接，验证默认不联网及授权缺项整体拒绝。

```sh
python3 -m unittest discover -s python/tests -p test_dream_api.py -v
```

如已有编译好的 Rust CLI，可启用完整合同用例：新建临时合成 Vault → Rust 捕获/旧记忆/Job 导出 → fake API → Rust review。用例不执行 apply，并验证 Memory 数量和 receipt 没有改变：

```sh
RECALLCARD_TEST_BINARY=target/debug/recallcard \
  python3 -m unittest discover -s python/tests -p test_dream_api.py -v
```

此版本验证了 Linux 上的离线结构、权限、用量与文件流程；未调用真实模型，未实测真实缓存、费用、记忆准确率、供应商 TLS/HTTP 行为或 Windows/macOS 文件提交语义。执行 API 前应自行核对供应商的当前兼容能力、数据政策和价格。
