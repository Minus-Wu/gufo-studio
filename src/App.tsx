import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import './styles.css';

import { Header } from './components/Header';
import { ChatView } from './components/ChatView';
import { ConfigView } from './components/ConfigView';
import { LogsView } from './components/LogsView';
import { GufoServerConfig, ServerStatus } from './types';

export function App() {
  const [activeTab, setActiveTab] = useState<'chat' | 'config' | 'logs'>('chat');
  const [isStarting, setIsStarting] = useState(false);

  const [status, setStatus] = useState<ServerStatus>({
    state: 'stopped',
    pid: null,
    port: 12223,
    host: '127.0.0.1',
    served_model_name: 'flash-next',
    endpoint: 'http://127.0.0.1:12223/v1',
    uptime_secs: 0,
    error_message: null,
  });

  const [config, setConfig] = useState<GufoServerConfig>({
    gufo_path: 'build\\release\\gufo.exe',
    model_path:
      'models\\Qwen3.8-Flash-Next-GGUF\\Qwen3.8-Flash-Next-UD-Q4_K_XL-00001-of-00004.gguf',
    mtp_path:
      'models\\Qwen3.8-Flash-Next-GGUF\\mtp-Qwen3.8-Flash-Next-shared-Q8_0.gguf',
    vision_path: 'models\\Qwen3.8-Flash-Next-GGUF\\mmproj-BF16.gguf',
    served_model_name: 'flash-next',
    host: '127.0.0.1',
    port: 12223,
    context: 131072,
    sessions: 2,
    max_request_bytes: 67108864,
    max_output_bytes: 8388608,
    think: 'off',
    temperature: 0.0,
    speculative: 'mtp',
    mtp_policy: 'survival',
    prompt_lookup: true,
    api_key: '',
  });

  const [logs, setLogs] = useState<string[]>([]);

  // 載入預設設定與狀態
  useEffect(() => {
    const init = async () => {
      try {
        const defaultCfg = await invoke<GufoServerConfig>('get_default_config');

        // 若 localStorage 存有設定且不是舊版寫死的 D:\Agents 絕對路徑，才使用 localStorage
        const saved = localStorage.getItem('gufo_studio_config');
        if (saved) {
          try {
            const parsed = JSON.parse(saved);
            if (
              parsed.gufo_path &&
              !parsed.gufo_path.includes('D:\\Agents\\gufo') &&
              !parsed.gufo_path.includes('D:/Agents/gufo')
            ) {
              setConfig(parsed);
            } else {
              setConfig(defaultCfg);
              localStorage.setItem('gufo_studio_config', JSON.stringify(defaultCfg));
            }
          } catch {
            setConfig(defaultCfg);
          }
        } else {
          setConfig(defaultCfg);
        }

        const currentStatus = await invoke<ServerStatus>('get_server_status');
        setStatus(currentStatus);

        const recentLogs = await invoke<string[]>('get_recent_logs');
        setLogs(recentLogs);
      } catch (err) {
        console.error('初始化失敗:', err);
      }
    };

    init();

    // 監聽後端即時日誌事件
    const unlistenLogPromise = listen<string>('gufo-log', (event) => {
      setLogs((prev) => [...prev.slice(-999), event.payload]);
    });

    // 監聽狀態改變事件
    const unlistenStatusPromise = listen<string>('gufo-status-changed', async () => {
      try {
        const s = await invoke<ServerStatus>('get_server_status');
        setStatus(s);
      } catch (e) {
        console.error(e);
      }
    });

    // 定期輪詢伺服器狀態（每 3 秒更新 uptime 與運行狀態）
    const interval = setInterval(async () => {
      try {
        const s = await invoke<ServerStatus>('get_server_status');
        setStatus(s);
      } catch {
        // 忽略連線輪詢錯誤
      }
    }, 3000);

    return () => {
      clearInterval(interval);
      unlistenLogPromise.then((unlisten) => unlisten());
      unlistenStatusPromise.then((unlisten) => unlisten());
    };
  }, []);

  // 啟動伺服器
  const handleStartServer = async () => {
    setIsStarting(true);
    try {
      setLogs((prev) => [...prev, '[系統] 收到啟動指令，正在配置伺服器...']);
      const res = await invoke<ServerStatus>('start_server', { config });
      setStatus(res);
    } catch (err: any) {
      alert(`啟動伺服器失敗：${err}`);
      const current = await invoke<ServerStatus>('get_server_status');
      setStatus(current);
      setActiveTab('logs');
    } finally {
      setIsStarting(false);
    }
  };

  // 停止伺服器
  const handleStopServer = async () => {
    try {
      const res = await invoke<ServerStatus>('stop_server');
      setStatus(res);
    } catch (err: any) {
      alert(`停止伺服器失敗：${err}`);
    }
  };

  // 保存設定（同時寫入前端 LocalStorage 與後端便攜設定檔 gufo-studio.config.json）
  const handleSaveConfig = async (newConfig: GufoServerConfig) => {
    setConfig(newConfig);
    localStorage.setItem('gufo_studio_config', JSON.stringify(newConfig));
    try {
      await invoke('save_config', { config: newConfig });
    } catch (e) {
      console.warn('無法儲存設定檔到磁碟:', e);
    }
  };

  // 還原預設配置
  const handleResetDefaults = async () => {
    try {
      const defaultCfg = await invoke<GufoServerConfig>('get_default_config');
      setConfig(defaultCfg);
    } catch (err) {
      console.error(err);
    }
  };

  return (
    <>
      <Header
        activeTab={activeTab}
        setActiveTab={setActiveTab}
        status={status}
        onStartServer={handleStartServer}
        onStopServer={handleStopServer}
        isStarting={isStarting}
      />

      <main className="app-body">
        {activeTab === 'chat' && (
          <ChatView status={status} onStartServer={handleStartServer} />
        )}
        {activeTab === 'config' && (
          <ConfigView
            config={config}
            onSaveConfig={handleSaveConfig}
            onResetDefaults={handleResetDefaults}
            isRunning={status.state === 'running'}
          />
        )}
        {activeTab === 'logs' && (
          <LogsView logs={logs} onClearLogs={() => setLogs([])} />
        )}
      </main>
    </>
  );
}

export default App;
