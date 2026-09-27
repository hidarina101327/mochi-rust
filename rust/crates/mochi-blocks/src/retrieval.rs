//! 向量查询使用 sqlite-vec 的 vec0，不退回 Rust 距离计算。
//! 其 C 入口实际接受三个参数；register_sqlite_vec 负责 ABI 转换，建表前校验 vec_version()。

use std::collections::HashMap;
use std::ffi::{c_char, c_void, CStr};
use std::fmt;
use std::path::Path;
use std::sync::OnceLock;
use std::time::Duration;

use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;
use sqlite_vec::sqlite3_vec_init;

/// 选用的 sqlite-vec 版本：发布包包含完整源码，也能在项目使用的 Windows/MSVC 工具链下编译。
///
/// 不选 `0.1.10-alpha.4`：它发布的 `sqlite-vec.c` 引用了包内缺失的源码文件。
/// 稳定版 `0.1.9` 的发布包则是完整的。
pub const SQLITE_VEC_CRATE_VERSION: &str = "0.1.9";

// 版本 2 保存分词后的 FTS 文本。原文仍在 `mochi_block_records.text` 中，
// 因此打开旧索引时可以重建 FTS 文本，不会丢失原文。
const SCHEMA_VERSION: &str = "2";
const META_SCHEMA_VERSION: &str = "schema_version";
const META_DIMENSIONS: &str = "dimensions";
const META_MODEL_VERSION: &str = "model_version";
const DEFAULT_RRF_K: f64 = 60.0;
const DEFAULT_CANDIDATE_LIMIT: usize = 64;

/// 块的稳定标识，以及生成向量时对应的内容版本。
///
/// `document_path` 按调用方提供的 UTF-8 路径原样保存。使用 `PathBuf` 的调用方
/// 应先将路径转换为稳定、跨平台的形式。`(document_path, block_id)` 是持久主键；
/// 文本一旦变化，`content_revision` 也必须变化，避免悄悄复用旧向量。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BlockRecord {
    pub document_path: String,
    pub block_id: String,
    pub text: String,
    pub content_revision: String,
}

impl BlockRecord {
    pub fn new(
        document_path: impl Into<String>,
        block_id: impl Into<String>,
        text: impl Into<String>,
        content_revision: impl Into<String>,
    ) -> Self {
        Self {
            document_path: document_path.into(),
            block_id: block_id.into(),
            text: text.into(),
            content_revision: content_revision.into(),
        }
    }
}

/// 索引使用的模型约束。存储和查询的向量都必须符合这里的维度和模型版本。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RetrievalConfig {
    pub dimensions: usize,
    pub model_version: String,
}

impl RetrievalConfig {
    pub fn new(dimensions: usize, model_version: impl Into<String>) -> Result<Self> {
        let config = Self {
            dimensions,
            model_version: model_version.into(),
        };
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<()> {
        if self.dimensions == 0 {
            return Err(RetrievalError::InvalidInput(
                "embedding dimensions must be greater than zero".into(),
            ));
        }
        if self.model_version.trim().is_empty() {
            return Err(RetrievalError::InvalidInput(
                "embedding model version must not be empty".into(),
            ));
        }
        validate_no_nul("model version", &self.model_version)
    }
}

/// 混合查询的候选数量上限和最终返回的块数。默认会让两路排序列表各自多取一些结果，
/// 使 RRF 也能找回只出现在其中一路的块。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SearchOptions {
    pub limit: usize,
    pub text_candidates: usize,
    pub vector_candidates: usize,
    pub rrf_k: f64,
}

impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            limit: 10,
            text_candidates: DEFAULT_CANDIDATE_LIMIT,
            vector_candidates: DEFAULT_CANDIDATE_LIMIT,
            rrf_k: DEFAULT_RRF_K,
        }
    }
}

impl SearchOptions {
    pub fn new(limit: usize) -> Self {
        Self {
            limit,
            ..Self::default()
        }
    }

    fn validate(self) -> Result<Self> {
        if !self.rrf_k.is_finite() || self.rrf_k <= 0.0 {
            return Err(RetrievalError::InvalidInput(
                "RRF k must be a finite positive number".into(),
            ));
        }
        Ok(self)
    }
}

/// [`BlockRetriever::hybrid_search`] 返回的一条结果。
///
/// `score` 是各路倒数排名得分之和。分别保留排名和距离，方便调用方判断结果来自
/// FTS、向量检索，还是两者都有；这些值不能用来衡量模型质量。
#[derive(Clone, Debug, PartialEq)]
pub struct SearchResult {
    pub document_path: String,
    pub block_id: String,
    pub text: String,
    pub content_revision: String,
    pub score: f64,
    pub text_rank: Option<usize>,
    pub vector_rank: Option<usize>,
    pub vector_distance: Option<f64>,
}

/// 检索存储层或向量服务校验时产生的错误。
#[derive(Debug)]
pub enum RetrievalError {
    Sqlite(rusqlite::Error),
    InvalidInput(String),
    InvalidVector { index: usize, value: f32 },
    DimensionMismatch { expected: usize, actual: usize },
    ModelMismatch { expected: String, actual: String },
    ExtensionRegistration(String),
    ExtensionUnavailable(String),
    MetadataCorrupt(String),
    Embedding(String),
}

impl fmt::Display for RetrievalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sqlite(error) => write!(f, "SQLite error: {error}"),
            Self::InvalidInput(message) => write!(f, "invalid retrieval input: {message}"),
            Self::InvalidVector { index, value } => {
                write!(f, "embedding element {index} is not finite: {value:?}")
            }
            Self::DimensionMismatch { expected, actual } => {
                write!(
                    f,
                    "embedding dimension mismatch: expected {expected}, got {actual}"
                )
            }
            Self::ModelMismatch { expected, actual } => {
                write!(
                    f,
                    "embedding model mismatch: expected {expected}, got {actual}"
                )
            }
            Self::ExtensionRegistration(message) => {
                write!(f, "sqlite-vec registration failed: {message}")
            }
            Self::ExtensionUnavailable(message) => {
                write!(f, "sqlite-vec is unavailable: {message}")
            }
            Self::MetadataCorrupt(message) => write!(f, "retrieval metadata is corrupt: {message}"),
            Self::Embedding(message) => write!(f, "embedding provider error: {message}"),
        }
    }
}

impl std::error::Error for RetrievalError {}

