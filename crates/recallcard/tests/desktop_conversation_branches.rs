//! 合成对话验证可见分支、权限缺口与长度预算，不使用个人历史。
use recallcard::{
    desktop::{DesktopSession, VaultInfo},
    Event, Vault,
};
use serde_json::json;
use std::{collections::BTreeSet, fs, path::Path};
use tempfile::{tempdir, TempDir};

fn setup() -> (TempDir, DesktopSession, VaultInfo, Vault) {
    let dir = tempdir().unwrap();
    let mut ui = DesktopSession::default();
    let info = ui.select_vault(&dir.path().join("vault"), true).unwrap();
    let vault = Vault::open(Path::new(&info.root)).unwrap();
    (dir, ui, info, vault)
}

fn capture(vault: &Vault, id: &str, parent: Option<&str>, scope: &str, text: &str) -> Event {
    vault.capture(serde_json::from_value(json!({
        "scope":scope,"role":"user","origin":"user_input","content":text,
        "source":{"platform":"deepseek","conversation_id":"synthetic-branches","message_id":id},
        "metadata":{"conversation_title":"合成分支会话","previous_message_id":parent,
            "deepseek":{"node_id":id,"parent_id":parent,"nearest_visible_parent_id":parent,"omitted_parent_nodes":0}}
    })).unwrap()).unwrap()
}

fn reference(event: &Event) -> String {
    format!("event:{}", event.id)
}

#[test]
fn regenerated_siblings_require_a_leaf_and_selected_path_excludes_other_answers() {
    let (_dir, ui, info, vault) = setup();
    let root = capture(&vault, "root", None, "personal", "合成共同问题");
    let a = capture(&vault, "a", Some("root"), "personal", "合成路线甲");
    let a_end = capture(&vault, "a-end", Some("a"), "personal", "合成甲结尾");
    let b = capture(&vault, "b", Some("root"), "personal", "合成路线乙");
    let b_end = capture(&vault, "b-end", Some("b"), "personal", "合成乙结尾");
    let conversation = root.data.session_key();
    let page = ui
        .conversation_messages(&info.session_id, "personal", &conversation, 0)
        .unwrap();
    assert_eq!(page["total"], 5);
    assert_eq!(page["order_known"], false);
    assert_eq!(page["order_kind"], "branch_forest");
    assert_eq!(page["branch_summary"]["branch_count"], 2);
    assert_eq!(page["branch_summary"]["root_count"], 1);
    assert_eq!(page["branch_summary"]["selection_required"], true);
    assert_eq!(page["messages"][0]["branch"]["child_count"], 2);
    let leaves: BTreeSet<_> = page["branch_summary"]["branches"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["branch_ref"].as_str().unwrap())
        .collect();
    assert_eq!(
        leaves,
        BTreeSet::from([reference(&a_end), reference(&b_end)])
            .iter()
            .map(String::as_str)
            .collect()
    );
    assert!(ui
        .continuation(&info.session_id, "personal", &conversation, "")
        .unwrap_err()
        .contains("明确选择"));
    for invalid in [
        reference(&root),
        reference(&a),
        reference(&b),
        "event:missing".into(),
        "wrong-ref".into(),
    ] {
        assert!(ui
            .continuation_branch(
                &info.session_id,
                "personal",
                &conversation,
                "",
                Some(&invalid)
            )
            .is_err());
    }
    for (leaf, included, excluded) in [
        (&a_end, "合成路线甲", "合成路线乙"),
        (&b_end, "合成路线乙", "合成路线甲"),
    ] {
        let handoff = ui
            .continuation_branch(
                &info.session_id,
                "personal",
                &conversation,
                "合成目标",
                Some(&reference(leaf)),
            )
            .unwrap();
        let text = handoff["text"].as_str().unwrap();
        assert!(text.contains("合成共同问题") && text.contains(included));
        assert!(!text.contains(excluded));
        assert_eq!(handoff["message_count"], 3);
        assert_eq!(handoff["available_messages"], 3);
        assert_eq!(handoff["available_conversation_messages"], 5);
        assert_eq!(handoff["selected_branch_ref"], reference(leaf));
        assert_eq!(handoff["order_known"], true);
        assert_eq!(handoff["truncated"], false);
    }
}

