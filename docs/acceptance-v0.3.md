# RecallCard v0.3 设计执行与验收差距

审查日期：2026-10-07 UTC。本文面向项目维护者，依据 [整体设计 v0.2](RecallCard-DESIGN-v0.2.md) 检查当前实现、测试和使用文档，说明哪些能力可以依赖、哪些还需补齐。本文是验收记录，不替代或改写原设计；[v0.1](RecallCard-DESIGN.md) 仅作为历史。

**当前结论：本机文件正本、只读召回、手动整理、人工发送浏览器桥、Python worker、本机 daemon/IPC、受监督语义检索和中文 Tauri 桌面版已有实现与对应回归。整份设计仍未全部完成：真实 Web→Agent 使用、宿主生命周期、大库持久增量索引，以及真实供应商效果仍有缺口。** 本文第 1–6 节保留最初审查快照，后续实现与验收见末尾补充，当前功能入口以 [实施进度](progress.md) 为准。

## 1 审查范围与证据

本次完整阅读 v0.2，并核对 Rust 的正本、Context、MCP、Native、Dream、导入与同步路径，浏览器会话/输入框代码，以及 Python 向量与 Dream 执行器、对应测试和中文文档。审查只新增本文件，没有修改设计、源码或原有进度说明，没有运行重型构建，也没有调用真实模型或浏览器账号。

### 版本与验证快照

- 审查起点已发布基线：`23cfd96b8a7e0c1c4505d26e06c2f03227ac9ef6`
- 本次审查的同步、Python worker、Native 启动器及浏览器恢复阶段已于审查期间发布为 `d3105ef280b2d6ade56bacb41a8a8f3b37ea41cc`；该提交必须以自己的 CI 结果验收，不能直接继承基线结论
- 审查后补入实体/有效期/View、CLI 引用与持续遗忘修复；本机完整复测 161 项 Rust 通过，远端结果仍按本段提交单独确认
- v0.2 设计文件 SHA-256：`0348b3569a9992948d8f39170579094efc74de0d514cf83ceb7588353c48bcc2`

以下运行结果来自本轮已完成的验证记录；本审查没有以测试函数数量替代执行结果，也没有沿用旧报告的测试总数：