impl From<rusqlite::Error> for RetrievalError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error)
    }
}

/// 本模块使用的结果类型。
pub type Result<T> = std::result::Result<T, RetrievalError>;

/// 向量服务由调用方显式配置。创建服务实例不会发起网络请求；只有 `embed` 可能访问远端。
///
/// 使用向量服务建立索引并查询的最简流程：
///
/// ```no_run
/// # use mochi_blocks::retrieval::{
/// #     BlockRecord, BlockRetriever, OpenAiCompatibleEmbeddings, RetrievalConfig,
/// #     SearchOptions,
/// # };
/// # fn run() -> mochi_blocks::retrieval::Result<()> {
/// let config = RetrievalConfig::new(1536, "text-embedding-3-small")?;
/// let index = BlockRetriever::open("blocks.sqlite3", config)?;
/// let provider = OpenAiCompatibleEmbeddings::new(
///     "https://api.openai.com/v1/embeddings",
///     "text-embedding-3-small",
///     1536,
/// )?;
/// index.upsert_with_provider(
///     &BlockRecord::new(
///         "notes/today.md",
///         "block_00000000-0000-4000-8000-000000000001",
///         "A note",
///         "revision-1",
///     ),
///     &provider,
/// )?;
/// let hits = index.hybrid_search_with_provider("note", &provider, SearchOptions::new(8))?;
/// # let _ = hits;
/// # Ok(())
/// # }
/// ```
///
/// 示例没有提供密钥，因此单独运行时无法完成授权。调用方取得令牌后，
/// 可以通过 `with_api_key` 将其传给向量服务。
pub trait EmbeddingProvider {
    fn model_version(&self) -> &str;
    fn dimensions(&self) -> usize;
    fn embed(&self, input: &str) -> std::result::Result<Vec<f32>, EmbeddingError>;
}

/// [`EmbeddingProvider`] 返回的错误。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EmbeddingError(pub String);

impl fmt::Display for EmbeddingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for EmbeddingError {}

impl From<EmbeddingError> for RetrievalError {
    fn from(error: EmbeddingError) -> Self {
        Self::Embedding(error.0)
    }
}

/// 兼容 OpenAI `/embeddings` 接口的客户端。
///
/// `endpoint` 可以是完整的 `/embeddings` 地址，也可以是
/// `https://example.test/v1` 这样的 API 基础地址；后一种会自动补上
/// `/embeddings`。客户端不会默认读取环境变量中的密钥。调用方取得密钥后，
/// 可通过 [`OpenAiCompatibleEmbeddings::with_api_key`] 传入。
#[derive(Clone)]
pub struct OpenAiCompatibleEmbeddings {
    endpoint: String,
    model: String,
    dimensions: usize,
    api_key: Option<String>,
    agent: ureq::Agent,
}

impl fmt::Debug for OpenAiCompatibleEmbeddings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpenAiCompatibleEmbeddings")
            .field("endpoint", &self.endpoint)
            .field("model", &self.model)
            .field("dimensions", &self.dimensions)
            .field("api_key", &self.api_key.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

impl OpenAiCompatibleEmbeddings {
    pub fn new(
        endpoint: impl Into<String>,
        model: impl Into<String>,
        dimensions: usize,
    ) -> std::result::Result<Self, EmbeddingError> {
        let endpoint = normalize_embeddings_endpoint(&endpoint.into())?;
        let model = model.into();
        if model.trim().is_empty() {
            return Err(EmbeddingError("embedding model must not be empty".into()));
        }
        if dimensions == 0 {
            return Err(EmbeddingError(
                "embedding dimensions must be greater than zero".into(),
            ));
        }
        validate_no_nul_embedding("model", &model)?;
        Ok(Self {
            endpoint,
            model,
            dimensions,
            api_key: None,
            agent: embeddings_agent(Duration::from_secs(30)),
        })
    }

    /// 设置调用方提供的 Bearer 令牌。`Debug` 输出和错误信息不会包含该令牌。
    pub fn with_api_key(mut self, api_key: impl Into<String>) -> Self {
        self.api_key = Some(api_key.into());
        self
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }
}

impl EmbeddingProvider for OpenAiCompatibleEmbeddings {
    fn model_version(&self) -> &str {
        &self.model
    }

    fn dimensions(&self) -> usize {
        self.dimensions
    }

    fn embed(&self, input: &str) -> std::result::Result<Vec<f32>, EmbeddingError> {
        if input.contains('\0') {
            return Err(EmbeddingError("embedding input contains NUL".into()));
        }
        let payload = serde_json::json!({
            "input": input,
            "model": self.model,
        });
        let mut request = self
            .agent
            .post(&self.endpoint)
            .header("Content-Type", "application/json");
        let authorization;
        if let Some(api_key) = self.api_key.as_deref() {
            authorization = format!("Bearer {api_key}");
            request = request.header("Authorization", &authorization);
        }
        let mut response = request
            .send_json(&payload)
            .map_err(|error| EmbeddingError(format!("HTTP request failed: {error}")))?;
        let status = response.status();
        let body = response.body_mut().read_to_string().map_err(|error| {
            EmbeddingError(format!("reading embedding response failed: {error}"))
        })?;
        if !status.is_success() {
            return Err(EmbeddingError(format!(
                "embedding endpoint returned HTTP {}: {}",
                status.as_u16(),
                truncate_for_error(&body),
            )));
        }
        parse_openai_embedding_response(&body, &self.model, self.dimensions)
    }
}

fn embeddings_agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        // 保留一段响应正文，方便生成有用且长度受限的错误信息。
        .http_status_as_error(false)
        .build()
        .into()
}

fn normalize_embeddings_endpoint(endpoint: &str) -> std::result::Result<String, EmbeddingError> {
    let endpoint = endpoint.trim().trim_end_matches('/');
    if endpoint.is_empty() {
        return Err(EmbeddingError(
            "embedding endpoint must not be empty".into(),
        ));
    }
    validate_no_nul_embedding("endpoint", endpoint)?;
    let lower = endpoint.to_ascii_lowercase();
    if !(lower.starts_with("https://") || lower.starts_with("http://")) {
        return Err(EmbeddingError(
            "embedding endpoint must use http:// or https://".into(),
        ));
    }
    // 不接受藏在 URL 中的凭据。认证信息应放在显式的 Bearer 令牌字段里，
    // 避免 URL 被记录时泄露凭据。
    let authority = endpoint
        .split_once("://")
        .map(|(_, rest)| rest.split('/').next().unwrap_or(rest))
        .unwrap_or_default();
    if authority.contains('@') {
        return Err(EmbeddingError(
            "embedding endpoint must not contain URL credentials".into(),
        ));
    }
    if lower.ends_with("/embeddings") {
        Ok(endpoint.to_owned())
    } else {
        Ok(format!("{endpoint}/embeddings"))
    }
}

