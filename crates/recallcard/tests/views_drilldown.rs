//! 逐层下钻：读上层主题要能看到它下面的子主题内容。
mod support;

use recallcard::context::{Context, ReadArgs};
use recallcard::{policy::Access, Vault};
use serde_json::json;
use tempfile::tempdir;

#[test]
fn reading_a_parent_topic_reaches_its_children() {
    support::isolate_state();
    let dir = tempdir().unwrap();
    let vault = Vault::init(&dir.path().join("资料库")).unwrap();
    let event = vault
        .capture(
            serde_json::from_value(json!({
                "role": "user",
                "origin": "user_input",
                "scope": "personal",
                "content": "合成：Arch 上我用 pacman 和 yay",
                "source": {"platform": "synthetic", "conversation_id": "one", "message_id": "one"}
            }))
            .unwrap(),
        )
        .unwrap();
    let memory = vault
        .add_memory(
            serde_json::from_value(json!({
                "content": "Arch 上用 pacman 装包，AUR 走 yay",
                "source_refs": [event.id],
                "evidence": "user_explicit",
                "scope": "personal",
                "labels": ["环境/包管理"]
            }))
            .unwrap(),
        )
        .unwrap();

    let context = Context::new(&vault, Access::new(vec!["personal".into()]).unwrap());
    let read = |label: &str| {
        context
            .read(ReadArgs {
                refs: vec![format!("view:{label}")],
                budget_bytes: 32768,
            })
            .unwrap()
    };

    let parent = read("环境");
    let refs: Vec<String> = parent["results"][0]["records"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["ref"].as_str().unwrap().to_owned())
        .collect();
    assert!(
        refs.iter().any(|r| r.contains(&memory.id)),
        "读「环境」应当带出「环境/包管理」下的记忆：{parent}"
    );

    let child = read("环境/包管理");
    assert_eq!(
        child["results"][0]["records"].as_array().unwrap().len(),
        1,
        "子主题只给出自己那一条：{child}"
    );
}