| 检查 | 本轮确认结果 | 适用范围 |
|---|---|---|
| Linux Rust | 161 项通过 | 当前工作树的本机测试；不是 Windows/macOS 当前工作树通过的证明 |
| Python 完整回归 | 105 项通过，无跳过 | 已设置 `RECALLCARD_TEST_BINARY`，包含真实 Rust 导出与 fake API 合同；未访问真实供应商 |
| Rust→Embedding 合同 | 独立脚本通过 | 显式传入已有 Rust 二进制，离线验证导出、哈希、复用、撤权与 generation |
| 扩展 Node 回归 | 74 项通过 | 协议、后台、生命周期和模拟 DOM；不是登录网页实测 |
| 已发布基线 CI | Linux、macOS、Windows、Node 全绿 | [run 37558934749](https://github.com/Micraow/RecallCard/actions/runs/37558934749)，对应上述 `23cfd96`；于 2026-10-07 01:51 UTC 确认 |
| 恢复阶段远端 CI | Linux、macOS、Windows、Node、Python 全绿 | [run 37559424832](https://github.com/Micraow/RecallCard/actions/runs/37559424832)，绑定 `d3105ef`；2026-10-07 01:58 UTC 确认 |

平台 CI、真实宿主、真实网页与真实供应商是不同证据。Windows CI 通过不等于 Windows Chrome 已成功安装并完成 Native Messaging；fake HTTP 成功不等于供应商已兼容，也不证明账单或召回质量。

### 状态含义

- **已实现**：对应程序路径存在，且有匹配的自动回归；只限明确列出的机制
- **部分**：已有基础路径，但完整场景、集成、语义判断或容量验收仍缺失
- **未实现**：未找到对应可执行路径，不能用目录、接口草案或文档代替功能
- **未实机验证**：未在真实浏览器、选定宿主或供应商上完成场景；可与前三种状态同时存在

## 2 T01 至 T12 逐条验收

### T01 Web 刚作决定且未 Dream 时由 Agent 查询

**状态：部分；真实 Web→Agent 闭环未实机验证。**

已有：手动 JSONL、ChatGPT 官方导出当前分支、Claude Code JSONL 导入；捕获后的 Event 无需 Dream 即可参与文本检索。MCP 提供同一套 Context 查询，返回来源引用。

- 源码：[import.rs](../crates/recallcard/src/import.rs) `import_text`、[context.rs](../crates/recallcard/src/context.rs) `documents/search`、[transport.rs](../crates/recallcard/src/transport.rs) `serve_mcp_io`
- 回归：[context.rs 测试](../crates/recallcard/tests/context.rs) `undreamed_event_is_immediately_searchable`、`mcp_has_exactly_four_readonly_tools_and_no_mutation_route`；[import.rs 测试](../crates/recallcard/tests/import.rs) `manual_and_agent_fixtures_import_idempotently`、`chatgpt_import_chooses_current_branch_and_ignores_hidden_reasoning`
- 缺口：目前是显式文件导入与协议 fixture；未证明真实浏览器中的新决定经人工保存后，在真实 Agent 会话中正确被模型使用。扩展不自动抓取网页回复，不能把“刚在 Web 说过”当成“已经捕获”

完成场景验收还需记录一个合成决定从真实 Web、明确导入到真实宿主检索并引用原话的完整过程。

### T02 没有 Dream 预算且积累一周历史

**状态：部分；离线原文搜索已实现，一周规模与大库性能未验证。**

- 源码：[context.rs](../crates/recallcard/src/context.rs) `documents/search/rank` 不依赖 Python、模型或向量；`rebuild` 可离线重建文本快照
- 回归：[context.rs 测试](../crates/recallcard/tests/context.rs) `undreamed_event_is_immediately_searchable`、`chinese_tokenizer_keeps_bigrams_and_ascii_paths`、`disposable_index_and_empty_directories_are_rebuilt_offline`
- 缺口：没有一周累积数据集、长日志、持续导入和目标机器延迟/内存测量。搜索每次读取正本并重新分词，不是持久增量索引

可说明“没有 Dream 仍能查询已导入原文”，不能据此承诺长历史查询延迟或任意容量。

### T03 助手提议但用户没有回答

**状态：已实现证据与状态约束；真实模型语义表现未实机验证。**

- 源码：[vault.rs](../crates/recallcard/src/vault.rs) `validate_evidence/new_memory`；[dream.rs](../crates/recallcard/src/dream.rs) `review_current`；[Python Dream 指引](../python/recallcard_dream/client.py)
- 回归：[vault_safety.rs](../crates/recallcard/tests/vault_safety.rs) `evidence_role_matrix_rejects_facts_without_appropriate_sources`、`assistant_suggestions_remain_explicitly_labeled`；[dream.rs 测试](../crates/recallcard/tests/dream.rs) `assistant_suggestions_cannot_become_approved_user_facts`
- 实际保证：纯助手来源不能冒充 `user_explicit`；助手建议为 tentative，不能直接替代既有事实，模型不能自行取得发布权限
- 边界：校验器不能证明自然语言内容真的被某条用户原话支持。模型混引了一条无关用户消息时，仍需人工核对完整 diff 与证据

### T04 Memory 注入两家 Web 后再次捕获

**状态：部分；有标记的反回声机制已实现，两家 Web 往返未实机验证。**

- 源码：[capture.rs](../crates/recallcard/src/capture.rs) 注入标记识别；[model.rs](../crates/recallcard/src/model.rs) origin/parts；[context.rs](../crates/recallcard/src/context.rs) 默认召回过滤；[protocol.js](../extension/protocol.js) 胶囊来源
- 回归：[context.rs 测试](../crates/recallcard/tests/context.rs) `recaptured_capsule_is_not_new_evidence_or_search_noise`；[vault_safety.rs](../crates/recallcard/tests/vault_safety.rs) `injected_context_cannot_be_evidence_even_with_a_native_user_source`、`block_level_injection_cannot_hide_behind_a_native_user_envelope`、`mixed_injected_and_user_parts_are_not_whole_event_evidence`；[dream.rs 测试](../crates/recallcard/tests/dream.rs) `injected_context_is_not_exported_as_new_evidence`
- 缺口：浏览器增加了 Qwen/Z.ai 实验性手动 composer 适配和合成回归，但两家 Web 实际往返仍未验收。无标记的复制、同义改写或错误标成 user_input 的手工材料，不能保证被自动识别为旧记忆。含混合注入的整条 Event 保守拒绝，尚无精确 part 级证据引用与批准

### T05 笔记本 Arch 与服务器 Debian 同时成立

**状态：部分；已增加同 scope、两台机器与独立实体检索回归，模型语义判断未实测。**

- 源码：[model.rs](../crates/recallcard/src/model.rs) 存有 scope/entities；[dream.rs](../crates/recallcard/src/dream.rs) 校验 scope、来源和目标版本；[Python Dream 指引](../python/recallcard_dream/client.py) 要求区分机器、项目与时期
- 回归：[context.rs](../crates/recallcard/tests/context.rs) `same_scope_arch_laptop_and_debian_server_are_independent_entities` 验证两条事实同时保留并按实体召回；另有标签、具名 View 与权限回归
- 缺口：`review_current` 未对目标与提议的 entity/subject 一致性施加专门约束；没有真实模型判断测试

两条事实并存检索的合成 fixture 已通过；错误的跨实体 supersede 仍需人工复核，当前不会仅凭文本相似度自动替代。

### T06 上个月换了系统但没有具体日期

**状态：部分；未知时间的数据表示与不自动补日期已实现。**

- 源码：[model.rs](../crates/recallcard/src/model.rs) 可空时间和 `time_note`；[import.rs](../crates/recallcard/src/import.rs) 缺失时间保持 null；[Python Dream 指引](../python/recallcard_dream/client.py) 明确禁止编造日期
- 回归：[vault_safety.rs](../crates/recallcard/tests/vault_safety.rs) `unknown_occurrence_time_is_preserved_instead_of_replaced_with_capture_time`；[dream.rs 测试](../crates/recallcard/tests/dream.rs) `supersede_preserves_original_protection_and_does_not_invent_end_time`；[Python 测试](../python/tests/test_dream_api.py) `test_invalid_time_and_score`、`test_nanosecond_time_interval_is_not_truncated`
- 缺口：这些验证 null 保真及格式/区间，不证明真实模型会把“上个月”正确保留为模糊时间。不存在通用自然语言时态推理验收，也不能把计划在日期过后自动视为完成

### T07 重复导入相同 source result job

**状态：部分；本机正本幂等已实现，跨设备无冲突与费用不重复不作保证。**

- 源码：[vault.rs](../crates/recallcard/src/vault.rs) `capture`；[dream.rs](../crates/recallcard/src/dream.rs) Job 摘要、receipt 和 `already_applied`；[embedding.py](../python/recallcard_worker/embedding.py) 空间签名与输入哈希复用
- 回归：[vault_safety.rs](../crates/recallcard/tests/vault_safety.rs) `capture_is_idempotent_and_preserves_original_bytes`、`source_revision_links_to_the_original_and_reimport_is_idempotent`；[dream.rs 测试](../crates/recallcard/tests/dream.rs) `manual_export_review_apply_is_offline_and_idempotent`、`one_completed_job_cannot_publish_another_result`；[Python 向量测试](../python/tests/test_embedding.py) `test_build_and_exact_reuse_are_offline`、`test_partial_checkpoint_reuses_successful_batches`
- 已知边界：两端离线捕获同一个 Event 时，相同 ID/路径可能有不同 `captured_at`，当前保留为 Git 冲突，详见 [同步说明](sync.md) 与 `the_same_event_captured_offline_on_two_devices_preserves_both_timestamps_as_conflict`
- 费用边界：独立 Dream API 执行器不会查询 Rust receipt 来阻止再次付费调用；进程重启会重置请求预算。供应商成功与本机检查点落盘之间仍可能中断，不能宣称跨服务 exactly-once 或重复费用恒为零

### T08 Dream 导回前目标记录已被修改

**状态：已实现本机过期结果拒绝与人工重整合。**

- 源码：[dream.rs](../crates/recallcard/src/dream.rs) `review_current/dream_apply/validate_transaction` 同时检查来源摘要、整个 memory_read_set 的 revision 与内容摘要
- 回归：[dream.rs 测试](../crates/recallcard/tests/dream.rs) `stale_read_set_and_unlisted_targets_are_rejected`、`same_revision_but_edited_memory_content_invalidates_read_set_hash`、`changed_source_metadata_and_tampered_job_hash_are_rejected`、`approval_hash_is_bound_to_the_exact_result`
- 实际行为：旧结果明确失败，不盲写；用户需重新导出/整理/审查。没有自动模型重整合循环，拒绝后的“重新整合”目前是显式人工流程

### T09 多轮上下文或宿主 compact 后继续使用

**状态：部分；Agent 生命周期 Hook 代码和本机CLI合同已实现，真实宿主未实机验证。**

- 已有：[transport.rs](../crates/recallcard/src/transport.rs) 在 MCP initialize 中给出显式调用 bootstrap 的说明；[Claude Code 示例配置](../integrations/claude-code/mcp.example.json) 注册四个工具
- Web 已有：[broker.js](../extension/broker.js)、[content.js](../extension/content.js) 的会话绑定、重置和重新附上说明；[生命周期测试](../extension/tests/lifecycle.test.js) 验证 SPA/重载/过期绑定；[后台测试](../extension/tests/broker.test.js) 验证快照与去重
- 新增：[agent_hook.rs](../crates/recallcard/src/agent_hook.rs) 和 [17项回归](../crates/recallcard/tests/agent_hook.rs) 按官方 SessionStart 合同输出固定 scope 的 Bootstrap，覆盖 startup/resume/compact/clear；当前尚未接入真实宿主，仍没有显式跨端 continuation 命令
- 未实测：真实 Claude Code 在这些生命周期节点的上下文结果。MCP instructions 是指引，不是“模型必定调用”的保证；网页压缩不可观察，扩展只提供人工恢复入口

### T10 同快照 Bootstrap 同字节且不混入动态值

**状态：已实现确定性渲染与稳定前缀回归；真实供应商缓存未实机验证。**

- 源码：[context.rs](../crates/recallcard/src/context.rs) `bootstrap` 稳定排序、正文摘要和独立 coverage；[broker.js](../extension/broker.js) 稳定资料指纹；[Dream API](../python/recallcard_dream/client.py) 固定指引/schema 与动态 Job 分离
- 回归：[context.rs 测试](../crates/recallcard/tests/context.rs) `pinned_bootstrap_is_stable_and_does_not_promote_unpinned_text`；[后台测试](../extension/tests/broker.test.js) “Bootstrap 仅动态 coverage 改变时保留原稳定快照”；[Python 测试](../python/tests/test_dream_api.py) `test_two_jobs_have_byte_identical_fixed_prefix`
- 边界：没有实际 token 前缀、缓存命中率或节省金额测量；Dream API 将 cache_control 标为 unknown。Agent 侧缺少会话生命周期管理，不把稳定 renderer 等同于整套会话缓存策略已落地

### T11 撤权或遗忘后不能再次召回与提炼

**状态：部分；当前记录集合的过滤与旧 Job 拒绝已实现。**

- 源码：[policy.rs](../crates/recallcard/src/policy.rs)、[context.rs](../crates/recallcard/src/context.rs)、[dream.rs](../crates/recallcard/src/dream.rs)；浏览器插入前重新核验；Python 查询重新过滤本轮 scope/ref
- 回归：[context.rs 测试](../crates/recallcard/tests/context.rs) `suppression_prevents_recall_until_explicit_restore`、`suppression_of_memory_also_suppresses_its_sources`、`authorization_filters_search_read_sources_and_bootstrap`；[dream.rs 测试](../crates/recallcard/tests/dream.rs) `suppression_blocks_export_and_results_prepared_before_forgetting`；[Python 向量测试](../python/tests/test_embedding.py) `test_cosine_has_no_scope_or_revoked_ref_leakage`、`test_query_cache_reuses_encoding_but_rechecks_current_permissions`
- 缺口与边界：suppression 现保存记录 ID、source_refs 与受 scope 约束的来源身份摘要，继承到现有和未来修订。新增 [suppression.rs](../crates/recallcard/tests/suppression.rs) 5 项回归覆盖未来修订、独立消息/范围、各读取路径、旧规则兼容、缺失原文和损坏拒绝。单独执行 `memory state retracted` 只改状态，不生成 suppression，不能与 `forget` 的持续抑制语义混同。长期 MCP 进程权限固定于启动参数，没有热更新撤权服务
- 外发边界：独立 Python 执行器无法感知导出后撤权，用户仍可能主动发送旧文件；必须先重新导出和审核。Rust 拒绝最终发布不能撤回已上传内容、Git 历史或旧 clone，详见 [Dream API](dream-api.md)

### T12 Git 冲突 崩溃 Embedding 故障

**状态：部分；本机故障保护已实现，统一语义检索降级与完整跨平台实机链路尚未完成。**

- 源码：[sync.rs](../crates/recallcard/src/sync.rs) 先提交再整合、冲突停止、普通 push；[dream.rs](../crates/recallcard/src/dream.rs) 持久事务与恢复；[vault.rs](../crates/recallcard/src/vault.rs) 原子替换/锁/读取屏障；[embedding.py](../python/recallcard_worker/embedding.py) 有界重试与检查点
- 回归：[sync.rs 测试](../crates/recallcard/tests/sync.rs) `two_devices_commit_before_integrating_and_keep_both_histories`、`real_text_conflicts_remain_for_manual_resolution_and_repeat_sync_refuses`、`rejected_push_preserves_local_commit_without_force_or_reset`；[dream.rs 测试](../crates/recallcard/tests/dream.rs) `interrupted_multifile_publish_is_fail_closed_and_recovers_exactly_once`、`recovery_handles_before_first_write_and_after_receipt_without_duplicates`、`recovery_refuses_external_edits_without_overwriting_them`；[Python 向量测试](../python/tests/test_embedding.py) `test_partial_checkpoint_reuses_successful_batches`
- 证据边界：Dream 崩溃用例构造发布切点的磁盘状态，不是硬件断电测试；Git 用例连接本机临时 bare 仓库，不是真实私人远端。Windows 文件占用回归与基线 CI 已通过，但新增阶段仍需自己的 CI
- 集成缺口：Rust 文本搜索目前从未调用 worker，因此“不受 worker 故障影响”成立；尚未验证“同一次 MCP 语义搜索在 worker 超时/崩溃后自动降级为文本”的完整链路

## 3 重点架构差距

### daemon 与独立本机 IPC

**未实现。** [main.rs](../crates/recallcard/src/main.rs) 没有 daemon 子命令，未发现 Unix socket、Windows named pipe 或独立核心 IPC 协议。

实际路径是：MCP stdio 进程直接持有 Vault/Context；Native Host 进程也直接调用 Context；CLI 直接写 Vault。它们共享 Rust 库、操作系统文件锁和事务规则，已有单次写入互斥，不等于存在一个长期运行的中心 writer。

后续若实现 daemon，应保留现有权限边界，并补客户端身份、socket/pipe 权限、消息与超时边界、进程重启恢复、跨平台路径及多客户端并发测试。独立 IPC 不是当前小型离线纵切的前置阻碍，但不能在架构图上视作已完成。

### 宿主生命周期与跨端交接

**Agent 适配代码已实现、宿主未实测；Web 人工恢复部分实现。** Claude Code MCP 示例、日志导入与 SessionStart Hook 现均有代码/fixture；后者只运行本项目 CLI，不启动外部 Agent。没有显式选择 source session/branch 的 continuation 包生成入口，也没有 captured-through/dreamed-through 的统一覆盖游标管理。

浏览器的会话 nonce、稳定快照、导航失效和手动重置已落地，但不能证明外部模型在压缩后仍知道如何读 RecallCard，更不能读取宿主隐藏会话状态。

### Embedding 到 Rust 与 MCP 的接入

**worker 已实现，默认检索集成未实现。** [context.rs](../crates/recallcard/src/context.rs) `embedding_corpus` 提供显式 Memory 语料导出；[embedding.py](../python/recallcard_worker/embedding.py) 实现空间签名、增量复用、余弦查询、RRF、授权与预算；独立 stdio/文件模式有离线回归。

但 Rust `search` 和 `bootstrap` 仍明确返回 `semantic_search: unavailable`。没有 Rust supervisor 自动启动 worker、执行超时/字节限制、核对返回 generation/当前 revision/抑制状态并融合结果；没有由 MCP 查询触发的 query encoder 与故障降级。当前 RRF 是 worker 内的能力，不能据此宣传 MCP 已有混合召回。

集成时还应保留无向量的未 Dream Event 文本结果；不能因为只有 Memory 被向量化就让原始历史消失。scope/ref 及云外发许可必须由可信本机配置给出，不能接受模型请求授予权限。

### 大库文本索引与容量

**持久增量检索未实现，规模性能未验证。**

- `Context::documents` 每次从正本重建文档，`rank` 每次重新分词并计算 BM25
- `Vault::event(id)` 通过 `events()` 扫描事件库；逐条 Memory 来源校验还可能反复扫描 Event
- `rebuild` 写 `.index/text.json`，但搜索不读取这个快照；删除后能重建不等于查询已经使用持久索引
- 尚无 FTS/倒排增量更新、generation 原子切换、批量来源定位或大型数据延迟/内存基线
- Python worker 限制最多 10,000 文档，同时限制“文档数 × 维度 ≤ 500,000”；例如 1536 维时实际最多 325 条向量。当前是有界小规模 worker，不是已验收的大型向量库

先建立合成规模基线，再决定增量文本索引和事件定位表；索引仍应可丢弃，不能升级为新的知识正本。

### API Dream 与供应商缓存

**独立执行器已实现；真实供应商与统一调度未实机验证。** 固定前缀、scope/数据/接收端显式批准、请求预算、usage 缺失为 null、未信任结果输出都有测试。它不自动发布 Memory，也不自动重试/repair。

缺少与核心统一的任务运行与结果复用调度、已完成 Job 的费用避免机制、可强制终止卡住进程的 supervisor，以及实际供应商 cache capability 验证。当前请求/字节预算不是货币硬上限；不承诺模型效果、缓存命中或账单节省。

## 4 其余设计差距与已选择的边界

| 设计范围 | 当前实现与剩余边界 |
|---|---|
| Event 与 Memory 文件 | JSONL 单事件封段、Markdown/YAML Memory 已实现；Event 路径没有设备前缀，双端重复捕获冲突已有测试与说明 |
| 原文与 blob | 能表示文件路径/摘要并返回 `content_not_retained`；objects 有目录和同步摘要校验，尚无完整大对象捕获、按 ref 分块读取链路 |
| 来源粒度 | 支持内容块 origin；Memory 主要引用整个 Event，混合注入整条保守拒绝，未实现 part 级证据批准 |
| 搜索返回 | 中文字/二元组与 ASCII 路径匹配、scope/session/as_of、批量 read 和字节预算已实现；entities/labels 参与排序，具名 View 与当前有效期过滤已实现；相邻上下文展开与显式实体结构化参数仍未实现 |
| 捕获范围 | 主动选择文件导入，覆盖报告明确 partial/unsupported；不自动采集网页回复，也不扫描宿主私有历史目录 |
| 生成视图 | 确定性输出已有回归；编辑视图不会自动变成正本，尚无完整 edit/pin/import-edit 转换工具 |
| 四项权限 | 捕获由显式输入、Dream 由显式选择、读取由启动 scope、云发送由单独批准参数区分；尚无统一的持久会话策略与热更新权限管理 |
| Git 同步 | 本地双端临时远端、冲突、安全白名单、普通推送与失败保留已验证；真实 Git 认证/SSH/跨设备使用及新阶段三平台 CI 仍需分别核对 |
| 浏览器安装 | Native 可执行副本、固定配置、来源绑定、二进制分帧有 Linux 子进程回归；真实 Chrome 注册与登录网页、Windows ACL/注册表、macOS 安装仍需实机验收 |
| Web 使用成本 | 原有草稿、撤销、请求去重、人工发送边界有自动回归；尚无实际“回答一个个人问题需额外点击几次 Send”的观察记录 |

## 5 下一项最有价值的可实现工作

**建议优先完成一个真实 Claude Code 宿主的 Bootstrap 加载与恢复纵切，关闭 T01/T09 的关键缺口。** 现有正本、搜索和 MCP 已可支撑它，收益比立即铺设独立 daemon 或增加更多模型适配器更直接。

建议限定为一个宿主、一个合成 Vault、一个 scope：

1. 先核对所选宿主版本的官方启动/恢复/压缩扩展点，不凭 hook 名称猜测是否能注入下一轮上下文
2. 提供明确、可重复的 Bootstrap 加载与人工恢复入口；固定会话快照身份，避免重复注入，同时让纠正与撤权优先
3. 用合成 Web 原话显式导入，让真实宿主通过四个只读工具回答并展示来源；无 Dream、无 Embedding 仍可完成
4. 验证首次启动、恢复、compact、工具失败/中断、重复加载、遗忘后恢复及多项目不串线；记录实际宿主版本、命令、输入与可观察结果
5. 若宿主扩展点无法保证自动加载，保留明确人工恢复入口并准确记录限制；通过验收再扩大适配范围

真实宿主安装、选定账号与实际 QA 应在其授权条件满足后单独执行，本审查没有执行这些动作。

随后建议依次处理：Rust 受监督 worker 与 MCP 混合检索/自动文本降级；T05 跨实体更新审核和 T06 的模型语义验收；有规模测量支撑的增量文本索引；最后按多客户端需求完善 daemon/独立 IPC。实现次序可以按真实使用反馈调整，不应把未做的设计点重新描述为“原本就不需要”。

## 6 下一次验收记录应补什么

- 跟进 `d3105ef` 的新 CI run，终态确认后分别记录平台结论；后续改动另外绑定 commit/run，不累加不同提交的测试数
- 为真实 Web/宿主测试记录版本、来源、已捕获范围及模型是否实际使用证据，不只保留进程启动成功截图
- 为语义 worker 接入验证“缺 Python、超时、坏响应、旧 generation、撤权”时同一 MCP 查询仍返回安全文本结果
- 为容量验收记录合成数据量、目标机器、导入/查询/重建延迟与内存，不以小 fixture 推断大库表现
- 如计划调用真实供应商，先取得明确的数据、接收端与费用授权；实际 usage、缓存和质量结果单独记录，不用离线测试代替

## 6 本机 IPC / 语义后端验证分支补充

2026-10-07 新增本机 daemon、MCP/Native IPC 桥、固定配置的受监督 Python 语义检索。上文架构缺口记录的是原审查基线；该段的新实现详见 [IPC v0.3](ipc-v0.3.md) 和 [语义检索 v0.3](semantic-search-v0.3.md)。只读 daemon 不等于集中式写入服务；大型持久增量文本索引、真实宿主/浏览器安装、真实供应商效果仍未完成。

- 语义检索新增24项自动回归，包括真正的Python worker、CLI/MCP进程、多轮预算、阻塞stdin/超时、当前记录变化与撤权后的重新验证；无真实供应商调用
- IPC有5项内存双工协议回归及15项真实端点/权限/进程回归，另有3项CLI daemon→MCP/Native/复制启动器桥接回归
- 本机端点测试被云电脑底层socket策略以EPERM阻挡，授权后的相同命令仍失败。这不是已通过的IPC验证，也没有通过跳过用例伪造成功
- Windows交叉Clippy通过仅证明可编译；所有真实三平台用例必须在本段自己的Actions中成功后才能推进main
- 该段先发布临时验证分支，现有push工作流只运行检查和测试，没有部署或真实API调用

2026-10-07 后续验收结果：IPC/语义提交 `9061a9d2d9b35f0979b3544f69481aa264d3041c` 已通过 [验证分支 CI](https://github.com/Micraow/RecallCard/actions/runs/37563960312) 与 [main CI](https://github.com/Micraow/RecallCard/actions/runs/37564390185)。三平台Rust、Python105和Node74均成功；Linux完整208项Rust（包含真实IPC/桥接）通过，Windows的专属条件测试数量不同，以其日志为准。现已推进main；之前仅交叉编译/EPERM的证据局限没有被冒充成本机运行成功。

## 7 中文桌面版及当前交付补充

程序 `9d76621` 完成免配置的「创建资料库 → 粘贴首条记录 → 立即查找」，并支持文件导入、来源阅读、整理结果审阅与保存。真实 Tauri 程序在标准 Ubuntu 22.04 环境完成 9 步原生操作验收，另有 19 项界面/状态用例通过；[同次运行](https://github.com/Micraow/RecallCard/actions/runs/37579114793) 产出 AppImage/deb。`6581e52` 的[轻量分发](https://github.com/Micraow/RecallCard/actions/runs/37581139328) 只复用该成品，校验后按完整安装方式归类，不改变二进制。

桌面端已可单独使用；浏览器扩展、云模型和向量配置不再是开始保存/检索资料的前提。当前真实安装链路的限制仍保留：dot 云端 Chromium 明确阻止加载扩展，未绕过该策略；用户 Arch/KDE Wayland 环境没有实测。没有真实供应商调用或真实外部 Agent 执行，不能据桌面测试宣称这些场景通过。
