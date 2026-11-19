import React, { useState, useRef, useEffect } from 'react';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import {
  Send,
  Image as ImageIcon,
  Brain,
  Trash2,
  Copy,
  Check,
  RotateCw,
  Sparkles,
  Zap,
  Clock,
  X,
} from 'lucide-react';
import { ChatMessage, ChatMetrics, MessageImage, ServerStatus } from '../types';
import { describeCacheMissReason, formatCacheSourceLabel } from '../utils/metricsFormat';

interface ChatViewProps {
  status: ServerStatus;
  /** 對話預設思考模式 (來自伺服器設定的 --think) */
  defaultThink?: boolean;
  /** 對話預設採樣溫度 (來自伺服器設定的 --temperature) */
  defaultTemperature?: number;
}

const round1 = (n: number) => Math.round(n * 10) / 10;
const num = (v: unknown): number | undefined =>
  typeof v === 'number' && Number.isFinite(v) ? v : undefined;

/** 合併 Gufo 串流終端 chunk 的 timings 與 include_usage 的 usage，轉成伺服器端精確指標 */
function buildServerMetrics(
  timings: any,
  usage: any,
  clientTtftMs: number | undefined
): ChatMetrics {
  const t = timings ?? {};
  const u = usage ?? {};
  const g = u.gufo ?? {};
  const prefillTokens = num(t.prompt_n) ?? num(g.prefill_tokens);
  const cachedTokens =
    num(u.prompt_tokens_details?.cached_tokens) ?? num(u.cached_tokens) ?? num(t.cache_n);
  const promptTokens =
    num(u.prompt_tokens) ??
    (prefillTokens !== undefined ? prefillTokens + (cachedTokens ?? 0) : undefined);
  const decodeTps = num(t.predicted_per_second) ?? num(u.completion_tokens_per_second);
  const prefillTps = num(t.prompt_per_second) ?? num(u.prompt_tokens_per_second);
  const cacheDiskHit = g.cache_disk_hit === true;
  const cacheHit = g.cache_hit === true || (cachedTokens !== undefined && cachedTokens > 0);
  const cacheSource: ChatMetrics['cacheSource'] = cacheDiskHit
    ? 'disk'
    : cacheHit
    ? 'memory'
    : 'miss';
  const cacheHitPct =
    promptTokens !== undefined && promptTokens > 0 && cachedTokens !== undefined
      ? Math.min(100, round1((cachedTokens * 100) / promptTokens))
      : undefined;

  return {
    approximate: false,
    completionTokens: num(u.completion_tokens) ?? num(t.predicted_n),
    tokensPerSec: decodeTps !== undefined ? round1(decodeTps) : undefined,
    ttftMs: num(g.ttft_ms) ?? clientTtftMs,
    promptTokens,
    prefillTokens,
    prefillTps: prefillTps !== undefined && (prefillTokens ?? 1) > 0 ? round1(prefillTps) : undefined,
    prefillMs: num(t.prompt_ms) ?? num(g.prefill_ms),
    cachedTokens,
    cacheSource,
    cacheHitPct,
    cacheRestoreMs: num(t.cache_restore_ms) ?? num(g.cache_restore_ms),
    cacheMissReason: typeof g.cache_miss_reason === 'string' ? g.cache_miss_reason : undefined,
    commonPrefixTokens: num(g.cache_common_prefix_tokens),
    nearestCheckpointTokens: num(g.cache_checkpoint_tokens),
    draftTokens: num(t.draft_n) ?? num(u.draft_tokens),
    draftAccepted: num(t.draft_n_accepted) ?? num(u.draft_tokens_accepted),
    lookupTokens: num(t.lookup_n),
    lookupAccepted: num(t.lookup_n_accepted),
  };
}

