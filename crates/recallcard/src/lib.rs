//! RecallCard 的本地事实源；索引和视图都可重建。
pub mod model;
pub mod vault;
pub use model::*;
pub use vault::Vault;

pub mod capture;
pub mod context;
pub mod policy;
pub mod transport;

pub mod desktop;
pub mod dream;
pub mod dream_task;
pub mod http;
pub mod llm;
pub mod log;
pub mod semantic;
pub mod settings;
pub mod ui_state;

/// 单元测试用：把本机状态目录固定到临时目录，别写进用户真实的 `~/.local/state`。
#[cfg(test)]
pub(crate) fn isolate_test_state() -> std::path::PathBuf {
    use std::sync::OnceLock;
    static DIRECTORY: OnceLock<std::path::PathBuf> = OnceLock::new();
    DIRECTORY
        .get_or_init(|| {
            let directory = std::env::temp_dir().join(format!(
                "recallcard-unit-tests-{}-state",
                std::process::id()
            ));
            std::fs::create_dir_all(&directory).expect("创建单元测试状态目录");
            std::env::set_var("RECALLCARD_STATE_DIR", &directory);
            directory
        })
        .clone()
}
