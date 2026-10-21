//! 本机设置的不变量：密钥只落在本人可读的文件里，任何界面视图都不回显。
use recallcard::{
    settings::{LlmSettings, Settings},
    vault::Vault,
};
use serde_json::json;
use tempfile::tempdir;

const LLM_KEY: &str = "sk-合成-不要外泄-0001";
const EMBEDDING_KEY: &str = "sk-合成-不要外泄-0002";

#[test]
fn keys_never_come_back_through_the_settings_view() {
    let dir = tempdir().unwrap();
    Vault::init(dir.path()).unwrap();
    let vault = Vault::open(dir.path()).unwrap();

    let next = Settings {
        embedding: recallcard::settings::EmbeddingSettings {
            provider: "cloud".into(),
            endpoint: "https://example.com/v1/embeddings".into(),
            model: "text-embedding-v4".into(),
            dimensions: 1024,
            key: EMBEDDING_KEY.into(),
        },
        llm: LlmSettings {
            base_url: "https://api.example.com/v1".into(),
            model: "合成模型".into(),
            key: LLM_KEY.into(),
            max_output_tokens: 2048,
        },
    };
    Settings::save(&vault, &next).unwrap();

    let view = Settings::load(&vault).unwrap().public_view().to_string();
    assert!(!view.contains(LLM_KEY), "视图泄露了模型密钥：{view}");
    assert!(!view.contains(EMBEDDING_KEY), "视图泄露了编码密钥：{view}");
    assert!(view.contains("\"has_key\":true"), "视图应只报告是否已配置");

    // 留空表示保持原值，显式清除才会真的删掉。
    let loaded = Settings::load(&vault).unwrap();
    assert_eq!(loaded.llm.key, LLM_KEY);
    let kept = loaded.apply_patch(&json!({"llm": {"key": ""}})).unwrap();
    assert_eq!(kept.llm.key, LLM_KEY);
    let cleared = loaded
        .apply_patch(&json!({"llm": {"clear_key": true}}))
        .unwrap();
    assert!(cleared.llm.key.is_empty());

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(Settings::path(&vault).unwrap())
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "设置文件必须只有本人可读写");
    }
}
