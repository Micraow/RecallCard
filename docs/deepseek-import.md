# DeepSeek 官方导出导入

在 DeepSeek 网站的设置里使用官方「导出数据」，把下载的 ZIP 或其中的会话 JSON 交给 RecallCard。导入格式为 `deepseek-export`，自动识别也支持同一结构。整个过程只读用户选择的本地文件，不登录账号，不调用逆向接口，不下载附件。

当前适配器识别顶层会话数组，也接受从数组取出的单个会话对象：会话有 `id`、`mapping`；当前导出另有 `title`、`inserted_at`、`updated_at`。节点为 `id / parent / children / message`，消息为 `files / model / inserted_at / fragments`。平台识别依赖时间字段与 fragment 结构，不依赖 `conversations.json` 文件名。只有 `mapping` 的未知格式不会冒充 ChatGPT 或 DeepSeek。账号资料、设置 JSON 和其他附件只计入跳过统计。

## 保留什么

- `REQUEST` 的可见正文归为用户；`RESPONSE`、`TEMPLATE_RESPONSE` 归为助手。同一消息同时含两种角色时整条跳过，并显示歧义计数
- 保留所有可识别可见消息节点，包括重新生成的兄弟回答。不取第一个子节点，不猜最长分支，也不声称知道网站当时选中的分支
- 原始会话 ID、节点 ID、父节点、子节点、角色、原消息时间和标题进入 Event。平台统一为 `deepseek`，导入适配器记为 `metadata.import_adapter = deepseek-export`
- 时间缺失保持缺失；不会用导入时间或会话更新时间冒充消息时间。有效 RFC 3339 时区会统一到 UTC
- `THINK`、未知/工具/搜索/读取链接片段，以及附件的正文、名称、地址和下载链接都不保存。只保留省略数量，不把这些载荷放进 metadata

导出中的同层节点仅作确定性排放，不能当作线性时间轴。Event 的 `metadata.branch` 为 `all_exported_nodes`；`metadata.deepseek` 保留 `node_id / parent_id / children_ids`，并标记会话是否有分支、父节点是否为空结构根、父消息是否因过滤而省略。读取来源和生成续聊材料时必须保留这些说明。空根节点不生成文本事件，也不被误报成损坏孤儿。`nearest_visible_parent_id` 由已校验树的一次拓扑传播计算，供用户选择一个末端后，只沿该分支可见祖先生成交接；`omitted_parent_nodes` 明示跨过的省略节点，结构根不算遗漏。

每条 Event 均标记为部分覆盖。预览统计精确区分节点、可见消息、空消息、隐藏独占消息、不支持消息、混合角色消息、隐藏片段、其他片段、附件、分叉点和缺时间的消息。统计范围是收到的导出版本；重复版本的观察计数可能大于实际新增事件数。文件外的历史、已删除消息和未导出的附件不在覆盖范围内。

## 去重与完整性

多个文件可进入同一个任务。不同 ZIP 都叫 `conversations.json` 不会相互覆盖；会话选择键为平台加原始会话 ID。同一平台同一会话同一来源版本复导去重，正文或来源关系发生变化时按现有 Event 修订链追加；已见过的旧版本复导不会撤销新版本。导出没有可验证的逐消息修订时间，因此未见过的内容版本按本机观察顺序保留修订关系，不宣称这是平台编辑先后；会话更新时间不会伪装成逐消息修订时间。不同平台同名 ID 使用不同选择键。

对象键重复、节点 ID 与键冲突、重复子节点、缺父/子节点、不一致的双向引用、循环、损坏时间和不合法消息结构会使解析失败。不会只留下看起来正常的几条消息，然后把损坏备份报告成完整导入。没有当前 schema 实证的旧版或第三方转换格式不会被猜测兼容。

安全上限：每任务最多 32 个文件、输入合计 128 MiB、展开合计 256 MiB、100000 条不同来源版本事件；单 ZIP 最多 64 MiB、2048 个条目、展开 128 MiB；单 JSON/JSONL 最多 16 MiB；单 Event 正文最多 2 MiB、总结构最多 4 MiB。超限明确失败，不截断。5000 条不再是官方导出的人工拆分要求。上限不是性能承诺。ZIP 不落地解压，保留路径、压缩方式、重复名称、声明/实际大小和完整性检查。

批量捕获在同一写锁内只加载一次已有事件索引，整批先脱敏和验证，再追加文件。取消和文件系统错误可能留下已写前缀；该前缀保留，重新执行会按来源版本去重，不承诺事务回滚。Memory 和受保护背景记忆不会由导入自动改变。

## 核心接口与验证

- `inspect_import_files(format, &[&[u8]], scope)`：只读清单与覆盖统计
- `parse_import_files_selected(format, files, scope, selection_keys)`：统一解析和去重，空集合表示不选择任何会话
- 单文件兼容入口：`inspect_import_bytes`、`parse_import_bytes`、`parse_import_bytes_selected`
- `capture_batch_with_callbacks`：同一写锁下提供初始已存在 Event、逐事件取消检查和持久化后进度；回调不能重新取得 Vault 锁

`tests/import_deepseek.rs` 的样本全部人工构造，覆盖官方形状、角色、时间、全部兄弟分支、遗漏标签、结构拒绝、跨文件去重、修订链、5000 条以上会话以及批量取消/重试；沿用 `tests/import_archive.rs` 的 ZIP 安全测试。未使用真实账号历史作为 fixture，也未以合成测试冒充真实用户导出验收。

结构参考仅用于独立实现，没有复制第三方遍历逻辑或样本正文：

- [DeepSeek 官方公开前端包](https://fe-static.deepseek.com/chat/static/main.6fca03582d.js)，SHA-256 `c1a25cbb78e344205463cf54d673c1e0f01db007697b92d84fb8131c6feab7de`，用于确认正常导出入口及 `TEMPLATE_RESPONSE` 类型
- [公开导出结构模板](https://github.com/ngallodev-software/conversation-export-workbench/blob/47183570733e1d6cb41dc633b9936cda8c7ad0de/provider_templates/deepseek.conversations-template.json)，以及同提交的 `formatters/deepseek.py`、`sample_data/deepseek-convo.json`。第三方资料是当前结构佐证，不构成官方长期兼容承诺
