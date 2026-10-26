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
  cachedTokens?: number;
  draftTokens?: number;
  draftAccepted?: number;
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
}

export interface ChatSession {
  id: string;
  title: string;
  messages: ChatMessage[];
  createdAt: number;
  updatedAt: number;
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
  timestamp: number;
}
