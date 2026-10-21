//! 知道的资料库：都放在哪、还能不能打开、默认用哪个。
//!
//! 目的是不带参数也能直接干活，而不是每次都敲 `--vault`。这里只记路径和端口，
//! 不含任何密钥或对话内容；也不保存"可达"这个事实——每次现场探一遍，因为
//! 库可能被移走、删掉或者挂在没插的移动硬盘上。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

use crate::model::Result;
use crate::vault::Vault;

const FILE: &str = "vaults.json";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Known {
    pub path: PathBuf,
    pub last_opened: DateTime<Utc>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Registry {
    /// 不带 `--vault` 时用的库。
    pub default: Option<PathBuf>,
    pub known: Vec<Known>,
    /// `ui` 上次用的范围与端口，只作默认值。
    pub scope: Option<String>,
    pub port: Option<u16>,
}

pub fn path() -> Result<PathBuf> {
    Ok(crate::settings::global_dir()?.join(FILE))
}

pub fn load() -> Registry {
    let Ok(path) = path() else {
        return Registry::default();
    };
    match fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
        Err(_) => Registry::default(),
    }
}

fn save(registry: &Registry) -> Result<()> {
    let path = path()?;
    let temporary = path.with_extension("json.tmp");
    let text = serde_json::to_vec_pretty(registry).map_err(|e| e.to_string())?;
    fs::write(&temporary, &text).map_err(|_| "无法保存资料库清单".to_owned())?;
    fs::rename(&temporary, &path).map_err(|_| "无法保存资料库清单".to_owned())
}

/// 临时目录里的库是跑测试或临时试用留下的，记进真清单只会让下次打不开。
///
/// 状态目录被 `RECALLCARD_STATE_DIR` 重定向时不做这个判断：那已经不是用户的
/// 真实清单了（测试就这么跑），记下来不会污染任何人。
fn disposable(root: &Path) -> bool {
    if std::env::var_os("RECALLCARD_STATE_DIR").is_some() {
        return false;
    }
    let temp = std::env::temp_dir();
    root.starts_with(&temp)
        || root.starts_with("/tmp")
        || root.starts_with("/var/tmp")
        || root.starts_with("/run")
}

/// 能不能打开：只探标记文件与必需目录，不扫事件，所以列一百个库也很快。
pub fn reachable(root: &Path) -> Result<()> {
    Vault::open_existing(root).map(|_| ())
}

/// 任何命令成功打开资料库后都调用一次：记进清单，并设为默认。
pub fn remember(root: &Path) -> Result<()> {
    remember_with(root, None, None)
}

pub fn remember_session(root: &Path, scope: &str, port: u16) -> Result<()> {
    remember_with(root, Some(scope), Some(port))
}

fn remember_with(root: &Path, scope: Option<&str>, port: Option<u16>) -> Result<()> {
    if disposable(root) {
        return Ok(());
    }
    let canonical = fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let mut registry = load();
    let now = Utc::now();
    match registry.known.iter_mut().find(|e| e.path == canonical) {
        Some(entry) => entry.last_opened = now,
        None => registry.known.push(Known {
            path: canonical.clone(),
            last_opened: now,
        }),
    }
    registry.default = Some(canonical);
    if let Some(scope) = scope {
        registry.scope = Some(scope.to_owned());
    }
    if let Some(port) = port {
        registry.port = Some(port);
    }
    save(&registry)
}

/// 不带 `--vault` 时找库：默认库可用就用它；只有一个库可用就用它；
/// 否则把知道的库都摆出来，让人自己选。
pub fn resolve(explicit: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(path) = explicit {
        return Ok(path);
    }
    let registry = load();
    if let Some(path) = &registry.default {
        if reachable(path).is_ok() {
            return Ok(path.clone());
        }
    }
    let usable: Vec<&Known> = registry
        .known
        .iter()
        .filter(|entry| reachable(&entry.path).is_ok())
        .collect();
    match usable.len() {
        1 => return Ok(usable[0].path.clone()),
        0 if Path::new("vault").is_dir() => return Ok(PathBuf::from("vault")),
        0 => {
            return Err(match registry.known.is_empty() {
                true => {
                    "还没有可打开的资料库；先运行 recallcard init --vault ~/我的资料".to_owned()
                }
                false => format!(
                    "知道的资料库都打不开了：{}；用 --vault 指定，或 recallcard vault list 看看",
                    registry
                        .known
                        .iter()
                        .map(|entry| entry.path.display().to_string())
                        .collect::<Vec<_>>()
                        .join("、")
                ),
            })
        }
        _ => {}
    }
    let mut names: Vec<String> = usable
        .iter()
        .map(|entry| entry.path.display().to_string())
        .collect();
    names.sort();
    Err(format!(
        "有 {} 个可用的资料库，需要指定一个：{}；用 --vault 指定，或 recallcard vault use <路径> 设为默认",
        names.len(),
        names.join("、")
    ))
}

/// 清单里显示、也用来指代一个库的短名。
///
/// `<应用目录>/vault` 这种路径光看最后一段全是「vault」，所以固定取末两段：
/// `app.recallcard.desktop/vault`、`scratch/scale-demo`。
pub fn label(path: &Path) -> String {
    let mut parts: Vec<String> = path
        .components()
        .rev()
        .take(2)
        .map(|part| part.as_os_str().to_string_lossy().to_string())
        .collect();
    parts.reverse();
    parts.join("/")
}

