import { ReactNode, useEffect, useState } from 'react';
import { Icon } from './icons';
import { useAuth } from './auth';
import { apiGetNotifications } from './api';

export type PageId =
  | 'dashboard'
  | 'crm'
  | 'nfts'
  | 'pricing'
  | 'marketplace'
  | 'feed'
  | 'wallets'
  | 'notifications'
  | 'profile';

type NavItem = {
  id: PageId;
  label: string;
  icon: ReactNode;
  /** Shown directly in the mobile bottom bar; the rest go under «Ще». */
  mobile?: boolean;
};

const NAV_GROUPS: { title: string; items: NavItem[] }[] = [
  {
    title: 'Бізнес',
    items: [
      { id: 'dashboard',   label: 'Панель',      icon: <Icon.Dashboard />, mobile: true },
      { id: 'crm',         label: 'Замовлення',  icon: <Icon.Truck />,     mobile: true },
      { id: 'nfts',        label: 'NFT',         icon: <Icon.Box />,       mobile: true },
      { id: 'marketplace', label: 'Маркетплейс', icon: <Icon.Store />,     mobile: true },
    ],
  },
  {
    title: 'Спільнота',
    items: [
      { id: 'feed',          label: 'Стрічка',    icon: <Icon.Feed /> },
      { id: 'notifications', label: 'Сповіщення', icon: <Icon.Bell /> },
    ],
  },
  {
    title: 'Акаунт',
    items: [
      { id: 'pricing', label: 'Тарифи',  icon: <Icon.Tag /> },
      { id: 'wallets', label: 'Гаманці', icon: <Icon.Wallet /> },
      { id: 'profile', label: 'Профіль', icon: <Icon.User /> },
    ],
  },
];

const ALL_NAV = NAV_GROUPS.flatMap(g => g.items);

type Props = {
  page: PageId;
  setPage: (p: PageId) => void;
  title: string;
  children: ReactNode;
};

export default function Shell({ page, setPage, title, children }: Props) {
  const { fbUser, user, logout } = useAuth();
  const [unread, setUnread] = useState(0);
  const [moreOpen, setMoreOpen] = useState(false);
  const displayName = user?.companyName || user?.name || fbUser?.email || 'Ви';

  // Each page shows its own heading; the title goes to the browser tab.
  useEffect(() => { document.title = `${title} · Marki`; }, [title]);

  // Unread badge: refresh on page change and every minute.
  useEffect(() => {
    let cancelled = false;
    const load = () => apiGetNotifications()
      .then(n => { if (!cancelled) setUnread(n.filter(x => !x.read).length); })
      .catch(() => {});
    load();
    const t = window.setInterval(load, 60_000);
    return () => { cancelled = true; window.clearInterval(t); };
  }, [page]);

  const go = (p: PageId) => {
    setPage(p);
    setMoreOpen(false);
    window.scrollTo({ top: 0 });
  };

  const mobileItems = ALL_NAV.filter(n => n.mobile);
  const moreItems = ALL_NAV.filter(n => !n.mobile);
  const moreActive = moreItems.some(n => n.id === page);

  const navButton = (item: NavItem) => (
    <button
      key={item.id}
      className={`nav-item ${page === item.id ? 'active' : ''}`}
      onClick={() => go(item.id)}
    >
      <span className="ic">{item.icon}</span>
      {item.label}
      {item.id === 'notifications' && unread > 0 && <span className="nav-count">{unread}</span>}
    </button>
  );

  return (
    <div className="shell">
      <aside className="sidebar">
        <div className="brand">Marki <span className="muted">· Бізнес</span></div>
        {NAV_GROUPS.map(g => (
          <div key={g.title} className="nav-group">
            <div className="nav-group-title">{g.title}</div>
            {g.items.map(navButton)}
          </div>
        ))}
      </aside>

      <main className="main">
        <header className="topbar">
          <h1>Marki</h1>
          <div className="who">
            <button className="icon-btn" title="Сповіщення" onClick={() => go('notifications')}>
              <Icon.Bell />
              {unread > 0 && <span className="dot-count">{unread > 9 ? '9+' : unread}</span>}
            </button>
            <button className="user-chip" onClick={() => go('profile')} title="Профіль">
              {user?.avatar
                ? <img src={user.avatar} alt="" />
                : <span className="avatar-letter">{displayName.slice(0, 1).toUpperCase()}</span>}
              <span className="user-name">{displayName}</span>
            </button>
            <button className="logout" onClick={logout}>Вийти</button>
          </div>
        </header>

        <div className="content">{children}</div>
      </main>

      {moreOpen && (
        <div className="sheet-backdrop" onClick={() => setMoreOpen(false)}>
          <div className="sheet" onClick={e => e.stopPropagation()}>
            <div className="sheet-handle" />
            {moreItems.map(navButton)}
          </div>
        </div>
      )}

      <nav className="mobile-nav">
        {mobileItems.map(item => (
          <button
            key={item.id}
            className={`nav-item ${page === item.id ? 'active' : ''}`}
            onClick={() => go(item.id)}
          >
            <span className="ic">{item.icon}</span>
            {item.label}
          </button>
        ))}
        <button className={`nav-item ${moreActive || moreOpen ? 'active' : ''}`} onClick={() => setMoreOpen(o => !o)}>
          <span className="ic"><Icon.More /></span>
          Ще
          {unread > 0 && <span className="dot-count" style={{ top: 6, right: '28%' }}>{unread > 9 ? '9+' : unread}</span>}
        </button>
      </nav>
    </div>
  );
}
