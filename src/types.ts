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

export interface ChatMessage {
  id: string;
  role: 'user' | 'assistant' | 'system';
  content: string;
  reasoning?: string;
  images?: MessageImage[];
  timestamp: number;
  metrics?: {
    totalTokens?: number;
    tokensPerSec?: number;
    ttftMs?: number;
  };
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
