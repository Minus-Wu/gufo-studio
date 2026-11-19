import React, { useRef, useEffect, useState } from 'react';
import {
  Trash2,
  Copy,
  Check,
  ArrowDownCircle,
  ChevronDown,
  ChevronUp,
  Database,
  Loader2,
} from 'lucide-react';
import { ActiveInferenceRequest, InferenceMetrics } from '../types';
import {
  describeCacheMissReason,
  formatCacheSourceLabel,
  formatPayloadSize,
  formatTokenCount,
} from '../utils/metricsFormat';

interface LogsViewProps {
  logs: string[];
  metricsHistory?: InferenceMetrics[];
  activeRequest?: ActiveInferenceRequest | null;
  onClearLogs: () => void;
}

export const LogsView: React.FC<LogsViewProps> = ({
  logs,
  metricsHistory = [],
  activeRequest,
  onClearLogs,
}) => {
  const terminalEndRef = useRef<HTMLDivElement>(null);
  const [autoScroll, setAutoScroll] = useState(true);
  const [copied, setCopied] = useState(false);
  const [showMetricsPanel, setShowMetricsPanel] = useState(true);

  useEffect(() => {
    if (autoScroll) {
      terminalEndRef.current?.scrollIntoView({ behavior: 'smooth' });
    }
  }, [logs, autoScroll]);

  const handleCopy = () => {
    navigator.clipboard.writeText(logs.join('\n'));
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  };

  const reversedMetrics = [...metricsHistory].reverse();
  const totalPrompt = metricsHistory.reduce((acc, m) => acc + m.prompt_tokens, 0);
  const totalCached = metricsHistory.reduce((acc, m) => acc + m.cached_tokens, 0);
  const avgHitPct = totalPrompt > 0 ? (totalCached * 100) / totalPrompt : null;

  return (
    <div className="logs-container">
      <div className="logs-toolbar">
        <div style={{ display: 'flex', alignItems: 'center', gap: 10, fontSize: 13, color: '#94a3b8' }}>
          <span>共 {logs.length} 行紀錄</span>
          {activeRequest && (
            <span className="logs-active-pill">
              <Loader2 size={12} className="spin-icon" />
              <span>
                推論進行中 #{activeRequest.request_id} ({formatPayloadSize(activeRequest.body_bytes)})
              </span>
            </span>
          )}
        </div>

        <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
          <button
            className={`btn btn-secondary ${autoScroll ? 'active' : ''}`}
            onClick={() => setAutoScroll(!autoScroll)}
            style={{ fontSize: 12, padding: '4px 10px' }}
          >
            <ArrowDownCircle size={13} color={autoScroll ? '#10b981' : '#64748b'} />
            <span>自動滾動：{autoScroll ? '開啟' : '關閉'}</span>
          </button>

          <button
            className="btn btn-secondary"
            onClick={handleCopy}
            style={{ fontSize: 12, padding: '4px 10px' }}
          >
            {copied ? <Check size={13} color="#10b981" /> : <Copy size={13} />}
            <span>{copied ? '已複製！' : '複製全部'}</span>
          </button>

          <button
            className="btn btn-secondary"
            onClick={onClearLogs}
            style={{ fontSize: 12, padding: '4px 10px' }}
          >
            <Trash2 size={13} />
            <span>清空日誌</span>
          </button>
        </div>
      </div>

      {/* API 請求 Prefill & Cache 命中歷史明細面板 */}
      <div className="metrics-panel">
        <button
          type="button"
          className="metrics-panel-header"
          onClick={() => setShowMetricsPanel((prev) => !prev)}
        >
          <div className="metrics-panel-title">
            <Database size={14} color="#38bdf8" />
            <span>最近 API 請求 Prefill &amp; 快取命中明細</span>
            <span className="metrics-count-pill">{metricsHistory.length} 筆</span>
            {avgHitPct !== null && (
              <span className="metrics-summary-pill">
                整體快取命中 {avgHitPct.toFixed(1)}% ({formatTokenCount(totalCached)}/
                {formatTokenCount(totalPrompt)} tok)
              </span>
            )}
          </div>
          <span className="metrics-panel-toggle">
            {showMetricsPanel ? <ChevronUp size={15} /> : <ChevronDown size={15} />}
          </span>
        </button>

        {showMetricsPanel && (
          <div className="metrics-table-wrap">
            {!activeRequest && reversedMetrics.length === 0 ? (
              <div className="metrics-empty">
                尚無完成的 API 推論請求。透過 Copilot 或本機對話發送請求後，將在此顯示每次的 Cache 命中率、Rewind 斷點診斷與實際 Prefill 速度。
              </div>
            ) : (
              <table className="metrics-table">
                <thead>
                  <tr>
                    <th>請求 / 時間</th>
                    <th>Prompt</th>
                    <th>Cache 命中率 &amp; 來源</th>
                    <th>實際 Prefill</th>
                    <th>TTFT</th>
                    <th>Decode</th>
                    <th>MTP / Lookup</th>
                  </tr>
                </thead>
                <tbody>
                  {activeRequest && (
                    <tr className="metrics-row-active">
                      <td>
                        <div className="req-id-cell">
                          <Loader2 size={12} className="spin-icon" />
                          <span>#{activeRequest.request_id}</span>
                        </div>
                        <div className="req-time-sub">
                          {new Date(activeRequest.timestamp).toLocaleTimeString()}
                        </div>
                      </td>
                      <td colSpan={6}>
                        <span className="active-row-msg">
                          🔄 推論進行中（{activeRequest.method} {activeRequest.path}，請求大小{' '}
                          {formatPayloadSize(activeRequest.body_bytes)}）...
                        </span>
                      </td>
                    </tr>
                  )}

                  {reversedMetrics.map((m, idx) => {
                    const isHit = m.cached_tokens > 0;
                    const isFullSkip = isHit && m.prefill_tokens === 0;
                    const mtpPct =
                      m.draft_proposed > 0
                        ? m.acceptance_pct ?? (m.draft_accepted * 100) / m.draft_proposed
                        : null;
                    const cacheTitle = isHit
                      ? `來源：${formatCacheSourceLabel(m.cache)}，還原耗時 ${m.cache_restore_ms.toFixed(1)}ms`
                      : m.cache_miss_reason
                      ? describeCacheMissReason(
                          m.cache_miss_reason,
                          m.common_prefix_tokens,
                          m.nearest_checkpoint_tokens
                        )
                      : 'Cache Miss';

                    return (
                      <tr key={`${m.request_id}-${m.timestamp}-${idx}`}>
                        <td>
                          <div className="req-id-cell">
                            <span>#{m.request_id}</span>
                            {m.status >= 400 && (
                              <span className="req-warn-tag">HTTP {m.status}</span>
                            )}
                            {m.finish && m.finish !== 'stop' && (
                              <span className="req-finish-tag">{m.finish}</span>
                            )}
                          </div>
                          <div className="req-time-sub">
                            {new Date(m.timestamp).toLocaleTimeString()} · {m.path}
                          </div>
                        </td>
                        <td className="mono-cell">{m.prompt_tokens.toLocaleString()} tok</td>
                        <td title={cacheTitle}>
                          <div className="cache-cell">
                            <span
                              className={`cache-badge ${
                                isHit ? (m.cache === 'disk' ? 'disk' : 'ram') : 'miss'
                              }`}
                            >
                              {isHit
                                ? `${m.cache_hit_pct.toFixed(1)}% (${formatTokenCount(
                                    m.cached_tokens
                                  )} · ${formatCacheSourceLabel(m.cache)})`
                                : m.common_prefix_tokens != null && m.common_prefix_tokens > 0
                                ? `Miss (斷點 @${formatTokenCount(m.common_prefix_tokens)}${
                                    m.nearest_checkpoint_tokens
                                      ? ` / 快照 ${formatTokenCount(m.nearest_checkpoint_tokens)}`
                                      : ''
                                  })`
                                : 'Miss (0%)'}
                            </span>
                            <div className="cache-bar-track">
                              <div
                                className={`cache-bar-fill ${
                                  m.cache === 'disk' ? 'disk' : isHit ? 'ram' : 'miss'
                                }`}
                                style={{ width: `${Math.max(0, Math.min(100, m.cache_hit_pct))}%` }}
                              />
                            </div>
                          </div>
                        </td>
                        <td className="mono-cell">
                          {isFullSkip ? (
                            <span className="prefill-skip-tag">0 tok (免重算)</span>
                          ) : (
                            <span>
                              {m.prefill_tokens.toLocaleString()} tok @{' '}
                              {Math.round(m.prefill_tps).toLocaleString()} tok/s
                            </span>
                          )}
                        </td>
                        <td className="mono-cell">{m.ttft_ms.toFixed(1)} ms</td>
                        <td className="mono-cell">
                          {m.generated_tokens.toLocaleString()} tok @ {m.decode_tps.toFixed(1)} tok/s
                        </td>
                        <td className="mono-cell">
                          {mtpPct !== null ? (
                            <span title={`MTP 草稿接受 ${m.draft_accepted}/${m.draft_proposed}`}>
                              {mtpPct.toFixed(0)}%
                              {m.lookup_proposed && m.lookup_proposed > 0
                                ? ` (+L ${m.lookup_accepted ?? 0}/${m.lookup_proposed})`
                                : ''}
                            </span>
                          ) : m.lookup_proposed && m.lookup_proposed > 0 ? (
                            <span>
                              Lookup {m.lookup_accepted ?? 0}/{m.lookup_proposed}
                            </span>
                          ) : (
                            <span style={{ color: '#64748b' }}>-</span>
                          )}
                        </td>
                      </tr>
                    );
                  })}
                </tbody>
              </table>
            )}
          </div>
        )}
      </div>

      <div className="terminal-window">
        {logs.length === 0 ? (
          <div style={{ color: '#64748b', fontStyle: 'italic', padding: 20 }}>
            暫無日誌輸出。當伺服器啟動時，Gufo 的 ROCm 與推論引擎日誌將即時顯示在此。
          </div>
        ) : (
          logs.map((line, idx) => {
            const isSystem = line.startsWith('[系統]');
            const isError = line.toLowerCase().includes('error') || line.toLowerCase().includes('失敗');
            return (
              <div
                key={idx}
                className={`log-line ${isSystem ? 'system' : ''} ${isError ? 'error' : ''}`}
              >
                {line}
              </div>
            );
          })
        )}
        <div ref={terminalEndRef} />
      </div>
    </div>
  );
};
