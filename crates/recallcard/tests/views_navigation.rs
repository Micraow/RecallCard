//! 分层导航：一条记忆挂在多个主题下，正文始终只存一份。
mod support;

use recallcard::Vault;
use serde_json::json;
use tempfile::tempdir;

#[test]
fn a_memory_reaches_every_topic_but_keeps_one_body() {
    support::isolate_state();
    let dir = tempdir().unwrap();
    let vault = Vault::init(&dir.path().join("资料库")).unwrap();
    let event = vault
        .capture(
            serde_json::from_value(json!({
                "role": "user",
                "origin": "user_input",
                "scope": "personal",
                "content": "合成：我主力机是 Arch，默认 shell 用 fish",
                "source": {"platform": "synthetic", "conversation_id": "one", "message_id": "one"}
            }))
            .unwrap(),
        )
        .unwrap();
    let memory = vault
        .add_memory(
            serde_json::from_value(json!({
                "content": "主力机是 Arch Linux，默认 shell 是 fish\n包管理用 pacman，AUR 走 yay，升级前先看 archnews",
                "source_refs": [event.id],
                "evidence": "user_explicit",
                "scope": "personal",
                "labels": ["环境/操作系统", "工具/终端"]
            }))
            .unwrap(),
        )
        .unwrap();

    vault.rebuild_views().unwrap();
    let nav = std::fs::read_to_string(vault.root().join("generated/views/nav.md")).unwrap();

    for topic in ["## 环境/操作系统", "## 工具/终端"] {
        assert!(nav.contains(topic), "两处主题都要有入口：{nav}");
    }
    assert_eq!(
        nav.matches(&memory.id).count(),
        2,
        "两个入口各引用一次记忆编号：{nav}"
    );
    assert!(
        !nav.contains("包管理用 pacman"),
        "导航只给首行一句话，其余正文不该被复制进来：{nav}"
    );

    // 正文只有一份，仍然在 memories/ 里。
    let body = vault.root().join(format!("memories/{}.md", memory.id));
    assert!(body.exists(), "正文应当在 memories/ 里：{}", body.display());
}
