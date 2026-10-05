import { useCallback, useEffect, useState } from 'react';
import {
  apiAddCryptoWallet,
  apiGetCryptoWallets,
  apiGetMarkiWallet,
  apiRefreshWalletBalance,
  apiRemoveCryptoWallet,
  apiUpdateFingerprint,
  apiUpdateMarkiEmail,
  type CryptoWallet,
  type MarkiWallet,
} from '../api';
import { Icon } from '../icons';
import EmptyState from '../EmptyState';

export default function WalletsPage() {
  const [marki, setMarki] = useState<MarkiWallet | null>(null);
  const [cryptos, setCryptos] = useState<CryptoWallet[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const reload = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const [m, c] = await Promise.all([
        apiGetMarkiWallet().catch(() => null),
        apiGetCryptoWallets().catch(() => []),
      ]);
      setMarki(m);
      setCryptos(c);
    } catch (e: any) {
      setError(e?.message ?? 'Не вдалося завантажити');
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => { reload(); }, [reload]);

  return (
    <div>
      <div className="page-head">
        <div>
          <h2>Гаманці</h2>
          <p>Внутрішній гаманець Marki і твої ICP-гаманці.</p>
        </div>
        <button className="btn" onClick={reload}><Icon.Refresh /> Оновити</button>
      </div>

      {error && <div className="error-banner">{error}</div>}
      {loading && <div className="spinner">Завантаження…</div>}

      {!loading && marki && <MarkiCard wallet={marki} reload={reload} />}

      {!loading && (
        <>
          <div className="section-title">ICP-гаманці</div>
          <CryptoList wallets={cryptos} reload={reload} />
        </>
      )}
    </div>
  );
}

function MarkiCard({ wallet, reload }: { wallet: MarkiWallet; reload: () => Promise<void> }) {
  const [editingEmail, setEditingEmail] = useState(false);
  const [emailValue, setEmailValue] = useState(wallet.email);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  const saveEmail = async () => {
    setBusy(true);
    setErr(null);
    try {
      await apiUpdateMarkiEmail(emailValue.trim());
      setEditingEmail(false);
      await reload();
    } catch (e: any) {
      setErr(e?.message ?? 'Не вдалося зберегти');
    } finally {
      setBusy(false);
    }
  };

  const toggleFp = async (enabled: boolean) => {
    try {
      await apiUpdateFingerprint(enabled);
      await reload();
    } catch (e: any) {
      alert(e?.message ?? 'Не вдалося змінити налаштування');
    }
  };

  // Legacy chains (Solana, Polygon) show up only when they still hold something.
  const balanceRows = Object.entries(wallet.balance || {})
    .filter(([cur, amt]) => cur.toUpperCase() === 'ICP' || amt > 0)
    .sort(([a], [b]) => (a.toUpperCase() === 'ICP' ? -1 : b.toUpperCase() === 'ICP' ? 1 : a.localeCompare(b)));

  return (
    <div className="card">
      <h3>Marki Wallet</h3>
      <p className="sub" style={{ marginBottom: 16 }}>Гаманець, яким керує Marki: нічого не потрібно встановлювати.</p>

      <div className="field">
        <label>Email</label>
        {editingEmail ? (
          <div style={{ display: 'flex', gap: 8 }}>
            <input value={emailValue} onChange={e => setEmailValue(e.target.value)} />
            <button className="btn btn-primary" onClick={saveEmail} disabled={busy}>OK</button>
            <button className="btn" onClick={() => { setEditingEmail(false); setEmailValue(wallet.email); }} disabled={busy}>Скасувати</button>
          </div>
        ) : (
          <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
            <span>{wallet.email || <span className="muted">не вказано</span>}</span>
            <button className="btn" onClick={() => setEditingEmail(true)}>{wallet.email ? 'Змінити' : 'Додати'}</button>
          </div>
        )}
      </div>

      <div className="toggle-row">
        <div>
          <div className="lbl">Підтвердження відбитком</div>
          <div className="sub">Питати відбиток пальця перед кожним переказом.</div>
        </div>
        <div
          className={`switch ${wallet.fingerprintEnabled ? 'on' : ''}`}
          role="switch"
          aria-checked={wallet.fingerprintEnabled}
          onClick={() => toggleFp(!wallet.fingerprintEnabled)}
        >
          <div className="knob" />
        </div>
      </div>

      {balanceRows.length > 0 && (
        <>
          <div className="muted" style={{ fontSize: 12, margin: '16px 0 8px' }}>Баланс</div>
          <div style={{ display: 'flex', gap: 12, flexWrap: 'wrap' }}>
            {balanceRows.map(([cur, amt]) => (
              <div key={cur} className="card" style={{ background: 'var(--bg-soft)', padding: '10px 16px' }}>
                <div className="sub" style={{ fontSize: 11 }}>{cur.toUpperCase()}</div>
                <div style={{ fontWeight: 600, fontSize: 18 }}>{amt.toLocaleString('uk-UA', { maximumFractionDigits: 4 })}</div>
              </div>
            ))}
          </div>
        </>
      )}

      {err && <div className="error-banner" style={{ marginTop: 12 }}>{err}</div>}
    </div>
  );
}

