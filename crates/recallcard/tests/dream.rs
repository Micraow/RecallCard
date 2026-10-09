//! Manual Dream 的可重复执行、证据边界与中断恢复。所有内容均为合成数据。
use recallcard::{
    dream::{DreamJob, DreamResult},
    model::hash,
    vault::render_memory,
    Event, Memory, MemoryState, Vault,
};
use serde_json::{json, Value};
use std::fs;
use tempfile::{tempdir, TempDir};

fn vault() -> (TempDir, Vault) {
    let dir = tempdir().unwrap();
    let vault = Vault::init(&dir.path().join("合成资料库")).unwrap();
    (dir, vault)
}
fn event(vault: &Vault, label: &str, role: &str, scope: &str) -> Event {
    vault
        .capture(
            serde_json::from_value(json!({
                "occurred_at":null,"role":role,"origin":"native","scope":scope,
                "content":format!("合成对话：{label}"),
                "source":{"platform":"手动导出测试","conversation_id":"合成会话","message_id":label}
            }))
            .unwrap(),
        )
        .unwrap()
}
fn memory(vault: &Vault, event: &Event) -> Memory {
    vault.add_memory(serde_json::from_value(json!({
        "content":"合成旧偏好：说明使用中文","source_refs":[event.id],"evidence":"user_explicit","scope":event.data.scope
    })).unwrap()).unwrap()
}
fn result(job: &DreamJob, proposals: Vec<Value>) -> DreamResult {
    serde_json::from_value(json!({"schema":"recallcard.dream-result/1","job_id":job.job_id,"input_hash":job.input_hash,"proposals":proposals})).unwrap()
}
fn add(event: &Event) -> Value {
    json!({"operation":"add","scope":event.data.scope,"content":"合成新偏好：文档使用简洁中文", "source_refs":[format!("event:{}",event.id)],"evidence":"user_explicit"})
}
fn update(event: &Event, memory: &Memory, operation: &str) -> Value {
    let mut value = add(event);
    value["operation"] = operation.into();
    value["target_ref"] = format!("memory:{}@{}", memory.id, memory.revision).into();
    value["expected_revision"] = memory.revision.into();
    value
}
fn digest(value: &impl serde::Serialize) -> String {
    hash(&serde_json::to_vec(value).unwrap())
}

#[test]
fn manual_export_review_apply_is_offline_and_idempotent() {
    let (_dir, vault) = vault();
    let source = event(&vault, "人工流程", "user", "personal");
    let job = vault
        .dream_export(std::slice::from_ref(&source.id), &[], "personal")
        .unwrap();
    assert_eq!(
        job.job_id,
        vault
            .dream_export(std::slice::from_ref(&source.id), &[], "personal")
            .unwrap()
            .job_id
    );
    assert_eq!(job.source_refs[0].event, source);
    let result = result(&job, vec![add(&source)]);
    let review = vault.dream_review(&result).unwrap();
    assert!(review.can_apply);
    assert!(!review.requires_protected_approval);
    assert!(vault.memories().unwrap().is_empty());
    let receipt = vault
        .dream_apply(&result, &review.result_hash, false)
        .unwrap();
    assert!(!receipt.already_applied);
    assert_eq!(receipt.changes.len(), 1);
    let memory = vault.memory(&receipt.changes[0].id).unwrap();
    assert_eq!(memory.data.authority, "dream");
    assert!(!memory.data.protected);
    assert_eq!(memory.state, MemoryState::Active);
    assert_eq!(vault.sources(&memory.id).unwrap(), vec![source]);
    let again = vault
        .dream_apply(&result, &review.result_hash, false)
        .unwrap();
    assert!(again.already_applied);
    assert_eq!(again.changes[0].id, receipt.changes[0].id);
    assert_eq!(vault.memories().unwrap().len(), 1);
    assert!(vault.dream_review(&result).unwrap().already_applied);
    assert!(vault
        .root()
        .join("control/dream-receipts")
        .join(format!("{}.json", job.job_id))
        .exists());
}

