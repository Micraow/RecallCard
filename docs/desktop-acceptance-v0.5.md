# RecallCard 0.5 Linux 精确程序验收记录

## 已通过的同轮证据

2026-10-08 的[运行 37707650839](https://github.com/Micraow/RecallCard/actions/runs/37707650839)已完成且成功。验收脚本提交为 `76756867201aa91ed5494a9fab953beba97ac541`，应用源码提交固定为 `12f4672fe0f3fe3fac0702c33998501950b01538`。后续打包只复用以下两份已验收程序，不改变程序字节。

- 原生 job 113085754942：同一轮 37/37 唯一检查点全部通过，summary.success 为 true，74 张编号截图齐全，无 failure 文件
- Chromium job 113085755392：73/73 界面回归通过。模拟响应的浏览器回归与真实 Tauri IPC 原生闭环分开记账
- 原生 summary.json 的 SHA-256：`e89516c281be9856e6589d1ac23bb4cad334e33c62e98f069372f18795abecdc`
- canonical-evidence.json 的 SHA-256：`a831145ca5ca6ac92eb15cb646948903bd5a49f50db7d1567d3b9d8aeee7a058`

覆盖首次默认起步、JSON/ZIP、53 会话、整理提议与人工批准、保护/遗忘/背景、范围保存、原生双文件选择一次批准、重复去重和本批会话、具体分支接续、跨会话三条原话、相邻原文往返、820×620 窗口、结束驱动会话后新进程恢复、清理旧草稿及重新打开历史批次。

合成资料共119条 Event，其中 DeepSeek 六条：首次新增6，重复新增0/重复6；回答甲/乙两份交接各只含共同问题与被选回答。跨会话选文3条，退出前遗留选择2条，新搜索恢复默认3条，旧目标和预览不沿用；canonical Event/Memory 未变化。新进程从已完成的零新增批次找回恰好2个本批会话。真实窗口与内容区均为820×620。全部素材为合成数据，未使用真实私人聊天。

## 程序身份与原构建

[原构建 37702406803](https://github.com/Micraow/RecallCard/actions/runs/37702406803) 的 job 113068726523 已通过官方系统依赖安装、格式、严格 Clippy、GUI 编译及 CLI 编译。该次早期原生运行失败，不能称原构建全绿；上面的37步成功证据来自后续复验同一组程序。

- 原程序 artifact：11517704706，`RecallCard-gui-diagnostic-binaries-12f4672fe0f3fe3fac0702c33998501950b01538`
- 原程序 artifact ZIP SHA-256：`d1b0e8227384dde60b3c135c15ae49607a057735e581f57ae2d0d283dc2fe18b`
- GUI SHA-256：`43b173b96502e04b41ebf9057e892393b92075f6b6ead83b486aaca5a61c5569`
- CLI SHA-256：`8e58de814c9e47309aa61e800686d799389971005671fd6fb643026397c17a78`
- 程序元数据0.5.0，dev profile；左下角仍显示“桌面版0.4”，属于已知展示误差，本批不为改字重新编译
- Linux x86_64，GLIBC至少2.35，GTK3、WebKitGTK4.1与Git由系统提供

## 解压运行包的独立门禁

打包提交、原生验收提交与应用提交分别记在包内 build-info.json。依赖清单取 GUI 与独立 CLI 两个 Cargo.lock 的 Linux 精确解析并集，不用 GUI 清单替代 CLI 清单。许可证正文及 MPL 对应源码随包提供。

发布到本轮 Actions artifacts 前必须：核对上述已通过作业与固定 artifact；从同一输入连续打包两次并比较 ZIP SHA-256；安全解压校验全文件清单、权限和两份固定程序摘要；移走开发界面源码，从全新解压目录运行随包启动脚本和 CLI，再完整跑同一37步；最后再次核对成品文件未变。只声称同环境打包可复现，不声称二进制可复现构建。打包流水线的最终结果见包内 build-info.json 的 packaging_run 链接；本节说明门禁，不把先前原生通过推算为新包装已经通过。

这不是公开 Release，不使用仍排队的旧安装包任务的产物，也不执行 AppImage/deb 安装。运行前和升级/备份方法见[中文开始指南](desktop-quickstart-v0.5.md)。

## 未测边界

用户自己的 Arch/KDE Wayland、其他桌面平台、AppImage/deb 安装/升级事务、真实登录账号的官方导出与未知旧版文件、系统关闭按钮的退出路径尚未验收。原生选择分支用标准 WebDriver option 点击，不代表键盘导航已通过。小样本未强制抢点暂停，暂停/续传的语义由核心合同覆盖，不能声称所有原生暂停操作已实跑。旧版读取新版写入后的资料兼容性、移动 Vault 后未完成任务续传也未验收。
