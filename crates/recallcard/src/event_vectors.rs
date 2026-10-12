//! Offline raw-event vectors. Reads an explicitly selected cache; never encodes or calls a provider.
use crate::{
    context::{Context, Document},
    model::{hash, Result},
    policy::Access,
    Vault,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

const MAX_INDEX_BYTES: u64 = 32 * 1024 * 1024;
const MAX_VECTOR_BYTES: usize = 512 * 1024 * 1024;
const MAX_CHUNKS: usize = 100_000;
const MAX_CANDIDATES: usize = 256;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Space {
    pub schema: String,
    pub endpoint: String,
    pub model: String,
    pub dimensions: usize,
}
impl Space {
    fn validate(&self) -> Result<()> {
        if self.schema != "private-vector-cache-v1"
            || !(1..=16384).contains(&self.dimensions)
            || self.endpoint.len() > 2048
            || !self.endpoint.starts_with("https://")
            || !self.endpoint.is_ascii()
            || self.model.is_empty()
            || self.model.len() > 256
            || !self.model.is_ascii()
        {
            return Err("event_vector_space_invalid".into());
        }
        Ok(())
    }
    fn binding(&self) -> String {
        // The existing RCVEC001 writer uses Python json.dumps(sort_keys=True), including spaces.
        hash(format!("{{\"dimensions\": {}, \"endpoint\": {}, \"model\": {}, \"schema\": \"private-vector-cache-v1\"}}",
            self.dimensions, json!(self.endpoint), json!(self.model)).as_bytes())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Chunk {
    #[serde(rename = "ref")]
    pub reference: String,
    pub start_byte: usize,
    pub end_byte: usize,
    pub text_sha256: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema: String,
    pub space: Space,
    pub chunks: Vec<Chunk>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Index {
    schema: String,
    cache_dir: PathBuf,
    space: Space,
    chunks: Vec<Chunk>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pack: Option<Pack>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PackedVector {
    offset: u64,
    sha256: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Pack {
    path: PathBuf,
    bytes: u64,
    sha256: String,
    vectors: BTreeMap<String, PackedVector>,
}
impl Pack {
    fn validate(&self, space: &Space, chunks: &[Chunk]) -> Result<()> {
        let size = (72 + 8 * space.dimensions) as u64;
        let hashes: BTreeSet<_> = chunks.iter().map(|c| c.text_sha256.as_str()).collect();
        let offsets: BTreeSet<_> = self.vectors.values().map(|v| v.offset).collect();
        if !digest(&self.sha256)
            || self.bytes > MAX_VECTOR_BYTES as u64
            || self.bytes != hashes.len() as u64 * size
            || self.vectors.len() != hashes.len()
            || !hashes.iter().all(|h| self.vectors.contains_key(*h))
            || offsets.len() != hashes.len()
            || self
                .vectors
                .values()
                .any(|v| !digest(&v.sha256) || v.offset % size != 0 || v.offset >= self.bytes)
        {
            return Err("event_vector_pack_manifest_invalid".into());
        }
        self.open()?;
        Ok(())
    }
    fn open(&self) -> Result<std::fs::File> {
        safe_path(&self.path)?;
        let file = crate::filesystem::open_local_file(&self.path)?;
        let meta = file
            .metadata()
            .map_err(|_| "event_vector_pack_unavailable")?;
        if !meta.is_file() || meta.len() != self.bytes {
            return Err("event_vector_pack_size_invalid".into());
        }
        Ok(file)
    }
    fn vector(&self, file: &mut std::fs::File, space: &Space, text_hash: &str) -> Result<Vec<f64>> {
        let entry = self
            .vectors
            .get(text_hash)
            .ok_or("event_vector_pack_entry_missing")?;
        let mut raw = vec![0; 72 + 8 * space.dimensions];
        file.seek(SeekFrom::Start(entry.offset))
            .map_err(|_| "event_vector_pack_read_failed")?;
        file.read_exact(&mut raw)
            .map_err(|_| "event_vector_pack_read_failed")?;
        if hash(&raw) != entry.sha256 {
            return Err("event_vector_pack_hash_mismatch".into());
        }
        decode_vector(&raw, space, text_hash)
    }
}
pub struct EventVectorIndex {
    path: PathBuf,
    fingerprint: String,
    index: Index,
    query_cache: Option<PathBuf>,
    query_profile: Option<QueryCacheProfile>,
    ranking: String,
}

const LEGACY_QUERY_PROFILE: &str = "legacy_raw_provider_preprocessing_unverified";

/// Exact caller-side input construction. This does not attest to provider-side preprocessing.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", deny_unknown_fields)]
enum QueryEncodingProfile {
    #[serde(rename = "raw/1")]
    Raw {},
    #[serde(rename = "instruct_query/1")]
    Instruct { instruction: String },
}
impl QueryEncodingProfile {
    fn encoded_input(&self, query: &str) -> Result<String> {
        match self {
            Self::Raw {} => Ok(query.to_owned()),
            Self::Instruct { instruction } => {
                if instruction.trim().is_empty()
                    || instruction.len() > 4096
                    || instruction.chars().any(char::is_control)
                {
                    return Err("event_query_encoding_profile_invalid".into());
                }
                Ok(format!("Instruct: {instruction}\nQuery: {query}"))
            }
        }
    }
    fn name(&self) -> &'static str {
        match self {
            Self::Raw {} => "raw/1",
            Self::Instruct { .. } => "instruct_query/1",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct QueryCacheProfile {
    schema: String,
    space: Space,
    encoding_profile: QueryEncodingProfile,
}
impl QueryCacheProfile {
    fn new(space: &Space, encoding_profile: &QueryEncodingProfile) -> Self {
        Self {
            schema: "recallcard.query-cache-profile/1".into(),
            space: space.clone(),
            encoding_profile: encoding_profile.clone(),
        }
    }
    fn binding(&self) -> String {
        // Fixed Rust struct field ordering; no untrusted JSON serialization order is used.
        hash(&serde_json::to_vec(self).expect("query profile serialization"))
    }
}

/// Version 1 remains byte-compatible with the original raw cache writer. Version 2
/// stores a self-contained JSON entry so its input/profile binding is committed atomically.
#[derive(Serialize, Deserialize)]
#[serde(tag = "schema", deny_unknown_fields)]
enum QueryVector {
    #[serde(rename = "recallcard.query-vector/1")]
    Legacy {
        space: Space,
        query_sha256: String,
        vector: Vec<f64>,
    },
    #[serde(rename = "recallcard.query-vector/2")]
    Profiled {
        space: Space,
        query_sha256: String,
        encoded_input_sha256: String,
        encoding_profile: QueryEncodingProfile,
        vector: Vec<f64>,
    },
}
impl QueryVector {
    fn parts(&self) -> (&Space, &str, &[f64]) {
        match self {
            Self::Legacy {
                space,
                query_sha256,
                vector,
            }
            | Self::Profiled {
                space,
                query_sha256,
                vector,
                ..
            } => (space, query_sha256, vector),
        }
    }
    fn profile(&self) -> Option<QueryCacheProfile> {
        match self {
            Self::Legacy { .. } => None,
            Self::Profiled {
                space,
                encoding_profile,
                ..
            } => Some(QueryCacheProfile::new(space, encoding_profile)),
        }
    }
    fn validate(&self, query: &str) -> Result<()> {
        let (space, query_sha256, vector) = self.parts();
        space.validate()?;
        let norm = vector.iter().map(|v| v * v).sum::<f64>().sqrt();
        if query_sha256 != hash(query.as_bytes())
            || vector.len() != space.dimensions
            || !vector.iter().all(|v| v.is_finite())
            || !norm.is_finite()
            || (norm - 1.0).abs() > 1e-6
        {
            return Err("event_query_vector_binding_or_norm_invalid".into());
        }
        if let Self::Profiled {
            encoded_input_sha256,
            encoding_profile,
            ..
        } = self
        {
            let encoded = encoding_profile.encoded_input(query)?;
            if *encoded_input_sha256 != hash(encoded.as_bytes()) {
                return Err("event_query_encoded_input_hash_mismatch".into());
            }
        }
        Ok(())
    }
}

fn cache_query_profile(cache_dir: &Path) -> Result<Option<QueryCacheProfile>> {
    safe_path(cache_dir)?;
    let path = cache_dir.join("profile.json");
    // symlink_metadata notices broken symlinks too, so they cannot look like legacy caches.
    match std::fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let mut entries =
                std::fs::read_dir(cache_dir).map_err(|_| "event_query_cache_unavailable")?;
            if entries.any(|entry| {
                entry.map_or(true, |entry| {
                    entry
                        .file_name()
                        .to_string_lossy()
                        .ends_with(".query-v2.json")
                })
            }) {
                return Err("event_query_cache_profile_missing".into());
            }
            return Ok(None);
        }
        Err(_) => return Err("event_query_cache_unavailable".into()),
        Ok(_) => {}
    }
    let profile: QueryCacheProfile = serde_json::from_slice(&read(&path, 16 * 1024)?)
        .map_err(|_| "event_query_cache_profile_invalid")?;
    if profile.schema != "recallcard.query-cache-profile/1" {
        return Err("event_query_cache_profile_invalid".into());
    }
    profile.space.validate()?;
    profile.encoding_profile.encoded_input("")?;
    Ok(Some(profile))
}

fn write_query_file(path: &Path, bytes: &[u8], failure: &'static str) -> Result<()> {
    let parent = path.parent().ok_or("event_query_write_failed")?;
    let mut temp = tempfile::NamedTempFile::new_in(parent).map_err(|_| failure)?;
    temp.write_all(bytes).map_err(|_| failure)?;
    temp.as_file().sync_all().map_err(|_| failure)?;
    temp.persist_noclobber(path).map_err(|_| failure)?;
    crate::vault::sync_parent(path)?;
    Ok(())
}

struct QueryImportGuard(std::fs::File);
impl Drop for QueryImportGuard {
    fn drop(&mut self) {
        // Explicitly release even if a concurrent fork temporarily retained the fd.
        let _ = self.0.unlock();
    }
}
fn query_import_lock(cache_dir: &Path) -> Result<QueryImportGuard> {
    let path = cache_dir.join(".query-import.lock");
    if let Ok(meta) = std::fs::symlink_metadata(&path) {
        if !meta.is_file() || meta.is_symlink() {
            return Err("event_query_import_lock_invalid".into());
        }
    }
    let mut options = std::fs::OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let file = options
        .open(&path)
        .map_err(|_| "event_query_import_lock_unavailable")?;
    let meta = file
        .metadata()
        .map_err(|_| "event_query_import_lock_unavailable")?;
    if !meta.is_file() || meta.is_symlink() {
        return Err("event_query_import_lock_invalid".into());
    }
    file.try_lock().map_err(|_| "event_query_import_busy")?;
    Ok(QueryImportGuard(file))
}

/// Explicit query-vector import into the selected local cache; no provider calls.
pub fn import_query(query_file: &Path, vector_file: &Path, cache_dir: &Path) -> Result<Value> {
    let query = read(query_file, 4096)?;
    let query = std::str::from_utf8(&query).map_err(|_| "event_query_not_utf8")?;
    if query.trim().is_empty() {
        return Err("event_query_empty".into());
    }
    let input: QueryVector = serde_json::from_slice(&read(vector_file, 1024 * 1024)?)
        .map_err(|_| "event_query_vector_invalid")?;
    input.validate(query)?;
    let (space, query_sha256, vector) = input.parts();
    safe_path(cache_dir)?;
    // Serialize cooperating v1/v2 writers across the declaration and entry transaction.
    // The zero-byte lock is retained; closing this handle releases it, even on failure.
    let _cache_lock = query_import_lock(cache_dir)?;
    let profile = input.profile();
    let existing_profile = cache_query_profile(cache_dir)?;
    if existing_profile.is_some() && existing_profile != profile {
        return Err("event_query_cache_profile_mismatch".into());
    }
    if profile.is_some() && existing_profile.is_none() {
        // Never silently relabel legacy/source vectors as verified raw or instructed queries.
        let mut entries =
            std::fs::read_dir(cache_dir).map_err(|_| "event_query_cache_unavailable")?;
        if entries.any(|entry| {
            entry.map_or(true, |entry| {
                entry.file_name() != "metadata.json" && entry.file_name() != ".query-import.lock"
            })
        }) {
            return Err("event_query_profile_requires_empty_cache".into());
        }
    }
    let metadata = cache_dir.join("metadata.json");
    if metadata
        .try_exists()
        .map_err(|_| "event_query_cache_unavailable")?
        && cache_space(cache_dir)? != *space
    {
        return Err("event_vector_space_mismatch".into());
    }
    if let Some(profile) = profile.as_ref().filter(|_| existing_profile.is_none()) {
        write_query_file(
            &cache_dir.join("profile.json"),
            &serde_json::to_vec(profile).map_err(|_| "event_query_write_failed")?,
            "event_query_profile_exists_or_failed",
        )?;
    }
    if !metadata
        .try_exists()
        .map_err(|_| "event_query_cache_unavailable")?
    {
        write_query_file(
            &metadata,
            json!(space).to_string().as_bytes(),
            "event_query_metadata_exists_or_failed",
        )?;
    }
    if cache_space(cache_dir)? != *space {
        return Err("event_vector_space_mismatch".into());
    }
    if cache_query_profile(cache_dir)? != profile {
        return Err("event_query_cache_profile_mismatch".into());
    }
    let output;
    let raw;
    if profile.is_some() {
        output = cache_dir.join(format!("{query_sha256}.query-v2.json"));
        raw = serde_json::to_vec(&input).map_err(|_| "event_query_write_failed")?;
    } else {
        let decode = |hex: &str| -> Vec<u8> {
            (0..hex.len())
                .step_by(2)
                .map(|at| u8::from_str_radix(&hex[at..at + 2], 16).unwrap())
                .collect()
        };
        let mut bytes = b"RCVEC001".to_vec();
        bytes.extend(decode(&space.binding()));
        bytes.extend(decode(query_sha256));
        for value in vector {
            bytes.extend(value.to_le_bytes());
        }
        output = cache_dir.join(format!("{query_sha256}.bin"));
        raw = bytes;
    }
    write_query_file(&output, &raw, "event_query_vector_exists_or_failed")?;
    Ok(
        json!({"status":"query_vector_imported","model":space.model,"dimensions":space.dimensions,"network_requests":0,"cache_write":"explicit_query_destination_only",
            "query_encoding_profile":profile.as_ref().map_or(LEGACY_QUERY_PROFILE, |p| p.encoding_profile.name()),
            "query_encoding_profile_sha256":profile.as_ref().map(QueryCacheProfile::binding),
            "provider_preprocessing":"unverified"}),
    )
}
pub(crate) struct Hits {
    pub references: Vec<String>,
    pub ranges: BTreeMap<String, (usize, usize)>,
    pub scores: BTreeMap<String, f64>,
    chunks: Vec<Chunk>,
    pub coverage: Value,
}
impl Hits {
    pub(crate) fn revalidate(&self, documents: &[Document]) -> Result<()> {
        let docs = event_documents(documents);
        for chunk in &self.chunks {
            validate_chunk(chunk, &docs)?;
        }
        Ok(())
    }
}
fn event_documents(documents: &[Document]) -> BTreeMap<&str, &Document> {
    documents
        .iter()
        .filter(|doc| doc.kind == "event")
        .map(|doc| (doc.reference.as_str(), doc))
        .collect()
}
fn digest(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn validate_chunk(chunk: &Chunk, docs: &BTreeMap<&str, &Document>) -> Result<()> {
    let doc = docs
        .get(chunk.reference.as_str())
        .ok_or("event_vector_source_not_visible")?;
    let text = doc
        .text
        .get(chunk.start_byte..chunk.end_byte)
        .ok_or("event_vector_range_invalid")?;
    if text.trim().is_empty() || hash(text.as_bytes()) != chunk.text_sha256 {
        return Err("event_vector_text_hash_mismatch".into());
    }
    Ok(())
}
fn validate_manifest(space: &Space, chunks: &[Chunk]) -> Result<()> {
    space.validate()?;
    if chunks.is_empty() || chunks.len() > MAX_CHUNKS {
        return Err("event_vector_chunk_limit".into());
    }
    let mut seen = BTreeSet::new();
    let mut hashes = BTreeSet::new();
    for chunk in chunks {
        let (kind, _, _) = crate::context::parse_ref(&chunk.reference)?;
        if kind != "event"
            || chunk.start_byte >= chunk.end_byte
            || !digest(&chunk.text_sha256)
            || !seen.insert((&chunk.reference, chunk.start_byte, chunk.end_byte))
        {
            return Err("event_vector_chunk_invalid".into());
        }
        hashes.insert(&chunk.text_sha256);
    }
    if hashes.len().saturating_mul(72 + 8 * space.dimensions) > MAX_VECTOR_BYTES {
        return Err("event_vector_io_budget_exceeded".into());
    }
    Ok(())
}
fn safe_path(path: &Path) -> Result<()> {
    if !path.is_absolute()
        || path
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
    {
        return Err("event_vector_path_must_be_absolute".into());
    }
    for ancestor in path.ancestors() {
        let metadata =
            std::fs::symlink_metadata(ancestor).map_err(|_| "event_vector_file_unavailable")?;
        if metadata.is_symlink() {
            return Err("event_vector_symlink_rejected".into());
        }
    }
    Ok(())
}
fn read(path: &Path, limit: u64) -> Result<Vec<u8>> {
    safe_path(path)?;
    let file = crate::filesystem::open_local_file(path)?;
    let metadata = file
        .metadata()
        .map_err(|_| "event_vector_file_unavailable")?;
    if !metadata.is_file() || metadata.len() > limit {
        return Err("event_vector_file_invalid".into());
    }
    let mut bytes = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "event_vector_read_failed")?;
    if bytes.len() as u64 > limit {
        return Err("event_vector_file_limit".into());
    }
    Ok(bytes)
}
fn cache_space(cache_dir: &Path) -> Result<Space> {
    let space: Space = serde_json::from_slice(&read(&cache_dir.join("metadata.json"), 4096)?)
        .map_err(|_| "event_vector_metadata_invalid")?;
    space.validate()?;
    Ok(space)
}
fn read_vector(cache_dir: &Path, space: &Space, text_hash: &str) -> Result<Vec<f64>> {
    if !digest(text_hash) {
        return Err("event_vector_hash_invalid".into());
    }
    let size = 72 + 8 * space.dimensions;
    let raw = read(&cache_dir.join(format!("{text_hash}.bin")), size as u64)?;
    decode_vector(&raw, space, text_hash)
}
fn decode_vector(raw: &[u8], space: &Space, text_hash: &str) -> Result<Vec<f64>> {
    let size = 72 + 8 * space.dimensions;
    let hex = |bytes: &[u8]| bytes.iter().map(|v| format!("{v:02x}")).collect::<String>();
    if raw.len() != size
        || &raw[..8] != b"RCVEC001"
        || hex(&raw[8..40]) != space.binding()
        || hex(&raw[40..72]) != text_hash
    {
        return Err("event_vector_binding_mismatch".into());
    }
    let vector: Vec<_> = raw[72..]
        .chunks_exact(8)
        .map(|bytes| f64::from_le_bytes(bytes.try_into().unwrap()))
        .collect();
    let norm = vector.iter().map(|v| v * v).sum::<f64>().sqrt();
    if !vector.iter().all(|v| v.is_finite()) || !norm.is_finite() || (norm - 1.0).abs() > 1e-6 {
        return Err("event_vector_invalid_norm".into());
    }
    Ok(vector)
}

/// Explicit local import. Validates every referenced chunk/vector before writing a private index.
/// Cache files are neither rewritten nor uploaded. Refuses to overwrite an existing output.
pub fn import(
    vault: &Vault,
    access: Access,
    manifest: &Path,
    cache_dir: &Path,
    output: &Path,
) -> Result<Value> {
    let manifest: Manifest = serde_json::from_slice(&read(manifest, MAX_INDEX_BYTES)?)
        .map_err(|_| "event_vector_manifest_invalid")?;
    if manifest.schema != "recallcard.event-vector-manifest/1" {
        return Err("event_vector_manifest_version".into());
    }
    validate_manifest(&manifest.space, &manifest.chunks)?;
    if cache_space(cache_dir)? != manifest.space {
        return Err("event_vector_space_mismatch".into());
    }
    let parent = output.parent().ok_or("event_vector_output_invalid")?;
    safe_path(parent)?;
    if !output.is_absolute() {
        return Err("event_vector_output_must_be_absolute".into());
    }
    if output
        .try_exists()
        .map_err(|_| "event_vector_output_failed")?
    {
        return Err("event_vector_output_exists_or_failed".into());
    }
    // The index is the commit point. The pack is private and fully fsynced first;
    // failure before the index commit automatically removes the temporary pack.
    let mut packed = tempfile::Builder::new()
        .prefix("recallcard-event-vectors-")
        .suffix(".pack")
        .tempfile_in(parent)
        .map_err(|_| "event_vector_pack_write_failed")?;
    let mut pack = Pack {
        path: packed.path().to_owned(),
        bytes: 0,
        sha256: String::new(),
        vectors: BTreeMap::new(),
    };
    let mut pack_hash = Sha256::new();
    let guard = vault.read_guard()?;
    let documents = Context::new(vault, access).documents_locked()?;
    let docs = event_documents(&documents);
    let mut validated = BTreeSet::new();
    for chunk in &manifest.chunks {
        validate_chunk(chunk, &docs)?;
        if validated.insert(chunk.text_sha256.clone()) {
            let raw = read(
                &cache_dir.join(format!("{}.bin", chunk.text_sha256)),
                (72 + 8 * manifest.space.dimensions) as u64,
            )?;
            decode_vector(&raw, &manifest.space, &chunk.text_sha256)?;
            packed
                .write_all(&raw)
                .map_err(|_| "event_vector_pack_write_failed")?;
            pack.vectors.insert(
                chunk.text_sha256.clone(),
                PackedVector {
                    offset: pack.bytes,
                    sha256: hash(&raw),
                },
            );
            pack.bytes += raw.len() as u64;
            pack_hash.update(&raw);
        }
    }
    if cache_space(cache_dir)? != manifest.space {
        return Err("event_vector_space_changed".into());
    }
    packed
        .as_file()
        .sync_all()
        .map_err(|_| "event_vector_pack_write_failed")?;
    pack.sha256 = format!("{:x}", pack_hash.finalize());
    let index = Index {
        schema: "recallcard.event-vector-index/1".into(),
        cache_dir: cache_dir.to_owned(),
        space: manifest.space,
        chunks: manifest.chunks,
        pack: Some(pack),
    };
    let bytes = serde_json::to_vec(&index).map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_INDEX_BYTES {
        return Err("event_vector_index_limit".into());
    }
    let mut temp =
        tempfile::NamedTempFile::new_in(parent).map_err(|_| "event_vector_output_failed")?;
    temp.write_all(&bytes)
        .map_err(|_| "event_vector_output_failed")?;
    temp.as_file()
        .sync_all()
        .map_err(|_| "event_vector_output_failed")?;
    temp.persist_noclobber(output)
        .map_err(|_| "event_vector_output_exists_or_failed")?;
    // The committed index now owns this completed pack. Keep it before any
    // fallible post-commit operation so a reported fsync error cannot orphan it.
    packed.keep().map_err(|_| "event_vector_pack_keep_failed")?;
    crate::vault::sync_parent(output)?;
    drop(guard);
    Ok(
        json!({"status":"imported","chunks":index.chunks.len(),"unique_vectors":validated.len(),"model":index.space.model,"dimensions":index.space.dimensions,"network_requests":0,"query_mode":"existing_cache_only","storage":"validated_contiguous_pack","pack_bytes":index.pack.as_ref().unwrap().bytes}),
    )
}

impl EventVectorIndex {
    pub fn open(path: &Path) -> Result<Self> {
        let bytes = read(path, MAX_INDEX_BYTES)?;
        let index: Index =
            serde_json::from_slice(&bytes).map_err(|_| "event_vector_index_invalid")?;
        if index.schema != "recallcard.event-vector-index/1" {
            return Err("event_vector_index_version".into());
        }
        validate_manifest(&index.space, &index.chunks)?;
        if let Some(pack) = &index.pack {
            pack.validate(&index.space, &index.chunks)?;
        }
        if cache_space(&index.cache_dir)? != index.space {
            return Err("event_vector_space_mismatch".into());
        }
        Ok(Self {
            path: path.to_owned(),
            fingerprint: hash(&bytes),
            index,
            query_cache: None,
            query_profile: None,
            ranking: "hybrid".into(),
        })
    }
    /// Explicit operator policy. Non-hybrid policies are limited to target=events.
    pub fn with_ranking(mut self, ranking: &str) -> Result<Self> {
        if !matches!(ranking, "hybrid" | "semantic_only" | "semantic_first") {
            return Err("event_vector_ranking_invalid".into());
        }
        self.ranking = ranking.into();
        Ok(self)
    }
    pub(crate) fn ranking(&self) -> &str {
        &self.ranking
    }
    pub fn with_query_cache(mut self, cache: Option<&Path>) -> Result<Self> {
        if let Some(cache) = cache {
            if cache_space(cache)? != self.index.space {
                return Err("event_vector_space_mismatch".into());
            }
            let profile = cache_query_profile(cache)?;
            if profile
                .as_ref()
                .is_some_and(|p| p.space != self.index.space)
            {
                return Err("event_vector_space_mismatch".into());
            }
            self.query_cache = Some(cache.to_owned());
            self.query_profile = profile;
        } else {
            self.query_cache = None;
            self.query_profile = None;
        }
        Ok(self)
    }
    /// Binds cursors even when two profiles happen to produce the same ranking.
    pub(crate) fn query_encoding_binding(&self) -> String {
        hash(
            json!({
                "space":self.index.space,
                "query_cache":self.query_cache,
                "source_cache":self.index.cache_dir,
                "profile":self.query_profile,
                "legacy":LEGACY_QUERY_PROFILE,
            })
            .to_string()
            .as_bytes(),
        )
    }
    fn unchanged(&self) -> Result<()> {
        if hash(&read(&self.path, MAX_INDEX_BYTES)?) != self.fingerprint
            || cache_space(&self.index.cache_dir)? != self.index.space
            || self
                .query_cache
                .as_ref()
                .map(|cache| cache_space(cache))
                .transpose()?
                .is_some_and(|space| space != self.index.space)
        {
            return Err("event_vector_index_changed_restart_required".into());
        }
        if let Some(cache) = &self.query_cache {
            if cache_query_profile(cache)? != self.query_profile {
                return Err("event_query_cache_profile_changed_restart_required".into());
            }
        }
        Ok(())
    }
    pub(crate) fn candidates(&self, query: &str, documents: &[Document]) -> Result<Hits> {
        self.candidates_internal(query, documents, false)
    }
    /// Discovery groups by full authorized source identity before its own session cap.
    pub(crate) fn discovery_candidates(&self, query: &str, documents: &[Document]) -> Result<Hits> {
        self.candidates_internal(query, documents, true)
    }
    fn candidates_internal(
        &self,
        query: &str,
        documents: &[Document],
        discovery: bool,
    ) -> Result<Hits> {
        let began = std::time::Instant::now();
        self.unchanged()?;
        let index_ms = began.elapsed().as_millis();
        let query_hash = hash(query.as_bytes());
        let query_directory = if let Some(cache) = &self.query_cache {
            if self.query_profile.is_some()
                || cache
                    .join(format!("{query_hash}.bin"))
                    .try_exists()
                    .map_err(|_| "event_query_cache_unavailable")?
            {
                cache
            } else {
                &self.index.cache_dir
            }
        } else {
            &self.index.cache_dir
        };
        let query_filename = if self.query_profile.is_some() {
            format!("{query_hash}.query-v2.json")
        } else {
            format!("{query_hash}.bin")
        };
        let query_path = query_directory.join(query_filename);
        if !query_path
            .try_exists()
            .map_err(|_| "event_query_cache_unavailable")?
        {
            return Err("event_query_not_cached_no_network_attempted".into());
        }
        let (query_vector, encoded_input_hash) = if let Some(profile) = &self.query_profile {
            let input: QueryVector = serde_json::from_slice(&read(&query_path, 1024 * 1024)?)
                .map_err(|_| "event_query_vector_invalid")?;
            input.validate(query)?;
            if input.profile().as_ref() != Some(profile) {
                return Err("event_query_cache_profile_mismatch".into());
            }
            match input {
                QueryVector::Profiled {
                    vector,
                    encoded_input_sha256,
                    ..
                } => (vector, Some(encoded_input_sha256)),
                QueryVector::Legacy { .. } => unreachable!(),
            }
        } else {
            (
                read_vector(query_directory, &self.index.space, &query_hash)?,
                None,
            )
        };
        let query_ms = began.elapsed().as_millis() - index_ms;
        let scan = std::time::Instant::now();
        let docs = event_documents(documents);
        let mut pack_reader = self.index.pack.as_ref().map(Pack::open).transpose()?;
        let mut source_load = std::time::Duration::ZERO;
        let mut source_dot = std::time::Duration::ZERO;
        let mut best: BTreeMap<String, (f64, Chunk)> = BTreeMap::new();
        // Retain scalar scores, not a full copy of the source embedding corpus.
        let mut scores: BTreeMap<&str, f64> = BTreeMap::new();
        let mut eligible_chunks = 0;
        for chunk in &self.index.chunks {
            if !docs.contains_key(chunk.reference.as_str()) {
                continue;
            }
            validate_chunk(chunk, &docs)?;
            eligible_chunks += 1;
            let score = if let Some(score) = scores.get(chunk.text_sha256.as_str()) {
                *score
            } else {
                let at = std::time::Instant::now();
                let vector = match (&self.index.pack, &mut pack_reader) {
                    (Some(pack), Some(file)) => {
                        pack.vector(file, &self.index.space, &chunk.text_sha256)?
                    }
                    _ => read_vector(&self.index.cache_dir, &self.index.space, &chunk.text_sha256)?,
                };
                source_load += at.elapsed();
                let at = std::time::Instant::now();
                let score = query_vector
                    .iter()
                    .zip(&vector)
                    .map(|(a, b)| a * b)
                    .sum::<f64>();
                source_dot += at.elapsed();
                scores.insert(&chunk.text_sha256, score);
                score
            };
            let value = best
                .entry(chunk.reference.clone())
                .or_insert((score, chunk.clone()));
            if score > value.0 || score == value.0 && chunk.start_byte < value.1.start_byte {
                *value = (score, chunk.clone());
            }
        }
        let scan_ms = scan.elapsed().as_millis();
        self.unchanged()?;
        let total_events = best.len();
        let mut ranked: Vec<_> = best.into_iter().collect();
        ranked.sort_by(|a, b| b.1 .0.total_cmp(&a.1 .0).then(a.0.cmp(&b.0)));
        let limit = if discovery {
            MAX_CHUNKS
        } else {
            MAX_CANDIDATES
        };
        ranked.truncate(limit);
        let references = ranked
            .iter()
            .map(|(reference, _)| reference.clone())
            .collect();
        let ranges = ranked
            .iter()
            .map(|(reference, (_, chunk))| (reference.clone(), (chunk.start_byte, chunk.end_byte)))
            .collect();
        let scores = ranked
            .iter()
            .map(|(reference, (score, _))| (reference.clone(), *score))
            .collect();
        let chunks = ranked.into_iter().map(|(_, (_, chunk))| chunk).collect();
        Ok(Hits {
            references,
            ranges,
            scores,
            chunks,
            coverage: json!({"status":"available","model":self.index.space.model,"dimensions":self.index.space.dimensions,
            "query_encoding":"existing_cache_only","network_requests":0,"eligible_chunks":eligible_chunks,"eligible_events":total_events,
            "query_encoding_profile":self.query_profile.as_ref().map_or(LEGACY_QUERY_PROFILE, |p| p.encoding_profile.name()),
            "query_encoding_profile_sha256":self.query_profile.as_ref().map(QueryCacheProfile::binding),
            "query_input_binding":if self.query_profile.is_some() { "declared_input_hashes_validated_provider_preprocessing_unverified" } else { LEGACY_QUERY_PROFILE },
            "query_sha256":query_hash,"encoded_input_sha256":encoded_input_hash,
            "query_cache_selection":if query_directory == &self.index.cache_dir { "source_cache_legacy" } else { "explicit_query_cache" },
            "storage":if self.index.pack.is_some() { "validated_contiguous_pack" } else { "source_cache_files" },"candidate_limit":limit,"candidates_truncated":total_events>limit,"index_fingerprint":self.fingerprint,
            "timing_ms":{"index_check":index_ms,"query_vector":query_ms,"source_scan":scan_ms,"source_vector_load_validate":source_load.as_millis(),"dot_products":source_dot.as_millis(),"total":began.elapsed().as_millis()}}),
        })
    }
}

#[cfg(all(test, unix))]
mod import_lock_tests {
    use super::*;

    #[test]
    fn releasing_import_guard_unlocks_even_while_a_cloned_handle_remains() {
        let root = tempfile::tempdir().unwrap();
        let guard = query_import_lock(root.path()).unwrap();
        let retained = guard.0.try_clone().unwrap();
        assert!(query_import_lock(root.path()).is_err());
        drop(guard);
        let next = query_import_lock(root.path()).unwrap();
        drop(retained);
        drop(next);
        assert!(query_import_lock(root.path()).is_ok());
    }
}
