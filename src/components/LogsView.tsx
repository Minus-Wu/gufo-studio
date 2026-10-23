import React, { useRef, useEffect, useState } from 'react';
import { Trash2, Copy, Check, ArrowDownCircle } from 'lucide-react';

interface LogsViewProps {
  logs: string[];
  onClearLogs: () => void;
}

export const LogsView: React.FC<LogsViewProps> = ({ logs, onClearLogs }) => {
  const terminalEndRef = useRef<HTMLDivElement>(null);
  const [autoScroll, setAutoScroll] = useState(true);
  const [copied, setCopied] = useState(false);

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

  return (
    <div className="logs-container">
      <div className="logs-toolbar">
        <div style={{ display: 'flex', alignItems: 'center', gap: 8, fontSize: 13, color: '#94a3b8' }}>
          <span>共 {logs.length} 行紀錄</span>
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
