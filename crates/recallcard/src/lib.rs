//! RecallCard 的本地事实源；索引和视图都可重建。
pub mod model;
pub mod navigation;
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
pub mod import;
pub mod import_bundle;
pub mod import_deepseek;
pub mod import_qwen;
pub mod native;

pub mod agent_hook;
pub mod ipc;
pub mod semantic;
pub mod sync;

pub mod conversation;

pub mod application;

mod filesystem;
mod vault_stream;
pub use vault_stream::{ChunkReceipt, EventStreamWriter};

mod build_info;
pub use build_info::{build_info, BuildInfo};

#[cfg(unix)]
mod event_locator;
