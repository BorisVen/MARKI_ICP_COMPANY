import { useCallback, useEffect, useMemo, useState } from 'react';
import {
  apiAcceptCodOrder,
  apiAddCheckpoint,
  apiBindNfc,
  apiConfirmReceipt,
  apiCreateDelivery,
  apiGetNFTs,
  apiListCodOrders,
  apiListDeliveries,
  apiSearchUsers,
  apiSyncNovaPoshta,
  apiUpdateCarrier,
  apiUpdateDeliveryStatus,
  apiVerifyNfc,
  type CodOrder,
  type Delivery,
  type DeliveryCheckpoint,
  type NFT,
  type PublicUser,
} from '../api';
import { DELIVERY_STATUSES, statusLabel, statusTone } from '../status';
import { useAuth } from '../auth';
import { Icon } from '../icons';

type Tab = 'orders' | 'deliveries' | 'nfc';
type DeliveryFilter = 'active' | 'all' | string;
type RouteCheckpoint = DeliveryCheckpoint & {
  displayLocation: string;
  previousLocation?: string;
  movement?: string;
};

const NEXT_STATUS: Record<string, string[]> = {
  pending: ['assigned', 'picked_up', 'failed'],
  assigned: ['picked_up', 'failed'],
  picked_up: ['in_transit', 'failed'],
  in_transit: ['out_for_delivery', 'delivered', 'failed'],
  out_for_delivery: ['delivered', 'failed'],
  delivered: ['verified'],
  verified: [],
  failed: ['pending'],
};

const FINISHED_DELIVERY = new Set(['delivered', 'verified', 'completed', 'cancelled']);

function formatDate(value?: string): string {
  if (!value) return '—';
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return date.toLocaleString('uk-UA', { day: 'numeric', month: 'short', hour: '2-digit', minute: '2-digit' });
}

function toLocalDateTimeValue(value?: string): string {
  const date = value ? new Date(value) : new Date();
  if (Number.isNaN(date.getTime())) return '';
  const local = new Date(date.getTime() - date.getTimezoneOffset() * 60_000);
  return local.toISOString().slice(0, 16);
}

function localDateTimeToIso(value: string): string | undefined {
  if (!value) return undefined;
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return undefined;
  return date.toISOString();
}

function formatMoney(value: number, currency?: string): string {
  return `${value} ${currency || ''}`.trim();
}

function shortId(value?: string): string {
  if (!value) return '—';
  return value.length > 12 ? `${value.slice(0, 6)}…${value.slice(-4)}` : value;
}

function searchValue(value: unknown): string {
  return String(value ?? '').toLowerCase();
}

function nftGroupId(nft: NFT): string {
  return nft.batchId || (nft as any).batch_id || (nft as any).masterNftId || (nft as any).master_nft_id || `single:${nft.id}`;
}

function nftEditionNumber(nft: NFT): number | null {
  const value = (nft as any).editionNumber ?? (nft as any).edition_number ?? nft.batchIndex ?? (nft as any).batch_index;
  return typeof value === 'number' ? value : null;
}

function nftImageUrl(nft: NFT): string {
  return nft.imageUrl || nft.image || '';
}

function orderMatches(order: CodOrder, query: string): boolean {
  if (!query) return true;
  const haystack = [
    order.id,
    order.nftTitle,
    order.buyerName,
    order.fullName,
    order.phone,
    order.deliveryAddress,
    order.nftCurrency,
    order.paymentCurrency,
  ].map(searchValue).join(' ');
  return haystack.includes(query);
}

function deliveryMatches(delivery: Delivery, query: string): boolean {
  if (!query) return true;
  const haystack = [
    delivery.id,
    delivery.orderId,
    delivery.nftTitle,
    delivery.buyerName,
    delivery.deliveryAddress,
    delivery.courierName,
    delivery.controllerName,
    delivery.npTrackingNumber,
    delivery.nfcUid,
    delivery.status,
  ].map(searchValue).join(' ');
  return haystack.includes(query);
}

function checkpointLocation(checkpoint: DeliveryCheckpoint, delivery: Delivery): string {
  const value = checkpoint.location?.trim();
  if (!value || value === '—') return delivery.deliveryAddress || '—';
  return value;
}

function routeCheckpoints(delivery: Delivery): RouteCheckpoint[] {
  let previousLocation: string | null = null;
  return [...(delivery.checkpoints ?? [])]
    .sort((a, b) => a.timestamp.localeCompare(b.timestamp))
    .map(checkpoint => {
      const displayLocation = checkpointLocation(checkpoint, delivery);
      const routed: RouteCheckpoint = { ...checkpoint, displayLocation };
      if (previousLocation && previousLocation !== displayLocation) {
        routed.previousLocation = previousLocation;
        routed.movement = `${previousLocation} → ${displayLocation}`;
      }
      previousLocation = displayLocation;
      return routed;
    })
    .sort((a, b) => b.timestamp.localeCompare(a.timestamp));
}

function latestFirst<T extends { createdAt: string }>(items: T[]): T[] {
  return [...items].sort((a, b) => b.createdAt.localeCompare(a.createdAt));
}

