# 手动 Dream：先看证据，再批准发布

Dream 将已有 Event 整理成可追溯的 Memory。当前实现完全离线：不启动模型、不调用云 API，也不会操作网页的 Send。将任务交给哪个模型、是否跨服务分享其中的内容，以及最终发送，都由用户决定。

## 流程与边界

1. 明确选择一个 scope、1–64 条 Event，以及最多 32 条作为整合基线的旧 Memory
2. 导出有界 Job，检查其中的完整来源及旧记忆；不需要分享的内容应从选择中删除，然后重新导出，获得新的 input_hash
3. 用户自行把 Job 交给所选执行者，取得符合下述协议的完整 JSON
4. 运行 review，检查 before/after 差异、来源、有效时间和保护状态
5. 使用 review 返回的 result_hash 明确批准同一份结果；修改受保护记忆还要单独批准 protected 修改
6. 程序重新核对基线，持久发布 Memory 与成功收据，再刷新可重建的人类视图

review 不修改知识正本。自然语言结论的真实性不能由 JSON、哈希或模型评分证明：有争议的决定、主张的生效时间，以及助手是否真的获得用户批准，都需要对照原文。

首版保存完整且有界的选中来源，不把超长来源悄悄截成“完整证据”。超过任务上限就拒绝，并要求拆分任务。这个本机上限不保证适合任意供应商的上下文窗口，仍应按所选执行者能力拆分。`model_score` 只是模型提供的诊断数值，不会因为评分高而绕过来源或保护规则。

## 公共接口

```rust
let job = vault.dream_export(&source_ids, &memory_ids, "project:recallcard")?;
let review = vault.dream_review(&result)?;
let receipt = vault.dream_apply(&result, &review.result_hash, false)?;
let recovery = vault.dream_recover()?;
```

`source_ids` 接受原始 `evt_...` 或 `event:evt_...` 引用。旧记忆接受 `mem_...` 或 `memory:mem_...@revision`；版本不符会拒绝。调用者应使用实际产生的编号，不能照抄设计文档中的示意编号。

这些方法属于可信本机的人工管理界面，不进入模型侧四个只读工具。模型提出的操作名称、scope、nonce、保护字段或自然语言授权，都不扩大权限。

## 命令行示例

先把 `EVENT_ID` 设为实际已捕获的事件编号，并只选择希望交给本次 Dream 的内容：

```sh
recallcard --vault ./vault dream export --source "$EVENT_ID" --scope personal > dream-job.json
```

多个来源重复写 `--source`；整合已有记忆时重复写 `--memory "$MEMORY_ID"`。打开导出的 Job 预览后，由人决定是否发给所选模型。把模型给出的完整 Result 保存为 `dream-result.json`，然后审查：

```sh
recallcard --vault ./vault dream review --file dream-result.json
```

确认差异正确后，使用这次 review 的完整摘要批准，不要把未审查的输出自动管道到 apply：

```sh
recallcard --vault ./vault dream apply --file dream-result.json --approve '<已审查的完整 result_hash>'
```

修改 protected 记录时，只有确认该修改后才增加 `--approve-protected`。文件改变、来源改变或旧记忆已更新，都需要重新审查；旧批准摘要不能授权不同结果。

如提示有待恢复事务：

```sh
recallcard --vault ./vault dream recover
```

`--file -` 可从标准输入读取完整 JSON。命令成功时向 stdout 输出 JSON；失败以非零退出码和 stderr 中的错误 JSON 报告。不要只凭创建了输出文件判断命令成功。

## Job 协议

Job 使用 `schema: recallcard.dream-job/1`，包括：

- `job_id` 与 `input_hash`：按固定策略版本、scope、来源快照与旧记忆快照确定性计算
- `source_refs`：每项有 event 引用、完整内容摘要和 Event 快照
- `memory_read_set`：每项有版本化 memory 引用、完整内容摘要和 Memory 快照
- `prompt_version`、`projection_version`、`output_schema`

相同输入重复导出得到相同 Job。Job 完整快照保存在当前 Vault 对应的本机状态目录中，不自动同步到 Git，也不会自动上传。导出的 JSON 本身包含所选内容，应像来源资料一样妥善保管。

本机状态目录可以通过 `RECALLCARD_STATE_DIR` 显式设置。更换机器或删除该状态目录后，原 Job 不会凭空出现；应在拥有所需正本的机器重新导出并整理。持久成功收据位于 Vault 的 `control/dream-receipts/`，不属于可随意删除的索引。

## Result 协议

下面使用占位符展示结构，实际编号和摘要必须来自导出的 Job：