fn parse_openai_embedding_response(
    body: &str,
    expected_model: &str,
    expected_dimensions: usize,
) -> std::result::Result<Vec<f32>, EmbeddingError> {
    let value: Value = serde_json::from_str(body)
        .map_err(|error| EmbeddingError(format!("invalid embedding JSON: {error}")))?;
    if let Some(model) = value.get("model").and_then(Value::as_str) {
        if model != expected_model {
            return Err(EmbeddingError(format!(
                "embedding response model mismatch: expected {expected_model}, got {model}"
            )));
        }
    }
    let embedding = value
        .get("data")
        .and_then(Value::as_array)
        .and_then(|data| data.first())
        .and_then(|item| item.get("embedding"))
        .and_then(Value::as_array)
        .ok_or_else(|| EmbeddingError("embedding response has no data[0].embedding".into()))?;
    if embedding.len() != expected_dimensions {
        return Err(EmbeddingError(format!(
            "embedding response dimension mismatch: expected {expected_dimensions}, got {}",
            embedding.len()
        )));
    }
    let mut values = Vec::with_capacity(embedding.len());
    for (index, value) in embedding.iter().enumerate() {
        let number = value
            .as_f64()
            .ok_or_else(|| EmbeddingError(format!("embedding element {index} is not a number")))?;
        let number = number as f32;
        if !number.is_finite() {
            return Err(EmbeddingError(format!(
                "embedding element {index} is not finite"
            )));
        }
        values.push(number);
    }
    Ok(values)
}

fn validate_no_nul(name: &str, value: &str) -> Result<()> {
    if value.contains('\0') {
        return Err(RetrievalError::InvalidInput(format!("{name} contains NUL")));
    }
    Ok(())
}

fn validate_no_nul_embedding(name: &str, value: &str) -> std::result::Result<(), EmbeddingError> {
    if value.contains('\0') {
        return Err(EmbeddingError(format!("{name} contains NUL")));
    }
    Ok(())
}

fn validate_record(record: &BlockRecord) -> Result<()> {
    if record.document_path.trim().is_empty() {
        return Err(RetrievalError::InvalidInput(
            "document path must not be empty".into(),
        ));
    }
    if record.block_id.trim().is_empty() {
        return Err(RetrievalError::InvalidInput(
            "block id must not be empty".into(),
        ));
    }
    if record.content_revision.trim().is_empty() {
        return Err(RetrievalError::InvalidInput(
            "content revision must not be empty".into(),
        ));
    }
    validate_no_nul("document path", &record.document_path)?;
    validate_no_nul("block id", &record.block_id)?;
    validate_no_nul("text", &record.text)?;
    validate_no_nul("content revision", &record.content_revision)
}

fn validate_vector(values: &[f32], dimensions: usize) -> Result<()> {
    if values.len() != dimensions {
        return Err(RetrievalError::DimensionMismatch {
            expected: dimensions,
            actual: values.len(),
        });
    }
    for (index, value) in values.iter().copied().enumerate() {
        if !value.is_finite() {
            return Err(RetrievalError::InvalidVector { index, value });
        }
    }
    Ok(())
}

