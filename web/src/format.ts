// Date and number formatting for the Ukrainian UI.

const toDate = (v: string | number | Date) => (v instanceof Date ? v : new Date(v));

/** 27 вересня 2026, 10:21 */
export function fmtDateTime(v?: string | number | Date): string {
  if (v === undefined || v === '') return '—';
  const d = toDate(v);
  if (Number.isNaN(d.getTime())) return String(v);
  return d.toLocaleString('uk-UA', { day: 'numeric', month: 'long', year: 'numeric', hour: '2-digit', minute: '2-digit' });
}

/** 10:21 */
export const fmtTime = (v: string | number | Date) =>
  toDate(v).toLocaleTimeString('uk-UA', { hour: '2-digit', minute: '2-digit' });

/** «щойно», «5 хв тому», «вчора», then the date. */
export function fmtAgo(v?: string | number | Date): string {
  if (v === undefined || v === '') return '—';
  const d = toDate(v);
  if (Number.isNaN(d.getTime())) return String(v);
  const min = Math.round((Date.now() - d.getTime()) / 60_000);
  if (min < 1) return 'щойно';
  if (min < 60) return `${min} хв тому`;
  const h = Math.round(min / 60);
  if (h < 24) return `${h} год тому`;
  const days = Math.round(h / 24);
  if (days === 1) return 'вчора';
  if (days < 7) return `${days} дн. тому`;
  return d.toLocaleDateString('uk-UA', { day: 'numeric', month: 'long', year: d.getFullYear() === new Date().getFullYear() ? undefined : 'numeric' });
}

/** Ukrainian plural: plural(5, 'NFT', ...) — one / few / many. */
export function plural(n: number, one: string, few: string, many: string): string {
  const m10 = n % 10, m100 = n % 100;
  if (m10 === 1 && m100 !== 11) return one;
  if (m10 >= 2 && m10 <= 4 && (m100 < 12 || m100 > 14)) return few;
  return many;
}

export const fmtPrice = (price?: number | null, currency?: string) =>
  price === undefined || price === null ? '' : `${price.toLocaleString('uk-UA')} ${currency ?? ''}`.trim();
