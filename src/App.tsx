import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import './styles.css';

import { Header } from './components/Header';
import { ChatView } from './components/ChatView';
import { RagView } from './components/RagView';
import { ConfigSubTab, ConfigView } from './components/ConfigView';
import { LogsView } from './components/LogsView';
import {
  ActiveInferenceRequest,
  GufoServerConfig,
  InferenceMetrics,
  KnowledgeBaseSummary,
  RagConfig,
  ServerStatus,
  WebSearchConfig,
} from './types';
import { DEFAULT_WEB_SEARCH_CONFIG } from './utils/webSearchHelpers';

export function App() {
  const [activeTab, setActiveTab] = useState<'chat' | 'rag' | 'config' | 'logs'>('chat');
  const [configSubTab, setConfigSubTab] = useState<ConfigSubTab>('inference');
  const [isStarting, setIsStarting] = useState(false);
  const [metricsHistory, setMetricsHistory] = useState<InferenceMetrics[]>([]);
  const [activeRequest, setActiveRequest] = useState<ActiveInferenceRequest | null>(null);
  const latestMetrics = metricsHistory.length > 0 ? metricsHistory[metricsHistory.length - 1] : null;

  // RAG 狀態
  const [kbList, setKbList] = useState<KnowledgeBaseSummary[]>([]);
  const [ragEnabled, setRagEnabled] = useState(false);
  const [selectedKbIds, setSelectedKbIds] = useState<string[]>([]);
  const [ragConfig, setRagConfig] = useState<RagConfig>({
    chunk_size: 800,
    chunk_overlap: 120,
    top_k: 5,
    min_score: 0.05,
    embedding_enabled: false,
    embedding_endpoint: 'http://127.0.0.1:12224/v1',
    embedding_model: '',
    embedding_api_key: '',
    hybrid_alpha: 0.6,
    system_prompt_template:
      '請優先依據以下「檢索參考資料」回答使用者的問題。回答時若引用了資料內容，請以 [來源編號]（例如 [1]、[2]）標註出處；若參考資料不足以回答或與問題無關，請誠實說明，不要編造事實。\n\n【檢索參考資料】\n{{context}}',
  });

  // WebSearch 狀態
  const [webSearchEnabled, setWebSearchEnabled] = useState(false);
  const [webSearchConfig, setWebSearchConfig] = useState<WebSearchConfig>(DEFAULT_WEB_SEARCH_CONFIG);

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
    cache_disk: true,
    cache_disk_dir: 'gufo-cache',
    cache_disk_bytes: 0,
    https_enabled: true,
    https_port: 12443,
    https_cert_path: '',
    https_key_path: '',
    https_sans: '',
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
          setMetricsHistory(recentMetrics);
        }

        const [loadedRagCfg, loadedKbs, loadedWebCfg] = await Promise.all([
          invoke<RagConfig>('rag_get_config'),
          invoke<KnowledgeBaseSummary[]>('rag_list_kbs'),
          invoke<WebSearchConfig>('web_search_get_config'),
        ]);
        setRagConfig(loadedRagCfg);
        setKbList(loadedKbs);
        if (loadedKbs.length > 0) {
          setSelectedKbIds(loadedKbs.map((k) => k.id));
        }
        setWebSearchConfig(loadedWebCfg);
      } catch (err) {
        console.error('初始化失敗:', err);
      }
    };

    init();

    // 監聽後端即時日誌事件
    const unlistenLogPromise = listen<string>('gufo-log', (event) => {
      setLogs((prev) => [...prev.slice(-999), event.payload]);
    });

    // 監聽推論進行中 / 結束事件
    const unlistenActivePromise = listen<ActiveInferenceRequest>('gufo-request-active', (event) => {
      setActiveRequest(event.payload);
    });

    const unlistenFinishedPromise = listen<string>('gufo-request-finished', (event) => {
      const finishedId = event.payload;
      setActiveRequest((prev) => (prev && prev.request_id === finishedId ? null : prev));
    });

    // 監聽推論指標事件 (反應 token/s 與 prefill / cache 命中率)
    const unlistenMetricsPromise = listen<InferenceMetrics>('gufo-metrics', (event) => {
      const metric = event.payload;
      setMetricsHistory((prev) => [...prev.slice(-49), metric]);
      setActiveRequest((prev) => (prev && prev.request_id === metric.request_id ? null : prev));
    });

    // 監聽狀態改變事件
    const unlistenStatusPromise = listen<string>('gufo-status-changed', async () => {
      try {
        const s = await invoke<ServerStatus>('get_server_status');
        setStatus(s);
        if (s.state !== 'running') {
          setActiveRequest(null);
        }
      } catch (e) {
        console.error(e);
      }
    });

    // 定期輪詢伺服器狀態（每 3 秒更新 uptime 與運行狀態）
    const interval = setInterval(async () => {
      try {
        const s = await invoke<ServerStatus>('get_server_status');
        setStatus(s);
        if (s.state !== 'running') {
          setActiveRequest(null);
        }
      } catch {
        // 忽略連線輪詢錯誤
      }
    }, 3000);

    return () => {
      clearInterval(interval);
      unlistenLogPromise.then((unlisten) => unlisten());
      unlistenActivePromise.then((unlisten) => unlisten());
      unlistenFinishedPromise.then((unlisten) => unlisten());
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
      setActiveRequest(null);
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

  // 重新載入 RAG 知識庫列表，並保留有效選取（新建立的知識庫自動選取）
  const refreshRagKbs = async () => {
    try {
      const list = await invoke<KnowledgeBaseSummary[]>('rag_list_kbs');
      const prevIds = new Set(kbList.map((k) => k.id));
      const validSet = new Set(list.map((k) => k.id));
      const newlyAdded = list.filter((k) => !prevIds.has(k.id)).map((k) => k.id);
      setKbList(list);
      setSelectedKbIds((prevSel) => {
        const kept = prevSel.filter((id) => validSet.has(id));
        return Array.from(new Set([...kept, ...newlyAdded]));
      });
    } catch (err) {
      console.error('載入知識庫列表失敗:', err);
    }
  };

  const handleSaveRagConfig = async (newCfg: RagConfig) => {
    const saved = await invoke<RagConfig>('rag_save_config', { config: newCfg });
    setRagConfig(saved);
  };

  const handleToggleKbId = (id: string) => {
    setSelectedKbIds((prev) =>
      prev.includes(id) ? prev.filter((item) => item !== id) : [...prev, id]
    );
  };

  const handleSaveWebSearchConfig = async (newCfg: WebSearchConfig) => {
    const saved = await invoke<WebSearchConfig>('web_search_save_config', { config: newCfg });
    setWebSearchConfig(saved);
  };

  const handleOpenWebSearchSettings = () => {
    setConfigSubTab('web_search');
    setActiveTab('config');
  };

  return (
    <>
      <Header
        activeTab={activeTab}
        setActiveTab={setActiveTab}
        status={status}
        latestMetrics={latestMetrics}
        activeRequest={activeRequest}
        onStartServer={handleStartServer}
        onStopServer={handleStopServer}
        isStarting={isStarting}
      />

      <main className="app-body">
        <div
          style={{
            display: activeTab === 'chat' ? 'flex' : 'none',
            flex: 1,
            minWidth: 0,
            height: '100%',
          }}
        >
          <ChatView
            status={status}
            defaultThink={config.think === 'on'}
            defaultTemperature={config.temperature}
            kbList={kbList}
            ragConfig={ragConfig}
            ragEnabled={ragEnabled}
            onToggleRag={setRagEnabled}
            selectedKbIds={selectedKbIds}
            onToggleKbId={handleToggleKbId}
            onOpenRagTab={() => setActiveTab('rag')}
            webSearchConfig={webSearchConfig}
            webSearchEnabled={webSearchEnabled}
            onToggleWebSearch={setWebSearchEnabled}
            onOpenWebSearchSettings={handleOpenWebSearchSettings}
          />
        </div>
        {activeTab === 'rag' && (
          <RagView
            kbList={kbList}
            ragConfig={ragConfig}
            onKbsChanged={refreshRagKbs}
            onSaveConfig={handleSaveRagConfig}
          />
        )}
        {activeTab === 'config' && (
          <ConfigView
            config={config}
            status={status}
            onSaveConfig={handleSaveConfig}
            onResetDefaults={handleResetDefaults}
            isRunning={status.state === 'running' || status.state === 'starting'}
            webSearchConfig={webSearchConfig}
            onSaveWebSearchConfig={handleSaveWebSearchConfig}
            activeSubTab={configSubTab}
            onSubTabChange={setConfigSubTab}
          />
        )}
        {activeTab === 'logs' && (
          <LogsView
            logs={logs}
            metricsHistory={metricsHistory}
            activeRequest={activeRequest}
            onClearLogs={() => setLogs([])}
          />
        )}
      </main>
    </>
  );
}

export default App;
