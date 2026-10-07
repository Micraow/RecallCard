//! 用户显式发起的保守 Git 同步；不重写历史，不替用户解决记忆冲突。
use crate::{
    dream::DreamReceipt,
    model::{hash, validate_id, Result, SCHEMA_VERSION},
    policy::Suppression,
    vault::{read_json, reject_symlink},
    Vault,
};
use serde_json::{json, Value};
use std::{
    collections::BTreeSet,
    fs,
    path::Path,
    process::{Command, Output, Stdio},
};

impl Vault {
    /// 仅同步明确指定、已经配置的远端与当前同名分支。
    pub fn sync(&self, remote: &str) -> Result<Value> {
        validate_remote(remote)?;
        let result = {
            let _guard = self.lock()?;
            self.sync_locked(remote)?
        };
        // rebuild 自身按步骤加锁，不能在持有 writer 锁时调用。
        let rebuilt = self.rebuild().map_err(|error| {
            format!("Git 已正常推送，但本机索引/视图重建失败：{error}；请执行 rebuild")
        })?;
        Ok(json!({"ok":true,"git":result,"rebuild":rebuilt}))
    }

    /// 只读本机状态，不访问远端，也不暴露远端 URL 或凭据。
    pub fn git_status(&self) -> Result<Value> {
        let _guard = self.read_guard()?;
        self.check_git_root()?;
        let branch = self.git_optional(&["symbolic-ref", "--quiet", "--short", "HEAD"])?;
        let head = self.git_optional(&["rev-parse", "--verify", "HEAD"])?;
        let conflicts = self.git_paths(&["diff", "--name-only", "--diff-filter=U", "-z"])?;
        let changed = self.changed_paths()?;
        let staged = self.git_paths(&["diff", "--cached", "--name-only", "-z"])?;
        let remotes = self.git_text(&["remote"], "列出远端")?;
        Ok(json!({
            "ok":true,"branch":branch,"head":head,
            "ongoing_operations":self.ongoing_operations()?,"conflicts":conflicts,
            "changed_paths":changed,"staged_paths":staged,
            "syncable_paths":changed.iter().filter(|p| approved_path(p)).collect::<Vec<_>>(),
            "remote_names":remotes.lines().collect::<Vec<_>>()
        }))
    }

