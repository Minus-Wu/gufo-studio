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
} from 'lucide-react';
import { InferenceMetrics, ServerStatus } from '../types';

interface HeaderProps {
  activeTab: 'chat' | 'config' | 'logs';
  setActiveTab: (tab: 'chat' | 'config' | 'logs') => void;
  status: ServerStatus;
  latestMetrics?: InferenceMetrics | null;
  onStartServer: () => void;
  onStopServer: () => void;
  isStarting: boolean;
}

export const Header: React.FC<HeaderProps> = ({
  activeTab,
  setActiveTab,
  status,
  latestMetrics,
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
          <span>對話 (Chat)</span>
        </button>
        <button
          className={`nav-tab-btn ${activeTab === 'config' ? 'active' : ''}`}
          onClick={() => setActiveTab('config')}
        >
          <Settings size={16} />
          <span>配置 (Config)</span>
        </button>
        <button
          className={`nav-tab-btn ${activeTab === 'logs' ? 'active' : ''}`}
          onClick={() => setActiveTab('logs')}
        >
          <Terminal size={16} />
          <span>即時日誌 (Logs)</span>
        </button>
      </nav>

      <div className="header-actions">
        {/* 全局 API 推論效能即時看板（反應 token/s, prefill, TTFT, 投機採納率） */}
        {latestMetrics && status.state === 'running' && (
          <div
            className={`global-metrics-badge ${pulse ? 'metric-pulse' : ''}`}
            title={`最近 API 請求：${latestMetrics.method} ${latestMetrics.path} (${latestMetrics.duration_ms.toFixed(0)}ms)\n` +
              `提示詞：${latestMetrics.prompt_tokens} tokens (Prefill: ${latestMetrics.prefill_tps.toFixed(0)} tok/s)\n` +
              `生成量：${latestMetrics.generated_tokens} tokens (Decode: ${latestMetrics.decode_tps.toFixed(1)} tok/s)\n` +
              `首字延遲 (TTFT)：${latestMetrics.ttft_ms.toFixed(1)}ms\n` +
              `MTP 投機解碼命中：${latestMetrics.draft_accepted}/${latestMetrics.draft_proposed}` +
              (latestMetrics.acceptance_pct != null ? ` (${latestMetrics.acceptance_pct.toFixed(0)}%)` : '')}
          >
            <div className="metric-chip decode">
              <Zap size={13} className="metric-icon zap" />
              <span className="metric-val">{latestMetrics.decode_tps.toFixed(1)}</span>
              <span className="metric-unit">tok/s</span>
            </div>

            <span className="metric-sep">|</span>

            <div className="metric-chip prefill">
              <Activity size={13} className="metric-icon prefill" />
              <span className="metric-label">Prefill</span>
              <span className="metric-val">{latestMetrics.prefill_tps.toFixed(0)}</span>
              <span className="metric-unit">tok/s</span>
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
          title={`點擊複製 OpenAI API 端點：${status.endpoint}${status.api_key ? ' (已啟用 API Key 保護)' : ''}`}
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
              ? `Port ${status.port} (${formatUptime(status.uptime_secs)})`
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
