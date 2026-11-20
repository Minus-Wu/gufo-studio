use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;

use crate::gufo_manager::GufoManager;

const DEFAULT_CHUNK_SIZE: u32 = 800;
const DEFAULT_CHUNK_OVERLAP: u32 = 120;
const DEFAULT_TOP_K: u32 = 5;
const DEFAULT_MIN_SCORE: f32 = 0.05;
const DEFAULT_HYBRID_ALPHA: f32 = 0.6;
const EMBEDDING_BATCH_SIZE: usize = 16;
const MAX_EXPLICIT_FILE_BYTES: u64 = 10 * 1024 * 1024;
const MAX_FOLDER_FILE_BYTES: u64 = 5 * 1024 * 1024;
const MAX_FOLDER_FILES: usize = 500;

static ID_SEQ: AtomicU64 = AtomicU64::new(1);

pub const DEFAULT_RAG_PROMPT_TEMPLATE: &str = "請優先依據以下「檢索參考資料」回答使用者的問題。回答時若引用了資料內容，請以 [來源編號]（例如 [1]、[2]）標註出處；若參考資料不足以回答或與問題無關，請誠實說明，不要編造事實。\n\n【檢索參考資料】\n{{context}}";

fn now_unix_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn generate_id(prefix: &str) -> String {
    let seq = ID_SEQ.fetch_add(1, Ordering::Relaxed);
    format!("{}_{}_{}", prefix, now_unix_millis(), seq)
}

fn is_valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RagConfig {
    pub chunk_size: u32,
    pub chunk_overlap: u32,
    pub top_k: u32,
    pub min_score: f32,
    pub embedding_enabled: bool,
    pub embedding_endpoint: String,
    pub embedding_model: String,
    pub embedding_api_key: String,
    pub hybrid_alpha: f32,
    pub system_prompt_template: String,
}

impl Default for RagConfig {
    fn default() -> Self {
        Self {
            chunk_size: DEFAULT_CHUNK_SIZE,
            chunk_overlap: DEFAULT_CHUNK_OVERLAP,
            top_k: DEFAULT_TOP_K,
            min_score: DEFAULT_MIN_SCORE,
            embedding_enabled: false,
            embedding_endpoint: "http://127.0.0.1:12224/v1".to_string(),
            embedding_model: String::new(),
            embedding_api_key: String::new(),
            hybrid_alpha: DEFAULT_HYBRID_ALPHA,
            system_prompt_template: DEFAULT_RAG_PROMPT_TEMPLATE.to_string(),
        }
    }
}

