//! CLI、桌面与本机服务共用的应用合同；不依赖窗口或终端。
mod contract;
pub use contract::*;
pub mod import_reader;

pub mod imports;
pub use imports::{ImportResult, ImportService};
pub mod background_memory;

mod source_assets;

pub mod runtime;

pub mod connections;

pub mod credentials;
