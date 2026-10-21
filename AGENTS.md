# RecallCard 开发约定

- 文档、说明、用户可见错误与提交说明使用中文；代码标识符使用清晰的英文
- 事实源只有 Event 与 Memory 文件；检索索引与人类视图必须能重新生成
- 不把真实个人对话、私密记忆、密钥或本机配置提交到项目仓库
- 模型侧只读：Agent 可以追加 Event，不能改 Memory、不能删除
- 不引入后台常驻服务；`recallcard ui` 只在本机回环地址上、由人手动启动
- 云 embedding 与模型调用只在显式配置后由人触发的动作使用
- 先验证改动，再分阶段提交；明确记录未验证的平台和功能
- 每个测试文件只守护一条不变量；不为已删除的功能保留测试或兼容分支
- 验证：
  - `cargo fmt --all -- --check`
  - `cargo clippy --workspace --all-targets -- -D warnings`
  - `cargo test --workspace`
  - `cd web && pnpm build`（界面类型检查与构建）