    fn sync_locked(&self, remote: &str) -> Result<Value> {
        self.check_git_root()?;
        self.ensure_git_idle()?;
        let branch = self
            .git_optional(&["symbolic-ref", "--quiet", "--short", "HEAD"])?
            .ok_or("HEAD 处于游离状态；请先切换到要同步的本地分支")?;
        let branch_ref = format!("refs/heads/{branch}");
        self.git_ok(&["check-ref-format", &branch_ref], "检查分支名")?;
        let remotes = self.git_text(&["remote"], "列出远端")?;
        if !remotes.lines().any(|name| name == remote) {
            return Err(format!(
                "远端 {remote} 尚未配置；请先明确配置可信的 Git 远端"
            ));
        }
        let urls = self.git_text(&["remote", "get-url", "--all", remote], "检查远端")?;
        let push_urls = self.git_text(
            &["remote", "get-url", "--push", "--all", remote],
            "检查推送目标",
        )?;
        if urls.lines().count() != 1 || push_urls.lines().count() != 1 {
            return Err("同步只支持单个拉取 URL 与单个推送 URL；请明确整理远端配置".into());
        }
        // 不让已暂存的秘密或派生文件混入程序创建的提交。
        self.check_index_paths()?;
        self.validate_sync_canonical()?;
        let before = self.git_optional(&["rev-parse", "--verify", "HEAD"])?;
        if let Some(head) = before.as_deref() {
            self.check_tree(head)?;
            self.ensure_immutable_events(head, None)?;
        }
        let ignored = self.git_paths(&[
            "ls-files",
            "--others",
            "--ignored",
            "--exclude-standard",
            "-z",
        ])?;
        if ignored.iter().any(|path| approved_path(path)) {
            return Err(
                "存在被 Git 忽略的正本文件；请修正忽略规则后再同步，正本不能悄悄漏传".into(),
            );
        }
        let changed = self.changed_paths()?;
        let approved: Vec<&str> = changed
            .iter()
            .filter(|p| approved_path(p))
            .map(String::as_str)
            .collect();
        if !approved.is_empty() {
            self.ensure_git_identity()?;
            // 逐个使用字面路径，既不递归 add 整个目录，也不解释路径通配符。
            for batch in approved.chunks(128) {
                let mut args = vec!["add", "--all", "--"];
                args.extend_from_slice(batch);
                self.git_ok(&args, "暂存已完成正本")?;
            }
        }
        self.check_index_paths()?;
        let staged = self.git_output(&["diff", "--cached", "--quiet", "--exit-code"])?;
        match staged.status.code() {
            Some(0) => {}
            Some(1) => {
                self.ensure_git_identity()?;
                self.git_ok(
                    &[
                        "commit",
                        "--quiet",
                        "--no-gpg-sign",
                        "-m",
                        "保存 RecallCard 已完成事务",
                    ],
                    "提交本机正本",
                )?;
            }
            _ => return Err("无法检查 Git 暂存状态，未开始远端同步".into()),
        }
        let local = self.git_text(&["rev-parse", "--verify", "HEAD"], "读取本机提交")?;
        let local = local.trim().to_owned();
        let remote_state = self.git_output(&[
            "ls-remote",
            "--exit-code",
            "--heads",
            "--",
            remote,
            &branch_ref,
        ])?;
        let integration = match remote_state.status.code() {
            Some(2) => "new_remote_branch",
            Some(0) => {
                // 空 refmap 使本次只取指定分支，不采用仓库中宽泛的 fetch refspec。
                self.git_ok(
                    &[
                        "fetch",
                        "--quiet",
                        "--no-tags",
                        "--no-recurse-submodules",
                        "--refmap=",
                        "--",
                        remote,
                        &branch_ref,
                    ],
                    "拉取指定分支",
                )?;
                let incoming = self.git_text(
                    &["rev-parse", "--verify", "FETCH_HEAD^{commit}"],
                    "读取远端提交",
                )?;
                let incoming = incoming.trim();
                self.check_tree(incoming)?;
                self.integrate(&local, incoming)?
            }
            _ => return Err(
                "无法查询指定远端分支；本机已完成事务保留在本地提交中，请检查连接与远端配置后重试"
                    .into(),
            ),
        };
        self.ensure_git_idle()?;
        self.validate_sync_canonical().map_err(|error| {
            format!("整合后的正本校验失败，已保留工作区和历史，未推送：{error}")
        })?;
        self.check_index_paths()?;
        let after = self.git_text(&["rev-parse", "--verify", "HEAD"], "读取整合提交")?;
        let refspec = format!("HEAD:{branch_ref}");
        // 完整 refspec + 禁用 mirror/followTags，避免用户默认 push 配置扩大范围。
        let mirror = format!("remote.{remote}.mirror=false");
        let pushed = self.git_output(&[
            "-c",
            &mirror,
            "push",
            "--quiet",
            "--no-follow-tags",
            "--recurse-submodules=no",
            "--",
            remote,
            &refspec,
        ])?;
        if !pushed.status.success() {
            return Err("普通 Git 推送未成功，可能远端又有更新或连接/权限失败；所有本地提交均保留。请重试 sync 以重新 fetch/整合，程序不会强推".into());
        }
        Ok(
            json!({"remote":remote,"branch":branch,"before":before,"local_commit":local,"head":after.trim(),"integration":integration,"pushed":true}),
        )
    }

