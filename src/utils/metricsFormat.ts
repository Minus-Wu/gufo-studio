export function formatTokenCount(n: number): string {
  if (!Number.isFinite(n) || n < 0) return '0';
  if (n < 1000) return `${Math.round(n)}`;
  const decimals = n >= 10000 ? 1 : 2;
  return `${Number((n / 1000).toFixed(decimals))}k`;
}

export function formatCacheSourceLabel(cache?: string): 'RAM' | 'Disk' | 'Miss' {
  if (cache === 'disk') return 'Disk';
  if (cache === 'memory') return 'RAM';
  return 'Miss';
}

export function formatPayloadSize(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return '0 B';
  if (bytes >= 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  if (bytes >= 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${Math.round(bytes)} B`;
}

export function describeCacheMissReason(
  reason?: string,
  commonPrefixTokens?: number,
  nearestCheckpointTokens?: number
): string {
  const reasonLabel =
    reason === 'prefix_changed'
      ? '前綴變更 / Rewind 分叉'
      : reason === 'input_changed'
      ? '圖片或輸入識別碼變更'
      : reason === 'no_checkpoint'
      ? '尚無可用快照'
      : reason === 'disabled'
      ? '請求停用快取 (cache_prompt=false)'
      : reason || '未命中';

  const details: string[] = [];
  if (commonPrefixTokens !== undefined) {
    details.push(`共同前綴 ${commonPrefixTokens.toLocaleString()} tok`);
  }
  if (nearestCheckpointTokens !== undefined && nearestCheckpointTokens > 0) {
    details.push(`最近快照 ${nearestCheckpointTokens.toLocaleString()} tok`);
  }
  return details.length > 0 ? `${reasonLabel}（${details.join(' / ')}）` : reasonLabel;
}
