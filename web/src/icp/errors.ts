/** Turn canister / agent errors into short Ukrainian messages for the UI. */
export function friendlyIcpError(raw: string): string {
  const m = raw.toLowerCase();
  if (m.includes('insufficient funds')) return 'Недостатньо ICP на гаманці. Поповни баланс і спробуй ще раз.';
  if (m.includes('insufficient allowance')) return 'Гаманець не підтвердив списання. Спробуй ще раз і натисни «Підтвердити».';
  if (m.includes('no plan') || m.includes('no active plan')) return 'Немає активного тарифу. Обери тариф у розділі «Тарифи».';
  if (m.includes('plan expired')) return 'Тариф закінчився. Продовж його в розділі «Тарифи».';
  if (m.includes('not enough mints')) return 'У тарифі закінчилися мінти. Докупи тариф у розділі «Тарифи».';
  if (m.includes('photo is larger')) {
    const kb = raw.match(/(\d+) KB/)?.[1];
    return `Фото завелике для тарифу${kb ? ` (максимум ${Number(kb) >= 1024 ? `${Number(kb) / 1024} MB` : `${kb} KB`})` : ''}.`;
  }
  if (m.includes('price changed')) return 'Курс ICP змінився. Онови сторінку й спробуй ще раз.';
  if (m.includes('anonymous caller')) return 'Спершу підключи гаманець.';
  if (m.includes('user rejected') || m.includes('rejected by user') || m.includes('denied')) return 'Операцію скасовано в гаманці.';
  if (m.includes('failed to fetch') || m.includes('networkerror')) return 'Немає зв’язку з мережею ICP. Перевір, що локальна мережа запущена.';
  return raw;
}
