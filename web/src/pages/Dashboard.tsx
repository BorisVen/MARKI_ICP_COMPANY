import { useEffect, useState, type ReactNode } from 'react';
import {
  apiGetNFTs,
  apiGetNotifications,
  apiListCodOrders,
  apiListDeliveries,
  type CodOrder,
  type Delivery,
} from '../api';
import { useAuth } from '../auth';
import { Icon } from '../icons';
import PlanMeter from '../icp/PlanMeter';
import { useSubscription } from '../icp/useSubscription';
import { OPEN_CREATE_KEY } from './NftsPage';
import { PageId } from '../Shell';
import { statusLabel, statusTone } from '../status';
import { fmtAgo, plural } from '../format';

type Props = { onJumpTo: (p: PageId) => void };

type Data = {
  pendingOrders: CodOrder[];
  deliveries: Delivery[];
  nftCount: number;
  unread: number;
};

type Attention = {
  key: string;
  tone: 'info' | 'warn' | 'danger';
  icon: string;
  title: string;
  sub: string;
  cta: string;
  page: PageId;
};

const FINISHED = new Set(['delivered', 'verified', 'completed', 'cancelled']);

function greeting(): string {
  const h = new Date().getHours();
  if (h < 6) return 'Доброї ночі';
  if (h < 12) return 'Доброго ранку';
  if (h < 18) return 'Добрий день';
  return 'Добрий вечір';
}

