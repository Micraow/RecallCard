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
pub mod import;
pub mod import_bundle;
pub mod import_deepseek;
pub mod native;

pub mod agent_hook;
pub mod ipc;
pub mod semantic;
pub mod sync;

pub mod conversation;
