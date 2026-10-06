# RecallCard 开发约定

- 文档、说明、用户可见错误与提交说明使用中文；代码标识符使用清晰的英文
- 事实源只有 Event 与 Memory 文件；检索索引与人类视图必须能重新生成
- 不把真实个人对话、私密记忆、密钥或本机配置提交到项目仓库
- 模型侧能力只读；任何网页最终发送必须由用户点击
- 先验证改动，再分阶段提交；明确记录未验证的平台和功能
- Rust 验证：`cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace`
- Python 验证：`python3 -m unittest discover -s python/tests -v`
