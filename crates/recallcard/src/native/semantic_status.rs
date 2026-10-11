//! 连接状态只披露恢复动作与不可逆版本标记，不返回配置路径/端点/模型参数。
use crate::{application::background_memory::MemoryRuntime, policy::Access, Vault};
use serde::Serialize;
use std::path::Path;
#[derive(Serialize)]
pub(super) struct SemanticStatus {
    pub state: &'static str,
    pub reconnect_required: bool,
    pub message: &'static str,
    pub revision: String,
}
pub(super) fn managed_path(vault: &Vault, path: &Path) -> bool {
    vault
        .state_dir()
        .is_ok_and(|dir| path.starts_with(dir.join("semantic")))
}
pub(super) fn status(
    vault: &Vault,
    access: &Access,
    path: Option<&Path>,
    loaded_signature: Option<&str>,
) -> SemanticStatus {
    let desktop = MemoryRuntime::new(vault).configuration();
    let current_path = desktop
        .as_ref()
        .ok()
        .filter(|cfg| access.permits(&cfg.scope))
        .and_then(|cfg| cfg.candidate_semantic_config.as_deref());
    let current_signature = path
        .and_then(|path| super::load_native_semantic(vault, path, &access.scopes()).ok())
        .and_then(|backend| backend.config_fingerprint().ok());
    let managed = path.is_some_and(|p| managed_path(vault, p));
    let (state, reconnect_required, message) = if desktop.is_err() {
        (
            "unavailable",
            true,
            "无法核验当前桌面语义设置。请在桌面修复设置、重新生成浏览器连接文件，再重新检查。",
        )
    } else if (managed || current_path.is_some()) && current_path != path {
        ("stale",true,"浏览器启动文件与当前桌面语义设置不一致。请在桌面连接页重新生成/注册浏览器连接，再重新检查。当前保留目录与词法读取。")
    } else if path.is_some()
        && (loaded_signature.is_none() || loaded_signature != current_signature.as_deref())
    {
        ("stale",true,"本机语义配置已改变或无法读取。请在桌面核对设置并重新生成连接文件，再重新检查。当前保留目录与词法读取。")
    } else if loaded_signature.is_some() {
        (
            "configured",
            false,
            "此浏览器的独立语义启动配置已对齐；实际索引与新查询以读取结果为准。",
        )
    } else {
        (
            "unconfigured",
            false,
            "此浏览器尚未配置语义检索，目录与词法读取可用。启用桌面语义设置后需重新生成连接文件。",
        )
    };
    let revision = crate::model::hash(
        format!("{state}:{path:?}:{loaded_signature:?}:{current_path:?}:{current_signature:?}")
            .as_bytes(),
    );
    SemanticStatus {
        state,
        reconnect_required,
        message,
        revision,
    }
}
