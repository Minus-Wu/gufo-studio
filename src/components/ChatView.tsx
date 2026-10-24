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
import { ChatMessage, MessageImage, ServerStatus } from '../types';

interface ChatViewProps {
  status: ServerStatus;
  onStartServer?: () => void;
}

export const ChatView: React.FC<ChatViewProps> = ({ status }) => {
  const [messages, setMessages] = useState<ChatMessage[]>([
    {
      id: 'welcome',
      role: 'assistant',
      content:
        '你好！我是 **Gufo Studio**，本機以 AMD Strix Halo (Ryzen AI MAX+ 395) 硬體深度優化推論。\n\n目前預設支援 **Qwen3.8-Flash-Next (128K Vision)** 投機加速推論。你可以隨時傳送訊息、貼上圖片截圖，或是使用上方控制列調整思考模式！',
      timestamp: Date.now(),
    },
  ]);

  const [input, setInput] = useState('');
  const [images, setImages] = useState<MessageImage[]>([]);
  const [isGenerating, setIsGenerating] = useState(false);
  const [thinkMode, setThinkMode] = useState(false);
  const [temperature, setTemperature] = useState(0.0);
  const [copiedId, setCopiedId] = useState<string | null>(null);

  const messagesEndRef = useRef<HTMLDivElement>(null);
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);
  const abortControllerRef = useRef<AbortController | null>(null);

  const scrollToBottom = () => {
    messagesEndRef.current?.scrollIntoView({ behavior: 'smooth' });
  };

  useEffect(() => {
    scrollToBottom();
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
    setMessages(newMessages);
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

    setMessages((prev) => [...prev, assistantMessage]);

    // 格式化 OpenAI 對話歷史
    const apiMessages = newMessages.map((msg) => {
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
      return { role: msg.role, content: msg.content };
    });

    const abortController = new AbortController();
    abortControllerRef.current = abortController;

    const startTime = performance.now();
    let firstTokenTime: number | null = null;
    let accumulatedContent = '';
    let accumulatedReasoning = '';
    let tokenCount = 0;

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
          temperature: temperature,
          extra_body: {
            think: thinkMode ? 'on' : 'off',
          },
        }),
        signal: abortController.signal,
      });

      if (!response.ok) {
        throw new Error(`HTTP 錯誤: ${response.status} ${response.statusText}`);
      }

      const reader = response.body?.getReader();
      const decoder = new TextDecoder('utf-8');

      if (!reader) throw new Error('無法取得回傳流');

      let buffer = '';

      while (true) {
        const { done, value } = await reader.read();
        if (done) break;

        buffer += decoder.decode(value, { stream: true });
        const lines = buffer.split('\n');
        buffer = lines.pop() || '';

        for (const line of lines) {
          const trimmed = line.trim();
          if (!trimmed || !trimmed.startsWith('data: ')) continue;
          if (trimmed === 'data: [DONE]') break;

          try {
            const json = JSON.parse(trimmed.slice(6));
            const delta = json.choices?.[0]?.delta;
            if (!delta) continue;

            if (firstTokenTime === null) {
              firstTokenTime = performance.now();
            }

            // 支援思考鏈標籤或 reasoning_content
            if (delta.reasoning_content) {
              accumulatedReasoning += delta.reasoning_content;
            } else if (delta.content) {
              accumulatedContent += delta.content;
              tokenCount++;
            }

            const now = performance.now();
            const elapsedSec = (now - (firstTokenTime || startTime)) / 1000;
            const tps = elapsedSec > 0 ? Math.round((tokenCount / elapsedSec) * 10) / 10 : 0;
            const ttft = firstTokenTime ? Math.round(firstTokenTime - startTime) : undefined;

            setMessages((prev) =>
              prev.map((m) =>
                m.id === assistantMsgId
                  ? {
                      ...m,
                      content: accumulatedContent,
                      reasoning: accumulatedReasoning || undefined,
                      metrics: {
                        totalTokens: tokenCount,
                        tokensPerSec: tps,
                        ttftMs: ttft,
                      },
                    }
                  : m
              )
            );
          } catch (e) {
            // 忽略單行 JSON 解析錯誤
          }
        }
      }
    } catch (err: any) {
      if (err.name !== 'AbortError') {
        setMessages((prev) =>
          prev.map((m) =>
            m.id === assistantMsgId
              ? {
                  ...m,
                  content:
                    accumulatedContent ||
                    `⚠️ 連線請求失敗：${err.message || '請確認伺服器狀態與埠號是否正確'}`,
                }
              : m
          )
        );
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

  return (
    <div className="chat-container">
      {/* 訊息流列表 */}
      <div className="chat-messages">
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
                {msg.metrics && msg.metrics.tokensPerSec !== undefined && (
                  <div className="message-metrics">
                    <span style={{ display: 'flex', alignContent: 'center', gap: 3 }}>
                      <Zap size={11} color="#06b6d4" />
                      {msg.metrics.tokensPerSec} tokens/s
                    </span>
                    {msg.metrics.ttftMs && (
                      <span style={{ display: 'flex', alignContent: 'center', gap: 3 }}>
                        <Clock size={11} color="#f59e0b" />
                        首字延遲 {msg.metrics.ttftMs}ms
                      </span>
                    )}
                    <span>共 {msg.metrics.totalTokens} tokens</span>
                  </div>
                )}
              </div>
            </div>
          </div>
        ))}
        <div ref={messagesEndRef} />
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
