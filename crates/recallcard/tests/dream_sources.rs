//! 自动选源的不变量：已经进过某条记忆的原始记录，不会被再次整理。
use chrono::{Duration, Utc};
use recallcard::{dream_task, vault::Vault};
use serde_json::json;
use tempfile::tempdir;

const SCOPE: &str = "personal";

fn capture(vault: &Vault, content: &str, occurred_at: chrono::DateTime<Utc>) -> recallcard::Event {
    vault
        .capture(
            serde_json::from_value(json!({
                "role": "user", "origin": "native", "scope": SCOPE, "content": content,
                "occurred_at": occurred_at,
                "source": {"platform": "synthetic", "conversation_id": "选源", "message_id": content}
            }))
            .unwrap(),
        )
        .unwrap()
}

#[test]
fn already_consolidated_events_are_not_offered_again() {
    let dir = tempdir().unwrap();
    Vault::init(dir.path()).unwrap();
    let vault = Vault::open(dir.path()).unwrap();

    let now = Utc::now();
    let old = capture(&vault, "合成：已经整理过的决定", now - Duration::days(3));
    let fresh = capture(&vault, "合成：还没整理的对话", now - Duration::days(1));
    let older = capture(&vault, "合成：更早的对话", now - Duration::days(9));

    vault
        .add_memory(
            serde_json::from_value(json!({
                "content": "合成记忆：来源是那条已经整理过的决定",
                "source_refs": [old.id], "evidence": "user_explicit", "scope": SCOPE
            }))
            .unwrap(),
        )
        .unwrap();

    let all = dream_task::select_sources(&vault, SCOPE, None, None, 64).unwrap();
    let ids: Vec<&str> = all.iter().map(|event| event.id.as_str()).collect();
    assert_eq!(all.len(), 2, "只应剩下未整理的两条");
    assert!(!ids.contains(&old.id.as_str()), "已整理的记录被重复选中");
    assert_eq!(ids, vec![fresh.id.as_str(), older.id.as_str()], "新的优先");

    // 时间窗收窄后，只剩窗口内的那一条。
    let since = now - Duration::days(5);
    let windowed = dream_task::select_sources(&vault, SCOPE, Some(since), None, 64).unwrap();
    let windowed_ids: Vec<&str> = windowed.iter().map(|event| event.id.as_str()).collect();
    assert_eq!(windowed_ids, vec![fresh.id.as_str()]);

    // 条数上限同样生效。
    let limited = dream_task::select_sources(&vault, SCOPE, None, None, 1).unwrap();
    assert_eq!(limited.len(), 1);
    assert_eq!(limited[0].id, fresh.id);
}
