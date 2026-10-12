/** Placeholder cards shown while a grid loads, instead of a bare «Завантаження…». */
export function GridSkeleton({ count = 6 }: { count?: number }) {
  return (
    <div className="product-grid" aria-busy="true" aria-label="Завантаження">
      {Array.from({ length: count }, (_, i) => (
        <div key={i} className="card product-card skeleton-card">
          <div className="sk sk-img" />
          <div className="pc-body">
            <div className="sk sk-line" style={{ width: '70%' }} />
            <div className="sk sk-line" style={{ width: '45%' }} />
          </div>
        </div>
      ))}
    </div>
  );
}

export function ListSkeleton({ count = 3 }: { count?: number }) {
  return (
    <div className="order-list" aria-busy="true" aria-label="Завантаження">
      {Array.from({ length: count }, (_, i) => (
        <div key={i} className="card order-card">
          <div className="sk sk-line" style={{ width: '40%', height: 14 }} />
          <div className="sk sk-line" style={{ width: '65%' }} />
          <div className="sk sk-line" style={{ width: '90%', height: 26, marginTop: 14 }} />
        </div>
      ))}
    </div>
  );
}
