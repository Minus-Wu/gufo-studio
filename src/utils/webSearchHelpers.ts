import { openUrl } from '@tauri-apps/plugin-opener';
import { WebSearchConfig, WebSearchHit, WebSearchProvider } from '../types';

export const DEFAULT_WEB_PROMPT_TEMPLATE =
  '目前日期為 {{date}}。請優先依據以下「網路搜尋結果」中的最新資訊回答使用者的問題。回答時請盡量附上引用來源編號（例如 [W1]、[W2]）與重點事實；若搜尋結果無法涵蓋問題或彼此矛盾，請誠實說明，不要編造事實。\n\n【網路搜尋結果】\n{{results}}';

export const DEFAULT_WEB_SEARCH_CONFIG: WebSearchConfig = {
  provider: 'auto',
  api_key: '',
  searxng_url: 'http://127.0.0.1:8080',
  max_results: 5,
  fetch_page_content: true,
  fetch_top_n: 3,
  max_page_chars: 1500,
  timeout_secs: 10,
  region: '',
  prompt_template: DEFAULT_WEB_PROMPT_TEMPLATE,
};

export const WEB_SEARCH_PROVIDERS: {
  value: WebSearchProvider;
  label: string;
  needsApiKey: boolean;
  needsSearxngUrl: boolean;
  hint: string;
}[] = [
  {
    value: 'auto',
    label: '自動免 Key（DuckDuckGo → Bing RSS 備援）',
    needsApiKey: false,
    needsSearxngUrl: false,
    hint: '零成本免註冊，優先使用 DuckDuckGo HTML，遇到限制或無結果時自動切換為 Bing RSS。',
  },
  {
    value: 'duckduckgo',
    label: 'DuckDuckGo（免 API Key）',
    needsApiKey: false,
    needsSearxngUrl: false,
    hint: '直接查詢 DuckDuckGo HTML 端點，隱私友善但高頻查詢時可能受限。',
  },
  {
    value: 'bing_rss',
    label: 'Bing RSS（免 API Key）',
    needsApiKey: false,
    needsSearxngUrl: false,
    hint: '透過 Bing 公開 RSS 訂閱源取得搜尋結果，穩定度高、免註冊。',
  },
  {
    value: 'searxng',
    label: 'SearXNG 自架實例（免 Key）',
    needsApiKey: false,
    needsSearxngUrl: true,
    hint: '連線至本機或區域網路自架的 SearXNG JSON API（需於 SearXNG 啟用 json format）。',
  },
  {
    value: 'tavily',
    label: 'Tavily Search API（需 API Key）',
    needsApiKey: true,
    needsSearxngUrl: false,
    hint: '專為 LLM / AI RAG 設計的搜尋 API，提供高品質摘要與乾淨內文。',
  },
  {
    value: 'brave',
    label: 'Brave Search API（需 API Key）',
    needsApiKey: true,
    needsSearxngUrl: false,
    hint: '獨立索引搜尋引擎，支援 X-Subscription-Token 驗證。',
  },
  {
    value: 'serper',
    label: 'Serper.dev Google API（需 API Key）',
    needsApiKey: true,
    needsSearxngUrl: false,
    hint: '透過 Serper API 取得 Google 搜尋即時結果。',
  },
];

export function formatWebProviderLabel(provider: string): string {
  switch (provider) {
    case 'duckduckgo':
      return 'DuckDuckGo';
    case 'bing_rss':
      return 'Bing RSS';
    case 'searxng':
      return 'SearXNG';
    case 'tavily':
      return 'Tavily';
    case 'brave':
      return 'Brave';
    case 'serper':
      return 'Serper (Google)';
    case 'auto':
      return '自動免 Key';
    default:
      return provider;
  }
}

export async function openSafeExternalUrl(rawUrl: string): Promise<void> {
  const trimmed = rawUrl.trim();
  if (!trimmed.startsWith('http://') && !trimmed.startsWith('https://')) {
    return;
  }
  try {
    await openUrl(trimmed);
  } catch {
    window.open(trimmed, '_blank', 'noopener,noreferrer');
  }
}

export function formatLocalDate(): string {
  const now = new Date();
  const y = now.getFullYear();
  const m = String(now.getMonth() + 1).padStart(2, '0');
  const d = String(now.getDate()).padStart(2, '0');
  return `${y}-${m}-${d}`;
}

export function buildWebSearchPromptBlock(
  hits: WebSearchHit[],
  template?: string
): string {
  const resultsText = hits
    .map((hit, idx) => {
      const lines = [
        `[W${idx + 1}] 標題：${hit.title}`,
        `網址：${hit.url}`,
      ];
      if (hit.published_date) {
        lines.push(`日期：${hit.published_date}`);
      }
      if (hit.snippet) {
        lines.push(`摘要：${hit.snippet}`);
      }
      if (hit.fetched_content) {
        lines.push(`網頁正文摘錄：\n${hit.fetched_content}`);
      }
      return lines.join('\n');
    })
    .join('\n\n---\n\n');

  const tpl = template && template.trim() ? template : DEFAULT_WEB_PROMPT_TEMPLATE;
  const dateStr = formatLocalDate();
  let filled = tpl.split('{{date}}').join(dateStr);
  if (filled.includes('{{results}}')) {
    filled = filled.split('{{results}}').join(resultsText);
  } else {
    filled = `${filled}\n\n${resultsText}`;
  }
  return filled;
}
