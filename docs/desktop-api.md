# 桌面服务边界

`recallcard::desktop::DesktopSession`（别名 `SessionService`）是原生桌面壳与现有 Vault、Context、导入及 Dream 协议之间的本地服务。桌面壳使用 `Mutex<DesktopSession>`，每条服务命令从检查会话到读取/写入完成都持有同一把互斥锁。

## 宿主与界面约定

- 路径只能由原生目录/文件选择器交给 Rust。前端不能传入任意 Vault、导入或 Dream 路径，也没有通用文件、shell、CLI、环境变量、网络或凭据命令
- 创建、导入确认、导出本地 Dream Job、发布 Dream 结果由用户点击触发；打开失败不会自动创建资料库
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
| `preview_import` | `session_id, format: &str, path: &Path, scope: &str` | `ImportPreview` |
| `confirm_import` | `session_id, preview_id: &str` | 导入结果 JSON |
| `export_dream` | `session_id, scope: &str, source_refs, memory_refs: &[String]` | `DreamJob` |
| `review_dream` | `session_id: &str, path: &Path, scope: &str` | `DreamPreview` |
| `apply_dream` | `session_id, preview_id: &str, approve_protected: bool` | `DreamReceipt` |
| `cancel_previews` | `session_id: &str` | 清除两类待确认预览 |

可失败的方法均返回库内 `Result<T>`（中文字符串错误）。错误固定且有操作提示，不向界面转发操作系统、Git 或 JSON 解析器可能含有的路径、正文和秘密。

`VaultInfo` 包含 `session_id`、`root`、`display_name`、`scopes`、`event_count`、`memory_count` 和 `health`。空资料库默认提供 `personal` 范围。`target` 只接受 `all`、`events`、`memories`。读操作与搜索最多输出 32 KiB；浏览与搜索最多显示 30 条，超出通过 `truncated` 告知。大记录的完整读取可能返回 `pending_refs`，界面不能把空列表误报为没有内容。

## 导入预览与确认

仅支持 `manual-jsonl`、`chatgpt-export` 和 `claude-code`。单文件最多 16 MiB、5000 个事件；拒绝无内容导入、文件/任意父目录符号链接、目录、特殊设备及无效 UTF-8。macOS 仅允许根部 `/var`、`/tmp`、`/etc` 指向对应 `/private` 目录的确切系统别名，再核对规范路径与文件身份。解析函数与正式导入共用，不通过临时 Vault 做预览。

`ImportPreview` 返回 `preview_id`、`session_id`、`file_name`、`format`、`scope`、`file_hash`、`byte_count`、`event_count`、`redacted_event_count`、`samples`、`truncated`、`warning`。样本最多 6 条、每条正文最多 1200 字节，经过正式 capture 使用的脱敏函数。秘密检测仍是启发式的，不能承诺原文件没有其他秘密。

预览不会修改用户 Event、Memory 或产生导入副本。Rust 只保留原文件身份/摘要、格式、范围和随机预览编号；用户确认时重新有界读取原文件并检查身份及 SHA-256，再导入该份已核对的内容。前端不能替换路径、范围、格式或批准正文。新预览会替换旧预览，成功确认后消费编号。取消应调用 `cancel_previews`，并清空页面预览。

导入沿用既有追加语义。执行中断可能已写入部分 Event；不回滚或删除，重新预览并确认可去重。它不生成 Memory。

## Dream 审查与发布

导出只允许同一 scope 中 1–64 条 Event 和最多 32 条旧 Memory，总 Job 上限由 Dream 核心约束。核心在本机状态目录记录 Job；桌面壳另行通过保存对话框把完整 `DreamJob` 导出给用户，不外发。

结果文件最多 1 MiB、1–32 条提议，所有提议必须属于界面所选 scope。`DreamPreview` 包含 `preview_id`、`session_id`、`file_name`、`file_hash`、`scope` 和完整 `review`。审查包含 before/after、证据、版本、诊断、`requires_protected_approval` 和 `can_apply`。完整差异超过 4 MiB 时拒绝并要求拆分；不会截断后继续允许发布。

发布只接受服务保存的 `preview_id`。服务重新核对文件身份/字节摘要、结果摘要、当前来源、旧 Memory read-set、suppression 和冲突，再由核心执行有恢复记录的事务。`approve_protected` 必须来自独立、默认未勾选的用户确认控件，来源文件和模型输出不能替用户批准。审查失败或 `can_apply=false` 时不能发布。成功发布后消费预览编号；相同结果重新审查/确认仍由核心幂等处理。

## 已验证边界

`cargo test -p recallcard --test desktop --test import` 覆盖正常预览/确认、脱敏与去重、两个导出适配器、范围/抑制、取消/替换预览、文件篡改、相同字节替代文件、Vault 路径替换、迟到会话、Dream 的只读审查/幂等发布、受保护批准、冲突与过期 read-set。文件系统身份防护用于避免误写和常见替换，不宣称是抵御具有同等本机文件权限的恶意并发进程的完整沙箱。