#[test]
fn approval_hash_is_bound_to_the_exact_result() {
    let (_dir, vault) = vault();
    let source = event(&vault, "摘要批准", "user", "personal");
    let job = vault
        .dream_export(std::slice::from_ref(&source.id), &[], "personal")
        .unwrap();
    let mut result = result(&job, vec![add(&source)]);
    let review = vault.dream_review(&result).unwrap();
    assert!(vault.dream_apply(&result, "错误摘要", false).is_err());
    result.proposals[0].content = Some("修改后的合成提议".into());
    assert!(vault
        .dream_apply(&result, &review.result_hash, false)
        .is_err());
    assert!(vault.memories().unwrap().is_empty());
}

#[test]
fn one_completed_job_cannot_publish_another_result() {
    let (_dir, vault) = vault();
    let source = event(&vault, "结果重放", "user", "personal");
    let job = vault
        .dream_export(std::slice::from_ref(&source.id), &[], "personal")
        .unwrap();
    let mut result = result(&job, vec![add(&source)]);
    let review = vault.dream_review(&result).unwrap();
    vault
        .dream_apply(&result, &review.result_hash, false)
        .unwrap();
    result.proposals[0].content = Some("不允许再次发布的另一份结果".into());
    assert!(vault.dream_review(&result).is_err());
    assert!(vault.dream_apply(&result, &digest(&result), false).is_err());
    assert_eq!(vault.memories().unwrap().len(), 1);
}

#[test]
fn forged_jobs_inputs_sources_and_scopes_are_rejected() {
    let (_dir, vault) = vault();
    let source = event(&vault, "合法来源", "user", "personal");
    let outside = event(&vault, "任务外来源", "user", "personal");
    let job = vault
        .dream_export(std::slice::from_ref(&source.id), &[], "personal")
        .unwrap();
    let original = result(&job, vec![add(&source)]);
    let mut wrong = original.clone();
    wrong.input_hash = "错误输入摘要".into();
    assert!(vault.dream_review(&wrong).is_err());
    let mut wrong = original.clone();
    wrong.job_id = "../../外部路径".into();
    assert!(vault.dream_review(&wrong).is_err());
    let mut wrong = original.clone();
    wrong.proposals[0].source_refs = vec![outside.id];
    assert!(vault.dream_review(&wrong).is_err());
    let mut wrong = original.clone();
    wrong.proposals[0].source_refs = vec![format!("evt_{}", "0".repeat(64))];
    assert!(vault.dream_review(&wrong).is_err());
    let mut wrong = original;
    wrong.proposals[0].scope = "project:越界".into();
    assert!(vault.dream_review(&wrong).is_err());
    assert!(vault.memories().unwrap().is_empty());
}

#[test]
fn protected_memory_requires_separate_approval_and_stays_protected() {
    let (_dir, vault) = vault();
    let source = event(&vault, "保护修改", "user", "personal");
    let old = memory(&vault, &source);
    let job = vault
        .dream_export(
            std::slice::from_ref(&source.id),
            std::slice::from_ref(&old.id),
            "personal",
        )
        .unwrap();
    let result = result(&job, vec![update(&source, &old, "update")]);
    let review = vault.dream_review(&result).unwrap();
    assert!(review.requires_protected_approval);
    assert!(vault
        .dream_apply(&result, &review.result_hash, false)
        .is_err());
    assert_eq!(vault.memory(&old.id).unwrap(), old);
    vault
        .dream_apply(&result, &review.result_hash, true)
        .unwrap();
    let new = vault.memory(&old.id).unwrap();
    assert_eq!(new.revision, 2);
    assert!(new.data.protected);
    assert_eq!(new.data.authority, "user");
}

