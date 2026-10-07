# 桌面服务边界

`recallcard::desktop::DesktopSession`（别名 `SessionService`）是原生桌面壳与现有 Vault、Context、导入及 Dream 协议之间的本地服务。桌面壳使用 `Mutex<DesktopSession>`，每条服务命令从检查会话到读取/写入完成都持有同一把互斥锁。

## 宿主与界面约定

- 路径只能由原生目录/文件选择器交给 Rust。前端不能传入任意 Vault、导入或 Dream 路径，也没有通用文件、shell、CLI、环境变量、网络或凭据命令
- 创建、笔记保存、导入确认、导出本地 Dream Job、发布 Dream 结果由用户点击触发；打开失败不会自动创建资料库
- `select_vault(path, create)` 返回新的随机 `session_id`。所有后续操作都携带它。切换 Vault（包括切换失败）、关闭 Vault 或进程退出会废弃旧会话与预览
- 服务还核对目录身份和 schema marker，目录在同一路径被替换后，旧命令失效。Unix 使用设备/inode；Windows 通过 `GetFileInformationByHandle` 读取卷序列号与文件索引，不使用可能被 NTFS 保留的创建时间作为身份
- 原生文件选择器返回后再验证最初的 `session_id`。界面也应丢弃旧请求的迟到返回，切换范围时清除所选引用和未确认的预览
- `read`、`sources`、`search`、`browse` 都要求一个明确的 scope，并复用 Context 的抑制和证据可见性检查；没有 `*` 或隐式全部范围
- 无网络、云端执行器或 API key 配置。返回内容是参考资料，界面用纯文本渲染，不解释 HTML、指令或文件中夹带的命令

## 公开接口

| 方法 | 参数（除 self） | 结果 |
| --- | --- | --- |
| `select_vault` | `path: &Path, create: bool` | `VaultInfo` |
| `status` | `session_id: &str` | `VaultInfo` |
| `close_vault` | 无 | 清除整个会话 |
| `browse` | `session_id, scope, target: &str` | 列表 JSON |
| `search` | `session_id, scope, query, target: &str` | Context 搜索 JSON |
| `read` / `sources` | `session_id, scope, reference: &str` | Context 读取 JSON |
| `preview_note` | `session_id, scope, content: &str` | `NotePreview` |
| `confirm_note` | `session_id, preview_id: &str` | `{ref, event}` |
| `select_import_file` | `session_id, format: &str, path: &Path, scope: &str` | `ImportFilePreview`（普通文件预览或 ZIP 清单） |
| `preview_import_selection` | `session_id, selection_id: &str, source_ids: &[String]` | `ImportPreview` |
| `return_import_selection` | `session_id, selection_id: &str` | 撤销写入预览，保留会话清单 |
| `preview_import` | `session_id, format: &str, path: &Path, scope: &str` | `ImportPreview` |
| `confirm_import` | `session_id, preview_id: &str` | 导入结果 JSON |
| `export_dream` | `session_id, scope: &str, source_refs, memory_refs: &[String]` | `DreamJob` |
| `prepare_dream_task` | `session_id, scope: &str, source_refs, memory_refs: &[String]` | 完整中文任务与来源卡片 JSON |
| `review_dream` | `session_id: &str, path: &Path, scope: &str` | `DreamPreview` |
| `review_dream_text` | `session_id, scope, text: &str` | `DreamPreview` |
| `apply_dream` | `session_id, preview_id: &str, approve_protected: bool` | `DreamReceipt` |
| `cancel_previews` | `session_id: &str` | 清除全部待确认预览 |

可失败的方法均返回库内 `Result<T>`（中文字符串错误）。错误固定且有操作提示，不向界面转发操作系统、Git 或 JSON 解析器可能含有的路径、正文和秘密。

`VaultInfo` 包含 `session_id`、`root`、`display_name`、`scopes`、`event_count`、`memory_count` 和 `health`。空资料库默认提供 `personal` 范围。`target` 只接受 `all`、`events`、`memories`。读操作与搜索最多输出 32 KiB；浏览与搜索最多显示 30 条，超出通过 `truncated` 告知。大记录的完整读取可能返回 `pending_refs`，界面不能把空列表误报为没有内容。

## 第一条笔记：粘贴即可

用户可以直接输入或粘贴一段文字，不需要准备导出文件、选择导入格式或编写 JSON。`preview_note` 接受 1–65536 字节的 UTF-8 正文，拒绝纯空白，并返回完整脱敏预览 `NotePreview {preview_id, session_id, scope, content, redacted}`；预览不写入 Vault。脱敏使用 capture 的相同启发式规则，界面仍应提醒用户检查预览。

服务仅在内存中保留脱敏且经过验证的 EventInput，固定范围、发生时间和随机来源消息编号。用户点击保存后，`confirm_note` 只接受会话与预览编号，追加一条 `role=user`、`origin=user_input`、来源平台为 `recallcard-desktop` 的 Event，返回 `{ref, event}`。包含已知上下文注入标记的正文仍服从现有 capture 的来源分类保护。笔记立即可检索，但不自动生成 Memory。

前端编辑正文或改变范围时必须清除旧预览并重新预览；修改前端预览对象不能改变服务端已批准的内容或范围。新预览、取消、切换 Vault 和退出都会撤销旧笔记预览；成功保存后消费编号，重复点击不会再追加。文件来源导入流程仍可用于批量资料。

## 导入预览与确认

支持 `manual-jsonl`、`chatgpt-export`、`claude-code`、`recallcard-conversation` 与 `auto`。普通 JSON / JSONL 上限 16 MiB、5000 个事件；ChatGPT 可读取官方会话数组或单会话对象。ZIP 上限 64 MiB，最多 2048 个条目，每个 JSON 上限 16 MiB，实际展开总量上限 128 MiB。ZIP 只在内存读取，不解压到磁盘；拒绝加密、异常路径、符号链接及其他不支持的压缩包结构。计数和限制以实际读取结果为准。