/// 将向量编码为 sqlite-vec 使用的紧凑 float32 格式。扩展的 Rust 示例直接传入
/// 本机字节序；项目支持的 Windows 目标为小端序，这里显式指定小端序，
/// 让持久化索引的字节表示保持一致。
fn vector_bytes(values: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(std::mem::size_of_val(values));
    for value in values {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes
}

static SQLITE_VEC_REGISTERED: OnceLock<std::result::Result<(), String>> = OnceLock::new();

/// 为之后打开的 SQLite 连接注册静态链接的 sqlite-vec 入口。
pub fn register_sqlite_vec() -> Result<()> {
    let result = SQLITE_VEC_REGISTERED.get_or_init(|| unsafe {
        // 绑定库将 sqlite_vec::sqlite3_vec_init 声明为 extern "C" fn()，
        // 但 C 符号实际上使用 SQLite 扩展入口的三个参数。这里按 sqlite-vec
        // 官方 rusqlite 示例转换 ABI，与 rusqlite 0.40 的 RawAutoExtension 匹配。
        let raw: rusqlite::auto_extension::RawAutoExtension =
            std::mem::transmute(sqlite3_vec_init as *const ());
        rusqlite::auto_extension::register_auto_extension(raw).map_err(|error| error.to_string())
    });
    result
        .clone()
        .map_err(RetrievalError::ExtensionRegistration)
}

fn connection_has_sqlite_vec(connection: &Connection) -> bool {
    connection
        .query_row::<String, _, _>("SELECT vec_version()", [], |row| row.get(0))
        .is_ok()
}

/// 在全局自动注册之前打开的连接上初始化 sqlite-vec。`from_connection` 需要这样处理；
/// 普通构造函数会先注册，再走 SQLite 的自动扩展流程。
fn initialize_connection_sqlite_vec(connection: &Connection) -> Result<()> {
    if connection_has_sqlite_vec(connection) {
        return Ok(());
    }
    register_sqlite_vec()?;
    if connection_has_sqlite_vec(connection) {
        return Ok(());
    }

    let mut error_message: *mut c_char = std::ptr::null_mut();
    let raw: rusqlite::auto_extension::RawAutoExtension =
        unsafe { std::mem::transmute(sqlite3_vec_init as *const ()) };
    let rc = unsafe {
        // `sqlite_vec` 使用 SQLITE_CORE 编译，因此 pApi 特意传入空指针。
        // pzErrMsg 必须可写，扩展失败时可能在此分配错误说明。
        raw(connection.handle(), &mut error_message, std::ptr::null())
    };
    if rc != rusqlite::ffi::SQLITE_OK {
        let message = if error_message.is_null() {
            format!("extension entry point returned SQLite code {rc}")
        } else {
            let message = unsafe { CStr::from_ptr(error_message) }
                .to_string_lossy()
                .into_owned();
            unsafe { rusqlite::ffi::sqlite3_free(error_message.cast::<c_void>()) };
            message
        };
        return Err(RetrievalError::ExtensionUnavailable(message));
    }
    if connection_has_sqlite_vec(connection) {
        Ok(())
    } else {
        Err(RetrievalError::ExtensionUnavailable(
            "entry point returned success but vec_version() is missing".into(),
        ))
    }
}

/// 基于 SQLite 的块索引。此结构持有连接；返回实例前会确保数据库结构和向量扩展均已就绪。
pub struct BlockRetriever {
    connection: Connection,
    config: RetrievalConfig,
}

impl fmt::Debug for BlockRetriever {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BlockRetriever")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl BlockRetriever {
    /// 注册静态链接的扩展后，打开内存索引。
    pub fn open_in_memory(config: RetrievalConfig) -> Result<Self> {
        config.validate()?;
        register_sqlite_vec()?;
        let connection = Connection::open_in_memory()?;
        initialize_connection_sqlite_vec(&connection)?;
        Self::from_initialized_connection(connection, config)
    }

    /// 注册静态链接的扩展后，打开持久化索引。如果配置的维度或模型版本变化，
    /// 已有向量会自动失效。
    pub fn open(path: impl AsRef<Path>, config: RetrievalConfig) -> Result<Self> {
        config.validate()?;
        register_sqlite_vec()?;
        let connection = Connection::open(path)?;
        initialize_connection_sqlite_vec(&connection)?;
        Self::from_initialized_connection(connection, config)
    }

    /// 接管调用方提供的连接。如果连接创建于全局注册之前，先检查 `vec_version()`，
    /// 再直接在此连接上初始化扩展入口。
    pub fn from_connection(connection: Connection, config: RetrievalConfig) -> Result<Self> {
        config.validate()?;
        initialize_connection_sqlite_vec(&connection)?;
        Self::from_initialized_connection(connection, config)
    }

    fn from_initialized_connection(
        connection: Connection,
        config: RetrievalConfig,
    ) -> Result<Self> {
        if !connection_has_sqlite_vec(&connection) {
            return Err(RetrievalError::ExtensionUnavailable(
                "vec_version() is not available".into(),
            ));
        }
        let retriever = Self { connection, config };
        retriever.initialize_schema()?;
        Ok(retriever)
    }

    pub fn config(&self) -> &RetrievalConfig {
        &self.config
    }

    /// 已加载的 sqlite-vec 版本，也可用来快速确认向量扩展已启用。
    pub fn sqlite_vec_version(&self) -> Result<String> {
        Ok(self
            .connection
            .query_row("SELECT vec_version()", [], |row| row.get(0))?)
    }

    /// 在同一事务中插入或替换块及其向量。替换时会一并删除旧的 FTS 和向量记录，
    /// 避免 `content_revision` 变化后仍留下过期向量。
    pub fn upsert(&self, record: &BlockRecord, embedding: &[f32]) -> Result<()> {
        validate_record(record)?;
        validate_vector(embedding, self.config.dimensions)?;
        let bytes = vector_bytes(embedding);
        let transaction = self.connection.unchecked_transaction()?;

        let existing_id: Option<i64> = transaction
            .query_row(
                "SELECT id FROM mochi_block_records WHERE document_path = ?1 AND block_id = ?2",
                params![record.document_path, record.block_id],
                |row| row.get(0),
            )
            .optional()?;
        let id = if let Some(id) = existing_id {
            transaction.execute(
                "UPDATE mochi_block_records
                 SET text = ?1, content_revision = ?2, embedding_model_version = ?3,
                     embedding_dimensions = ?4
                 WHERE id = ?5",
                params![
                    record.text,
                    record.content_revision,
                    self.config.model_version,
                    self.config.dimensions as i64,
                    id,
                ],
            )?;
            id
        } else {
            transaction.execute(
                "INSERT INTO mochi_block_records
                 (document_path, block_id, text, content_revision,
                  embedding_model_version, embedding_dimensions)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    record.document_path,
                    record.block_id,
                    record.text,
                    record.content_revision,
                    self.config.model_version,
                    self.config.dimensions as i64,
                ],
            )?;
            transaction.last_insert_rowid()
        };

        transaction.execute("DELETE FROM mochi_block_fts WHERE rowid = ?1", params![id])?;
        transaction.execute(
            "INSERT INTO mochi_block_fts
             (rowid, text, document_path, block_id, content_revision)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                id,
                fts_text_projection(&record.text),
                record.document_path,
                record.block_id,
                record.content_revision,
            ],
        )?;
        transaction.execute(
            "DELETE FROM mochi_block_embeddings WHERE rowid = ?1",
            params![id],
        )?;
        transaction.execute(
            "INSERT INTO mochi_block_embeddings (rowid, embedding) VALUES (?1, ?2)",
            params![id, bytes.as_slice()],
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// 使用调用方指定的向量服务生成向量并写入块。发起网络请求前会先检查模型和维度。
    pub fn upsert_with_provider(
        &self,
        record: &BlockRecord,
        provider: &dyn EmbeddingProvider,
    ) -> Result<()> {
        self.validate_provider(provider)?;
        let embedding = provider.embed(&record.text)?;
        self.upsert(record, &embedding)
    }

    fn validate_provider(&self, provider: &dyn EmbeddingProvider) -> Result<()> {
        if provider.dimensions() != self.config.dimensions {
            return Err(RetrievalError::DimensionMismatch {
                expected: self.config.dimensions,
                actual: provider.dimensions(),
            });
        }
        if provider.model_version() != self.config.model_version {
            return Err(RetrievalError::ModelMismatch {
                expected: self.config.model_version.clone(),
                actual: provider.model_version().to_owned(),
            });
        }
        Ok(())
    }

    /// 删除一个块及其所有检索记录，并返回该块原先是否存在。
    pub fn delete(&self, document_path: &str, block_id: &str) -> Result<bool> {
        validate_no_nul("document path", document_path)?;
        validate_no_nul("block id", block_id)?;
        let transaction = self.connection.unchecked_transaction()?;
        let id: Option<i64> = transaction
            .query_row(
                "SELECT id FROM mochi_block_records WHERE document_path = ?1 AND block_id = ?2",
                params![document_path, block_id],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(id) = id {
            transaction.execute("DELETE FROM mochi_block_fts WHERE rowid = ?1", params![id])?;
            transaction.execute(
                "DELETE FROM mochi_block_embeddings WHERE rowid = ?1",
                params![id],
            )?;
            transaction.execute("DELETE FROM mochi_block_records WHERE id = ?1", params![id])?;
        }
        transaction.commit()?;
        Ok(id.is_some())
    }

    pub fn delete_document(&self, document_path: &str) -> Result<usize> {
        validate_no_nul("document path", document_path)?;
        let transaction = self.connection.unchecked_transaction()?;
        let ids = {
            let mut statement = transaction
                .prepare("SELECT id FROM mochi_block_records WHERE document_path = ?1")?;
            let rows = statement
                .query_map(params![document_path], |row| row.get::<_, i64>(0))?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            rows
        };
        for id in &ids {
            transaction.execute("DELETE FROM mochi_block_fts WHERE rowid = ?1", params![id])?;
            transaction.execute(
                "DELETE FROM mochi_block_embeddings WHERE rowid = ?1",
                params![id],
            )?;
        }
        let deleted = transaction.execute(
            "DELETE FROM mochi_block_records WHERE document_path = ?1",
            params![document_path],
        )?;
        transaction.commit()?;
        Ok(deleted)
    }

    /// 清除全部向量记录，保留现有 FTS 和文本记录。下次成功写入时会重新生成向量记录。
    /// 更换向量服务或模型时，可用此方法显式使向量缓存失效。
    pub fn invalidate_vector_cache(&self) -> Result<()> {
        let transaction = self.connection.unchecked_transaction()?;
        transaction.execute("DELETE FROM mochi_block_embeddings", [])?;
        transaction.execute(
            "UPDATE mochi_block_records
             SET embedding_model_version = '', embedding_dimensions = 0",
            [],
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// 分别搜索 FTS5 和 sqlite-vec 的近邻候选，再用标准倒数排名融合合并两路结果。
    /// 向量候选通过 sqlite-vec 的 `MATCH` 运算符获取；这里不会退回到 Rust
    /// 实现的余弦相似度或点积计算。
    pub fn hybrid_search(
        &self,
        text_query: &str,
        query_embedding: &[f32],
        options: SearchOptions,
    ) -> Result<Vec<SearchResult>> {
        validate_no_nul("text query", text_query)?;
        validate_vector(query_embedding, self.config.dimensions)?;
        let options = options.validate()?;
        if options.limit == 0 {
            return Ok(Vec::new());
        }

        #[derive(Default)]
        struct Candidate {
            text_rank: Option<usize>,
            vector_rank: Option<usize>,
            vector_distance: Option<f64>,
        }

        let mut candidates: HashMap<i64, Candidate> = HashMap::new();
        if options.text_candidates > 0 {
            if let Some(fts_query) = fts_query(text_query) {
                let mut statement = self.connection.prepare(
                    "SELECT rowid
                     FROM mochi_block_fts
                     WHERE mochi_block_fts MATCH ?1
                     ORDER BY bm25(mochi_block_fts) ASC, rowid ASC
                     LIMIT ?2",
                )?;
                let rows = statement.query_map(
                    params![fts_query, bound_limit(options.text_candidates)],
                    |row| row.get::<_, i64>(0),
                )?;
                for (rank, row) in rows.enumerate() {
                    let id = row?;
                    candidates.entry(id).or_default().text_rank = Some(rank + 1);
                }
            }
        }

        if options.vector_candidates > 0 {
            let bytes = vector_bytes(query_embedding);
            let mut statement = self.connection.prepare(
                "SELECT rowid, distance
                 FROM mochi_block_embeddings
                 WHERE embedding MATCH ?1
                 ORDER BY distance ASC
                 LIMIT ?2",
            )?;
            let rows = statement.query_map(
                params![bytes.as_slice(), bound_limit(options.vector_candidates)],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, f64>(1)?)),
            )?;
            for (rank, row) in rows.enumerate() {
                let (id, distance) = row?;
                let candidate = candidates.entry(id).or_default();
                candidate.vector_rank = Some(rank + 1);
                candidate.vector_distance = Some(distance);
            }
        }

        let mut results = Vec::with_capacity(candidates.len());
        for (id, candidate) in candidates {
            let row = self
                .connection
                .query_row(
                    "SELECT document_path, block_id, text, content_revision,
                            embedding_model_version, embedding_dimensions
                     FROM mochi_block_records WHERE id = ?1",
                    params![id],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, String>(3)?,
                            row.get::<_, String>(4)?,
                            row.get::<_, i64>(5)?,
                        ))
                    },
                )
                .optional()?;
            let Some((
                document_path,
                block_id,
                text,
                content_revision,
                embedding_model_version,
                embedding_dimensions,
            )) = row
            else {
                continue;
            };
            // 过期或损坏的向量记录不能参与向量排序。重建期间，FTS 仍可返回对应的文本记录。
            let vector_is_current = embedding_model_version == self.config.model_version
                && embedding_dimensions == self.config.dimensions as i64;
            let vector_rank = candidate.vector_rank.filter(|_| vector_is_current);
            let vector_distance = candidate.vector_distance.filter(|_| vector_is_current);
            if candidate.text_rank.is_none() && vector_rank.is_none() {
                continue;
            }
            let score = candidate
                .text_rank
                .map(|rank| 1.0 / (options.rrf_k + rank as f64))
                .unwrap_or(0.0)
                + vector_rank
                    .map(|rank| 1.0 / (options.rrf_k + rank as f64))
                    .unwrap_or(0.0);
            results.push(SearchResult {
                document_path,
                block_id,
                text,
                content_revision,
                score,
                text_rank: candidate.text_rank,
                vector_rank,
                vector_distance,
            });
        }
        results.sort_by(|left, right| {
            right
                .score
                .total_cmp(&left.score)
                .then_with(|| left.document_path.cmp(&right.document_path))
                .then_with(|| left.block_id.cmp(&right.block_id))
        });
        results.truncate(options.limit);
        Ok(results)
    }

    /// 使用配置的向量服务生成查询向量，再执行混合查询。
    pub fn hybrid_search_with_provider(
        &self,
        text_query: &str,
        provider: &dyn EmbeddingProvider,
        options: SearchOptions,
    ) -> Result<Vec<SearchResult>> {
        self.validate_provider(provider)?;
        let embedding = provider.embed(text_query)?;
        self.hybrid_search(text_query, &embedding, options)
    }

    /// 当前存储的块数，主要用于集成验证中的更新和删除检查。
    pub fn len(&self) -> Result<usize> {
        let count: i64 =
            self.connection
                .query_row("SELECT COUNT(*) FROM mochi_block_records", [], |row| {
                    row.get(0)
                })?;
        Ok(count as usize)
    }

    pub fn is_empty(&self) -> Result<bool> {
        Ok(self.len()? == 0)
    }

    fn initialize_schema(&self) -> Result<()> {
        self.connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS mochi_block_records (
                 id INTEGER PRIMARY KEY,
                 document_path TEXT NOT NULL,
                 block_id TEXT NOT NULL,
                 text TEXT NOT NULL,
                 content_revision TEXT NOT NULL,
                 embedding_model_version TEXT NOT NULL,
                 embedding_dimensions INTEGER NOT NULL,
                 UNIQUE(document_path, block_id)
             );
             CREATE VIRTUAL TABLE IF NOT EXISTS mochi_block_fts USING fts5(
                 text,
                 document_path UNINDEXED,
                 block_id UNINDEXED,
                 content_revision UNINDEXED
             );
             CREATE TABLE IF NOT EXISTS mochi_retrieval_meta (
                 key TEXT PRIMARY KEY,
                 value TEXT NOT NULL
             );",
        )?;

        let stored_schema: Option<String> = self
            .connection
            .query_row(
                "SELECT value FROM mochi_retrieval_meta WHERE key = ?1",
                params![META_SCHEMA_VERSION],
                |row| row.get(0),
            )
            .optional()?;
        let stored_dimensions: Option<String> = self
            .connection
            .query_row(
                "SELECT value FROM mochi_retrieval_meta WHERE key = ?1",
                params![META_DIMENSIONS],
                |row| row.get(0),
            )
            .optional()?;
        let stored_model: Option<String> = self
            .connection
            .query_row(
                "SELECT value FROM mochi_retrieval_meta WHERE key = ?1",
                params![META_MODEL_VERSION],
                |row| row.get(0),
            )
            .optional()?;
        let stored_dimensions = match stored_dimensions {
            Some(value) => Some(value.parse::<usize>().map_err(|_| {
                RetrievalError::MetadataCorrupt(format!("invalid dimensions value {value:?}"))
            })?),
            None => None,
        };
        let needs_fts_rebuild = stored_schema.as_deref() != Some(SCHEMA_VERSION);
        let needs_vector_reset = needs_fts_rebuild
            || stored_dimensions != Some(self.config.dimensions)
            || stored_model.as_deref() != Some(self.config.model_version.as_str());
        if needs_vector_reset {
            // 更换向量模型后，FTS 和文本仍可使用。这里只删除 vec0 表，
            // 一次性使旧向量全部失效，再创建固定新维度的表。
            self.connection
                .execute_batch("DROP TABLE IF EXISTS mochi_block_embeddings;")?;
        }
        let create_vectors = format!(
            "CREATE VIRTUAL TABLE IF NOT EXISTS mochi_block_embeddings USING vec0(embedding float[{}]);",
            self.config.dimensions
        );
        self.connection.execute_batch(&create_vectors)?;
        if needs_fts_rebuild {
            // v1 直接索引原文。发布 v2 数据库结构前，先根据记录表重建索引；
            // 若迁移中断，元数据仍保留旧版本，下次可以重试。
            rebuild_fts_projection(&self.connection)?;
        }
        self.connection.execute(
            "INSERT INTO mochi_retrieval_meta(key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![META_SCHEMA_VERSION, SCHEMA_VERSION],
        )?;
        self.connection.execute(
            "INSERT INTO mochi_retrieval_meta(key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![META_DIMENSIONS, self.config.dimensions.to_string()],
        )?;
        self.connection.execute(
            "INSERT INTO mochi_retrieval_meta(key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![META_MODEL_VERSION, self.config.model_version],
        )?;
        Ok(())
    }
}