#[test]
fn separate_roots_never_become_a_single_continuation() {
    let (_dir, ui, info, vault) = setup();
    let first = capture(&vault, "first-root", None, "personal", "合成第一独立根");
    let first_leaf = capture(
        &vault,
        "first-leaf",
        Some("first-root"),
        "personal",
        "合成第一末端",
    );
    let second = capture(&vault, "second-root", None, "personal", "合成第二独立根");
    let conversation = first.data.session_key();
    let page = ui
        .conversation_messages(&info.session_id, "personal", &conversation, 0)
        .unwrap();
    assert_eq!(page["branch_summary"]["root_count"], 2);
    assert_eq!(page["branch_summary"]["branch_count"], 2);
    assert!(ui
        .continuation(&info.session_id, "personal", &conversation, "")
        .is_err());
    let selected = ui
        .continuation_branch(
            &info.session_id,
            "personal",
            &conversation,
            "",
            Some(&reference(&second)),
        )
        .unwrap();
    assert_eq!(selected["message_count"], 1);
    assert!(!selected["text"].as_str().unwrap().contains("合成第一"));
    let first_selected = ui
        .continuation_branch(
            &info.session_id,
            "personal",
            &conversation,
            "",
            Some(&reference(&first_leaf)),
        )
        .unwrap();
    assert_eq!(first_selected["message_count"], 2);
    assert!(!first_selected["text"]
        .as_str()
        .unwrap()
        .contains("合成第二"));
}

