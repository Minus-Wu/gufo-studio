import React, { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import {
  Globe,
  Save,
  Eye,
  EyeOff,
  Search,
  Loader2,
  ExternalLink,
  AlertCircle,
  CheckCircle2,
  FileText,
} from 'lucide-react';
import { WebSearchConfig, WebSearchProvider, WebSearchResponse } from '../types';
import {
  DEFAULT_WEB_PROMPT_TEMPLATE,
  WEB_SEARCH_PROVIDERS,
  formatWebProviderLabel,
  openSafeExternalUrl,
} from '../utils/webSearchHelpers';

interface WebSearchSettingsViewProps {
  config: WebSearchConfig;
  onSaveConfig: (cfg: WebSearchConfig) => Promise<void>;
}

export const WebSearchSettingsView: React.FC<WebSearchSettingsViewProps> = ({
  config,
  onSaveConfig,
}) => {
  const [form, setForm] = useState<WebSearchConfig>(config);
  const [showApiKey, setShowApiKey] = useState(false);
  const [saving, setSaving] = useState(false);
  const [savedOk, setSavedOk] = useState(false);

  const [testQuery, setTestQuery] = useState('');
  const [testing, setTesting] = useState(false);
  const [testError, setTestError] = useState<string | null>(null);
  const [testResult, setTestResult] = useState<WebSearchResponse | null>(null);

  useEffect(() => {
    setForm(config);
  }, [config]);

  const providerMeta =
    WEB_SEARCH_PROVIDERS.find((p) => p.value === form.provider) ?? WEB_SEARCH_PROVIDERS[0];

  const handleSave = async (e: React.FormEvent) => {
    e.preventDefault();
    setSaving(true);
    try {
      await onSaveConfig(form);
      setSavedOk(true);
      setTimeout(() => setSavedOk(false), 3000);
    } catch (err: any) {
      alert(`儲存 WebSearch 設定失敗：${err}`);
    } finally {
      setSaving(false);
    }
  };

  const handleTestSearch = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!testQuery.trim() || testing) return;
    setTesting(true);
    setTestError(null);
    setTestResult(null);
    try {
      const res = await invoke<WebSearchResponse>('web_search_query', {
        query: testQuery.trim(),
        configOverride: form,
      });
      setTestResult(res);
    } catch (err: any) {
      setTestError(String(err));
    } finally {
      setTesting(false);
    }
  };

  return (
    <>
      <form onSubmit={handleSave}>
        <div className="config-card">
          <div className="card-title">
            <Globe size={18} color="#38bdf8" />
            <span>網路搜尋引擎與內容擷取設定 (WebSearch)</span>
          </div>
          <div className="card-subtitle">
            在對話頁開啟「網路搜尋」後，會即時搜尋網路結果並選擇性擷取前幾筆網頁正文作為模型回答依據。預設為免
            API Key 的自動模式（DuckDuckGo 與 Bing RSS 備援）。
          </div>

          <div className="form-grid">
            <div className="form-group">
              <label className="form-label">搜尋引擎提供者 (Provider)</label>
              <select
                className="form-input"
                value={form.provider}
                onChange={(e) =>
                  setForm((p) => ({
                    ...p,
                    provider: e.target.value as WebSearchProvider,
                  }))
                }
              >
                {WEB_SEARCH_PROVIDERS.map((p) => (
                  <option key={p.value} value={p.value}>
                    {p.label}
                  </option>
                ))}
              </select>
              <span style={{ fontSize: 11.5, color: '#64748b' }}>{providerMeta.hint}</span>
            </div>

            {providerMeta.needsApiKey && (
              <div className="form-group">
                <label className="form-label">搜尋 API Key</label>
                <div className="path-input-group">
                  <input
                    type={showApiKey ? 'text' : 'password'}
                    className="form-input"
                    placeholder={`輸入 ${formatWebProviderLabel(form.provider)} API Key`}
                    value={form.api_key}
                    onChange={(e) => setForm((p) => ({ ...p, api_key: e.target.value }))}
                  />
                  <button
                    type="button"
                    className="btn btn-secondary"
                    onClick={() => setShowApiKey((v) => !v)}
                    title={showApiKey ? '隱藏' : '顯示'}
                  >
                    {showApiKey ? <EyeOff size={14} /> : <Eye size={14} />}
                  </button>
                </div>
              </div>
            )}

            {providerMeta.needsSearxngUrl && (
              <div className="form-group">
                <label className="form-label">SearXNG 實例網址</label>
                <input
                  type="text"
                  className="form-input"
                  placeholder="http://127.0.0.1:8080"
                  value={form.searxng_url}
                  onChange={(e) => setForm((p) => ({ ...p, searxng_url: e.target.value }))}
                />
                <span style={{ fontSize: 11.5, color: '#64748b' }}>
                  會請求 <code>{form.searxng_url.replace(/\/+$/, '') || '...'}/search?format=json</code>
                </span>
              </div>
            )}

            <div className="form-group">
              <label className="form-label">地區 / 語系代碼 (選填)</label>
              <input
                type="text"
                className="form-input"
                placeholder="例如：wt-wt 或 zh-TW（留空為預設）"
                value={form.region}
                onChange={(e) => setForm((p) => ({ ...p, region: e.target.value }))}
              />
              <span style={{ fontSize: 11.5, color: '#64748b' }}>
                DuckDuckGo 可填 <code>tw-tzh</code> / <code>wt-wt</code>；Bing / SearXNG / Serper 可填{' '}
                <code>zh-TW</code> 或 <code>tw</code>
              </span>
            </div>

            <div className="form-group">
              <label className="form-label">最大搜尋筆數 (Max Results: 1–10)</label>
              <input
                type="number"
                className="form-input"
                min={1}
                max={10}
                value={form.max_results}
                onChange={(e) =>
                  setForm((p) => ({ ...p, max_results: Number(e.target.value) || 5 }))
                }
              />
            </div>

            <div className="form-group">
              <label className="form-label">請求逾時秒數 (Timeout: 3–30s)</label>
              <input
                type="number"
                className="form-input"
                min={3}
                max={30}
                value={form.timeout_secs}
                onChange={(e) =>
                  setForm((p) => ({ ...p, timeout_secs: Number(e.target.value) || 10 }))
                }
              />
            </div>
          </div>

          <div
            style={{
              marginTop: 18,
              paddingTop: 14,
              borderTop: '1px solid var(--border-color)',
            }}
          >
            <label
              style={{
                display: 'flex',
                alignItems: 'center',
                gap: 8,
                cursor: 'pointer',
                marginBottom: 12,
              }}
            >
              <input
                type="checkbox"
                checked={form.fetch_page_content}
                onChange={(e) =>
                  setForm((p) => ({ ...p, fetch_page_content: e.target.checked }))
                }
              />
              <span style={{ fontWeight: 500 }}>
                自動擷取前幾筆搜尋結果的網頁正文（推薦，能大幅提升回答完整度）
              </span>
            </label>

            {form.fetch_page_content && (
              <div className="form-grid">
                <div className="form-group">
                  <label className="form-label">擷取正文筆數 (Top-N: 1–5)</label>
                  <input
                    type="number"
                    className="form-input"
                    min={1}
                    max={5}
                    value={form.fetch_top_n}
                    onChange={(e) =>
                      setForm((p) => ({ ...p, fetch_top_n: Number(e.target.value) || 3 }))
                    }
                  />
                </div>
                <div className="form-group">
                  <label className="form-label">單頁最大擷取字元數 (300–6000)</label>
                  <input
                    type="number"
                    className="form-input"
                    min={300}
                    max={6000}
                    step={100}
                    value={form.max_page_chars}
                    onChange={(e) =>
                      setForm((p) => ({ ...p, max_page_chars: Number(e.target.value) || 1500 }))
                    }
                  />
                </div>
              </div>
            )}
          </div>
        </div>

        <div className="config-card">
          <div className="card-title">
            <FileText size={18} color="#10b981" />
            <span>WebSearch 提示詞模板</span>
          </div>
          <div className="card-subtitle">
            支援 <code>{'{{date}}'}</code>（今天日期）與 <code>{'{{results}}'}</code>（搜尋結果清單）佔位符。
          </div>
          <div className="form-group">
            <textarea
              className="form-input"
              rows={5}
              style={{ resize: 'vertical', fontFamily: 'var(--font-sans)' }}
              value={form.prompt_template}
              onChange={(e) => setForm((p) => ({ ...p, prompt_template: e.target.value }))}
            />
          </div>

          <div
            style={{
              marginTop: 14,
              display: 'flex',
              alignItems: 'center',
              justifyContent: 'space-between',
              gap: 10,
            }}
          >
            <button
              type="button"
              className="btn btn-secondary"
              onClick={() =>
                setForm((p) => ({ ...p, prompt_template: DEFAULT_WEB_PROMPT_TEMPLATE }))
              }
            >
              還原預設模板
            </button>

            <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
              {savedOk && (
                <span
                  style={{
                    color: '#34d399',
                    fontSize: 13,
                    display: 'inline-flex',
                    alignItems: 'center',
                    gap: 4,
                  }}
                >
                  <CheckCircle2 size={14} /> 已儲存 WebSearch 設定
                </span>
              )}
              <button type="submit" className="btn btn-primary" disabled={saving}>
                <Save size={14} />
                <span>{saving ? '儲存中...' : '儲存 WebSearch 設定'}</span>
              </button>
            </div>
          </div>
        </div>
      </form>

      {/* 即時搜尋測試工作台 */}
      <div className="config-card">
        <div className="card-title">
          <Search size={18} color="#c084fc" />
          <span>即時網路搜尋測試</span>
        </div>
        <div className="card-subtitle">
          直接以目前表單設定測試連線與擷取結果（不需先儲存即可測試）。
        </div>

        <form onSubmit={handleTestSearch} className="path-input-group" style={{ marginBottom: 14 }}>
          <input
            type="text"
            className="form-input"
            style={{ fontFamily: 'var(--font-sans)' }}
            placeholder="輸入搜尋關鍵字，例如：AMD Ryzen AI MAX+ 395 ROCm 最新支援"
            value={testQuery}
            onChange={(e) => setTestQuery(e.target.value)}
          />
          <button
            type="submit"
            className="btn btn-emerald"
            disabled={testing || !testQuery.trim()}
          >
            {testing ? <Loader2 size={14} className="spin-icon" /> : <Search size={14} />}
            <span>{testing ? '搜尋中...' : '測試網路搜尋'}</span>
          </button>
        </form>

        {testError && (
          <div className="rag-banner error">
            <AlertCircle size={15} />
            <span>搜尋失敗：{testError}</span>
          </div>
        )}

        {testResult && (
          <div>
            <div className="rag-stats-bar" style={{ marginBottom: 12 }}>
              <span>使用引擎：{formatWebProviderLabel(testResult.provider_used)}</span>
              <span>結果筆數：{testResult.hits.length}</span>
              <span>耗時：{testResult.elapsed_ms} ms</span>
            </div>

            {testResult.warning && (
              <div className="rag-banner warning" style={{ marginBottom: 12 }}>
                <AlertCircle size={15} />
                <span>{testResult.warning}</span>
              </div>
            )}

            {testResult.hits.length === 0 ? (
              <div className="rag-empty-docs">沒有回傳任何搜尋結果，請嘗試更換關鍵字或引擎。</div>
            ) : (
              <div className="rag-hits-list">
                {testResult.hits.map((hit, idx) => (
                  <div key={`${hit.url}-${idx}`} className="rag-hit-card">
                    <div className="rag-hit-header">
                      <button
                        type="button"
                        className="web-hit-link"
                        onClick={() => openSafeExternalUrl(hit.url)}
                        title={hit.url}
                      >
                        <span>
                          [W{idx + 1}] {hit.title}
                        </span>
                        <ExternalLink size={12} />
                      </button>
                      <div className="rag-hit-scores">
                        {hit.domain && <span className="rag-type-badge">{hit.domain}</span>}
                        {hit.fetched_content && (
                          <span className="cache-badge ram">
                            已擷取正文 ({hit.fetched_content.length} 字)
                          </span>
                        )}
                      </div>
                    </div>
                    {hit.snippet && (
                      <div style={{ fontSize: 12.5, color: '#cbd5e1', marginBottom: hit.fetched_content ? 8 : 0 }}>
                        {hit.snippet}
                      </div>
                    )}
                    {hit.fetched_content && (
                      <pre className="rag-hit-content" style={{ color: '#94a3b8', fontSize: 12 }}>
                        {hit.fetched_content}
                      </pre>
                    )}
                  </div>
                ))}
              </div>
            )}
          </div>
        )}
      </div>
    </>
  );
};
