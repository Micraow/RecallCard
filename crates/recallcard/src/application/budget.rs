//! 月度配额选择；无限只取消月度停止条件，不取消计量、授权或单次安全上限。
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BudgetMode {
    #[default]
    Limited,
    Unlimited,
}
impl BudgetMode {
    pub fn is_limited(&self) -> bool {
        *self == Self::Limited
    }
}
