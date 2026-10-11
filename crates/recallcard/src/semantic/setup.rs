//! 桌面受限表单到私有启动配置；不接受解释器、缓存路径或任意提供器权限字段。
use super::*;
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalSemanticSetup {
    pub endpoint: String,
    pub model: String,
    pub dimensions: usize,
    pub revision: String,
    #[serde(default)]
    pub query_instruction: String,
    #[serde(default)]
    pub document_instruction: String,
    pub query_approved: bool,
    pub memory_approved: bool,
    #[serde(default)]
    pub resume_background: bool,
}
impl LocalSemanticSetup {
    pub fn validate(&self) -> Result<()> {
        if (!self.query_approved && !self.memory_approved)
            || !(LocalQueryConfig {
                endpoint: self.endpoint.clone(),
                scopes: vec!["personal".into()],
                max_local_calls: 1,
                max_input_bytes: 65536,
            })
            .valid()
            || !bounded_string(&self.model, 256, false)
            || !bounded_string(&self.revision, 256, false)
            || !(1..=8192).contains(&self.dimensions)
            || !bounded_string(&self.query_instruction, 2048, true)
            || !bounded_string(&self.document_instruction, 2048, true)
            || self.model.chars().any(char::is_control)
            || self.revision.chars().any(char::is_control)
        {
            return Err("本机语义配置无效；需数字回环接口、明确模型/维度/版本，并确认查询或 Memory 编码范围".into());
        }
        Ok(())
    }
}
pub(crate) fn managed_local_setup(
    vault: &crate::Vault,
    scope: &str,
    request: &LocalSemanticSetup,
    python: PathBuf,
    python_path: PathBuf,
) -> Result<PathBuf> {
    request.validate()?;
    validate_scope(scope)?;
    if !python_path.join("recallcard_worker/__main__.py").is_file() {
        return Err("未找到随应用安装的向量 worker；请补齐 Python 组件后重试".into());
    }
    let space = Space {
        provider: "local-openai-compatible".into(),
        endpoint: request.endpoint.clone(),
        model: request.model.clone(),
        dimensions: request.dimensions,
        revision: request.revision.clone(),
        document_instruction: request.document_instruction.clone(),
        query_instruction: request.query_instruction.clone(),
        normalization: "l2".into(),
        preprocessing_version: "utf8-v1".into(),
    };
    let (path, mut config) = seed_space(vault, scope, space, python, python_path)?;
    config.local_query = request.query_approved.then(|| LocalQueryConfig {
        endpoint: request.endpoint.clone(),
        scopes: vec![scope.into()],
        max_local_calls: 20,
        max_input_bytes: 65536,
    });
    config.local_index = request.memory_approved.then(|| LocalIndexConfig {
        endpoint: request.endpoint.clone(),
        scopes: vec![scope.into()],
        max_documents_per_step: 4,
        max_input_bytes: 65536,
    });
    SemanticSearch::new(config.clone())?;
    vault.write_replace(&path, &config)?;
    Ok(path)
}
fn seed_space(
    vault: &crate::Vault,
    scope: &str,
    space: Space,
    python: PathBuf,
    python_path: PathBuf,
) -> Result<(PathBuf, SemanticConfig)> {
    let signature = hash(
        &serde_json::to_vec(&serde_json::to_value(&space).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?,
    );
    let parent = vault.state_dir()?.join("semantic");
    crate::vault::reject_symlink(&parent)?;
    std::fs::create_dir_all(&parent).map_err(|e| e.to_string())?;
    let dir = parent.join(hash(format!("{signature}\n{scope}").as_bytes()));
    crate::vault::reject_symlink(&dir)?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let index_path = dir.join("index.json");
    let query_cache_path = dir.join("queries.json");
    if index_path.exists() {
        let index: Index =
            read_local(&index_path).map_err(|_| "已有同空间索引不可安全读取；没有覆盖它")?;
        index
            .validate_shape(&signature, false)
            .map_err(|_| "已有索引空间不兼容；没有覆盖它")?;
        if index.scope != [scope] {
            return Err("已有索引范围不兼容；没有覆盖它".into());
        }
    } else {
        let index = Index {
            schema: "recallcard.embedding-index/1".into(),
            space,
            space_signature: signature.clone(),
            generation: corpus_generation(&[]),
            scope: vec![scope.into()],
            documents: vec![],
            complete: true,
        };
        index.validate(&signature).map_err(|_| "初始模型空间无效")?;
        vault.write_new(&index_path, &index)?;
    }
    if !query_cache_path.exists() {
        vault.write_new(
            &query_cache_path,
            &QueryCache {
                schema: "recallcard.embedding-query-cache/1".into(),
                space_signature: signature.clone(),
                queries: vec![],
            },
        )?;
    }
    let config = SemanticConfig {
        python,
        python_path,
        index_path,
        expected_space_signature: signature,
        query_cache_path: Some(query_cache_path),
        cloud_query: None,
        local_query: None,
        local_index: None,
        timeout_ms: 30000,
    };
    Ok((dir.join("config.json"), config))
}
pub(crate) fn managed_cloud_setup(
    vault: &crate::Vault,
    scope: &str,
    request: &crate::application::embedding_cloud::CloudEmbeddingSettings,
    python: PathBuf,
    python_path: PathBuf,
) -> Result<PathBuf> {
    request.validate().map_err(|e| e.to_string())?;
    validate_scope(scope)?;
    if !python_path.join("recallcard_worker/__main__.py").is_file() {
        return Err("随应用安装的向量 worker 不可用".into());
    }
    let space = Space {
        provider: "managed-cloud-openai-compatible".into(),
        endpoint: request.endpoint.clone(),
        model: request.model.clone(),
        dimensions: request.dimensions,
        revision: request.revision.clone(),
        document_instruction: request.document_instruction.clone(),
        query_instruction: request.query_instruction.clone(),
        normalization: "l2".into(),
        preprocessing_version: "utf8-v1".into(),
    };
    let (path, mut config) = seed_space(vault, scope, space, python, python_path)?;
    config.cloud_query = request.query_approved.then(|| CloudQueryApproval {
        endpoint: request.endpoint.clone(),
        scopes: vec![scope.into()],
        key_env: "RECALLCARD_MANAGED_EMBEDDING_KEY".into(),
        max_network_calls: 1,
        max_transmitted_bytes: 4 * 1024 * 1024,
    });
    SemanticSearch::new(config.clone())?;
    let grant = crate::application::embedding_cloud::CloudEmbeddingRuntime::new(vault)
        .configure(scope, request.clone())
        .map_err(|e| e.to_string())?;
    managed_cloud::write_config(vault, &path, grant, config)?;
    Ok(path)
}
pub(crate) fn local_setup_snapshot(path: &Path, scope: &str) -> Result<LocalSemanticSetup> {
    let backend = SemanticSearch::from_offline_config_file(path)?;
    let config = backend.config;
    let index: Index = read_local(&config.index_path).map_err(|_| "模型空间暂不可读取")?;
    index
        .validate_shape(&config.expected_space_signature, false)
        .map_err(|_| "模型空间暂不可核验")?;
    if index.scope != [scope] {
        return Err("模型空间不属于当前后台范围".into());
    }
    Ok(LocalSemanticSetup {
        endpoint: index.space.endpoint,
        model: index.space.model,
        dimensions: index.space.dimensions,
        revision: index.space.revision,
        query_instruction: index.space.query_instruction,
        document_instruction: index.space.document_instruction,
        query_approved: config.local_query.is_some(),
        memory_approved: config.local_index.is_some(),
        resume_background: false,
    })
}
