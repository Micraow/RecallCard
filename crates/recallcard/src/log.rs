//! 终端日志。
//!
//! 界面里的动作（检索、整理、重建索引、Agent 调工具）都必须能在终端看见：
//! 出问题时人第一眼应该看终端，而不是去猜界面为什么转圈。全部写 stderr，
//! stdout 留给命令的 JSON 输出。

use std::time::Instant;

pub fn note(scope: &str, message: impl AsRef<str>) {
    let now = chrono::Local::now().format("%H:%M:%S");
    eprintln!("{now} [{scope}] {}", message.as_ref());
}

/// 记录一次动作的耗时与结果。
pub fn done(scope: &str, message: impl AsRef<str>, started: Instant) {
    note(
        scope,
        format!(
            "{} （{} ms）",
            message.as_ref(),
            started.elapsed().as_millis()
        ),
    );
}
