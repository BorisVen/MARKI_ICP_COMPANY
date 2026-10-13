import type { GoodsCategory, GoodsEventKind, GoodsStage } from '../api';

/** Shared with the buyer app (keep both copies in sync). */
export const CATEGORY: Record<GoodsCategory, { label: string; icon: string; mode: 'unit' | 'batch'; attrs: string[] }> = {
  wine:      { label: 'Вино',            icon: '🍷', mode: 'unit',  attrs: ['Сорт', 'Рік урожаю', 'Регіон', "Об'єм", 'Міцність'] },
  spirits:   { label: 'Міцний алкоголь', icon: '🥃', mode: 'unit',  attrs: ['Вид', 'Витримка', "Об'єм", 'Міцність'] },
  beer:      { label: 'Пиво',            icon: '🍺', mode: 'batch', attrs: ['Стиль', "Об'єм", 'Міцність'] },
  dairy:     { label: 'Молочне',         icon: '🥛', mode: 'batch', attrs: ['Ферма', 'Жирність', "Об'єм"] },
  food:      { label: 'Продукти',        icon: '🧺', mode: 'batch', attrs: ['Склад', 'Вага', 'Походження'] },
  cosmetics: { label: 'Косметика',       icon: '🧴', mode: 'unit',  attrs: ['Лінійка', "Об'єм"] },
  other:     { label: 'Інше',            icon: '📦', mode: 'unit',  attrs: [] },
};

export const STAGE: Record<GoodsStage, string> = {
  produced: 'Вироблено',
  warehouse: 'На складі',
  shipped: 'У дорозі',
  in_store: 'У магазині',
};

export const EVENT: Record<GoodsEventKind, { label: string; icon: string }> = {
  produced:  { label: 'Вироблено / розлито', icon: '🏭' },
  warehouse: { label: 'Прийнято на склад',   icon: '📦' },
  shipped:   { label: 'Відвантажено',        icon: '🚚' },
  in_store:  { label: 'Надійшло в магазин',  icon: '🏪' },
  lab:       { label: 'Лабораторний аналіз', icon: '🧪' },
  note:      { label: 'Примітка',            icon: '📝' },
};