function CryptoList({ wallets, reload }: { wallets: CryptoWallet[]; reload: () => Promise<void> }) {
  const [address, setAddress] = useState('');
  const [label, setLabel] = useState('');
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  const add = async () => {
    if (!address.trim()) return;
    setBusy(true);
    setErr(null);
    try {
      await apiAddCryptoWallet({ address: address.trim(), label: label.trim() || undefined });
      setAddress(''); setLabel('');
      await reload();
    } catch (e: any) {
      setErr(e?.message ?? 'Не вдалося додати гаманець');
    } finally {
      setBusy(false);
    }
  };

  const remove = async (id: string) => {
    if (!confirm('Видалити гаманець?')) return;
    try {
      await apiRemoveCryptoWallet(id);
      await reload();
    } catch (e: any) {
      alert(e?.message ?? 'Не вдалося видалити');
    }
  };

  const refresh = async (id: string) => {
    try {
      await apiRefreshWalletBalance(id);
      await reload();
    } catch (e: any) {
      alert(e?.message ?? 'Не вдалося оновити баланс');
    }
  };

  return (
    <>
      <div className="card" style={{ marginBottom: 14 }}>
        <h3 style={{ fontSize: 14 }}>Додати ICP-гаманець</h3>
        <p className="sub" style={{ fontSize: 12, margin: '2px 0 0' }}>Встав principal з Plug (кнопка «Copy principal»), щоб бачити баланс тут.</p>
        <div style={{ display: 'grid', gridTemplateColumns: '2fr 1fr auto', gap: 8, marginTop: 10 }}>
          <input
            placeholder="Principal, напр. 7zpkm-…-vfe"
            value={address}
            onChange={e => setAddress(e.target.value)}
            style={{ background: 'var(--bg-soft)', border: '1px solid var(--border)', borderRadius: 10, padding: '10px 12px', color: 'var(--text)' }}
          />
          <input
            placeholder="Назва (необов’язково)"
            value={label}
            onChange={e => setLabel(e.target.value)}
            style={{ background: 'var(--bg-soft)', border: '1px solid var(--border)', borderRadius: 10, padding: '10px 12px', color: 'var(--text)' }}
          />
          <button className="btn btn-primary" onClick={add} disabled={busy || !address.trim()}>Додати</button>
        </div>
        {err && <div className="error-banner" style={{ marginTop: 10 }}>{err}</div>}
      </div>

      {wallets.length === 0 ? (
        <EmptyState icon="👛" title="Ще немає ICP-гаманців" text="Додай principal свого Plug-гаманця вище — і тут з’явиться його баланс." />
      ) : (
        <div className="grid grid-2">
          {wallets.map(w => (
            <div key={w.id} className="card">
              <h3>{w.label || 'ICP-гаманець'}</h3>
              <div className="sub" style={{ fontFamily: 'monospace', fontSize: 11, wordBreak: 'break-all' }}>{w.address}</div>
              <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginTop: 12 }}>
                <span>
                  {w.balance != null ? <strong>{w.balance.toFixed(4)} ICP</strong> : <span className="sub">—</span>}
                </span>
                <div style={{ display: 'flex', gap: 8 }}>
                  <button className="btn" onClick={() => refresh(w.id)} title="Оновити баланс"><Icon.Refresh /> Баланс</button>
                  <button className="btn ghost-danger" onClick={() => remove(w.id)} title="Видалити гаманець"><Icon.Trash /></button>
                </div>
              </div>
            </div>
          ))}
        </div>
      )}
    </>
  );
}
