import React, { useState, useEffect } from 'react';
import {
  MessageSquare,
  Settings,
  Terminal,
  Play,
  Square,
  Copy,
  Check,
  Lock,
  Zap,
  Clock,
  Sparkles,
  Activity,
  Database,
  Loader2,
  BookOpen,
} from 'lucide-react';
import { ActiveInferenceRequest, InferenceMetrics, ServerStatus } from '../types';
import {
  describeCacheMissReason,
  formatCacheSourceLabel,
  formatPayloadSize,
  formatTokenCount,
} from '../utils/metricsFormat';

interface HeaderProps {
  activeTab: 'chat' | 'rag' | 'config' | 'logs';
  setActiveTab: (tab: 'chat' | 'rag' | 'config' | 'logs') => void;
  status: ServerStatus;
  latestMetrics?: InferenceMetrics | null;
  activeRequest?: ActiveInferenceRequest | null;
  onStartServer: () => void;
  onStopServer: () => void;
  isStarting: boolean;
}

function buildMetricsTooltip(m: InferenceMetrics): string {
  const lines: string[] = [
    `最近 API 請求：#${m.request_id} ${m.method} ${m.path} (${m.duration_ms.toFixed(0)}ms, finish: ${m.finish || 'stop'})`,
    `提示詞結構：總計 ${m.prompt_tokens.toLocaleString()} tokens = 快取命中 ${m.cached_tokens.toLocaleString()} (${m.cache_hit_pct.toFixed(1)}%) + 實際 Prefill ${m.prefill_tokens.toLocaleString()} tokens`,
  ];

  const sourceDesc =
    m.cache === 'disk'
      ? '磁碟快取 (Disk)'
      : m.cache === 'memory'
      ? '記憶體快取 (RAM)'
      : '未命中 (Miss)';
  lines.push(
    m.cached_tokens > 0
      ? `快取狀態：${sourceDesc}（還原耗時 ${m.cache_restore_ms.toFixed(1)}ms）`
      : `快取狀態：${sourceDesc}`
  );

  if (m.cached_tokens === 0 && m.cache_miss_reason) {
    lines.push(
      `未命中診斷：${describeCacheMissReason(
        m.cache_miss_reason,
        m.common_prefix_tokens,
        m.nearest_checkpoint_tokens
      )}`
    );
  }

  lines.push(
    m.prefill_tokens === 0 && m.cached_tokens > 0
      ? `Prefill：免重算（100% 命中快取）`
      : `Prefill：${m.prefill_tps.toFixed(1)} tok/s（實際處理 ${m.prefill_tokens.toLocaleString()} tokens）`
  );
  lines.push(`Decode：${m.generated_tokens.toLocaleString()} tokens (${m.decode_tps.toFixed(1)} tok/s)`);
  lines.push(`首字延遲 (TTFT)：${m.ttft_ms.toFixed(1)}ms`);

  if (m.draft_proposed > 0) {
    lines.push(
      `MTP 投機解碼命中：${m.draft_accepted}/${m.draft_proposed}` +
        (m.acceptance_pct != null ? ` (${m.acceptance_pct.toFixed(0)}%)` : '')
    );
  }
  if (m.lookup_proposed && m.lookup_proposed > 0) {
    lines.push(`Prompt Lookup 命中：${m.lookup_accepted ?? 0}/${m.lookup_proposed}`);
  }
  return lines.join('\n');
}

