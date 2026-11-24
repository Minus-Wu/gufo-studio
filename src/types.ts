export type ServerState = 'stopped' | 'starting' | 'running' | 'error';

export interface GufoServerConfig {
  gufo_path: string;
  model_path: string;
  mtp_path: string;
  vision_path: string;
  served_model_name: string;
  host: string;
  port: number;
  context: number;
  sessions: number;
  max_request_bytes: number;
  max_output_bytes: number;
  think: 'off' | 'on' | 'auto';
  temperature: number;
  speculative: 'mtp' | 'dflash2' | 'off';
  mtp_policy: 'survival' | 'length';
  prompt_lookup: boolean;
  api_key: string;
  auto_open_gui: boolean;
  auto_start_server: boolean;
  /** 請求未指定 max_tokens 時的預設生成上限 (0 = Gufo 預設，直到 EOS 或 context 用盡) */
  max_tokens: number;
  /** MTP 每輪草稿 token 數 (0 = Gufo 預設) */
  draft_tokens: number;
  mtp_draft_vocab: 'full' | 'latin';
  /** Prefill 分塊大小 (0 = Gufo 預設) */
  prefill_chunk: number;
  cache_disk: boolean;
  cache_disk_dir: string;
  /** 磁碟快取容量上限 bytes (0 = Gufo 預設) */
  cache_disk_bytes: number;
  https_enabled: boolean;
  https_port: number;
  https_cert_path: string;
  https_key_path: string;
  https_sans: string;
}

export interface ServerStatus {
  state: ServerState;
  pid: number | null;
  port: number;
  host: string;
  served_model_name: string;
  endpoint: string;
  uptime_secs: number;
  error_message: string | null;
  api_key?: string;
  https_enabled: boolean;
  https_port: number;
  https_running: boolean;
  https_endpoint: string | null;
  https_cert_path: string | null;
  lan_ip: string | null;
}

export interface MessageImage {
  url: string;
  name?: string;
}

export interface ChatMetrics {
  /** true 表示串流中的前端估算值 (以 SSE chunk 數計算，非精確 token 數) */
  approximate?: boolean;
  completionTokens?: number;
  /** 解碼速度 tok/s (完成後為伺服器量測值) */
  tokensPerSec?: number;
  ttftMs?: number;
  promptTokens?: number;
  /** 實際執行 prefill 的 token 數 (扣除快取命中) */
  prefillTokens?: number;
  prefillTps?: number;
  prefillMs?: number;
  cachedTokens?: number;
  cacheSource?: 'memory' | 'disk' | 'miss';
  cacheHitPct?: number;
  cacheRestoreMs?: number;
  cacheMissReason?: string;
  commonPrefixTokens?: number;
  nearestCheckpointTokens?: number;
  draftTokens?: number;
  draftAccepted?: number;
  lookupTokens?: number;
  lookupAccepted?: number;
}

export interface ChatMessage {
  id: string;
  role: 'user' | 'assistant' | 'system';
  content: string;
  reasoning?: string;
  images?: MessageImage[];
  timestamp: number;
  /** 歡迎訊息、錯誤提示等不送入模型的對話歷史 */
  excludeFromContext?: boolean;
  isError?: boolean;
  metrics?: ChatMetrics;
  ragHits?: RagSearchHit[];
  ragWarning?: string;
  ragElapsedMs?: number;
  webHits?: WebSearchHit[];
  webProviderUsed?: string;
  webWarning?: string;
  webElapsedMs?: number;
  statusNote?: string;
}

export interface ChatSession {
  id: string;
  title: string;
  messages: ChatMessage[];
  createdAt: number;
  updatedAt: number;
}

export interface ActiveInferenceRequest {
  request_id: string;
  method: string;
  path: string;
  body_bytes: number;
  timestamp: number;
}

export interface InferenceMetrics {
  request_id: string;
  method: string;
  path: string;
  status: number;
  duration_ms: number;
  prompt_tokens: number;
  prefill_tokens: number;
  generated_tokens: number;
  ttft_ms: number;
  prefill_tps: number;
  decode_tps: number;
  draft_accepted: number;
  draft_proposed: number;
  acceptance_pct?: number;
  outcome?: string;
  finish?: string;
  cache: 'memory' | 'disk' | 'miss' | string;
  cached_tokens: number;
  cache_hit_pct: number;
  cache_restore_ms: number;
  cache_miss_reason?: string;
  common_prefix_tokens?: number;
  nearest_checkpoint_tokens?: number;
  lookup_accepted?: number;
  lookup_proposed?: number;
  timestamp: number;
}

export interface RagConfig {
  chunk_size: number;
  chunk_overlap: number;
  top_k: number;
  min_score: number;
  embedding_enabled: boolean;
  embedding_endpoint: string;
  embedding_model: string;
  embedding_api_key: string;
  hybrid_alpha: number;
  system_prompt_template: string;
}

export interface KnowledgeBaseSummary {
  id: string;
  name: string;
  description: string;
  created_at: number;
  updated_at: number;
  doc_count: number;
  chunk_count: number;
  embedded_chunk_count: number;
  embedding_model_tag: string | null;
}

export interface RagDocument {
  id: string;
  name: string;
  source_path: string;
  file_type: string;
  char_count: number;
  chunk_count: number;
  embedded_chunk_count: number;
  added_at: number;
}

export interface KnowledgeBaseDetail {
  summary: KnowledgeBaseSummary;
  documents: RagDocument[];
}

export interface RagChunkPreview {
  id: string;
  chunk_index: number;
  content: string;
  char_count: number;
  has_embedding: boolean;
}

export interface RagImportItemError {
  file: string;
  reason: string;
}

export interface RagImportResult {
  imported_docs: number;
  replaced_docs: number;
  added_chunks: number;
  embedded_chunks: number;
  skipped_files: number;
  errors: RagImportItemError[];
  embedding_warning: string | null;
}

export interface RagReindexResult {
  total_chunks: number;
  embedded_chunks: number;
  embedding_model_tag: string | null;
}

export interface EmbeddingTestResult {
  ok: boolean;
  endpoint_url: string;
  model: string;
  dimension: number;
  latency_ms: number;
}

export interface RagSearchHit {
  chunk_id: string;
  kb_id: string;
  kb_name: string;
  doc_id: string;
  doc_name: string;
  chunk_index: number;
  content: string;
  score: number;
  bm25_score: number;
  vector_score: number | null;
  match_mode: 'hybrid' | 'bm25' | string;
}

export interface RagSearchResponse {
  hits: RagSearchHit[];
  used_embedding: boolean;
  warning: string | null;
  elapsed_ms: number;
}

export type WebSearchProvider =
  | 'auto'
  | 'duckduckgo'
  | 'bing_rss'
  | 'searxng'
  | 'tavily'
  | 'brave'
  | 'serper';

export interface WebSearchConfig {
  provider: WebSearchProvider;
  api_key: string;
  searxng_url: string;
  max_results: number;
  fetch_page_content: boolean;
  fetch_top_n: number;
  max_page_chars: number;
  timeout_secs: number;
  region: string;
  prompt_template: string;
}

export interface WebSearchHit {
  title: string;
  url: string;
  domain: string;
  snippet: string;
  fetched_content?: string | null;
  published_date?: string | null;
}

export interface WebSearchResponse {
  query: string;
  provider_used: string;
  hits: WebSearchHit[];
  warning: string | null;
  elapsed_ms: number;
}
