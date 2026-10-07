//! 同步验收全部使用临时本地 bare 仓库与合成记录；不连接网络或用户资料库。
use recallcard::{Event, EventInput, Evidence, Memory, MemoryInput, Origin, Role, Vault};
use serde_json::json;
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
use tempfile::{tempdir, TempDir};

fn git_output(root: &Path, args: &[&str]) -> Output {
    Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .expect("测试需要系统 Git")
}
fn git(root: &Path, args: &[&str]) -> String {
    let out = git_output(root, args);
    assert!(
        out.status.success(),
        "Git {args:?} 失败：{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap().trim().to_owned()
}
fn identity(root: &Path) {
    git(root, &["config", "user.name", "RecallCard 合成测试"]);
    git(
        root,
        &["config", "user.email", "recallcard-test@example.invalid"],
    );
    git(root, &["config", "commit.gpgSign", "false"]);
}
fn capture(vault: &Vault, name: &str) -> Event {
    let input: EventInput = serde_json::from_value(json!({
        "occurred_at":"2026-10-06T12:00:00Z","role":Role::User,"origin":Origin::Native,
        "content":format!("合成同步测试：{name}"),
        "source":{"platform":"合成平台","conversation_id":"同步测试","message_id":name}
    }))
    .unwrap();
    vault.capture(input).unwrap()
}
fn memory(vault: &Vault, event: &Event) -> Memory {
    let input: MemoryInput = serde_json::from_value(json!({
        "content":"合成偏好：中文说明","source_refs":[event.id],
        "evidence":Evidence::UserExplicit,"confidence":0.9,"tags":["合成同步"]
    }))
    .unwrap();
    vault.add_memory(input).unwrap()
}
struct Pair {
    _dir: TempDir,
    a: Vault,
    b: Vault,
    remote: std::path::PathBuf,
}
fn pair() -> Pair {
    let dir = tempdir().unwrap();
    let remote = dir.path().join("远端.git");
    git(
        dir.path(),
        &[
            "init",
            "--quiet",
            "--bare",
            "--initial-branch=main",
            remote.to_str().unwrap(),
        ],
    );
    let a = Vault::init(&dir.path().join("甲端")).unwrap();
    git(a.root(), &["symbolic-ref", "HEAD", "refs/heads/main"]);
    identity(a.root());
    git(
        a.root(),
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );
    a.sync("origin").unwrap();
    let b_root = dir.path().join("乙端");
    git(
        dir.path(),
        &[
            "clone",
            "--quiet",
            remote.to_str().unwrap(),
            b_root.to_str().unwrap(),
        ],
    );
    identity(&b_root);
    let b = Vault::open(&b_root).unwrap();
    Pair {
        _dir: dir,
        a,
        b,
        remote,
    }
}
fn head(vault: &Vault) -> String {
    git(vault.root(), &["rev-parse", "HEAD"])
}
fn remote_head(pair: &Pair) -> String {
    git(&pair.remote, &["rev-parse", "refs/heads/main"])
}

#[test]
fn two_devices_commit_before_integrating_and_keep_both_histories() {
    let pair = pair();
    let a = capture(&pair.a, "甲离线新增");
    let b = capture(&pair.b, "乙离线新增");
    pair.a.sync("origin").unwrap();
    let a_head = head(&pair.a);
    let result = pair.b.sync("origin").unwrap();
    assert_eq!(result["git"]["integration"], "merge");
    assert!(pair.b.event(&a.id).is_ok());
    assert!(pair.b.event(&b.id).is_ok());
    let parents = git(pair.b.root(), &["show", "-s", "--format=%P", "HEAD"]);
    assert_eq!(parents.split_whitespace().count(), 2);
    assert!(parents.split_whitespace().any(|parent| parent == a_head));
    let local = result["git"]["local_commit"].as_str().unwrap();
    assert!(parents.split_whitespace().any(|parent| parent == local));
    assert_eq!(remote_head(&pair), head(&pair.b));
    assert_eq!(
        pair.a.sync("origin").unwrap()["git"]["integration"],
        "fast_forward"
    );
    assert!(pair.a.event(&b.id).is_ok());
    assert_eq!(
        pair.a.sync("origin").unwrap()["git"]["integration"],
        "already_equal"
    );
}

#[test]
fn a_fresh_clone_recreates_derived_files_after_sync() {
    let pair = pair();
    assert!(!pair.b.root().join("generated").exists());
    assert!(!pair.b.root().join(".index").exists());
    pair.b.sync("origin").unwrap();
    assert!(pair.b.root().join("generated/views/memories.md").is_file());
    assert!(pair.b.root().join(".index/text.json").is_file());
    assert!(git(pair.b.root(), &["ls-files", "generated", ".index"]).is_empty());
}

#[test]
fn sync_never_stages_secrets_hidden_files_or_derived_output() {
    let pair = pair();
    fs::write(
        pair.a.root().join(".env"),
        "SYNTHETIC_TOKEN=not-a-real-token\n",
    )
    .unwrap();
    fs::write(pair.a.root().join("private-notes.txt"), "合成本机旁文件").unwrap();
    fs::write(
        pair.a.root().join("events/.private.jsonl"),
        "合成隐藏旁文件",
    )
    .unwrap();
    fs::write(
        pair.a.root().join("generated/local-secret.txt"),
        "合成派生内容",
    )
    .unwrap();
    let event = capture(&pair.a, "只有正本应被提交");
    pair.a.sync("origin").unwrap();
    let tracked = git(pair.a.root(), &["ls-files"]);
    assert!(tracked.contains(&event.id));
    for forbidden in [".env", "private-notes", ".private", "generated/", ".index/"] {
        assert!(!tracked.contains(forbidden), "不应跟踪 {forbidden}");
    }
    assert!(pair.a.root().join("private-notes.txt").exists());
}

#[test]
fn unrelated_staged_files_are_left_untouched_and_block_sync() {
    let pair = pair();
    fs::write(pair.a.root().join("private.txt"), "合成秘密").unwrap();
    git(pair.a.root(), &["add", "private.txt"]);
    let before = head(&pair.a);
    assert!(pair.a.sync("origin").unwrap_err().contains("未获准"));
    assert_eq!(head(&pair.a), before);
    assert_eq!(
        git(pair.a.root(), &["diff", "--cached", "--name-only"]),
        "private.txt"
    );
}

#[test]
fn missing_identity_is_reported_before_staging_local_records() {
    let pair = pair();
    capture(&pair.a, "身份尚未配置");
    git(pair.a.root(), &["config", "user.name", ""]);
    git(pair.a.root(), &["config", "user.email", ""]);
    let before = head(&pair.a);
    assert!(pair.a.sync("origin").unwrap_err().contains("身份"));
    assert_eq!(head(&pair.a), before);
    assert!(git(pair.a.root(), &["diff", "--cached", "--name-only"]).is_empty());
    assert_eq!(git(pair.a.root(), &["config", "--get", "user.email"]), "");
}

#[test]
fn pending_dream_transaction_blocks_all_git_changes() {
    let pair = pair();
    capture(&pair.a, "等待 Dream 恢复");
    let marker = pair.a.state_dir().unwrap().join("dream-transaction.json");
    fs::write(&marker, "{\"synthetic_pending\":true}\n").unwrap();
    let before = head(&pair.a);
    let error = pair.a.sync("origin").unwrap_err();
    fs::remove_file(marker).unwrap();
    assert!(error.contains("dream recover"));
    assert_eq!(head(&pair.a), before);
    assert!(git(pair.a.root(), &["diff", "--cached", "--name-only"]).is_empty());
}

#[test]
fn active_writer_lock_blocks_sync() {
    let pair = pair();
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(pair.a.state_dir().unwrap().join("write.lock"))
        .unwrap();
    lock.try_lock().unwrap();
    assert!(pair.a.sync("origin").unwrap_err().contains("另一个写入"));
    lock.unlock().unwrap();
}

#[test]
fn simultaneous_memory_edits_require_semantic_review_even_when_text_can_merge() {
    let pair = pair();
    let event = capture(&pair.a, "双端记忆更新");
    let original = memory(&pair.a, &event);
    pair.a.sync("origin").unwrap();
    pair.b.sync("origin").unwrap();
    let mut a_input = original.data.clone();
    a_input.content = "甲端合成偏好：更详细的中文说明".into();
    pair.a.update_memory(&original.id, 1, a_input).unwrap();
    let mut b_input = original.data.clone();
    b_input.tags.push("乙端新增标签".into());
    pair.b.update_memory(&original.id, 1, b_input).unwrap();
    pair.a.sync("origin").unwrap();
    let remote_before = remote_head(&pair);
    let error = pair.b.sync("origin").unwrap_err();
    assert!(error.contains("必须人工审查"));
    assert!(pair
        .b
        .memory(&original.id)
        .unwrap()
        .data
        .tags
        .contains(&"乙端新增标签".to_owned()));
    assert_eq!(remote_head(&pair), remote_before);
    assert!(!pair.b.root().join(".git/MERGE_HEAD").exists());
    assert!(git(pair.b.root(), &["status", "--porcelain"]).is_empty());
}

#[test]
fn real_text_conflicts_remain_for_manual_resolution_and_repeat_sync_refuses() {
    let pair = pair();
    fs::write(
        pair.a.root().join(".gitignore"),
        "# 甲端合成注释\ngenerated/\n.index/\n.env\n",
    )
    .unwrap();
    fs::write(
        pair.b.root().join(".gitignore"),
        "# 乙端合成注释\ngenerated/\n.index/\n.env\n",
    )
    .unwrap();
    pair.a.sync("origin").unwrap();
    let remote_before = remote_head(&pair);
    assert!(pair.b.sync("origin").unwrap_err().contains("冲突"));
    assert!(pair.b.root().join(".git/MERGE_HEAD").exists());
    let conflicted = fs::read(pair.b.root().join(".gitignore")).unwrap();
    assert!(String::from_utf8_lossy(&conflicted).contains("<<<<<<<"));
    assert!(pair.b.sync("origin").unwrap_err().contains("已有未完成"));
    assert_eq!(
        fs::read(pair.b.root().join(".gitignore")).unwrap(),
        conflicted
    );
    assert_eq!(remote_head(&pair), remote_before);
    let status = pair.b.git_status().unwrap();
    assert_eq!(status["conflicts"][0], ".gitignore");
}

#[test]
fn an_existing_rebase_is_not_aborted_or_overwritten() {
    let pair = pair();
    let path = pair.a.root().join(".git/rebase-merge");
    fs::create_dir(&path).unwrap();
    fs::write(path.join("synthetic-state"), "保留此状态").unwrap();
    assert!(pair.a.sync("origin").unwrap_err().contains("已有未完成"));
    assert_eq!(
        fs::read_to_string(path.join("synthetic-state")).unwrap(),
        "保留此状态"
    );
}

#[test]
fn repository_root_must_match_the_vault_exactly() {
    let pair = pair();
    let nested = Vault::init(&pair.a.root().join("嵌套资料库")).unwrap();
    fs::remove_dir_all(nested.root().join(".git")).unwrap();
    assert!(nested
        .sync("origin")
        .unwrap_err()
        .contains("独立 Git 仓库根目录"));
}

#[test]
fn remote_name_must_be_explicit_configured_and_not_an_option_or_url() {
    let pair = pair();
    for bad in [
        "",
        "--all",
        "file:///tmp/other",
        "origin;echo",
        "origin\nother",
    ] {
        assert!(pair.a.sync(bad).is_err());
    }
    assert!(pair
        .a
        .sync("unconfigured")
        .unwrap_err()
        .contains("尚未配置"));
}

#[test]
fn invalid_incoming_tree_is_rejected_before_checkout() {
    let pair = pair();
    fs::write(pair.a.root().join("forbidden.txt"), "合成未批准文件").unwrap();
    git(pair.a.root(), &["add", "forbidden.txt"]);
    git(pair.a.root(), &["commit", "--quiet", "-m", "合成坏提交"]);
    git(
        pair.a.root(),
        &["push", "--quiet", "origin", "HEAD:refs/heads/main"],
    );
    let before = head(&pair.b);
    assert!(pair.b.sync("origin").unwrap_err().contains("未获准"));
    assert_eq!(head(&pair.b), before);
    assert!(!pair.b.root().join("forbidden.txt").exists());
}

#[test]
fn ignored_canonical_records_cannot_be_silently_dropped() {
    let pair = pair();
    capture(&pair.a, "不允许漏传");
    let ignore = pair.a.root().join(".gitignore");
    let mut text = fs::read_to_string(&ignore).unwrap();
    text.push_str("events/\n");
    fs::write(ignore, text).unwrap();
    assert!(pair.a.sync("origin").unwrap_err().contains("被 Git 忽略"));
}

#[test]
fn completed_events_are_append_only_even_if_a_deletion_would_parse() {
    let pair = pair();
    let event = capture(&pair.a, "不可删除已提交证据");
    pair.a.sync("origin").unwrap();
    let path = git(pair.a.root(), &["ls-files"])
        .lines()
        .find(|p| p.contains(&event.id))
        .unwrap()
        .to_owned();
    fs::remove_file(pair.a.root().join(path)).unwrap();
    assert!(pair.a.sync("origin").unwrap_err().contains("只追加"));
}

#[cfg(unix)]
#[test]
fn client_hooks_are_never_run_by_program_invoked_git() {
    use std::os::unix::fs::PermissionsExt;
    let pair = pair();
    let marker = pair.a.root().join("hook-executed");
    for hook in ["pre-commit", "post-commit", "pre-push"] {
        let path = pair.a.root().join(".git/hooks").join(hook);
        fs::write(
            &path,
            format!(
                "#!/bin/sh\nprintf executed > '{}'\nexit 1\n",
                marker.display()
            ),
        )
        .unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    }
    capture(&pair.a, "禁用任意 hooks");
    pair.a.sync("origin").unwrap();
    assert!(!marker.exists());
}

#[cfg(unix)]
#[test]
fn rejected_push_preserves_local_commit_without_force_or_reset() {
    use std::os::unix::fs::PermissionsExt;
    let pair = pair();
    let hook = pair.remote.join("hooks/pre-receive");
    fs::write(&hook, "#!/bin/sh\nexit 1\n").unwrap();
    fs::set_permissions(hook, fs::Permissions::from_mode(0o700)).unwrap();
    let remote_before = remote_head(&pair);
    let event = capture(&pair.a, "远端拒绝推送");
    assert!(pair.a.sync("origin").unwrap_err().contains("不会强推"));
    assert_ne!(head(&pair.a), remote_before);
    assert_eq!(remote_head(&pair), remote_before);
    assert!(pair.a.event(&event.id).is_ok());
    assert!(git(pair.a.root(), &["status", "--porcelain"]).is_empty());
}

#[test]
fn completed_dream_receipts_and_suppressions_sync_as_canonical_metadata() {
    use recallcard::dream::DreamResult;
    let pair = pair();
    let event = capture(&pair.a, "含收据的已完成事务");
    let job = pair
        .a
        .dream_export(std::slice::from_ref(&event.id), &[], "personal")
        .unwrap();
    let result: DreamResult = serde_json::from_value(json!({
        "schema":"recallcard.dream-result/1","job_id":job.job_id,"input_hash":job.input_hash,
        "proposals":[{"operation":"add","scope":"personal","content":"合成偏好：完整测试同步",
        "source_refs":[format!("event:{}", event.id)],"evidence":"user_explicit"}]
    }))
    .unwrap();
    let review = pair.a.dream_review(&result).unwrap();
    let receipt = pair
        .a
        .dream_apply(&result, &review.result_hash, false)
        .unwrap();
    pair.a.suppress(&event.id, "合成撤回测试".into()).unwrap();
    pair.a.sync("origin").unwrap();
    pair.b.sync("origin").unwrap();
    assert!(pair.b.memory(&receipt.changes[0].id).is_ok());
    assert!(pair.b.is_suppressed(&event.id).unwrap());
    assert!(pair
        .b
        .root()
        .join("control/dream-receipts")
        .join(format!("{}.json", job.job_id))
        .exists());
    assert!(!git(pair.b.root(), &["ls-files"]).contains("dream-jobs"));
}

#[test]
fn incoming_canonical_corruption_is_reported_after_integration_without_cleanup() {
    let pair = pair();
    let path = format!("memories/mem_{}.md", "a".repeat(32));
    fs::write(
        pair.a.root().join(&path),
        "损坏的合成 Memory，不含合法 frontmatter\n",
    )
    .unwrap();
    git(pair.a.root(), &["add", "--", &path]);
    git(pair.a.root(), &["commit", "--quiet", "-m", "合成损坏记录"]);
    git(
        pair.a.root(),
        &["push", "--quiet", "origin", "HEAD:refs/heads/main"],
    );
    let incoming = remote_head(&pair);
    assert!(pair
        .b
        .sync("origin")
        .unwrap_err()
        .contains("整合后的正本校验失败"));
    assert_eq!(head(&pair.b), incoming);
    assert_eq!(remote_head(&pair), incoming);
    assert!(pair.b.root().join(path).exists());
    assert!(pair.b.doctor().is_err());
}

#[test]
fn fetch_failure_keeps_a_completed_local_commit() {
    let pair = pair();
    let before = head(&pair.a);
    let event = capture(&pair.a, "离线本机提交");
    git(
        pair.a.root(),
        &[
            "remote",
            "set-url",
            "origin",
            pair._dir.path().join("不存在的远端.git").to_str().unwrap(),
        ],
    );
    assert!(pair.a.sync("origin").unwrap_err().contains("本地提交"));
    assert_ne!(head(&pair.a), before);
    assert!(pair.a.event(&event.id).is_ok());
    assert!(git(pair.a.root(), &["status", "--porcelain"]).is_empty());
}

#[test]
fn mirror_and_follow_tags_configuration_does_not_expand_the_push() {
    let pair = pair();
    git(pair.a.root(), &["config", "remote.origin.mirror", "true"]);
    git(pair.a.root(), &["config", "push.followTags", "true"]);
    git(pair.a.root(), &["branch", "unrelated-local-branch"]);
    git(
        pair.a.root(),
        &["tag", "-a", "synthetic-tag", "-m", "合成标签"],
    );
    capture(&pair.a, "只推明确分支");
    pair.a.sync("origin").unwrap();
    assert!(!git_output(
        &pair.remote,
        &["show-ref", "--verify", "refs/heads/unrelated-local-branch"]
    )
    .status
    .success());
    assert!(!git_output(
        &pair.remote,
        &["show-ref", "--verify", "refs/tags/synthetic-tag"]
    )
    .status
    .success());
}

#[test]
fn content_addressed_objects_must_match_their_names() {
    let pair = pair();
    let bytes = b"synthetic object bytes";
    let hash = recallcard::model::hash(bytes);
    fs::write(pair.a.root().join("objects").join(&hash), bytes).unwrap();
    pair.a.sync("origin").unwrap();
    pair.b.sync("origin").unwrap();
    assert_eq!(
        fs::read(pair.b.root().join("objects").join(&hash)).unwrap(),
        bytes
    );
    fs::write(pair.a.root().join("objects").join("a".repeat(64)), bytes).unwrap();
    assert!(pair.a.sync("origin").unwrap_err().contains("摘要不匹配"));
}

#[cfg(unix)]
#[test]
fn a_custom_clean_filter_is_rejected_without_execution() {
    let pair = pair();
    let marker = pair.a.root().join("filter-executed");
    git(
        pair.a.root(),
        &[
            "config",
            "filter.synthetic.clean",
            &format!("touch '{}'", marker.display()),
        ],
    );
    fs::write(
        pair.a.root().join(".git/info/attributes"),
        "events/** filter=synthetic\n",
    )
    .unwrap();
    capture(&pair.a, "不能运行过滤器");
    assert!(pair.a.sync("origin").unwrap_err().contains("filter"));
    assert!(!marker.exists());
}

#[test]
fn sparse_or_assume_unchanged_index_entries_cannot_hide_records_from_validation() {
    for flag in ["--skip-worktree", "--assume-unchanged"] {
        let pair = pair();
        git(
            pair.a.root(),
            &["update-index", flag, "control/schema-version.json"],
        );
        assert!(pair.a.sync("origin").unwrap_err().contains("隐藏变化"));
    }
}

#[test]
fn unrelated_remote_history_is_never_forced_or_replaced() {
    let pair = pair();
    let other_root = pair._dir.path().join("另一资料库");
    let other_remote = pair._dir.path().join("另一远端.git");
    git(
        pair._dir.path(),
        &[
            "init",
            "--quiet",
            "--bare",
            "--initial-branch=main",
            other_remote.to_str().unwrap(),
        ],
    );
    let other = Vault::init(&other_root).unwrap();
    git(other.root(), &["symbolic-ref", "HEAD", "refs/heads/main"]);
    identity(other.root());
    git(
        other.root(),
        &["remote", "add", "origin", other_remote.to_str().unwrap()],
    );
    capture(&other, "独立历史合成事件");
    other.sync("origin").unwrap();
    let other_before = git(&other_remote, &["rev-parse", "refs/heads/main"]);
    let before = head(&pair.a);
    git(
        pair.a.root(),
        &[
            "remote",
            "set-url",
            "origin",
            other_remote.to_str().unwrap(),
        ],
    );
    assert!(pair.a.sync("origin").unwrap_err().contains("没有共同祖先"));
    assert_eq!(head(&pair.a), before);
    assert_eq!(
        git(&other_remote, &["rev-parse", "refs/heads/main"]),
        other_before
    );
}

#[cfg(unix)]
#[test]
fn a_literal_backslash_in_a_canonical_directory_is_not_treated_as_a_separator() {
    let pair = pair();
    let event = capture(&pair.a, "路径分隔符安全");
    let event_path = pair
        .a
        .events()
        .unwrap()
        .into_iter()
        .find(|e| e.id == event.id)
        .unwrap();
    let session = recallcard::model::hash(event_path.data.session_key().as_bytes());
    let old_dir = pair.a.root().join("events/2026/10").join(&session[..24]);
    let strange_dir = pair.a.root().join("events/2026\\10").join(&session[..24]);
    fs::create_dir_all(&strange_dir).unwrap();
    fs::rename(
        old_dir.join(format!("{}.jsonl", event.id)),
        strange_dir.join(format!("{}.jsonl", event.id)),
    )
    .unwrap();
    assert!(pair.a.sync("origin").unwrap_err().contains("不支持的路径"));
}

#[test]
fn the_same_event_captured_offline_on_two_devices_preserves_both_timestamps_as_conflict() {
    let pair = pair();
    let a = capture(&pair.a, "两端重复离线导入同一条原话");
    let b = capture(&pair.b, "两端重复离线导入同一条原话");
    assert_eq!(a.id, b.id);
    assert_ne!(a.captured_at, b.captured_at);
    pair.a.sync("origin").unwrap();
    let remote_before = remote_head(&pair);
    let error = pair.b.sync("origin").unwrap_err();
    assert!(error.contains("冲突"));
    let status = pair.b.git_status().unwrap();
    let conflicted = status["conflicts"].as_array().unwrap();
    assert_eq!(conflicted.len(), 1);
    let path = conflicted[0].as_str().unwrap();
    assert!(path.ends_with(&format!("{}.jsonl", a.id)));
    let text = fs::read_to_string(pair.b.root().join(path)).unwrap();
    assert!(text.contains(&serde_json::to_string(&a).unwrap()));
    assert!(text.contains(&serde_json::to_string(&b).unwrap()));
    assert_eq!(remote_head(&pair), remote_before);
    assert!(pair.b.root().join(".git/MERGE_HEAD").exists());
    assert!(pair.b.sync("origin").unwrap_err().contains("已有未完成"));
}
