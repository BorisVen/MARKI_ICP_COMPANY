type IconProps = { size?: number };

const wrap = (path: React.ReactNode, size = 18) => (
  <svg
    width={size}
    height={size}
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    strokeWidth="2"
    strokeLinecap="round"
    strokeLinejoin="round"
  >
    {path}
  </svg>
);

export const Icon = {
  Dashboard: ({ size }: IconProps = {}) =>
    wrap(<><rect x="3" y="3" width="7" height="9" rx="1.5" /><rect x="14" y="3" width="7" height="5" rx="1.5" /><rect x="14" y="12" width="7" height="9" rx="1.5" /><rect x="3" y="16" width="7" height="5" rx="1.5" /></>, size),
  Truck: ({ size }: IconProps = {}) =>
    wrap(<><rect x="1" y="6" width="13" height="11" rx="1.5" /><path d="M14 9h4l3 4v4h-7" /><circle cx="6"  cy="19" r="2" /><circle cx="17" cy="19" r="2" /></>, size),
  Box: ({ size }: IconProps = {}) =>
    wrap(<><path d="M21 8 12 3 3 8v8l9 5 9-5z" /><path d="M3 8l9 5 9-5" /><path d="M12 13v9" /></>, size),
  Store: ({ size }: IconProps = {}) =>
    wrap(<><path d="M3 9l1.5-5h15L21 9" /><path d="M4 9v11h16V9" /><path d="M9 22V13h6v9" /></>, size),
  Feed: ({ size }: IconProps = {}) =>
    wrap(<><line x1="4" y1="6"  x2="20" y2="6"  /><line x1="4" y1="12" x2="20" y2="12" /><line x1="4" y1="18" x2="14" y2="18" /></>, size),
  Wallet: ({ size }: IconProps = {}) =>
    wrap(<><rect x="3" y="6" width="18" height="13" rx="2" /><path d="M16 13h2" /><path d="M3 10h18" /></>, size),
  Tag: ({ size }: IconProps = {}) =>
    wrap(<><path d="M20.6 13.4 13.4 20.6a2 2 0 0 1-2.8 0L3 13V3h10l7.6 7.6a2 2 0 0 1 0 2.8z" /><circle cx="7.5" cy="7.5" r="1.5" /></>, size),
  More: ({ size }: IconProps = {}) =>
    wrap(<><circle cx="5" cy="12" r="1" /><circle cx="12" cy="12" r="1" /><circle cx="19" cy="12" r="1" /></>, size),
  Upload: ({ size }: IconProps = {}) =>
    wrap(<><path d="M12 16V4" /><path d="m6 10 6-6 6 6" /><path d="M4 20h16" /></>, size),
  Bell: ({ size }: IconProps = {}) =>
    wrap(<><path d="M18 16V11a6 6 0 1 0-12 0v5l-2 3h16z" /><path d="M10 21a2 2 0 0 0 4 0" /></>, size),
  User: ({ size }: IconProps = {}) =>
    wrap(<><circle cx="12" cy="8" r="4" /><path d="M4 21a8 8 0 0 1 16 0" /></>, size),
  Plus: ({ size }: IconProps = {}) =>
    wrap(<><line x1="12" y1="5" x2="12" y2="19" /><line x1="5" y1="12" x2="19" y2="12" /></>, size),
  Trash: ({ size }: IconProps = {}) =>
    wrap(<><polyline points="3 6 5 6 21 6" /><path d="M19 6l-2 14H7L5 6" /><path d="M10 11v6M14 11v6" /></>, size),
  Refresh: ({ size }: IconProps = {}) =>
    wrap(<><path d="M21 12a9 9 0 1 1-3-6.7" /><polyline points="21 3 21 9 15 9" /></>, size),
  Check: ({ size }: IconProps = {}) =>
    wrap(<polyline points="5 12 10 17 19 8" />, size),
  Close: ({ size }: IconProps = {}) =>
    wrap(<><line x1="6" y1="6" x2="18" y2="18" /><line x1="18" y1="6" x2="6" y2="18" /></>, size),
  ChevronRight: ({ size }: IconProps = {}) =>
    wrap(<polyline points="9 6 15 12 9 18" />, size),
  Search: ({ size }: IconProps = {}) =>
    wrap(<><circle cx="11" cy="11" r="7" /><line x1="20" y1="20" x2="16.65" y2="16.65" /></>, size),
  QrCode: ({ size }: IconProps = {}) =>
    wrap(<><rect x="3" y="3" width="7" height="7" rx="1" /><rect x="14" y="3" width="7" height="7" rx="1" /><rect x="3" y="14" width="7" height="7" rx="1" /><path d="M14 14h3v3h-3z" /><path d="M19 17v4M17 19h4M14 19v2" /></>, size),
};
