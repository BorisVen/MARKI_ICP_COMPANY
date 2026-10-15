import QRCode from 'qrcode';
import type { Gift } from '../api';
import { giftUrl } from './giftLink';

/** Opens a print-ready A4 sheet with one QR per sub-collection. */
export async function printGiftQrs(gifts: Gift[]) {
  const qrs = await Promise.all(gifts.map(g => QRCode.toDataURL(giftUrl(g.id), { width: 300, margin: 1, errorCorrectionLevel: 'M' })));
  const esc = (s: string) => s.replace(/[&<>"]/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]!));
  const cards = gifts.map((g, i) => `
    <div class="c"><img src="${qrs[i]}"><b>${esc(g.title)}</b>
      <span>${g.parts ? `Частина ${g.part} з ${g.parts} · ` : ''}${g.total} NFT</span>
      <small>Відскануй і забери подарунок у Marki</small></div>`).join('');
  const w = window.open('', '_blank');
  if (!w) { alert('Дозволь спливні вікна, щоб надрукувати QR.'); return; }
  w.document.write(`<!doctype html><html><head><meta charset="utf-8"><title>QR подарунків · ${esc(gifts[0]?.title ?? '')}</title>
    <style>
      @page { size: A4; margin: 10mm; }
      body { font-family: -apple-system, system-ui, sans-serif; margin: 0; }
      .g { display: grid; grid-template-columns: repeat(3, 1fr); gap: 8mm; }
      .c { border: 1px dashed #bbb; border-radius: 4mm; padding: 4mm; text-align: center; break-inside: avoid; display: flex; flex-direction: column; gap: 1.5mm; }
      .c img { width: 100%; max-width: 50mm; margin: 0 auto; }
      .c b { font-size: 11pt; } .c span { font-size: 9pt; } .c small { font-size: 7.5pt; color: #666; }
    </style></head><body><div class="g">${cards}</div><script>window.onload = () => window.print();</script></body></html>`);
  w.document.close();
}