```json
{
  "schema": "recallcard.dream-result/1",
  "job_id": "<Job 的完整 job_id>",
  "input_hash": "<Job 的完整 input_hash>",
  "proposals": [
    {
      "operation": "add",
      "scope": "project:recallcard",
      "content": "该项目使用 Rust 主程序与可选 Python worker。",
      "source_refs": ["event:<真实 evt 编号>"],
      "evidence": "user_explicit",
      "model_score": 0.8,
      "observed_at": null,
      "valid_from": null,
      "valid_to": null,
      "time_note": "来源没有给出独立的生效日期。",
      "labels": ["架构"],
      "entities": ["RecallCard"]
    }
  ]
}
```

没有完整 schema 的自由回复、截断 JSON、空 `proposals` 或未知字段都会被拒绝。确实无需变更时，应显式返回 `noop`。

### 操作

| operation | 含义 | 约束 |
|---|---|---|
| `add` | 新增独立 Memory | 无 target_ref/expected_revision；来源必须在 Job 中 |
| `update` | 修改已有 Memory | target_ref、expected_revision 与整个 read-set 必须一致 |
| `supersede` | 新增替代 Memory，并将旧记录标为 superseded | 保留旧记录，记录 supersedes；不会自动制造 valid_to |
| `noop` | 明确处理完成但无需修改 | 不接受修改目标；仍保存成功收据 |
| `conflict` | 报告不能安全决定的冲突 | review 展示诊断；整个 Result 暂不发布 |

修改与替代提议还要提供：

```json
{
  "target_ref": "memory:<真实 mem 编号>@1",
  "expected_revision": 1
}
```

一个 Result 不能重复修改同一个目标，也不能引用 read-set 之外的旧记忆。任意一条提议校验失败，整份结果都不发布。

### 证据与保护

- `user_explicit` 必须引用明确的用户原话；助手声称“用户已同意”不构成用户批准
- `observed` 必须有受支持的用户或工具观察来源
- `assistant_suggestion` 只生成 tentative，不能直接替代既有事实
- 省略 evidence 时保守采用 assistant_suggestion
- recallcard_context、Dream 对话以及无法安全拆分的混合注入，不作为新的独立证据
- scope 必须与 Job 及来源完全一致；不能将项目私有来源提炼进 personal
- 被抑制的 Event、Memory 或相关证据不能导出或通过旧 Job 重新复活
- Result 不接受 authority/protected 字段。普通 Dream 新增为 authority=dream、protected=false
- 受保护目标的更新/替代需要明确额外批准；替代后的新记录继承保护，不通过换 ID 绕过保护
- 未知发生时间、observed_at、valid_from/to 保留 null，不用导入时间或当前时间伪造历史

## 一致性、幂等与恢复

每次发布先持有单写者锁，并重新校验来源摘要、整个旧记忆 read-set 的版本与内容摘要。即使用户直接编辑 Markdown 而没有递增 revision，内容变化也会使旧结果失效。

发布顺序：

1. 在本机状态目录写入并同步完整 `dream-transaction.json`
2. 原子替换各 Memory 文件；完整日志保留每个目标的旧、新快照
3. 最后持久保存成功 receipt
4. 删除并同步事务日志，重新开放普通读取/写入
5. 刷新派生 Views

如果在任意中间步骤退出，普通读取/写入会明确报告待恢复事务，不将磁盘上的一部分结果冒充完成状态。`dream_recover` 在持锁下按同一已批准快照继续发布：当前文件只能与旧快照或新快照之一相同。

如遇外部编辑、来源缺失、suppression 变化、日志损坏或收据冲突，恢复停止，保留日志和当前文件供人工核对，不盲目覆盖。恢复不会生成新结果，也不批准新的修改。

相同 Job/Result 已成功发布时返回 `already_applied: true`，不会新增 Memory、重新递增 revision 或重新执行模型。相同 Job 的不同 Result 被拒绝。若 Memory/receipt 已提交但生成视图失败，错误会明确说明正本已经发布；可以重建视图或用同一结果重试，不回滚已完成的知识记录。

Git 同步必须在没有待恢复事务时进行，并持有 writer 锁。Git commit 本身不是多文件事务。直接使用文件系统读取的外部程序不受 RecallCard 的读取屏障约束，因此不能在发布途中把零散文件自行当成一致快照。

## 当前验证

`crates/recallcard/tests/dream.rs` 使用合成中文资料验证离线完整流程、重复导入、批准摘要绑定、来源与范围、受保护修改/替代、助手建议、suppression、过期版本与内容摘要，以及发布前、部分写入后、收据已写入后的恢复。另有损坏日志与外部修改冲突用例。

```sh
RECALLCARD_STATE_DIR=/tmp/recallcard-test-state cargo test -j2 -p recallcard --test dream --test vault_safety
```

这里只验证本机 Rust 文件与协议行为，不表示已经实测任一外部模型的记忆准确率、真实网页交互、云账单、向量服务或跨平台崩溃语义。