fn bound_limit(limit: usize) -> i64 {
    limit.min(i64::MAX as usize) as i64
}

/// SQLite 的 `unicode61` 分词器会把连续的汉字当成一个词元，因此搜索较短的中文词时，
/// 无法命中 `数据库迁移方案` 这样的长文本。拉丁文字仍按 unicode61 的常规方式处理；
/// 建立索引时，在每个汉字两侧加空格。查询也做同样转换，并作为短语搜索，
/// 这样 `数据库迁移` 及其中的 `数据`、`迁移` 都能匹配对应的连续汉字。
fn fts_text_projection(text: &str) -> String {
    let mut projection = String::with_capacity(text.len() + text.chars().count());
    for character in text.chars() {
        if is_han(character) {
            if projection
                .chars()
                .last()
                .map(|last| !last.is_whitespace())
                .unwrap_or(true)
            {
                projection.push(' ');
            }
            projection.push(character);
            projection.push(' ');
        } else {
            projection.push(character);
        }
    }
    projection
}

/// 覆盖中文常用的汉字区段，包括扩展 B 区和兼容汉字。匹配范围保持保守：
/// 标点、假名等仍交由 FTS5 按常规方式处理。
fn is_han(character: char) -> bool {
    matches!(
        character,
        '\u{3400}'..='\u{4DBF}'
            | '\u{4E00}'..='\u{9FFF}'
            | '\u{F900}'..='\u{FAFF}'
            | '\u{20000}'..='\u{2FA1F}'
    )
}

