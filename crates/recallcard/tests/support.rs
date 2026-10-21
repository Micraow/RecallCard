//! 测试共用：把本机状态目录固定到本次进程的临时目录。
//!
//! 不这么做的话，每个测试库都会在用户真实的 `~/.local/state/recallcard` 下留一个
//! 哈希目录，跑几次就攒出成千上万个；而且命令行子进程与父进程必须看到同一个
//! 状态目录，否则那把 `write.lock` 就不是同一把锁。

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// 整个测试进程共用一个临时状态目录，并返回它。
pub fn isolate_state() -> &'static Path {
    static DIRECTORY: OnceLock<PathBuf> = OnceLock::new();
    DIRECTORY
        .get_or_init(|| {
            let directory =
                std::env::temp_dir().join(format!("recallcard-tests-{}-state", std::process::id()));
            std::fs::create_dir_all(&directory).expect("创建测试状态目录");
            std::env::set_var("RECALLCARD_STATE_DIR", &directory);
            directory
        })
        .as_path()
}
