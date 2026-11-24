import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import {
  Save,
  RotateCcw,
  CheckCircle2,
  AlertCircle,
  HardDrive,
  Cpu,
  Shield,
  Eye,
  EyeOff,
  Sparkles,
  FolderOpen,
  SlidersHorizontal,
  Info,
  Check,
  Laptop,
  Terminal,
  Code,
  Zap,
  Lock,
  Globe,
  RefreshCw,
  Layers,
  Plus,
  Trash2,
} from 'lucide-react';
import {
  GufoServerConfig,
  ModelProfile,
  ServerStatus,
  SpeculativeMode,
  WebSearchConfig,
  YarnMode,
} from '../types';
import { WebSearchSettingsView } from './WebSearchSettingsView';

export type ConfigSubTab = 'inference' | 'preferences' | 'web_search' | 'api_guide';

interface ConfigViewProps {
  config: GufoServerConfig;
  status: ServerStatus;
  onSaveConfig: (newConfig: GufoServerConfig) => void;
  onResetDefaults: () => void;
  isRunning: boolean;
  webSearchConfig: WebSearchConfig;
  onSaveWebSearchConfig: (cfg: WebSearchConfig) => Promise<void>;
  activeSubTab?: ConfigSubTab;
  onSubTabChange?: (tab: ConfigSubTab) => void;
}

const GIB = 1024 ** 3;
const round2 = (n: number) => Math.round(n * 100) / 100;

const clampNumber = (raw: string, min: number, max: number, fallback: number) => {
  const n = parseFloat(raw);
  if (!Number.isFinite(n)) return fallback;
  return Math.min(max, Math.max(min, n));
};

const clampInt = (raw: string, min: number, max = Number.MAX_SAFE_INTEGER) =>
  Math.trunc(clampNumber(raw, min, max, min));

const hintStyle: React.CSSProperties = { fontSize: 11.5, color: '#64748b', marginTop: 2 };
const checkboxLabelStyle: React.CSSProperties = {
  display: 'flex',
  alignItems: 'center',
  gap: 8,
  fontSize: 13,
  cursor: 'pointer',
};

const STANDARD_CONTEXT_OPTIONS = [32768, 65536, 131072, 262144, 409600, 524288, 1048576];

function splitCliArgsPreview(raw: string): string[] {
  const args: string[] = [];
  let current = '';
  let hasToken = false;
  let inSingle = false;
  let inDouble = false;

  for (let i = 0; i < raw.length; i++) {
    const c = raw[i];
    if (inDouble) {
      if (c === '\\' && raw[i + 1] === '"') {
        current += '"';
        i++;
      } else if (c === '"') {
        inDouble = false;
      } else {
        current += c;
      }
    } else if (inSingle) {
      if (c === "'") {
        inSingle = false;
      } else {
        current += c;
      }
    } else if (/\s/.test(c)) {
      if (hasToken) {
        args.push(current);
        current = '';
        hasToken = false;
      }
    } else if (c === '"') {
      inDouble = true;
      hasToken = true;
    } else if (c === "'") {
      inSingle = true;
      hasToken = true;
    } else {
      current += c;
      hasToken = true;
    }
  }
  if (hasToken) {
    args.push(current);
  }
  return args;
}

function quoteCliPreview(arg: string): string {
  return arg.includes(' ') ? `"${arg}"` : arg;
}

function buildCliPreview(cfg: GufoServerConfig): string {
  const bindHost = (cfg.host || '').trim().replace(/^\[|\]$/g, '') || '0.0.0.0';
  const args: string[] = [
    cfg.gufo_path || 'gufo.exe',
    'serve',
    '--host',
    bindHost,
    '--port',
    String(cfg.port),
    '--sessions',
    String(cfg.sessions),
    '--max-request-bytes',
    String(cfg.max_request_bytes),
    'llm',
    '--model',
    cfg.model_path || '<model.gguf>',
    '--served-model-name',
    cfg.served_model_name || 'model',
    '--context',
    String(cfg.context),
  ];

  const yarnMode = cfg.yarn_mode || 'auto';
  const enableYarn =
    yarnMode === 'on' || (yarnMode !== 'off' && cfg.context > 262144);
  if (enableYarn) {
    args.push('--enable-yarn');
  }

  args.push(
    '--max-output-bytes',
    String(cfg.max_output_bytes),
    '--think',
    cfg.think,
    '--temperature',
    String(cfg.temperature)
  );
  if (cfg.max_tokens > 0) {
    args.push('--max-tokens', String(cfg.max_tokens));
  }
  if (cfg.speculative === 'mtp' && cfg.mtp_path.trim()) {
    args.push('--speculative', 'mtp', '--mtp-model', cfg.mtp_path.trim());
  } else if (cfg.speculative === 'dspark' && cfg.dspark_path.trim()) {
    args.push('--speculative', 'dspark', '--dspark-model', cfg.dspark_path.trim());
  }
  if (cfg.cache_disk) {
    args.push('--cache-disk', cfg.cache_disk_dir.trim() || 'gufo-cache');
    if (cfg.cache_disk_bytes > 0) {
      args.push('--cache-disk-bytes', String(cfg.cache_disk_bytes));
    }
  }
  if (cfg.vision_path.trim()) {
    args.push('--mmproj', cfg.vision_path.trim());
  }
  if (cfg.extra_args?.trim()) {
    args.push(...splitCliArgsPreview(cfg.extra_args));
  }
  return args.map(quoteCliPreview).join(' ');
}

