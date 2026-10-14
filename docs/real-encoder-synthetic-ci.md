# 独立真实小编码器合成验收

此目标新增工程验收与质量观测，不修改核心、扩展、提供器默认设置或发布流程。实现代码不代表已下载、已运行或已达到语义质量门槛。必须查看指定源码 SHA 的 CI 和 `summary.json`。

## 触发与边界

- workflow：`.github/workflows/real-encoder-synthetic.yml`，仅 `workflow_dispatch` 或 `.github/real-encoder-ci-request.json` 这个显式 marker 变化触发；普通 push 不下载模型。
- marker 可写一个新的合成请求编号；此变更必须确实是一次明确请求，不要把 marker 纳入自动版本更新。脚本拒绝非 GitHub-hosted CI；不在用户机器或本地助手环境下载模型、启动回环服务。
- 不上传聊天、真实事件、Vault、索引、运行配置、凭据、日志或机器路径。产物白名单仅两个 JSON：合成指标 summary 与包名/版本/包摘要 dependency lock。
- 只需 `contents: read`。不改变任何仓库/账户/安全配置，没有在线推理和自动云端回退。GitHub Actions 的实际计费未读取，不能声称运行免费。

## 产品提供器与本次评估后端

BGE 只是此 CI 的固定、可离线复现评估后端，不是 RecallCard 产品绑定。现有云设置接受可配置 HTTPS OpenAI-compatible embedding 地址、模型、维度与独立 query/Memory 批准及月预算，可用于符合该协议的 Qwen 等提供器；这不代表已真实调用或验收某个云模型。当前新 Event/附件索引仍采用独立本机端点与批准，不继承旧云 query/Memory 批准。

## 固定模型和可复现来源

- FastEmbed 0.9.0，BAAI/bge-small-zh-v1.5，512 维，CLS pooling，L2 normalization。
- 官方 ONNX 仓库 Qdrant/bge-small-zh-v1.5 revision `46fbe35fd4374a00fee7de77dfddaeb6dd6a2c59`；ONNX 文件 94,781,076 bytes，SHA256 `1294ea4b6331115a353d81f96b85e8c8d7fdcc284453d5b2fab5b016230aad38`。
- `model-lock.json` 的 6 个小文件使用该固定 revision 官方 tree API 的原始长度和 Git blob SHA-1，算法精确为 SHA1(`blob ` + ASCII 十进制原始字节长度 + NUL + 原始文件字节)。它们不是 SHA256；CI 验证 blob identity 后另外计算全部文件 SHA256 写入 summary。不要把 Git blob OID 错写成文件 SHA256。
- 固定关键编码运行时版本在 `requirements.txt`。其他依赖由官方 PyPI 解析，本次实际所有安装包版本和分发文件 hash 写入 `dependency-lock.json`；再次复现应在相同 Linux x86_64/Python 版本下按该产物锁定完整依赖，不把未固定的传递依赖声称为跨时间 bit-for-bit 固定。
- workflow 用根 Cargo.lock 编译已有库。验收驱动从 `scripts/real_encoder/harness.rs` 临时复制为 Cargo example；这是显式 CI-only overlay，不提交核心代码。summary 记录仓库实际 SHA、模型锁、驱动、二进制和输入 fixture SHA256；已跟踪文件有变更就失败。

