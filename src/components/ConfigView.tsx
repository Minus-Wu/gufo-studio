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
} from 'lucide-react';
import { GufoServerConfig } from '../types';

interface ConfigViewProps {
  config: GufoServerConfig;
  onSaveConfig: (newConfig: GufoServerConfig) => void;
  onResetDefaults: () => void;
  isRunning: boolean;
}

export const ConfigView: React.FC<ConfigViewProps> = ({
  config: initialConfig,
  onSaveConfig,
  onResetDefaults,
  isRunning,
}) => {
  const [subTab, setSubTab] = useState<'inference' | 'preferences' | 'api_guide'>('inference');
  const [form, setForm] = useState<GufoServerConfig>(initialConfig);
  const [pathStatus, setPathStatus] = useState<{ [key: string]: boolean }>({});
  const [savedSuccess, setSavedSuccess] = useState(false);
  const [prefSaved, setPrefSaved] = useState(false);
  const [showApiKey, setShowApiKey] = useState(false);

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
  }, [form.gufo_path, form.model_path, form.mtp_path, form.vision_path]);

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

          {/* 核心路徑配置卡片 */}
          <div className="config-card">
            <div className="card-title">
              <HardDrive size={18} color="#06b6d4" />
              <span>核心模型與執行檔路徑 (Model & Binary Paths)</span>
            </div>
            <div className="card-subtitle">
              針對 Qwen3.8-Flash-Next GGUF 模型與 AMD Strix Halo 的深度優化配置。
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
                  <span>Qwen3.8-Flash-Next 主模型 (GGUF 00001)</span>
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
                    placeholder="工作目錄相對路徑或絕對路徑，例如: models\Qwen3.8-Flash-Next-GGUF\..."
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
                        '選擇 Qwen3.8-Flash-Next 主模型 (GGUF)',
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

              {/* MTP 投機解碼模型 */}
              <div className="form-group">
                <div className="form-label">
                  <span>MTP 投機解碼加速模型 (mtp-*.gguf)</span>
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

              {/* Vision 視覺側車模型 */}
              <div className="form-group">
                <div className="form-label">
                  <span>多模態視覺模型 (mmproj-BF16.gguf)</span>
                  {pathStatus.vision_path ? (
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
              {/* OpenAI API Port */}
              <div className="form-group">
                <div className="form-label">
                  <span>OpenAI API 埠號 (Port)</span>
                </div>
                <input
                  type="number"
                  className="form-input"
                  value={form.port}
                  disabled={isRunning}
                  onChange={(e) => handleChange('port', parseInt(e.target.value) || 12223)}
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
                  <option value={262144}>262,144 (256K 全開)</option>
                </select>
              </div>

              {/* GPU 並發 Sessions */}
              <div className="form-group">
                <div className="form-label">
                  <span>GPU 預先分配會話數 (Sessions)</span>
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
              </div>

              {/* 投機解碼策略 */}
              <div className="form-group">
                <div className="form-label">
                  <span>MTP 投機解碼策略 (Policy)</span>
                </div>
                <select
                  className="form-input"
                  value={form.mtp_policy}
                  disabled={isRunning}
                  onChange={(e) => handleChange('mtp_policy', e.target.value)}
                >
                  <option value="survival">survival (動態生存機率停止 - fast_start 推薦)</option>
                  <option value="length">length (固定長度)</option>
                </select>
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
                  若設定 API Key，外部或本地客戶端（如 Open WebUI, Cursor, VS Code 等）向 <code>http://127.0.0.1:12223/v1</code> 發出請求時，皆必須攜帶 <code>Authorization: Bearer &lt;KEY&gt;</code> 標頭。
                </div>
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

      {/* 分頁三：API 串接與效能速度指南 (API & Metrics) */}
      {subTab === 'api_guide' && (
        <>
          <div className="config-card">
            <div className="card-title">
              <Code size={18} color="#38bdf8" />
              <span>本機 OpenAI 相容端點資訊 (API Endpoint)</span>
            </div>
            <div className="card-subtitle">
              Gufo 提供 100% 相容 OpenAI Chat Completions 標準規格的本機高效能推論 API。
            </div>

            <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fit, minmax(260px, 1fr))', gap: 12, marginTop: 8 }}>
              <div style={{ background: 'var(--bg-primary)', padding: '12px 14px', borderRadius: 8, border: '1px solid var(--border-color)' }}>
                <div style={{ fontSize: 11, color: 'var(--text-muted)' }}>API Base URL (端點基礎位址)</div>
                <div style={{ fontFamily: 'var(--font-mono)', fontWeight: 600, color: 'var(--accent-cyan)', marginTop: 4 }}>
                  http://{form.host}:{form.port}/v1
                </div>
              </div>

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

url = "http://${form.host}:${form.port}/v1/chat/completions"
headers = {"Content-Type": "application/json"}
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
      )}
    </div>
  );
};
