import { useEffect, useMemo, useState } from 'react';
import QRCode from 'qrcode';
import { apiCreateGift, type Gift, type NFT } from '../api';
import { Icon } from '../icons';
import { giftUrl } from './giftLink';

/**
 * Turn NFTs into a gift link. `nfts` are the candidates (one passport or a whole
 * collection); the sender picks how many copies the link gives away.
 */
export default function CreateGiftModal({ nfts, onClose, onCreated }: {
  nfts: NFT[];
  onClose: () => void;
  onCreated?: (gift: Gift) => void;
}) {
  // The edition master is the original, not a copy: it is neither counted nor gifted.
  const copies = useMemo(() => nfts.filter(n => !(n.editionNumber === 0 && n.editionCount)), [nfts]);
  // Items on sale cannot be gifted.
  const available = useMemo(() => copies.filter(n => !n.forSale), [copies]);
  const first = available[0] ?? copies[0] ?? nfts[0];
  const [count, setCount] = useState(Math.min(available.length, 1) || 0);
  const [title, setTitle] = useState(first?.batchName || first?.title || '');
  const [message, setMessage] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [gift, setGift] = useState<Gift | null>(null);

  const create = async () => {
    setBusy(true);
    setError(null);
    try {
      const g = await apiCreateGift({
        nftIds: available.slice(0, count).map(n => n.id),
        title: title.trim() || undefined,
        message: message.trim() || undefined,
      });
      setGift(g);
      onCreated?.(g);
    } catch (e: any) {
      setError(e?.message ?? 'Не вдалося створити подарунок');
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="modal-backdrop" onClick={busy ? undefined : onClose}>
      <div className="modal product-modal" onClick={e => e.stopPropagation()}>
        {gift ? <GiftShare gift={gift} onClose={onClose} /> : (
          <>
            <h3 style={{ display: 'flex', alignItems: 'center', gap: 8 }}><Icon.Gift /> Подарувати</h3>
            <p className="sub" style={{ marginTop: 0 }}>
              Створимо посилання й QR. Хто відкриє й увійде в Marki — отримає NFT у свої паспорти. Ти бачитимеш, хто й коли забрав.
            </p>

            {first && (
              <div className="gift-head">
                {(first.imageUrl || first.image) && <img src={first.imageUrl || first.image} alt="" />}
                <div>
                  <strong>{first.batchName || first.title}</strong>
                  <div className="muted" style={{ fontSize: 12 }}>
                    Доступно для подарунку: {available.length} з {copies.length}
                  </div>
                </div>
              </div>
            )}

            {available.length === 0 ? (
              <div className="notice notice-warn">Усі ці NFT виставлені на продаж. Зніми їх із продажу, щоб подарувати.</div>
            ) : (
              <>
                <div className="field">
                  <label>Скільки людей отримають подарунок</label>
                  <div className="stepper-input">
                    <button type="button" className="btn" onClick={() => setCount(Math.max(1, count - 1))}>−</button>
                    <input type="number" min={1} max={available.length} value={count}
                      onChange={e => setCount(Math.min(available.length, Math.max(1, Number(e.target.value) || 1)))} />
                    <button type="button" className="btn" onClick={() => setCount(Math.min(available.length, count + 1))}>+</button>
                  </div>
                  <div className="muted" style={{ fontSize: 11, marginTop: 4 }}>Кожна людина отримає 1 NFT. Одне посилання — до {count} отримувачів.</div>
                </div>
                <div className="field">
                  <label>Назва подарунку</label>
                  <input value={title} maxLength={60} onChange={e => setTitle(e.target.value)} />
                </div>
                <div className="field">
                  <label>Повідомлення <span className="muted">(необов’язково)</span></label>
                  <textarea value={message} maxLength={280} onChange={e => setMessage(e.target.value)} placeholder="Дякуємо, що ви з нами! 🎉" />
                </div>
              </>
            )}

            {error && <div className="error-banner">{error}</div>}
            <div className="actions">
              <button className="btn" onClick={onClose} disabled={busy}>Скасувати</button>
              <button className="btn btn-primary" onClick={create} disabled={busy || count < 1}>
                <Icon.Gift /> {busy ? 'Створюємо…' : 'Створити посилання'}
              </button>
            </div>
          </>
        )}
      </div>
    </div>
  );
}

/** Link + QR of a gift with copy / share. Also used from the gifts page. */
export function GiftShare({ gift, onClose }: { gift: Gift; onClose?: () => void }) {
  const url = giftUrl(gift.id);
  const [qr, setQr] = useState('');
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    QRCode.toDataURL(url, { width: 360, margin: 1, color: { dark: '#0b0f14', light: '#ffffff' } }).then(setQr).catch(() => {});
  }, [url]);

  const copy = async () => {
    await navigator.clipboard.writeText(url);
    setCopied(true);
    setTimeout(() => setCopied(false), 1500);
  };
  const share = async () => {
    if (navigator.share) {
      try { await navigator.share({ title: gift.title, text: `🎁 ${gift.title} — подарунок у Marki`, url }); return; } catch { /* cancelled */ }
    }
    await copy();
  };

  return (
    <div style={{ textAlign: 'center' }}>
      <div style={{ fontSize: 40 }}>🎁</div>
      <h3 style={{ margin: '4px 0' }}>Подарунок готовий</h3>
      <p className="sub" style={{ marginTop: 0 }}>
        «{gift.title}» · {gift.remainingNftIds.length} з {gift.total} ще не забрали
      </p>
      {qr && <img src={qr} alt="QR подарунку" className="gift-qr" />}
      <div className="copy-row" style={{ marginTop: 12 }}>
        <code>{url.replace(/^https?:\/\//, '')}</code>
        <button className="btn" onClick={copy}>{copied ? 'Скопійовано' : 'Копіювати'}</button>
      </div>
      <div className="actions" style={{ justifyContent: 'center' }}>
        <button className="btn btn-primary" onClick={share}><Icon.Upload /> Поділитися</button>
        {onClose && <button className="btn" onClick={onClose}>Готово</button>}
      </div>
    </div>
  );
}
