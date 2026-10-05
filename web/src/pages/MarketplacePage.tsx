import { useCallback, useEffect, useState } from 'react';
import {
  apiCreatePost,
  apiDeletePost,
  apiGetNFTs,
  apiGetPosts,
  type NFT,
  type Post,
} from '../api';
import { useAuth } from '../auth';
import { Icon } from '../icons';
import EmptyState from '../EmptyState';
import { fmtPrice, plural } from '../format';

export default function MarketplacePage() {
  const { user } = useAuth();
  const [posts, setPosts] = useState<Post[]>([]);
  const [nfts, setNfts] = useState<NFT[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [creating, setCreating] = useState(false);

  const reload = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const [p, n] = await Promise.all([apiGetPosts(), apiGetNFTs()]);
      setPosts(p);
      setNfts(n);
    } catch (e: any) {
      setError(e?.message ?? 'Не вдалося завантажити');
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => { reload(); }, [reload]);

  const myListings = posts.filter(p => p.userId === user?.uid && p.forSale);

  const onDelete = async (id: string) => {
    if (!confirm('Зняти з продажу?')) return;
    try {
      await apiDeletePost(id);
      await reload();
    } catch (e: any) {
      alert(e?.message ?? 'Не вдалося зняти з продажу');
    }
  };

  return (
    <div>
      <div className="page-head">
        <div>
          <h2>Маркетплейс</h2>
          <p>{myListings.length > 0 ? `${myListings.length} ${plural(myListings.length, 'товар', 'товари', 'товарів')} у продажу` : 'Твої товари, виставлені на продаж.'}</p>
        </div>
        <div style={{ display: 'flex', gap: 8 }}>
          <button className="btn" onClick={reload} title="Оновити"><Icon.Refresh /></button>
          <button className="btn btn-primary" onClick={() => setCreating(true)}>
            <Icon.Plus /> Виставити товар
          </button>
        </div>
      </div>

      {error && <div className="error-banner">{error}</div>}

      {loading ? (
        <div className="spinner">Завантаження…</div>
      ) : myListings.length === 0 ? (
        <EmptyState
          icon="🏷️"
          title="У продажу поки нічого"
          text="Вистав NFT на продаж — покупці побачать його в застосунку Marki й зможуть оплатити."
          action={<button className="btn btn-primary" onClick={() => setCreating(true)}><Icon.Plus /> Виставити товар</button>}
        />
      ) : (
        <div className="grid grid-3">
          {myListings.map(p => (
            <div className="card" key={p.id}>
              {(p.nftImage || p.nftImages?.[0]) && (
                <img src={p.nftImage || p.nftImages![0]} alt={p.nftTitle || p.title || 'NFT'}
                  style={{ width: '100%', aspectRatio: '1 / 1', objectFit: 'cover', borderRadius: 10, marginBottom: 12 }}
                />
              )}
              <h3>{p.nftTitle || p.title || 'Без назви'}</h3>
              {(p.text || p.description) && <p className="sub" style={{ fontSize: 12 }}>{p.text || p.description}</p>}
              {p.nftImages && p.nftImages.length > 1 && (
                <div className="muted" style={{ fontSize: 12 }}>Колекція · {p.nftImages.length} шт</div>
              )}
              <div className="card-actions">
                <strong style={{ fontSize: 16 }}>{fmtPrice(p.price, p.currency)}</strong>
                <span className="spacer" />
                <button className="btn" onClick={() => onDelete(p.id)}>Зняти з продажу</button>
              </div>
            </div>
          ))}
        </div>
      )}

      {creating && (
        <CreateListingModal
          nfts={nfts}
          onClose={() => setCreating(false)}
          onDone={async () => { setCreating(false); await reload(); }}
        />
      )}
    </div>
  );
}

function CreateListingModal({ nfts, onClose, onDone }: { nfts: NFT[]; onClose: () => void; onDone: () => Promise<void> }) {
  const [nftId, setNftId] = useState('');
  const [text, setText] = useState('');
  const [price, setPrice] = useState('');
  const [currency, setCurrency] = useState('UAH');
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  const submit = async () => {
    if (!nftId || !price.trim()) {
      setErr('Вибери NFT та вкажи ціну.');
      return;
    }
    setBusy(true);
    setErr(null);
    try {
      const nft = nfts.find(n => n.id === nftId);
      await apiCreatePost({
        nftId,
        nftTitle: nft?.title,
        nftImage: nft?.imageUrl || nft?.image,
        text: text.trim(),
        forSale: true,
        price: parseFloat(price),
        currency,
      });
      await onDone();
    } catch (e: any) {
      setErr(e?.message ?? 'Не вдалося виставити');
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="modal-backdrop" onClick={busy ? undefined : onClose}>
      <div className="modal" onClick={e => e.stopPropagation()}>
        <h3>Виставити на продаж</h3>

        <div className="field">
          <label>NFT</label>
          <select value={nftId} onChange={e => setNftId(e.target.value)}>
            <option value="">Обери NFT зі свого каталогу</option>
            {nfts.map(n => <option key={n.id} value={n.id}>{n.title}</option>)}
          </select>
        </div>

        <div style={{ display: 'grid', gridTemplateColumns: '1fr 120px', gap: 12 }}>
          <div className="field">
            <label>Ціна</label>
            <input value={price} onChange={e => setPrice(e.target.value)} inputMode="decimal" />
          </div>
          <div className="field">
            <label>Валюта</label>
            <select value={currency} onChange={e => setCurrency(e.target.value)}>
              <option value="UAH">₴ UAH</option>
              <option value="USD">$ USD</option>
              <option value="ICP">ICP</option>
              <option value="USDC">USDC</option>
            </select>
          </div>
        </div>

        <div className="field">
          <label>Опис для покупців</label>
          <textarea value={text} onChange={e => setText(e.target.value)} placeholder="Що це за товар, чим він особливий…" />
        </div>

        {err && <div className="error-banner">{err}</div>}

        <div className="actions">
          <button className="btn" onClick={onClose} disabled={busy}>Скасувати</button>
          <button className="btn btn-primary" onClick={submit} disabled={busy}>
            {busy ? 'Створення…' : 'Виставити'}
          </button>
        </div>
      </div>
    </div>
  );
}
