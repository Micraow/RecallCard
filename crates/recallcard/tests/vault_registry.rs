//! 已知资料库清单：列得出、选得中、移除只动清单不动文件。
mod support;

use std::path::Path;
use std::process::Command;

fn run(state: &Path, args: &[&str]) -> (bool, serde_json::Value) {
    let output = Command::new(env!("CARGO_BIN_EXE_recallcard"))
        .args(args)
        .env("RECALLCARD_STATE_DIR", state)
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&output.stdout).to_string();
    (
        output.status.success(),
        serde_json::from_str(&text).unwrap_or_default(),
    )
}

fn make_vault(state: &Path, name: &str) -> String {
    let path = state.join(name);
    let path_text = path.to_string_lossy().to_string();
    assert!(run(state, &["init", "--vault", &path_text]).0);
    path_text
}

#[test]
fn the_loaded_vault_list_shows_reachability_and_the_default() {
    support::isolate_state();
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    std::fs::create_dir_all(&state).unwrap();

    let first = make_vault(&state, "资料一");
    let second = make_vault(&state, "资料二");
    // 打开第二个，它就成为默认。
    assert!(run(&state, &["--vault", &second, "views"]).0);

    let (ok, listing) = run(&state, &["vault", "list"]);
    assert!(ok, "{listing}");
    let rows = listing["known"].as_array().unwrap();
    assert_eq!(rows.len(), 2, "两个库都要在清单里：{listing}");
    let pick = |path: &str| {
        rows.iter()
            .find(|row| row["path"].as_str() == Some(path))
            .unwrap()
            .clone()
    };
    assert_eq!(pick(&second)["default"], true);
    assert_eq!(pick(&first)["default"], false);
    assert_eq!(pick(&first)["reachable"], true);

    // 库被搬走：仍然列出，但标成不可达，并给出原因。
    std::fs::rename(&first, dir.path().join("搬走")).unwrap();
    let (_, listing) = run(&state, &["vault", "list"]);
    let row = listing["known"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["path"].as_str() == Some(&first))
        .unwrap()
        .clone();
    assert_eq!(row["reachable"], false, "{listing}");
    assert!(row["problem"].is_string());

    // 移出清单：只掉条目，库本身还在。
    assert!(run(&state, &["vault", "forget", &first]).0);
    let (_, listing) = run(&state, &["vault", "list"]);
    assert_eq!(listing["known"].as_array().unwrap().len(), 1, "{listing}");
    assert!(Path::new(&second)
        .join("control/schema-version.json")
        .exists());
}
