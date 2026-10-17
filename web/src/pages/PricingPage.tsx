import { useCallback, useEffect, useState } from 'react';
import { apiBillingPrice, fmtBillingPrice, type BillingPrice } from '../api';
import { paymentsIdl, type PaymentsService, type PlanQuote } from '../icp/idl';
import { friendlyIcpError } from '../icp/errors';
import { CardOrders, payByCard } from '../icp/CardPay';
import PlanMeter from '../icp/PlanMeter';
import { useSubscription } from '../icp/useSubscription';
import { useWallet } from '../icp/useWallet';
import WalletConnect from '../icp/WalletConnect';
import { anonymousActor, PAYMENTS_CANISTER_ID } from '../icp/wallet';

/** Plan shown as the recommended one. */
const FEATURED_PLAN = 'business';

const fmtNum = (n: bigint | number) => Number(n).toLocaleString('uk-UA');
const fmtUsd = (cents: bigint) => `$${(Number(cents) / 100).toLocaleString('en-US')}`;
const fmtSize = (kb: bigint) => (kb >= 1024n ? `${Number(kb) / 1024} MB` : `${kb} KB`);
const fmtDate = (ns: bigint) => new Date(Number(ns / 1_000_000n)).toLocaleDateString('uk-UA', { day: 'numeric', month: 'long', year: 'numeric' });
const perNft = (q: PlanQuote) => {
  const usd = Number(q.plan.price_usd_cents) / 100 / Number(q.plan.photo_mints);
  return `$${usd.toLocaleString('en-US', { maximumSignificantDigits: 2 })}`;
};

function errText(e: unknown): string {
  const raw = e instanceof Error ? e.message : JSON.stringify(e, (_, v) => (typeof v === 'bigint' ? v.toString() : v));
  return friendlyIcpError(raw);
}

/** Plans, paid by card; the plan goes to the connected wallet (it signs the mints). */
export default function PricingPage() {
  const wallet = useWallet();
  const plan = useSubscription();
  const [plans, setPlans] = useState<PlanQuote[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [success, setSuccess] = useState<string | null>(null);
  const [price, setPrice] = useState<BillingPrice | null>(null);

  useEffect(() => { apiBillingPrice().then(setPrice).catch(() => {}); }, []);

  const loadPlans = useCallback(async () => {
    const payments = await anonymousActor<PaymentsService>(PAYMENTS_CANISTER_ID, paymentsIdl);
    // Plans are quoted from the ICP rate inside the canister; it needs a fresh one.
    await payments.refresh_rate().catch(() => {});
    const p = await payments.plans();
    if ('Err' in p) throw new Error(p.Err);
    setPlans(p.Ok);
  }, []);

  useEffect(() => {
    if (!PAYMENTS_CANISTER_ID) return;
    loadPlans().catch(e => setError(`Не вдалося завантажити тарифи. ${errText(e)}`));
  }, [loadPlans]);

  if (!PAYMENTS_CANISTER_ID) {
    return <div className="notice notice-warn">Оплата тарифів ще не налаштована.</div>;
  }

  const card = async (q: PlanQuote) => {
    if (!wallet) return;
    setError(null);
    setSuccess(null);
    setBusy(`Відкриваємо оплату «${q.plan.name}»…`);
    // On the web the page leaves for WayForPay; in the app it opens in a tab over this page.
    try { await payByCard(q.plan.id, wallet.principal); setBusy(null); } catch (e) { setError(errText(e)); setBusy(null); }
  };

  return (
    <div>
      <div className="page-head">
        <div>
          <h2>Тарифи</h2>
          <p>Оберіть пакет — і випускайте NFT без турбот про газ і зберігання.</p>
        </div>
      </div>

      {error && <div className="error-banner">{error}</div>}
      {success && <div className="success-banner" style={{ marginBottom: 12 }}>{success}</div>}
      {busy && <div className="progress-banner" style={{ marginBottom: 12 }}><div className="mini-spin" /><div style={{ fontSize: 13 }}>{busy}</div></div>}

      <CardOrders onPaid={() => { setSuccess('Оплата пройшла — тариф активовано.'); plan.refresh(); }} />

      {/* Current plan first: it is what a returning customer looks for. */}
      {plan.active && (
        <div className="card" style={{ marginBottom: 16 }}>
          <PlanMeter state={plan} />
          <div className="muted" style={{ fontSize: 12, marginTop: 10 }}>
            Діє до {fmtDate(plan.subscription!.expires_at_ns)}
          </div>
        </div>
      )}

      <div className="card" style={{ marginBottom: 20 }}>
        <div style={{ display: 'flex', gap: 12, flexWrap: 'wrap', alignItems: 'center', justifyContent: 'space-between' }}>
          <div>
            <div style={{ fontWeight: 600 }}>Тариф для гаманця</div>
            <div className="muted" style={{ fontSize: 12 }}>На ньому мінтяться ваші NFT.</div>
          </div>
          <WalletConnect onError={setError} />
        </div>
      </div>

      {plans.length === 0 && !error && <div className="spinner">Завантаження тарифів…</div>}

      <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fit, minmax(240px, 1fr))', gap: 14, paddingTop: 10 }}>
        {plans.map(q => {
          const p = q.plan;
          const featured = p.id === FEATURED_PLAN;
          const current = plan.active && plan.subscription?.plan_id === p.id;
          return (
            <div key={p.id} className={`card plan-card ${featured ? 'featured' : ''}`}>
              {featured && <span className="plan-ribbon">Найпопулярніший</span>}
              <div>
                <div style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
                  <h3 style={{ margin: 0 }}>{p.name}</h3>
                  {current && <span className="badge badge-success">ваш</span>}
                </div>
                <div className="plan-price">
                  {fmtUsd(p.price_usd_cents)}
                  <span className="muted" style={{ fontSize: 14, fontWeight: 400 }}> / міс</span>
                </div>
                <div className="muted" style={{ fontSize: 12 }}>{perNft(q)} за NFT з фото</div>
              </div>
              <ul className="plan-features">
                <li><span><strong>{fmtNum(p.photo_mints)}</strong> NFT з фото (до {fmtSize(p.max_photo_kb)})</span></li>
                <li><span><strong>{fmtNum(p.metadata_mints)}</strong> NFT без фото</span></li>
                <li><span>Зберігання в блокчейні {p.storage_years} рік</span></li>
                <li><span>Газ за мінт платить Marki</span></li>
              </ul>
              <button
                className={`btn btn-block ${featured || current ? 'btn-primary' : ''}`}
                style={{ marginTop: 'auto' }}
                onClick={() => card(q)}
                disabled={!wallet || !!busy}
              >
                {!wallet ? 'Спершу гаманець' : `💳 ${current ? 'Продовжити' : 'Оплатити'}${price ? ` · ${fmtBillingPrice(price)}${price.testMode ? ' (тест)' : ''}` : ''}`}
              </button>
            </div>
          );
        })}
      </div>
    </div>
  );
}