/// 把命令行给的东西认成某个库：先当路径试，再按短名或末段名在清单里找。
pub fn pick(query: &str) -> Result<PathBuf> {
    let as_path = PathBuf::from(query);
    if as_path.is_dir() {
        return Ok(fs::canonicalize(&as_path).unwrap_or(as_path));
    }
    let registry = load();
    let matched: Vec<PathBuf> = registry
        .known
        .iter()
        .filter(|entry| {
            label(&entry.path) == query
                || entry
                    .path
                    .file_name()
                    .is_some_and(|name| name == std::ffi::OsStr::new(query))
        })
        .map(|entry| entry.path.clone())
        .collect();
    match matched.len() {
        1 => Ok(matched[0].clone()),
        0 => Err(format!(
            "没有叫 `{query}` 的资料库，也不是一个存在的路径；用 recallcard vault list 看看有哪些"
        )),
        _ => Err(format!(
            "有好几个叫 `{query}` 的资料库，请给完整路径：{}",
            matched
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join("、")
        )),
    }
}

/// 把清单整理成给人看的样子；`reachable` 是现场探测的结果。
pub fn listing() -> serde_json::Value {
    let registry = load();
    let rows: Vec<serde_json::Value> = registry
        .known
        .iter()
        .map(|entry| {
            let problem = reachable(&entry.path).err();
            serde_json::json!({
                "path": entry.path,
                "name": label(&entry.path),
                "default": registry.default.as_deref() == Some(entry.path.as_path()),
                "reachable": problem.is_none(),
                "problem": problem,
                "last_opened": entry.last_opened,
            })
        })
        .collect();
    serde_json::json!({"known": rows, "default": registry.default})
}

/// 设为默认。库必须打不开就报错，不让人把一个死路径设成默认。
pub fn set_default(path: &Path) -> Result<serde_json::Value> {
    reachable(path)?;
    remember(path)?;
    Ok(
        serde_json::json!({"default": fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())}),
    )
}

/// 只是从清单里去掉，不动磁盘上的任何文件。
pub fn forget(path: &Path) -> Result<serde_json::Value> {
    let canonical = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let mut registry = load();
    let before = registry.known.len();
    registry.known.retain(|entry| entry.path != canonical);
    if registry.default.as_deref() == Some(canonical.as_path()) {
        registry.default = registry.known.first().map(|entry| entry.path.clone());
    }
    save(&registry)?;
    Ok(serde_json::json!({
        "removed": before != registry.known.len(),
        "default": registry.default,
    }))
}

/// 记住的端口只作提示，占用时会自动换。
pub fn remembered_port(fallback: u16) -> u16 {
    load().port.filter(|port| *port >= 1024).unwrap_or(fallback)
}

/// 范围没指定时用第一个。
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

/// 路径里的符号链接容易让人认不出是哪个库，能规范化就规范化。
pub fn describe(root: &Path) -> String {
    fs::canonicalize(root)
        .unwrap_or_else(|_| root.to_path_buf())
        .display()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, MutexGuard};

    /// 清单是进程级状态，几个测试并行写会互相踩；串行跑并每次清空。
    static LOCK: Mutex<()> = Mutex::new(());

    fn fresh() -> MutexGuard<'static, ()> {
        crate::isolate_test_state();
        let guard = LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let _ = fs::remove_file(path().unwrap());
        guard
    }

    #[test]
    fn the_default_wins_and_a_dead_default_falls_back() {
        let _guard = fresh();
        let dir = tempfile::tempdir().unwrap();
        let first = Vault::init(&dir.path().join("一")).unwrap();
        let second = Vault::init(&dir.path().join("二")).unwrap();
        remember(first.root()).unwrap();
        remember(second.root()).unwrap();
        assert_eq!(resolve(None).unwrap(), second.root());

        // 默认库被搬走：只剩一个可用的，就直接用它，不必再问。
        fs::rename(second.root(), dir.path().join("二-搬走")).unwrap();
        assert_eq!(resolve(None).unwrap(), first.root());
    }

    #[test]
    fn a_broken_vault_stays_listed_with_its_problem() {
        let _guard = fresh();
        let dir = tempfile::tempdir().unwrap();
        let vault = Vault::init(&dir.path().join("库")).unwrap();
        remember(vault.root()).unwrap();
        fs::remove_file(vault.root().join("control/schema-version.json")).unwrap();

        let listing = listing();
        let rows = listing["known"].as_array().unwrap();
        assert_eq!(rows.len(), 1, "库坏了也要留着，让人知道它去哪了：{listing}");
        assert_eq!(rows[0]["reachable"], false);
        assert!(rows[0]["problem"].is_string());
        assert!(resolve(None).is_err(), "唯一可用的库坏了就该报错");
    }

    #[test]
    fn forgetting_only_drops_the_entry_and_leaves_the_vault_alone() {
        let _guard = fresh();
        let dir = tempfile::tempdir().unwrap();
        let vault = Vault::init(&dir.path().join("库")).unwrap();
        remember(vault.root()).unwrap();
        forget(vault.root()).unwrap();
        assert!(listing()["known"].as_array().unwrap().is_empty());
        assert!(vault.root().join("control/schema-version.json").exists());
    }

    #[test]
    fn a_dead_path_cannot_become_the_default() {
        let _guard = fresh();
        let dir = tempfile::tempdir().unwrap();
        assert!(set_default(&dir.path().join("没有这个库")).is_err());
    }
}