/// 在同一事务中，根据原始记录重建 FTS 检索文本。打开 v1 索引时会用到此方法；
/// v1 的 FTS `text` 列保存的是未经分词的原文。
fn rebuild_fts_projection(connection: &Connection) -> Result<()> {
    let transaction = connection.unchecked_transaction()?;
    transaction.execute("DELETE FROM mochi_block_fts", [])?;
    let records = {
        let mut statement = transaction.prepare(
            "SELECT id, text, document_path, block_id, content_revision
             FROM mochi_block_records ORDER BY id ASC",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()?
    };
    {
        let mut statement = transaction.prepare(
            "INSERT INTO mochi_block_fts
             (rowid, text, document_path, block_id, content_revision)
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )?;
        for (id, text, document_path, block_id, content_revision) in records {
            statement.execute(params![
                id,
                fts_text_projection(&text),
                document_path,
                block_id,
                content_revision,
            ])?;
        }
    }
    transaction.commit()?;
    Ok(())
}

fn fts_query(query: &str) -> Option<String> {
    let terms = query
        .split_whitespace()
        .filter(|term| !term.is_empty())
        .map(fts_text_projection)
        .map(|term| term.trim().to_owned())
        .filter(|term| !term.is_empty())
        .map(|term| format!("\"{}\"", term.replace('"', "\"\"")))
        .collect::<Vec<_>>();
    if terms.is_empty() {
        None
    } else {
        Some(terms.join(" OR "))
    }
}

fn truncate_for_error(value: &str) -> String {
    const MAX: usize = 512;
    if value.len() <= MAX {
        value.to_owned()
    } else {
        let mut end = MAX;
        // 使用项目所用 Rust 1.88 的接口，确保截断位置落在 UTF-8 字符边界上。
        while !value.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}…", &value[..end])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_text_truncation_preserves_utf8_boundaries() {
        for value in [
            "a".repeat(513),
            "中".repeat(172),
            "🦀".repeat(129),
            format!("{}中", "a".repeat(511)),
        ] {
            let truncated = truncate_for_error(&value);
            let prefix = truncated.strip_suffix('…').unwrap();
            assert!(prefix.len() <= 512);
            assert!(value.starts_with(prefix));
            assert!(prefix.len() + value[prefix.len()..].chars().next().unwrap().len_utf8() > 512);
        }
        assert_eq!(truncate_for_error(""), "");
        assert_eq!(truncate_for_error(&"a".repeat(512)), "a".repeat(512));
    }

    struct FixedProvider {
        model: String,
        values: Vec<f32>,
    }

    impl EmbeddingProvider for FixedProvider {
        fn model_version(&self) -> &str {
            &self.model
        }

        fn dimensions(&self) -> usize {
            self.values.len()
        }

        fn embed(&self, _input: &str) -> std::result::Result<Vec<f32>, EmbeddingError> {
            Ok(self.values.clone())
        }
    }

    fn record(path: &str, id: &str, text: &str, revision: &str) -> BlockRecord {
        BlockRecord::new(path, id, text, revision)
    }

    #[test]
    fn sqlite_vec_is_loaded_and_real_knn_orders_fixed_vectors() -> Result<()> {
        let index = BlockRetriever::open_in_memory(RetrievalConfig::new(2, "fixed-v1")?)?;
        assert!(index.sqlite_vec_version()?.starts_with('v'));
        index.upsert(
            &record("notes/a.md", "a", "SQLite vector indexing", "r1"),
            &[1.0, 0.0],
        )?;
        index.upsert(
            &record("notes/b.md", "b", "Rust ownership", "r1"),
            &[0.0, 1.0],
        )?;
        index.upsert(
            &record("notes/c.md", "c", "SQLite FTS search", "r1"),
            &[-1.0, 0.0],
        )?;

        let results = index.hybrid_search(
            "unmatched-term",
            &[1.0, 0.0],
            SearchOptions {
                limit: 3,
                text_candidates: 0,
                vector_candidates: 3,
                rrf_k: 60.0,
            },
        )?;
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].block_id, "a");
        assert_eq!(results[0].vector_rank, Some(1));
        assert_eq!(results[1].block_id, "b");
        assert_eq!(results[2].block_id, "c");
        // 固定样本只验证结果是否稳定，不能说明生产模型的语义检索质量。
        Ok(())
    }

    #[test]
    fn fts_and_vector_ranks_are_rrf_merged_with_stable_source() -> Result<()> {
        let index = BlockRetriever::open_in_memory(RetrievalConfig::new(2, "fixed-v1")?)?;
        index.upsert(
            &record("notes/a.md", "a", "SQLite retrieval and vectors", "r1"),
            &[1.0, 0.0],
        )?;
        index.upsert(
            &record("notes/b.md", "b", "SQLite text search", "r1"),
            &[0.0, 1.0],
        )?;
        let results = index.hybrid_search(
            "retrieval",
            &[1.0, 0.0],
            SearchOptions {
                limit: 2,
                text_candidates: 2,
                vector_candidates: 2,
                rrf_k: 60.0,
            },
        )?;
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].block_id, "a");
        assert_eq!(results[0].text_rank, Some(1));
        assert_eq!(results[0].vector_rank, Some(1));
        assert_eq!(results[0].document_path, "notes/a.md");
        assert!(results[0].score > results[1].score);
        Ok(())
    }

    #[test]
    fn revision_update_replaces_rows_and_invalidates_explicit_cache() -> Result<()> {
        let index = BlockRetriever::open_in_memory(RetrievalConfig::new(2, "fixed-v1")?)?;
        index.upsert(&record("notes/a.md", "a", "old text", "r1"), &[1.0, 0.0])?;
        index.upsert(&record("notes/a.md", "a", "new text", "r2"), &[0.0, 1.0])?;
        assert_eq!(index.len()?, 1);
        let updated = index.hybrid_search(
            "new",
            &[0.0, 1.0],
            SearchOptions {
                limit: 4,
                text_candidates: 4,
                vector_candidates: 4,
                rrf_k: 60.0,
            },
        )?;
        assert_eq!(updated[0].content_revision, "r2");
        assert_eq!(updated[0].text, "new text");

        index.invalidate_vector_cache()?;
        let fts_only = index.hybrid_search(
            "new",
            &[0.0, 1.0],
            SearchOptions {
                limit: 4,
                text_candidates: 4,
                vector_candidates: 4,
                rrf_k: 60.0,
            },
        )?;
        assert_eq!(fts_only.len(), 1);
        assert_eq!(fts_only[0].text_rank, Some(1));
        assert_eq!(fts_only[0].vector_rank, None);
        Ok(())
    }

    #[test]
    fn failed_upsert_rolls_back_record_fts_and_vector_together() -> Result<()> {
        let index = BlockRetriever::open_in_memory(RetrievalConfig::new(2, "fixed-v1")?)?;
        index.upsert(&record("notes/a.md", "a", "old text", "r1"), &[1.0, 0.0])?;
        index.connection.execute_batch(
            "CREATE TRIGGER force_block_update_failure
             BEFORE UPDATE OF text ON mochi_block_records
             BEGIN SELECT RAISE(ABORT, 'forced retrieval transaction failure'); END;",
        )?;

        let failure = index.upsert(&record("notes/a.md", "a", "new text", "r2"), &[0.0, 1.0]);
        assert!(matches!(failure, Err(RetrievalError::Sqlite(_))));
        index
            .connection
            .execute_batch("DROP TRIGGER force_block_update_failure;")?;

        let results = index.hybrid_search(
            "old",
            &[1.0, 0.0],
            SearchOptions {
                limit: 4,
                text_candidates: 4,
                vector_candidates: 4,
                rrf_k: 60.0,
            },
        )?;
        assert_eq!(index.len()?, 1);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].content_revision, "r1");
        assert_eq!(results[0].text, "old text");
        assert_eq!(results[0].vector_rank, Some(1));
        Ok(())
    }

    #[test]
    fn rejects_nan_wrong_dimensions_and_provider_contract_mismatch() -> Result<()> {
        let index = BlockRetriever::open_in_memory(RetrievalConfig::new(2, "fixed-v1")?)?;
        let item = record("notes/a.md", "a", "text", "r1");
        assert!(matches!(
            index.upsert(&item, &[f32::NAN, 0.0]),
            Err(RetrievalError::InvalidVector { index: 0, .. })
        ));
        assert!(matches!(
            index.upsert(&item, &[1.0]),
            Err(RetrievalError::DimensionMismatch {
                expected: 2,
                actual: 1
            })
        ));
        assert_eq!(index.len()?, 0);
        let provider = FixedProvider {
            model: "other-v1".into(),
            values: vec![1.0, 0.0],
        };
        assert!(matches!(
            index.upsert_with_provider(&item, &provider),
            Err(RetrievalError::ModelMismatch { .. })
        ));
        Ok(())
    }

    #[test]
    fn compatible_provider_is_lazy_and_endpoint_is_normalized() -> Result<()> {
        let provider = OpenAiCompatibleEmbeddings::new("https://example.test/v1", "fixed-v1", 2)
            .map_err(RetrievalError::from)?;
        assert_eq!(provider.endpoint(), "https://example.test/v1/embeddings");
        assert_eq!(provider.model_version(), "fixed-v1");
        assert_eq!(provider.dimensions(), 2);
        Ok(())
    }

    #[test]
    fn persistent_model_change_drops_old_vectors_but_keeps_fts() -> Result<()> {
        let directory =
            tempfile::tempdir().map_err(|e| RetrievalError::InvalidInput(e.to_string()))?;
        let path = directory.path().join("blocks.sqlite3");
        {
            let index = BlockRetriever::open(&path, RetrievalConfig::new(2, "model-a")?)?;
            index.upsert(&record("notes/a.md", "a", "kept text", "r1"), &[1.0, 0.0])?;
        }
        let index = BlockRetriever::open(&path, RetrievalConfig::new(2, "model-b")?)?;
        let results = index.hybrid_search(
            "kept",
            &[1.0, 0.0],
            SearchOptions {
                limit: 4,
                text_candidates: 4,
                vector_candidates: 4,
                rrf_k: 60.0,
            },
        )?;
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].text_rank, Some(1));
        assert_eq!(results[0].vector_rank, None);
        Ok(())
    }

    #[test]
    fn chinese_keyword_search_uses_substring_tokens_and_preserves_source_text() -> Result<()> {
        let index = BlockRetriever::open_in_memory(RetrievalConfig::new(2, "fixed-v1")?)?;
        let source = "数据库迁移方案";
        index.upsert(&record("notes/zh.md", "zh-1", source, "r1"), &[1.0, 0.0])?;

        for query in ["数据库迁移", "数据", "迁移"] {
            let results = index.hybrid_search(
                query,
                &[0.0, 1.0],
                SearchOptions {
                    limit: 4,
                    text_candidates: 4,
                    vector_candidates: 0,
                    rrf_k: 60.0,
                },
            )?;
            assert_eq!(results.len(), 1, "query {query:?}");
            assert_eq!(results[0].block_id, "zh-1");
            assert_eq!(results[0].text, source);
            assert_eq!(results[0].text_rank, Some(1));
            assert_eq!(results[0].vector_rank, None);
        }
        Ok(())
    }

    #[test]
    fn v1_persistent_fts_rows_are_rebuilt_with_chinese_projection() -> Result<()> {
        let directory =
            tempfile::tempdir().map_err(|e| RetrievalError::InvalidInput(e.to_string()))?;
        let path = directory.path().join("blocks.sqlite3");
        let source = "数据库迁移方案";
        {
            let index = BlockRetriever::open(&path, RetrievalConfig::new(2, "fixed-v1")?)?;
            index.upsert(&record("notes/zh.md", "zh-1", source, "r1"), &[1.0, 0.0])?;
            let id: i64 = index.connection.query_row(
                "SELECT id FROM mochi_block_records WHERE document_path = ?1 AND block_id = ?2",
                params!["notes/zh.md", "zh-1"],
                |row| row.get(0),
            )?;
            // 模拟磁盘上的 v1 索引：FTS 记录直接保存原文，元数据仍标记为版本 1。
            index
                .connection
                .execute("DELETE FROM mochi_block_fts WHERE rowid = ?1", params![id])?;
            index.connection.execute(
                "INSERT INTO mochi_block_fts
                 (rowid, text, document_path, block_id, content_revision)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![id, source, "notes/zh.md", "zh-1", "r1"],
            )?;
            index.connection.execute(
                "UPDATE mochi_retrieval_meta SET value = '1' WHERE key = ?1",
                params![META_SCHEMA_VERSION],
            )?;
        }

        let index = BlockRetriever::open(&path, RetrievalConfig::new(2, "fixed-v1")?)?;
        let results = index.hybrid_search(
            "数据",
            &[0.0, 1.0],
            SearchOptions {
                limit: 4,
                text_candidates: 4,
                vector_candidates: 0,
                rrf_k: 60.0,
            },
        )?;
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].text, source);
        assert_eq!(results[0].text_rank, Some(1));
        assert_eq!(results[0].vector_rank, None);
        let schema: String = index.connection.query_row(
            "SELECT value FROM mochi_retrieval_meta WHERE key = ?1",
            params![META_SCHEMA_VERSION],
            |row| row.get(0),
        )?;
        assert_eq!(schema, SCHEMA_VERSION);
        Ok(())
    }
}