    fn integrate(&self, local: &str, incoming: &str) -> Result<&'static str> {
        if local == incoming {
            return Ok("already_equal");
        }
        let base = self
            .git_optional(&["merge-base", local, incoming])?
            .ok_or("本机与远端没有共同祖先；已保留双方历史，请人工确认仓库来源")?;
        self.ensure_immutable_events(&base, Some(local))?;
        self.ensure_immutable_events(&base, Some(incoming))?;
        if base == incoming {
            return Ok("local_ahead");
        }
        if base == local {
            self.git_ok(
                &["merge", "--ff-only", "--no-edit", "--no-gpg-sign", incoming],
                "快进整合远端",
            )?;
            return Ok("fast_forward");
        }
        let local_paths = self.git_paths(&[
            "diff",
            "--name-only",
            "--no-renames",
            "-z",
            &base,
            local,
            "--",
        ])?;
        let remote_paths = self.git_paths(&[
            "diff",
            "--name-only",
            "--no-renames",
            "-z",
            &base,
            incoming,
            "--",
        ])?;
        let overlap: Vec<_> = local_paths
            .intersection(&remote_paths)
            .filter(|path| path.starts_with("memories/") || path.starts_with("control/"))
            .collect();
        if !overlap.is_empty() {
            return Err(format!("双方修改了相同 Memory/控制记录，必须人工审查，未自动合并或推送：{}；本地提交 {local}，远端提交 {incoming}", overlap.into_iter().cloned().collect::<Vec<_>>().join("、")));
        }
        self.ensure_git_identity()?;
        let merge = self.git_output(&[
            "merge",
            "--no-ff",
            "--no-commit",
            "--no-edit",
            "--no-gpg-sign",
            incoming,
        ])?;
        if !merge.status.success() {
            return Err("Git 整合未完成；工作区、冲突文件与双方提交均保留。请人工检查 git status，解决或中止合并后重试 sync".into());
        }
        self.validate_sync_canonical().map_err(|error| {
            format!("合并结果未通过正本校验，已保留待审工作区，未提交或推送：{error}")
        })?;
        self.check_index_paths()?;
        self.git_ok(
            &[
                "commit",
                "--quiet",
                "--no-gpg-sign",
                "-m",
                "整合 RecallCard 双端已完成事务",
            ],
            "保存合并提交",
        )?;
        Ok("merge")
    }

    fn validate_sync_canonical(&self) -> Result<()> {
        let marker: Value = read_json(&self.root().join("control/schema-version.json"))?;
        if marker["schema_version"] != SCHEMA_VERSION || marker["application"] != "RecallCard" {
            return Err("Vault schema 标记不匹配".into());
        }
        self.doctor()?;
        self.suppressed_ids()?;
        let mut canonical = canonical_files(self.root())?;
        canonical.push(".gitignore".into());
        self.check_safe_attributes(&canonical)?;
        for path in canonical {
            reject_symlink(&self.root().join(&path))?;
            if path.starts_with("objects/") {
                let bytes = fs::read(self.root().join(&path)).map_err(|e| e.to_string())?;
                if path != format!("objects/{}", hash(&bytes)) {
                    return Err(format!("内容寻址对象摘要不匹配：{path}"));
                }
            } else if path.starts_with("control/dream-receipts/") {
                let receipt: DreamReceipt = read_json(&self.root().join(&path))?;
                if receipt.schema_version != SCHEMA_VERSION
                    || receipt.job_id != format!("dream_{}", receipt.input_hash)
                    || receipt.already_applied
                    || path != format!("control/dream-receipts/{}.json", receipt.job_id)
                    || !hex_id(&receipt.input_hash, "", 64)
                    || !hex_id(&receipt.result_hash, "", 64)
                {
                    return Err(format!("Dream 收据结构无效：{path}"));
                }
                for id in &receipt.source_coverage {
                    self.event(id.strip_prefix("event:").ok_or("Dream 收据来源引用无效")?)?;
                }
                for change in &receipt.changes {
                    validate_id(&change.id, "mem_")?;
                    if change.revision == 0 || !hex_id(&change.content_hash, "", 64) {
                        return Err(format!("Dream 收据变更无效：{path}"));
                    }
                }
            } else if path.starts_with("control/suppressions/") {
                let suppression: Suppression = read_json(&self.root().join(&path))?;
                if path != format!("control/suppressions/{}.json", suppression.id)
                    || suppression.schema_version != SCHEMA_VERSION
                    || suppression.reason.trim().is_empty()
                {
                    return Err(format!("抑制记录结构无效：{path}"));
                }
                for id in suppression.source_refs {
                    self.event(&id)?;
                }
            }
        }
        Ok(())
    }

    fn ensure_immutable_events(&self, base: &str, target: Option<&str>) -> Result<()> {
        let mut args = vec![
            "diff",
            "--name-only",
            "--no-renames",
            "--diff-filter=MDT",
            "-z",
            base,
        ];
        if let Some(target) = target {
            args.push(target);
        }
        args.extend(["--", "events", "objects", "control/dream-receipts"]);
        if !self.git_paths(&args)?.is_empty() {
            return Err("已经提交的 Event、对象或 Dream 收据被修改/删除；这些正本只追加，请先人工检查历史，未同步".into());
        }
        Ok(())
    }

    fn check_tree(&self, revision: &str) -> Result<()> {
        let output = self.git_ok(&["ls-tree", "-r", "-z", revision], "检查提交树")?;
        let mut paths = Vec::new();
        for entry in nul_strings(&output.stdout)? {
            let (metadata, path) = entry.split_once('\t').ok_or("Git 提交树格式无效")?;
            if !metadata.starts_with("100644 blob ") || !approved_path(path) {
                return Err(format!(
                    "Git 历史当前树包含未获准同步的路径/文件类型：{path}"
                ));
            }
            paths.push(path.to_owned());
        }
        self.check_safe_attributes(&paths)
    }

    fn check_safe_attributes(&self, paths: &[String]) -> Result<()> {
        for batch in paths.chunks(128) {
            let mut args = vec![
                "check-attr",
                "-z",
                "filter",
                "working-tree-encoding",
                "merge",
                "--",
            ];
            args.extend(batch.iter().map(String::as_str));
            let output = self.git_ok(&args, "检查 Git 文件转换规则")?;
            let values = nul_strings(&output.stdout)?;
            let (attributes, remainder) = values.as_chunks::<3>();
            if !remainder.is_empty() {
                return Err("Git 属性响应格式无效".into());
            }
            for value in attributes {
                let safe = matches!(value[2].as_str(), "unspecified" | "unset")
                    || (value[1] == "merge" && matches!(value[2].as_str(), "text" | "binary"));
                if !safe {
                    return Err(format!("正本启用了不安全/非标准 Git filter、编码转换或合并驱动：{}；请先人工检查 attributes 配置", value[0]));
                }
            }
        }
        Ok(())
    }

    fn check_index_paths(&self) -> Result<()> {
        let flags = self.git_ok(&["ls-files", "-v", "-z"], "检查索引隐藏标记")?;
        for entry in nul_strings(&flags.stdout)? {
            if entry.starts_with("S ")
                || entry.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
            {
                return Err("Git 索引存在 skip-worktree/assume-unchanged 标记；请先恢复完整工作树，不能在稀疏或隐藏变化状态下同步正本".into());
            }
        }
        let output = self.git_ok(&["ls-files", "--stage", "-z"], "检查暂存路径")?;
        for entry in nul_strings(&output.stdout)? {
            let (metadata, path) = entry.split_once('\t').ok_or("Git 暂存格式无效")?;
            if !metadata.starts_with("100644 ") || !metadata.ends_with(" 0") || !approved_path(path)
            {
                return Err(format!("暂存区/已跟踪文件含未获准同步的路径、类型或冲突：{path}；请人工检查，程序未清理暂存区"));
            }
        }
        Ok(())
    }

    fn changed_paths(&self) -> Result<BTreeSet<String>> {
        let mut paths = self.git_paths(&[
            "ls-files",
            "--modified",
            "--deleted",
            "--others",
            "--exclude-standard",
            "-z",
        ])?;
        paths.extend(self.git_paths(&["diff", "--cached", "--name-only", "-z"])?);
        Ok(paths)
    }

    fn ensure_git_identity(&self) -> Result<()> {
        for key in ["user.name", "user.email"] {
            let value = self.git_optional(&["config", "--get", key])?;
            if value.as_deref().is_none_or(|v| v.trim().is_empty()) {
                return Err("Git 提交身份未完整配置；请自行为此 Vault 设置 user.name 与 user.email（可使用 GitHub noreply 地址），程序不会猜测或代填私人邮箱".into());
            }
        }
        self.git_ok(&["var", "GIT_AUTHOR_IDENT"], "验证 Git 提交身份")?;
        Ok(())
    }

    fn check_git_root(&self) -> Result<()> {
        reject_symlink(&self.root().join(".git"))?;
        let top = self.git_text(&["rev-parse", "--show-toplevel"], "检查 Git 根目录")?;
        let top = fs::canonicalize(top.trim()).map_err(|e| format!("无法确认 Git 根目录：{e}"))?;
        if top != self.root() {
            return Err("Vault 必须恰好是独立 Git 仓库根目录，不能借用父目录仓库".into());
        }
        Ok(())
    }

    fn ongoing_operations(&self) -> Result<Vec<String>> {
        let mut operations = Vec::new();
        for marker in [
            "MERGE_HEAD",
            "rebase-merge",
            "rebase-apply",
            "CHERRY_PICK_HEAD",
            "REVERT_HEAD",
            "sequencer",
            "BISECT_LOG",
            "index.lock",
        ] {
            let path =
                self.git_text(&["rev-parse", "--git-path", marker], "检查未完成 Git 操作")?;
            if self.root().join(path.trim()).exists() {
                operations.push(marker.to_owned());
            }
        }
        Ok(operations)
    }

    fn ensure_git_idle(&self) -> Result<()> {
        let operations = self.ongoing_operations()?;
        let conflicts = self.git_paths(&["diff", "--name-only", "--diff-filter=U", "-z"])?;
        if !operations.is_empty() || !conflicts.is_empty() {
            return Err("已有未完成的 Git 合并、变基、拣选、二分或冲突/锁；请先人工处理，sync 不会清理或覆盖它们".into());
        }
        Ok(())
    }

    fn git_paths(&self, args: &[&str]) -> Result<BTreeSet<String>> {
        let output = self.git_ok(args, "读取 Git 路径")?;
        Ok(nul_strings(&output.stdout)?.into_iter().collect())
    }

    fn git_optional(&self, args: &[&str]) -> Result<Option<String>> {
        let output = self.git_output(args)?;
        if output.status.success() {
            Ok(Some(
                String::from_utf8(output.stdout)
                    .map_err(|_| "Git 返回非 UTF-8 文本")?
                    .trim()
                    .to_owned(),
            ))
        } else if matches!(output.status.code(), Some(1 | 128)) {
            Ok(None)
        } else {
            Err("无法读取 Git 状态".into())
        }
    }

    fn git_text(&self, args: &[&str], operation: &str) -> Result<String> {
        String::from_utf8(self.git_ok(args, operation)?.stdout)
            .map_err(|_| "Git 返回非 UTF-8 文本".into())
    }

    fn git_ok(&self, args: &[&str], operation: &str) -> Result<Output> {
        let output = self.git_output(args)?;
        if output.status.success() {
            Ok(output)
        } else {
            // 不把可能带 token/密码的远端 URL 或原始 Git stderr 回显出去。
            Err(format!(
                "{operation}失败（Git 退出码 {:?}）；现有文件与提交已保留，请检查本机 Git 状态",
                output.status.code()
            ))
        }
    }

    fn git_output(&self, args: &[&str]) -> Result<Output> {
        let hooks = tempfile::tempdir().map_err(|e| format!("无法创建禁用 hooks 的空目录：{e}"))?;
        let mut command = Command::new("git");
        command
            .arg("--literal-pathspecs")
            .args(["-C"])
            .arg(self.root())
            .args([
                "-c",
                "user.useConfigOnly=true",
                "-c",
                "core.autocrlf=false",
                "-c",
                "commit.gpgSign=false",
                "-c",
                "merge.gpgSign=false",
                "-c",
                "core.fsmonitor=false",
                "-c",
                "maintenance.auto=false",
                "-c",
                "gc.auto=0",
                "-c",
                "protocol.ext.allow=never",
                "-c",
                "fetch.fsckObjects=true",
            ])
            .arg("-c")
            .arg(format!("core.hooksPath={}", hooks.path().display()))
            .args(args)
            .stdin(Stdio::null())
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_MERGE_AUTOEDIT", "no")
            .env("GIT_NO_REPLACE_OBJECTS", "1");
        for key in [
            "GIT_DIR",
            "GIT_WORK_TREE",
            "GIT_COMMON_DIR",
            "GIT_INDEX_FILE",
            "GIT_OBJECT_DIRECTORY",
            "GIT_ALTERNATE_OBJECT_DIRECTORIES",
            "GIT_NAMESPACE",
            "GIT_CONFIG",
            "GIT_CONFIG_COUNT",
            "GIT_CONFIG_PARAMETERS",
            "GIT_AUTHOR_NAME",
            "GIT_AUTHOR_EMAIL",
            "GIT_AUTHOR_DATE",
            "GIT_COMMITTER_NAME",
            "GIT_COMMITTER_EMAIL",
            "GIT_COMMITTER_DATE",
        ] {
            command.env_remove(key);
        }
        command
            .output()
            .map_err(|e| format!("无法运行 Git，请检查安装：{e}"))
    }
}