#[test]
fn stale_read_set_and_unlisted_targets_are_rejected() {
    let (_dir, vault) = vault();
    let source = event(&vault, "过期基线", "user", "personal");
    let old = memory(&vault, &source);
    let job = vault
        .dream_export(
            std::slice::from_ref(&source.id),
            std::slice::from_ref(&old.id),
            "personal",
        )
        .unwrap();
    let proposal = update(&source, &old, "update");
    let result = result(&job, vec![proposal.clone()]);
    let review = vault.dream_review(&result).unwrap();
    let mut input = old.data.clone();
    input.content = "用户已作出更晚的修改".into();
    let updated = vault.update_memory(&old.id, 1, input).unwrap();
    assert!(vault
        .dream_apply(&result, &review.result_hash, true)
        .is_err());
    assert_eq!(vault.memory(&old.id).unwrap(), updated);
    let job = vault
        .dream_export(std::slice::from_ref(&source.id), &[], "personal")
        .unwrap();
    assert!(vault
        .dream_review(&self::result(&job, vec![proposal]))
        .is_err());
}

#[test]
fn assistant_suggestions_cannot_become_approved_user_facts() {
    let (_dir, vault) = vault();
    let source = event(&vault, "未获批准的助手提议", "assistant", "personal");
    let job = vault
        .dream_export(std::slice::from_ref(&source.id), &[], "personal")
        .unwrap();
    let promoted = result(&job, vec![add(&source)]);
    assert!(vault.dream_review(&promoted).is_err());
    let mut suggestion = add(&source);
    suggestion["evidence"] = "assistant_suggestion".into();
    let result = result(&job, vec![suggestion]);
    let review = vault.dream_review(&result).unwrap();
    let receipt = vault
        .dream_apply(&result, &review.result_hash, false)
        .unwrap();
    assert_eq!(
        vault.memory(&receipt.changes[0].id).unwrap().state,
        MemoryState::Tentative
    );
}

#[test]
fn supersede_preserves_original_protection_and_does_not_invent_end_time() {
    let (_dir, vault) = vault();
    let first = event(&vault, "原始事实", "user", "personal");
    let old = memory(&vault, &first);
    let new = event(&vault, "新证据明确替代", "user", "personal");
    let job = vault
        .dream_export(
            std::slice::from_ref(&new.id),
            std::slice::from_ref(&old.id),
            "personal",
        )
        .unwrap();
    let result = result(&job, vec![update(&new, &old, "supersede")]);
    let review = vault.dream_review(&result).unwrap();
    assert_eq!(review.changes.len(), 2);
    assert!(review.requires_protected_approval);
    let receipt = vault
        .dream_apply(&result, &review.result_hash, true)
        .unwrap();
    assert_eq!(receipt.changes.len(), 2);
    let original = vault.memory(&old.id).unwrap();
    assert_eq!(original.state, MemoryState::Superseded);
    assert!(original.data.valid_to.is_none());
    let replacement = vault
        .memories()
        .unwrap()
        .into_iter()
        .find(|m| m.id != old.id)
        .unwrap();
    assert_eq!(replacement.data.supersedes, vec![old.id]);
    assert!(replacement.data.protected);
    assert!(replacement.data.valid_from.is_none());
}

#[test]
fn suppression_blocks_export_and_results_prepared_before_forgetting() {
    let (_dir, vault) = vault();
    let source = event(&vault, "后来撤销", "user", "personal");
    let job = vault
        .dream_export(std::slice::from_ref(&source.id), &[], "personal")
        .unwrap();
    let result = result(&job, vec![add(&source)]);
    let review = vault.dream_review(&result).unwrap();
    vault
        .suppress(&source.id, "合成用户要求忘记".into())
        .unwrap();
    assert!(vault
        .dream_export(std::slice::from_ref(&source.id), &[], "personal")
        .is_err());
    assert!(vault
        .dream_apply(&result, &review.result_hash, false)
        .is_err());
    assert!(vault.memories().unwrap().is_empty());
}

