import React, { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import {
  BookOpen,
  Plus,
  Trash2,
  FileUp,
  FolderUp,
  FileText,
  Search,
  Settings,
  RefreshCw,
  Eye,
  EyeOff,
  CheckCircle2,
  AlertCircle,
  Sparkles,
  Layers,
  X,
  Save,
  Loader2,
  Zap,
} from 'lucide-react';
import {
  EmbeddingTestResult,
  KnowledgeBaseDetail,
  KnowledgeBaseSummary,
  RagChunkPreview,
  RagConfig,
  RagImportResult,
  RagReindexResult,
  RagSearchResponse,
} from '../types';

interface RagViewProps {
  kbList: KnowledgeBaseSummary[];
  ragConfig: RagConfig;
  onKbsChanged: () => Promise<void>;
  onSaveConfig: (cfg: RagConfig) => Promise<void>;
}

const DEFAULT_TEMPLATE =
  '請優先依據以下「檢索參考資料」回答使用者的問題。回答時若引用了資料內容，請以 [來源編號]（例如 [1]、[2]）標註出處；若參考資料不足以回答或與問題無關，請誠實說明，不要編造事實。\n\n【檢索參考資料】\n{{context}}';

export const RagView: React.FC<RagViewProps> = ({
  kbList,
  ragConfig,
  onKbsChanged,
  onSaveConfig,
}) => {
  const [subTab, setSubTab] = useState<'kbs' | 'search' | 'settings'>('kbs');
  const [selectedKbId, setSelectedKbId] = useState<string | null>(null);
  const [kbDetail, setKbDetail] = useState<KnowledgeBaseDetail | null>(null);
  const [loadingDetail, setLoadingDetail] = useState(false);

  // 建立知識庫表單
  const [showCreateKb, setShowCreateKb] = useState(false);
  const [newKbName, setNewKbName] = useState('');
  const [newKbDesc, setNewKbDesc] = useState('');
  const [creatingKb, setCreatingKb] = useState(false);

  // 匯入 / 操作狀態
  const [importing, setImporting] = useState(false);
  const [reindexing, setReindexing] = useState(false);
  const [importFeedback, setImportFeedback] = useState<{
    type: 'success' | 'error';
    message: string;
    details?: string[];
  } | null>(null);

  // 手動貼上文字表單
  const [showTextForm, setShowTextForm] = useState(false);
  const [noteTitle, setNoteTitle] = useState('');
  const [noteContent, setNoteContent] = useState('');

  // 切片預覽
  const [previewDocName, setPreviewDocName] = useState<string | null>(null);
  const [previewChunks, setPreviewChunks] = useState<RagChunkPreview[]>([]);

  // 檢索測試
  const [testKbIds, setTestKbIds] = useState<string[]>([]);
  const [testQuery, setTestQuery] = useState('');
  const [searching, setSearching] = useState(false);
  const [searchResult, setSearchResult] = useState<RagSearchResponse | null>(null);

  // 設定表單
  const [cfgForm, setCfgForm] = useState<RagConfig>(ragConfig);
  const [showEmbedKey, setShowEmbedKey] = useState(false);
  const [savingCfg, setSavingCfg] = useState(false);
  const [savedCfgOk, setSavedCfgOk] = useState(false);
  const [testingEmbed, setTestingEmbed] = useState(false);
  const [embedTestResult, setEmbedTestResult] = useState<{
    ok: boolean;
    message: string;
  } | null>(null);

  useEffect(() => {
    setCfgForm(ragConfig);
  }, [ragConfig]);

  // 自動選中第一個知識庫，或同步刪除後的選取狀態
  useEffect(() => {
    if (kbList.length === 0) {
      setSelectedKbId(null);
      setKbDetail(null);
      setTestKbIds([]);
      return;
    }
    if (!selectedKbId || !kbList.some((k) => k.id === selectedKbId)) {
      setSelectedKbId(kbList[0].id);
    }
    setTestKbIds((prev) => {
      const valid = prev.filter((id) => kbList.some((k) => k.id === id));
      return valid.length > 0 ? valid : kbList.map((k) => k.id);
    });
  }, [kbList, selectedKbId]);

  const loadKbDetail = async (id: string) => {
    setLoadingDetail(true);
    try {
      const detail = await invoke<KnowledgeBaseDetail>('rag_get_kb', { kbId: id });
      setKbDetail(detail);
    } catch (err) {
      console.error('載入知識庫明細失敗:', err);
      setKbDetail(null);
    } finally {
      setLoadingDetail(false);
    }
  };

  useEffect(() => {
    if (selectedKbId) {
      loadKbDetail(selectedKbId);
    }
  }, [selectedKbId, kbList]);

  const handleCreateKb = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!newKbName.trim() || creatingKb) return;
    setCreatingKb(true);
    try {
      const created = await invoke<KnowledgeBaseSummary>('rag_create_kb', {
        name: newKbName.trim(),
        description: newKbDesc.trim(),
      });
      setNewKbName('');
      setNewKbDesc('');
      setShowCreateKb(false);
      await onKbsChanged();
      setSelectedKbId(created.id);
    } catch (err: any) {
      alert(`建立知識庫失敗：${err}`);
    } finally {
      setCreatingKb(false);
    }
  };

  const handleDeleteKb = async (kb: KnowledgeBaseSummary) => {
    if (!confirm(`確定要刪除知識庫「${kb.name}」及其中所有文件嗎？此動作無法復原。`)) {
      return;
    }
    try {
      await invoke('rag_delete_kb', { kbId: kb.id });
      setImportFeedback(null);
      await onKbsChanged();
    } catch (err: any) {
      alert(`刪除知識庫失敗：${err}`);
    }
  };

  const applyImportFeedback = (res: RagImportResult, label: string) => {
    const details: string[] = [];
    if (res.replaced_docs > 0) details.push(`已更新取代 ${res.replaced_docs} 份同名路徑文件`);
    if (res.skipped_files > 0) details.push(`略過 ${res.skipped_files} 個非支援文字格式檔案`);
    if (res.embedding_warning) details.push(res.embedding_warning);
    for (const err of res.errors.slice(0, 8)) {
      details.push(`❌ ${err.file}：${err.reason}`);
    }
    if (res.errors.length > 8) {
      details.push(`...另有 ${res.errors.length - 8} 個檔案錯誤`);
    }

    const msg = `${label}：匯入 ${res.imported_docs} 份文件，產生 ${res.added_chunks} 個切片${
      res.embedded_chunks > 0 ? `（已完成 ${res.embedded_chunks} 個向量化）` : ''
    }`;
    setImportFeedback({
      type: res.imported_docs > 0 ? 'success' : 'error',
      message: msg,
      details,
    });
  };

  const handlePickFiles = async () => {
    if (!selectedKbId || importing) return;
    try {
      const paths = await invoke<string[]>('rag_pick_files');
      if (!paths || paths.length === 0) return;
      setImporting(true);
      setImportFeedback(null);
      const res = await invoke<RagImportResult>('rag_add_files', {
        kbId: selectedKbId,
        filePaths: paths,
      });
      applyImportFeedback(res, '檔案匯入完成');
      await onKbsChanged();
      await loadKbDetail(selectedKbId);
    } catch (err: any) {
      setImportFeedback({ type: 'error', message: `匯入檔案失敗：${err}` });
    } finally {
      setImporting(false);
    }
  };

  const handlePickFolder = async () => {
    if (!selectedKbId || importing) return;
    try {
      const folder = await invoke<string | null>('rag_pick_folder');
      if (!folder) return;
      setImporting(true);
      setImportFeedback(null);
      const res = await invoke<RagImportResult>('rag_add_folder', {
        kbId: selectedKbId,
        folderPath: folder,
      });
      applyImportFeedback(res, '資料夾批次匯入完成');
      await onKbsChanged();
      await loadKbDetail(selectedKbId);
    } catch (err: any) {
      setImportFeedback({ type: 'error', message: `匯入資料夾失敗：${err}` });
    } finally {
      setImporting(false);
    }
  };

  const handleAddTextNote = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!selectedKbId || !noteTitle.trim() || !noteContent.trim() || importing) return;
    setImporting(true);
    setImportFeedback(null);
    try {
      const res = await invoke<RagImportResult>('rag_add_text', {
        kbId: selectedKbId,
        title: noteTitle.trim(),
        content: noteContent,
      });
      applyImportFeedback(res, '筆記匯入完成');
      setNoteTitle('');
      setNoteContent('');
      setShowTextForm(false);
      await onKbsChanged();
      await loadKbDetail(selectedKbId);
    } catch (err: any) {
      setImportFeedback({ type: 'error', message: `匯入筆記失敗：${err}` });
    } finally {
      setImporting(false);
    }
  };

  const handleDeleteDoc = async (docId: string, docName: string) => {
    if (!selectedKbId) return;
    if (!confirm(`確定要從知識庫移除「${docName}」嗎？`)) return;
    try {
      await invoke('rag_delete_document', { kbId: selectedKbId, docId });
      await onKbsChanged();
      await loadKbDetail(selectedKbId);
    } catch (err: any) {
      alert(`刪除文件失敗：${err}`);
    }
  };

  const handlePreviewChunks = async (docId: string, docName: string) => {
    if (!selectedKbId) return;
    try {
      const chunks = await invoke<RagChunkPreview[]>('rag_get_doc_chunks', {
        kbId: selectedKbId,
        docId,
      });
      setPreviewDocName(docName);
      setPreviewChunks(chunks);
    } catch (err: any) {
      alert(`讀取切片失敗：${err}`);
    }
  };

  const handleReindexKb = async () => {
    if (!selectedKbId || reindexing) return;
    setReindexing(true);
    setImportFeedback(null);
    try {
      const res = await invoke<RagReindexResult>('rag_reindex_kb', { kbId: selectedKbId });
      setImportFeedback({
        type: 'success',
        message: `向量索引重建完成：共 ${res.embedded_chunks}/${res.total_chunks} 個切片已向量化${
          res.embedding_model_tag ? `（模型：${res.embedding_model_tag}）` : ''
        }`,
      });
      await onKbsChanged();
      await loadKbDetail(selectedKbId);
    } catch (err: any) {
      setImportFeedback({ type: 'error', message: `重建向量索引失敗：${err}` });
    } finally {
      setReindexing(false);
    }
  };

  const handleRunSearch = async (e?: React.FormEvent) => {
    e?.preventDefault();
    if (!testQuery.trim() || testKbIds.length === 0 || searching) return;
    setSearching(true);
    try {
      const res = await invoke<RagSearchResponse>('rag_search', {
        kbIds: testKbIds,
        query: testQuery.trim(),
      });
      setSearchResult(res);
    } catch (err: any) {
      alert(`檢索失敗：${err}`);
    } finally {
      setSearching(false);
    }
  };

  const handleSaveSettings = async (e: React.FormEvent) => {
    e.preventDefault();
    setSavingCfg(true);
    try {
      await onSaveConfig(cfgForm);
      setSavedCfgOk(true);
      setTimeout(() => setSavedCfgOk(false), 3000);
    } catch (err: any) {
      alert(`儲存 RAG 設定失敗：${err}`);
    } finally {
      setSavingCfg(false);
    }
  };

  const handleTestEmbedding = async () => {
    setTestingEmbed(true);
    setEmbedTestResult(null);
    try {
      const res = await invoke<EmbeddingTestResult>('rag_test_embedding', { config: cfgForm });
      setEmbedTestResult({
        ok: true,
        message: `連線成功！端點 ${res.endpoint_url}，向量維度 ${res.dimension}，耗時 ${res.latency_ms}ms`,
      });
    } catch (err: any) {
      setEmbedTestResult({
        ok: false,
        message: `${err}`,
      });
    } finally {
      setTestingEmbed(false);
    }
  };

  const toggleTestKb = (id: string) => {
    setTestKbIds((prev) =>
      prev.includes(id) ? prev.filter((item) => item !== id) : [...prev, id]
    );
  };

  const modelMismatch =
    ragConfig.embedding_enabled &&
    !!ragConfig.embedding_model &&
    !!kbDetail?.summary.embedding_model_tag &&
    kbDetail.summary.embedding_model_tag !== ragConfig.embedding_model;

  return (
    <div className="config-container">
      {/* 子導覽頁籤 */}
      <div className="config-nav">
        <button
          type="button"
          className={`config-nav-btn ${subTab === 'kbs' ? 'active' : ''}`}
          onClick={() => setSubTab('kbs')}
        >
          <BookOpen size={15} />
          <span>知識庫管理</span>
        </button>
        <button
          type="button"
          className={`config-nav-btn ${subTab === 'search' ? 'active' : ''}`}
          onClick={() => setSubTab('search')}
        >
          <Search size={15} />
          <span>檢索測試</span>
        </button>
        <button
          type="button"
          className={`config-nav-btn ${subTab === 'settings' ? 'active' : ''}`}
          onClick={() => setSubTab('settings')}
        >
          <Settings size={15} />
          <span>切片與 Embedding 設定</span>
        </button>
      </div>

      {/* 1. 知識庫管理 */}
      {subTab === 'kbs' && (
        <div className="rag-workspace">
          {/* 左側：知識庫清單 */}
          <div className="rag-sidebar">
            <div className="rag-sidebar-header">
              <span className="rag-sidebar-title">知識庫清單 ({kbList.length})</span>
              <button
                type="button"
                className="btn btn-primary"
                style={{ padding: '4px 10px', fontSize: 12 }}
                onClick={() => setShowCreateKb((v) => !v)}
              >
                <Plus size={13} />
                <span>新增</span>
              </button>
            </div>

            {showCreateKb && (
              <form className="rag-create-form" onSubmit={handleCreateKb}>
                <input
                  type="text"
                  className="form-input"
                  placeholder="知識庫名稱 (例如：專案文件)"
                  value={newKbName}
                  onChange={(e) => setNewKbName(e.target.value)}
                  autoFocus
                  required
                />
                <input
                  type="text"
                  className="form-input"
                  placeholder="說明 (選填)"
                  value={newKbDesc}
                  onChange={(e) => setNewKbDesc(e.target.value)}
                />
                <div style={{ display: 'flex', gap: 6, justifyContent: 'flex-end' }}>
                  <button
                    type="button"
                    className="btn btn-secondary"
                    style={{ padding: '4px 10px', fontSize: 12 }}
                    onClick={() => setShowCreateKb(false)}
                  >
                    取消
                  </button>
                  <button
                    type="submit"
                    className="btn btn-emerald"
                    style={{ padding: '4px 10px', fontSize: 12 }}
                    disabled={creatingKb || !newKbName.trim()}
                  >
                    {creatingKb ? '建立中...' : '建立'}
                  </button>
                </div>
              </form>
            )}

            <div className="rag-kb-list">
              {kbList.length === 0 ? (
                <div className="rag-empty-hint">
                  尚未建立任何知識庫。點擊上方「新增」建立第一個本地知識庫。
                </div>
              ) : (
                kbList.map((kb) => (
                  <div
                    key={kb.id}
                    className={`rag-kb-item ${selectedKbId === kb.id ? 'active' : ''}`}
                    onClick={() => setSelectedKbId(kb.id)}
                  >
                    <div className="rag-kb-item-top">
                      <span className="rag-kb-name">{kb.name}</span>
                      <button
                        type="button"
                        className="rag-icon-danger"
                        title="刪除知識庫"
                        onClick={(e) => {
                          e.stopPropagation();
                          handleDeleteKb(kb);
                        }}
                      >
                        <Trash2 size={13} />
                      </button>
                    </div>
                    {kb.description && <div className="rag-kb-desc">{kb.description}</div>}
                    <div className="rag-kb-meta">
                      <span>{kb.doc_count} 文件</span>
                      <span>·</span>
                      <span>{kb.chunk_count} 切片</span>
                      {kb.embedded_chunk_count > 0 && (
                        <>
                          <span>·</span>
                          <span className="rag-embed-pill">
                            向量 {kb.embedded_chunk_count}/{kb.chunk_count}
                          </span>
                        </>
                      )}
                    </div>
                  </div>
                ))
              )}
            </div>
          </div>

          {/* 右側：知識庫內容與文件管理 */}
          <div className="rag-main">
            {!selectedKbId ? (
              <div className="config-card" style={{ margin: 0, textAlign: 'center', padding: 48 }}>
                <BookOpen size={36} color="#64748b" style={{ marginBottom: 12 }} />
                <div style={{ fontSize: 15, fontWeight: 600, marginBottom: 6 }}>
                  開始使用本地 RAG 知識庫
                </div>
                <div style={{ fontSize: 13, color: '#94a3b8', marginBottom: 16 }}>
                  支援 `.txt`、`.md`、`.json`、`.csv` 與各類程式碼檔案。預設使用支援繁簡中文與英文的
                  BM25 檢索，並可選配 `/v1/embeddings` 升級為混合檢索。
                </div>
                <button
                  type="button"
                  className="btn btn-primary"
                  onClick={() => setShowCreateKb(true)}
                >
                  <Plus size={14} />
                  <span>建立第一個知識庫</span>
                </button>
              </div>
            ) : (
              <div className="config-card" style={{ margin: 0, maxWidth: 'none' }}>
                <div className="rag-detail-header">
                  <div>
                    <div className="card-title" style={{ marginBottom: 2 }}>
                      <BookOpen size={17} color="#38bdf8" />
                      <span>{kbDetail?.summary.name || '載入中...'}</span>
                    </div>
                    {kbDetail?.summary.description && (
                      <div style={{ fontSize: 12.5, color: '#94a3b8' }}>
                        {kbDetail.summary.description}
                      </div>
                    )}
                  </div>

                  <div className="rag-toolbar-actions">
                    <button
                      type="button"
                      className="btn btn-primary"
                      disabled={importing}
                      onClick={handlePickFiles}
                    >
                      {importing ? <Loader2 size={14} className="spin-icon" /> : <FileUp size={14} />}
                      <span>匯入檔案</span>
                    </button>
                    <button
                      type="button"
                      className="btn btn-secondary"
                      disabled={importing}
                      onClick={handlePickFolder}
                    >
                      <FolderUp size={14} />
                      <span>匯入資料夾</span>
                    </button>
                    <button
                      type="button"
                      className="btn btn-secondary"
                      disabled={importing}
                      onClick={() => setShowTextForm((v) => !v)}
                    >
                      <FileText size={14} />
                      <span>貼上文字</span>
                    </button>
                    {ragConfig.embedding_enabled && (kbDetail?.summary.chunk_count ?? 0) > 0 && (
                      <button
                        type="button"
                        className="btn btn-emerald"
                        disabled={reindexing}
                        onClick={handleReindexKb}
                        title="使用目前的 Embedding 設定重新計算此知識庫全部切片的向量"
                      >
                        <RefreshCw size={14} className={reindexing ? 'spin-icon' : ''} />
                        <span>{reindexing ? '向量化中...' : '重建向量索引'}</span>
                      </button>
                    )}
                  </div>
                </div>

                {/* 狀態統計條 */}
                {kbDetail && (
                  <div className="rag-stats-bar">
                    <span>文件數：{kbDetail.summary.doc_count}</span>
                    <span>切片總數：{kbDetail.summary.chunk_count}</span>
                    <span>
                      檢索模式：
                      {ragConfig.embedding_enabled
                        ? `混合模式 (向量 ${kbDetail.summary.embedded_chunk_count}/${kbDetail.summary.chunk_count} + BM25)`
                        : 'BM25 關鍵字檢索'}
                    </span>
                    {kbDetail.summary.embedding_model_tag && (
                      <span>向量模型：{kbDetail.summary.embedding_model_tag}</span>
                    )}
                  </div>
                )}

                {modelMismatch && (
                  <div className="rag-banner warning">
                    <AlertCircle size={15} />
                    <span>
                      此知識庫先前使用「{kbDetail?.summary.embedding_model_tag}」建立向量，與目前設定「
                      {ragConfig.embedding_model}」不同，建議點擊右上方「重建向量索引」。
                    </span>
                  </div>
                )}

                {importFeedback && (
                  <div className={`rag-banner ${importFeedback.type}`}>
                    <div style={{ flex: 1 }}>
                      <div style={{ display: 'flex', alignItems: 'center', gap: 6, fontWeight: 600 }}>
                        {importFeedback.type === 'success' ? (
                          <CheckCircle2 size={15} />
                        ) : (
                          <AlertCircle size={15} />
                        )}
                        <span>{importFeedback.message}</span>
                      </div>
                      {importFeedback.details && importFeedback.details.length > 0 && (
                        <ul className="rag-feedback-list">
                          {importFeedback.details.map((d, idx) => (
                            <li key={idx}>{d}</li>
                          ))}
                        </ul>
                      )}
                    </div>
                    <button
                      type="button"
                      className="btn-icon"
                      style={{ background: 'transparent', border: 'none', color: 'inherit', cursor: 'pointer' }}
                      onClick={() => setImportFeedback(null)}
                    >
                      <X size={14} />
                    </button>
                  </div>
                )}

                {/* 貼上文字表單 */}
                {showTextForm && (
                  <form className="rag-text-form" onSubmit={handleAddTextNote}>
                    <div className="form-group">
                      <label className="form-label">文件標題</label>
                      <input
                        type="text"
                        className="form-input"
                        placeholder="例如：部署注意事項 / API 規格筆記"
                        value={noteTitle}
                        onChange={(e) => setNoteTitle(e.target.value)}
                        required
                      />
                    </div>
                    <div className="form-group">
                      <label className="form-label">文字內容</label>
                      <textarea
                        className="form-input"
                        rows={6}
                        style={{ resize: 'vertical', fontFamily: 'var(--font-sans)' }}
                        placeholder="貼上筆記、Markdown 或程式碼內容..."
                        value={noteContent}
                        onChange={(e) => setNoteContent(e.target.value)}
                        required
                      />
                    </div>
                    <div style={{ display: 'flex', gap: 8, justifyContent: 'flex-end' }}>
                      <button
                        type="button"
                        className="btn btn-secondary"
                        onClick={() => setShowTextForm(false)}
                      >
                        取消
                      </button>
                      <button
                        type="submit"
                        className="btn btn-emerald"
                        disabled={importing || !noteTitle.trim() || !noteContent.trim()}
                      >
                        {importing ? '處理中...' : '加入知識庫'}
                      </button>
                    </div>
                  </form>
                )}

                {/* 文件列表 */}
                {loadingDetail ? (
                  <div style={{ padding: 24, color: '#94a3b8', fontSize: 13 }}>正在載入文件...</div>
                ) : !kbDetail || kbDetail.documents.length === 0 ? (
                  <div className="rag-empty-docs">
                    此知識庫目前沒有文件。可點擊上方「匯入檔案」、「匯入資料夾」或「貼上文字」新增資料。
                  </div>
                ) : (
                  <div className="rag-doc-table-wrap">
                    <table className="metrics-table">
                      <thead>
                        <tr>
                          <th>文件名稱</th>
                          <th>格式</th>
                          <th>字元數</th>
                          <th>切片數</th>
                          <th>向量狀態</th>
                          <th style={{ textAlign: 'right' }}>操作</th>
                        </tr>
                      </thead>
                      <tbody>
                        {kbDetail.documents.map((doc) => (
                          <tr key={doc.id}>
                            <td title={doc.source_path || doc.name} style={{ maxWidth: 320 }}>
                              <div style={{ fontWeight: 500, wordBreak: 'break-all' }}>{doc.name}</div>
                            </td>
                            <td>
                              <span className="rag-type-badge">.{doc.file_type}</span>
                            </td>
                            <td className="mono-cell">{doc.char_count.toLocaleString()}</td>
                            <td className="mono-cell">{doc.chunk_count}</td>
                            <td>
                              {doc.embedded_chunk_count === doc.chunk_count && doc.chunk_count > 0 ? (
                                <span className="cache-badge ram">已向量化</span>
                              ) : doc.embedded_chunk_count > 0 ? (
                                <span className="cache-badge disk">
                                  部分 ({doc.embedded_chunk_count}/{doc.chunk_count})
                                </span>
                              ) : (
                                <span className="cache-badge miss">僅 BM25</span>
                              )}
                            </td>
                            <td style={{ textAlign: 'right', whiteSpace: 'nowrap' }}>
                              <button
                                type="button"
                                className="btn btn-secondary"
                                style={{ padding: '3px 8px', fontSize: 11.5, marginRight: 6 }}
                                onClick={() => handlePreviewChunks(doc.id, doc.name)}
                              >
                                <Layers size={12} />
                                <span>檢視切片</span>
                              </button>
                              <button
                                type="button"
                                className="btn btn-secondary"
                                style={{ padding: '3px 8px', fontSize: 11.5, color: '#fb7185' }}
                                onClick={() => handleDeleteDoc(doc.id, doc.name)}
                              >
                                <Trash2 size={12} />
                              </button>
                            </td>
                          </tr>
                        ))}
                      </tbody>
                    </table>
                  </div>
                )}
              </div>
            )}
          </div>
        </div>
      )}

      {/* 2. 檢索測試 */}
      {subTab === 'search' && (
        <div className="config-card">
          <div className="card-title">
            <Search size={18} color="#38bdf8" />
            <span>RAG 檢索測試工作台</span>
          </div>
          <div className="card-subtitle">
            直接測試查詢語句在知識庫中的切片召回效果，檢視 BM25 與向量語意分數分佈。
          </div>

          {kbList.length === 0 ? (
            <div className="rag-empty-docs">請先在「知識庫管理」建立知識庫並匯入文件。</div>
          ) : (
            <>
              <div style={{ marginBottom: 14 }}>
                <div className="form-label" style={{ marginBottom: 6 }}>
                  選擇要檢索的知識庫
                </div>
                <div className="rag-kb-pills">
                  {kbList.map((kb) => {
                    const checked = testKbIds.includes(kb.id);
                    return (
                      <button
                        key={kb.id}
                        type="button"
                        className={`toggle-btn ${checked ? 'active' : ''}`}
                        onClick={() => toggleTestKb(kb.id)}
                      >
                        <BookOpen size={12} />
                        <span>
                          {kb.name} ({kb.chunk_count})
                        </span>
                      </button>
                    );
                  })}
                </div>
              </div>

              <form onSubmit={handleRunSearch} className="path-input-group" style={{ marginBottom: 16 }}>
                <input
                  type="text"
                  className="form-input"
                  style={{ fontFamily: 'var(--font-sans)' }}
                  placeholder="輸入測試問題或關鍵字，例如：如何啟用 HTTPS 自簽憑證？"
                  value={testQuery}
                  onChange={(e) => setTestQuery(e.target.value)}
                />
                <button
                  type="submit"
                  className="btn btn-primary"
                  disabled={searching || !testQuery.trim() || testKbIds.length === 0}
                >
                  {searching ? <Loader2 size={14} className="spin-icon" /> : <Search size={14} />}
                  <span>執行檢索</span>
                </button>
              </form>

              {searchResult && (
                <div>
                  <div className="rag-stats-bar" style={{ marginBottom: 12 }}>
                    <span>命中：{searchResult.hits.length} 筆</span>
                    <span>
                      模式：{searchResult.used_embedding ? '混合檢索 (Vector + BM25)' : 'BM25 關鍵字'}
                    </span>
                    <span>耗時：{searchResult.elapsed_ms} ms</span>
                  </div>

                  {searchResult.warning && (
                    <div className="rag-banner warning" style={{ marginBottom: 12 }}>
                      <AlertCircle size={15} />
                      <span>{searchResult.warning}</span>
                    </div>
                  )}

                  {searchResult.hits.length === 0 ? (
                    <div className="rag-empty-docs">沒有找到符合門檻的相關切片。</div>
                  ) : (
                    <div className="rag-hits-list">
                      {searchResult.hits.map((hit, idx) => (
                        <div key={hit.chunk_id} className="rag-hit-card">
                          <div className="rag-hit-header">
                            <span className="rag-hit-title">
                              [{idx + 1}] {hit.doc_name} · {hit.kb_name} (#{hit.chunk_index + 1})
                            </span>
                            <div className="rag-hit-scores">
                              <span className="cache-badge ram">
                                綜合 {(hit.score * 100).toFixed(1)}%
                              </span>
                              {hit.vector_score !== null && (
                                <span className="cache-badge disk">
                                  語意 {(hit.vector_score * 100).toFixed(1)}%
                                </span>
                              )}
                              <span className="rag-type-badge">BM25 {hit.bm25_score.toFixed(2)}</span>
                            </div>
                          </div>
                          <pre className="rag-hit-content">{hit.content}</pre>
                        </div>
                      ))}
                    </div>
                  )}
                </div>
              )}
            </>
          )}
        </div>
      )}

      {/* 3. 切片與 Embedding 設定 */}
      {subTab === 'settings' && (
        <form onSubmit={handleSaveSettings}>
          <div className="config-card">
            <div className="card-title">
              <Layers size={18} color="#38bdf8" />
              <span>文本切片與召回參數</span>
            </div>
            <div className="card-subtitle">
              調整文件匯入時的切片長度、重疊字元數，以及對話時注入模型的參考片段上限。
            </div>

            <div className="form-grid">
              <div className="form-group">
                <label className="form-label">切片長度 (Chunk Size / 字元)</label>
                <input
                  type="number"
                  className="form-input"
                  min={150}
                  max={8000}
                  value={cfgForm.chunk_size}
                  onChange={(e) =>
                    setCfgForm((p) => ({ ...p, chunk_size: Number(e.target.value) || 800 }))
                  }
                />
                <span style={{ fontSize: 11.5, color: '#64748b' }}>
                  建議 600–1200 字元，會自動優先於段落與句末邊界切割
                </span>
              </div>

              <div className="form-group">
                <label className="form-label">重疊長度 (Chunk Overlap / 字元)</label>
                <input
                  type="number"
                  className="form-input"
                  min={0}
                  max={4000}
                  value={cfgForm.chunk_overlap}
                  onChange={(e) =>
                    setCfgForm((p) => ({ ...p, chunk_overlap: Number(e.target.value) || 0 }))
                  }
                />
                <span style={{ fontSize: 11.5, color: '#64748b' }}>
                  相鄰切片保留的重疊字元數，避免語意被切斷（上限為切片長度一半）
                </span>
              </div>

              <div className="form-group">
                <label className="form-label">預設召回筆數 (Top-K)</label>
                <input
                  type="number"
                  className="form-input"
                  min={1}
                  max={30}
                  value={cfgForm.top_k}
                  onChange={(e) =>
                    setCfgForm((p) => ({ ...p, top_k: Number(e.target.value) || 5 }))
                  }
                />
                <span style={{ fontSize: 11.5, color: '#64748b' }}>
                  每次對話最多從知識庫檢索並注入的相關切片數
                </span>
              </div>

              <div className="form-group">
                <label className="form-label">最低分數門檻 (Min Score: 0.0–1.0)</label>
                <input
                  type="number"
                  step="0.05"
                  className="form-input"
                  min={0}
                  max={1}
                  value={cfgForm.min_score}
                  onChange={(e) =>
                    setCfgForm((p) => ({ ...p, min_score: parseFloat(e.target.value) || 0 }))
                  }
                />
                <span style={{ fontSize: 11.5, color: '#64748b' }}>
                  低於此綜合分數的切片將被過濾，避免無關雜訊干擾回答
                </span>
              </div>
            </div>
          </div>

          <div className="config-card">
            <div className="card-title">
              <Sparkles size={18} color="#c084fc" />
              <span>向量 Embedding 與混合檢索（選填）</span>
            </div>
            <div className="card-subtitle">
              未啟用時會使用純本地 BM25（支援中文 unigram/bigram 與英文關鍵字）；若另有啟動 Gufo /
              Ollama 等 OpenAI 相容 <code>/v1/embeddings</code> 服務，可開啟此項目升級為語意＋關鍵字混合檢索。
            </div>

            <div style={{ marginBottom: 16 }}>
              <label style={{ display: 'flex', alignItems: 'center', gap: 8, cursor: 'pointer' }}>
                <input
                  type="checkbox"
                  checked={cfgForm.embedding_enabled}
                  onChange={(e) =>
                    setCfgForm((p) => ({ ...p, embedding_enabled: e.target.checked }))
                  }
                />
                <span style={{ fontWeight: 500 }}>啟用向量 Embedding 混合檢索</span>
              </label>
            </div>

            <div className="form-grid">
              <div className="form-group">
                <label className="form-label">Embedding API 端點</label>
                <input
                  type="text"
                  className="form-input"
                  placeholder="http://127.0.0.1:12224/v1"
                  value={cfgForm.embedding_endpoint}
                  disabled={!cfgForm.embedding_enabled}
                  onChange={(e) =>
                    setCfgForm((p) => ({ ...p, embedding_endpoint: e.target.value }))
                  }
                />
                <span style={{ fontSize: 11.5, color: '#64748b' }}>
                  自動正規化為 <code>/v1/embeddings</code>
                </span>
              </div>

              <div className="form-group">
                <label className="form-label">Embedding 模型名稱</label>
                <input
                  type="text"
                  className="form-input"
                  placeholder="例如：embeddinggemma / bge-m3 / nomic-embed-text"
                  value={cfgForm.embedding_model}
                  disabled={!cfgForm.embedding_enabled}
                  onChange={(e) =>
                    setCfgForm((p) => ({ ...p, embedding_model: e.target.value }))
                  }
                />
              </div>

              <div className="form-group">
                <label className="form-label">API Key (選填)</label>
                <div className="path-input-group">
                  <input
                    type={showEmbedKey ? 'text' : 'password'}
                    className="form-input"
                    placeholder="若無驗證可留空"
                    value={cfgForm.embedding_api_key}
                    disabled={!cfgForm.embedding_enabled}
                    onChange={(e) =>
                      setCfgForm((p) => ({ ...p, embedding_api_key: e.target.value }))
                    }
                  />
                  <button
                    type="button"
                    className="btn btn-secondary"
                    onClick={() => setShowEmbedKey((v) => !v)}
                    title={showEmbedKey ? '隱藏' : '顯示'}
                  >
                    {showEmbedKey ? <EyeOff size={14} /> : <Eye size={14} />}
                  </button>
                </div>
              </div>

              <div className="form-group">
                <label className="form-label">
                  <span>混合權重 (Hybrid Alpha)</span>
                  <span style={{ fontFamily: 'var(--font-mono)', color: '#38bdf8' }}>
                    向量 {Math.round(cfgForm.hybrid_alpha * 100)}% / BM25{' '}
                    {Math.round((1 - cfgForm.hybrid_alpha) * 100)}%
                  </span>
                </label>
                <input
                  type="range"
                  min={0}
                  max={1}
                  step={0.05}
                  value={cfgForm.hybrid_alpha}
                  disabled={!cfgForm.embedding_enabled}
                  onChange={(e) =>
                    setCfgForm((p) => ({ ...p, hybrid_alpha: parseFloat(e.target.value) }))
                  }
                />
                <span style={{ fontSize: 11.5, color: '#64748b' }}>
                  0% 為純 BM25 關鍵字，100% 為純向量語意相似度（建議 60%）
                </span>
              </div>
            </div>

            {cfgForm.embedding_enabled && (
              <div style={{ marginTop: 14, display: 'flex', alignItems: 'center', gap: 12, flexWrap: 'wrap' }}>
                <button
                  type="button"
                  className="btn btn-secondary"
                  disabled={testingEmbed}
                  onClick={handleTestEmbedding}
                >
                  {testingEmbed ? <Loader2 size={14} className="spin-icon" /> : <Zap size={14} />}
                  <span>測試 Embedding 連線</span>
                </button>
                {embedTestResult && (
                  <span
                    style={{
                      fontSize: 12.5,
                      color: embedTestResult.ok ? '#34d399' : '#fb7185',
                    }}
                  >
                    {embedTestResult.message}
                  </span>
                )}
              </div>
            )}
          </div>

          <div className="config-card">
            <div className="card-title">
              <FileText size={18} color="#10b981" />
              <span>RAG 提示詞模板</span>
            </div>
            <div className="card-subtitle">
              請保留 <code>{'{{context}}'}</code> 佔位符，系統會於對話送出前自動填入檢索到的參考片段。
            </div>
            <div className="form-group">
              <textarea
                className="form-input"
                rows={5}
                style={{ resize: 'vertical', fontFamily: 'var(--font-sans)' }}
                value={cfgForm.system_prompt_template}
                onChange={(e) =>
                  setCfgForm((p) => ({ ...p, system_prompt_template: e.target.value }))
                }
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
                  setCfgForm((p) => ({ ...p, system_prompt_template: DEFAULT_TEMPLATE }))
                }
              >
                還原預設模板
              </button>

              <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
                {savedCfgOk && (
                  <span style={{ color: '#34d399', fontSize: 13 }}>✅ 設定已儲存</span>
                )}
                <button type="submit" className="btn btn-primary" disabled={savingCfg}>
                  <Save size={14} />
                  <span>{savingCfg ? '儲存中...' : '儲存 RAG 設定'}</span>
                </button>
              </div>
            </div>
          </div>
        </form>
      )}

      {/* 切片預覽 Modal */}
      {previewDocName && (
        <div className="rag-modal-backdrop" onClick={() => setPreviewDocName(null)}>
          <div className="rag-modal" onClick={(e) => e.stopPropagation()}>
            <div className="rag-modal-header">
              <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                <Layers size={16} color="#38bdf8" />
                <span>切片預覽：{previewDocName} ({previewChunks.length} 個切片)</span>
              </div>
              <button
                type="button"
                className="btn-icon"
                style={{ background: 'transparent', border: 'none', color: '#94a3b8', cursor: 'pointer' }}
                onClick={() => setPreviewDocName(null)}
              >
                <X size={16} />
              </button>
            </div>
            <div className="rag-modal-body">
              {previewChunks.map((c) => (
                <div key={c.id} className="rag-hit-card">
                  <div className="rag-hit-header">
                    <span className="rag-hit-title">切片 #{c.chunk_index + 1}</span>
                    <div className="rag-hit-scores">
                      <span className="rag-type-badge">{c.char_count} 字元</span>
                      {c.has_embedding ? (
                        <span className="cache-badge ram">已向量化</span>
                      ) : (
                        <span className="cache-badge miss">僅 BM25</span>
                      )}
                    </div>
                  </div>
                  <pre className="rag-hit-content">{c.content}</pre>
                </div>
              ))}
            </div>
          </div>
        </div>
      )}
    </div>
  );
};