fn validate_remote(remote: &str) -> Result<()> {
    if remote.is_empty()
        || remote.starts_with('-')
        || remote.len() > 128
        || !remote
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
    {
        return Err(
            "必须显式指定已配置的远端名称，只接受字母、数字、点、下划线与连字符；不能传 URL 或命令"
                .into(),
        );
    }
    Ok(())
}

fn nul_strings(bytes: &[u8]) -> Result<Vec<String>> {
    bytes
        .split(|b| *b == 0)
        .filter(|v| !v.is_empty())
        .map(|v| {
            String::from_utf8(v.to_vec())
                .map_err(|_| "Git 路径不是 UTF-8；请先人工整理文件名".into())
        })
        .collect()
}

fn hex_id(value: &str, prefix: &str, size: usize) -> bool {
    value.strip_prefix(prefix).is_some_and(|rest| {
        rest.len() == size
            && rest
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}

fn approved_path(path: &str) -> bool {
    if path == ".gitignore" || path == "control/schema-version.json" {
        return true;
    }
    let parts: Vec<_> = path.split('/').collect();
    match parts.as_slice() {
        ["events", year, month, session, name] => {
            year.len() == 4
                && year.bytes().all(|b| b.is_ascii_digit())
                && month.len() == 2
                && month.parse::<u8>().is_ok_and(|m| (1..=12).contains(&m))
                && hex_id(session, "", 24)
                && name
                    .strip_suffix(".jsonl")
                    .is_some_and(|id| hex_id(id, "evt_", 64))
        }
        ["memories", name] => name
            .strip_suffix(".md")
            .is_some_and(|id| hex_id(id, "mem_", 32)),
        ["objects", name] => hex_id(name, "", 64),
        ["control", "dream-receipts", name] => name
            .strip_suffix(".json")
            .is_some_and(|id| hex_id(id, "dream_", 64)),
        ["control", "suppressions", name] => name
            .strip_suffix(".json")
            .is_some_and(|id| hex_id(id, "evt_", 64) || hex_id(id, "mem_", 32)),
        _ => false,
    }
}

fn canonical_files(root: &Path) -> Result<Vec<String>> {
    fn visit(root: &Path, path: &Path, result: &mut Vec<String>) -> Result<()> {
        reject_symlink(path)?;
        for entry in fs::read_dir(path).map_err(|e| e.to_string())? {
            let path = entry.map_err(|e| e.to_string())?.path();
            reject_symlink(&path)?;
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .ok_or("正本文件名不是 UTF-8")?;
            // 本机隐藏旁文件不作为正本，不暂存；已跟踪的旁文件另由 index 校验拒绝。
            if name.starts_with('.') {
                continue;
            }
            if path.is_dir() {
                visit(root, &path, result)?;
            } else {
                let relative = path
                    .strip_prefix(root)
                    .map_err(|e| e.to_string())?
                    .components()
                    .map(|component| component.as_os_str().to_str().ok_or("正本路径不是 UTF-8"))
                    .collect::<std::result::Result<Vec<_>, _>>()?
                    .join("/");
                if !path.is_file() || !approved_path(&relative) {
                    return Err(format!("正本目录包含不支持的路径/类型：{relative}"));
                }
                result.push(relative);
            }
        }
        Ok(())
    }
    let mut result = Vec::new();
    for directory in ["events", "memories", "objects", "control"] {
        visit(root, &root.join(directory), &mut result)?;
    }
    Ok(result)
}
