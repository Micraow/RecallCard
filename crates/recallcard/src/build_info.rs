//! 由实际 Rust 构建取得的版本与提交；开发工作树明确标记 dirty。
#[derive(Debug, Clone, serde::Serialize)]
pub struct BuildInfo {
    pub version: &'static str,
    pub commit: &'static str,
    pub dirty: bool,
}
pub fn build_info() -> BuildInfo {
    BuildInfo {
        version: env!("CARGO_PKG_VERSION"),
        commit: env!("RECALLCARD_BUILD_COMMIT"),
        dirty: env!("RECALLCARD_BUILD_DIRTY") == "true",
    }
}