#[test]
fn result_and_job_sizes_are_bounded_and_empty_output_is_not_completion() {
    let (_dir, vault) = vault();
    let source = event(&vault, "任务限额", "user", "personal");
    assert!(vault.dream_export(&[], &[], "personal").is_err());
    assert!(vault
        .dream_export(&vec![source.id.clone(); 65], &[], "personal")
        .is_err());
    let job = vault
        .dream_export(std::slice::from_ref(&source.id), &[], "personal")
        .unwrap();
    assert!(vault.dream_review(&result(&job, vec![])).is_err());
    assert!(vault
        .dream_review(&result(&job, vec![add(&source); 33]))
        .is_err());
    let mut huge = add(&source);
    huge["content"] = "超".repeat(64 * 1024).into();
    assert!(vault.dream_review(&result(&job, vec![huge])).is_err());
}

#[test]
fn noop_records_completion_but_conflict_publishes_nothing() {
    let (_dir, vault) = vault();
    let source = event(&vault, "无需新增", "user", "personal");
    let job = vault
        .dream_export(std::slice::from_ref(&source.id), &[], "personal")
        .unwrap();
    let conflict = result(
        &job,
        vec![json!({"operation":"conflict","scope":"personal","content":"两份合成事实互相冲突"})],
    );
    let review = vault.dream_review(&conflict).unwrap();
    assert!(!review.can_apply);
    assert!(vault
        .dream_apply(&conflict, &review.result_hash, false)
        .is_err());
    let noop = result(&job, vec![json!({"operation":"noop","scope":"personal"})]);
    let review = vault.dream_review(&noop).unwrap();
    let receipt = vault
        .dream_apply(&noop, &review.result_hash, false)
        .unwrap();
    assert!(receipt.changes.is_empty());
    assert!(vault.memories().unwrap().is_empty());
}

#[test]
fn proposal_cannot_forge_protection_or_authority_fields() {
    let (_dir, vault) = vault();
    let source = event(&vault, "权限伪造", "user", "personal");
    let job = vault
        .dream_export(std::slice::from_ref(&source.id), &[], "personal")
        .unwrap();
    let mut proposal = add(&source);
    proposal["authority"] = "user".into();
    proposal["protected"] = true.into();
    let value = json!({"schema":"recallcard.dream-result/1","job_id":job.job_id,"input_hash":job.input_hash,"proposals":[proposal]});
    assert!(serde_json::from_value::<DreamResult>(value).is_err());
}

#[test]
fn injected_context_is_not_exported_as_new_evidence() {
    let (_dir, vault) = vault();
    let source=vault.capture(serde_json::from_value(json!({"occurred_at":null,"role":"user","origin":"recallcard_context","content":"合成注入：旧记忆的副本","source":{"platform":"测试","conversation_id":"会话","message_id":"注入"}})).unwrap()).unwrap();
    assert!(vault.dream_export(&[source.id], &[], "personal").is_err());
}

// 直接构造“日志已 fsync、发布中断”的磁盘快照，验证真实恢复路径而非 mock。
fn stage_interrupted(vault: &Vault, job: &DreamJob, result: &DreamResult) -> Value {
    let review = vault.dream_review(result).unwrap();
    let changes: Vec<_> = review
        .changes
        .iter()
        .map(|c| {
            let m = c.after.as_ref().unwrap();
            json!({"id":m.id,"revision":m.revision,"content_hash":digest(m)})
        })
        .collect();
    let receipt = json!({"schema_version":1,"job_id":job.job_id,"input_hash":job.input_hash,"result_hash":review.result_hash,"applied_at":"2026-10-06T12:00:00Z","source_coverage":job.source_refs.iter().map(|s|s.reference.clone()).collect::<Vec<_>>(),"changes":changes,"already_applied":false});
    let transaction = json!({"schema_version":1,"vault_root":vault.root().to_string_lossy(),"job":job,"result":result,"approve_protected":true,"writes":review.changes,"receipt":receipt});
    fs::write(
        vault.state_dir().unwrap().join("dream-transaction.json"),
        serde_json::to_vec(&transaction).unwrap(),
    )
    .unwrap();
    transaction
}

