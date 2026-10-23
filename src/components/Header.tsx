import React, { useState } from 'react';
import { MessageSquare, Settings, Terminal, Play, Square, Copy, Check, Lock } from 'lucide-react';
import { ServerStatus } from '../types';

interface HeaderProps {
  activeTab: 'chat' | 'config' | 'logs';
  setActiveTab: (tab: 'chat' | 'config' | 'logs') => void;
  status: ServerStatus;
  onStartServer: () => void;
  onStopServer: () => void;
  isStarting: boolean;
}

export const Header: React.FC<HeaderProps> = ({
  activeTab,
  setActiveTab,
  status,
  onStartServer,
  onStopServer,
  isStarting,
}) => {
  const [copied, setCopied] = useState(false);

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
              ? '啟動中...'
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