impl RagConfig {
    pub fn sanitized(mut self) -> Self {
        self.chunk_size = self.chunk_size.clamp(150, 8000);
        let max_overlap = self.chunk_size / 2;
        self.chunk_overlap = self.chunk_overlap.min(max_overlap);
        self.top_k = self.top_k.clamp(1, 30);
        if !self.min_score.is_finite() {
            self.min_score = DEFAULT_MIN_SCORE;
        }
        self.min_score = self.min_score.clamp(0.0, 1.0);
        if !self.hybrid_alpha.is_finite() {
            self.hybrid_alpha = DEFAULT_HYBRID_ALPHA;
        }
        self.hybrid_alpha = self.hybrid_alpha.clamp(0.0, 1.0);
        self.embedding_endpoint = self.embedding_endpoint.trim().to_string();
        self.embedding_model = self.embedding_model.trim().to_string();
        self.embedding_api_key = self.embedding_api_key.trim().to_string();
        if self.system_prompt_template.trim().is_empty() {
            self.system_prompt_template = DEFAULT_RAG_PROMPT_TEMPLATE.to_string();
        }
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RagDocument {
    pub id: String,
    pub name: String,
    pub source_path: String,
    pub file_type: String,
    pub char_count: usize,
    pub chunk_count: usize,
    pub embedded_chunk_count: usize,
    pub added_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RagChunk {
    pub id: String,
    pub doc_id: String,
    pub doc_name: String,
    pub chunk_index: usize,
    pub content: String,
    pub char_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embedding: Option<Vec<f32>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeBase {
    pub id: String,
    pub name: String,
    pub description: String,
    pub created_at: u64,
    pub updated_at: u64,
    #[serde(default)]
    pub embedding_model_tag: Option<String>,
    #[serde(default)]
    pub documents: Vec<RagDocument>,
    #[serde(default)]
    pub chunks: Vec<RagChunk>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeBaseSummary {
    pub id: String,
    pub name: String,
    pub description: String,
    pub created_at: u64,
    pub updated_at: u64,
    pub doc_count: usize,
    pub chunk_count: usize,
    pub embedded_chunk_count: usize,
    pub embedding_model_tag: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeBaseDetail {
    pub summary: KnowledgeBaseSummary,
    pub documents: Vec<RagDocument>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RagChunkPreview {
    pub id: String,
    pub chunk_index: usize,
    pub content: String,
    pub char_count: usize,
    pub has_embedding: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RagImportItemError {
    pub file: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RagImportResult {
    pub imported_docs: usize,
    pub replaced_docs: usize,
    pub added_chunks: usize,
    pub embedded_chunks: usize,
    pub skipped_files: usize,
    pub errors: Vec<RagImportItemError>,
    pub embedding_warning: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RagReindexResult {
    pub total_chunks: usize,
    pub embedded_chunks: usize,
    pub embedding_model_tag: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingTestResult {
    pub ok: bool,
    pub endpoint_url: String,
    pub model: String,
    pub dimension: usize,
    pub latency_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RagSearchHit {
    pub chunk_id: String,
    pub kb_id: String,
    pub kb_name: String,
    pub doc_id: String,
    pub doc_name: String,
    pub chunk_index: usize,
    pub content: String,
    pub score: f32,
    pub bm25_score: f32,
    pub vector_score: Option<f32>,
    pub match_mode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RagSearchResponse {
    pub hits: Vec<RagSearchHit>,
    pub used_embedding: bool,
    pub warning: Option<String>,
    pub elapsed_ms: u64,
}

impl KnowledgeBase {
    fn summary(&self) -> KnowledgeBaseSummary {
        let embedded_chunk_count = self.chunks.iter().filter(|c| c.embedding.is_some()).count();
        KnowledgeBaseSummary {
            id: self.id.clone(),
            name: self.name.clone(),
            description: self.description.clone(),
            created_at: self.created_at,
            updated_at: self.updated_at,
            doc_count: self.documents.len(),
            chunk_count: self.chunks.len(),
            embedded_chunk_count,
            embedding_model_tag: self.embedding_model_tag.clone(),
        }
    }

    fn refresh_doc_counts(&mut self) {
        let mut counts: HashMap<String, (usize, usize)> = HashMap::new();
        for chunk in &self.chunks {
            let entry = counts.entry(chunk.doc_id.clone()).or_insert((0, 0));
            entry.0 += 1;
            if chunk.embedding.is_some() {
                entry.1 += 1;
            }
        }
        for doc in &mut self.documents {
            let (total, emb) = counts.get(&doc.id).copied().unwrap_or((0, 0));
            doc.chunk_count = total;
            doc.embedded_chunk_count = emb;
        }
    }
}

pub fn is_supported_text_extension(path: &Path) -> bool {
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        let lower = ext.to_ascii_lowercase();
        return matches!(
            lower.as_str(),
            "txt"
                | "md"
                | "markdown"
                | "mdx"
                | "json"
                | "jsonl"
                | "csv"
                | "tsv"
                | "yaml"
                | "yml"
                | "toml"
                | "xml"
                | "html"
                | "htm"
                | "css"
                | "scss"
                | "less"
                | "js"
                | "mjs"
                | "cjs"
                | "jsx"
                | "ts"
                | "tsx"
                | "rs"
                | "py"
                | "pyi"
                | "go"
                | "java"
                | "kt"
                | "kts"
                | "c"
                | "h"
                | "cpp"
                | "hpp"
                | "cc"
                | "cs"
                | "rb"
                | "php"
                | "swift"
                | "sh"
                | "bash"
                | "zsh"
                | "ps1"
                | "bat"
                | "cmd"
                | "sql"
                | "graphql"
                | "proto"
                | "ini"
                | "conf"
                | "cfg"
                | "env"
                | "log"
                | "vue"
                | "svelte"
                | "astro"
                | "r"
                | "lua"
                | "dart"
                | "ex"
                | "exs"
        );
    }

    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
        return matches!(
            name.to_ascii_lowercase().as_str(),
            "readme" | "license" | "makefile" | "dockerfile" | "changelog"
        );
    }
    false
}

fn should_skip_dir(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    matches!(
        lower.as_str(),
        ".git"
            | "node_modules"
            | "target"
            | "dist"
            | "build"
            | ".venv"
            | "venv"
            | "__pycache__"
            | ".next"
            | ".nuxt"
            | ".idea"
            | ".vscode"
    )
}

fn read_text_file(path: &Path, max_bytes: u64) -> Result<String, String> {
    let meta = fs::metadata(path).map_err(|e| format!("無法讀取檔案資訊：{}", e))?;
    if !meta.is_file() {
        return Err("目標不是一般檔案".to_string());
    }
    if meta.len() > max_bytes {
        return Err(format!(
            "檔案大小 ({:.1} MB) 超過上限 ({:.1} MB)",
            meta.len() as f64 / (1024.0 * 1024.0),
            max_bytes as f64 / (1024.0 * 1024.0)
        ));
    }

    let bytes = fs::read(path).map_err(|e| format!("讀取檔案失敗：{}", e))?;
    if bytes.contains(&0) {
        return Err("偵測到二進位內容 (包含 NUL 位元組)，不支援純文字解析".to_string());
    }

    let raw = String::from_utf8(bytes)
        .map_err(|_| "檔案非有效 UTF-8 文字編碼，無法匯入".to_string())?;
    let normalized = raw.strip_prefix('\u{FEFF}').unwrap_or(&raw).replace("\r\n", "\n");
    Ok(normalized)
}

fn is_sentence_boundary(ch: char, next: Option<char>) -> bool {
    if matches!(ch, '。' | '！' | '？' | '；') {
        return true;
    }
    if matches!(ch, '.' | '!' | '?') {
        return next.map(|n| n.is_whitespace()).unwrap_or(true);
    }
    false
}

/// 依 Unicode 字元切割文本並保留重疊上下文，優先在段落、換行或句末斷開。
pub fn chunk_text(text: &str, chunk_size: u32, chunk_overlap: u32) -> Vec<String> {
    let clean = text.trim();
    if clean.is_empty() {
        return Vec::new();
    }

    let size = (chunk_size.max(150)) as usize;
    let overlap = (chunk_overlap as usize).min(size / 2);
    let chars: Vec<char> = clean.chars().collect();
    let total = chars.len();
    if total <= size {
        return vec![clean.to_string()];
    }

    let mut chunks = Vec::new();
    let mut start = 0usize;

    while start < total {
        let hard_end = (start + size).min(total);
        let mut split_at = hard_end;

        if hard_end < total {
            let search_floor = start + (size * 3) / 4;
            let mut found = None;

            // 1. 優先尋找雙換行 (段落邊界)
            for i in (search_floor..hard_end).rev() {
                if i > start && chars[i] == '\n' && chars[i - 1] == '\n' {
                    found = Some(i + 1);
                    break;
                }
            }
            // 2. 其次尋找單換行
            if found.is_none() {
                for i in (search_floor..hard_end).rev() {
                    if chars[i] == '\n' {
                        found = Some(i + 1);
                        break;
                    }
                }
            }
            // 3. 其次尋找句末標點
            if found.is_none() {
                for i in (search_floor..hard_end).rev() {
                    let next = chars.get(i + 1).copied();
                    if is_sentence_boundary(chars[i], next) {
                        found = Some(i + 1);
                        break;
                    }
                }
            }
            // 4. 最後尋找空白字元
            if found.is_none() {
                for i in (search_floor..hard_end).rev() {
                    if chars[i].is_whitespace() {
                        found = Some(i + 1);
                        break;
                    }
                }
            }

            if let Some(pos) = found
                && pos > start
            {
                split_at = pos;
            }
        }

        let piece: String = chars[start..split_at].iter().collect();
        let trimmed = piece.trim();
        if !trimmed.is_empty() {
            chunks.push(trimmed.to_string());
        }

        if split_at >= total {
            break;
        }

        let next_start = split_at.saturating_sub(overlap);
        start = if next_start <= start {
            split_at
        } else {
            next_start
        };
    }

    chunks
}

fn is_cjk_char(c: char) -> bool {
    matches!(
        c as u32,
        0x4E00..=0x9FFF
            | 0x3400..=0x4DBF
            | 0xF900..=0xFAFF
            | 0x3040..=0x309F
            | 0x30A0..=0x30FF
            | 0xAC00..=0xD7AF
    )
}

/// 支援繁簡中文 (unigram + bigram) 與英文/程式碼詞彙的多語分詞器。
pub fn tokenize(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut ascii_buf = String::new();
    let mut prev_cjk: Option<char> = None;

    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() {
            prev_cjk = None;
            ascii_buf.push(ch.to_ascii_lowercase());
        } else {
            if !ascii_buf.is_empty() {
                tokens.push(std::mem::take(&mut ascii_buf));
            }
            if is_cjk_char(ch) {
                tokens.push(ch.to_string());
                if let Some(prev) = prev_cjk {
                    let mut bigram = String::with_capacity(prev.len_utf8() + ch.len_utf8());
                    bigram.push(prev);
                    bigram.push(ch);
                    tokens.push(bigram);
                }
                prev_cjk = Some(ch);
            } else {
                prev_cjk = None;
            }
        }
    }

    if !ascii_buf.is_empty() {
        tokens.push(ascii_buf);
    }

    tokens
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> Option<f32> {
    if a.is_empty() || a.len() != b.len() {
        return None;
    }
    let mut dot = 0.0f64;
    let mut norm_a = 0.0f64;
    let mut norm_b = 0.0f64;
    for (&x, &y) in a.iter().zip(b.iter()) {
        let xf = x as f64;
        let yf = y as f64;
        dot += xf * yf;
        norm_a += xf * xf;
        norm_b += yf * yf;
    }
    let denom = norm_a.sqrt() * norm_b.sqrt();
    if denom <= 1e-12 {
        return Some(0.0);
    }
    Some((dot / denom) as f32)
}

pub fn normalize_embeddings_url(endpoint: &str) -> Result<String, String> {
    let trimmed = endpoint.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Err("Embedding API 端點不可為空".to_string());
    }
    if !trimmed.starts_with("http://") && !trimmed.starts_with("https://") {
        return Err("Embedding API 端點必須以 http:// 或 https:// 開頭".to_string());
    }
    if trimmed.ends_with("/embeddings") {
        Ok(trimmed.to_string())
    } else if trimmed.ends_with("/v1") {
        Ok(format!("{}/embeddings", trimmed))
    } else {
        Ok(format!("{}/v1/embeddings", trimmed))
    }
}

#[derive(Deserialize)]
struct OpenAiEmbeddingsResponse {
    data: Vec<OpenAiEmbeddingItem>,
}

#[derive(Deserialize)]
struct OpenAiEmbeddingItem {
    index: usize,
    embedding: Vec<f32>,
}

async fn fetch_embeddings(
    client: &Client,
    endpoint: &str,
    api_key: &str,
    model: &str,
    inputs: &[String],
) -> Result<Vec<Vec<f32>>, String> {
    if inputs.is_empty() {
        return Ok(Vec::new());
    }
    let url = normalize_embeddings_url(endpoint)?;
    let model = model.trim();
    if model.is_empty() {
        return Err("尚未設定 Embedding 模型名稱".to_string());
    }

    let mut all_vectors = Vec::with_capacity(inputs.len());
    for batch in inputs.chunks(EMBEDDING_BATCH_SIZE) {
        let mut req = client.post(&url).json(&serde_json::json!({
            "model": model,
            "input": batch,
        }));
        if !api_key.trim().is_empty() {
            req = req.bearer_auth(api_key.trim());
        }

        let resp = req
            .send()
            .await
            .map_err(|e| format!("連線 Embedding 服務失敗 ({}): {}", url, e))?;

        let status = resp.status();
        let body = resp
            .text()
            .await
            .map_err(|e| format!("讀取 Embedding 回應失敗：{}", e))?;

        if !status.is_success() {
            let preview: String = body.chars().take(300).collect();
            return Err(format!("Embedding API 回傳 HTTP {}：{}", status, preview));
        }

        let mut parsed: OpenAiEmbeddingsResponse = serde_json::from_str(&body)
            .map_err(|e| format!("解析 Embedding JSON 失敗：{}", e))?;
        parsed.data.sort_by_key(|d| d.index);

        if parsed.data.len() != batch.len() {
            return Err(format!(
                "Embedding 回傳筆數不符（預期 {} 筆，實際 {} 筆）",
                batch.len(),
                parsed.data.len()
            ));
        }

        for item in parsed.data {
            if item.embedding.is_empty() {
                return Err("Embedding API 回傳空向量".to_string());
            }
            all_vectors.push(item.embedding);
        }
    }

    Ok(all_vectors)
}

pub struct RagManager {
    base_dir: PathBuf,
    config: RwLock<RagConfig>,
    kbs: RwLock<BTreeMap<String, KnowledgeBase>>,
    http: Client,
}

impl Default for RagManager {
    fn default() -> Self {
        Self::new()
    }
}

impl RagManager {
    pub fn new() -> Self {
        Self::with_base_dir(GufoManager::app_base_dir().join("gufo-rag"))
    }

    pub fn with_base_dir(base_dir: PathBuf) -> Self {
        let _ = fs::create_dir_all(base_dir.join("kbs"));
        let config = Self::load_config_from_disk(&base_dir).unwrap_or_default().sanitized();
        let kbs = Self::load_all_kbs_from_disk(&base_dir);
        let http = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap_or_default();

        Self {
            base_dir,
            config: RwLock::new(config),
            kbs: RwLock::new(kbs),
            http,
        }
    }

    fn config_path(base_dir: &Path) -> PathBuf {
        base_dir.join("rag-config.json")
    }

    fn kb_path(&self, kb_id: &str) -> Result<PathBuf, String> {
        if !is_valid_id(kb_id) {
            return Err("無效的知識庫 ID".to_string());
        }
        Ok(self.base_dir.join("kbs").join(format!("{}.json", kb_id)))
    }

    fn load_config_from_disk(base_dir: &Path) -> Option<RagConfig> {
        let raw = fs::read_to_string(Self::config_path(base_dir)).ok()?;
        serde_json::from_str::<RagConfig>(&raw).ok()
    }

    fn load_all_kbs_from_disk(base_dir: &Path) -> BTreeMap<String, KnowledgeBase> {
        let mut map = BTreeMap::new();
        let kbs_dir = base_dir.join("kbs");
        let Ok(entries) = fs::read_dir(&kbs_dir) else {
            return map;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            if let Ok(raw) = fs::read_to_string(&path)
                && let Ok(mut kb) = serde_json::from_str::<KnowledgeBase>(&raw)
                && is_valid_id(&kb.id)
            {
                kb.refresh_doc_counts();
                map.insert(kb.id.clone(), kb);
            }
        }
        map
    }

    fn write_json_atomic<T: Serialize>(&self, path: &Path, value: &T) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("建立目錄失敗：{}", e))?;
        }
        let json = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
        let tmp_path = path.with_extension("json.tmp");
        fs::write(&tmp_path, json)
            .map_err(|e| format!("寫入暫存檔失敗 {}: {}", tmp_path.display(), e))?;
        if let Err(err) = fs::rename(&tmp_path, path) {
            let _ = fs::remove_file(&tmp_path);
            return Err(format!("儲存檔案失敗 {}: {}", path.display(), err));
        }
        Ok(())
    }

    pub async fn get_config(&self) -> RagConfig {
        self.config.read().await.clone()
    }

    pub async fn save_config(&self, cfg: RagConfig) -> Result<RagConfig, String> {
        let clean = cfg.sanitized();
        self.write_json_atomic(&Self::config_path(&self.base_dir), &clean)?;
        *self.config.write().await = clean.clone();
        Ok(clean)
    }

    pub async fn test_embedding(&self, cfg: RagConfig) -> Result<EmbeddingTestResult, String> {
        let clean = cfg.sanitized();
        let url = normalize_embeddings_url(&clean.embedding_endpoint)?;
        let start = Instant::now();
        let vecs = fetch_embeddings(
            &self.http,
            &clean.embedding_endpoint,
            &clean.embedding_api_key,
            &clean.embedding_model,
            &["Gufo Studio RAG embedding connectivity test".to_string()],
        )
        .await?;
        let dim = vecs.first().map(|v| v.len()).unwrap_or(0);
        Ok(EmbeddingTestResult {
            ok: true,
            endpoint_url: url,
            model: clean.embedding_model,
            dimension: dim,
            latency_ms: start.elapsed().as_millis() as u64,
        })
    }

    pub async fn list_kbs(&self) -> Vec<KnowledgeBaseSummary> {
        let kbs = self.kbs.read().await;
        let mut list: Vec<KnowledgeBaseSummary> = kbs.values().map(|kb| kb.summary()).collect();
        list.sort_by_key(|kb| std::cmp::Reverse(kb.updated_at));
        list
    }

    pub async fn get_kb(&self, kb_id: &str) -> Result<KnowledgeBaseDetail, String> {
        let kbs = self.kbs.read().await;
        let kb = kbs.get(kb_id).ok_or_else(|| "找不到指定的知識庫".to_string())?;
        Ok(KnowledgeBaseDetail {
            summary: kb.summary(),
            documents: kb.documents.clone(),
        })
    }

    pub async fn create_kb(&self, name: String, description: String) -> Result<KnowledgeBaseSummary, String> {
        let trimmed_name = name.trim().to_string();
        if trimmed_name.is_empty() {
            return Err("知識庫名稱不可為空".to_string());
        }
        let now = now_unix_millis();
        let kb = KnowledgeBase {
            id: generate_id("kb"),
            name: trimmed_name,
            description: description.trim().to_string(),
            created_at: now,
            updated_at: now,
            embedding_model_tag: None,
            documents: Vec::new(),
            chunks: Vec::new(),
        };
        let path = self.kb_path(&kb.id)?;
        self.write_json_atomic(&path, &kb)?;
        let summary = kb.summary();
        self.kbs.write().await.insert(kb.id.clone(), kb);
        Ok(summary)
    }

    pub async fn delete_kb(&self, kb_id: &str) -> Result<(), String> {
        let path = self.kb_path(kb_id)?;
        let removed = self.kbs.write().await.remove(kb_id);
        if removed.is_none() && !path.exists() {
            return Err("找不到指定的知識庫".to_string());
        }
        if path.exists() {
            fs::remove_file(&path).map_err(|e| format!("刪除知識庫檔案失敗：{}", e))?;
        }
        Ok(())
    }

    pub async fn get_doc_chunks(
        &self,
        kb_id: &str,
        doc_id: &str,
    ) -> Result<Vec<RagChunkPreview>, String> {
        let kbs = self.kbs.read().await;
        let kb = kbs.get(kb_id).ok_or_else(|| "找不到指定的知識庫".to_string())?;
        let chunks = kb
            .chunks
            .iter()
            .filter(|c| c.doc_id == doc_id)
            .map(|c| RagChunkPreview {
                id: c.id.clone(),
                chunk_index: c.chunk_index,
                content: c.content.clone(),
                char_count: c.char_count,
                has_embedding: c.embedding.is_some(),
            })
            .collect();
        Ok(chunks)
    }

    pub async fn delete_document(&self, kb_id: &str, doc_id: &str) -> Result<(), String> {
        let path = self.kb_path(kb_id)?;
        let mut kbs = self.kbs.write().await;
        let kb = kbs.get_mut(kb_id).ok_or_else(|| "找不到指定的知識庫".to_string())?;
        let before = kb.documents.len();
        kb.documents.retain(|d| d.id != doc_id);
        if kb.documents.len() == before {
            return Err("找不到指定的文件".to_string());
        }
        kb.chunks.retain(|c| c.doc_id != doc_id);
        kb.updated_at = now_unix_millis();
        self.write_json_atomic(&path, kb)?;
        Ok(())
    }

    async fn ingest_entries(
        &self,
        kb_id: &str,
        entries: Vec<(String, String, String, String)>, // (name, source_path, file_type, text)
        skipped_files: usize,
        mut errors: Vec<RagImportItemError>,
    ) -> Result<RagImportResult, String> {
        let path = self.kb_path(kb_id)?;
        {
            let kbs = self.kbs.read().await;
            if !kbs.contains_key(kb_id) {
                return Err("找不到指定的知識庫".to_string());
            }
        }

        let cfg = self.get_config().await;
        let now = now_unix_millis();
        let mut new_docs = Vec::new();
        let mut new_chunks = Vec::new();
        let mut replace_source_paths: HashSet<String> = HashSet::new();

        for (name, source_path, file_type, text) in entries {
            let pieces = chunk_text(&text, cfg.chunk_size, cfg.chunk_overlap);
            if pieces.is_empty() {
                errors.push(RagImportItemError {
                    file: name,
                    reason: "文件內容為空，已略過".to_string(),
                });
                continue;
            }
            if !source_path.is_empty() {
                replace_source_paths.insert(source_path.clone());
            }
            let doc_id = generate_id("doc");
            let char_count = text.chars().count();
            let chunk_count = pieces.len();

            for (idx, content) in pieces.into_iter().enumerate() {
                new_chunks.push(RagChunk {
                    id: generate_id("chk"),
                    doc_id: doc_id.clone(),
                    doc_name: name.clone(),
                    chunk_index: idx,
                    char_count: content.chars().count(),
                    content,
                    embedding: None,
                });
            }

            new_docs.push(RagDocument {
                id: doc_id,
                name,
                source_path,
                file_type,
                char_count,
                chunk_count,
                embedded_chunk_count: 0,
                added_at: now,
            });
        }

        let mut embedded_chunks = 0usize;
        let mut embedding_warning: Option<String> = None;
        let mut embedded_model_tag: Option<String> = None;

        if cfg.embedding_enabled && !new_chunks.is_empty() {
            let texts: Vec<String> = new_chunks.iter().map(|c| c.content.clone()).collect();
            match fetch_embeddings(
                &self.http,
                &cfg.embedding_endpoint,
                &cfg.embedding_api_key,
                &cfg.embedding_model,
                &texts,
            )
            .await
            {
                Ok(vectors) => {
                    for (chunk, vec) in new_chunks.iter_mut().zip(vectors) {
                        chunk.embedding = Some(vec);
                        embedded_chunks += 1;
                    }
                    embedded_model_tag = Some(cfg.embedding_model.clone());
                }
                Err(err) => {
                    embedding_warning = Some(format!(
                        "文件已匯入並建立 BM25 索引，但向量化失敗（可稍後於知識庫按「重建向量索引」補齊）：{}",
                        err
                    ));
                }
            }
        }

        let mut kbs = self.kbs.write().await;
        let kb = kbs.get_mut(kb_id).ok_or_else(|| "找不到指定的知識庫".to_string())?;

        let mut replaced_docs = 0usize;
        if !replace_source_paths.is_empty() {
            let mut removed_doc_ids = HashSet::new();
            kb.documents.retain(|doc| {
                if !doc.source_path.is_empty() && replace_source_paths.contains(&doc.source_path) {
                    removed_doc_ids.insert(doc.id.clone());
                    replaced_docs += 1;
                    false
                } else {
                    true
                }
            });
            if !removed_doc_ids.is_empty() {
                kb.chunks.retain(|c| !removed_doc_ids.contains(&c.doc_id));
            }
        }

        let imported_docs = new_docs.len();
        let added_chunks = new_chunks.len();

        kb.documents.extend(new_docs);
        kb.chunks.extend(new_chunks);
        if let Some(tag) = embedded_model_tag {
            kb.embedding_model_tag = Some(tag);
        }
        kb.refresh_doc_counts();
        kb.updated_at = now_unix_millis();

        self.write_json_atomic(&path, kb)?;

        Ok(RagImportResult {
            imported_docs,
            replaced_docs,
            added_chunks,
            embedded_chunks,
            skipped_files,
            errors,
            embedding_warning,
        })
    }

    pub async fn add_files(
        &self,
        kb_id: &str,
        file_paths: Vec<String>,
    ) -> Result<RagImportResult, String> {
        let mut entries = Vec::new();
        let mut errors = Vec::new();

        for raw_path in file_paths {
            let p = PathBuf::from(&raw_path);
            let name = p
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(&raw_path)
                .to_string();
            let file_type = p
                .extension()
                .and_then(|e| e.to_str())
                .map(|s| s.to_ascii_lowercase())
                .unwrap_or_else(|| "txt".to_string());

            match read_text_file(&p, MAX_EXPLICIT_FILE_BYTES) {
                Ok(text) => entries.push((name, raw_path, file_type, text)),
                Err(reason) => errors.push(RagImportItemError { file: name, reason }),
            }
        }

        self.ingest_entries(kb_id, entries, 0, errors).await
    }

    pub async fn add_folder(
        &self,
        kb_id: &str,
        folder_path: String,
    ) -> Result<RagImportResult, String> {
        let root = PathBuf::from(&folder_path);
        if !root.is_dir() {
            return Err("指定的路徑不是資料夾".to_string());
        }

        let mut stack = vec![root.clone()];
        let mut candidate_files = Vec::new();
        let mut skipped_files = 0usize;

        while let Some(dir) = stack.pop() {
            let Ok(read_dir) = fs::read_dir(&dir) else {
                continue;
            };
            for entry in read_dir.flatten() {
                let path = entry.path();
                let name = entry.file_name().to_string_lossy().to_string();
                if path.is_dir() {
                    if !should_skip_dir(&name) {
                        stack.push(path);
                    }
                } else if path.is_file() {
                    if is_supported_text_extension(&path) {
                        if candidate_files.len() < MAX_FOLDER_FILES {
                            candidate_files.push(path);
                        } else {
                            skipped_files += 1;
                        }
                    } else {
                        skipped_files += 1;
                    }
                }
            }
        }

        let mut entries = Vec::new();
        let mut errors = Vec::new();

        for file in candidate_files {
            let rel_name = file
                .strip_prefix(&root)
                .unwrap_or(&file)
                .to_string_lossy()
                .to_string();
            let file_type = file
                .extension()
                .and_then(|e| e.to_str())
                .map(|s| s.to_ascii_lowercase())
                .unwrap_or_else(|| "txt".to_string());

            match read_text_file(&file, MAX_FOLDER_FILE_BYTES) {
                Ok(text) => entries.push((
                    rel_name,
                    file.to_string_lossy().to_string(),
                    file_type,
                    text,
                )),
                Err(reason) => errors.push(RagImportItemError {
                    file: rel_name,
                    reason,
                }),
            }
        }

        self.ingest_entries(kb_id, entries, skipped_files, errors)
            .await
    }

    pub async fn add_text(
        &self,
        kb_id: &str,
        title: String,
        content: String,
    ) -> Result<RagImportResult, String> {
        let clean_title = title.trim().to_string();
        if clean_title.is_empty() {
            return Err("文件標題不可為空".to_string());
        }
        if content.trim().is_empty() {
            return Err("文件內容不可為空".to_string());
        }
        let normalized = content.replace("\r\n", "\n");
        let entries = vec![(clean_title, String::new(), "note".to_string(), normalized)];
        self.ingest_entries(kb_id, entries, 0, Vec::new()).await
    }

    pub async fn reindex_kb(&self, kb_id: &str) -> Result<RagReindexResult, String> {
        let path = self.kb_path(kb_id)?;
        let cfg = self.get_config().await;
        if !cfg.embedding_enabled {
            return Err("尚未啟用 Embedding 向量服務，請先於設定中開啟並填入端點與模型".to_string());
        }

        let chunk_texts: Vec<String> = {
            let kbs = self.kbs.read().await;
            let kb = kbs.get(kb_id).ok_or_else(|| "找不到指定的知識庫".to_string())?;
            kb.chunks.iter().map(|c| c.content.clone()).collect()
        };

        if chunk_texts.is_empty() {
            return Ok(RagReindexResult {
                total_chunks: 0,
                embedded_chunks: 0,
                embedding_model_tag: None,
            });
        }

        let vectors = fetch_embeddings(
            &self.http,
            &cfg.embedding_endpoint,
            &cfg.embedding_api_key,
            &cfg.embedding_model,
            &chunk_texts,
        )
        .await?;

        let mut kbs = self.kbs.write().await;
        let kb = kbs.get_mut(kb_id).ok_or_else(|| "找不到指定的知識庫".to_string())?;
        if kb.chunks.len() != vectors.len() {
            return Err("知識庫在重建向量期間發生變動，請重試".to_string());
        }

        for (chunk, vec) in kb.chunks.iter_mut().zip(vectors) {
            chunk.embedding = Some(vec);
        }
        let embedded_chunks = kb.chunks.len();
        kb.embedding_model_tag = Some(cfg.embedding_model.clone());
        kb.refresh_doc_counts();
        kb.updated_at = now_unix_millis();

        self.write_json_atomic(&path, kb)?;

        Ok(RagReindexResult {
            total_chunks: embedded_chunks,
            embedded_chunks,
            embedding_model_tag: kb.embedding_model_tag.clone(),
        })
    }

    pub async fn search(
        &self,
        kb_ids: Vec<String>,
        query: String,
        top_k_override: Option<u32>,
    ) -> Result<RagSearchResponse, String> {
        let start = Instant::now();
        let trimmed_query = query.trim();
        if trimmed_query.is_empty() || kb_ids.is_empty() {
            return Ok(RagSearchResponse {
                hits: Vec::new(),
                used_embedding: false,
                warning: None,
                elapsed_ms: 0,
            });
        }

        let cfg = self.get_config().await;
        let top_k = top_k_override.unwrap_or(cfg.top_k).clamp(1, 30) as usize;

        struct Candidate<'a> {
            kb_id: String,
            kb_name: String,
            chunk: RagChunk,
            _phantom: std::marker::PhantomData<&'a ()>,
        }

        let mut candidates: Vec<Candidate<'_>> = Vec::new();
        let mut any_embedded = false;

        {
            let kbs = self.kbs.read().await;
            let unique_ids: HashSet<&str> = kb_ids.iter().map(|s| s.as_str()).collect();
            for id in unique_ids {
                if let Some(kb) = kbs.get(id) {
                    for chunk in &kb.chunks {
                        if chunk.embedding.is_some() {
                            any_embedded = true;
                        }
                        candidates.push(Candidate {
                            kb_id: kb.id.clone(),
                            kb_name: kb.name.clone(),
                            chunk: chunk.clone(),
                            _phantom: std::marker::PhantomData,
                        });
                    }
                }
            }
        }

        if candidates.is_empty() {
            return Ok(RagSearchResponse {
                hits: Vec::new(),
                used_embedding: false,
                warning: None,
                elapsed_ms: start.elapsed().as_millis() as u64,
            });
        }

        let mut query_embedding: Option<Vec<f32>> = None;
        let mut warning: Option<String> = None;

        if cfg.embedding_enabled && any_embedded && !cfg.embedding_model.is_empty() {
            match fetch_embeddings(
                &self.http,
                &cfg.embedding_endpoint,
                &cfg.embedding_api_key,
                &cfg.embedding_model,
                &[trimmed_query.to_string()],
            )
            .await
            {
                Ok(mut vecs) => {
                    query_embedding = vecs.pop();
                }
                Err(err) => {
                    warning = Some(format!(
                        "Embedding 服務暫時無法使用，已自動降級為 BM25 關鍵字檢索：{}",
                        err
                    ));
                }
            }
        }

        let used_embedding = query_embedding.is_some();

        // 1. 計算所有候選切片的 BM25 統計
        let query_tokens = tokenize(trimmed_query);
        let unique_query_terms: Vec<String> = {
            let mut seen = HashSet::new();
            query_tokens
                .into_iter()
                .filter(|t| seen.insert(t.clone()))
                .collect()
        };

        let n_docs = candidates.len() as f64;
        let mut doc_tf: Vec<HashMap<String, usize>> = Vec::with_capacity(candidates.len());
        let mut doc_lens: Vec<usize> = Vec::with_capacity(candidates.len());
        let mut df: HashMap<String, usize> = HashMap::new();
        let mut total_len = 0usize;

        for cand in &candidates {
            let tokens = tokenize(&cand.chunk.content);
            let len = tokens.len();
            total_len += len;
            doc_lens.push(len);

            let mut tf_map: HashMap<String, usize> = HashMap::new();
            for t in tokens {
                *tf_map.entry(t).or_insert(0) += 1;
            }
            for qt in &unique_query_terms {
                if tf_map.contains_key(qt) {
                    *df.entry(qt.clone()).or_insert(0) += 1;
                }
            }
            doc_tf.push(tf_map);
        }

        let avgdl = (total_len as f64 / n_docs).max(1.0);
        let k1 = 1.5f64;
        let b = 0.75f64;

        let mut hits: Vec<RagSearchHit> = Vec::new();

        for (idx, cand) in candidates.into_iter().enumerate() {
            let dl = doc_lens[idx] as f64;
            let tf_map = &doc_tf[idx];

            let mut bm25_raw = 0.0f64;
            for qt in &unique_query_terms {
                if let Some(&tf_val) = tf_map.get(qt) {
                    let df_val = *df.get(qt).unwrap_or(&0) as f64;
                    let idf = ((n_docs - df_val + 0.5) / (df_val + 0.5) + 1.0).ln();
                    let tf = tf_val as f64;
                    let denom = tf + k1 * (1.0 - b + b * (dl / avgdl));
                    bm25_raw += idf * ((tf * (k1 + 1.0)) / denom);
                }
            }
            let bm25_f32 = bm25_raw as f32;
            let bm25_norm = if bm25_f32 > 0.0 {
                bm25_f32 / (bm25_f32 + 3.5)
            } else {
                0.0
            };

            let vec_score = match (&query_embedding, &cand.chunk.embedding) {
                (Some(qv), Some(cv)) => cosine_similarity(qv, cv).map(|s| s.clamp(0.0, 1.0)),
                _ => None,
            };

            let (final_score, match_mode) = match vec_score {
                Some(vs) => {
                    let score = cfg.hybrid_alpha * vs + (1.0 - cfg.hybrid_alpha) * bm25_norm;
                    (score, "hybrid".to_string())
                }
                None => (bm25_norm, "bm25".to_string()),
            };

            if final_score <= 0.0 || final_score < cfg.min_score {
                continue;
            }

            hits.push(RagSearchHit {
                chunk_id: cand.chunk.id,
                kb_id: cand.kb_id,
                kb_name: cand.kb_name,
                doc_id: cand.chunk.doc_id,
                doc_name: cand.chunk.doc_name,
                chunk_index: cand.chunk.chunk_index,
                content: cand.chunk.content,
                score: final_score,
                bm25_score: bm25_f32,
                vector_score: vec_score,
                match_mode,
            });
        }

        hits.sort_by(|a, b| b.score.total_cmp(&a.score));
        hits.truncate(top_k);

        Ok(RagSearchResponse {
            hits,
            used_embedding,
            warning,
            elapsed_ms: start.elapsed().as_millis() as u64,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    fn temp_rag_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("gufo_rag_test_{}_{}", name, generate_id("t")));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn test_chunk_text_cjk_and_overlap() {
        let text = "第一段介紹 Gufo Studio 的架構。\n\n第二段說明 AMD Strix Halo 統一記憶體的優勢與本地 RAG 檢索流程。\n\n第三段包含實際操作範例與效能調優建議。";
        let chunks = chunk_text(text, 150, 30);
        assert_eq!(chunks.len(), 1);

        let long_text = "這是一個測試句子。".repeat(30);
        let multi = chunk_text(&long_text, 150, 30);
        assert!(multi.len() >= 2);
        for chunk in &multi {
            assert!(chunk.chars().count() <= 155);
            assert!(!chunk.is_empty());
        }
    }

    #[test]
    fn test_tokenize_cjk_and_ascii() {
        let tokens = tokenize("Gufo RAG 向量檢索 v0.35");
        assert!(tokens.contains(&"gufo".to_string()));
        assert!(tokens.contains(&"rag".to_string()));
        assert!(tokens.contains(&"向".to_string()));
        assert!(tokens.contains(&"向量".to_string()));
        assert!(tokens.contains(&"檢索".to_string()));
        assert!(tokens.contains(&"v0".to_string()));
        assert!(tokens.contains(&"35".to_string()));
    }

    #[test]
    fn test_normalize_embeddings_url() {
        assert_eq!(
            normalize_embeddings_url("http://127.0.0.1:12224").unwrap(),
            "http://127.0.0.1:12224/v1/embeddings"
        );
        assert_eq!(
            normalize_embeddings_url("http://127.0.0.1:12224/v1/").unwrap(),
            "http://127.0.0.1:12224/v1/embeddings"
        );
        assert_eq!(
            normalize_embeddings_url("http://127.0.0.1:12224/v1/embeddings").unwrap(),
            "http://127.0.0.1:12224/v1/embeddings"
        );
        assert!(normalize_embeddings_url("ftp://bad").is_err());
    }

    #[tokio::test]
    async fn test_kb_lifecycle_and_bm25_search() {
        let dir = temp_rag_dir("bm25");
        let mgr = RagManager::with_base_dir(dir.clone());

        let kb = mgr
            .create_kb("技術手冊".to_string(), "Strix Halo 文件".to_string())
            .await
            .unwrap();

        let res1 = mgr
            .add_text(
                &kb.id,
                "GPU 設定".to_string(),
                "AMD Ryzen AI MAX+ 395 支援高達 96GB 的 UMA 顯存分配，適合執行大型本地推論模型。".to_string(),
            )
            .await
            .unwrap();
        assert_eq!(res1.imported_docs, 1);
        assert_eq!(res1.added_chunks, 1);

        let res2 = mgr
            .add_text(
                &kb.id,
                "食譜筆記".to_string(),
                "番茄炒蛋需要準備三顆雞蛋、兩顆牛番茄與少許蔥花。".to_string(),
            )
            .await
            .unwrap();
        assert_eq!(res2.imported_docs, 1);

        let search = mgr
            .search(vec![kb.id.clone()], "UMA 顯存分配".to_string(), Some(3))
            .await
            .unwrap();
        assert!(!search.used_embedding);
        assert!(!search.hits.is_empty());
        assert_eq!(search.hits[0].doc_name, "GPU 設定");
        assert_eq!(search.hits[0].match_mode, "bm25");

        // 重啟 RagManager 應可從磁碟還原知識庫
        let mgr2 = RagManager::with_base_dir(dir.clone());
        let list = mgr2.list_kbs().await;
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].doc_count, 2);

        mgr2.delete_kb(&kb.id).await.unwrap();
        assert!(mgr2.list_kbs().await.is_empty());

        let _ = fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_hybrid_embedding_search_with_mock_server() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();

        std::thread::spawn(move || {
            for stream in listener.incoming().take(3) {
                let Ok(mut stream) = stream else { break };
                let mut buf = [0u8; 4096];
                let n = stream.read(&mut buf).unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]);
                let body_part = req.split("\r\n\r\n").nth(1).unwrap_or("");
                let parsed: serde_json::Value =
                    serde_json::from_str(body_part).unwrap_or_default();
                let inputs = parsed["input"].as_array().cloned().unwrap_or_default();
                let data: Vec<serde_json::Value> = inputs
                    .iter()
                    .enumerate()
                    .map(|(i, val)| {
                        let s = val.as_str().unwrap_or("");
                        let vec = if s.contains("密碼") || s.contains("憑證") || s.contains("TLS") {
                            vec![0.95, 0.05]
                        } else {
                            vec![0.05, 0.95]
                        };
                        serde_json::json!({ "index": i, "embedding": vec })
                    })
                    .collect();
                let resp_json = serde_json::json!({ "data": data }).to_string();
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    resp_json.len(),
                    resp_json
                );
                let _ = stream.write_all(resp.as_bytes());
            }
        });

        let dir = temp_rag_dir("hybrid");
        let mgr = RagManager::with_base_dir(dir.clone());
        let cfg = RagConfig {
            embedding_enabled: true,
            embedding_endpoint: format!("http://{}/v1", addr),
            embedding_model: "mock-embed".to_string(),
            ..Default::default()
        };
        mgr.save_config(cfg).await.unwrap();

        let kb = mgr.create_kb("混合測試".to_string(), "".to_string()).await.unwrap();
        let imp1 = mgr
            .add_text(&kb.id, "安全憑證".to_string(), "內建 TLS 憑證自動產生".to_string())
            .await
            .unwrap();
        assert_eq!(imp1.embedded_chunks, 1);

        let imp2 = mgr
            .add_text(&kb.id, "天氣".to_string(), "今天台北晴時多雲".to_string())
            .await
            .unwrap();
        assert_eq!(imp2.embedded_chunks, 1);

        // 即使查詢字詞與文件完全不重疊（「密碼」vs「TLS 憑證」），向量語意仍能命中「安全憑證」
        let res = mgr
            .search(vec![kb.id.clone()], "密碼".to_string(), Some(2))
            .await
            .unwrap();
        assert!(res.used_embedding);
        assert!(!res.hits.is_empty());
        assert_eq!(res.hits[0].doc_name, "安全憑證");
        assert_eq!(res.hits[0].match_mode, "hybrid");
        assert!(res.hits[0].vector_score.unwrap_or(0.0) > 0.9);

        let _ = fs::remove_dir_all(&dir);
    }
}