#[test]
fn interrupted_multifile_publish_is_fail_closed_and_recovers_exactly_once() {
    let (_dir, vault) = vault();
    let source = event(&vault, "恢复流程", "user", "personal");
    let job = vault
        .dream_export(std::slice::from_ref(&source.id), &[], "personal")
        .unwrap();
    let mut second = add(&source);
    second["content"] = "合成第二条记忆".into();
    let result = result(&job, vec![add(&source), second]);
    let tx = stage_interrupted(&vault, &job, &result);
    let first: Memory = serde_json::from_value(tx["writes"][0]["after"].clone()).unwrap();
    fs::write(
        vault
            .root()
            .join("memories")
            .join(format!("{}.md", first.id)),
        render_memory(&first).unwrap(),
    )
    .unwrap();
    assert!(vault.memories().is_err());
    assert!(vault.memory(&first.id).is_err());
    assert!(vault.events().is_err());
    assert!(vault
        .dream_apply(
            &result,
            tx["receipt"]["result_hash"].as_str().unwrap(),
            true
        )
        .is_err());
    let recovered = vault.dream_recover().unwrap();
    assert_eq!(recovered["recovered"], true);
    assert_eq!(vault.memories().unwrap().len(), 2);
    assert!(!vault
        .state_dir()
        .unwrap()
        .join("dream-transaction.json")
        .exists());
    assert_eq!(vault.dream_recover().unwrap()["recovered"], false);
    let receipt = vault
        .dream_apply(
            &result,
            tx["receipt"]["result_hash"].as_str().unwrap(),
            true,
        )
        .unwrap();
    assert!(receipt.already_applied);
}

#[test]
fn recovery_refuses_external_edits_without_overwriting_them() {
    let (_dir, vault) = vault();
    let source = event(&vault, "恢复冲突", "user", "personal");
    let old = memory(&vault, &source);
    let job = vault
        .dream_export(
            std::slice::from_ref(&source.id),
            std::slice::from_ref(&old.id),
            "personal",
        )
        .unwrap();
    let result = result(&job, vec![update(&source, &old, "update")]);
    stage_interrupted(&vault, &job, &result);
    let mut external = old;
    external.data.content = "用户在中断后手动编辑的正文".into();
    let path = vault
        .root()
        .join("memories")
        .join(format!("{}.md", external.id));
    let bytes = render_memory(&external).unwrap();
    fs::write(&path, &bytes).unwrap();
    assert!(vault.dream_recover().is_err());
    assert_eq!(fs::read_to_string(path).unwrap(), bytes);
    assert!(vault
        .state_dir()
        .unwrap()
        .join("dream-transaction.json")
        .exists());
}

#[test]
fn corrupted_transaction_stays_visible_and_blocks_reads() {
    let (_dir, vault) = vault();
    let source = event(&vault, "损坏事务", "user", "personal");
    fs::write(
        vault.state_dir().unwrap().join("dream-transaction.json"),
        "<<<<<<< 合成冲突",
    )
    .unwrap();
    assert!(vault.dream_recover().is_err());
    assert!(vault.event(&source.id).is_err());
}

#[test]
fn changed_source_metadata_and_tampered_job_hash_are_rejected() {
    let (_dir, vault) = vault();
    let source = event(&vault, "来源摘要校验", "user", "personal");
    let job = vault
        .dream_export(std::slice::from_ref(&source.id), &[], "personal")
        .unwrap();
    let result = result(&job, vec![add(&source)]);
    fn find(path: &std::path::Path, id: &str) -> Option<std::path::PathBuf> {
        for entry in fs::read_dir(path).ok()? {
            let path = entry.ok()?.path();
            if path.is_dir() {
                if let Some(found) = find(&path, id) {
                    return Some(found);
                }
            } else if path.file_stem().and_then(|s| s.to_str()) == Some(id) {
                return Some(path);
            }
        }
        None
    }
    let path = find(&vault.root().join("events"), &source.id).unwrap();
    let original = fs::read(&path).unwrap();
    let mut changed: Value = serde_json::from_slice(&original).unwrap();
    changed["captured_at"] = "2026-10-05T00:00:00Z".into();
    fs::write(
        &path,
        format!("{}\n", serde_json::to_string(&changed).unwrap()),
    )
    .unwrap();
    assert!(vault.dream_review(&result).is_err());
    fs::write(path, original).unwrap();
    let path = vault
        .state_dir()
        .unwrap()
        .join("dream-jobs")
        .join(format!("{}.json", job.job_id));
    let mut changed = serde_json::to_value(job).unwrap();
    changed["source_refs"][0]["content_hash"] = "错误摘要".into();
    fs::write(path, serde_json::to_vec(&changed).unwrap()).unwrap();
    assert!(vault.dream_review(&result).is_err());
}

