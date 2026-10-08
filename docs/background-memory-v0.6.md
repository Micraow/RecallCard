# 后台记忆整理纵切 v0.6

本页描述已落地的运行时模块和验证边界。CLI、桌面、独立进程接线由共同应用服务提供；单独模块测试不代表发行物已经完成全链路验收。

## 一次配置与可见边界

`application::background_memory::MemoryRuntime` 保存本机配置、任务、输入快照、模型结果和用量预留。文件位于资料库对应的本机状态目录，不进入资料库 Git 正本。Event/Memory 与 Dream 成功收据仍是可追溯事实源；普通检索不依赖 provider。

- 当前纵切每个资料库支持一个活动整理范围。切换范围或模型目的地须提供新的精确授权，旧任务不能沿用授权。
- 模型配置与授权记录分离。授权明确绑定 HTTPS 完整接收地址、模型、scope、完整来源快照（包括角色、时间、来源与元数据）、相关旧记忆快照、普通可回退更改的自动应用。
- 请求次数、输出 token 上限与发送字节上限由配置约束。月度 token 预算是保守预留，供应商报告用量单独记录；未知用量不退还预留。它不是账单或货币费用保证。
- 正常设置从应用的密码字段接收密钥，选择操作系统保护存储或仅当前后台服务会话。真实适配器使用 [keyring 3.6.3](https://docs.rs/keyring/3.6.3/keyring/) 的 macOS Keychain、Windows Credential Manager 与 Linux Secret Service 后端；每个平台的运行验证单独报告。后端确定不可用时明确降级为会话内存，保存状态不明时停止并说明，不自动重复保存。
- 会话凭据通过新建后台进程的匿名 stdin 管道交接，模型 worker 每次通过自己的 `Command.env` 接收相应供应商的凭据。绝不改变多线程服务的全局环境，也不写入 Vault、Git、配置、任务、参数或日志。关闭 GUI 不会清除仍运行服务的会话凭据；停止服务后丢失。
- `RECALLCARD_DREAM_API_KEY` 保留为高级兼容启动渠道。明确选择会话存储后，重启服务不会悄悄加载旧系统凭据或环境变量。仅配置 provider 不能授权发送来源数据。
- 当前 adapter 为 HTTPS OpenAI-compatible Chat Completions；没有宣称本地模型或所有供应商已验证。

## 处理与恢复

服务调用 `tick`：增量发现 → 有界来源投影 → 检索相关旧记忆 → 一次受监督的 Python 调用 → 确定性审查 → Dream 事务提交 → 派生视图刷新。

不会把已注入的背景、旧版本、越界、被遗忘或撤权资料当作新证据。原始角色、来源和未知时间保持原样；未知时间不能凭整理填成捕获时间。助手推断只可保存为 tentative，不能自动成为用户事实。

普通符合配置的更改自动应用。冲突、受保护记忆、用户维护的记忆以及缺乏来源依据的时间进入 `needs_input`。用户实际修正正文或标签后取得维护权，原有 evidence、来源与 tentative 状态不会因此伪造升级。混合助手/用户引用不会自动升级为明确用户事实。模型评分不创造授权。

修订只在同范围、同结构化来源身份内生效。历史跨范围错误修订边不会隐藏另一个范围的来源。ChatGPT 未选择的替代分支仍可搜索，但默认不会自动整理为当前用户事实；分支未知的导出保留歧义。

任务与月度预算在调用前落盘。调用是否完成不明时不会自动重发；明确重试会占用新的预算。已缓存有效结果继续审查/提交，不重复调用模型。已有成功收据优先恢复，避免重复提交。旧来源已修订或被遗忘时，重试退出旧输入并仅依据当前仍有效来源重新排队。

暂停、取消、配置撤权在提交前再次检查。在请求已经发出之后，暂停不能收回供应商已经接收的数据，但会阻止此结果自动提交。连接或额度故障会阻止继续用下一批资料消耗调用预算。

单条来源超过投影限制时不截断、不发送，状态显示需处理数量；原始来源仍保留用于检索。当前调度器使用安静期或批次阈值，单实例串行处理，最大并发为 1。

## 共享 API

- `MemoryRuntime::new(&Vault)`
- `configure(MemoryConfig)`
- `status()` / `status_for_scope(scope)`
- `jobs()`
- `control(job_id, Pause | Resume | Retry | Cancel)`
- `tick(&mut impl MemoryProvider)`
- `PythonMemoryProvider::new(python, python_path)`，仅由受信任发行物启动器选择执行路径
- `runtime::configure_provider(...)`：共享设置入口，瞬态 key 参数与配置严格分离
- `runtime::run_service / ensure_service / ensure_service_with_credential / service_status / request_service_stop`
- `DesktopSession::model_operation(...)`：短暂会话锁内生成拥有资料库身份与范围的操作句柄；keyring、停机等待和设置在独立 blocking 任务中执行
- `MemoryRuntime::review(job_id, scope)`：当前来源/撤权检查后的可读候选差异；不是自动批准

任务 envelope 复用 `JobStatus<MemoryProgress>` 与公共 `JobState` / `JobPhase`。没有伪造进度百分比。默认状态明确显示“已保存的来源可搜索；后台整理尚未启用”，不要求人工贴 JSON。

## 已验证与未验证

新增回归只使用合成资料与 fake provider，覆盖授权、来源版本/遗忘、未知时间、推断角色、保护与用户维护事实、预算预留、暂停/恢复、旧结果缓存、失败阻塞、重试和幂等收据。Python 协议测试使用注入 transport；从未调用真实云模型或发送私人归档。

这些结果证明协议和确定性边界，不证明真实模型记忆质量、价格、账单、跨客户端理解效果或所有目标平台发行物可用。完整检查结果以本次构建记录为准。


## 服务与凭据验收边界

共同进程有单实例租约、导入调度线程、记忆调度和真实心跳。已包含独立 CLI 服务在启动器退出后继续运行、停机时不应用迟到模型结果的合成测试。运行时保存实际 build 信息与程序摘要，避免静默混用旧后台程序。

系统 keyring adapter 已实现；默认回归使用 keyring 官方 mock 和合成 backend，不写真实系统凭据。Linux 图形登录会话、macOS Keychain 和 Windows Credential Manager 的真实保存/解锁/重启行为仍需各平台验收，不能用 mock 通过代替。当前会话管道、父进程环境不变、不同供应商子进程隔离、重启缺密钥时不收费重试均需独立回归。
