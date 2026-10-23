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
  const [form, setForm] = useState<GufoServerConfig>(initialConfig);
  const [pathStatus, setPathStatus] = useState<{ [key: string]: boolean }>({});
  const [savedSuccess, setSavedSuccess] = useState(false);
  const [showApiKey, setShowApiKey] = useState(false);

  const generateApiKey = () => {
    const randomHex = Array.from(crypto.getRandomValues(new Uint8Array(16)))
      .map((b) => b.toString(16).padStart(2, '0'))
      .join('');
    handleChange('api_key', `sk-gufo-${randomHex}`);
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

  const handleSave = () => {
    onSaveConfig(form);
    setSavedSuccess(true);
    setTimeout(() => setSavedSuccess(false), 2500);
  };

  return (
    <div className="config-container">
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
            <input
              type="text"
              className="form-input"
              value={form.gufo_path}
              placeholder="工作目錄相對路徑或絕對路徑，例如: build\release\gufo.exe 或 gufo.exe"
              disabled={isRunning}
              onChange={(e) => handleChange('gufo_path', e.target.value)}
            />
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
            <input
              type="text"
              className="form-input"
              value={form.model_path}
              placeholder="工作目錄相對路徑或絕對路徑，例如: models\Qwen3.8-Flash-Next-GGUF\..."
              disabled={isRunning}
              onChange={(e) => handleChange('model_path', e.target.value)}
            />
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
            <input
              type="text"
              className="form-input"
              value={form.mtp_path}
              placeholder="工作目錄相對路徑或絕對路徑，例如: models\Qwen3.8-Flash-Next-GGUF\mtp-*.gguf"
              disabled={isRunning}
              onChange={(e) => handleChange('mtp_path', e.target.value)}
            />
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
            <input
              type="text"
              className="form-input"
              value={form.vision_path}
              placeholder="工作目錄相對路徑或絕對路徑，例如: models\Qwen3.8-Flash-Next-GGUF\mmproj-BF16.gguf"
              disabled={isRunning}
              onChange={(e) => handleChange('vision_path', e.target.value)}
            />
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
    </div>
  );
};
