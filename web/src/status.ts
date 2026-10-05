// Delivery and COD order statuses shared by CRM and the dashboard.

export const DELIVERY_STATUSES = [
  { value: 'pending',          label: 'Нова',         tone: 'badge-pending' },
  { value: 'assigned',         label: 'Призначена',     tone: 'badge-info' },
  { value: 'picked_up',        label: 'Забрана',       tone: 'badge-info' },
  { value: 'in_transit',       label: 'У дорозі',        tone: 'badge-info' },
  { value: 'out_for_delivery', label: 'На врученні',   tone: 'badge-pending' },
  { value: 'delivered',        label: 'Доставлена',    tone: 'badge-success' },
  { value: 'verified',         label: 'Перевірено NFC', tone: 'badge-success' },
  { value: 'failed',           label: 'Проблема',      tone: 'badge-danger' },
];

export const ORDER_STATUS_LABEL: Record<string, string> = {
  pending: 'Новий',
  in_delivery: 'У доставці',
  completed: 'Закрито',
  cancelled: 'Скасовано',
};

export const STATUS_TONE: Record<string, string> = {
  pending: 'badge-pending',
  assigned: 'badge-info',
  picked_up: 'badge-info',
  in_transit: 'badge-info',
  out_for_delivery: 'badge-pending',
  delivered: 'badge-success',
  verified: 'badge-success',
  failed: 'badge-danger',
  in_delivery: 'badge-info',
  completed: 'badge-success',
  cancelled: 'badge-muted',
};

export function statusLabel(status: string): string {
  return DELIVERY_STATUSES.find(s => s.value === status)?.label ?? ORDER_STATUS_LABEL[status] ?? status;
}

export function statusTone(status: string): string {
  return STATUS_TONE[status] ?? 'badge-muted';
}

