import { useCallback, useEffect, useState } from 'react';
import QRCode from 'qrcode';
import {
  apiAddGoodsEvent, apiClearGoodsFlag, apiCreateGoodsBatch, apiCreateGoodsProduct, apiGoodsBatch, apiGoodsProducts,
  type GoodsBatch, type GoodsBatchSummary, type GoodsCategory, type GoodsEventKind, type GoodsProduct,
} from '../api';
import EmptyState from '../EmptyState';
import { fmtDateTime } from '../format';
import { Icon } from '../icons';
import { ListSkeleton } from '../Skeleton';
import { CATEGORY, EVENT, STAGE } from './labels';

type ProductRow = GoodsProduct & { batches: GoodsBatchSummary[] };

/** Buyer app address: QR codes on labels open the check page there. */
const USER_APP = ((import.meta.env.VITE_USER_APP_URL as string) || 'http://localhost:3002/').replace(/\/?$/, '/');
export const goodsUrl = (code: string) => `${USER_APP}?v=${encodeURIComponent(code)}`;

/** Brand side of goods authenticity: products, batches, supply chain and QR labels. */
export default function GoodsPage() {
  const [rows, setRows] = useState<ProductRow[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [newProduct, setNewProduct] = useState(false);
  const [batchFor, setBatchFor] = useState<GoodsProduct | null>(null);
  const [openBatch, setOpenBatch] = useState<{ product: GoodsProduct; id: string } | null>(null);

  const reload = useCallback(async () => {
    setError(null);
    try {
      setRows(await apiGoodsProducts());
    } catch (e: any) {
      setError(e?.status === 404 ? 'Товари запрацюють після оновлення сервера.' : e?.message ?? 'Не вдалося завантажити товари');
      setRows([]);
    }
  }, []);

  useEffect(() => { reload(); }, [reload]);

  return (
    <div>
      <div className="page-head">
        <div>
          <h2>Товари</h2>
          <p>Паспорти справжності для вина, напоїв і продуктів: партії, QR-етикетки та шлях товару.</p>
        </div>
        <button className="btn btn-primary" onClick={() => setNewProduct(true)}><Icon.Plus /> Новий товар</button>
      </div>

      {error && <div className="notice notice-info">{error}</div>}

      {!rows ? <ListSkeleton /> : rows.length === 0 ? (
        <EmptyState
          icon="🍷"
          title="Ще немає товарів"
          text="Додай товар, створи партію — і роздрукуй QR-етикетки. Покупці скануватимуть їх і бачитимуть, що товар справжній."
          action={<button className="btn btn-primary" onClick={() => setNewProduct(true)}>Додати товар</button>}
        />
      ) : (
        <div className="order-list">
          {rows.map(p => (
            <div key={p.id} className="card order-card goods-product">
              <div className="gp-head">
                {p.image ? <img src={p.image} alt="" className="gift-thumb" /> : <div className="gp-icon">{CATEGORY[p.category]?.icon ?? '📦'}</div>}
                <div style={{ minWidth: 0, flex: 1 }}>
                  <div className="oc-title">{p.name}</div>
                  <div className="muted" style={{ fontSize: 12 }}>
                    {CATEGORY[p.category]?.label} · {p.mode === 'unit' ? 'QR на кожну одиницю' : 'один QR на партію'}
                  </div>
                </div>
                <button className="btn" onClick={() => setBatchFor(p)}><Icon.Plus /> Партія</button>
              </div>

              {p.batches.length === 0 ? (
                <div className="muted" style={{ fontSize: 13, marginTop: 10 }}>Партій ще немає.</div>
              ) : (
                <div className="gp-batches">
                  {p.batches.map(b => (
                    <button key={b.id} className="gp-batch" onClick={() => setOpenBatch({ product: p, id: b.id })}>
                      <strong>Партія {b.lot}</strong>
                      <span className="badge badge-info">{STAGE[b.stage]}</span>
                      <span className="muted">
                        {p.mode === 'unit' ? `${b.size} шт · куплено ${b.claimed}` : 'QR на партію'}
                        {b.expiresAt && ` · до ${b.expiresAt}`}
                      </span>
                    </button>
                  ))}
                </div>
              )}
            </div>
          ))}
        </div>
      )}

      {newProduct && <NewProductModal onClose={() => setNewProduct(false)} onCreated={p => { setNewProduct(false); reload(); setBatchFor(p); }} />}
      {batchFor && <NewBatchModal product={batchFor} onClose={() => setBatchFor(null)} onCreated={b => { setBatchFor(null); reload(); setOpenBatch({ product: batchFor, id: b.id }); }} />}
      {openBatch && <BatchModal product={openBatch.product} batchId={openBatch.id} onClose={() => { setOpenBatch(null); reload(); }} />}
    </div>
  );
}

/** Shrinks a photo to a JPEG data URL small enough to send in JSON. */
function photoDataUrl(file: File, maxPx = 900): Promise<string> {
  return new Promise((resolve, reject) => {
    const img = new Image();
    img.onload = () => {
      const scale = Math.min(1, maxPx / Math.max(img.width, img.height));
      const canvas = document.createElement('canvas');
      canvas.width = Math.round(img.width * scale);
      canvas.height = Math.round(img.height * scale);
      canvas.getContext('2d')!.drawImage(img, 0, 0, canvas.width, canvas.height);
      URL.revokeObjectURL(img.src);
      resolve(canvas.toDataURL('image/jpeg', 0.82));
    };
    img.onerror = reject;
    img.src = URL.createObjectURL(file);
  });
}

function NewProductModal({ onClose, onCreated }: { onClose: () => void; onCreated: (p: GoodsProduct) => void }) {
  const [category, setCategory] = useState<GoodsCategory>('wine');
  const [mode, setMode] = useState<'unit' | 'batch'>('unit');
  const [name, setName] = useState('');
  const [description, setDescription] = useState('');
  const [attrs, setAttrs] = useState<Record<string, string>>({});
  const [photo, setPhoto] = useState<string>('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const pickCategory = (c: GoodsCategory) => {
    setCategory(c);
    setMode(CATEGORY[c].mode);
  };

  const create = async () => {
    setBusy(true);
    setError(null);
    try {
      const p = await apiCreateGoodsProduct({ name, category, mode, description, imageData: photo || undefined, attributes: attrs });
      onCreated(p);
    } catch (e: any) {
      setError(e?.message ?? 'Не вдалося створити товар');
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="modal-backdrop" onClick={busy ? undefined : onClose}>
      <div className="modal product-modal" onClick={e => e.stopPropagation()}>
        <h3>Новий товар</h3>

        <div className="field">
          <label>Категорія</label>
          <div className="chip-row">
            {(Object.keys(CATEGORY) as GoodsCategory[]).map(c => (
              <button key={c} type="button" className={`chip ${category === c ? 'active' : ''}`} onClick={() => pickCategory(c)}>
                {CATEGORY[c].icon} {CATEGORY[c].label}
              </button>
            ))}
          </div>
        </div>

        <div className="field">
          <label>Назва *</label>
          <input value={name} maxLength={120} onChange={e => setName(e.target.value)} placeholder={category === 'wine' ? 'Каберне Совіньйон 2021' : 'Молоко 2,5%'} />
        </div>

        <div className="field">
          <label>Як маркуємо</label>
          <div className="segmented">
            <button type="button" className={mode === 'unit' ? 'active' : ''} onClick={() => setMode('unit')}>QR на кожну пляшку / одиницю</button>
            <button type="button" className={mode === 'batch' ? 'active' : ''} onClick={() => setMode('batch')}>Один QR на партію</button>
          </div>
          <div className="muted" style={{ fontSize: 11, marginTop: 4 }}>
            {mode === 'unit'
              ? 'Кожна одиниця отримує свій код: покупець реєструє її за чеком, а підробку з тим самим кодом видно одразу.'
              : 'Один код на всю партію: покупець бачить походження, дати й аналізи. Підходить для молока й продуктів.'}
          </div>
        </div>

        {CATEGORY[category].attrs.map(a => (
          <div className="field" key={a}>
            <label>{a}</label>
            <input value={attrs[a] ?? ''} maxLength={200} onChange={e => setAttrs({ ...attrs, [a]: e.target.value })} />
          </div>
        ))}

        <div className="field">
          <label>Опис</label>
          <textarea value={description} maxLength={2000} onChange={e => setDescription(e.target.value)} />
        </div>

        <div className="field">
          <label>Фото</label>
          <input type="file" accept="image/*" onChange={async e => {
            const f = e.target.files?.[0];
            if (f) setPhoto(await photoDataUrl(f).catch(() => ''));
          }} />
          {photo && <img src={photo} alt="" className="gift-thumb" style={{ marginTop: 8 }} />}
        </div>

        {error && <div className="error-banner">{error}</div>}
        <div className="actions">
          <button className="btn" onClick={onClose} disabled={busy}>Скасувати</button>
          <button className="btn btn-primary" onClick={create} disabled={busy || !name.trim()}>{busy ? 'Створюємо…' : 'Створити'}</button>
        </div>
      </div>
    </div>
  );
}

function NewBatchModal({ product, onClose, onCreated }: { product: GoodsProduct; onClose: () => void; onCreated: (b: GoodsBatch) => void }) {
  const today = new Date().toISOString().slice(0, 10);
  const [lot, setLot] = useState('');
  const [producedAt, setProducedAt] = useState(today);
  const [expiresAt, setExpiresAt] = useState('');
  const [units, setUnits] = useState(product.mode === 'unit' ? 12 : 0);
  const [location, setLocation] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const create = async () => {
    setBusy(true);
    setError(null);
    try {
      onCreated(await apiCreateGoodsBatch(product.id, { lot, producedAt, expiresAt: expiresAt || undefined, units, location }));
    } catch (e: any) {
      setError(e?.message ?? 'Не вдалося створити партію');
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="modal-backdrop" onClick={busy ? undefined : onClose}>
      <div className="modal product-modal" onClick={e => e.stopPropagation()}>
        <h3>Нова партія · {product.name}</h3>
        <div className="field">
          <label>Номер партії (лот) *</label>
          <input value={lot} maxLength={60} onChange={e => setLot(e.target.value)} placeholder="L-2026-014" />
        </div>
        <div className="grid grid-2">
          <div className="field">
            <label>{product.category === 'wine' || product.category === 'spirits' ? 'Дата розливу' : 'Дата виробництва'}</label>
            <input type="date" value={producedAt} onChange={e => setProducedAt(e.target.value)} />
          </div>
          <div className="field">
            <label>Придатний до <span className="muted">(необов’язково)</span></label>
            <input type="date" value={expiresAt} onChange={e => setExpiresAt(e.target.value)} />
          </div>
        </div>
        {product.mode === 'unit' && (
          <div className="field">
            <label>Скільки одиниць (QR-кодів)</label>
            <input type="number" min={1} max={2000} value={units} onChange={e => setUnits(Math.max(1, Math.min(2000, Number(e.target.value) || 1)))} />
          </div>
        )}
        <div className="field">
          <label>Де вироблено <span className="muted">(необов’язково)</span></label>
          <input value={location} maxLength={120} onChange={e => setLocation(e.target.value)} placeholder="Одеса, винзавод" />
        </div>
        {error && <div className="error-banner">{error}</div>}
        <div className="actions">
          <button className="btn" onClick={onClose} disabled={busy}>Скасувати</button>
          <button className="btn btn-primary" onClick={create} disabled={busy || !lot.trim()}>{busy ? 'Створюємо…' : 'Створити партію'}</button>
        </div>
      </div>
    </div>
  );
}

function BatchModal({ product, batchId, onClose }: { product: GoodsProduct; batchId: string; onClose: () => void }) {
  const [batch, setBatch] = useState<GoodsBatch | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [kind, setKind] = useState<GoodsEventKind>('warehouse');
  const [location, setLocation] = useState('');
  const [note, setNote] = useState('');
  const [busy, setBusy] = useState(false);
  const [printing, setPrinting] = useState(false);

  useEffect(() => {
    apiGoodsBatch(batchId).then(setBatch).catch(e => setError(e?.message ?? 'Не вдалося завантажити партію'));
  }, [batchId]);

  const addEvent = async () => {
    setBusy(true);
    setError(null);
    try {
      setBatch(await apiAddGoodsEvent(batchId, { kind, location, note }));
      setLocation('');
      setNote('');
    } catch (e: any) {
      setError(e?.message ?? 'Не вдалося додати подію');
    } finally {
      setBusy(false);
    }
  };

  const print = async () => {
    if (!batch) return;
    setPrinting(true);
    try {
      await printLabels(product, batch);
    } finally {
      setPrinting(false);
    }
  };

  const claimed = batch?.units.filter(u => u.ownerId).length ?? 0;
  const flagged = batch?.units.filter(u => u.flag) ?? [];

  const clearFlag = async (serial: number) => {
    if (!confirm(`Зняти позначку підробки з № ${serial}? Робіть це, лише якщо впевнені, що це не копія етикетки.`)) return;
    try { setBatch(await apiClearGoodsFlag(batchId, serial)); } catch (e: any) { alert(e?.message ?? 'Не вдалося'); }
  };

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div className="modal product-modal" onClick={e => e.stopPropagation()}>
        <h3 style={{ marginBottom: 2 }}>{product.name}</h3>
        <div className="muted" style={{ fontSize: 13, marginBottom: 12 }}>Партія {batch?.lot ?? '…'}</div>
        {error && <div className="error-banner">{error}</div>}
        {!batch ? <ListSkeleton count={2} /> : (
          <>
            <div className="goods-stats">
              <div><strong>{product.mode === 'unit' ? batch.units.length : 1}</strong><span>QR-кодів</span></div>
              {product.mode === 'unit' && <div><strong>{claimed}</strong><span>куплено й зареєстровано</span></div>}
              <div><strong>{batch.expiresAt || '—'}</strong><span>придатний до</span></div>
              {product.mode === 'unit' && <div className={flagged.length ? 'danger' : ''}><strong>{flagged.length}</strong><span>підозра на підробку</span></div>}
            </div>

            {flagged.length > 0 && (
              <div className="notice notice-warn" style={{ marginTop: 12 }}>
                <strong>Коди, які сканували в різних країнах за добу:</strong>
                {flagged.map(u => (
                  <div key={u.serial} className="goods-flag">
                    <span>№ {u.serial} · {u.flag!.regions.join(', ')} · {fmtDateTime(u.flag!.at)}</span>
                    <button className="btn" onClick={() => clearFlag(u.serial)}>Зняти позначку</button>
                  </div>
                ))}
              </div>
            )}

            <button className="btn btn-primary btn-block" onClick={print} disabled={printing} style={{ marginTop: 12 }}>
              <Icon.QrCode /> {printing ? 'Готуємо етикетки…' : product.mode === 'unit' ? `Друкувати ${batch.units.length} QR-етикеток` : 'Друкувати QR партії'}
            </button>

            <h4 style={{ margin: '18px 0 8px' }}>Шлях партії</h4>
            <div className="passport-timeline">
              {[...batch.events].reverse().map((ev, i) => (
                <div key={i} className="pt-item">
                  <span className="pt-icon">{EVENT[ev.kind]?.icon ?? '•'}</span>
                  <div>
                    <div style={{ fontWeight: 600, fontSize: 14 }}>{EVENT[ev.kind]?.label ?? ev.kind}{ev.location && ` · ${ev.location}`}</div>
                    {ev.note && <div style={{ fontSize: 13 }}>{ev.note}</div>}
                    <div className="muted" style={{ fontSize: 11 }}>{fmtDateTime(ev.at)} · {ev.actorName}</div>
                  </div>
                </div>
              ))}
            </div>

            <div className="card" style={{ marginTop: 12, background: 'var(--bg-soft)' }}>
              <div className="field">
                <label>Нова подія</label>
                <select value={kind} onChange={e => setKind(e.target.value as GoodsEventKind)}>
                  {(Object.keys(EVENT) as GoodsEventKind[]).map(k => <option key={k} value={k}>{EVENT[k].icon} {EVENT[k].label}</option>)}
                </select>
              </div>
              <div className="field">
                <label>Де</label>
                <input value={location} maxLength={120} onChange={e => setLocation(e.target.value)} placeholder="Київ, склад «Вина Півдня»" />
              </div>
              <div className="field">
                <label>Примітка</label>
                <input value={note} maxLength={500} onChange={e => setNote(e.target.value)} placeholder={kind === 'lab' ? 'Аналіз №123: відповідає ДСТУ' : ''} />
              </div>
              <button className="btn btn-primary" onClick={addEvent} disabled={busy}>{busy ? 'Додаємо…' : 'Додати подію'}</button>
            </div>
          </>
        )}
        <div className="actions"><button className="btn" onClick={onClose}>Закрити</button></div>
      </div>
    </div>
  );
}

/** Opens a print-ready sheet of QR labels (A4, 4 per row) in a new window. */
async function printLabels(product: GoodsProduct, batch: GoodsBatch) {
  const items = product.mode === 'unit'
    ? batch.units.map(u => ({ code: u.code, caption: `№ ${u.serial}` }))
    : [{ code: batch.id, caption: 'QR партії' }];
  const qrs = await Promise.all(items.map(i => QRCode.toDataURL(goodsUrl(i.code), { width: 240, margin: 1, errorCorrectionLevel: 'M' })));
  const esc = (s: string) => s.replace(/[&<>"]/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]!));
  const labels = items.map((it, i) => `
    <div class="l"><img src="${qrs[i]}"><b>${esc(product.name)}</b><span>Партія ${esc(batch.lot)} · ${esc(it.caption)}</span><small>Перевір у Marki · ${esc(it.code)}</small></div>`).join('');
  const w = window.open('', '_blank');
  if (!w) { alert('Дозволь спливні вікна, щоб надрукувати етикетки.'); return; }
  w.document.write(`<!doctype html><html><head><meta charset="utf-8"><title>QR · ${esc(product.name)} · ${esc(batch.lot)}</title>
    <style>
      @page { size: A4; margin: 10mm; }
      body { font-family: -apple-system, system-ui, sans-serif; margin: 0; }
      .g { display: grid; grid-template-columns: repeat(4, 1fr); gap: 6mm; }
      .l { border: 1px dashed #bbb; border-radius: 3mm; padding: 3mm; text-align: center; break-inside: avoid; display: flex; flex-direction: column; gap: 1mm; }
      .l img { width: 100%; max-width: 38mm; margin: 0 auto; }
      .l b { font-size: 9pt; } .l span { font-size: 8pt; } .l small { font-size: 6.5pt; color: #666; word-break: break-all; }
    </style></head><body><div class="g">${labels}</div><script>window.onload = () => window.print();</script></body></html>`);
  w.document.close();
}