原生 `pick_import` 调用 `select_import_file`。普通文件返回 `ImportPreview`；ZIP 返回 `ImportSelection {selection_id, session_id, file_name, file_hash, byte_count, scope, coverage, conversations}`。界面先显示会话标题、用户/助手/工具数量及覆盖范围，默认不勾选。即使整个备份超过 5000 条，也可先看清单，再分批选择。`preview_import_selection` 仅接受会话令牌和原始 `source_ids`，不接受路径或正文；空选择、未知编号、所选消息超过 5000 条都不生成可写令牌。

`ImportPreview` 返回 `preview_id`、`session_id`、`file_name`、`format`、`scope`、`file_hash`、`byte_count`、`event_count`、`redacted_event_count`、`samples`、`truncated`、`warning`、`coverage` 和本批 `conversations`。样本最多 6 条、正文每条最多 1200 字节，并显示来源、角色和原始时间；无时间显示未知，不补成导入时间。正文经过正式 capture 使用的脱敏函数，秘密检测仍是启发式的。ZIP 的 coverage 保留整个原文件的跳过统计，界面明确注明包含未选会话；本批消息数和会话列表只包含选择项。

导入只收集 `current_node` 指定分支的可见文本。Markdown 副本、附件、不支持或损坏的 JSON、其他分支、空消息和隐藏推理均单独计数，不能把跳过内容说成已经导入。保留来源消息编号、原始时间、角色与顺序，重复消息版本去重。

预览不修改 Event、Memory，也不产生导入副本。服务将原生选定文件的身份、完整二进制 SHA-256、大小、范围和所选会话绑定到令牌。列出清单 → 生成预览 → 确认写入，每个阶段重新有界读取、核对完整快照；即使仅修改被跳过的附件，或用相同字节的新文件替换，也会废弃批准。拒绝所选文件与父目录符号链接、目录和特殊设备。macOS 仅允许根部 `/var`、`/tmp`、`/etc` 指向对应 `/private` 目录的确切系统别名。解析与正式导入共用，无临时 Vault。

前端不能替换路径、范围、格式或批准正文。返回选择调用 `return_import_selection` 立即撤销旧写入令牌；新预览替换旧预览。成功确认消费令牌，并保留只读清单供下一批使用。取消、切换范围与切换 Vault 清除清单和预览。文件核对失败会废弃令牌，恢复旧字节也不能继续旧批准。

执行中断可能已写入部分 Event；不回滚或删除，重新预览并确认可去重。导入不自动生成 Memory。CLI 的 `import --format chatgpt-export --file backup.zip --scope personal` 同样使用有界二进制解析；`--file -` 仍接收 UTF-8 JSON / JSONL。CLI 单批超限时不会写入，用户可改用桌面会话选择分批导入。

## Dream 审查与发布

导出只允许同一 scope 中 1–64 条 Event 和最多 32 条旧 Memory，总 Job 上限由 Dream 核心约束。核心在本机状态目录记录 Job；桌面壳另行通过保存对话框把完整 `DreamJob` 导出给用户，不外发。

`prepare_dream_task` 复用导出登记，再调用 Rust 的 `dream_task::render_task` 生成最多 1 MiB 的完整中文任务。返回任务编号、输入摘要、范围、数量、字节数、完整 `text` 和来源卡片。卡片正文最多 1200 字节，超过时 `truncated=true`；完整任务中的来源不会截断。该接口不调用模型、不写剪贴板、不发布 Memory。界面在用户点复制时重新调用并核对完整任务字节，确认一致后才调用原生剪贴板命令。

结果文件最多 1 MiB、1–32 条提议，所有提议必须属于界面所选 scope。`DreamPreview` 包含 `preview_id`、`session_id`、`file_name`、`file_hash`、`scope` 和完整 `review`。审查包含 before/after、证据、版本、诊断、`requires_protected_approval` 和 `can_apply`。完整差异超过 4 MiB 时拒绝并要求拆分；不会截断后继续允许发布。

`review_dream_text` 接受同样有界的完整 JSON，或唯一完整的 `json` / `recallcard-dream-result` 代码块。严格解析后保留服务端 `DreamResult`，不使用临时结果文件。部分 JSON、聊天包裹、多对象/多围栏、未知或重复字段全部拒绝，失败也会撤销旧 Dream 预览。文本与文件审查互相替换，不能并存两个可确认的预览。

发布只接受服务保存的 `preview_id`。文件流程重新核对文件身份/字节摘要，文本流程使用服务端保留的解析结果；二者都重新核对结果摘要、当前来源、旧 Memory read-set、suppression 和冲突，再由核心执行有恢复记录的事务。`approve_protected` 必须来自独立、默认未勾选的用户确认控件，来源文件和模型输出不能替用户批准。审查失败或 `can_apply=false` 时不能发布。成功发布后消费预览编号；相同结果重新审查/确认仍由核心幂等处理。

## 已验证边界

`cargo test -p recallcard --test desktop --test import` 覆盖纯文本笔记的空白/字节上限、只读脱敏预览、范围固定、一次确认与过期预览；导入正常预览/确认、脱敏与去重、两个导出适配器、范围/抑制、取消/替换预览、文件篡改、相同字节替代文件、Vault 路径替换、迟到会话、Dream 的只读审查/幂等发布、受保护批准、冲突与过期 read-set。文件系统身份防护用于避免误写和常见替换，不宣称是抵御具有同等本机文件权限的恶意并发进程的完整沙箱。
