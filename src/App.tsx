import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import './styles.css';

import { Header } from './components/Header';
import { ChatView } from './components/ChatView';
import { ConfigView } from './components/ConfigView';
import { LogsView } from './components/LogsView';
import { GufoServerConfig, InferenceMetrics, ServerStatus } from './types';

export function App() {
  const [activeTab, setActiveTab] = useState<'chat' | 'config' | 'logs'>('chat');
  const [isStarting, setIsStarting] = useState(false);
  const [latestMetrics, setLatestMetrics] = useState<InferenceMetrics | null>(null);

  const [status, setStatus] = useState<ServerStatus>({
    state: 'stopped',
    pid: null,
    port: 12223,
    host: '0.0.0.0',
    served_model_name: 'flash-next',
    endpoint: 'http://127.0.0.1:12223/v1',
    uptime_secs: 0,
    error_message: null,
    https_enabled: true,
    https_port: 12443,
    https_running: false,
    https_endpoint: 'https://127.0.0.1:12443/v1',
    https_cert_path: null,
    lan_ip: null,
  });

  const [config, setConfig] = useState<GufoServerConfig>({
    gufo_path: 'build\\release\\gufo.exe',
    model_path:
      'models\\Qwen3.8-Flash-Next-GGUF\\Qwen3.8-Flash-Next-UD-Q4_K_XL-00001-of-00004.gguf',
    mtp_path:
      'models\\Qwen3.8-Flash-Next-GGUF\\mtp-Qwen3.8-Flash-Next-shared-Q8_0.gguf',
    vision_path: 'models\\Qwen3.8-Flash-Next-GGUF\\mmproj-BF16.gguf',
    served_model_name: 'flash-next',
    host: '0.0.0.0',
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
    auto_open_gui: true,
    auto_start_server: false,
    max_tokens: 32768,
    draft_tokens: 0,
    mtp_draft_vocab: 'full',
    prefill_chunk: 0,
    cache_disk: false,
    cache_disk_dir: 'gufo-cache',
    cache_disk_bytes: 0,
    https_enabled: true,
    https_port: 12443,
    https_cert_path: '',
    https_key_path: '',
  });

  const [logs, setLogs] = useState<string[]>([]);

  // 載入設定與狀態 (gufo-studio.config.json 為唯一設定來源)
  useEffect(() => {
    const init = async () => {
      try {
        let cfg = await invoke<GufoServerConfig>('get_config');

        // 一次性遷移：舊版把設定存在 localStorage，若磁碟尚無設定檔則沿用並寫回磁碟
        const legacy = localStorage.getItem('gufo_studio_config');
        if (legacy) {
          try {
            const hasSaved = await invoke<boolean>('has_saved_config');
            const parsed = JSON.parse(legacy);
            const isStaleAbsolute =
              typeof parsed.gufo_path === 'string' &&
              (parsed.gufo_path.includes('D:\\Agents\\gufo') ||
                parsed.gufo_path.includes('D:/Agents/gufo'));
            if (!hasSaved && parsed && typeof parsed === 'object' && !isStaleAbsolute) {
              cfg = { ...cfg, ...parsed };
              await invoke('save_config', { config: cfg });
            }
          } catch (e) {
            console.warn('舊版設定遷移失敗，改用磁碟設定:', e);
          }
          localStorage.removeItem('gufo_studio_config');
        }
        setConfig(cfg);

        const currentStatus = await invoke<ServerStatus>('get_server_status');
        setStatus(currentStatus);

        const recentLogs = await invoke<string[]>('get_recent_logs');
        setLogs(recentLogs);

        const recentMetrics = await invoke<InferenceMetrics[]>('get_recent_metrics');
        if (recentMetrics && recentMetrics.length > 0) {
          setLatestMetrics(recentMetrics[recentMetrics.length - 1]);
        }
      } catch (err) {
        console.error('初始化失敗:', err);
      }
    };

    init();

    // 監聽後端即時日誌事件
    const unlistenLogPromise = listen<string>('gufo-log', (event) => {
      setLogs((prev) => [...prev.slice(-999), event.payload]);
    });

    // 監聽推論指標事件 (反應 token/s 與 prefill 速度)
    const unlistenMetricsPromise = listen<InferenceMetrics>('gufo-metrics', (event) => {
      setLatestMetrics(event.payload);
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
      unlistenMetricsPromise.then((unlisten) => unlisten());
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

  // 保存設定到便攜設定檔 gufo-studio.config.json
  const handleSaveConfig = async (newConfig: GufoServerConfig) => {
    setConfig(newConfig);
    try {
      await invoke('save_config', { config: newConfig });
    } catch (e) {
      alert(`無法儲存設定檔到磁碟：${e}`);
    }
  };

  // 還原預設配置 (同時寫回設定檔)
  const handleResetDefaults = async () => {
    try {
      const defaultCfg = await invoke<GufoServerConfig>('get_default_config');
      setConfig(defaultCfg);
      await invoke('save_config', { config: defaultCfg });
    } catch (err) {
      alert(`還原預設配置失敗：${err}`);
    }
  };

  return (
    <>
      <Header
        activeTab={activeTab}
        setActiveTab={setActiveTab}
        status={status}
        latestMetrics={latestMetrics}
        onStartServer={handleStartServer}
        onStopServer={handleStopServer}
        isStarting={isStarting}
      />

      <main className="app-body">
        {activeTab === 'chat' && (
          <ChatView
            status={status}
            defaultThink={config.think === 'on'}
            defaultTemperature={config.temperature}
          />
        )}
        {activeTab === 'config' && (
          <ConfigView
            config={config}
            status={status}
            onSaveConfig={handleSaveConfig}
            onResetDefaults={handleResetDefaults}
            isRunning={status.state === 'running' || status.state === 'starting'}
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