export default function Dashboard({ onJumpTo }: Props) {
  const { user } = useAuth();
  const plan = useSubscription();
  const [data, setData] = useState<Data | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      const [orders, deliveries, nfts, notes] = await Promise.all([
        apiListCodOrders().catch(() => [] as CodOrder[]),
        apiListDeliveries().catch(() => [] as Delivery[]),
        apiGetNFTs().catch(() => []),
        apiGetNotifications().catch(() => []),
      ]);
      if (cancelled) return;
      setData({
        pendingOrders: orders.filter(o => o.status === 'pending'),
        deliveries: [...deliveries].sort((a, b) => b.createdAt.localeCompare(a.createdAt)),
        nftCount: nfts.length,
        unread: notes.filter(n => !n.read).length,
      });
      setLoading(false);
    })();
    return () => { cancelled = true; };
  }, []);

  const openCreateNft = () => {
    try { sessionStorage.setItem(OPEN_CREATE_KEY, '1'); } catch { /* noop */ }
    onJumpTo('nfts');
  };

  const name = user?.companyName || user?.name || '';
  const active = data?.deliveries.filter(d => !FINISHED.has(d.status)) ?? [];
  const failed = active.filter(d => d.status === 'failed');
  const delivered = data?.deliveries.filter(d => FINISHED.has(d.status) && d.status !== 'cancelled') ?? [];

  // Most urgent first: money waiting, broken deliveries, plan running out, messages.
  const attention: Attention[] = [];
  if (data && data.pendingOrders.length > 0) {
    const n = data.pendingOrders.length;
    attention.push({
      key: 'orders', tone: 'warn', icon: '🛒',
      title: `${n} ${plural(n, 'нове замовлення', 'нові замовлення', 'нових замовлень')} чекає на вас`,
      sub: `Покупці оформили оплату при отриманні. Прийміть, щоб створити доставку.`,
      cta: 'Прийняти', page: 'crm',
    });
  }
  if (failed.length > 0) {
    attention.push({
      key: 'failed', tone: 'danger', icon: '⚠️',
      title: `${failed.length} ${plural(failed.length, 'доставка має', 'доставки мають', 'доставок мають')} проблему`,
      sub: failed.slice(0, 2).map(d => d.nftTitle).join(', '),
      cta: 'Розібратися', page: 'crm',
    });
  }
  if (plan.enabled && plan.principal) {
    const sub = plan.subscription;
    if (!plan.loading && !plan.active) {
      attention.push({
        key: 'plan', tone: 'warn', icon: '💳',
        title: sub ? 'Тариф закінчився' : 'Оберіть тариф, щоб випускати NFT',
        sub: 'Без тарифу мінт у блокчейн недоступний.',
        cta: 'Обрати тариф', page: 'pricing',
      });
    } else if (sub && plan.plan) {
      const low = sub.photo_mints_left * 10n <= plan.plan.photo_mints;
      if (low || plan.daysLeft <= 5) {
        attention.push({
          key: 'plan', tone: 'warn', icon: '⏳',
          title: low ? `Залишилось ${sub.photo_mints_left} NFT з фото` : `Тариф закінчується через ${plan.daysLeft} дн.`,
          sub: 'Продовжте тариф заздалегідь, щоб не зупиняти випуск.',
          cta: 'Продовжити', page: 'pricing',
        });
      }
    }
  }
  if (data && data.unread > 0) {
    attention.push({
      key: 'notes', tone: 'info', icon: '🔔',
      title: `${data.unread} ${plural(data.unread, 'нове сповіщення', 'нові сповіщення', 'нових сповіщень')}`,
      sub: 'Покупки, продажі та зміни статусів.',
      cta: 'Переглянути', page: 'notifications',
    });
  }

  const quick: { label: string; sub: string; icon: ReactNode; onClick: () => void }[] = [
    { label: 'Створити NFT', sub: 'Фото, колекція або серія', icon: <Icon.Plus />, onClick: openCreateNft },
    { label: 'Замовлення і доставки', sub: 'CRM, Нова пошта, кур’єри', icon: <Icon.Truck />, onClick: () => onJumpTo('crm') },
    { label: 'Перевірити NFC', sub: 'Сканувати мітку товару', icon: <Icon.QrCode />, onClick: () => onJumpTo('crm') },
    { label: 'Тарифи', sub: 'Мінти та оплата', icon: <Icon.Tag />, onClick: () => onJumpTo('pricing') },
  ];

  return (
    <div>
      <div className="page-head greeting">
        <div>
          <h2>{greeting()}{name ? `, ${name}` : ''} 👋</h2>
          <p>{new Date().toLocaleDateString('uk-UA', { weekday: 'long', day: 'numeric', month: 'long' })}</p>
        </div>
      </div>

      <div className="section-title" style={{ marginTop: 0 }}>Потребує уваги</div>
      {loading ? (
        <div className="spinner">Завантаження…</div>
      ) : attention.length === 0 ? (
        <div className="all-good"><Icon.Check /> Усе під контролем. Нових справ немає.</div>
      ) : (
        <div className="attention-list">
          {attention.map(a => (
            <button key={a.key} className={`attention-item ${a.tone}`} onClick={() => onJumpTo(a.page)}>
              <span className="ai-icon">{a.icon}</span>
              <span className="ai-body">
                <div className="ai-title">{a.title}</div>
                <div className="ai-sub">{a.sub}</div>
              </span>
              <span className="ai-cta">{a.cta} →</span>
            </button>
          ))}
        </div>
      )}

      <div className="section-title">Швидкі дії</div>
      <div className="quick-actions">
        {quick.map(q => (
          <button key={q.label} className="quick-action" onClick={q.onClick}>
            <span className="qa-icon">{q.icon}</span>
            <span className="qa-label">{q.label}</span>
            <span className="qa-sub">{q.sub}</span>
          </button>
        ))}
      </div>

      <div className="dash-split" style={{ marginTop: 26 }}>
        <div>
          <div className="section-title" style={{ marginTop: 0 }}>Бізнес у цифрах</div>
          <div className="stat-row">
            <button className="stat-tile" onClick={() => onJumpTo('crm')}>
              <div className="st-value" style={{ color: 'var(--primary)' }}>{active.length}</div>
              <div className="st-label">у доставці</div>
            </button>
            <button className="stat-tile" onClick={() => onJumpTo('crm')}>
              <div className="st-value" style={{ color: 'var(--success)' }}>{delivered.length}</div>
              <div className="st-label">доставлено</div>
            </button>
            <button className="stat-tile" onClick={() => onJumpTo('nfts')}>
              <div className="st-value">{data?.nftCount ?? '—'}</div>
              <div className="st-label">NFT у каталозі</div>
            </button>
          </div>
        </div>

        {plan.enabled && (
          <div>
            <div className="section-title" style={{ marginTop: 0 }}>Тариф</div>
            <div className="card">
              {plan.active ? (
                <PlanMeter state={plan} />
              ) : (
                <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', gap: 12, flexWrap: 'wrap' }}>
                  <span className="muted">{plan.principal ? 'Активного тарифу немає.' : 'Підключіть гаманець на сторінці «Тарифи».'}</span>
                  <button className="btn btn-primary" onClick={() => onJumpTo('pricing')}>Обрати тариф</button>
                </div>
              )}
            </div>
          </div>
        )}
      </div>

      <div className="section-title">Останні доставки</div>
      {!data || data.deliveries.length === 0 ? (
        <div className="empty">
          Доставок ще немає. Вони з’являться, коли покупці оформлять замовлення.
        </div>
      ) : (
        <div className="table-wrap">
          <div className="table-scroll">
            <table className="table">
              <thead>
                <tr><th>Товар</th><th>Покупець</th><th>Статус</th><th>Коли</th></tr>
              </thead>
              <tbody>
                {data.deliveries.slice(0, 5).map(d => (
                  <tr key={d.id} onClick={() => onJumpTo('crm')} style={{ cursor: 'pointer' }}>
                    <td>{d.nftTitle}</td>
                    <td>{d.buyerName}</td>
                    <td><span className={`badge ${statusTone(d.status)}`}>{statusLabel(d.status)}</span></td>
                    <td className="muted">{fmtAgo(d.createdAt)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </div>
      )}
    </div>
  );
}
