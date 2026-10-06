//! RecallCard 的本地事实源；索引和视图都可重建。
pub mod model;
pub mod vault;
pub use model::*;
pub use vault::Vault;

pub mod capture;
pub mod context;
pub mod policy;
pub mod transport;

pub mod dream;
pub mod import;
pub mod native;
