import type { ReactNode } from 'react';

/** Friendly placeholder for an empty list: icon, what it means, what to do next. */
export default function EmptyState({ icon, title, text, action }: {
  icon: string;
  title: string;
  text?: string;
  action?: ReactNode;
}) {
  return (
    <div className="empty-state">
      <div className="es-icon">{icon}</div>
      <div className="es-title">{title}</div>
      {text && <div className="es-sub">{text}</div>}
      {action}
    </div>
  );
}
