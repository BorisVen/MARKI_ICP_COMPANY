import { useCallback, useEffect, useState } from 'react';
import { apiCancelGift, apiMyGifts, type Gift } from '../api';
import EmptyState from '../EmptyState';
import { fmtAgo, fmtDateTime } from '../format';
import { Icon } from '../icons';
import { ListSkeleton } from '../Skeleton';
import { GiftShare } from './CreateGiftModal';
import { printGiftQrs } from './printGifts';

const STATUS: Record<Gift['status'], { label: string; tone: string }> = {
  active: { label: 'Активний', tone: 'badge-info' },
  completed: { label: 'Усі забрали', tone: 'badge-success' },
  cancelled: { label: 'Скасовано', tone: 'badge-muted' },
};

/** Gifts I sent (with who claimed what and when) and gifts I received. */
export default function GiftsPage({ hint }: { hint?: string }) {
  const [tab, setTab] = useState<'sent' | 'received'>('sent');
  const [data, setData] = useState<{ sent: Gift[]; received: Gift[] } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [sharing, setSharing] = useState<Gift | null>(null);

  const reload = useCallback(async () => {
    setError(null);
    try {
      setData(await apiMyGifts());
    } catch (e: any) {
      setError(e?.status === 404 ? 'Подарунки запрацюють після оновлення сервера.' : e?.message ?? 'Не вдалося завантажити подарунки');
      setData({ sent: [], received: [] });
    }
  }, []);

  useEffect(() => { reload(); }, [reload]);

  const cancel = async (g: Gift) => {
    if (!confirm(`Скасувати посилання «${g.title}»? Незабрані NFT (${g.remainingNftIds.length}) залишаться в тебе.`)) return;
    try { await apiCancelGift(g.id); await reload(); } catch (e: any) { alert(e?.message ?? 'Не вдалося скасувати'); }
  };

  const list = data ? data[tab] : null;

  return (
    <div>
      <div className="page-head">
        <div>
          <h2>Подарунки</h2>
          <p>{hint ?? 'Посилання й QR на подарунки та хто їх забрав.'}</p>
        </div>
        <button className="btn" onClick={reload} title="Оновити"><Icon.Refresh /></button>
      </div>

      <div className="mode-tabs" role="tablist">
        <button className={`mode-tab ${tab === 'sent' ? 'active' : ''}`} onClick={() => setTab('sent')}>
          Надіслані {data && data.sent.length > 0 && <span className="pill">{data.sent.length}</span>}
        </button>
        <button className={`mode-tab ${tab === 'received' ? 'active' : ''}`} onClick={() => setTab('received')}>
          Отримані {data && data.received.length > 0 && <span className="pill">{data.received.length}</span>}
        </button>
      </div>

      {error && <div className="notice notice-info">{error}</div>}

      {!list ? <ListSkeleton /> : list.length === 0 ? (
        <EmptyState
          icon="🎁"
          title={tab === 'sent' ? 'Ти ще нічого не дарував(ла)' : 'Подарунків поки немає'}
          text={tab === 'sent' ? 'Відкрий колекцію чи паспорт і натисни «Подарувати» — отримаєш посилання й QR.' : 'Коли хтось подарує тобі NFT, він з’явиться тут і в паспортах.'}
        />
      ) : (
        <div className="order-list">
          {tab === 'sent'
            ? groupSent(list).map(item => Array.isArray(item)
              ? <GiftGroup key={item[0].groupId} gifts={item} onShare={setSharing} onCancel={cancel} />
              : <SentGift key={item.id} gift={item} onShare={() => setSharing(item)} onCancel={() => cancel(item)} />)
            : list.map(g => <ReceivedGift key={g.id} gift={g} />)}
        </div>
      )}

      {sharing && (
        <div className="modal-backdrop" onClick={() => setSharing(null)}>
          <div className="modal product-modal" onClick={e => e.stopPropagation()}>
            <GiftShare gift={sharing} onClose={() => setSharing(null)} />
          </div>
        </div>
      )}
    </div>
  );
}

/** Sub-collections of one gift collection go together, in part order. */
function groupSent(gifts: Gift[]): (Gift | Gift[])[] {
  const out: (Gift | Gift[])[] = [];
  const groups = new Map<string, Gift[]>();
  for (const g of gifts) {
    if (!g.groupId) { out.push(g); continue; }
    let group = groups.get(g.groupId);
    if (!group) { group = []; groups.set(g.groupId, group); out.push(group); }
    group.push(g);
  }
  for (const group of groups.values()) group.sort((a, b) => (a.part ?? 0) - (b.part ?? 0));
  return out;
}

