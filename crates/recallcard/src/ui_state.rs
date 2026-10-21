//! 上次用过的资料库与界面参数。
//!
//! 目的是让 `recallcard ui` 不带参数也能直接打开：记住最近成功打开过的库、
//! 用过的范围和端口。这里只存路径与端口，不含任何密钥或对话内容。

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

use crate::model::Result;
use crate::vault::Vault;

const FILE: &str = "last-used.json";

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LastUsed {
    pub vault: Option<PathBuf>,
    pub scope: Option<String>,
    pub port: Option<u16>,
}

pub fn path() -> Result<PathBuf> {
    Ok(crate::settings::global_dir()?.join(FILE))
}

pub fn load() -> Result<LastUsed> {
    let path = path()?;
    match fs::read_to_string(&path) {
        Ok(text) => Ok(serde_json::from_str(&text).unwrap_or_default()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(LastUsed::default()),
        Err(_) => Ok(LastUsed::default()),
    }
}

fn save(next: &LastUsed) -> Result<()> {
    let path = path()?;
    let temporary = path.with_extension("json.tmp");
    let text = serde_json::to_vec_pretty(next).map_err(|e| e.to_string())?;
    fs::write(&temporary, &text).map_err(|_| "无法保存上次使用的资料库".to_owned())?;
    fs::rename(&temporary, &path).map_err(|_| "无法保存上次使用的资料库".to_owned())
}

/// 任何命令成功打开资料库后都调用一次，下一次 `ui` 就能零参数启动。
pub fn remember_vault(root: &Path) -> Result<()> {
    let mut state = load()?;
    state.vault = Some(root.to_path_buf());
    save(&state)
}

pub fn remember_session(root: &Path, scope: &str, port: u16) -> Result<()> {
    let mut state = load()?;
    state.vault = Some(root.to_path_buf());
    state.scope = Some(scope.to_owned());
    state.port = Some(port);
    save(&state)
}

/// 零参数 `ui` 找库的顺序：显式参数 > 上次用过的 > 当前目录下的 vault/。
pub fn resolve_vault(explicit: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(path) = explicit {
        return Ok(path);
    }
    if let Some(path) = load()?.vault {
        if path.is_dir() {
            return Ok(path);
        }
    }
    let fallback = PathBuf::from("vault");
    if fallback.is_dir() {
        return Ok(fallback);
    }
    Err("还没有可打开的资料库；先运行 recallcard init --vault ~/我的资料".to_owned())
}

/// 记住的端口只作提示，占用时会自动换。
pub fn remembered_port(fallback: u16) -> u16 {
    load()
        .ok()
        .and_then(|state| state.port)
        .filter(|port| *port >= 1024)
        .unwrap_or(fallback)
}

/// 打开一个库，并把它的范围取出来；范围没指定时用第一个。
pub fn resolve_scope(root: &Path, explicit: Option<String>) -> Result<String> {
    if let Some(scope) = explicit {
        return Ok(scope);
    }
    let mut session = crate::desktop::DesktopSession::default();
    let info = session.select_vault(root, false)?;
    Ok(info
        .scopes
        .first()
        .cloned()
        .unwrap_or_else(|| "personal".into()))
}

/// 收藏夹里留下的地址不该因为端口变化而失效，这里只在提示里说明。
pub fn describe(root: &Path) -> String {
    match Vault::open_existing(root) {
        Ok(vault) => vault.root().display().to_string(),
        Err(_) => root.display().to_string(),
    }
}