export default function CrmPage() {
  const [tab, setTab] = useState<Tab>('orders');
  const [orders, setOrders] = useState<CodOrder[]>([]);
  const [deliveries, setDeliveries] = useState<Delivery[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [lastUpdated, setLastUpdated] = useState<Date | null>(null);

  const reload = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const [o, d] = await Promise.all([apiListCodOrders(), apiListDeliveries()]);
      setOrders(o);
      setDeliveries(d);
      setLastUpdated(new Date());
    } catch (err: any) {
      setError(err?.message ?? 'Failed to load CRM');
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => { reload(); }, [reload]);

  const stats = useMemo(() => {
    const activeDeliveries = deliveries.filter(d => !FINISHED_DELIVERY.has(d.status)).length;
    return {
      pendingOrders: orders.filter(o => o.status === 'pending').length,
      activeDeliveries,
      problemDeliveries: deliveries.filter(d => d.status === 'failed').length,
      completedDeliveries: deliveries.filter(d => ['delivered', 'verified', 'completed'].includes(d.status)).length,
    };
  }, [orders, deliveries]);

  return (
    <div className="crm-page">
      <div className="page-head">
        <div>
          <h2>Замовлення і доставки</h2>
          <p>Приймай замовлення, відстежуй доставки й перевіряй товари за NFC.</p>
        </div>
        <div className="crm-head-actions">
          {lastUpdated && <span className="crm-last-updated">Оновлено о {lastUpdated.toLocaleTimeString('uk-UA', { hour: '2-digit', minute: '2-digit' })}</span>}
          <button className="btn" type="button" onClick={reload} disabled={loading}>
            <Icon.Refresh /> Оновити
          </button>
        </div>
      </div>

      <div className="crm-metrics">
        <MetricCard
          label="Нові замовлення"
          value={stats.pendingOrders}
          tone="pending"
          active={tab === 'orders'}
          onClick={() => setTab('orders')}
        />
        <MetricCard
          label="Активні доставки"
          value={stats.activeDeliveries}
          tone="info"
          active={tab === 'deliveries'}
          onClick={() => setTab('deliveries')}
        />
        <MetricCard
          label="Проблеми"
          value={stats.problemDeliveries}
          tone="danger"
          active={tab === 'deliveries'}
          onClick={() => setTab('deliveries')}
        />
        <MetricCard
          label="Закрито"
          value={stats.completedDeliveries}
          tone="success"
          active={tab === 'deliveries'}
          onClick={() => setTab('deliveries')}
        />
      </div>

      <div className="tabs crm-tabs">
        <button className={`tab ${tab === 'orders' ? 'active' : ''}`} type="button" onClick={() => setTab('orders')}>
          Замовлення<span className="count">{stats.pendingOrders}</span>
        </button>
        <button className={`tab ${tab === 'deliveries' ? 'active' : ''}`} type="button" onClick={() => setTab('deliveries')}>
          Доставки<span className="count">{deliveries.length}</span>
        </button>
        <button className={`tab ${tab === 'nfc' ? 'active' : ''}`} type="button" onClick={() => setTab('nfc')}>
          NFC
        </button>
      </div>

      {error && <div className="error-banner">{error}</div>}
      {loading && <div className="spinner">Завантаження…</div>}

      {!loading && tab === 'orders' && (
        <OrdersTab orders={orders} deliveries={deliveries} reload={reload} />
      )}
      {!loading && tab === 'deliveries' && (
        <DeliveriesTab deliveries={deliveries} reload={reload} />
      )}
      {!loading && tab === 'nfc' && <NfcTab />}
    </div>
  );
}

function MetricCard({
  label,
  value,
  tone,
  active,
  onClick,
}: {
  label: string;
  value: number;
  tone: 'pending' | 'info' | 'danger' | 'success';
  active: boolean;
  onClick: () => void;
}) {
  return (
    <button className={`crm-metric ${active ? 'active' : ''} tone-${tone}`} type="button" onClick={onClick}>
      <span>{label}</span>
      <strong>{value}</strong>
    </button>
  );
}

// ── Orders ───────────────────────────────────────────────────────────────────

function OrdersTab({
  orders,
  deliveries,
  reload,
}: {
  orders: CodOrder[];
  deliveries: Delivery[];
  reload: () => Promise<void>;
}) {
  const [query, setQuery] = useState('');
  const [accepting, setAccepting] = useState<CodOrder | null>(null);
  const normalized = query.trim().toLowerCase();

  const pending = useMemo(
    () => latestFirst(orders.filter(o => o.status === 'pending' && orderMatches(o, normalized))),
    [orders, normalized],
  );
  const history = useMemo(
    () => latestFirst(orders.filter(o => o.status !== 'pending' && orderMatches(o, normalized))),
    [orders, normalized],
  );

  const deliveryByOrder = useMemo(() => {
    const map = new Map<string, Delivery>();
    deliveries.forEach(delivery => {
      if (delivery.orderId) map.set(delivery.orderId, delivery);
    });
    return map;
  }, [deliveries]);

  return (
    <div className="crm-stack">
      <div className="crm-toolbar">
        <div className="crm-search">
          <Icon.Search />
          <input
            value={query}
            onChange={e => setQuery(e.target.value)}
            placeholder="Пошук: клієнт, телефон, NFT, адреса"
          />
        </div>
      </div>

      <section className="crm-section">
        <div className="crm-section-head">
          <h3>Черга</h3>
          <span className="muted">{pending.length} до обробки</span>
        </div>

        {pending.length === 0 ? (
          <div className="empty">Нових замовлень немає.</div>
        ) : (
          <div className="crm-order-grid">
            {pending.map(order => (
              <article className="crm-order-card" key={order.id}>
                <div className="crm-card-top">
                  <div>
                    <h3>{order.nftTitle}</h3>
                    <p>{order.fullName || order.buyerName}</p>
                  </div>
                  <span className={`badge ${statusTone(order.status)}`}>{statusLabel(order.status)}</span>
                </div>

                <div className="crm-fact-grid">
                  <Fact label="Телефон" value={order.phone || '—'} />
                  <Fact label="Сума" value={formatMoney(order.price, order.nftCurrency)} />
                  <Fact label="Адреса" value={order.deliveryAddress || '—'} wide />
                  <Fact label="Дата" value={formatDate(order.createdAt)} />
                </div>

                <div className="crm-card-actions">
                  <span className="muted">#{shortId(order.id)}</span>
                  <button className="btn btn-primary" type="button" onClick={() => setAccepting(order)}>
                    <Icon.Check /> Прийняти
                  </button>
                </div>
              </article>
            ))}
          </div>
        )}
      </section>

      <section className="crm-section">
        <div className="crm-section-head">
          <h3>Історія замовлень</h3>
          <span className="muted">{history.length}</span>
        </div>

        {history.length === 0 ? (
          <div className="empty">Історія порожня.</div>
        ) : (
          <div className="table-wrap">
            <div className="table-scroll">
              <table className="table crm-table">
                <thead>
                  <tr>
                    <th>NFT</th>
                    <th>Клієнт</th>
                    <th>Сума</th>
                    <th>Замовлення</th>
                    <th>Доставка</th>
                    <th>Дата</th>
                  </tr>
                </thead>
                <tbody>
                  {history.map(order => {
                    const delivery = deliveryByOrder.get(order.id);
                    return (
                      <tr key={order.id}>
                        <td>{order.nftTitle}</td>
                        <td>
                          <div>{order.buyerName}</div>
                          <div className="muted">{order.phone || order.fullName}</div>
                        </td>
                        <td>{formatMoney(order.price, order.nftCurrency)}</td>
                        <td><span className={`badge ${statusTone(order.status)}`}>{statusLabel(order.status)}</span></td>
                        <td>
                          {delivery ? (
                            <span className={`badge ${statusTone(delivery.status)}`}>{statusLabel(delivery.status)}</span>
                          ) : (
                            <span className="muted">—</span>
                          )}
                        </td>
                        <td>{formatDate(order.createdAt)}</td>
                      </tr>
                    );
                  })}
                </tbody>
              </table>
            </div>
          </div>
        )}
      </section>

      {accepting && (
        <AcceptOrderModal
          order={accepting}
          onClose={() => setAccepting(null)}
          onDone={async () => {
            setAccepting(null);
            await reload();
          }}
        />
      )}
    </div>
  );
}