function GiftGroup({ gifts, onShare, onCancel }: { gifts: Gift[]; onShare: (g: Gift) => void; onCancel: (g: Gift) => void }) {
  const [open, setOpen] = useState(false);
  const total = gifts.reduce((n, g) => n + g.total, 0);
  const claimed = gifts.reduce((n, g) => n + g.claims.length, 0);
  const pct = total ? Math.round((claimed / total) * 100) : 0;
  const first = gifts[0];
  return (
    <div className="card order-card gift-card">
      <div className="oc-head">
        <div style={{ display: 'flex', gap: 12, minWidth: 0 }}>
          {first.image && <img src={first.image} alt="" className="gift-thumb" />}
          <div style={{ minWidth: 0 }}>
            <div className="oc-title">{first.title}</div>
            <div className="muted" style={{ fontSize: 12 }}>{gifts.length} підколекцій · створено {fmtAgo(first.createdAt)}</div>
          </div>
        </div>
        <span className="badge badge-info">🎁 Колекція</span>
      </div>
      <div className="meter" style={{ marginTop: 12 }}>
        <div className="meter-head"><span>Забрали</span><span><strong>{claimed}</strong> <span className="muted">з {total}</span></span></div>
        <div className="meter-track"><div className="meter-fill" style={{ width: `${pct}%` }} /></div>
      </div>
      <div className="card-actions">
        <button className="btn btn-primary" onClick={() => printGiftQrs(gifts.filter(g => g.status === 'active'))}><Icon.QrCode /> Друкувати QR</button>
        <button className="btn" onClick={() => setOpen(o => !o)}>{open ? 'Сховати частини' : `Частини (${gifts.length})`}</button>
      </div>
      {open && (
        <div className="order-list" style={{ marginTop: 10 }}>
          {gifts.map(g => <SentGift key={g.id} gift={g} onShare={() => onShare(g)} onCancel={() => onCancel(g)} />)}
        </div>
      )}
    </div>
  );
}

function SentGift({ gift: g, onShare, onCancel }: { gift: Gift; onShare: () => void; onCancel: () => void }) {
  const [open, setOpen] = useState(false);
  const claimed = g.claims.length;
  const pct = g.total ? Math.round((claimed / g.total) * 100) : 0;
  return (
    <div className="card order-card gift-card">
      <div className="oc-head">
        <div style={{ display: 'flex', gap: 12, minWidth: 0 }}>
          {g.image && <img src={g.image} alt="" className="gift-thumb" />}
          <div style={{ minWidth: 0 }}>
            <div className="oc-title">{g.parts ? `Частина ${g.part} з ${g.parts}` : g.title}</div>
            <div className="muted" style={{ fontSize: 12 }}>
              {g.parts ? `${g.total} NFT${g.allowAll ? ' · можна забрати всю' : ''}` : `Створено ${fmtAgo(g.createdAt)}`}
            </div>
          </div>
        </div>
        <span className={`badge ${STATUS[g.status].tone}`}>{STATUS[g.status].label}</span>
      </div>

      <div className="meter" style={{ marginTop: 12 }}>
        <div className="meter-head"><span>Забрали</span><span><strong>{claimed}</strong> <span className="muted">з {g.total}</span></span></div>
        <div className="meter-track"><div className="meter-fill" style={{ width: `${pct}%` }} /></div>
      </div>

      <div className="card-actions">
        {g.status === 'active' && <button className="btn btn-primary" onClick={onShare}><Icon.QrCode /> Посилання й QR</button>}
        {claimed > 0 && <button className="btn" onClick={() => setOpen(o => !o)}>{open ? 'Сховати' : `Хто забрав (${claimed})`}</button>}
        <span className="spacer" />
        {g.status === 'active' && <button className="btn ghost-danger" onClick={onCancel}>Скасувати</button>}
      </div>

      {open && (
        <div className="passport-timeline" style={{ marginTop: 10 }}>
          {[...g.claims].reverse().map(c => (
            <div key={c.nftId} className="pt-item">
              <span className="pt-icon">🎁</span>
              <div>
                <div style={{ fontWeight: 600, fontSize: 14 }}>{c.name || 'Користувач Marki'} {c.username && <span className="muted">@{c.username}</span>}</div>
                <div className="muted" style={{ fontSize: 11 }}>{fmtDateTime(c.claimedAt)}</div>
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

function ReceivedGift({ gift: g }: { gift: Gift }) {
  return (
    <div className="card order-card gift-card">
      <div style={{ display: 'flex', gap: 12 }}>
        {g.image && <img src={g.image} alt="" className="gift-thumb" />}
        <div style={{ minWidth: 0 }}>
          <div className="oc-title">{g.title}</div>
          <div className="muted" style={{ fontSize: 12 }}>
            від {g.ownerName}{g.fromCompany && ' · перевірений бренд'}
          </div>
          {g.message && <p style={{ fontSize: 13, margin: '6px 0 0' }}>«{g.message}»</p>}
        </div>
      </div>
    </div>
  );
}