#[test]
fn recovery_handles_before_first_write_and_after_receipt_without_duplicates() {
    for after_receipt in [false, true] {
        let (_dir, vault) = vault();
        let source = event(&vault, "崩溃窗口", "user", "personal");
        let job = vault
            .dream_export(std::slice::from_ref(&source.id), &[], "personal")
            .unwrap();
        let result = result(&job, vec![add(&source)]);
        let tx = stage_interrupted(&vault, &job, &result);
        if after_receipt {
            let memory: Memory = serde_json::from_value(tx["writes"][0]["after"].clone()).unwrap();
            fs::write(
                vault
                    .root()
                    .join("memories")
                    .join(format!("{}.md", memory.id)),
                render_memory(&memory).unwrap(),
            )
            .unwrap();
            fs::write(
                vault
                    .root()
                    .join("control/dream-receipts")
                    .join(format!("{}.json", job.job_id)),
                serde_json::to_vec(&tx["receipt"]).unwrap(),
            )
            .unwrap();
        }
        assert_eq!(vault.dream_recover().unwrap()["recovered"], true);
        assert_eq!(vault.memories().unwrap().len(), 1);
        assert!(vault.root().join("generated/views/memories.md").exists());
        let receipt = vault
            .dream_apply(
                &result,
                tx["receipt"]["result_hash"].as_str().unwrap(),
                false,
            )
            .unwrap();
        assert!(receipt.already_applied);
    }
}

#[test]
fn same_revision_but_edited_memory_content_invalidates_read_set_hash() {
    let (_dir, vault) = vault();
    let source = event(&vault, "无版本号手工修改", "user", "personal");
    let mut old = memory(&vault, &source);
    let job = vault
        .dream_export(
            std::slice::from_ref(&source.id),
            std::slice::from_ref(&old.id),
            "personal",
        )
        .unwrap();
    let result = result(&job, vec![update(&source, &old, "update")]);
    old.data.content = "用户直接编辑了 Markdown 正文，版本号未改".into();
    fs::write(
        vault.root().join("memories").join(format!("{}.md", old.id)),
        render_memory(&old).unwrap(),
    )
    .unwrap();
    assert!(vault.dream_review(&result).is_err());
    assert_eq!(vault.memory(&old.id).unwrap(), old);
}

#[cfg(unix)]
#[test]
fn derived_view_failure_keeps_committed_memory_and_retries_without_duplicates() {
    use std::os::unix::fs::symlink;
    let (dir, vault) = vault();
    let source = event(&vault, "派生视图故障", "user", "personal");
    let job = vault
        .dream_export(std::slice::from_ref(&source.id), &[], "personal")
        .unwrap();
    let result = result(&job, vec![add(&source)]);
    let review = vault.dream_review(&result).unwrap();
    let views = vault.root().join("generated/views");
    fs::remove_dir(&views).unwrap();
    let external = dir.path().join("外部视图目录");
    fs::create_dir(&external).unwrap();
    symlink(&external, &views).unwrap();
    let error = vault
        .dream_apply(&result, &review.result_hash, false)
        .unwrap_err();
    assert!(error.contains("已安全发布"));
    assert_eq!(vault.memories().unwrap().len(), 1);
    assert!(!vault
        .state_dir()
        .unwrap()
        .join("dream-transaction.json")
        .exists());
    assert!(fs::read_dir(&external).unwrap().next().is_none());
    fs::remove_file(&views).unwrap();
    fs::create_dir(&views).unwrap();
    let retry = vault
        .dream_apply(&result, &review.result_hash, false)
        .unwrap();
    assert!(retry.already_applied);
    assert_eq!(vault.memories().unwrap().len(), 1);
    assert!(views.join("memories.md").exists());
}

