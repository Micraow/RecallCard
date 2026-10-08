# 不通过 GUI 使用 RecallCard

CLI 与桌面使用同一个资料库和导入服务。首次使用不需要模型密钥，原话导入后即可检索、读取出处。

```sh
recallcard --vault ./my-context setup
recallcard --vault ./my-context import ./official-export.zip --scope personal
recallcard --vault ./my-context search "项目最后决定" --scope personal --budget-bytes 4000 --json
recallcard --vault ./my-context read event:evt_... --scope personal --budget-bytes 4000 --json
recallcard --vault ./my-context sources memory:mem_... --scope personal --budget-bytes 4000 --json
recallcard --vault ./my-context status --json
```

`import <path>` 是整份官方归档入口。旧的 `import --file` 保留原来的兼容合同；新流程优先使用位置参数。

`--json` 输出一个紧凑 JSON，不混入说明或进度日志。应用命令 `setup/import/jobs/status/connect` 返回 `recallcard.cli/1` 包装；兼容查询命令保留原数据结构。失败写入 stderr 并返回非零退出码。普通调用仍提供人类可读的说明或格式化 JSON。

## 让 AI 自己检索与读取

```sh
recallcard --vault ./my-context mcp --scope personal
```

这个 stdio 进程只提供 `bootstrap/search/read/sources`，不提供写入、任意文件读取或命令执行。范围由启动参数固定，不能让模型在查询参数中自行扩大。

`bootstrap` 是已有稳定背景，可能包含尚未更新的旧记忆。回答当前问题时仍应 `search`，必要时 `read` 和 `sources` 核对原话、时间及证据性质。后来的用户纠正与旧 active Memory 冲突时，需要据证据说明冲突；看到 active 不等于已经核对过最新情况。

ChatGPT 原生读取连接也可从 CLI 准备本机授权记录：

```sh
recallcard --vault ./my-context connect chatgpt-mcp \
  --host-identity openai-chatgpt --platform chatgpt \
  --recall-scope personal --provider-disclosure --auto-recall --json
```

这一步只保存明确选择的本机读取权限。正式接入仍须在用户长期使用的电脑上完成官方宿主授权；配置记录不代表真实 ChatGPT 已读取成功。实际取出的资料会提供给该 AI 服务，应仅授权打算共享的范围。

## 长正文与可验证出处

`budget_bytes` 和 CLI `--budget-bytes` 都指 UTF-8 JSON 字节上限，不是模型 token 数。旧名称 `budget_tokens/--budget-tokens` 仅为兼容别名。

搜索结果给出命中附近的原文，以及 `text_range` 的字节定位。完整长正文不必一次塞进模型上下文：

```sh
recallcard --vault ./my-context read event:evt_... --scope personal \
  --offset-bytes 12345 --budget-bytes 4000 --json
recallcard --vault ./my-context read event:evt_... --scope personal \
  --cursor '上一页返回的next_cursor' --budget-bytes 4000 --json
```

偏移必须直接采用服务返回的 UTF-8 字节位置。下一页继续核对来源、授权与快照；记忆修订、抑制或范围变化后，旧游标会失效。`budget_exhausted` 表示预算放不下内容，`no_matches` 才表示本次查询无结果。不得把未读到的部分猜成事实，或把截取的正文冒充完整记录。