#[test]
fn omitted_messages_keep_visible_ancestry_and_explicit_gaps_without_hidden_reasoning() {
    let (dir, mut ui, info, _vault) = setup();
    let exported = json!({"id":"synthetic-export","title":"合成隐藏节点","inserted_at":null,"updated_at":null,"mapping":{
        "root":{"id":"root","parent":null,"children":["question"],"message":null},
        "question":{"id":"question","parent":"root","children":["hidden","other"],"message":{"fragments":[{"type":"REQUEST","content":"合成可见问题"}]}},
        "hidden":{"id":"hidden","parent":"question","children":["answer"],"message":{"fragments":[{"type":"THINK","content":"绝不应出现的合成推理正文"}]}},
        "answer":{"id":"answer","parent":"hidden","children":[],"message":{"fragments":[{"type":"RESPONSE","content":"合成选择的可见答案"}]}},
        "other":{"id":"other","parent":"question","children":[],"message":{"fragments":[{"type":"RESPONSE","content":"合成另一答案"}]}}
    }});
    let file = dir.path().join("synthetic-deepseek.json");
    fs::write(&file, exported.to_string()).unwrap();
    let preview = ui
        .preview_import(&info.session_id, "deepseek-export", &file, "personal")
        .unwrap();
    ui.confirm_import(&info.session_id, &preview.preview_id)
        .unwrap();
    let list = ui.conversations(&info.session_id, "personal").unwrap();
    let conversation = list["conversations"][0]["session_ref"].as_str().unwrap();
    let page = ui
        .conversation_messages(&info.session_id, "personal", conversation, 0)
        .unwrap();
    assert_eq!(page["total"], 3);
    assert_eq!(page["branch_summary"]["root_count"], 1);
    assert_eq!(page["branch_summary"]["branch_count"], 2);
    let question = page["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["text"] == "合成可见问题")
        .unwrap();
    let answer = page["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["text"] == "合成选择的可见答案")
        .unwrap();
    assert_eq!(question["branch"]["gap_before"], false);
    assert_eq!(answer["branch"]["parent_ref"], question["ref"]);
    assert_eq!(answer["branch"]["gap_before"], true);
    assert_eq!(answer["branch"]["omitted_parent_nodes"], 1);
    let selected = ui
        .continuation_branch(
            &info.session_id,
            "personal",
            conversation,
            "",
            answer["ref"].as_str(),
        )
        .unwrap();
    assert_eq!(selected["message_count"], 2);
    assert_eq!(selected["has_gaps"], true);
    assert_eq!(selected["order_known"], false);
    assert!(selected["text"].as_str().unwrap().contains("来源缺口"));
    assert!(!selected["text"].as_str().unwrap().contains("合成另一答案"));
    assert!(!page.to_string().contains("绝不应出现"));
    assert!(!selected.to_string().contains("绝不应出现"));
}

#[test]
fn scopes_and_forgotten_parents_cut_the_chain_without_disclosing_inaccessible_sources() {
    let (_dir, ui, info, vault) = setup();
    let root = capture(&vault, "public-root", None, "personal", "合成已授权根");
    let hidden = capture(
        &vault,
        "private-parent-marker",
        Some("public-root"),
        "work",
        "合成另一个范围正文",
    );
    let end = capture(
        &vault,
        "visible-end",
        Some("private-parent-marker"),
        "personal",
        "合成可见末端",
    );
    let conversation = root.data.session_key();
    let page = ui
        .conversation_messages(&info.session_id, "personal", &conversation, 0)
        .unwrap();
    let serialized = page.to_string();
    assert!(!serialized.contains("private-parent-marker"));
    assert!(!serialized.contains(&hidden.id));
    assert!(!serialized.contains("合成另一个范围正文"));
    assert_eq!(page["branch_summary"]["root_count"], 2);
    assert_eq!(page["branch_summary"]["has_gaps"], true);
    assert!(ui
        .continuation_branch(
            &info.session_id,
            "personal",
            &conversation,
            "",
            Some(&reference(&hidden))
        )
        .is_err());
    let selected = ui
        .continuation_branch(
            &info.session_id,
            "personal",
            &conversation,
            "",
            Some(&reference(&end)),
        )
        .unwrap();
    assert_eq!(selected["message_count"], 1);
    assert!(selected["text"].as_str().unwrap().contains("来源缺口"));
    assert!(!selected.to_string().contains("合成已授权根"));
    assert!(!selected.to_string().contains(&hidden.id));
    vault
        .suppress(&hidden.id, "合成遗忘中间来源".into())
        .unwrap();
    assert!(ui
        .continuation_branch(
            &info.session_id,
            "work",
            &conversation,
            "",
            Some(&reference(&hidden))
        )
        .is_err());
    vault.suppress(&root.id, "合成遗忘根".into()).unwrap();
    let after = ui
        .continuation(&info.session_id, "personal", &conversation, "")
        .unwrap();
    assert_eq!(after["selected_branch_ref"], reference(&end));
    assert_eq!(after["has_gaps"], true);
    assert!(!after.to_string().contains(&root.id));
}

#[test]
fn legacy_visible_dom_chain_keeps_its_order_and_existing_entry_point() {
    let (_dir, ui, info, vault) = setup();
    let mut refs = Vec::new();
    for (id, parent, role, text) in [
        ("old-a", None, "user", "合成旧格式用户决定"),
        ("old-b", Some("old-a"), "assistant", "合成旧格式助手建议"),
    ] {
        refs.push(vault.capture(serde_json::from_value(json!({
            "scope":"personal","role":role,"origin":if role == "user" {"user_input"} else {"assistant_output"},"content":text,
            "source":{"platform":"deepseek","conversation_id":"synthetic-dom","message_id":id},
            "metadata":{"previous_message_id":parent}
        })).unwrap()).unwrap());
    }
    let conversation = refs[0].data.session_key();
    let page = ui
        .conversation_messages(&info.session_id, "personal", &conversation, 0)
        .unwrap();
    assert_eq!(page["order_known"], true);
    assert_eq!(page["branch_summary"]["selection_required"], false);
    assert_eq!(
        page["messages"][1]["branch"]["parent_ref"],
        reference(&refs[0])
    );
    let handoff = ui
        .continuation(&info.session_id, "personal", &conversation, "")
        .unwrap();
    let explicit = ui
        .continuation_branch(
            &info.session_id,
            "personal",
            &conversation,
            "",
            Some(&reference(&refs[1])),
        )
        .unwrap();
    assert_eq!(handoff, explicit);
    assert_eq!(handoff["message_count"], 2);
    assert!(
        handoff["text"].as_str().unwrap().find("合成旧格式用户决定")
            < handoff["text"].as_str().unwrap().find("合成旧格式助手建议")
    );
    vault
        .suppress(&refs[0].id, "合成忘记旧消息".into())
        .unwrap();
    let after = ui
        .continuation(&info.session_id, "personal", &conversation, "")
        .unwrap();
    assert_eq!(after["message_count"], 1);
    assert_eq!(after["has_gaps"], true);
    assert!(!after.to_string().contains("合成旧格式用户决定"));
}

#[test]
fn branch_summaries_and_pages_stay_bounded_and_unlisted_leaves_are_selectable() {
    let (_dir, ui, info, vault) = setup();
    let mut events = Vec::new();
    for index in 0..120 {
        let text = if index == 119 {
            "长正文".repeat(2000)
        } else {
            format!("合成独立分支 {index} {}", "内容".repeat(70))
        };
        events.push(capture(
            &vault,
            &format!("root-{index}"),
            None,
            "personal",
            &text,
        ));
    }
    let conversation = events[0].data.session_key();
    let first = ui
        .conversation_messages(&info.session_id, "personal", &conversation, 0)
        .unwrap();
    assert_eq!(first["branch_summary"]["branch_count"], 120);
    assert_eq!(first["branch_summary"]["branches_truncated"], true);
    assert!(first["branch_summary"]["shown_branches"].as_u64().unwrap() <= 100);
    let listed: BTreeSet<_> = first["branch_summary"]["branches"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["branch_ref"].as_str().unwrap().to_owned())
        .collect();
    let unlisted = events
        .iter()
        .find(|event| !listed.contains(&reference(event)))
        .unwrap();
    let selected = ui
        .continuation_branch(
            &info.session_id,
            "personal",
            &conversation,
            "",
            Some(&reference(unlisted)),
        )
        .unwrap();
    assert_eq!(selected["message_count"], 1);
    let mut seen = BTreeSet::new();
    let mut offset = 0;
    loop {
        let page = ui
            .conversation_messages(&info.session_id, "personal", &conversation, offset)
            .unwrap();
        assert!(page.to_string().len() < 32768);
        for row in page["messages"].as_array().unwrap() {
            assert_eq!(row["branch"]["is_branch_end"], true);
            assert!(seen.insert(row["ref"].as_str().unwrap().to_owned()));
            if row["ref"] == reference(&events[119]) {
                assert_eq!(row["text_truncated"], true);
                assert!(row["text"].as_str().unwrap().len() <= 6000);
            }
        }
        match page["next_offset"].as_u64() {
            Some(next) => {
                assert!(next as usize > offset);
                offset = next as usize;
            }
            None => break,
        }
    }
    assert_eq!(seen.len(), 120);
    let long = ui
        .continuation_branch(
            &info.session_id,
            "personal",
            &conversation,
            "",
            Some(&reference(&events[119])),
        )
        .unwrap();
    assert_eq!(long["message_count"], 1);
    assert_eq!(long["truncated"], true);
    assert!(long["text"].as_str().unwrap().contains("本条仅节选"));
}

#[test]
fn long_selected_paths_keep_latest_messages_under_the_handoff_budget() {
    let (_dir, ui, info, vault) = setup();
    let mut last = None;
    for index in 0usize..50 {
        let parent = index.checked_sub(1).map(|i| format!("chain-{i}"));
        last = Some(capture(
            &vault,
            &format!("chain-{index}"),
            parent.as_deref(),
            "personal",
            &format!("合成长链消息-{index}: {}", "正文".repeat(500)),
        ));
    }
    let last = last.unwrap();
    let result = ui
        .continuation(&info.session_id, "personal", &last.data.session_key(), "")
        .unwrap();
    assert_eq!(result["available_messages"], 50);
    assert!(result["message_count"].as_u64().unwrap() <= 40);
    assert_eq!(result["truncated"], true);
    assert!(result["text"]
        .as_str()
        .unwrap()
        .contains("合成长链消息-49:"));
    assert!(!result["text"].as_str().unwrap().contains("合成长链消息-0:"));
    assert!(result["text"].as_str().unwrap().len() < 32768);
}

#[test]
fn cyclic_saved_relationships_fail_closed_instead_of_hanging() {
    let (_dir, ui, info, vault) = setup();
    let a = capture(&vault, "cycle-a", Some("cycle-b"), "personal", "合成循环甲");
    capture(&vault, "cycle-b", Some("cycle-a"), "personal", "合成循环乙");
    let conversation = a.data.session_key();
    assert!(ui
        .conversation_messages(&info.session_id, "personal", &conversation, 0)
        .unwrap_err()
        .contains("循环"));
    assert!(ui
        .continuation(&info.session_id, "personal", &conversation, "")
        .unwrap_err()
        .contains("循环"));
}

#[test]
fn escaped_message_text_cannot_bypass_the_serialized_page_budget() {
    let (_dir, ui, info, vault) = setup();
    let event = capture(
        &vault,
        "escaped",
        None,
        "personal",
        &"\u{0001}".repeat(6000),
    );
    let page = ui
        .conversation_messages(&info.session_id, "personal", &event.data.session_key(), 0)
        .unwrap();
    assert!(page.to_string().len() < 32768);
    assert_eq!(page["messages"][0]["text_truncated"], true);
    assert!(!page["messages"][0]["text"].as_str().unwrap().is_empty());
    assert_eq!(vault.event(&event.id).unwrap().data.text().len(), 6000);
}

#[test]
fn event_location_uses_the_same_visible_parent_forest_as_conversation_pages() {
    let (_dir, ui, info, vault) = setup();
    // 片段可能先后分批导入；不能假设父节点一定更早写入资料库。
    let child = vault.capture(serde_json::from_value(json!({
        "scope":"personal","role":"assistant","origin":"assistant_output","content":"合成先导入的后续回答",
        "source":{"platform":"deepseek","conversation_id":"synthetic-branches","message_id":"early-child"},
        "metadata":{"previous_message_id":"omitted-node","deepseek":{"node_id":"early-child","parent_id":"omitted-node","nearest_visible_parent_id":"late-parent","omitted_parent_nodes":1}}
    })).unwrap()).unwrap();
    let parent = capture(
        &vault,
        "late-parent",
        None,
        "personal",
        "合成后导入的先前问题",
    );
    let conversation = parent.data.session_key();
    let page = ui
        .conversation_messages(&info.session_id, "personal", &conversation, 0)
        .unwrap();
    assert_eq!(page["messages"][0]["ref"], reference(&parent));
    assert_eq!(page["messages"][1]["ref"], reference(&child));
    for event in [&parent, &child] {
        let location = ui
            .event_location(&info.session_id, "personal", &reference(event))
            .unwrap();
        let located_page = ui
            .conversation_messages(
                &info.session_id,
                "personal",
                &conversation,
                location["offset"].as_u64().unwrap() as usize,
            )
            .unwrap();
        assert_eq!(located_page["messages"][0]["ref"], reference(event));
    }
}

#[test]
fn legacy_records_without_relationships_remain_an_unordered_collection() {
    let (_dir, ui, info, vault) = setup();
    let mut events = Vec::new();
    for (id, text) in [
        ("legacy-one", "合成未知关系片段甲"),
        ("legacy-two", "合成未知关系片段乙"),
    ] {
        events.push(vault.capture(serde_json::from_value(json!({
            "scope":"personal","role":"user","origin":"user_input","content":text,
            "source":{"platform":"chatgpt","conversation_id":"synthetic-unknown-order","message_id":id}
        })).unwrap()).unwrap());
    }
    let conversation = events[0].data.session_key();
    let page = ui
        .conversation_messages(&info.session_id, "personal", &conversation, 0)
        .unwrap();
    assert_eq!(page["branch_summary"]["branch_count"], 0);
    assert_eq!(page["branch_summary"]["root_count"], 0);
    assert_eq!(page["branch_summary"]["selection_required"], false);
    assert!(page["branch_summary"]["branches"]
        .as_array()
        .unwrap()
        .is_empty());
    assert_eq!(page["order_kind"], "unverified");
    assert_eq!(page["order_known"], false);
    for row in page["messages"].as_array().unwrap() {
        assert_eq!(row["branch"]["is_branch_end"], false);
        assert_eq!(row["branch"]["relationship_known"], false);
    }
    let selected = ui
        .continuation(&info.session_id, "personal", &conversation, "")
        .unwrap();
    assert_eq!(selected["message_count"], 2);
    assert_eq!(selected["available_messages"], 2);
    assert_eq!(selected["order_known"], false);
    assert_eq!(selected["branch_count"], 0);
    assert!(selected["selected_branch_ref"].is_null());
    let text = selected["text"].as_str().unwrap();
    assert!(text.contains("未知顺序的可见片段集合"));
    assert!(text.contains("合成未知关系片段甲") && text.contains("合成未知关系片段乙"));
    assert!(ui
        .continuation_branch(
            &info.session_id,
            "personal",
            &conversation,
            "",
            Some(&reference(&events[0]))
        )
        .is_err());
}