export const ChatView: React.FC<ChatViewProps> = ({
  status,
  defaultThink = false,
  defaultTemperature = 0,
}) => {
  const [messages, setMessages] = useState<ChatMessage[]>([
    {
      id: 'welcome',
      role: 'assistant',
      content:
        '你好！我是 **Gufo Studio**，本機以 AMD Strix Halo (Ryzen AI MAX+ 395) 硬體深度優化推論。\n\n目前預設支援 **Qwen3.8-Flash-Next (128K Vision)** 投機加速推論。你可以隨時傳送訊息、貼上圖片截圖，或是使用上方控制列調整思考模式！',
      timestamp: Date.now(),
      excludeFromContext: true,
    },
  ]);

  const [input, setInput] = useState('');
  const [images, setImages] = useState<MessageImage[]>([]);
  const [isGenerating, setIsGenerating] = useState(false);
  const [thinkMode, setThinkMode] = useState(defaultThink);
  const [temperature, setTemperature] = useState(defaultTemperature);
  const [copiedId, setCopiedId] = useState<string | null>(null);

  const messagesContainerRef = useRef<HTMLDivElement>(null);
  const stickToBottomRef = useRef(true);
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);
  const abortControllerRef = useRef<AbortController | null>(null);

  // 伺服器設定變更時同步對話預設值
  useEffect(() => {
    setThinkMode(defaultThink);
  }, [defaultThink]);

  useEffect(() => {
    setTemperature(defaultTemperature);
  }, [defaultTemperature]);

  const handleMessagesScroll = () => {
    const el = messagesContainerRef.current;
    if (!el) return;
    stickToBottomRef.current = el.scrollHeight - el.scrollTop - el.clientHeight < 80;
  };

  // 只在使用者停留在底部時自動捲動，避免串流時搶走捲軸
  useEffect(() => {
    const el = messagesContainerRef.current;
    if (el && stickToBottomRef.current) {
      el.scrollTop = el.scrollHeight;
    }
  }, [messages]);

  const handleCopy = (id: string, text: string) => {
    navigator.clipboard.writeText(text);
    setCopiedId(id);
    setTimeout(() => setCopiedId(null), 2000);
  };

  const handleClearHistory = () => {
    if (confirm('確定要清空對話紀錄嗎？')) {
      setMessages([]);
    }
  };

  const handleImageUpload = (file: File) => {
    const reader = new FileReader();
    reader.onload = (e) => {
      if (e.target?.result) {
        setImages((prev) => [
          ...prev,
          { url: e.target!.result as string, name: file.name },
        ]);
      }
    };
    reader.readAsDataURL(file);
  };

  const handlePaste = (e: React.ClipboardEvent<HTMLTextAreaElement>) => {
    if (e.clipboardData && e.clipboardData.items) {
      for (let i = 0; i < e.clipboardData.items.length; i++) {
        const item = e.clipboardData.items[i];
        if (item.type.indexOf('image') !== -1) {
          const file = item.getAsFile();
          if (file) {
            handleImageUpload(file);
          }
        }
      }
    }
  };

  const removeImage = (index: number) => {
    setImages((prev) => prev.filter((_, i) => i !== index));
  };

  const handleSend = async () => {
    if ((!input.trim() && images.length === 0) || isGenerating || status.state !== 'running') {
      return;
    }

    const userMessageId = `msg-${Date.now()}`;
    const userMessage: ChatMessage = {
      id: userMessageId,
      role: 'user',
      content: input.trim(),
      images: images.length > 0 ? [...images] : undefined,
      timestamp: Date.now(),
    };

    const newMessages = [...messages, userMessage];
    stickToBottomRef.current = true;
    setInput('');
    setImages([]);
    setIsGenerating(true);

    const assistantMsgId = `asst-${Date.now()}`;
    const assistantMessage: ChatMessage = {
      id: assistantMsgId,
      role: 'assistant',
      content: '',
      timestamp: Date.now(),
    };

    setMessages([...newMessages, assistantMessage]);

    // 格式化 OpenAI 對話歷史 (排除歡迎訊息、錯誤提示與空白回覆)
    const apiMessages = newMessages
      .filter(
        (msg) =>
          !msg.excludeFromContext &&
          !(msg.role === 'assistant' && !msg.content.trim())
      )
      .map((msg) => {
        if (msg.images && msg.images.length > 0) {
          const contentParts: any[] = [];
          if (msg.content) {
            contentParts.push({ type: 'text', text: msg.content });
          }
          for (const img of msg.images) {
            contentParts.push({
              type: 'image_url',
              image_url: { url: img.url },
            });
          }
          return { role: msg.role, content: contentParts };
        }
        if (msg.role === 'assistant' && msg.reasoning) {
          return { role: msg.role, content: msg.content, reasoning_content: msg.reasoning };
        }
        return { role: msg.role, content: msg.content };
      });

    const abortController = new AbortController();
    abortControllerRef.current = abortController;

    const startTime = performance.now();
    let firstTokenTime: number | null = null;
    let accumulatedContent = '';
    let accumulatedReasoning = '';
    let chunkCount = 0;
    let finalTimings: any = null;
    let finalUsage: any = null;
    let frameHandle: number | null = null;

    const patchAssistant = (patch: Partial<ChatMessage>) => {
      setMessages((prev) =>
        prev.map((m) => (m.id === assistantMsgId ? { ...m, ...patch } : m))
      );
    };

    // 以 requestAnimationFrame 節流 UI 更新，避免每個 token 都觸發 Markdown 重繪
    const scheduleFlush = () => {
      if (frameHandle !== null) return;
      frameHandle = requestAnimationFrame(() => {
        frameHandle = null;
        const now = performance.now();
        const elapsedSec = firstTokenTime ? (now - firstTokenTime) / 1000 : 0;
        patchAssistant({
          content: accumulatedContent,
          reasoning: accumulatedReasoning || undefined,
          metrics: {
            approximate: true,
            completionTokens: chunkCount,
            tokensPerSec: elapsedSec > 0.2 ? round1(chunkCount / elapsedSec) : undefined,
            ttftMs: firstTokenTime ? Math.round(firstTokenTime - startTime) : undefined,
          },
        });
      });
    };

    const cancelFlush = () => {
      if (frameHandle !== null) {
        cancelAnimationFrame(frameHandle);
        frameHandle = null;
      }
    };

    try {
      const headers: Record<string, string> = {
        'Content-Type': 'application/json',
      };
      if (status.api_key) {
        headers['Authorization'] = `Bearer ${status.api_key}`;
      }

      const response = await fetch(`${status.endpoint}/chat/completions`, {
        method: 'POST',
        headers,
        body: JSON.stringify({
          model: status.served_model_name || 'flash-next',
          messages: apiMessages,
          stream: true,
          stream_options: { include_usage: true },
          temperature: temperature,
          // Gufo 以 chat_template_kwargs.enable_thinking 逐請求控制思考鏈
          chat_template_kwargs: { enable_thinking: thinkMode },
        }),
        signal: abortController.signal,
      });

      if (!response.ok) {
        const bodyText = await response.text().catch(() => '');
        let detail = bodyText.trim();
        try {
          const parsed = JSON.parse(bodyText);
          detail = parsed?.error?.message || parsed?.message || detail;
        } catch {
          // 非 JSON 錯誤內容，保留原文
        }
        throw new Error(
          `HTTP ${response.status} ${response.statusText}${detail ? `：${detail.slice(0, 500)}` : ''}`
        );
      }

      const reader = response.body?.getReader();
      const decoder = new TextDecoder('utf-8');

      if (!reader) throw new Error('無法取得回傳流');

      let buffer = '';
      let streamDone = false;

      while (!streamDone) {
        const { done, value } = await reader.read();
        if (done) break;

        buffer += decoder.decode(value, { stream: true });
        const lines = buffer.split('\n');
        buffer = lines.pop() || '';

        for (const line of lines) {
          const trimmed = line.trim();
          if (!trimmed.startsWith('data:')) continue;
          const payload = trimmed.slice(5).trim();
          if (payload === '[DONE]') {
            streamDone = true;
            break;
          }

          let json: any;
          try {
            json = JSON.parse(payload);
          } catch {
            continue;
          }

          if (json.error) {
            throw new Error(json.error.message || JSON.stringify(json.error));
          }
          // include_usage 的最後一個 chunk 的 choices 為空陣列，需在 delta 判斷之前處理
          if (json.usage) finalUsage = json.usage;
          const choice = json.choices?.[0];
          const timings = json.timings ?? choice?.timings;
          if (timings) finalTimings = timings;

          const delta = choice?.delta;
          if (!delta) continue;

          const reasoningPiece = delta.reasoning_content || delta.reasoning || '';
          const contentPiece = delta.content || '';
          if (!reasoningPiece && !contentPiece) continue;

          if (firstTokenTime === null) {
            firstTokenTime = performance.now();
          }
          accumulatedReasoning += reasoningPiece;
          accumulatedContent += contentPiece;
          chunkCount++;
          scheduleFlush();
        }
      }

      cancelFlush();
      const clientTtft = firstTokenTime ? Math.round(firstTokenTime - startTime) : undefined;
      const hasServerMetrics = !!(finalTimings || finalUsage);
      patchAssistant({
        content: accumulatedContent,
        reasoning: accumulatedReasoning || undefined,
        metrics: hasServerMetrics
          ? buildServerMetrics(finalTimings, finalUsage, clientTtft)
          : { approximate: true, completionTokens: chunkCount, ttftMs: clientTtft },
      });
    } catch (err: any) {
      cancelFlush();
      if (err.name === 'AbortError') {
        patchAssistant({
          content: accumulatedContent,
          reasoning: accumulatedReasoning || undefined,
          excludeFromContext: !accumulatedContent.trim(),
        });
      } else {
        const errorText = `⚠️ 連線請求失敗：${err.message || '請確認伺服器狀態與埠號是否正確'}`;
        patchAssistant({
          content: accumulatedContent ? `${accumulatedContent}\n\n${errorText}` : errorText,
          reasoning: accumulatedReasoning || undefined,
          isError: true,
          excludeFromContext: true,
        });
      }
    } finally {
      setIsGenerating(false);
      abortControllerRef.current = null;
    }
  };

  const handleStopGenerating = () => {
    if (abortControllerRef.current) {
      abortControllerRef.current.abort();
      setIsGenerating(false);
    }
  };

  const renderMetrics = (m: ChatMetrics) => {
    const items: React.ReactNode[] = [];
    if (m.tokensPerSec !== undefined) {
      items.push(
        <span
          key="tps"
          style={{ display: 'flex', alignContent: 'center', gap: 3 }}
          title={
            m.approximate
              ? '串流中前端估算 (以 SSE chunk 數計，非精確 token 數)'
              : '伺服器量測的解碼速度 (decode tok/s)'
          }
        >
          <Zap size={11} color="#06b6d4" />
          {m.approximate ? '≈' : ''}
          {m.tokensPerSec} tok/s
        </span>
      );
    }
    if (m.ttftMs !== undefined) {
      items.push(
        <span key="ttft" style={{ display: 'flex', alignContent: 'center', gap: 3 }}>
          <Clock size={11} color="#f59e0b" />
          首字延遲 {Math.round(m.ttftMs)}ms
        </span>
      );
    }
    if (!m.approximate) {
      if (m.prefillTokens === 0 && (m.cachedTokens ?? 0) > 0) {
        items.push(
          <span key="prefill" title="100% 命中快取，無須重新 Prefill">
            Prefill 0 tok (快取跳過)
          </span>
        );
      } else if (m.prefillTps !== undefined) {
        items.push(
          <span
            key="prefill"
            title={
              m.prefillMs !== undefined
                ? `實際 prefill ${m.prefillTokens ?? '?'} tokens，耗時 ${round1(m.prefillMs)}ms`
                : `實際 prefill ${m.prefillTokens ?? '?'} tokens`
            }
          >
            Prefill {m.prefillTps} tok/s ({m.prefillTokens ?? '?'} tok)
          </span>
        );
      }
    }
    if (m.completionTokens !== undefined) {
      items.push(
        <span key="tokens">
          {m.approximate
            ? `≈ ${m.completionTokens} chunks`
            : `${m.promptTokens !== undefined ? `輸入 ${m.promptTokens} / ` : ''}輸出 ${m.completionTokens} tokens`}
        </span>
      );
    }
    if (!m.approximate && m.cachedTokens && m.cachedTokens > 0) {
      const sourceLabel = formatCacheSourceLabel(m.cacheSource);
      const hitSuffix =
        m.cacheHitPct !== undefined ? ` (${m.cacheHitPct}% · ${sourceLabel})` : ` (${sourceLabel})`;
      items.push(
        <span
          key="cache"
          title={
            m.cacheRestoreMs !== undefined
              ? `快取來源：${sourceLabel}，還原耗時 ${round1(m.cacheRestoreMs)}ms`
              : undefined
          }
        >
          快取命中 {m.cachedTokens}
          {m.promptTokens !== undefined ? `/${m.promptTokens}` : ''}
          {hitSuffix}
        </span>
      );
    } else if (!m.approximate && m.cachedTokens === 0 && m.cacheMissReason) {
      items.push(
        <span
          key="cache"
          title={describeCacheMissReason(
            m.cacheMissReason,
            m.commonPrefixTokens,
            m.nearestCheckpointTokens
          )}
        >
          {m.commonPrefixTokens !== undefined && m.commonPrefixTokens > 0
            ? `快取未命中 (共同前綴 ${m.commonPrefixTokens} tok)`
            : '快取未命中'}
        </span>
      );
    }
    if (!m.approximate && m.draftTokens && m.draftAccepted !== undefined) {
      items.push(
        <span
          key="mtp"
          title={
            `MTP 草稿接受 ${m.draftAccepted}/${m.draftTokens}` +
            (m.lookupTokens && m.lookupTokens > 0
              ? `\nPrompt Lookup 接受 ${m.lookupAccepted ?? 0}/${m.lookupTokens}`
              : '')
          }
        >
          MTP 接受率 {Math.round((m.draftAccepted / m.draftTokens) * 100)}%
        </span>
      );
    }
    return items.length > 0 ? <div className="message-metrics">{items}</div> : null;
  };

  return (
    <div className="chat-container">
      {/* 訊息流列表 */}
      <div className="chat-messages" ref={messagesContainerRef} onScroll={handleMessagesScroll}>
        {messages.map((msg) => (
          <div key={msg.id} className="message-row">
            <div className={`message-avatar ${msg.role}`}>
              {msg.role === 'user' ? '👤' : '🦉'}
            </div>
            <div className="message-content-wrapper">
              <div className="message-header">
                <span>{msg.role === 'user' ? '使用者' : 'Gufo AI'}</span>
                <span>{new Date(msg.timestamp).toLocaleTimeString()}</span>
                <button
                  className="btn-icon"
                  style={{ background: 'transparent', border: 'none', cursor: 'pointer' }}
                  onClick={() => handleCopy(msg.id, msg.content)}
                  title="複製內容"
                >
                  {copiedId === msg.id ? (
                    <Check size={12} color="#10b981" />
                  ) : (
                    <Copy size={12} color="#64748b" />
                  )}
                </button>
              </div>

              <div className={`message-bubble ${msg.role === 'user' ? 'user-bubble' : ''}`}>
                {/* 思考鏈展開區 */}
                {msg.reasoning && (
                  <div className="reasoning-box">
                    <div className="reasoning-title">
                      <Brain size={13} />
                      <span>思考推導過程</span>
                    </div>
                    <div style={{ whiteSpace: 'pre-wrap', fontFamily: 'var(--font-mono)' }}>
                      {msg.reasoning}
                    </div>
                  </div>
                )}

                {/* 圖片展示 */}
                {msg.images && msg.images.length > 0 && (
                  <div className="image-preview-strip" style={{ marginBottom: 10 }}>
                    {msg.images.map((img, idx) => (
                      <div key={idx} className="image-preview-card" style={{ width: 120, height: 120 }}>
                        <img src={img.url} alt={`上傳圖片 ${idx + 1}`} />
                      </div>
                    ))}
                  </div>
                )}

                {/* 文字 Markdown 內容 */}
                <div className="markdown-body">
                  <ReactMarkdown remarkPlugins={[remarkGfm]}>{msg.content}</ReactMarkdown>
                </div>

                {/* 生成速率指標 */}
                {msg.metrics && renderMetrics(msg.metrics)}
              </div>
            </div>
          </div>
        ))}
      </div>

      {/* 輸入區塊 */}
      <div className="chat-input-container">
        {/* 圖片預覽條 */}
        {images.length > 0 && (
          <div className="image-preview-strip">
            {images.map((img, index) => (
              <div key={index} className="image-preview-card">
                <img src={img.url} alt="預覽" />
                <button className="remove-btn" onClick={() => removeImage(index)}>
                  <X size={12} />
                </button>
              </div>
            ))}
          </div>
        )}

        {/* 控制列 */}
        <div className="chat-input-controls">
          <div className="controls-left">
            <button
              className={`toggle-btn ${thinkMode ? 'active' : ''}`}
              onClick={() => setThinkMode(!thinkMode)}
              title="切換思考鏈模式 (Thinking On / Off)"
            >
              <Brain size={14} />
              <span>思考鏈：{thinkMode ? '開啟 (On)' : '關閉 (Off)'}</span>
            </button>

            <button
              className="toggle-btn"
              onClick={() => setTemperature(temperature === 0 ? 0.7 : 0)}
              title="切換解碼模式 (0.0 確定性貪婪解碼 / 0.7 創意取樣)"
            >
              <Sparkles size={14} />
              <span>採樣溫度：{temperature}</span>
            </button>

            <button
              className="toggle-btn"
              onClick={() => fileInputRef.current?.click()}
              title="上傳圖片或截圖"
            >
              <ImageIcon size={14} />
              <span>加入圖片</span>
            </button>
            <input
              type="file"
              ref={fileInputRef}
              style={{ display: 'none' }}
              accept="image/*"
              onChange={(e) => {
                if (e.target.files?.[0]) {
                  handleImageUpload(e.target.files[0]);
                }
              }}
            />
          </div>

          <div>
            <button
              className="btn-icon"
              style={{ background: 'transparent', border: 'none', color: 'var(--text-muted)' }}
              onClick={handleClearHistory}
              title="清空對話"
            >
              <Trash2 size={16} />
            </button>
          </div>
        </div>

        {/* 輸入框 */}
        <div className="chat-input-box">
          <textarea
            ref={textareaRef}
            className="chat-textarea"
            placeholder={
              status.state === 'running'
                ? '輸入訊息... (Shift+Enter 換行，Ctrl+V 可直接貼上圖片截圖)'
                : status.state === 'starting'
                ? '伺服器載入模型中，請稍候...'
                : '伺服器未啟動，請點擊右上角「啟動伺服器」'
            }
            rows={1}
            value={input}
            disabled={status.state !== 'running' && !isGenerating}
            onChange={(e) => setInput(e.target.value)}
            onPaste={handlePaste}
            onKeyDown={(e) => {
              if (e.key === 'Enter' && !e.shiftKey) {
                e.preventDefault();
                if (status.state === 'running') {
                  handleSend();
                }
              }
            }}
          />

          {isGenerating ? (
            <button className="btn btn-danger" onClick={handleStopGenerating} title="停止生成">
              <RotateCw size={14} />
              <span>停止</span>
            </button>
          ) : (
            <button
              className="btn btn-primary"
              onClick={handleSend}
              disabled={status.state !== 'running' || (!input.trim() && images.length === 0)}
              title={
                status.state === 'running'
                  ? '傳送訊息 (Enter)'
                  : status.state === 'starting'
                  ? '伺服器載入模型中，暫時無法傳送'
                  : '伺服器未啟動，請先於右上角啟動伺服器'
              }
            >
              <Send size={15} />
            </button>
          )}
        </div>
      </div>
    </div>
  );
};
