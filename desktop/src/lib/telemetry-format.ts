export function finiteNumber(value: unknown): value is number { return typeof value === 'number' && Number.isFinite(value); }
export function gib(value: number | null) { return finiteNumber(value) ? `${value.toFixed(1)} GiB` : '—'; }
export function gibBytes(value: number | null) { return finiteNumber(value) ? gib(value / 1024 ** 3) : '—'; }
export function percent(value: number | null) { return finiteNumber(value) ? `${value.toFixed(0)}%` : '—'; }
export function ramUsed(total: number | null, available: number | null) {
  if (!finiteNumber(total) || !finiteNumber(available) || total <= 0) return '—';
  return `${Math.max(0, Math.min(100, ((total - available) / total) * 100)).toFixed(0)}% used`;
}
export function byteRate(value: number | null) {
  if (!finiteNumber(value)) return '—';
  if (value < 1024) return `${value.toFixed(0)} B/s`;
  if (value < 1024 * 1024) return `${(value / 1024).toFixed(1)} KiB/s`;
  return `${(value / 1024 / 1024).toFixed(1)} MiB/s`;
}
