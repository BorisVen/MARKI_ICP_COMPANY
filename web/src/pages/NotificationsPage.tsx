import { useCallback, useEffect, useState } from 'react';
import {
  apiDeleteNotification,
  apiGetNotifications,
  apiMarkAllRead,
  apiMarkRead,
  type Notification,
} from '../api';
import { Icon } from '../icons';
import EmptyState from '../EmptyState';
import { fmtAgo, fmtDateTime, plural } from '../format';

/** Backend texts are English; show Ukrainian ones for the known types. */
const TYPE_TEXT: Record<string, { icon: string; title?: string }> = {
  nft_created: { icon: '🎨', title: 'NFT створено' },
  welcome:     { icon: '👋', title: 'Ласкаво просимо до Marki' },
  purchase:    { icon: '🛍️', title: 'Покупка' },
  sale:        { icon: '💰', title: 'Продаж' },
  delivery:    { icon: '🚚', title: 'Доставка' },
  cod_order:   { icon: '🛒', title: 'Нове замовлення' },
};

export default function NotificationsPage() {
  const [items, setItems] = useState<Notification[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const reload = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      setItems(await apiGetNotifications());
    } catch (e: any) {
      setError(e?.message ?? 'Не вдалося завантажити сповіщення');
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => { reload(); }, [reload]);

  const markRead = async (id: string) => {
    try { await apiMarkRead(id); await reload(); } catch (e: any) { alert(e?.message ?? 'Не вдалося виконати дію'); }
  };

  const markAll = async () => {
    try { await apiMarkAllRead(); await reload(); } catch (e: any) { alert(e?.message ?? 'Не вдалося виконати дію'); }
  };

  const remove = async (id: string) => {
    try { await apiDeleteNotification(id); await reload(); } catch (e: any) { alert(e?.message ?? 'Не вдалося виконати дію'); }
  };

  const unreadCount = items.filter(i => !i.read).length;

  return (
    <div>
      <div className="page-head">
        <div>
          <h2>Сповіщення</h2>
          <p>{unreadCount === 0 ? 'Усе прочитано.' : `${unreadCount} ${plural(unreadCount, 'непрочитане', 'непрочитані', 'непрочитаних')}`}</p>
        </div>
        <div style={{ display: 'flex', gap: 8 }}>
          <button className="btn" onClick={reload} title="Оновити"><Icon.Refresh /></button>
          {unreadCount > 0 && (
            <button className="btn btn-primary" onClick={markAll}>
              <Icon.Check /> Прочитати всі
            </button>
          )}
        </div>
      </div>

      {error && <div className="error-banner">{error}</div>}

      {loading ? (
        <div className="spinner">Завантаження…</div>
      ) : items.length === 0 ? (
        <EmptyState icon="🔔" title="Сповіщень немає" text="Тут з’являтимуться покупки, продажі й зміни статусів доставок." />
      ) : (
        items.map(n => {
          const t = TYPE_TEXT[n.type];
          return (
            <div
              key={n.id}
              className="card"
              style={{ marginBottom: 10, opacity: n.read ? 0.75 : 1, cursor: n.read ? 'default' : 'pointer' }}
              onClick={() => { if (!n.read) markRead(n.id); }}
            >
              <div style={{ display: 'flex', gap: 12, alignItems: 'flex-start' }}>
                <span style={{ fontSize: 22, lineHeight: 1 }}>{t?.icon ?? '📌'}</span>
                <div style={{ flex: 1, minWidth: 0 }}>
                  <div style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
                    <h3 style={{ fontSize: 14, margin: 0 }}>{t?.title ?? n.title}</h3>
                    {!n.read && <span className="unread-dot" style={{ marginTop: 0 }} title="Непрочитане" />}
                  </div>
                  {n.body && <p className="sub" style={{ fontSize: 13, margin: '4px 0' }}>{n.body}</p>}
                  <div className="sub" style={{ fontSize: 11 }} title={fmtDateTime(n.createdAt)}>{fmtAgo(n.createdAt)}</div>
                </div>
                <button
                  className="btn ghost-danger"
                  title="Видалити сповіщення"
                  onClick={e => { e.stopPropagation(); remove(n.id); }}
                >
                  <Icon.Trash />
                </button>
              </div>
            </div>
          );
        })
      )}
    </div>
  );
}