fn cli(root: &std::path::Path, args: &[&str], input: Option<&Value>) -> std::process::Output {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let mut child = Command::new(env!("CARGO_BIN_EXE_recallcard"))
        .arg("--vault")
        .arg(root)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    if let Some(input) = input {
        child
            .stdin
            .take()
            .unwrap()
            .write_all(&serde_json::to_vec(input).unwrap())
            .unwrap();
    }
    child.wait_with_output().unwrap()
}
fn cli_ok(output: std::process::Output) -> Value {
    assert!(
        output.status.success(),
        "命令失败：{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn real_cli_capture_export_review_apply_and_replay_complete_the_manual_workflow() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("命令行Dream资料库");
    cli_ok(cli(&root, &["init"], None));
    let event = cli_ok(cli(
        &root,
        &["capture", "--file", "-"],
        Some(&json!({
            "occurred_at":null,"role":"user","origin":"native","scope":"personal","content":"合成决定：新的说明使用中文",
            "source":{"platform":"命令行测试","conversation_id":"合成对话","message_id":"第一条"}
        })),
    ));
    let job: DreamJob = serde_json::from_value(cli_ok(cli(
        &root,
        &[
            "dream",
            "export",
            "--source",
            event["id"].as_str().unwrap(),
            "--scope",
            "personal",
        ],
        None,
    )))
    .unwrap();
    let source: Event = serde_json::from_value(event).unwrap();
    let result = serde_json::to_value(result(&job, vec![add(&source)])).unwrap();
    let review = cli_ok(cli(
        &root,
        &["dream", "review", "--file", "-"],
        Some(&result),
    ));
    let denied = cli(
        &root,
        &["dream", "apply", "--file", "-", "--approve", "错误摘要"],
        Some(&result),
    );
    assert!(!denied.status.success());
    let receipt = cli_ok(cli(
        &root,
        &[
            "dream",
            "apply",
            "--file",
            "-",
            "--approve",
            review["result_hash"].as_str().unwrap(),
        ],
        Some(&result),
    ));
    assert_eq!(receipt["already_applied"], false);
    assert_eq!(receipt["changes"].as_array().unwrap().len(), 1);
    let replay = cli_ok(cli(
        &root,
        &[
            "dream",
            "apply",
            "--file",
            "-",
            "--approve",
            review["result_hash"].as_str().unwrap(),
        ],
        Some(&result),
    ));
    assert_eq!(replay["already_applied"], true);
    let recovered = cli_ok(cli(&root, &["dream", "recover"], None));
    assert_eq!(recovered["recovered"], false);
    let doctor = cli_ok(cli(&root, &["doctor"], None));
    assert_eq!(doctor["memories"], 1);
}

#[test]
fn real_cli_protected_approval_flag_is_required() {
    let (_dir, vault) = vault();
    let source = event(&vault, "CLI保护批准", "user", "personal");
    let old = memory(&vault, &source);
    let job: DreamJob = serde_json::from_value(cli_ok(cli(
        vault.root(),
        &[
            "dream", "export", "--source", &source.id, "--memory", &old.id, "--scope", "personal",
        ],
        None,
    )))
    .unwrap();
    let result = serde_json::to_value(result(&job, vec![update(&source, &old, "update")])).unwrap();
    let review = cli_ok(cli(
        vault.root(),
        &["dream", "review", "--file", "-"],
        Some(&result),
    ));
    let args = [
        "dream",
        "apply",
        "--file",
        "-",
        "--approve",
        review["result_hash"].as_str().unwrap(),
    ];
    assert!(!cli(vault.root(), &args, Some(&result)).status.success());
    assert_eq!(vault.memory(&old.id).unwrap().revision, 1);
    let mut approved = args.to_vec();
    approved.push("--approve-protected");
    cli_ok(cli(vault.root(), &approved, Some(&result)));
    assert_eq!(vault.memory(&old.id).unwrap().revision, 2);
}

#[test]
fn review_snapshot_is_not_reused_after_external_source_revision() {
    let (_dir, vault) = vault();
    let first = event(&vault, "多源一", "user", "personal");
    let second = event(&vault, "多源二", "user", "personal");
    let job = vault
        .dream_export(&[first.id.clone(), second.id.clone()], &[], "personal")
        .unwrap();
    let mut proposal = add(&first);
    proposal["source_refs"] = json!([first.id, second.id]);
    let result = result(&job, vec![proposal]);
    let review = vault.dream_review(&result).unwrap();
    assert!(review.can_apply);
    let external = Vault::open(vault.root()).unwrap();
    let mut revised = second.data.clone();
    revised.content = "合成后续修订：旧信息作废".into();
    let newer = external.capture(revised).unwrap();
    assert_eq!(newer.data.revision_of.as_deref(), Some(second.id.as_str()));
    assert!(vault.dream_review(&result).unwrap_err().contains("新修订"));
    assert!(vault
        .dream_apply(&result, &review.result_hash, false)
        .unwrap_err()
        .contains("新修订"));
    assert!(vault.memories().unwrap().is_empty());
}

#[test]
fn recovery_checks_event_transaction_barrier_before_reading_events() {
    let (_dir, vault) = vault();
    let source = event(&vault, "事件事务屏障", "user", "personal");
    let job = vault.dream_export(&[source.id], &[], "personal").unwrap();
    let result = result(&job, vec![add(&job.source_refs[0].event)]);
    stage_interrupted(&vault, &job, &result);
    let pending = vault.state_dir().unwrap().join("event-transaction.json");
    fs::write(&pending, b"{}").unwrap();
    let corrupt = vault.root().join("events/corrupt.jsonl");
    fs::write(&corrupt, b"not-json").unwrap();
    assert!(vault
        .dream_recover()
        .unwrap_err()
        .contains("未恢复的事件事务"));
    assert!(fs::read_dir(vault.root().join("memories"))
        .unwrap()
        .next()
        .is_none());
    fs::remove_file(pending).unwrap();
    fs::remove_file(corrupt).unwrap();
    assert_eq!(vault.dream_recover().unwrap()["recovered"], true);
}

#[test]
fn recovery_snapshot_respects_legacy_and_persisted_suppression_identity() {
    for missing_original in [false, true] {
        let (_dir, vault) = vault();
        let old = event(&vault, "恢复抑制身份", "user", "personal");
        let mut changed = old.data.clone();
        changed.content = "合成同来源新修订".into();
        let current = vault.capture(changed).unwrap();
        vault.suppress(&old.id, "合成临时规则".into()).unwrap();
        vault.restore(&old.id).unwrap();
        let job = vault.dream_export(&[current.id], &[], "personal").unwrap();
        let result = result(&job, vec![add(&job.source_refs[0].event)]);
        stage_interrupted(&vault, &job, &result);
        let path = vault
            .root()
            .join("control/suppressions")
            .join(format!("{}.json", old.id));
        let mut rule: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        rule["active"] = true.into();
        if missing_original {
            fn remove_original(path: &std::path::Path, id: &str) {
                for entry in fs::read_dir(path).unwrap() {
                    let path = entry.unwrap().path();
                    if path.is_dir() {
                        remove_original(&path, id);
                    } else if path.file_stem().and_then(|s| s.to_str()) == Some(id) {
                        fs::remove_file(path).unwrap();
                    }
                }
            }
            remove_original(&vault.root().join("events"), &old.id);
        } else {
            rule.as_object_mut().unwrap().remove("source_hashes");
        }
        fs::write(path, serde_json::to_vec(&rule).unwrap()).unwrap();
        assert!(vault.dream_recover().unwrap_err().contains("抑制"));
        assert!(fs::read_dir(vault.root().join("memories"))
            .unwrap()
            .next()
            .is_none());
        assert!(vault
            .state_dir()
            .unwrap()
            .join("dream-transaction.json")
            .exists());
    }
}