export const ConfigView: React.FC<ConfigViewProps> = ({
  config: initialConfig,
  status,
  onSaveConfig,
  onResetDefaults,
  isRunning,
  webSearchConfig,
  onSaveWebSearchConfig,
  activeSubTab,
  onSubTabChange,
}) => {
  const [internalSubTab, setInternalSubTab] = useState<ConfigSubTab>('inference');
  const subTab = activeSubTab ?? internalSubTab;
  const setSubTab = (tab: ConfigSubTab) => {
    setInternalSubTab(tab);
    onSubTabChange?.(tab);
  };
  const [form, setForm] = useState<GufoServerConfig>(initialConfig);
  const [pathStatus, setPathStatus] = useState<{ [key: string]: boolean }>({});
  const [savedSuccess, setSavedSuccess] = useState(false);
  const [prefSaved, setPrefSaved] = useState(false);
  const [showApiKey, setShowApiKey] = useState(false);
  const [certRegenMsg, setCertRegenMsg] = useState<string | null>(null);
  const [showSaveProfile, setShowSaveProfile] = useState(false);
  const [newProfileName, setNewProfileName] = useState('');
  const [profileNotice, setProfileNotice] = useState<string | null>(null);

  const handleRegenerateCert = async () => {
    try {
      const certPath = await invoke<string>('regenerate_https_cert', {
        host: form.host,
        httpsSans: form.https_sans,
      });
      setCertRegenMsg(`已重新產生自簽憑證：${certPath}`);
      setTimeout(() => setCertRegenMsg(null), 5000);
    } catch (err) {
      alert(`重新產生憑證失敗：${err}`);
    }
  };

  const generateApiKey = () => {
    const randomHex = Array.from(crypto.getRandomValues(new Uint8Array(16)))
      .map((b) => b.toString(16).padStart(2, '0'))
      .join('');
    handleChange('api_key', `sk-gufo-${randomHex}`);
  };

  const handleBrowse = async (
    field: keyof GufoServerConfig,
    title: string,
    filterName: string,
    extensions: string[]
  ) => {
    try {
      const selected = await invoke<string | null>('select_file', {
        title,
        filterName,
        extensions,
        defaultPath: (form[field] as string) || undefined,
      });
      if (selected) {
        handleChange(field, selected);
      }
    } catch (err) {
      console.error('選擇檔案失敗:', err);
    }
  };

  useEffect(() => {
    setForm(initialConfig);
  }, [initialConfig]);

  // 檢查路徑是否存在
  useEffect(() => {
    const checkPaths = async () => {
      const results: { [key: string]: boolean } = {};
      const pathsToCheck = [
        { key: 'gufo_path', path: form.gufo_path },
        { key: 'model_path', path: form.model_path },
        { key: 'mtp_path', path: form.mtp_path },
        { key: 'dspark_path', path: form.dspark_path },
        { key: 'vision_path', path: form.vision_path },
      ];

      for (const item of pathsToCheck) {
        if (item.path) {
          try {
            results[item.key] = await invoke<boolean>('check_path_exists', { path: item.path });
          } catch {
            results[item.key] = false;
          }
        } else {
          results[item.key] = false;
        }
      }
      setPathStatus(results);
    };

    checkPaths();
  }, [form.gufo_path, form.model_path, form.mtp_path, form.dspark_path, form.vision_path]);

  const handleChange = (field: keyof GufoServerConfig, value: any) => {
    setForm((prev) => ({ ...prev, [field]: value }));
  };

  // 系統偏好設定切換時立即自動儲存 (不受伺服器運行狀態限制)
  const handlePreferenceChange = (field: 'auto_open_gui' | 'auto_start_server', value: boolean) => {
    const updated = { ...form, [field]: value };
    setForm(updated);
    onSaveConfig(updated);
    setPrefSaved(true);
    setTimeout(() => setPrefSaved(false), 2000);
  };

  const handleSave = () => {
    onSaveConfig(form);
    setSavedSuccess(true);
    setTimeout(() => setSavedSuccess(false), 2500);
  };

  const showProfileMsg = (msg: string) => {
    setProfileNotice(msg);
    setTimeout(() => setProfileNotice(null), 5000);
  };

  const applyPreset = (preset: 'qwen3.8' | 'inkling2') => {
    if (preset === 'qwen3.8') {
      setForm((prev) => ({
        ...prev,
        served_model_name: 'flash-next',
        speculative: 'mtp',
        yarn_mode: 'auto',
        context: 131072,
        sessions: 2,
        think: 'off',
        temperature: 0,
      }));
      showProfileMsg('已套用 Qwen3.8-Flash-Next 建議參數，請確認下方模型路徑並點擊「儲存配置」。');
    } else {
      setForm((prev) => ({
        ...prev,
        served_model_name: 'inkling-2-mini',
        speculative: prev.dspark_path.trim() ? 'dspark' : 'off',
        yarn_mode: 'off',
        context: 65536,
        sessions: 2,
        think: 'auto',
        temperature: 0,
      }));
      showProfileMsg(
        '已套用 Inkling-2-Mini 建議參數（YaRN 關閉、支援最高 1M Context）。請於下方選取 Inkling-2-Mini 主模型、可選 mmproj 與 DSpark 模型後按「儲存配置」。'
      );
    }
  };

  const toProfileSnapshot = (id: string, name: string, cfg: GufoServerConfig): ModelProfile => ({
    id,
    name,
    model_path: cfg.model_path,
    mtp_path: cfg.mtp_path,
    dspark_path: cfg.dspark_path,
    vision_path: cfg.vision_path,
    served_model_name: cfg.served_model_name,
    context: cfg.context,
    sessions: cfg.sessions,
    think: cfg.think,
    temperature: cfg.temperature,
    max_tokens: cfg.max_tokens,
    speculative: cfg.speculative,
    yarn_mode: cfg.yarn_mode,
    extra_args: cfg.extra_args,
  });

  const handleSelectProfile = (profileId: string) => {
    if (!profileId) {
      setForm((prev) => ({ ...prev, active_profile_id: '' }));
      return;
    }
    const found = (form.model_profiles || []).find((p) => p.id === profileId);
    if (!found) return;
    setForm((prev) => ({
      ...prev,
      active_profile_id: found.id,
      model_path: found.model_path,
      mtp_path: found.mtp_path,
      dspark_path: found.dspark_path,
      vision_path: found.vision_path,
      served_model_name: found.served_model_name,
      context: found.context,
      sessions: found.sessions,
      think: found.think,
      temperature: found.temperature,
      max_tokens: found.max_tokens,
      speculative: found.speculative,
      yarn_mode: found.yarn_mode,
      extra_args: found.extra_args,
    }));
    showProfileMsg(`已載入設定檔「${found.name}」，確認無誤後請點擊「儲存配置」。`);
  };

  const handleCreateProfile = (e: React.FormEvent) => {
    e.preventDefault();
    const cleanName = newProfileName.trim();
    if (!cleanName) return;
    const id = `profile_${Date.now()}`;
    const snap = toProfileSnapshot(id, cleanName, form);
    const updated: GufoServerConfig = {
      ...form,
      model_profiles: [...(form.model_profiles || []), snap],
      active_profile_id: id,
    };
    setForm(updated);
    onSaveConfig(updated);
    setNewProfileName('');
    setShowSaveProfile(false);
    showProfileMsg(`已儲存新設定檔「${cleanName}」。`);
  };

  const handleUpdateActiveProfile = () => {
    const existing = (form.model_profiles || []).find((p) => p.id === form.active_profile_id);
    if (!existing) return;
    const snap = toProfileSnapshot(existing.id, existing.name, form);
    const updated: GufoServerConfig = {
      ...form,
      model_profiles: (form.model_profiles || []).map((p) => (p.id === existing.id ? snap : p)),
    };
    setForm(updated);
    onSaveConfig(updated);
    showProfileMsg(`已更新設定檔「${existing.name}」。`);
  };

  const handleDeleteActiveProfile = () => {
    const existing = (form.model_profiles || []).find((p) => p.id === form.active_profile_id);
    if (!existing) return;
    if (!confirm(`確定要刪除模型設定檔「${existing.name}」嗎？`)) return;
    const updated: GufoServerConfig = {
      ...form,
      model_profiles: (form.model_profiles || []).filter((p) => p.id !== existing.id),
      active_profile_id: '',
    };
    setForm(updated);
    onSaveConfig(updated);
    showProfileMsg(`已刪除設定檔「${existing.name}」。`);
  };

  const activeProfile = (form.model_profiles || []).find((p) => p.id === form.active_profile_id);

  return (
    <div className="config-container">
      {/* 頂部次導覽分頁：推論核心配置 vs 系統偏好設定 */}
      <div className="config-nav">
        <button
          type="button"
          className={`config-nav-btn ${subTab === 'inference' ? 'active' : ''}`}
          onClick={() => setSubTab('inference')}
        >
          <Cpu size={15} />
          <span>伺服器與推論設定 (Inference)</span>
        </button>
        <button
          type="button"
          className={`config-nav-btn ${subTab === 'preferences' ? 'active' : ''}`}
          onClick={() => setSubTab('preferences')}
        >
          <SlidersHorizontal size={15} />
          <span>系統啟動偏好設定 (Preferences)</span>
          {prefSaved && (
            <span
              style={{
                fontSize: 11,
                color: '#10b981',
                display: 'flex',
                alignItems: 'center',
                gap: 2,
                marginLeft: 4,
              }}
            >
              <Check size={12} /> 已即時保存
            </span>
          )}
        </button>
        <button
          type="button"
          className={`config-nav-btn ${subTab === 'web_search' ? 'active' : ''}`}
          onClick={() => setSubTab('web_search')}
        >
          <Globe size={15} />
          <span>網路搜尋 (WebSearch)</span>
        </button>
        <button
          type="button"
          className={`config-nav-btn ${subTab === 'api_guide' ? 'active' : ''}`}
          onClick={() => setSubTab('api_guide')}
        >
          <Terminal size={15} />
          <span>API 串接與效能速度指南 (API & Metrics)</span>
        </button>
      </div>

      {/* 分頁一：伺服器與推論設定 (Inference) */}
      {subTab === 'inference' && (
        <>
          {isRunning && (
            <div className="server-running-banner">
              <Info size={16} />
              <span>
                伺服器目前正在運行中 (Port {form.port})。若需變更推論引擎參數或更換模型，請先點擊右上角「停止伺服器」。
              </span>
            </div>
          )}

          {/* 模型設定檔與快速範本卡片 */}
          <div className="config-card">
            <div className="card-title">
              <Layers size={18} color="#38bdf8" />
              <span>模型設定檔與快速範本 (Profiles &amp; Presets)</span>
            </div>
            <div className="card-subtitle">
              可一鍵套用模型建議參數，或將目前模型路徑與推論設定儲存為設定檔，方便於多個模型之間快速切換。
            </div>

            <div className="profile-toolbar">
              <div style={{ display: 'flex', alignItems: 'center', gap: 8, flex: 1, minWidth: 240 }}>
                <select
                  className="form-input"
                  style={{ flex: 1 }}
                  value={form.active_profile_id || ''}
                  disabled={isRunning}
                  onChange={(e) => handleSelectProfile(e.target.value)}
                >
                  <option value="">-- 選擇已儲存的模型設定檔 ({(form.model_profiles || []).length}) --</option>
                  {(form.model_profiles || []).map((p) => (
                    <option key={p.id} value={p.id}>
                      {p.name} ({p.served_model_name || '未命名'})
                    </option>
                  ))}
                </select>

                {activeProfile && (
                  <>
                    <button
                      type="button"
                      className="btn btn-secondary"
                      disabled={isRunning}
                      onClick={handleUpdateActiveProfile}
                      title="以目前表單內容覆寫更新此設定檔"
                    >
                      <Save size={13} />
                      <span>更新</span>
                    </button>
                    <button
                      type="button"
                      className="btn btn-secondary"
                      style={{ color: '#fb7185' }}
                      disabled={isRunning}
                      onClick={handleDeleteActiveProfile}
                      title="刪除此設定檔"
                    >
                      <Trash2 size={13} />
                    </button>
                  </>
                )}
              </div>

              <button
                type="button"
                className="btn btn-secondary"
                disabled={isRunning}
                onClick={() => setShowSaveProfile((v) => !v)}
              >
                <Plus size={14} />
                <span>另存為新設定檔</span>
              </button>
            </div>

            {showSaveProfile && (
              <form
                onSubmit={handleCreateProfile}
                style={{ display: 'flex', gap: 8, marginTop: 10, alignItems: 'center' }}
              >
                <input
                  type="text"
                  className="form-input"
                  style={{ flex: 1 }}
                  placeholder="輸入設定檔名稱，例如：Inkling-2-Mini (64K + DSpark)"
                  value={newProfileName}
                  onChange={(e) => setNewProfileName(e.target.value)}
                  autoFocus
                  required
                />
                <button type="submit" className="btn btn-emerald" disabled={!newProfileName.trim()}>
                  確認儲存
                </button>
                <button
                  type="button"
                  className="btn btn-secondary"
                  onClick={() => setShowSaveProfile(false)}
                >
                  取消
                </button>
              </form>
            )}

            <div className="preset-chip-row">
              <span style={{ fontSize: 12, color: 'var(--text-muted)' }}>快速套用參數範本：</span>
              <button
                type="button"
                className="btn btn-secondary"
                style={{ padding: '4px 10px', fontSize: 12 }}
                disabled={isRunning}
                onClick={() => applyPreset('qwen3.8')}
              >
                套用 Qwen3.8-Flash-Next 建議參數
              </button>
              <button
                type="button"
                className="btn btn-secondary"
                style={{ padding: '4px 10px', fontSize: 12, borderColor: 'rgba(56, 189, 248, 0.45)', color: '#38bdf8' }}
                disabled={isRunning}
                onClick={() => applyPreset('inkling2')}
              >
                套用 Inkling-2-Mini 建議參數
              </button>
            </div>

            {profileNotice && (
              <div style={{ marginTop: 10, fontSize: 12, color: '#34d399' }}>{profileNotice}</div>
            )}
          </div>

          {/* 核心路徑配置卡片 */}
          <div className="config-card">
            <div className="card-title">
              <HardDrive size={18} color="#06b6d4" />
              <span>核心模型與執行檔路徑 (Model & Binary Paths)</span>
            </div>
            <div className="card-subtitle">
              設定 Gufo 引擎執行檔、主模型 GGUF、投機加速草稿模型與多模態視覺投影檔。
            </div>

            <div className="form-grid single-col">
              {/* Gufo 主程式路徑 */}
              <div className="form-group">
                <div className="form-label">
                  <span>Gufo 執行檔路徑 (gufo.exe)</span>
                  {pathStatus.gufo_path ? (
                    <span style={{ color: '#10b981', display: 'flex', alignItems: 'center', gap: 4 }}>
                      <CheckCircle2 size={13} /> 檔案存在
                    </span>
                  ) : (
                    <span style={{ color: '#f43f5e', display: 'flex', alignItems: 'center', gap: 4 }}>
                      <AlertCircle size={13} /> 檔案不存在
                    </span>
                  )}
                </div>
                <div className="path-input-group">
                  <input
                    type="text"
                    className="form-input"
                    value={form.gufo_path}
                    placeholder="工作目錄相對路徑或絕對路徑，例如: build\release\gufo.exe 或 gufo.exe"
                    disabled={isRunning}
                    onChange={(e) => handleChange('gufo_path', e.target.value)}
                  />
                  <button
                    type="button"
                    className="btn btn-secondary"
                    style={{ flexShrink: 0, padding: '7px 12px' }}
                    disabled={isRunning}
                    onClick={() =>
                      handleBrowse(
                        'gufo_path',
                        '選擇 Gufo 執行檔 (gufo.exe)',
                        'Gufo 執行檔 (*.exe)',
                        ['exe']
                      )
                    }
                    title="瀏覽選擇執行檔"
                  >
                    <FolderOpen size={14} />
                    <span>瀏覽...</span>
                  </button>
                </div>
              </div>

              {/* 模型主檔路徑 */}
              <div className="form-group">
                <div className="form-label">
                  <span>主模型檔案 (GGUF)</span>
                  {pathStatus.model_path ? (
                    <span style={{ color: '#10b981', display: 'flex', alignItems: 'center', gap: 4 }}>
                      <CheckCircle2 size={13} /> 檔案存在
                    </span>
                  ) : (
                    <span style={{ color: '#f43f5e', display: 'flex', alignItems: 'center', gap: 4 }}>
                      <AlertCircle size={13} /> 檔案不存在
                    </span>
                  )}
                </div>
                <div className="path-input-group">
                  <input
                    type="text"
                    className="form-input"
                    value={form.model_path}
                    placeholder="工作目錄相對路徑或絕對路徑，例如: models\Inkling-2-Mini-GGUF\Inkling-2-Mini-UD-Q4_K_M.gguf"
                    disabled={isRunning}
                    onChange={(e) => handleChange('model_path', e.target.value)}
                  />
                  <button
                    type="button"
                    className="btn btn-secondary"
                    style={{ flexShrink: 0, padding: '7px 12px' }}
                    disabled={isRunning}
                    onClick={() =>
                      handleBrowse(
                        'model_path',
                        '選擇主模型 (GGUF)',
                        'GGUF 模型檔 (*.gguf)',
                        ['gguf']
                      )
                    }
                    title="瀏覽選擇主模型 GGUF 檔案"
                  >
                    <FolderOpen size={14} />
                    <span>瀏覽...</span>
                  </button>
                </div>
              </div>

              {/* MTP 投機解碼模型 (僅當 speculative === 'mtp' 時顯示) */}
              {form.speculative === 'mtp' && (
                <div className="form-group">
                  <div className="form-label">
                    <span>MTP 投機解碼加速模型 (--mtp-model)</span>
                    {pathStatus.mtp_path ? (
                      <span style={{ color: '#10b981', display: 'flex', alignItems: 'center', gap: 4 }}>
                        <CheckCircle2 size={13} /> 檔案存在
                      </span>
                    ) : (
                      <span style={{ color: '#f43f5e', display: 'flex', alignItems: 'center', gap: 4 }}>
                        <AlertCircle size={13} /> 檔案不存在
                      </span>
                    )}
                  </div>
                  <div className="path-input-group">
                    <input
                      type="text"
                      className="form-input"
                      value={form.mtp_path}
                      placeholder="工作目錄相對路徑或絕對路徑，例如: models\Qwen3.8-Flash-Next-GGUF\mtp-*.gguf"
                      disabled={isRunning}
                      onChange={(e) => handleChange('mtp_path', e.target.value)}
                    />
                    <button
                      type="button"
                      className="btn btn-secondary"
                      style={{ flexShrink: 0, padding: '7px 12px' }}
                      disabled={isRunning}
                      onClick={() =>
                        handleBrowse(
                          'mtp_path',
                          '選擇 MTP 投機解碼加速模型 (GGUF)',
                          'GGUF 模型檔 (*.gguf)',
                          ['gguf']
                        )
                      }
                      title="瀏覽選擇 MTP 模型 GGUF 檔案"
                    >
                      <FolderOpen size={14} />
                      <span>瀏覽...</span>
                    </button>
                  </div>
                </div>
              )}

              {/* DSpark 投機解碼模型 (僅當 speculative === 'dspark' 時顯示) */}
              {form.speculative === 'dspark' && (
                <div className="form-group">
                  <div className="form-label">
                    <span>DSpark 投機解碼加速模型 (--dspark-model，Inkling-2-Mini 專用)</span>
                    {form.dspark_path.trim() ? (
                      pathStatus.dspark_path ? (
                        <span style={{ color: '#10b981', display: 'flex', alignItems: 'center', gap: 4 }}>
                          <CheckCircle2 size={13} /> 檔案存在
                        </span>
                      ) : (
                        <span style={{ color: '#f43f5e', display: 'flex', alignItems: 'center', gap: 4 }}>
                          <AlertCircle size={13} /> 檔案不存在
                        </span>
                      )
                    ) : (
                      <span style={{ color: '#94a3b8' }}>尚未指定檔案</span>
                    )}
                  </div>
                  <div className="path-input-group">
                    <input
                      type="text"
                      className="form-input"
                      value={form.dspark_path || ''}
                      placeholder="例如: models\Inkling-2-Mini-GGUF\Inkling-2-Mini-dspark-Q8_0.gguf"
                      disabled={isRunning}
                      onChange={(e) => handleChange('dspark_path', e.target.value)}
                    />
                    <button
                      type="button"
                      className="btn btn-secondary"
                      style={{ flexShrink: 0, padding: '7px 12px' }}
                      disabled={isRunning}
                      onClick={() =>
                        handleBrowse(
                          'dspark_path',
                          '選擇 DSpark 投機解碼加速模型 (GGUF)',
                          'GGUF 模型檔 (*.gguf)',
                          ['gguf']
                        )
                      }
                      title="瀏覽選擇 DSpark 模型 GGUF 檔案"
                    >
                      <FolderOpen size={14} />
                      <span>瀏覽...</span>
                    </button>
                  </div>
                </div>
              )}

              {form.speculative === 'off' && (
                <div style={hintStyle}>
                  投機解碼目前已設為關閉 (<code>off</code>)，無須設定 MTP 或 DSpark 草稿模型。若需啟用請至下方「投機解碼模式」切換。
                </div>
              )}

              {/* Vision 視覺側車模型 */}
              <div className="form-group">
                <div className="form-label">
                  <span>多模態視覺模型 (mmproj-*.gguf，選填)</span>
                  {form.vision_path.trim() ? (
                    pathStatus.vision_path ? (
                      <span style={{ color: '#10b981', display: 'flex', alignItems: 'center', gap: 4 }}>
                        <CheckCircle2 size={13} /> 檔案存在
                      </span>
                    ) : (
                      <span style={{ color: '#f43f5e', display: 'flex', alignItems: 'center', gap: 4 }}>
                        <AlertCircle size={13} /> 檔案不存在
                      </span>
                    )
                  ) : (
                    <span style={{ color: '#94a3b8' }}>留空（純文字模式）</span>
                  )}
                </div>
                <div className="path-input-group">
                  <input
                    type="text"
                    className="form-input"
                    value={form.vision_path}
                    placeholder="工作目錄相對路徑或絕對路徑，例如: models\Qwen3.8-Flash-Next-GGUF\mmproj-BF16.gguf"
                    disabled={isRunning}
                    onChange={(e) => handleChange('vision_path', e.target.value)}
                  />
                  <button
                    type="button"
                    className="btn btn-secondary"
                    style={{ flexShrink: 0, padding: '7px 12px' }}
                    disabled={isRunning}
                    onClick={() =>
                      handleBrowse(
                        'vision_path',
                        '選擇多模態視覺模型 (mmproj *.gguf)',
                        'GGUF 模型檔 (*.gguf)',
                        ['gguf']
                      )
                    }
                    title="瀏覽選擇視覺模型 GGUF 檔案"
                  >
                    <FolderOpen size={14} />
                    <span>瀏覽...</span>
                  </button>
                </div>
              </div>
            </div>
          </div>

          {/* 伺服器與推論超參數卡片 */}
          <div className="config-card">
            <div className="card-title">
              <Cpu size={18} color="#8b5cf6" />
              <span>網路與推論超參數 (Server & Inference Options)</span>
            </div>
            <div className="card-subtitle">
              管理 OpenAI API 連接埠、上下文長度、並發會話與投機策略。
            </div>

            <div className="form-grid">
              {/* 監聽位址 (Bind Host) */}
              <div className="form-group" style={{ gridColumn: 'span 2' }}>
                <div className="form-label">
                  <span style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                    <Globe size={14} color="#06b6d4" />
                    <span>監聽網路位址 (Bind Host)</span>
                  </span>
                  <span style={{ fontSize: 11, color: '#94a3b8' }}>
                    {form.host === '0.0.0.0' || form.host === '::' || !form.host.trim()
                      ? '監聽所有 IP（允許區域網路與本機連線）'
                      : form.host === '127.0.0.1' || form.host === '::1'
                      ? '僅限本機連線'
                      : `指定綁定 ${form.host}`}
                  </span>
                </div>
                <div style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
                  <input
                    type="text"
                    className="form-input"
                    style={{ flex: 1 }}
                    value={form.host}
                    placeholder="0.0.0.0 或 127.0.0.1"
                    disabled={isRunning}
                    onChange={(e) => handleChange('host', e.target.value)}
                  />
                  <button
                    type="button"
                    className={`btn ${form.host === '0.0.0.0' ? 'btn-primary' : 'btn-secondary'}`}
                    style={{ flexShrink: 0, padding: '7px 12px' }}
                    disabled={isRunning}
                    onClick={() => handleChange('host', '0.0.0.0')}
                    title="監聽所有 IPv4 網路介面"
                  >
                    0.0.0.0 (所有 IP)
                  </button>
                  <button
                    type="button"
                    className={`btn ${form.host === '127.0.0.1' ? 'btn-primary' : 'btn-secondary'}`}
                    style={{ flexShrink: 0, padding: '7px 12px' }}
                    disabled={isRunning}
                    onClick={() => handleChange('host', '127.0.0.1')}
                    title="僅允許本機連線"
                  >
                    127.0.0.1 (僅本機)
                  </button>
                </div>
                <div style={hintStyle}>
                  預設為 <code>0.0.0.0</code> 以監聽所有本機與區域網路 IP（需注意 Windows 防火牆放行，對外建議設定下方 API Key）。
                </div>
              </div>

              {/* OpenAI API Port */}
              <div className="form-group">
                <div className="form-label">
                  <span>OpenAI API HTTP 埠號 (Port)</span>
                </div>
                <input
                  type="number"
                  className="form-input"
                  value={form.port}
                  disabled={isRunning}
                  onChange={(e) => handleChange('port', clampInt(e.target.value, 1, 65535))}
                />
              </div>

              {/* 服務識別名稱 */}
              <div className="form-group">
                <div className="form-label">
                  <span>模型服務識別名稱 (Served Model Name)</span>
                </div>
                <input
                  type="text"
                  className="form-input"
                  value={form.served_model_name}
                  disabled={isRunning}
                  onChange={(e) => handleChange('served_model_name', e.target.value)}
                />
              </div>

              {/* Context 長度 */}
              <div className="form-group">
                <div className="form-label">
                  <span>上下文視窗長度 (Context Tokens)</span>
                </div>
                <select
                  className="form-input"
                  value={form.context}
                  disabled={isRunning}
                  onChange={(e) => handleChange('context', parseInt(e.target.value))}
                >
                  <option value={32768}>32,768 (32K)</option>
                  <option value={65536}>65,536 (64K)</option>
                  <option value={131072}>131,072 (128K 推薦)</option>
                  <option value={262144}>262,144 (256K Qwen3.8 原生上限)</option>
                  <option value={409600}>409,600 (400K)</option>
                  <option value={524288}>524,288 (512K)</option>
                  <option value={1048576}>1,048,576 (1M - Inkling-2-Mini 原生上限)</option>
                  {!STANDARD_CONTEXT_OPTIONS.includes(form.context) && (
                    <option value={form.context}>{form.context.toLocaleString()} (自訂)</option>
                  )}
                </select>
                <div style={hintStyle}>
                  若使用原生 1M Context 模型（如 Inkling-2-Mini），請確認下方 YaRN 模式設為 <code>off</code>。
                </div>
              </div>

              {/* YaRN 位置編碼擴展模式 */}
              <div className="form-group">
                <div className="form-label">
                  <span>YaRN 位置編碼擴展 (--enable-yarn)</span>
                </div>
                <select
                  className="form-input"
                  value={form.yarn_mode || 'auto'}
                  disabled={isRunning}
                  onChange={(e) => handleChange('yarn_mode', e.target.value as YarnMode)}
                >
                  <option value="auto">auto (超過 256K 自動附加 — Qwen3.8 適用)</option>
                  <option value="off">off (不附加 — Inkling-2-Mini 原生 1M 適用)</option>
                  <option value="on">on (強制一律附加 --enable-yarn)</option>
                </select>
                <div style={hintStyle}>
                  控制啟動時是否傳遞 <code>--enable-yarn</code> 旗標。
                </div>
              </div>

              {/* GPU 並發 Sessions */}
              <div className="form-group">
                <div className="form-label">
                  <span>GPU 並發會話數 (Sessions)</span>
                </div>
                <select
                  className="form-input"
                  value={form.sessions}
                  disabled={isRunning}
                  onChange={(e) => handleChange('sessions', parseInt(e.target.value))}
                >
                  <option value={1}>1 (單會話，顯存最省)</option>
                  <option value={2}>2 (雙路並發，推薦)</option>
                  <option value={4}>4 (多路並發)</option>
                </select>
                <div style={hintStyle}>
                  目前 context × sessions = {(form.context * form.sessions).toLocaleString()} tokens。官方 <code>v0.35.0+</code> 支援按需映射 (Demand-mapped) KV 狀態與動態會話共享；若硬體可用記憶體小於 96GB，過大的配置仍可能影響載入與並發排隊。
                </div>
              </div>

              {/* 投機解碼模式 */}
              <div className="form-group">
                <div className="form-label">
                  <span>投機解碼模式 (--speculative)</span>
                </div>
                <select
                  className="form-input"
                  value={
                    form.speculative === 'dspark'
                      ? 'dspark'
                      : form.speculative === 'off'
                      ? 'off'
                      : 'mtp'
                  }
                  disabled={isRunning}
                  onChange={(e) => handleChange('speculative', e.target.value as SpeculativeMode)}
                >
                  <option value="mtp">mtp (MTP 投機加速 — Qwen3.8 適用)</option>
                  <option value="dspark">dspark (DSpark 投機加速 — Inkling-2-Mini 適用)</option>
                  <option value="off">off (關閉投機，僅使用單步自回歸解碼)</option>
                </select>
                <div style={hintStyle}>
                  切換後上方會顯示對應的草稿模型路徑設定欄位（<code>--mtp-model</code> 或 <code>--dspark-model</code>）。
                </div>
              </div>

              {/* 預設思考模式 */}
              <div className="form-group">
                <div className="form-label">
                  <span>預設思考鏈狀態 (Think Mode)</span>
                </div>
                <select
                  className="form-input"
                  value={form.think}
                  disabled={isRunning}
                  onChange={(e) => handleChange('think', e.target.value)}
                >
                  <option value="off">off (關閉思考，延遲最低)</option>
                  <option value="on">on (開啟深度推導思考)</option>
                  <option value="auto">auto (自動遵循模型提示)</option>
                </select>
                <div style={hintStyle}>
                  僅為伺服器預設值；聊天頁的思考鏈開關會以 <code>chat_template_kwargs.enable_thinking</code> 逐請求覆寫。
                </div>
              </div>

              {/* 預設採樣溫度 */}
              <div className="form-group">
                <div className="form-label">
                  <span>預設採樣溫度 (Temperature)</span>
                </div>
                <input
                  type="number"
                  className="form-input"
                  min={0}
                  max={2}
                  step={0.05}
                  value={form.temperature}
                  disabled={isRunning}
                  onChange={(e) => handleChange('temperature', clampNumber(e.target.value, 0, 2, 0))}
                />
                <div style={hintStyle}>0 = 貪婪解碼 (MTP 接受率最高、速度最快)；請求可自行覆寫。</div>
              </div>

              {/* 預設最大生成長度 */}
              <div className="form-group">
                <div className="form-label">
                  <span>預設最大生成 Tokens (--max-tokens)</span>
                </div>
                <input
                  type="number"
                  className="form-input"
                  min={0}
                  step={1024}
                  value={form.max_tokens}
                  disabled={isRunning}
                  onChange={(e) => handleChange('max_tokens', clampInt(e.target.value, 0))}
                />
                <div style={hintStyle}>請求未帶 max_tokens 時的上限；0 = 交給 Gufo 預設 (直到 EOS 或 context 用盡)。</div>
              </div>

              {/* 磁碟 Prompt 快取 */}
              <div className="form-group" style={{ gridColumn: 'span 2' }}>
                <div className="form-label">
                  <span>磁碟 Prompt 快取 (--cache-disk)</span>
                </div>
                <label style={checkboxLabelStyle}>
                  <input
                    type="checkbox"
                    checked={form.cache_disk}
                    disabled={isRunning}
                    onChange={(e) => handleChange('cache_disk', e.target.checked)}
                  />
                  <span>將 prompt KV 快取寫入磁碟，重啟後或長 system prompt 可跳過重新 prefill</span>
                </label>
                {form.cache_disk && (
                  <div style={{ display: 'flex', gap: 8, marginTop: 6 }}>
                    <input
                      type="text"
                      className="form-input"
                      style={{ flex: 2 }}
                      placeholder="快取目錄 (相對路徑以 Gufo Studio 執行目錄為準)"
                      value={form.cache_disk_dir}
                      disabled={isRunning}
                      onChange={(e) => handleChange('cache_disk_dir', e.target.value)}
                    />
                    <input
                      type="number"
                      className="form-input"
                      style={{ flex: 1 }}
                      min={0}
                      step={1}
                      title="容量上限 (GiB)，0 = Gufo 預設"
                      placeholder="容量上限 GiB (0 = 預設)"
                      value={form.cache_disk_bytes ? round2(form.cache_disk_bytes / GIB) : 0}
                      disabled={isRunning}
                      onChange={(e) =>
                        handleChange('cache_disk_bytes', Math.round(clampNumber(e.target.value, 0, 4096, 0) * GIB))
                      }
                    />
                  </div>
                )}
              </div>

              {/* API Key 安全保護 */}
              <div className="form-group" style={{ gridColumn: 'span 2' }}>
                <div className="form-label">
                  <span style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                    <Shield size={14} color="#10b981" />
                    <span>API Key 金鑰保護 (Authorization Bearer Token)</span>
                  </span>
                  <span style={{ fontSize: 11, color: '#94a3b8' }}>
                    {form.api_key ? '已啟用金鑰驗證' : '留空表示無金鑰限制'}
                  </span>
                </div>
                <div style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
                  <div style={{ position: 'relative', flex: 1 }}>
                    <input
                      type={showApiKey ? 'text' : 'password'}
                      className="form-input"
                      style={{ width: '100%', paddingRight: 36 }}
                      placeholder="可選，例如 sk-gufo-xxx (留空表示不限制存取)"
                      value={form.api_key || ''}
                      disabled={isRunning}
                      onChange={(e) => handleChange('api_key', e.target.value)}
                    />
                    <button
                      type="button"
                      style={{
                        position: 'absolute',
                        right: 8,
                        top: '50%',
                        transform: 'translateY(-50%)',
                        background: 'transparent',
                        border: 'none',
                        color: '#94a3b8',
                        cursor: 'pointer',
                      }}
                      onClick={() => setShowApiKey(!showApiKey)}
                      title={showApiKey ? '隱藏金鑰' : '顯示金鑰'}
                    >
                      {showApiKey ? <EyeOff size={15} /> : <Eye size={15} />}
                    </button>
                  </div>
                  <button
                    type="button"
                    className="btn btn-secondary"
                    disabled={isRunning}
                    onClick={generateApiKey}
                    title="隨機生成高強度金鑰"
                  >
                    <Sparkles size={13} color="#06b6d4" />
                    <span>隨機生成</span>
                  </button>
                  {form.api_key && (
                    <button
                      type="button"
                      className="btn btn-secondary"
                      disabled={isRunning}
                      onClick={() => handleChange('api_key', '')}
                      title="清除金鑰"
                    >
                      <span>清除</span>
                    </button>
                  )}
                </div>
                <div style={{ fontSize: 11.5, color: '#64748b', marginTop: 2 }}>
                  官方 <code>gufo-org/gufo</code> 已移除原生的 <code>--api-key</code> 參數，改由前置反向代理負責認證。設定 API Key 後，Gufo Studio 內建的 <strong>HTTPS (TLS) 反向代理</strong>（Port <code>{form.https_port}</code>）會強制驗證 <code>Authorization: Bearer &lt;KEY&gt;</code> 標頭；底層 HTTP 端點（<code>{status.endpoint}</code>）維持直連不檢查金鑰。
                  {form.api_key && !form.https_enabled && (
                    <span style={{ color: '#f59e0b', display: 'block', marginTop: 4 }}>
                      ⚠️ 目前未啟用內建 HTTPS 代理，API Key 驗證暫時不會生效。
                    </span>
                  )}
                </div>
              </div>

              {/* HTTPS 安全連線設定 */}
              <div className="form-group" style={{ gridColumn: 'span 2' }}>
                <div className="form-label">
                  <span style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                    <Lock size={14} color="#38bdf8" />
                    <span>HTTPS 安全連線設定 (內建 TLS 反向代理)</span>
                  </span>
                  <span style={{ fontSize: 11, color: form.https_enabled ? '#10b981' : '#94a3b8' }}>
                    {form.https_enabled ? `已啟用 (Port ${form.https_port})` : '未啟用'}
                  </span>
                </div>
                <label style={checkboxLabelStyle}>
                  <input
                    type="checkbox"
                    checked={form.https_enabled}
                    disabled={isRunning}
                    onChange={(e) => handleChange('https_enabled', e.target.checked)}
                  />
                  <span>加開 HTTPS (TLS) 端點，將加密流量轉發至本地 Gufo 引擎（與 HTTP 同時運作）</span>
                </label>
                {form.https_enabled && (
                  <div style={{ display: 'flex', flexDirection: 'column', gap: 8, marginTop: 8 }}>
                    <div style={{ display: 'grid', gridTemplateColumns: '180px 1fr', gap: 8, alignItems: 'center' }}>
                      <div>
                        <div style={{ fontSize: 11.5, color: '#94a3b8', marginBottom: 4 }}>HTTPS 埠號</div>
                        <input
                          type="number"
                          className="form-input"
                          value={form.https_port}
                          min={1}
                          max={65535}
                          disabled={isRunning}
                          onChange={(e) => handleChange('https_port', clampInt(e.target.value, 1, 65535))}
                        />
                      </div>
                      <div>
                        <div style={{ fontSize: 11.5, color: '#94a3b8', marginBottom: 4 }}>
                          自訂 TLS 憑證 PEM（選填，留空將自動產生自簽憑證）
                        </div>
                        <div className="path-input-group">
                          <input
                            type="text"
                            className="form-input"
                            placeholder="留空 = 自動產生 gufo-studio.cert.pem"
                            value={form.https_cert_path || ''}
                            disabled={isRunning}
                            onChange={(e) => handleChange('https_cert_path', e.target.value)}
                          />
                          <button
                            type="button"
                            className="btn btn-secondary"
                            style={{ flexShrink: 0, padding: '7px 12px' }}
                            disabled={isRunning}
                            onClick={() =>
                              handleBrowse('https_cert_path', '選擇 TLS 憑證檔 (PEM / CRT)', '憑證檔案 (*.pem;*.crt;*.cer)', [
                                'pem',
                                'crt',
                                'cer',
                              ])
                            }
                          >
                            <FolderOpen size={14} />
                            <span>瀏覽...</span>
                          </button>
                        </div>
                      </div>
                    </div>

                    <div>
                      <div style={{ fontSize: 11.5, color: '#94a3b8', marginBottom: 4 }}>
                        自訂 TLS 私鑰 PEM（選填，需與上方憑證成對提供）
                      </div>
                      <div className="path-input-group">
                        <input
                          type="text"
                          className="form-input"
                          placeholder="留空 = 自動產生 gufo-studio.key.pem"
                          value={form.https_key_path || ''}
                          disabled={isRunning}
                          onChange={(e) => handleChange('https_key_path', e.target.value)}
                        />
                        <button
                          type="button"
                          className="btn btn-secondary"
                          style={{ flexShrink: 0, padding: '7px 12px' }}
                          disabled={isRunning}
                          onClick={() =>
                            handleBrowse('https_key_path', '選擇 TLS 私鑰檔 (PEM / KEY)', '私鑰檔案 (*.pem;*.key)', [
                              'pem',
                              'key',
                            ])
                          }
                        >
                          <FolderOpen size={14} />
                          <span>瀏覽...</span>
                        </button>
                      </div>
                    </div>

                    {!form.https_cert_path.trim() && !form.https_key_path.trim() && (
                      <>
                        <div>
                          <div style={{ fontSize: 11.5, color: '#94a3b8', marginBottom: 4 }}>
                            額外憑證 SAN（IP / 網域，選填、逗號分隔）
                          </div>
                          <input
                            type="text"
                            className="form-input"
                            placeholder="例如: 192.168.122.122, gufo.local（留空仍會自動掃描本機介面 IP）"
                            value={form.https_sans || ''}
                            disabled={isRunning}
                            onChange={(e) => handleChange('https_sans', e.target.value)}
                          />
                        </div>
                        <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 8, flexWrap: 'wrap' }}>
                          <div style={hintStyle}>
                            未指定自訂憑證時，會自動建立包含 <code>localhost</code>、<code>127.0.0.1</code>、主機名稱、本機所有網路介面 IP 與上方 SAN 的自簽憑證
                            {status.https_cert_path ? <>（目前憑證：<code>{status.https_cert_path}</code>）</> : ''}。
                          </div>
                          <button
                            type="button"
                            className="btn btn-secondary"
                            style={{ fontSize: 12, padding: '5px 10px' }}
                            disabled={isRunning}
                            onClick={handleRegenerateCert}
                            title="依目前設定與本機介面重新產生自簽憑證"
                          >
                            <RefreshCw size={12} />
                            <span>重新產生自簽憑證</span>
                          </button>
                        </div>
                      </>
                    )}
                    {certRegenMsg && (
                      <div style={{ fontSize: 12, color: '#10b981' }}>{certRegenMsg}</div>
                    )}
                  </div>
                )}
              </div>

              {/* 額外 CLI 啟動參數 */}
              <div className="form-group" style={{ gridColumn: 'span 2' }}>
                <div className="form-label">
                  <span>額外 CLI 啟動參數 (Extra Arguments，選填)</span>
                </div>
                <input
                  type="text"
                  className="form-input"
                  placeholder="例如: --extension extensions\inkling2.dll（支援以雙引號包住含空白的路徑）"
                  value={form.extra_args || ''}
                  disabled={isRunning}
                  onChange={(e) => handleChange('extra_args', e.target.value)}
                />
                <div style={hintStyle}>
                  會直接附加於啟動指令尾端，適合傳遞社群擴充套件或引擎實驗旗標。
                </div>
              </div>

              {/* 即時啟動指令預覽 */}
              <div className="form-group" style={{ gridColumn: 'span 2' }}>
                <div className="form-label">
                  <span>啟動指令預覽 (Command Preview)</span>
                </div>
                <div className="cli-preview-box">{buildCliPreview(form)}</div>
              </div>
            </div>

            {/* 底部按鈕區 */}
            <div style={{ marginTop: 24, display: 'flex', justifyContent: 'flex-end', gap: 12 }}>
              <button className="btn btn-secondary" onClick={onResetDefaults} disabled={isRunning}>
                <RotateCcw size={14} />
                <span>還原預設配置</span>
              </button>
              <button className="btn btn-primary" onClick={handleSave} disabled={isRunning}>
                <Save size={14} />
                <span>{savedSuccess ? '已儲存！' : '儲存配置'}</span>
              </button>
            </div>
          </div>
        </>
      )}

      {/* 分頁二：系統啟動偏好設定 (Preferences) - 永遠開放調整，不受 isRunning 限制 */}
      {subTab === 'preferences' && (
        <>
          <div className="config-card">
            <div className="card-title">
              <SlidersHorizontal size={18} color="#10b981" />
              <span>系統啟動偏好設定 (Startup Preferences)</span>
              {prefSaved && (
                <span style={{ fontSize: 12, color: '#10b981', fontWeight: 500, marginLeft: 'auto', display: 'flex', alignItems: 'center', gap: 4 }}>
                  <Check size={14} /> 已自動儲存偏好
                </span>
              )}
            </div>
            <div className="card-subtitle">
              這些偏好設定隨時皆可直接切換（開關變更時立即自動儲存生效，無須停止伺服器）。
            </div>

            <div className="form-grid single-col" style={{ gap: 14 }}>
              {/* 啟動後是否自動開啟 GUI */}
              <div
                style={{
                  display: 'flex',
                  alignItems: 'center',
                  justifyContent: 'space-between',
                  padding: '14px 18px',
                  backgroundColor: 'var(--bg-primary)',
                  borderRadius: 'var(--radius-sm)',
                  border: '1px solid var(--border-color)',
                }}
              >
                <div>
                  <div style={{ fontWeight: 600, fontSize: 13.5, color: 'var(--text-primary)' }}>
                    啟動後自動顯示主視窗 (Open GUI on Startup)
                  </div>
                  <div style={{ fontSize: 12, color: 'var(--text-muted)', marginTop: 4 }}>
                    預設開啟。若關閉，應用程式啟動時將自動縮小隱藏於系統匣 (Tray)，點擊右下角系統匣圖示即可隨時喚出主視窗。
                  </div>
                </div>
                <input
                  type="checkbox"
                  style={{ width: 20, height: 20, cursor: 'pointer', accentColor: '#10b981', marginLeft: 16 }}
                  checked={form.auto_open_gui}
                  onChange={(e) => handlePreferenceChange('auto_open_gui', e.target.checked)}
                />
              </div>

              {/* 啟動後是否自動啟動伺服器 */}
              <div
                style={{
                  display: 'flex',
                  alignItems: 'center',
                  justifyContent: 'space-between',
                  padding: '14px 18px',
                  backgroundColor: 'var(--bg-primary)',
                  borderRadius: 'var(--radius-sm)',
                  border: '1px solid var(--border-color)',
                }}
              >
                <div>
                  <div style={{ fontWeight: 600, fontSize: 13.5, color: 'var(--text-primary)' }}>
                    啟動後自動開啟推論伺服器 (Auto-start Server on Startup)
                  </div>
                  <div style={{ fontSize: 12, color: 'var(--text-muted)', marginTop: 4 }}>
                    應用程式啟動完成後，立即自動加載指定模型並在背景運行 Gufo 推理引擎 (預設 Port {form.port})。
                  </div>
                </div>
                <input
                  type="checkbox"
                  style={{ width: 20, height: 20, cursor: 'pointer', accentColor: '#10b981', marginLeft: 16 }}
                  checked={form.auto_start_server}
                  onChange={(e) => handlePreferenceChange('auto_start_server', e.target.checked)}
                />
              </div>
            </div>
          </div>

          {/* 視窗狀態與環境資訊卡片 */}
          <div className="config-card">
            <div className="card-title">
              <Laptop size={18} color="#38bdf8" />
              <span>視窗位置與便攜環境 (Window & Environment)</span>
            </div>
            <div className="card-subtitle">
              Gufo Studio 已啟用視窗狀態智慧記憶與免安裝便攜架構。
            </div>

            <div style={{ display: 'flex', flexDirection: 'column', gap: 10, fontSize: 13, color: 'var(--text-secondary)' }}>
              <div style={{ display: 'flex', alignItems: 'flex-start', gap: 8 }}>
                <CheckCircle2 size={16} color="#10b981" style={{ flexShrink: 0, marginTop: 2 }} />
                <div>
                  <strong>多螢幕智慧還原</strong>：系統會自動記住上次關閉或隱藏至系統匣時的視窗座標與大小；若偵測到外接螢幕拔除或座標超出顯示範圍，會自動安全回退至第一個螢幕（主螢幕）中央。
                </div>
              </div>
              <div style={{ display: 'flex', alignItems: 'flex-start', gap: 8 }}>
                <CheckCircle2 size={16} color="#10b981" style={{ flexShrink: 0, marginTop: 2 }} />
                <div>
                  <strong>綠色便攜設計</strong>：所有配置檔案（<code>gufo-studio.config.json</code> 與 <code>gufo-studio.window.json</code>）皆存放於執行檔同層工作目錄，複製資料夾即可隨身攜帶使用。
                </div>
              </div>
            </div>
          </div>
        </>
      )}

      {/* 分頁三：網路搜尋設定 (WebSearch) */}
      {subTab === 'web_search' && (
        <WebSearchSettingsView
          config={webSearchConfig}
          onSaveConfig={onSaveWebSearchConfig}
        />
      )}

      {/* 分頁四：API 串接與效能速度指南 (API & Metrics) */}
      {subTab === 'api_guide' && (() => {
        const cleanHost = (form.host || '').trim().replace(/^\[|\]$/g, '');
        const isWildcard = cleanHost === '' || cleanHost === '0.0.0.0' || cleanHost === '::';
        const clientHost = cleanHost === '' || cleanHost === '0.0.0.0' ? '127.0.0.1' : cleanHost === '::' ? '[::1]' : cleanHost.includes(':') ? `[${cleanHost}]` : cleanHost;
        const firstCustomSan = (form.https_sans || '').split(/[\s,;]+/).map((s) => s.trim()).find((s) => s && s !== '0.0.0.0' && s !== '::');
        const effectiveLan = firstCustomSan || status.lan_ip;
        const localHttpUrl = `http://${clientHost}:${form.port}/v1`;
        const localHttpsUrl = form.https_enabled ? `https://${clientHost}:${form.https_port}/v1` : null;
        const lanHttpUrl = isWildcard && effectiveLan ? `http://${effectiveLan}:${form.port}/v1` : null;
        const lanHttpsUrl = isWildcard && form.https_enabled && effectiveLan ? `https://${effectiveLan}:${form.https_port}/v1` : null;
        return (
        <>
          <div className="config-card">
            <div className="card-title">
              <Code size={18} color="#38bdf8" />
              <span>OpenAI 相容端點資訊 (API Endpoints)</span>
            </div>
            <div className="card-subtitle">
              Gufo 提供 100% 相容 OpenAI Chat Completions 標準規格的推論 API，目前綁定於 <code>{cleanHost || '0.0.0.0'}</code>。
            </div>

            <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fit, minmax(260px, 1fr))', gap: 12, marginTop: 8 }}>
              <div style={{ background: 'var(--bg-primary)', padding: '12px 14px', borderRadius: 8, border: '1px solid var(--border-color)' }}>
                <div style={{ fontSize: 11, color: 'var(--text-muted)' }}>本機 HTTP Base URL</div>
                <div style={{ fontFamily: 'var(--font-mono)', fontWeight: 600, color: 'var(--accent-cyan)', marginTop: 4 }}>
                  {localHttpUrl}
                </div>
                {lanHttpUrl && (
                  <div style={{ fontSize: 11.5, color: 'var(--text-secondary)', marginTop: 4, fontFamily: 'var(--font-mono)' }}>
                    LAN: {lanHttpUrl}
                  </div>
                )}
              </div>

              {localHttpsUrl && (
                <div style={{ background: 'var(--bg-primary)', padding: '12px 14px', borderRadius: 8, border: '1px solid var(--border-color)' }}>
                  <div style={{ fontSize: 11, color: 'var(--text-muted)' }}>HTTPS Base URL (內建 TLS 代理)</div>
                  <div style={{ fontFamily: 'var(--font-mono)', fontWeight: 600, color: '#38bdf8', marginTop: 4 }}>
                    {localHttpsUrl}
                  </div>
                  {lanHttpsUrl && (
                    <div style={{ fontSize: 11.5, color: 'var(--text-secondary)', marginTop: 4, fontFamily: 'var(--font-mono)' }}>
                      LAN: {lanHttpsUrl}
                    </div>
                  )}
                </div>
              )}

              <div style={{ background: 'var(--bg-primary)', padding: '12px 14px', borderRadius: 8, border: '1px solid var(--border-color)' }}>
                <div style={{ fontSize: 11, color: 'var(--text-muted)' }}>模型名稱 (Model Identifier)</div>
                <div style={{ fontFamily: 'var(--font-mono)', fontWeight: 600, color: 'var(--accent-emerald)', marginTop: 4 }}>
                  {form.served_model_name}
                </div>
              </div>

              <div style={{ background: 'var(--bg-primary)', padding: '12px 14px', borderRadius: 8, border: '1px solid var(--border-color)' }}>
                <div style={{ fontSize: 11, color: 'var(--text-muted)' }}>API Key 驗證</div>
                <div style={{ fontFamily: 'var(--font-mono)', fontWeight: 600, color: form.api_key ? 'var(--accent-amber)' : 'var(--text-muted)', marginTop: 4 }}>
                  {form.api_key ? 'Bearer ' + form.api_key.substring(0, 10) + '...' : '未啟用 (免金鑰開放連線)'}
                </div>
              </div>
            </div>
            {form.https_enabled && !form.https_cert_path.trim() && (
              <div style={{ ...hintStyle, marginTop: 10 }}>
                使用自動產生的自簽憑證時，於 <code>curl</code> 可加上 <code>--cacert gufo-studio.cert.pem</code>（或測試用 <code>-k</code>），Python <code>requests</code> 可傳入 <code>verify=&quot;{status.https_cert_path || 'gufo-studio.cert.pem'}&quot;</code>。
              </div>
            )}
          </div>

          {/* 方案 A：如何從 API 獲取推論速度與 Prefill 吞吐量 */}
          <div className="config-card">
            <div className="card-title">
              <Zap size={18} color="#f59e0b" />
              <span>如何在程式與 API 回應中讀取 token/s 與 Prefill 速度 (方案 A)</span>
            </div>
            <div className="card-subtitle">
              底層 Gufo 引擎在每次推論完成時，已於 JSON 回應中原生附帶了完整的時間與吞吐量指標。
            </div>

            <div style={{ display: 'flex', flexDirection: 'column', gap: 16, marginTop: 8 }}>
              {/* 方法一：非串流 (JSON) */}
              <div style={{ background: 'var(--bg-primary)', padding: 14, borderRadius: 8, border: '1px solid var(--border-color)' }}>
                <div style={{ fontWeight: 600, color: 'var(--text-primary)', marginBottom: 6 }}>
                  1. 非串流呼叫 (Non-streaming JSON)
                </div>
                <p style={{ fontSize: 13, color: 'var(--text-secondary)', marginBottom: 8 }}>
                  直接從回傳 JSON 的 <code>response["timings"]</code> 與 <code>response["usage"]</code> 取得精準的吞吐量數據：
                </p>
                <pre style={{ background: 'var(--bg-secondary)', padding: '10px 12px', borderRadius: 6, fontSize: 12, fontFamily: 'var(--font-mono)', color: '#38bdf8', overflowX: 'auto' }}>
{`import requests

url = "${localHttpUrl}/chat/completions"
${localHttpsUrl ? `# 或使用 HTTPS 端點: "${localHttpsUrl}/chat/completions"\n` : ''}headers = {"Content-Type": "application/json"}
${form.api_key ? `headers["Authorization"] = "Bearer ${form.api_key}"\n` : ''}
payload = {
    "model": "${form.served_model_name}",
    "messages": [{"role": "user", "content": "請分析這份數據"}],
    "temperature": 0.0
}

data = requests.post(url, json=payload, headers=headers).json()

# 直接讀取底層 Gufo 原生計算的吞吐度量：
prefill_tps = data["timings"]["prompt_per_second"]       # Prefill 處理速度 (tok/s)
decode_tps  = data["timings"]["predicted_per_second"]    # 生成解碼速度 (tok/s)
prompt_ms   = data["timings"]["prompt_ms"]               # Prefill 耗時 (ms)
decode_ms   = data["timings"]["predicted_ms"]            # 生成耗時 (ms)

print(f"🚀 Prefill: {prefill_tps:.1f} tok/s | ⚡ Decode: {decode_tps:.1f} tok/s")`}
                </pre>
              </div>

              {/* 方法二：串流模式 (Streaming SSE) */}
              <div style={{ background: 'var(--bg-primary)', padding: 14, borderRadius: 8, border: '1px solid var(--border-color)' }}>
                <div style={{ fontWeight: 600, color: 'var(--text-primary)', marginBottom: 6 }}>
                  2. 串流呼叫 (Streaming SSE)
                </div>
                <p style={{ fontSize: 13, color: 'var(--text-secondary)', marginBottom: 8 }}>
                  在請求體加入 <code>{'stream_options: {"include_usage": true}'}</code>，或直接讀取最後一個 choice chunk 的 <code>timings</code>：
                </p>
                <pre style={{ background: 'var(--bg-secondary)', padding: '10px 12px', borderRadius: 6, fontSize: 12, fontFamily: 'var(--font-mono)', color: '#a7f3d0', overflowX: 'auto' }}>
{`payload = {
    "model": "${form.served_model_name}",
    "messages": [{"role": "user", "content": "寫一篇短文"}],
    "stream": True,
    "stream_options": {"include_usage": True} # 開啟使用量與計時 Chunk
}

with requests.post(url, json=payload, headers=headers, stream=True) as resp:
    for line in resp.iter_lines():
        if line.startswith(b"data: ") and not line.endswith(b"[DONE]"):
            chunk = json.loads(line[6:])
            # 串流結束時的終端 Chunk 必定自帶 timings 物件：
            if "timings" in chunk:
                timings = chunk["timings"]
                print(f"\\nPrefill: {timings['prompt_per_second']:.1f} tok/s")
                print(f"Decode:  {timings['predicted_per_second']:.1f} tok/s")`}
                </pre>
              </div>

              {/* 方法三：HTTP Header 首字延遲 */}
              <div style={{ background: 'var(--bg-primary)', padding: 14, borderRadius: 8, border: '1px solid var(--border-color)' }}>
                <div style={{ fontWeight: 600, color: 'var(--text-primary)', marginBottom: 6 }}>
                  3. HTTP 回應標頭首字延遲 (Server-Timing Header)
                </div>
                <p style={{ fontSize: 13, color: 'var(--text-secondary)' }}>
                  Gufo 每次回應時均附帶標準 <code>Server-Timing</code> 標頭：
                  <code style={{ display: 'block', background: 'var(--bg-secondary)', padding: '6px 10px', borderRadius: 4, marginTop: 6, color: '#fde047' }}>
                    Server-Timing: ttft;dur=10.500, inter_token;dur=31.250, max_inter_token;dur=45.100
                  </code>
                  可直接讀取 <code>ttft</code>（Time To First Token 首字延遲，單位為毫秒）。
                </p>
              </div>
            </div>
          </div>
        </>
        );
      })()}
    </div>
  );
};