export const Header: React.FC<HeaderProps> = ({
  activeTab,
  setActiveTab,
  status,
  latestMetrics,
  activeRequest,
  onStartServer,
  onStopServer,
  isStarting,
}) => {
  const [copied, setCopied] = useState(false);
  const [pulse, setPulse] = useState(false);

  // 當有新的 API 推論指標進來時觸發呼吸燈高亮動畫
  useEffect(() => {
    if (latestMetrics) {
      setPulse(true);
      const timer = setTimeout(() => setPulse(false), 1200);
      return () => clearTimeout(timer);
    }
  }, [latestMetrics?.timestamp]);

  const handleCopyEndpoint = () => {
    navigator.clipboard.writeText(status.endpoint);
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  };

  const formatUptime = (secs: number) => {
    const mins = Math.floor(secs / 60);
    const hours = Math.floor(mins / 60);
    if (hours > 0) return `${hours}h ${mins % 60}m`;
    if (mins > 0) return `${mins}m ${secs % 60}s`;
    return `${secs}s`;
  };

  const fullCacheHit =
    !!latestMetrics && latestMetrics.prefill_tokens === 0 && latestMetrics.cached_tokens > 0;
  const cacheHit = !!latestMetrics && latestMetrics.cached_tokens > 0;

  return (
    <header className="app-header">
      <div className="brand-section">
        <div className="brand-logo">🦉</div>
        <div>
          <span className="brand-title">Gufo Studio</span>
        </div>
        <span className="brand-badge">AMD Strix Halo (gfx1151)</span>
      </div>

      <nav className="nav-tabs">
        <button
          className={`nav-tab-btn ${activeTab === 'chat' ? 'active' : ''}`}
          onClick={() => setActiveTab('chat')}
        >
          <MessageSquare size={16} />
          <span>
            對話<span className="tab-en"> (Chat)</span>
          </span>
        </button>
        <button
          className={`nav-tab-btn ${activeTab === 'rag' ? 'active' : ''}`}
          onClick={() => setActiveTab('rag')}
        >
          <BookOpen size={16} />
          <span>
            知識庫<span className="tab-en"> (RAG)</span>
          </span>
        </button>
        <button
          className={`nav-tab-btn ${activeTab === 'config' ? 'active' : ''}`}
          onClick={() => setActiveTab('config')}
        >
          <Settings size={16} />
          <span>
            配置<span className="tab-en"> (Config)</span>
          </span>
        </button>
        <button
          className={`nav-tab-btn ${activeTab === 'logs' ? 'active' : ''}`}
          onClick={() => setActiveTab('logs')}
        >
          <Terminal size={16} />
          <span>
            即時日誌<span className="tab-en"> (Logs)</span>
          </span>
        </button>
      </nav>

      <div className="header-actions">
        {/* 推論進行中即時狀態指示 */}
        {activeRequest && status.state === 'running' && (
          <div
            className="active-request-badge"
            title={`推論進行中：#${activeRequest.request_id} ${activeRequest.method} ${activeRequest.path}\n請求大小：${formatPayloadSize(activeRequest.body_bytes)}`}
          >
            <Loader2 size={13} className="spin-icon" />
            <span>推論中 #{activeRequest.request_id}</span>
            <span className="metric-sub">({formatPayloadSize(activeRequest.body_bytes)})</span>
          </div>
        )}

        {/* 全局 API 推論效能即時看板（反應 token/s, prefill, cache 命中率, TTFT, 投機採納率） */}
        {latestMetrics && status.state === 'running' && (
          <div
            className={`global-metrics-badge ${pulse ? 'metric-pulse' : ''}`}
            title={buildMetricsTooltip(latestMetrics)}
          >
            <div className="metric-chip decode">
              <Zap size={13} className="metric-icon zap" />
              <span className="metric-val">{latestMetrics.decode_tps.toFixed(1)}</span>
              <span className="metric-unit">tok/s</span>
            </div>

            <span className="metric-sep">|</span>

            <div className={`metric-chip prefill ${fullCacheHit ? 'is-skipped' : ''}`}>
              <Activity size={13} className="metric-icon prefill" />
              <span className="metric-label">Prefill</span>
              {fullCacheHit ? (
                <span className="metric-val">免重算</span>
              ) : (
                <>
                  <span className="metric-val">{latestMetrics.prefill_tps.toFixed(0)}</span>
                  <span className="metric-unit">tok/s</span>
                  <span className="metric-sub">
                    ({formatTokenCount(latestMetrics.prefill_tokens)})
                  </span>
                </>
              )}
            </div>

            <span className="metric-sep">|</span>

            <div
              className={`metric-chip cache ${
                cacheHit ? (latestMetrics.cache === 'disk' ? 'is-disk' : 'is-hit') : 'is-miss'
              }`}
            >
              <Database size={13} className="metric-icon cache" />
              <span className="metric-label">Cache</span>
              <span className="metric-val">{latestMetrics.cache_hit_pct.toFixed(0)}%</span>
              {cacheHit ? (
                <span className="metric-sub">
                  ({formatTokenCount(latestMetrics.cached_tokens)}/
                  {formatTokenCount(latestMetrics.prompt_tokens)}·
                  {formatCacheSourceLabel(latestMetrics.cache)})
                </span>
              ) : latestMetrics.common_prefix_tokens != null &&
                latestMetrics.common_prefix_tokens > 0 ? (
                <span className="metric-sub">
                  (斷點@{formatTokenCount(latestMetrics.common_prefix_tokens)})
                </span>
              ) : (
                <span className="metric-sub">(Miss)</span>
              )}
            </div>

            <span className="metric-sep">|</span>

            <div className="metric-chip ttft">
              <Clock size={13} className="metric-icon ttft" />
              <span className="metric-label">TTFT</span>
              <span className="metric-val">{latestMetrics.ttft_ms.toFixed(0)}</span>
              <span className="metric-unit">ms</span>
            </div>

            {latestMetrics.acceptance_pct != null && latestMetrics.draft_proposed > 0 && (
              <>
                <span className="metric-sep">|</span>
                <div className="metric-chip draft">
                  <Sparkles size={13} className="metric-icon sparkles" />
                  <span className="metric-label">MTP</span>
                  <span className="metric-val">{latestMetrics.acceptance_pct.toFixed(0)}%</span>
                </div>
              </>
            )}
          </div>
        )}

        <div
          className="server-badge"
          title={
            `點擊複製 OpenAI API 端點：${status.endpoint}` +
            (status.https_running && status.https_endpoint ? `\nHTTPS 端點：${status.https_endpoint}` : '') +
            (status.lan_ip ? `\n區域網路 IP：${status.lan_ip}` : '') +
            (status.api_key ? '\n(已啟用 API Key 保護)' : '')
          }
          onClick={handleCopyEndpoint}
          style={{ cursor: 'pointer' }}
        >
          <span className={`status-dot ${status.state}`} />
          {status.api_key && (
            <span title="API Key 保護已啟用" style={{ display: 'flex', alignItems: 'center' }}>
              <Lock size={12} color="#10b981" />
            </span>
          )}
          <span>
            {status.state === 'running'
              ? `Port ${status.port}${status.https_running ? ` / HTTPS ${status.https_port}` : ''} (${formatUptime(status.uptime_secs)})`
              : status.state === 'starting'
              ? '模型載入中...'
              : status.state === 'error'
              ? '發生錯誤'
              : '已停止'}
          </span>
          {copied ? <Check size={12} color="#10b981" /> : <Copy size={12} color="#94a3b8" />}
        </div>

        {status.state === 'running' ? (
          <button className="btn btn-danger" onClick={onStopServer}>
            <Square size={14} />
            <span>停止伺服器</span>
          </button>
        ) : (
          <button
            className="btn btn-emerald"
            onClick={onStartServer}
            disabled={isStarting || status.state === 'starting'}
          >
            <Play size={14} />
            <span>{isStarting || status.state === 'starting' ? '正在載入...' : '啟動伺服器'}</span>
          </button>
        )}
      </div>
    </header>
  );
};
