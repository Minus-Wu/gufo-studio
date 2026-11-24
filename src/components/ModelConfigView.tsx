import React, { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import {
  Save,
  RotateCcw,
  CheckCircle2,
  AlertCircle,
  FolderOpen,
  Layers,
  Plus,
  Trash2,
  ChevronDown,
  ChevronUp,
  Cpu,
  Zap,
  Sparkles,
  Settings2,
} from 'lucide-react';
import {
  GufoServerConfig,
  ModelFamily,
  ModelProfile,
  SpeculativeMode,
  YarnMode,
} from '../types';

interface ModelConfigViewProps {
  form: GufoServerConfig;
  onChangeForm: React.Dispatch<React.SetStateAction<GufoServerConfig>>;
  onSaveConfig: (newConfig: GufoServerConfig) => void;
  onResetDefaults: () => void;
  isRunning: boolean;
  pathStatus: Record<string, boolean>;
  onBrowse: (
    field: keyof GufoServerConfig,
    title: string,
    filterName: string,
    extensions: string[]
  ) => Promise<void>;
  savedSuccess: boolean;
  onSave: (overrideConfig?: GufoServerConfig) => void;
}

const hintStyle: React.CSSProperties = { fontSize: 11.5, color: '#64748b', marginTop: 2 };
const checkboxLabelStyle: React.CSSProperties = {
  display: 'flex',
  alignItems: 'center',
  gap: 8,
  fontSize: 13,
  cursor: 'pointer',
};

const clampNumber = (raw: string, min: number, max: number, fallback: number) => {
  const n = parseFloat(raw);
  if (!Number.isFinite(n)) return fallback;
  return Math.min(max, Math.max(min, n));
};

const clampInt = (raw: string, min: number, max = Number.MAX_SAFE_INTEGER) =>
  Math.trunc(clampNumber(raw, min, max, min));

const FAMILY_CARDS: {
  id: ModelFamily;
  title: string;
  subtitle: string;
  badge: string;
}[] = [
  {
    id: 'qwen3.8-flash-next',
    title: 'Qwen3.8-Flash-Next',
    subtitle: '高效率混合推論與 MTP 投機加速主力模型',
    badge: 'MTP 投機 · 256K / 400K YaRN · Vision',
  },
  {
    id: 'inkling-2-mini',
    title: 'Inkling-2-Mini',
    subtitle: '26.4B Looped MoE 輕量長視窗多模態模型',
    badge: 'Looped MoE · 1M 原生 · Vision · 可選 DSpark',
  },
  {
    id: 'custom',
    title: '通用 / 自訂模型',
    subtitle: '自由搭配 Qwen3.8-27B 或其他 GGUF 模型參數',
    badge: '自由設定 · MTP / DSpark / YaRN',
  },
];

const QWEN_CONTEXT_OPTIONS = [
  { value: 32768, label: '32,768 (32K)' },
  { value: 65536, label: '65,536 (64K)' },
  { value: 131072, label: '131,072 (128K 推薦)' },
  { value: 262144, label: '262,144 (256K 原生上限)' },
  { value: 409600, label: '409,600 (400K 自動 YaRN)' },
];

const INKLING_CONTEXT_OPTIONS = [
  { value: 32768, label: '32,768 (32K)' },
  { value: 65536, label: '65,536 (64K 推薦)' },
  { value: 131072, label: '131,072 (128K)' },
  { value: 262144, label: '262,144 (256K)' },
  { value: 524288, label: '524,288 (512K)' },
  { value: 1048576, label: '1,048,576 (1M 原生上限)' },
];

const ALL_CONTEXT_OPTIONS = [
  { value: 32768, label: '32,768 (32K)' },
  { value: 65536, label: '65,536 (64K)' },
  { value: 131072, label: '131,072 (128K)' },
  { value: 262144, label: '262,144 (256K)' },
  { value: 409600, label: '409,600 (400K)' },
  { value: 524288, label: '524,288 (512K)' },
  { value: 1048576, label: '1,048,576 (1M)' },
];

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

export function buildCliPreview(cfg: GufoServerConfig): string {
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
  const enableYarn = yarnMode === 'on' || (yarnMode !== 'off' && cfg.context > 262144);
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

function toProfileSnapshot(id: string, name: string, cfg: GufoServerConfig): ModelProfile {
  return {
    id,
    name,
    model_family: cfg.model_family || 'qwen3.8-flash-next',
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
  };
}

function upsertFamilySnapshot(cfg: GufoServerConfig): GufoServerConfig {
  const family = cfg.model_family || 'qwen3.8-flash-next';
  const snapId = `__family_${family}`;
  const snap = toProfileSnapshot(snapId, family, cfg);
  const existing = cfg.model_profiles || [];
  const hasSnap = existing.some((p) => p.id === snapId);
  const nextProfiles = hasSnap
    ? existing.map((p) => (p.id === snapId ? snap : p))
    : [...existing, snap];
  return { ...cfg, model_profiles: nextProfiles };
}

async function pickFirstExistingPath(candidates: string[]): Promise<string> {
  for (const path of candidates) {
    try {
      const ok = await invoke<boolean>('check_path_exists', { path });
      if (ok) return path;
    } catch {
      // 忽略檢查錯誤，繼續下一個候選路徑
    }
  }
  return candidates[0];
}

interface PathFieldProps {
  label: string;
  value: string;
  placeholder: string;
  exists?: boolean;
  optionalEmptyLabel?: string;
  disabled: boolean;
  onChange: (val: string) => void;
  onBrowse: () => void;
  browseTitle: string;
}

const PathField: React.FC<PathFieldProps> = ({
  label,
  value,
  placeholder,
  exists,
  optionalEmptyLabel,
  disabled,
  onChange,
  onBrowse,
  browseTitle,
}) => (
  <div className="form-group">
    <div className="form-label">
      <span>{label}</span>
      {!value.trim() && optionalEmptyLabel ? (
        <span style={{ color: '#94a3b8' }}>{optionalEmptyLabel}</span>
      ) : exists ? (
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
        value={value}
        placeholder={placeholder}
        disabled={disabled}
        onChange={(e) => onChange(e.target.value)}
      />
      <button
        type="button"
        className="btn btn-secondary"
        style={{ flexShrink: 0, padding: '7px 12px' }}
        disabled={disabled}
        onClick={onBrowse}
        title={browseTitle}
      >
        <FolderOpen size={14} />
        <span>瀏覽...</span>
      </button>
    </div>
  </div>
);

export const ModelConfigView: React.FC<ModelConfigViewProps> = ({
  form,
  onChangeForm,
  onSaveConfig,
  onResetDefaults,
  isRunning,
  pathStatus,
  onBrowse,
  savedSuccess,
  onSave,
}) => {
  const [showSaveProfile, setShowSaveProfile] = useState(false);
  const [newProfileName, setNewProfileName] = useState('');
  const [profileNotice, setProfileNotice] = useState<string | null>(null);
  const [showAdvanced, setShowAdvanced] = useState(false);

  const currentFamily: ModelFamily = form.model_family || 'qwen3.8-flash-next';
  const userProfiles = (form.model_profiles || []).filter((p) => !p.id.startsWith('__family_'));
  const activeProfile = userProfiles.find((p) => p.id === form.active_profile_id);

  const updateField = <K extends keyof GufoServerConfig>(field: K, value: GufoServerConfig[K]) => {
    onChangeForm((prev) => ({ ...prev, [field]: value }));
  };

  const showNotice = (msg: string) => {
    setProfileNotice(msg);
    setTimeout(() => setProfileNotice(null), 5000);
  };

  const handleSelectFamily = async (targetFamily: ModelFamily) => {
    if (isRunning || targetFamily === currentFamily) return;

    const withSnapshot = upsertFamilySnapshot(form);
    const profiles = withSnapshot.model_profiles || [];
    const targetSnap = profiles.find((p) => p.id === `__family_${targetFamily}`);

    if (targetSnap) {
      onChangeForm({
        ...withSnapshot,
        model_family: targetFamily,
        active_profile_id: '',
        model_path: targetSnap.model_path,
        mtp_path: targetSnap.mtp_path,
        dspark_path: targetSnap.dspark_path,
        vision_path: targetSnap.vision_path,
        served_model_name: targetSnap.served_model_name,
        context: targetSnap.context,
        sessions: targetSnap.sessions,
        think: targetSnap.think,
        temperature: targetSnap.temperature,
        max_tokens: targetSnap.max_tokens,
        speculative: targetSnap.speculative,
        yarn_mode: targetSnap.yarn_mode,
        extra_args: targetSnap.extra_args,
      });
      showNotice(`已切換至「${FAMILY_CARDS.find((c) => c.id === targetFamily)?.title}」並還原上次設定，確認後請點擊「儲存模型配置」。`);
      return;
    }

    if (targetFamily === 'inkling-2-mini') {
      const [modelPath, visionPath] = await Promise.all([
        pickFirstExistingPath([
          'D:\\Models\\GGUF\\Inkling-2-Mini-GGUF\\Inkling-2-Mini-UD-Q4_K_XL.gguf',
          'models\\Inkling-2-Mini-GGUF\\Inkling-2-Mini-UD-Q4_K_XL.gguf',
          'D:\\Models\\GGUF\\Inkling-2-Mini-GGUF\\Inkling-2-Mini-UD-Q4_K_M.gguf',
        ]),
        pickFirstExistingPath([
          'D:\\Models\\GGUF\\Inkling-2-Mini-GGUF\\mmproj-BF16.gguf',
          'models\\Inkling-2-Mini-GGUF\\mmproj-BF16.gguf',
        ]),
      ]);
      onChangeForm({
        ...withSnapshot,
        model_family: 'inkling-2-mini',
        active_profile_id: '',
        model_path: modelPath,
        mtp_path: '',
        dspark_path: '',
        vision_path: visionPath,
        served_model_name: 'inkling-2-mini',
        context: 65536,
        sessions: 2,
        think: 'auto',
        temperature: 0,
        max_tokens: 16384,
        speculative: 'off',
        yarn_mode: 'off',
        extra_args: '',
      });
      showNotice('已切換至 Inkling-2-Mini 專屬參數（原生 1M 視窗、YaRN 關閉），確認路徑後請點擊「儲存模型配置」。');
    } else if (targetFamily === 'qwen3.8-flash-next') {
      const [modelPath, mtpPath, visionPath] = await Promise.all([
        pickFirstExistingPath([
          'D:\\Models\\GGUF\\Qwen3.8-Flash-Next-GGUF\\Qwen3.8-Flash-Next-UD-Q4_K_XL-00001-of-00004.gguf',
          'models\\Qwen3.8-Flash-Next-GGUF\\Qwen3.8-Flash-Next-UD-Q4_K_XL-00001-of-00004.gguf',
        ]),
        pickFirstExistingPath([
          'D:\\Models\\GGUF\\Qwen3.8-Flash-Next-GGUF\\mtp-Qwen3.8-Flash-Next-shared-Q8_0.gguf',
          'models\\Qwen3.8-Flash-Next-GGUF\\mtp-Qwen3.8-Flash-Next-shared-Q8_0.gguf',
        ]),
        pickFirstExistingPath([
          'D:\\Models\\GGUF\\Qwen3.8-Flash-Next-GGUF\\mmproj-BF16.gguf',
          'models\\Qwen3.8-Flash-Next-GGUF\\mmproj-BF16.gguf',
        ]),
      ]);
      onChangeForm({
        ...withSnapshot,
        model_family: 'qwen3.8-flash-next',
        active_profile_id: '',
        model_path: modelPath,
        mtp_path: mtpPath,
        dspark_path: '',
        vision_path: visionPath,
        served_model_name: 'flash-next',
        context: 131072,
        sessions: 2,
        think: 'off',
        temperature: 0,
        max_tokens: 32768,
        speculative: 'mtp',
        yarn_mode: 'auto',
        extra_args: '',
      });
      showNotice('已切換至 Qwen3.8-Flash-Next 專屬參數，確認後請點擊「儲存模型配置」。');
    } else {
      onChangeForm({
        ...withSnapshot,
        model_family: 'custom',
        active_profile_id: '',
        speculative: 'off',
        yarn_mode: 'auto',
      });
      showNotice('已切換至通用 / 自訂模型模式，可自由調整所有推論參數。');
    }
  };

  const handleSelectProfile = (profileId: string) => {
    if (!profileId) {
      updateField('active_profile_id', '');
      return;
    }
    const found = userProfiles.find((p) => p.id === profileId);
    if (!found) return;
    onChangeForm((prev) => ({
      ...prev,
      active_profile_id: found.id,
      model_family: found.model_family || 'custom',
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
    showNotice(`已載入自訂設定檔「${found.name}」，確認無誤後請點擊「儲存模型配置」。`);
  };

  const handleCreateProfile = (e: React.FormEvent) => {
    e.preventDefault();
    const cleanName = newProfileName.trim();
    if (!cleanName) return;
    const id = `profile_${Date.now()}`;
    const snap = toProfileSnapshot(id, cleanName, form);
    const withFamily = upsertFamilySnapshot(form);
    const updated: GufoServerConfig = {
      ...withFamily,
      model_profiles: [...(withFamily.model_profiles || []), snap],
      active_profile_id: id,
    };
    onChangeForm(updated);
    onSaveConfig(updated);
    setNewProfileName('');
    setShowSaveProfile(false);
    showNotice(`已儲存新設定檔「${cleanName}」。`);
  };

  const handleUpdateActiveProfile = () => {
    if (!activeProfile) return;
    const snap = toProfileSnapshot(activeProfile.id, activeProfile.name, form);
    const withFamily = upsertFamilySnapshot(form);
    const updated: GufoServerConfig = {
      ...withFamily,
      model_profiles: (withFamily.model_profiles || []).map((p) =>
        p.id === activeProfile.id ? snap : p
      ),
    };
    onChangeForm(updated);
    onSaveConfig(updated);
    showNotice(`已更新設定檔「${activeProfile.name}」。`);
  };

  const handleDeleteActiveProfile = () => {
    if (!activeProfile) return;
    if (!confirm(`確定要刪除自訂模型設定檔「${activeProfile.name}」嗎？`)) return;
    const updated: GufoServerConfig = {
      ...form,
      model_profiles: (form.model_profiles || []).filter((p) => p.id !== activeProfile.id),
      active_profile_id: '',
    };
    onChangeForm(updated);
    onSaveConfig(updated);
    showNotice(`已刪除設定檔「${activeProfile.name}」。`);
  };

  const handleSaveModel = () => {
    const updated = upsertFamilySnapshot(form);
    onChangeForm(updated);
    onSave(updated);
  };

  const contextOptions =
    currentFamily === 'qwen3.8-flash-next'
      ? QWEN_CONTEXT_OPTIONS
      : currentFamily === 'inkling-2-mini'
      ? INKLING_CONTEXT_OPTIONS
      : ALL_CONTEXT_OPTIONS;

  return (
    <>
      {/* 步驟一：選擇模型 */}
      <div className="config-card">
        <div className="card-title">
          <Layers size={18} color="#38bdf8" />
          <span>步驟 1：選擇模型類型</span>
        </div>
        <div className="card-subtitle">
          先選擇要使用的模型，系統只會顯示該模型專屬的檔案與參數，並自動記憶各自最後一次配置。
        </div>

        <div className="model-family-grid">
          {FAMILY_CARDS.map((card) => {
            const active = currentFamily === card.id;
            return (
              <button
                key={card.id}
                type="button"
                className={`model-family-card ${active ? 'active' : ''}`}
                disabled={isRunning}
                onClick={() => handleSelectFamily(card.id)}
              >
                <div className="model-family-card-head">
                  <span className="model-family-title">{card.title}</span>
                  {active && <span className="model-family-active-dot">目前選擇</span>}
                </div>
                <div className="model-family-sub">{card.subtitle}</div>
                <span className="model-family-badge">{card.badge}</span>
              </button>
            );
          })}
        </div>

        {/* 自訂設定檔精簡列 */}
        <div className="profile-toolbar" style={{ marginTop: 14, paddingTop: 12, borderTop: '1px solid var(--border-color)' }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 8, flex: 1, minWidth: 240 }}>
            <span style={{ fontSize: 12, color: 'var(--text-muted)', whiteSpace: 'nowrap' }}>
              自訂設定檔：
            </span>
            <select
              className="form-input"
              style={{ flex: 1 }}
              value={activeProfile ? activeProfile.id : ''}
              disabled={isRunning}
              onChange={(e) => handleSelectProfile(e.target.value)}
            >
              <option value="">-- 選擇已儲存的自訂設定檔 ({userProfiles.length}) --</option>
              {userProfiles.map((p) => (
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
            <span>另存自訂設定檔</span>
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
              placeholder="輸入設定檔名稱，例如：Inkling-2-Mini (128K 長對話)"
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

        {profileNotice && (
          <div style={{ marginTop: 10, fontSize: 12, color: '#34d399' }}>{profileNotice}</div>
        )}
      </div>

      {/* 步驟二：模型專屬檔案與參數 */}
      <div className="config-card">
        <div className="card-title">
          {currentFamily === 'qwen3.8-flash-next' ? (
            <Zap size={18} color="#06b6d4" />
          ) : currentFamily === 'inkling-2-mini' ? (
            <Sparkles size={18} color="#38bdf8" />
          ) : (
            <Cpu size={18} color="#8b5cf6" />
          )}
          <span>
            步驟 2：
            {currentFamily === 'qwen3.8-flash-next'
              ? 'Qwen3.8-Flash-Next 檔案與推論參數'
              : currentFamily === 'inkling-2-mini'
              ? 'Inkling-2-Mini 檔案與推論參數'
              : '自訂模型檔案與推論參數'}
          </span>
        </div>
        <div className="card-subtitle">
          {currentFamily === 'qwen3.8-flash-next'
            ? '專為 Qwen3.8-Flash-Next 配置：超過 256K 上下文時自動啟用 YaRN，支援 MTP 投機加速與視覺輸入。'
            : currentFamily === 'inkling-2-mini'
            ? '專為 Inkling-2-Mini (Looped MoE) 配置：原生支援最高 1M Context（預設關閉 YaRN），支援視覺多模態與選用 DSpark。'
            : '顯示所有進階模型控制項，適合 Qwen3.8-27B 或其他社群 GGUF 模型。'}
        </div>

        {/* 模型檔案區塊 */}
        <div className="form-grid single-col" style={{ marginBottom: 18 }}>
          <PathField
            label={
              currentFamily === 'qwen3.8-flash-next'
                ? 'Qwen3.8-Flash-Next 主模型 (GGUF 00001)'
                : currentFamily === 'inkling-2-mini'
                ? 'Inkling-2-Mini 主模型 (GGUF)'
                : '主模型檔案 (GGUF)'
            }
            value={form.model_path}
            placeholder={
              currentFamily === 'inkling-2-mini'
                ? 'D:\\Models\\GGUF\\Inkling-2-Mini-GGUF\\Inkling-2-Mini-UD-Q4_K_XL.gguf'
                : 'models\\Qwen3.8-Flash-Next-GGUF\\...'
            }
            exists={pathStatus.model_path}
            disabled={isRunning}
            onChange={(val) => updateField('model_path', val)}
            onBrowse={() =>
              onBrowse('model_path', '選擇主模型 (GGUF)', 'GGUF 模型檔 (*.gguf)', ['gguf'])
            }
            browseTitle="瀏覽選擇主模型 GGUF 檔案"
          />

          <PathField
            label="多模態視覺模型 (mmproj-*.gguf，選填)"
            value={form.vision_path}
            placeholder="留空表示僅使用純文字模式"
            exists={pathStatus.vision_path}
            optionalEmptyLabel="留空（純文字模式）"
            disabled={isRunning}
            onChange={(val) => updateField('vision_path', val)}
            onBrowse={() =>
              onBrowse('vision_path', '選擇多模態視覺模型 (mmproj *.gguf)', 'GGUF 模型檔 (*.gguf)', ['gguf'])
            }
            browseTitle="瀏覽選擇視覺模型 GGUF 檔案"
          />

          {/* 投機解碼開關 / 路徑 (依模型類型精簡呈現) */}
          {currentFamily === 'qwen3.8-flash-next' && (
            <div className="form-group">
              <label style={checkboxLabelStyle}>
                <input
                  type="checkbox"
                  checked={form.speculative === 'mtp'}
                  disabled={isRunning}
                  onChange={(e) => updateField('speculative', e.target.checked ? 'mtp' : 'off')}
                />
                <span>啟用 MTP 投機解碼加速（推薦）</span>
              </label>
              {form.speculative === 'mtp' && (
                <div style={{ marginTop: 8 }}>
                  <PathField
                    label="MTP 投機解碼加速模型 (--mtp-model)"
                    value={form.mtp_path}
                    placeholder="models\\Qwen3.8-Flash-Next-GGUF\\mtp-*.gguf"
                    exists={pathStatus.mtp_path}
                    disabled={isRunning}
                    onChange={(val) => updateField('mtp_path', val)}
                    onBrowse={() =>
                      onBrowse('mtp_path', '選擇 MTP 投機解碼加速模型 (GGUF)', 'GGUF 模型檔 (*.gguf)', ['gguf'])
                    }
                    browseTitle="瀏覽選擇 MTP 模型 GGUF 檔案"
                  />
                </div>
              )}
            </div>
          )}

          {currentFamily === 'inkling-2-mini' && (
            <div className="form-group">
              <label style={checkboxLabelStyle}>
                <input
                  type="checkbox"
                  checked={form.speculative === 'dspark'}
                  disabled={isRunning}
                  onChange={(e) => updateField('speculative', e.target.checked ? 'dspark' : 'off')}
                />
                <span>啟用 DSpark 投機解碼加速（選用，需搭配 dspark-Inkling-2-Mini-Q8_0.gguf）</span>
              </label>
              {form.speculative === 'dspark' && (
                <div style={{ marginTop: 8 }}>
                  <PathField
                    label="DSpark 投機解碼模型 (--dspark-model)"
                    value={form.dspark_path || ''}
                    placeholder="D:\\Models\\GGUF\\Inkling-2-Mini-GGUF\\dspark-Inkling-2-Mini-Q8_0.gguf"
                    exists={pathStatus.dspark_path}
                    optionalEmptyLabel="尚未指定檔案"
                    disabled={isRunning}
                    onChange={(val) => updateField('dspark_path', val)}
                    onBrowse={() =>
                      onBrowse('dspark_path', '選擇 DSpark 投機解碼模型 (GGUF)', 'GGUF 模型檔 (*.gguf)', ['gguf'])
                    }
                    browseTitle="瀏覽選擇 DSpark 模型 GGUF 檔案"
                  />
                </div>
              )}
            </div>
          )}

          {currentFamily === 'custom' && (
            <>
              <div className="form-group">
                <div className="form-label">
                  <span>投機解碼模式 (--speculative)</span>
                </div>
                <select
                  className="form-input"
                  value={
                    form.speculative === 'dspark'
                      ? 'dspark'
                      : form.speculative === 'mtp'
                      ? 'mtp'
                      : 'off'
                  }
                  disabled={isRunning}
                  onChange={(e) => updateField('speculative', e.target.value as SpeculativeMode)}
                >
                  <option value="off">off (關閉投機解碼)</option>
                  <option value="mtp">mtp (MTP 草稿模型)</option>
                  <option value="dspark">dspark (DSpark 草稿模型)</option>
                </select>
              </div>

              {form.speculative === 'mtp' && (
                <PathField
                  label="MTP 投機解碼模型 (--mtp-model)"
                  value={form.mtp_path}
                  placeholder="選擇 mtp-*.gguf"
                  exists={pathStatus.mtp_path}
                  disabled={isRunning}
                  onChange={(val) => updateField('mtp_path', val)}
                  onBrowse={() =>
                    onBrowse('mtp_path', '選擇 MTP 投機解碼模型 (GGUF)', 'GGUF 模型檔 (*.gguf)', ['gguf'])
                  }
                  browseTitle="瀏覽選擇 MTP 模型"
                />
              )}

              {form.speculative === 'dspark' && (
                <PathField
                  label="DSpark 投機解碼模型 (--dspark-model)"
                  value={form.dspark_path || ''}
                  placeholder="選擇 dspark-*.gguf"
                  exists={pathStatus.dspark_path}
                  optionalEmptyLabel="尚未指定檔案"
                  disabled={isRunning}
                  onChange={(val) => updateField('dspark_path', val)}
                  onBrowse={() =>
                    onBrowse('dspark_path', '選擇 DSpark 投機解碼模型 (GGUF)', 'GGUF 模型檔 (*.gguf)', ['gguf'])
                  }
                  browseTitle="瀏覽選擇 DSpark 模型"
                />
              )}
            </>
          )}
        </div>

        {/* 模型推論參數網格 */}
        <div className="form-grid">
          <div className="form-group">
            <div className="form-label">
              <span>模型服務識別名稱 (Served Model Name)</span>
            </div>
            <input
              type="text"
              className="form-input"
              value={form.served_model_name}
              disabled={isRunning}
              onChange={(e) => updateField('served_model_name', e.target.value)}
            />
          </div>

          <div className="form-group">
            <div className="form-label">
              <span>上下文視窗長度 (Context Tokens)</span>
            </div>
            <select
              className="form-input"
              value={form.context}
              disabled={isRunning}
              onChange={(e) => updateField('context', parseInt(e.target.value, 10))}
            >
              {contextOptions.map((opt) => (
                <option key={opt.value} value={opt.value}>
                  {opt.label}
                </option>
              ))}
              {!contextOptions.some((o) => o.value === form.context) && (
                <option value={form.context}>{form.context.toLocaleString()} (自訂)</option>
              )}
            </select>
            {currentFamily === 'qwen3.8-flash-next' && form.context > 262144 && (
              <div style={hintStyle}>超過 256K 時會自動附加 <code>--enable-yarn</code>。</div>
            )}
            {currentFamily === 'inkling-2-mini' && (
              <div style={hintStyle}>Inkling-2-Mini 原生支援最高 1M，無須開啟 YaRN。</div>
            )}
          </div>

          {currentFamily === 'custom' && (
            <div className="form-group">
              <div className="form-label">
                <span>YaRN 位置編碼擴展 (--enable-yarn)</span>
              </div>
              <select
                className="form-input"
                value={form.yarn_mode || 'auto'}
                disabled={isRunning}
                onChange={(e) => updateField('yarn_mode', e.target.value as YarnMode)}
              >
                <option value="auto">auto (超過 256K 自動附加)</option>
                <option value="off">off (強制關閉)</option>
                <option value="on">on (強制開啟)</option>
              </select>
            </div>
          )}

          <div className="form-group">
            <div className="form-label">
              <span>GPU 並發會話數 (Sessions)</span>
            </div>
            <select
              className="form-input"
              value={form.sessions}
              disabled={isRunning}
              onChange={(e) => updateField('sessions', parseInt(e.target.value, 10))}
            >
              <option value={1}>1 (單會話，顯存最省)</option>
              <option value={2}>2 (雙路並發，推薦)</option>
              <option value={4}>4 (多路並發)</option>
            </select>
            <div style={hintStyle}>
              目前 context × sessions = {(form.context * form.sessions).toLocaleString()} tokens
            </div>
          </div>

          <div className="form-group">
            <div className="form-label">
              <span>預設思考鏈狀態 (Think Mode)</span>
            </div>
            <select
              className="form-input"
              value={form.think}
              disabled={isRunning}
              onChange={(e) =>
                updateField('think', e.target.value as 'off' | 'on' | 'auto')
              }
            >
              <option value="off">off (關閉思考，延遲最低)</option>
              <option value="on">on (開啟深度推導思考)</option>
              <option value="auto">auto (自動遵循模型提示)</option>
            </select>
          </div>

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
              onChange={(e) => updateField('temperature', clampNumber(e.target.value, 0, 2, 0))}
            />
          </div>

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
              onChange={(e) => updateField('max_tokens', clampInt(e.target.value, 0))}
            />
            <div style={hintStyle}>
              {currentFamily === 'inkling-2-mini'
                ? '建議保留生成上限（如 16384），避免循環架構偶發長推理迴圈；0 = 不限制。'
                : '請求未帶 max_tokens 時的上限；0 = 交給 Gufo 預設。'}
            </div>
          </div>
        </div>

        {/* 折疊式進階參數與啟動指令預覽 */}
        <div style={{ marginTop: 18 }}>
          <button
            type="button"
            className="model-advanced-toggle"
            onClick={() => setShowAdvanced((v) => !v)}
          >
            <span style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
              <Settings2 size={14} color="#94a3b8" />
              <span>進階參數與啟動指令預覽</span>
            </span>
            {showAdvanced ? <ChevronUp size={15} /> : <ChevronDown size={15} />}
          </button>

          {showAdvanced && (
            <div style={{ marginTop: 12, display: 'flex', flexDirection: 'column', gap: 12 }}>
              <div className="form-group">
                <div className="form-label">
                  <span>額外 CLI 啟動參數 (Extra Arguments，選填)</span>
                </div>
                <input
                  type="text"
                  className="form-input"
                  placeholder="例如: --extension extensions\inkling2.dll（支援雙引號包住含空白路徑）"
                  value={form.extra_args || ''}
                  disabled={isRunning}
                  onChange={(e) => updateField('extra_args', e.target.value)}
                />
                <div style={hintStyle}>直接附加於啟動指令尾端，適合傳遞擴充套件或實驗性旗標。</div>
              </div>

              <div className="form-group">
                <div className="form-label">
                  <span>啟動指令預覽 (Command Preview)</span>
                </div>
                <div className="cli-preview-box">{buildCliPreview(form)}</div>
              </div>
            </div>
          )}
        </div>

        {/* 底部按鈕區 */}
        <div style={{ marginTop: 22, display: 'flex', justifyContent: 'flex-end', gap: 12 }}>
          <button className="btn btn-secondary" onClick={onResetDefaults} disabled={isRunning}>
            <RotateCcw size={14} />
            <span>還原預設配置</span>
          </button>
          <button className="btn btn-primary" onClick={handleSaveModel} disabled={isRunning}>
            <Save size={14} />
            <span>{savedSuccess ? '已儲存！' : '儲存模型配置'}</span>
          </button>
        </div>
      </div>
    </>
  );
};