function AcceptOrderModal({
  order,
  onClose,
  onDone,
}: {
  order: CodOrder;
  onClose: () => void;
  onDone: () => Promise<void> | void;
}) {
  const [carrierType, setCarrierType] = useState<'self' | 'nova_poshta'>('self');
  const [npTrackingNumber, setNpTrackingNumber] = useState('');
  const [courier, setCourier] = useState<PublicUser | null>(null);
  const [courierUid, setCourierUid] = useState('');
  const [controller, setController] = useState<PublicUser | null>(null);
  const [controllerUid, setControllerUid] = useState('');
  const [nfcUid, setNfcUid] = useState('');
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  const courierId = courier?.uid ?? courierUid.trim();
  const controllerId = controller?.uid ?? controllerUid.trim();

  const submit = async () => {
    setBusy(true);
    setErr(null);
    try {
      await apiAcceptCodOrder(order.id, {
        carrierType,
        npTrackingNumber: carrierType === 'nova_poshta' ? npTrackingNumber.trim() || undefined : undefined,
        courierId: carrierType === 'self' ? courierId || undefined : undefined,
        controllerId: carrierType === 'self' ? controllerId || undefined : undefined,
        nfcUid: nfcUid.trim() || undefined,
      });
      await onDone();
    } catch (e: any) {
      setErr(e?.message ?? 'Accept failed');
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="modal-backdrop" onClick={busy ? undefined : onClose}>
      <div className="modal crm-modal" onClick={e => e.stopPropagation()}>
        <div className="crm-modal-head">
          <div>
            <h3>Прийняти замовлення</h3>
            <p className="sub">#{shortId(order.id)} · {order.nftTitle}</p>
          </div>
          <button className="btn" type="button" onClick={onClose} disabled={busy}>
            <Icon.Close />
          </button>
        </div>

        <div className="crm-readonly">
          <Fact label="Клієнт" value={order.fullName || order.buyerName} />
          <Fact label="Телефон" value={order.phone || '—'} />
          <Fact label="Сума" value={formatMoney(order.price, order.nftCurrency)} />
          <Fact label="Адреса" value={order.deliveryAddress || '—'} wide />
        </div>

        <div className="field">
          <label>Перевізник</label>
          <div className="segmented">
            <button
              className={carrierType === 'self' ? 'active' : ''}
              type="button"
              onClick={() => setCarrierType('self')}
            >
              Власний кур’єр
            </button>
            <button
              className={carrierType === 'nova_poshta' ? 'active' : ''}
              type="button"
              onClick={() => setCarrierType('nova_poshta')}
            >
              Нова Пошта
            </button>
          </div>
        </div>

        {carrierType === 'nova_poshta' ? (
          <div className="field">
            <label>ТТН</label>
            <input value={npTrackingNumber} onChange={e => setNpTrackingNumber(e.target.value)} placeholder="Номер накладної" />
          </div>
        ) : (
          <div className="crm-lookup-grid">
            <UserLookup
              label="Кур’єр"
              selected={courier}
              manualUid={courierUid}
              onSelect={setCourier}
              onManualUid={setCourierUid}
              onClear={() => setCourier(null)}
            />
            <UserLookup
              label="Контролер"
              selected={controller}
              manualUid={controllerUid}
              onSelect={setController}
              onManualUid={setControllerUid}
              onClear={() => setController(null)}
            />
          </div>
        )}

        <div className="field">
          <label>NFC UID</label>
          <input value={nfcUid} onChange={e => setNfcUid(e.target.value)} placeholder="04:A2:B5:..." />
        </div>

        {err && <div className="error-banner">{err}</div>}

        <div className="actions">
          <button className="btn" type="button" onClick={onClose} disabled={busy}>Скасувати</button>
          <button className="btn btn-primary" type="button" onClick={submit} disabled={busy}>
            {busy ? 'Створюємо…' : 'Створити доставку'}
          </button>
        </div>
      </div>
    </div>
  );
}

// ── Deliveries ───────────────────────────────────────────────────────────────

function DeliveriesTab({
  deliveries,
  reload,
}: {
  deliveries: Delivery[];
  reload: () => Promise<void>;
}) {
  const [query, setQuery] = useState('');
  const [filter, setFilter] = useState<DeliveryFilter>('active');
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [detailClosed, setDetailClosed] = useState(false);
  const [chainModalOpen, setChainModalOpen] = useState(false);
  const normalized = query.trim().toLowerCase();

  const filtered = useMemo(() => {
    return latestFirst(deliveries)
      .filter(delivery => {
        if (filter === 'active') return !FINISHED_DELIVERY.has(delivery.status);
        if (filter === 'all') return true;
        return delivery.status === filter;
      })
      .filter(delivery => deliveryMatches(delivery, normalized));
  }, [deliveries, filter, normalized]);

  const selected = useMemo(
    () => selectedId ? deliveries.find(delivery => delivery.id === selectedId) ?? null : null,
    [deliveries, selectedId],
  );

  useEffect(() => {
    if (!detailClosed && !selectedId && filtered[0]) setSelectedId(filtered[0].id);
    if (selectedId && !deliveries.some(delivery => delivery.id === selectedId)) {
      setSelectedId(filtered[0]?.id ?? null);
    }
  }, [deliveries, filtered, selectedId, detailClosed]);

  return (
    <div className="crm-stack">
      <div className="crm-toolbar">
        <div className="crm-search">
          <Icon.Search />
          <input
            value={query}
            onChange={e => setQuery(e.target.value)}
            placeholder="Пошук: клієнт, NFT, адреса, ТТН, NFC"
          />
        </div>
        <div className="crm-filter-row">
          <button className="btn btn-primary" type="button" onClick={() => setChainModalOpen(true)}>
            <Icon.Plus /> Ланцюжок NFT
          </button>
          <button className={`chip ${filter === 'active' ? 'active' : ''}`} type="button" onClick={() => setFilter('active')}>Активні</button>
          <button className={`chip ${filter === 'all' ? 'active' : ''}`} type="button" onClick={() => setFilter('all')}>Усі</button>
          {DELIVERY_STATUSES.map(status => (
            <button
              key={status.value}
              className={`chip ${filter === status.value ? 'active' : ''}`}
              type="button"
              onClick={() => setFilter(status.value)}
            >
              {status.label}
            </button>
          ))}
        </div>
      </div>

      {filtered.length === 0 ? (
        <div className="empty">Немає доставок під вибраний фільтр.</div>
      ) : (
        <div className={`crm-split ${selected ? '' : 'no-detail'}`}>
          <div className="table-wrap crm-delivery-list">
            <div className="table-scroll">
              <table className="table crm-table">
                <thead>
                  <tr>
                    <th>Статус</th>
                    <th>NFT</th>
                    <th>Клієнт</th>
                    <th>Перевізник</th>
                    <th>Виконавець</th>
                    <th>Оновлено</th>
                  </tr>
                </thead>
                <tbody>
                  {filtered.map(delivery => (
                    <tr
                      key={delivery.id}
                      className={selected?.id === delivery.id ? 'selected-row' : ''}
                      onClick={() => {
                        setDetailClosed(false);
                        setSelectedId(delivery.id);
                      }}
                    >
                      <td><span className={`badge ${statusTone(delivery.status)}`}>{statusLabel(delivery.status)}</span></td>
                      <td>
                        <div>{delivery.nftTitle}</div>
                        <div className="muted">#{shortId(delivery.id)}</div>
                      </td>
                      <td>
                        <div>{delivery.buyerName}</div>
                        <div className="muted">{delivery.deliveryAddress}</div>
                      </td>
                      <td>{delivery.carrierType === 'nova_poshta' ? 'Нова Пошта' : 'Власний'}</td>
                      <td>{delivery.courierName || delivery.controllerName || shortId(delivery.courierId || delivery.controllerId)}</td>
                      <td>{formatDate(delivery.updatedAt || delivery.createdAt)}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </div>

          {selected && (
            <DeliveryDetail
              key={selected.id}
              delivery={selected}
              onChanged={async () => { await reload(); }}
              onClose={() => {
                setDetailClosed(true);
                setSelectedId(null);
              }}
            />
          )}
        </div>
      )}

      {chainModalOpen && (
        <NftChainModal
          deliveries={deliveries}
          onClose={() => setChainModalOpen(false)}
          onDone={async () => {
            setChainModalOpen(false);
            await reload();
          }}
        />
      )}
    </div>
  );
}

function DeliveryDetail({
  delivery,
  onChanged,
  onClose,
}: {
  delivery: Delivery;
  onChanged: (delivery: Delivery) => Promise<void> | void;
  onClose: () => void;
}) {
  const [busy, setBusy] = useState<string | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [newStatus, setNewStatus] = useState(delivery.status);
  const [carrierType, setCarrierType] = useState<'self' | 'nova_poshta'>(delivery.carrierType);
  const [npTrackingNumber, setNpTrackingNumber] = useState(delivery.npTrackingNumber ?? '');
  const [courier, setCourier] = useState<PublicUser | null>(null);
  const [courierUid, setCourierUid] = useState(delivery.courierId ?? '');
  const [controller, setController] = useState<PublicUser | null>(null);
  const [controllerUid, setControllerUid] = useState(delivery.controllerId ?? '');
  const [ckStatus, setCkStatus] = useState('in_transit');
  const [ckLocation, setCkLocation] = useState('');
  const [ckTimestamp, setCkTimestamp] = useState(() => toLocalDateTimeValue());
  const [ckNote, setCkNote] = useState('');

  useEffect(() => {
    setNewStatus(delivery.status);
    setCarrierType(delivery.carrierType);
    setNpTrackingNumber(delivery.npTrackingNumber ?? '');
    setCourier(null);
    setCourierUid(delivery.courierId ?? '');
    setController(null);
    setControllerUid(delivery.controllerId ?? '');
  }, [delivery]);

  const wrap = async <T,>(key: string, fn: () => Promise<T>) => {
    setBusy(key);
    setErr(null);
    try {
      return await fn();
    } catch (e: any) {
      setErr(e?.message ?? 'Failed');
      return undefined;
    } finally {
      setBusy(null);
    }
  };

  const updateStatus = async (status = newStatus) => {
    const next = await wrap(`status:${status}`, () => apiUpdateDeliveryStatus(delivery.id, status));
    if (next) await onChanged(next);
  };

  const saveCarrier = async () => {
    const next = await wrap('carrier', () => apiUpdateCarrier(delivery.id, {
      carrierType,
      npTrackingNumber: carrierType === 'nova_poshta' ? npTrackingNumber.trim() || undefined : undefined,
      courierId: carrierType === 'self' ? (courier?.uid ?? courierUid.trim()) || undefined : undefined,
      controllerId: carrierType === 'self' ? (controller?.uid ?? controllerUid.trim()) || undefined : undefined,
    }));
    if (next) await onChanged(next);
  };

  const addCheckpoint = async () => {
    if (!ckLocation.trim()) return;
    const next = await wrap('checkpoint', () => apiAddCheckpoint(delivery.id, {
      status: ckStatus,
      location: ckLocation.trim(),
      timestamp: localDateTimeToIso(ckTimestamp),
      note: ckNote.trim() || undefined,
    }));
    if (next) {
      setCkLocation('');
      setCkTimestamp(toLocalDateTimeValue());
      setCkNote('');
      await onChanged(next);
    }
  };

  const syncNovaPoshta = async () => {
    const next = await wrap('np', () => apiSyncNovaPoshta(delivery.id));
    if (next) await onChanged(next);
  };

  const confirmReceipt = async () => {
    if (!window.confirm('Підтвердити отримання покупцем?')) return;
    const next = await wrap('confirm', () => apiConfirmReceipt(delivery.id));
    if (next) await onChanged(next);
  };

  const quickStatuses = NEXT_STATUS[delivery.status] ?? [];
  const checkpoints = routeCheckpoints(delivery);

  return (
    <aside className="crm-detail-panel">
      <div className="crm-detail-head">
        <div>
          <h3>{delivery.nftTitle}</h3>
          <p>{delivery.buyerName}</p>
        </div>
        <button className="btn" type="button" onClick={onClose}>
          <Icon.Close />
        </button>
      </div>

      <div className="crm-badge-row">
        <span className={`badge ${statusTone(delivery.status)}`}>{statusLabel(delivery.status)}</span>
        <span className="badge badge-muted">{delivery.carrierType === 'nova_poshta' ? 'Нова Пошта' : 'Власний кур’єр'}</span>
        {delivery.nfcVerified && <span className="badge badge-success">NFC OK</span>}
      </div>

      {err && <div className="error-banner">{err}</div>}

      <div className="crm-readonly">
        <Fact label="Адреса" value={delivery.deliveryAddress || '—'} wide />
        <Fact label="Кур’єр" value={delivery.courierName || shortId(delivery.courierId)} />
        <Fact label="Контролер" value={delivery.controllerName || shortId(delivery.controllerId)} />
        <Fact label="ТТН" value={delivery.npTrackingNumber || '—'} />
        <Fact label="NFC UID" value={delivery.nfcUid || '—'} />
        <Fact label="Створена" value={formatDate(delivery.createdAt)} />
      </div>

      {quickStatuses.length > 0 && (
        <section className="crm-panel-section">
          <h4>Швидка дія</h4>
          <div className="crm-quick-actions">
            {quickStatuses.map(status => (
              <button
                key={status}
                className={`btn ${status === 'failed' ? 'btn-danger' : 'btn-primary'}`}
                type="button"
                disabled={busy === `status:${status}`}
                onClick={() => updateStatus(status)}
              >
                {busy === `status:${status}` ? '…' : statusLabel(status)}
              </button>
            ))}
          </div>
        </section>
      )}

      <section className="crm-panel-section">
        <h4>Статус</h4>
        <div className="crm-inline-form">
          <select value={newStatus} onChange={e => setNewStatus(e.target.value)}>
            {DELIVERY_STATUSES.map(status => (
              <option key={status.value} value={status.value}>{status.label}</option>
            ))}
          </select>
          <button className="btn btn-primary" type="button" disabled={busy === `status:${newStatus}`} onClick={() => updateStatus()}>
            Зберегти
          </button>
        </div>
      </section>

      <section className="crm-panel-section">
        <h4>Перевізник і команда</h4>
        <div className="field">
          <label>Перевізник</label>
          <div className="segmented">
            <button className={carrierType === 'self' ? 'active' : ''} type="button" onClick={() => setCarrierType('self')}>Власний</button>
            <button className={carrierType === 'nova_poshta' ? 'active' : ''} type="button" onClick={() => setCarrierType('nova_poshta')}>Нова Пошта</button>
          </div>
        </div>

        {carrierType === 'nova_poshta' ? (
          <div className="field">
            <label>ТТН</label>
            <input value={npTrackingNumber} onChange={e => setNpTrackingNumber(e.target.value)} />
          </div>
        ) : (
          <div className="crm-lookup-grid">
            <UserLookup
              label="Кур’єр"
              selected={courier}
              manualUid={courierUid}
              onSelect={setCourier}
              onManualUid={setCourierUid}
              onClear={() => setCourier(null)}
            />
            <UserLookup
              label="Контролер"
              selected={controller}
              manualUid={controllerUid}
              onSelect={setController}
              onManualUid={setControllerUid}
              onClear={() => setController(null)}
            />
          </div>
        )}

        <button className="btn btn-primary btn-block" type="button" disabled={busy === 'carrier'} onClick={saveCarrier}>
          {busy === 'carrier' ? 'Зберігаємо…' : 'Зберегти призначення'}
        </button>
      </section>

      {delivery.carrierType === 'nova_poshta' && (
        <button className="btn btn-block" type="button" disabled={busy === 'np'} onClick={syncNovaPoshta}>
          <Icon.Refresh /> {busy === 'np' ? 'Синхронізація…' : 'Синхронізувати Нова Пошта'}
        </button>
      )}

      {!delivery.customerReceived && (
        <button className="btn btn-primary btn-block" type="button" disabled={busy === 'confirm'} onClick={confirmReceipt}>
          <Icon.Check /> {busy === 'confirm' ? 'Підтверджуємо…' : 'Підтвердити отримання'}
        </button>
      )}

      <section className="crm-panel-section">
        <h4>Чекпоінт</h4>
        <div className="field">
          <label>Статус</label>
          <select value={ckStatus} onChange={e => setCkStatus(e.target.value)}>
            {DELIVERY_STATUSES.map(status => (
              <option key={status.value} value={status.value}>{status.label}</option>
            ))}
          </select>
        </div>
        <div className="field">
          <label>Локація</label>
          <input value={ckLocation} onChange={e => setCkLocation(e.target.value)} placeholder="Склад / Адреса / Відділення" />
        </div>
        <div className="field">
          <label>Час події</label>
          <input type="datetime-local" value={ckTimestamp} onChange={e => setCkTimestamp(e.target.value)} />
        </div>
        <div className="field">
          <label>Нотатка</label>
          <input value={ckNote} onChange={e => setCkNote(e.target.value)} />
        </div>
        <button className="btn btn-primary btn-block" type="button" disabled={busy === 'checkpoint' || !ckLocation.trim()} onClick={addCheckpoint}>
          {busy === 'checkpoint' ? 'Додаємо…' : 'Додати чекпоінт'}
        </button>
      </section>

      <section className="crm-panel-section">
        <h4>Історія</h4>
        {checkpoints.length === 0 ? (
          <div className="muted">Поки немає чекпоінтів.</div>
        ) : (
          <div className="crm-timeline">
            {checkpoints.map(checkpoint => (
              <div className="crm-timeline-item" key={checkpoint.id}>
                <span className={`badge ${statusTone(checkpoint.status)}`}>{statusLabel(checkpoint.status)}</span>
                {checkpoint.previousLocation ? (
                  <strong className="crm-route">
                    <span>{checkpoint.previousLocation}</span>
                    <span className="crm-route-arrow">→</span>
                    <span>{checkpoint.displayLocation}</span>
                  </strong>
                ) : (
                  <strong>{checkpoint.displayLocation}</strong>
                )}
                <small>
                  {formatDate(checkpoint.timestamp)}
                  {checkpoint.recordedByName ? ` · ${checkpoint.recordedByName}` : ''}
                </small>
                {checkpoint.note && <p>{checkpoint.note}</p>}
              </div>
            ))}
          </div>
        )}
      </section>
    </aside>
  );
}

function NftChainModal({
  deliveries,
  onClose,
  onDone,
}: {
  deliveries: Delivery[];
  onClose: () => void;
  onDone: () => Promise<void> | void;
}) {
  const { user } = useAuth();
  const [nfts, setNfts] = useState<NFT[]>([]);
  const [query, setQuery] = useState('');
  const [activeGroup, setActiveGroup] = useState<string>('all');
  const [selectedIds, setSelectedIds] = useState<string[]>([]);
  const [status, setStatus] = useState('in_transit');
  const [location, setLocation] = useState('');
  const [eventTime, setEventTime] = useState(() => toLocalDateTimeValue());
  const [note, setNote] = useState('');
  const [createMissing, setCreateMissing] = useState(true);
  const [recipient, setRecipient] = useState<PublicUser | null>(null);
  const [recipientUid, setRecipientUid] = useState('');
  const [address, setAddress] = useState('');
  const [busy, setBusy] = useState(false);
  const [loading, setLoading] = useState(true);
  const [err, setErr] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    apiGetNFTs()
      .then(items => {
        if (!cancelled) {
          const ordered = [...items].sort((a, b) => {
            const groupCompare = nftGroupId(a).localeCompare(nftGroupId(b));
            if (groupCompare !== 0) return groupCompare;
            return (nftEditionNumber(a) ?? 9999) - (nftEditionNumber(b) ?? 9999);
          });
          const groups = new Map<string, NFT[]>();
          ordered.forEach(nft => {
            const key = nftGroupId(nft);
            groups.set(key, [...(groups.get(key) ?? []), nft]);
          });
          const largestGroup = [...groups.entries()].sort((a, b) => b[1].length - a[1].length)[0];
          setNfts(ordered);
          if (largestGroup && largestGroup[1].length > 1) {
            setActiveGroup(largestGroup[0]);
            setSelectedIds(largestGroup[1].map(nft => nft.id));
          } else {
            setSelectedIds(ordered.length === 1 ? [ordered[0].id] : []);
          }
        }
      })
      .catch((e: any) => { if (!cancelled) setErr(e?.message ?? 'Failed to load NFTs'); })
      .finally(() => { if (!cancelled) setLoading(false); });
    return () => { cancelled = true; };
  }, []);

  const deliveryByNft = useMemo(() => {
    const map = new Map<string, Delivery>();
    latestFirst(deliveries).forEach(delivery => {
      if (!map.has(delivery.nftId)) map.set(delivery.nftId, delivery);
    });
    return map;
  }, [deliveries]);

  const nftGroups = useMemo(() => {
    const groups = new Map<string, NFT[]>();
    nfts.forEach(nft => {
      const key = nftGroupId(nft);
      groups.set(key, [...(groups.get(key) ?? []), nft]);
    });

    return [...groups.entries()]
      .map(([id, items]) => {
        const sorted = [...items].sort((a, b) => (nftEditionNumber(a) ?? 9999) - (nftEditionNumber(b) ?? 9999));
        const master = sorted.find(nft => nftEditionNumber(nft) === 0) ?? sorted[0];
        const selectedCount = sorted.filter(nft => selectedIds.includes(nft.id)).length;
        const existingChainCount = sorted.filter(nft => deliveryByNft.has(nft.id)).length;
        const mintedCount = sorted.filter(nft => nft.tokenId).length;
        return {
          id,
          items: sorted,
          title: master?.batchName || master?.title?.replace(/\s+#\d+.*$/u, '') || 'NFT group',
          image: master ? nftImageUrl(master) || nftImageUrl(sorted.find(nft => nftImageUrl(nft)) ?? master) : '',
          selectedCount,
          existingChainCount,
          mintedCount,
        };
      })
      .sort((a, b) => b.items.length - a.items.length || a.title.localeCompare(b.title));
  }, [deliveryByNft, nfts, selectedIds]);

  const filteredNfts = useMemo(() => {
    const q = query.trim().toLowerCase();
    const scoped = activeGroup === 'all' ? nfts : nfts.filter(nft => nftGroupId(nft) === activeGroup);
    if (!q) return scoped;
    return scoped.filter(nft => [
      nft.id,
      nft.title,
      nft.tokenId,
      nft.nfcUid,
      nft.batchName,
      nft.category,
      ...(nft.tags ?? []),
    ].map(searchValue).join(' ').includes(q));
  }, [activeGroup, nfts, query]);

  const selectedNfts = useMemo(
    () => nfts.filter(nft => selectedIds.includes(nft.id)),
    [nfts, selectedIds],
  );
  const missingCount = selectedNfts.filter(nft => !deliveryByNft.has(nft.id)).length;
  const existingCount = selectedNfts.length - missingCount;
  const allVisibleSelected = filteredNfts.length > 0 && filteredNfts.every(nft => selectedIds.includes(nft.id));

  const selectedGroup = nftGroups.find(group => group.id === activeGroup);

  const toggleNft = (id: string) => {
    setSelectedIds(prev => prev.includes(id) ? prev.filter(x => x !== id) : [...prev, id]);
  };

  const selectGroup = (groupId: string) => {
    const group = nftGroups.find(item => item.id === groupId);
    if (!group) return;
    setActiveGroup(groupId);
    setQuery('');
    setSelectedIds(group.items.map(nft => nft.id));
  };

  const toggleVisible = () => {
    const visibleIds = filteredNfts.map(nft => nft.id);
    setSelectedIds(prev => {
      if (visibleIds.every(id => prev.includes(id))) {
        return prev.filter(id => !visibleIds.includes(id));
      }
      return Array.from(new Set([...prev, ...visibleIds]));
    });
  };

  const submit = async () => {
    setErr(null);
    if (selectedNfts.length === 0) {
      setErr('Вибери хоча б один NFT.');
      return;
    }
    if (!location.trim()) {
      setErr('Вкажи куди прибув товар.');
      return;
    }

    const timestamp = localDateTimeToIso(eventTime);
    const buyerId = recipient?.uid || recipientUid.trim() || user?.uid || '';
    if (createMissing && missingCount > 0 && !buyerId) {
      setErr('Потрібен отримувач або поточний користувач для нових ланцюжків.');
      return;
    }

    setBusy(true);
    try {
      let created = 0;
      let updated = 0;
      let skipped = 0;
      const deliveryAddress = address.trim() || location.trim();
      const checkpointNote = note.trim() || undefined;

      for (const nft of selectedNfts) {
        const existing = deliveryByNft.get(nft.id);
        if (existing) {
          await apiAddCheckpoint(existing.id, {
            status,
            location: location.trim(),
            timestamp,
            note: checkpointNote,
          });
          updated += 1;
          continue;
        }

        if (!createMissing) {
          skipped += 1;
          continue;
        }

        await apiCreateDelivery({
          nftId: nft.id,
          buyerId,
          deliveryAddress,
          carrierType: 'self',
          createdAt: timestamp,
          status,
          initialLocation: location.trim(),
          initialNote: checkpointNote,
        });
        created += 1;
      }

      if (created === 0 && updated === 0 && skipped > 0) {
        setErr('Для вибраних NFT немає ланцюжків. Увімкни створення нових ланцюжків.');
        return;
      }
      await onDone();
    } catch (e: any) {
      setErr(e?.message ?? 'Failed to save chain');
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="modal-backdrop" onClick={busy ? undefined : onClose}>
      <div className="modal crm-modal chain-modal" onClick={e => e.stopPropagation()}>
        <div className="crm-modal-head">
          <div>
            <h3>Ланцюжок NFT</h3>
            <p className="sub">{selectedNfts.length} вибрано · {existingCount} з ланцюжком · {missingCount} без ланцюжка</p>
          </div>
          <button className="btn" type="button" onClick={onClose} disabled={busy}>
            <Icon.Close />
          </button>
        </div>

        {loading ? (
          <div className="spinner">Завантаження…</div>
        ) : (
          <div className="chain-layout">
            <section className="chain-picker">
              <div className="chain-summary-strip">
                <div>
                  <strong>{selectedNfts.length}</strong>
                  <span>вибрано</span>
                </div>
                <div>
                  <strong>{filteredNfts.length}</strong>
                  <span>у списку</span>
                </div>
                <div>
                  <strong>{existingCount}</strong>
                  <span>з ланцюжком</span>
                </div>
              </div>

              {nftGroups.length > 1 && (
                <div className="chain-group-list">
                  <button
                    className={`chain-group-card ${activeGroup === 'all' ? 'active' : ''}`}
                    type="button"
                    onClick={() => setActiveGroup('all')}
                  >
                    <span className="chain-group-thumb empty-thumb">Усі</span>
                    <span>
                      <strong>Усі NFT</strong>
                      <small>{nfts.length} усього · {selectedIds.length} вибрано</small>
                    </span>
                  </button>
                  {nftGroups.map(group => (
                    <button
                      className={`chain-group-card ${activeGroup === group.id ? 'active' : ''}`}
                      type="button"
                      key={group.id}
                      onClick={() => selectGroup(group.id)}
                    >
                      {group.image ? (
                        <img className="chain-group-thumb" src={group.image} alt="" />
                      ) : (
                        <span className="chain-group-thumb empty-thumb">{group.items.length}</span>
                      )}
                      <span>
                        <strong>{group.title}</strong>
                        <small>
                          {group.items.length} NFT · {group.mintedCount} mint · {group.selectedCount} вибрано
                        </small>
                      </span>
                    </button>
                  ))}
                </div>
              )}

              <div className="crm-search">
                <Icon.Search />
                <input value={query} onChange={e => setQuery(e.target.value)} placeholder={selectedGroup ? `Пошук у ${selectedGroup.title}` : 'NFT, mint, batch, tag'} />
              </div>

              <div className="chain-picker-actions">
                {selectedGroup && (
                  <button className="btn btn-primary" type="button" onClick={() => selectGroup(selectedGroup.id)}>
                    Вибрати групу
                  </button>
                )}
                <button className="btn" type="button" onClick={toggleVisible} disabled={filteredNfts.length === 0}>
                  {allVisibleSelected ? 'Зняти видимі' : 'Вибрати видимі'}
                </button>
                <button className="btn" type="button" onClick={() => setSelectedIds(nfts.map(nft => nft.id))} disabled={nfts.length === 0}>
                  Усі
                </button>
                <button className="btn" type="button" onClick={() => setSelectedIds([])} disabled={selectedIds.length === 0}>
                  Очистити
                </button>
              </div>

              <div className="nft-select-list">
                {filteredNfts.length === 0 ? (
                  <div className="empty">NFT не знайдено.</div>
                ) : filteredNfts.map(nft => {
                  const delivery = deliveryByNft.get(nft.id);
                  const image = nftImageUrl(nft);
                  return (
                    <label className="nft-select-row" key={nft.id}>
                      <input
                        type="checkbox"
                        checked={selectedIds.includes(nft.id)}
                        onChange={() => toggleNft(nft.id)}
                      />
                      {image ? <img className="nft-select-thumb" src={image} alt="" /> : <span className="nft-select-thumb empty-thumb">{nftEditionNumber(nft) ?? ''}</span>}
                      <span>
                        <strong>{nft.title}</strong>
                        <small>
                          {delivery ? statusLabel(delivery.status) : 'Немає ланцюжка'}
                          {nft.tokenId ? ' · mint' : ''}
                          {' · '}
                          {shortId(nft.id)}
                        </small>
                      </span>
                    </label>
                  );
                })}
              </div>
            </section>

            <section className="chain-form">
              <div className="field">
                <label>Статус</label>
                <select value={status} onChange={e => setStatus(e.target.value)}>
                  {DELIVERY_STATUSES.map(item => <option key={item.value} value={item.value}>{item.label}</option>)}
                </select>
              </div>
              <div className="field">
                <label>Куди прибув</label>
                <input value={location} onChange={e => setLocation(e.target.value)} placeholder="Склад / магазин / місто / відділення" />
              </div>
              <div className="field">
                <label>Час створення / події</label>
                <input type="datetime-local" value={eventTime} onChange={e => setEventTime(e.target.value)} />
              </div>
              <div className="field">
                <label>Нотатка</label>
                <textarea value={note} onChange={e => setNote(e.target.value)} />
              </div>

              <div className="toggle-row compact-toggle">
                <div>
                  <div className="lbl">Створювати ланцюжки для NFT без доставки</div>
                  <div className="sub">Інакше такі NFT буде пропущено</div>
                </div>
                <button
                  className={`switch ${createMissing ? 'on' : ''}`}
                  type="button"
                  onClick={() => setCreateMissing(v => !v)}
                  aria-label="Toggle create missing chains"
                >
                  <span className="knob" />
                </button>
              </div>

              {createMissing && missingCount > 0 && (
                <>
                  <UserLookup
                    label="Отримувач / відповідальний"
                    selected={recipient}
                    manualUid={recipientUid}
                    onSelect={setRecipient}
                    onManualUid={setRecipientUid}
                    onClear={() => setRecipient(null)}
                  />
                  <div className="field">
                    <label>Адреса / місце призначення</label>
                    <input value={address} onChange={e => setAddress(e.target.value)} placeholder="За замовчуванням: куди прибув" />
                  </div>
                </>
              )}

              {err && <div className="error-banner">{err}</div>}

              <div className="actions">
                <button className="btn" type="button" onClick={onClose} disabled={busy}>Скасувати</button>
                <button className="btn btn-primary" type="button" onClick={submit} disabled={busy}>
                  {busy ? 'Зберігаємо…' : 'Зберегти ланцюжок'}
                </button>
              </div>
            </section>
          </div>
        )}
      </div>
    </div>
  );
}

function UserLookup({
  label,
  selected,
  manualUid,
  onSelect,
  onManualUid,
  onClear,
}: {
  label: string;
  selected: PublicUser | null;
  manualUid: string;
  onSelect: (user: PublicUser) => void;
  onManualUid: (uid: string) => void;
  onClear: () => void;
}) {
  const [query, setQuery] = useState('');
  const [results, setResults] = useState<PublicUser[]>([]);
  const [searching, setSearching] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const q = query.trim();
    setError(null);
    if (q.length < 2) {
      setResults([]);
      setSearching(false);
      return;
    }

    let cancelled = false;
    const timer = window.setTimeout(async () => {
      setSearching(true);
      try {
        const users = await apiSearchUsers(q);
        if (!cancelled) setResults(users);
      } catch (e: any) {
        if (!cancelled) setError(e?.message ?? 'Search failed');
      } finally {
        if (!cancelled) setSearching(false);
      }
    }, 250);

    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [query]);

  const pick = (user: PublicUser) => {
    onSelect(user);
    setQuery('');
    setResults([]);
    onManualUid('');
  };

  return (
    <div className="crm-user-lookup">
      <div className="field">
        <label>{label}</label>
        <input
          value={query}
          onChange={e => setQuery(e.target.value)}
          placeholder="Ім’я або @username"
        />
      </div>

      {selected && (
        <div className="selected-user">
          <div>
            <strong>{selected.name || selected.username}</strong>
            <span>@{selected.username || shortId(selected.uid)} · {shortId(selected.uid)}</span>
          </div>
          <button type="button" onClick={onClear}><Icon.Close size={14} /></button>
        </div>
      )}

      {query.trim().length >= 2 && (
        <div className="lookup-results">
          {searching && <div className="lookup-row muted">Шукаємо…</div>}
          {!searching && error && <div className="lookup-row danger-text">{error}</div>}
          {!searching && !error && results.length === 0 && <div className="lookup-row muted">Нікого не знайдено.</div>}
          {!searching && !error && results.map(user => (
            <button className="lookup-row" type="button" key={user.uid} onClick={() => pick(user)}>
              <span>{user.name || user.username}</span>
              <small>@{user.username || shortId(user.uid)}</small>
            </button>
          ))}
        </div>
      )}

      <div className="field crm-manual-uid">
        <label>UID вручну</label>
        <input
          value={manualUid}
          onChange={e => {
            onManualUid(e.target.value);
            if (selected) onClear();
          }}
          placeholder="Firebase UID"
        />
      </div>
    </div>
  );
}

// ── NFC ──────────────────────────────────────────────────────────────────────

function NfcTab() {
  return (
    <div className="grid grid-2">
      <NfcBindForm />
      <NfcVerifyForm />
    </div>
  );
}

function NfcBindForm() {
  const [nfts, setNfts] = useState<NFT[]>([]);
  const [nftId, setNftId] = useState('');
  const [nfcUid, setNfcUid] = useState('');
  const [nftQuery, setNftQuery] = useState('');
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState<string | null>(null);
  const [err, setErr] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    apiGetNFTs()
      .then(items => { if (!cancelled) setNfts(items); })
      .catch(() => { if (!cancelled) setNfts([]); });
    return () => { cancelled = true; };
  }, []);

  const filteredNfts = useMemo(() => {
    const q = nftQuery.trim().toLowerCase();
    if (!q) return nfts;
    return nfts.filter(nft => [nft.id, nft.title, nft.tokenId, nft.nfcUid].map(searchValue).join(' ').includes(q));
  }, [nfts, nftQuery]);

  const submit = async () => {
    setBusy(true);
    setErr(null);
    setMsg(null);
    try {
      const res = await apiBindNfc({ nftId, nfcUid: nfcUid.trim() });
      setMsg(`Прив’язано: ${shortId(res.nftId)} ↔ ${res.nfcUid}`);
      setNfcUid('');
    } catch (e: any) {
      setErr(e?.message ?? 'Failed');
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="card crm-nfc-card">
      <h3>Прив’язка NFC</h3>
      <div className="field">
        <label>Пошук NFT</label>
        <input value={nftQuery} onChange={e => setNftQuery(e.target.value)} placeholder="Назва, mint, UID" />
      </div>
      <div className="field">
        <label>NFT</label>
        <select value={nftId} onChange={e => setNftId(e.target.value)}>
          <option value="">— вибрати —</option>
          {filteredNfts.map(nft => (
            <option key={nft.id} value={nft.id}>
              {nft.title} ({shortId(nft.id)})
            </option>
          ))}
        </select>
      </div>
      <div className="field">
        <label>NFC UID</label>
        <input value={nfcUid} onChange={e => setNfcUid(e.target.value)} placeholder="04:A2:B5:..." />
      </div>
      <button className="btn btn-primary btn-block" type="button" disabled={busy || !nftId || !nfcUid.trim()} onClick={submit}>
        {busy ? 'Прив’язка…' : 'Прив’язати'}
      </button>
      {msg && <div className="success-banner">{msg}</div>}
      {err && <div className="error-banner crm-inline-error">{err}</div>}
    </div>
  );
}

function NfcVerifyForm() {
  const [nfcUid, setNfcUid] = useState('');
  const [busy, setBusy] = useState(false);
  const [scanning, setScanning] = useState(false);
  const [result, setResult] = useState<any>(null);
  const [err, setErr] = useState<string | null>(null);
  const canScan = typeof window !== 'undefined' && 'NDEFReader' in window;

  const verifyUid = async (uid: string) => {
    setBusy(true);
    setErr(null);
    setResult(null);
    try {
      const res = await apiVerifyNfc(uid.trim());
      setResult(res);
    } catch (e: any) {
      setErr(e?.message ?? 'Failed');
    } finally {
      setBusy(false);
    }
  };

  const scan = async () => {
    const Reader = (window as any).NDEFReader;
    if (!Reader) return;
    setScanning(true);
    setErr(null);
    try {
      const reader = new Reader();
      await reader.scan();
      reader.onreading = async (event: any) => {
        const uid = event.serialNumber || '';
        setScanning(false);
        if (!uid) {
          setErr('Не вдалося зчитати UID мітки.');
          return;
        }
        setNfcUid(uid);
        await verifyUid(uid);
      };
      reader.onreadingerror = () => {
        setScanning(false);
        setErr('Помилка зчитування NFC.');
      };
    } catch (e: any) {
      setScanning(false);
      setErr(e?.message ?? 'NFC scan failed');
    }
  };

  return (
    <div className="card crm-nfc-card">
      <h3>Перевірка NFC</h3>
      <div className="field">
        <label>NFC UID</label>
        <input value={nfcUid} onChange={e => setNfcUid(e.target.value)} placeholder="UID мітки" />
      </div>
      <div className="crm-button-row">
        <button className="btn btn-primary" type="button" disabled={busy || !nfcUid.trim()} onClick={() => verifyUid(nfcUid)}>
          {busy ? 'Перевірка…' : 'Перевірити'}
        </button>
        <button className="btn" type="button" disabled={!canScan || busy || scanning} onClick={scan}>
          <Icon.QrCode /> {scanning ? 'Чекаємо мітку…' : 'Сканувати'}
        </button>
      </div>

      {result && (
        <div className="nfc-result">
          <strong>{result.nftTitle}</strong>
          <span>Владелец: {result.ownerName}</span>
          {result.tokenId && <small>Token: {result.tokenId}</small>}
          {result.autoConfirmedReceipt && <span className="badge badge-success">Доставку автопідтверджено</span>}
        </div>
      )}

      {err && <div className="error-banner crm-inline-error">{err}</div>}
    </div>
  );
}

function Fact({ label, value, wide }: { label: string; value: string; wide?: boolean }) {
  return (
    <div className={wide ? 'wide' : ''}>
      <span>{label}</span>
      <strong>{value}</strong>
    </div>
  );
}