官方依据：[固定模型文件目录](https://huggingface.co/Qdrant/bge-small-zh-v1.5/tree/46fbe35fd4374a00fee7de77dfddaeb6dd6a2c59)、[FastEmbed 0.9.0 ONNX 编码实现](https://github.com/qdrant/fastembed/blob/v0.9.0/fastembed/text/onnx_embedding.py)、[tokenizer 配置实现](https://github.com/qdrant/fastembed/blob/v0.9.0/fastembed/common/preprocessor_utils.py)。pooling 依据实现，不能把通用 docstring 的 mean pooling 当成本模型实际行为。

## 真实链路和防静默截断

独立 Rust 驱动导入原始 `short-question/corpus.json` 的 12 个来源，读取原 `queries.json` 的 52 问。不另编 gold 冒充原集。先通过现有 search 记录 exact 和 question_intent 词法基线，再配置现有 LocalEvents，经现有 Python worker 和 SQLite 建索引，调用实际 `transport::invoke(..., "recall_search", ...)` 与 `recall_fetch`。每个返回来源均核验原文和快照；再次查询核验缓存复用。没有写 Memory。

服务只接受固定合成文档和固定查询 allowlist。生产 `EmbeddingSpace.prepare` 加查询指令“为这个句子生成表示以用于检索相关文章：”、换行、原查询；文档不加指令。适配器只调用 `model.embed`，不调用 query_embed，不二次加 prefix。

从同一固定文件独立加载 tokenizer，注册相同特殊符号，调用 `no_truncation()`、`no_padding()`，对全部实际输入含指令及特殊符号计数；超过 512 就在模型执行前拒绝。模型 tokenizer 同样关闭截断。CI 还以额外纯合成超长输入验证拒绝，不将超长输入交给模型。返回维度、数目、有限值、L2 范数和有序 index 都有校验。

下载在独立阶段结束；编码阶段使用 `specific_model_path`、`local_files_only=True`、离线环境变量，并禁止编码 Python 进程的外部 socket/DNS。此 audit 防线不是整个 runner 的网络沙箱；Rust/现有 worker 使用既有严格数字回环端点。没有 firewall、权限或网络设置变更。

## 指标含义

- 原集总数与 9 个 synonym_question 单独报告 source-hit@1/@3/@5，并与两个词法基线同集比较。
- 同时列出非预期来源候选数、无关/负例问题的非空候选数和全部固定查询的返回来源顺序。原生 facade 自己的固定结果/字节预算可能截断候选，原样记录，不偷偷扩大预算。
- 小集新增 `edge-cases.json` 是单独的 4 源/3 问否定与更正诊断；显式报告过时/冲突来源出现在前 k 的情况。它不能证明模型理解了否定、能自动挑出当前事实或答案正确。
- 此轮不事后选择一个好看的阈值；第一次 baseline 不设生产召回率验收门槛。工作流绿色表示固定模型和工程链通过，质量数据须另行解释。相似度候选检索没有拒答阈值，负例返回候选可能是预期局限。

## 预算和实测

单 job 最多 30 分钟；编译 15 分钟、依赖 5 分钟、下载 4 分钟、运行 5 分钟。模型下载总量不超过 100 MiB；下载脚本内部 180 秒，单请求 30 秒，下载前要求 2 GiB 余量。CPU 2 线程、batch ≤8；运行总时限 240 秒，外层 timeout 260 秒；最多 128 次编码请求/256 个输入；每个派生索引上限 16 MiB。

监控每 50ms 采样进程树 RSS，总和超过 1280 MiB 就停止验收；该值存在共享页重复，可能遗漏短峰。另报告编码进程实际 ru_maxrss；不是硬 cgroup 隔离。报告模型加载、首次推理、累计推理、索引构建、查询首个/中位/最大、运行总耗时、模型字节、环境安装大小和实际 SQLite 文件大小。冷启动专指本进程首次加载固定本地文件，不包含从网络下载或宿主页缓存保证。

## 不用模型的测试

`python3 -m unittest discover -s scripts/real_encoder -p 'test_*.py' -v`

这些单测不加载 FastEmbed、不下载模型、不监听端口。它们验证 hash、门禁、prefix、防截断计数、维度/范数、合成白名单、指标口径等纯代码合同；不能替代真实 CI 模型质量结果。

## 独立附件批准的固定留出配对（2026-10-14）

当前 CI 显式使用 `--suite attachment-holdout`；旧 `legacy` 驱动仍保留。只回放 `scripts/query_diagnostics/holdout.json` 的原创合成 24 源／64 问，其完整原始字节 SHA256 固定为 `c2c130e503bc8e753884865f48bed5fddc2dc95c1ddb0f6947b5a9e131883515`。该集此前已用于词法诊断，不能再称为新的盲测。问题文本、scope、角色、撤销标记和相关来源标注均不改；适配后完整 JSON 另算 canonical SHA256，结果再逐问核对原标注与范围。

同一临时 Vault 保留 12 个 Event 与 12 个附件，两个固定阶段依次执行：

1. `events_semantic_attachment_lexical`：只批准 Event 正文编码；附件仍可经既有本机词法入口读取。
2. `events_and_attachments_semantic`：明确增加本轮合成附件的独立批准，模型、文本和问题不变；复用 Event 和问题向量。

附件父事件正文为空，文件名、父 scope 和独立引用保留；附件正文保持 role:null，助手 Event 仍是 assistant。两个撤销来源在建索引前通过真实 Vault 抑制。6 个原始 scope 一一映射为 `project:synthetic-<原scope>`，每问严格使用原可见范围。此 fixture 提供的是 UTF-8 正文，即使文件名以 `.pdf` 结尾也只写成已知合成文本原件，绝不声称测试了 PDF 提取或 OCR。对话 title 保存在元数据中，当前生产路线不索引 title，这一差距不修订 qrels 来掩盖。

词法搜索可能返回附件父 Event 的文件名别名；它以 `parent:ATT-*` / `attachment_parent` 单独记录，按非标注候选计数，不转换为附件正文命中，也不删除它以改善精确率。两套词法基线使用 `lexical_intent`，不把 where/decision 等语义标签误当词法策略。

原集 42 个正例、22 个无答案问题，其中 20 个只需 Event、20 个只需附件、2 个任一类可答。真正运行语义路线的有 59 问；Q59/Q61 是空输入拒绝，Q60/Q62/Q63 有 5 个 scope，按现有最多 4 scope 门槛拒绝，不静默缩小范围。摘要同时给出全量、已执行、未执行、来源种类、问题类型及状态分组，不能把入口拒绝算成模型正确拒答。

同时报告 Top1/3/5 标注来源命中、无答案非空率、非预期候选、基于现有标注的候选 precision，以及逐问配对新增／丢失命中与新增／消除无答案候选。所有返回来源都经真实 scope/撤销/角色核验和 `recall_fetch` 原文字节／快照复核。指标仅指候选相关性；不生成或评分答案，不事后调阈值。

固定数据的理论新编码上限为 22 个未撤销文档＋59 个唯一 scope/query＝81 次输入／请求，单 scope 问题最多 13 条，第二阶段逐 scope 断言问题向量来自缓存。因此继续使用原 128 请求／256 输入、240 秒、1280 MiB 采样 RSS 和 100 MiB 模型下载限制。语料、文件、驱动、二进制、模型锁及实际运行时身份均写入脱敏摘要。实际运行状态和质量必须以对应提交 CI 产物为准；本段记录方案，不预填成绩。

### 首轮固定留出实测结果

源码 [`50805ba`](https://github.com/Micraow/RecallCard/commit/50805ba0d325a9026b922d8f187f40987d827170) 的 [CI 38571842293](https://github.com/Micraow/RecallCard/actions/runs/38571842293) 已完成：26 项纯 Python 合同、3 项 Rust 驱动合同及真实 BGE/生产检索/逐候选 fetch 链通过。以下只是原创合成固定集候选质量，不是用户资料验收，也没有形成默认开启或相关性拒答的依据。

| 路线 | 正例首位命中 | 正例 Top3 / Top5 | 无答案非空 | 非预期候选总数 | 全查询标注 precision@5 |
| --- | ---: | ---: | ---: | ---: | ---: |
| 生产词法（exact 与 question_intent 本集相同） | 22/42 | 23/42 | 8/22 | 34 | 25/59 = 42.37% |
| Event 语义＋附件词法 | 24/42 | 34/42 | 17/22 | 87 | 36/123 = 29.27% |
| Event＋独立批准附件语义 | 34/42 | 41/42 | 17/22 | 77 | 43/120 = 35.83% |

最后两行的 22 个无答案中，5 个没有运行语义路线（2 个空输入拒绝、3 个 scope_limit）。**实际执行的 17 个无答案问题全部返回候选**，所以 17/22 不能被解读为模型正确拒答了剩下的 5 个。标注 precision 是返回候选中的 qrel 匹配比例，不是生成答案准确率；两语义路线相较词法有更强的来源命中，也带来更多无答案候选。

分组不能被整体提升掩盖：

- 只需附件的 20 个正例：首位 4→18，Top3 12→20。
- 只需普通 Event 的 20 个正例：首位 **18→14**，Top3 **20→19**。
- Event 或附件任一可答的 2 个正例：两阶段首位均 2/2。
- 首位新增命中 14 问、丢失 4 问；Top3 新增 8 问、丢失 1 问。原固定集只用于诊断，不按这些题修改排序或事后选择阈值。

首位回退的普通 Event 中，有 3 个 Event 的余弦分数高于前面的附件，说明最终顺序还受到词法融合影响。唯一 Top3 丢失的 Q11 在第二阶段显示 `partial` 且仅返回 2 项，固定首屏预算参与了丢候选。尚需通用同分排序、来源竞争与预算的确定性负对照，不能据此把每种回退都归因于模型、或直接给某类来源提高权重。附件批准仍默认关闭；批准与角色归属不因本轮成绩改变。

实际接受编码 81 请求／81 输入，最大 99 tokens，243 次逐候选 fetch 核验通过。模型加载 0.531 秒，验收运行 20.507 秒，进程树采样峰值 254,693,376 bytes；模型文件 95,332,206 bytes，依赖环境 273,260,562 bytes。这些是单个 GitHub-hosted Linux CPU 进程测量，第二阶段复用缓存，不能当作两阶段独立冷启动对比。

[脱敏合成结果产物](https://github.com/Micraow/RecallCard/actions/runs/38571842293/artifacts/11897288839) 仅含 summary.json 与 dependency-lock.json。ZIP SHA256：`228ecab70c8bdaaec1eb4a1a1a2bac30b3ee9615c8852cc844b2dc8400d3431f`；适配输入 canonical SHA256：`bc17b82340fcad289915bbcd581f4d99edfdd6947b2366d111acd97b37da5a84`。已下载复核这两个身份及完整原始 holdout hash；本次只保存汇总；未把运行时 Vault、提取原文、配置、路径或原始进程日志提交到仓库。
