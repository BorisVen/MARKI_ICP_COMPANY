import { useCallback, useEffect, useState } from 'react';
import { Principal } from '@dfinity/principal';
import { Icon } from '../icons';
import {
  ledgerIdl,
  paymentsIdl,
  type LedgerService,
  type PaymentsService,
  type PlanPurchase,
  type PlanQuote,
  type Pricing,
} from '../icp/idl';
import { friendlyIcpError } from '../icp/errors';
import PlanMeter from '../icp/PlanMeter';
import { useSubscription } from '../icp/useSubscription';
import { disconnectWallet, useWallet } from '../icp/useWallet';
import WalletConnect from '../icp/WalletConnect';
import {
  anonymousActor,
  IS_LOCAL_NETWORK,
  LEDGER_CANISTER_ID,
  PAYMENTS_CANISTER_ID,
  type Wallet,
} from '../icp/wallet';

/** Extra allowance over the quote, so a small ICP/XDR move does not fail the payment. */
const PRICE_BUFFER_BPS = 200n;
/** Plan shown as the recommended one. */
const FEATURED_PLAN = 'business';

function fmtIcp(e8s: bigint, maxDecimals = 8): string {
  const whole = e8s / 100_000_000n;
  const frac = (e8s % 100_000_000n).toString().padStart(8, '0').slice(0, maxDecimals).replace(/0+$/, '');
  return frac ? `${whole}.${frac}` : whole.toString();
}

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

export default function PricingPage() {
  const wallet = useWallet();
  const plan = useSubscription();
  const [plans, setPlans] = useState<PlanQuote[]>([]);
  const [pricing, setPricing] = useState<Pricing | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [balance, setBalance] = useState<bigint | null>(null);
  const [purchases, setPurchases] = useState<PlanPurchase[]>([]);
  const [busy, setBusy] = useState<string | null>(null);
  const [success, setSuccess] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);

  const loadPlans = useCallback(async () => {
    const payments = await anonymousActor<PaymentsService>(PAYMENTS_CANISTER_ID, paymentsIdl);
    const rate = await payments.refresh_rate();
    if ('Err' in rate) throw new Error(rate.Err);
    const [p, pr] = await Promise.all([payments.plans(), payments.pricing()]);
    if ('Err' in p) throw new Error(p.Err);
    setPlans(p.Ok);
    setPricing(pr);
  }, []);

  const loadAccount = useCallback(async (w: Wallet) => {
    const owner = Principal.fromText(w.principal);
    const ledger = await anonymousActor<LedgerService>(LEDGER_CANISTER_ID, ledgerIdl);
    const payments = await anonymousActor<PaymentsService>(PAYMENTS_CANISTER_ID, paymentsIdl);
    const [b, hist] = await Promise.all([
      ledger.icrc1_balance_of({ owner, subaccount: [] }),
      payments.purchases_of(owner),
    ]);
    setBalance(b);
    setPurchases([...hist].reverse());
  }, []);

  useEffect(() => {
    if (!PAYMENTS_CANISTER_ID) return;
    loadPlans().catch(e => setError(`Не вдалося завантажити тарифи. ${errText(e)}`));
  }, [loadPlans]);

  useEffect(() => {
    if (!wallet) { setBalance(null); setPurchases([]); return; }
    loadAccount(wallet).catch(e => setError(errText(e)));
  }, [wallet, loadAccount]);

  if (!PAYMENTS_CANISTER_ID) {
    return (
      <div className="notice notice-warn">
        Оплата тарифів ще не налаштована (<code>VITE_ICP_PAYMENTS_CANISTER_ID</code>). Для локального тесту запусти <code>npm run dev:icp</code>.
      </div>
    );
  }

  const refreshAll = async () => {
    if (!wallet) return;
    await Promise.all([loadAccount(wallet), plan.refresh()]).catch(e => setError(errText(e)));
  };

  const buy = async (q: PlanQuote) => {
    if (!wallet || !pricing) return;
    setError(null);
    setSuccess(null);
    try {
      const fee = pricing.ledger_fee_e8s;
      const maxPrice = q.price_e8s + (q.price_e8s * PRICE_BUFFER_BPS) / 10_000n;

      setBusy(`Крок 1 з 2 · Підтвердь оплату «${q.plan.name}» у гаманці`);
      const ledger = await wallet.actor<LedgerService>(LEDGER_CANISTER_ID, ledgerIdl);
      const approve = await ledger.icrc2_approve({
        from_subaccount: [],
        spender: { owner: Principal.fromText(PAYMENTS_CANISTER_ID), subaccount: [] },
        amount: maxPrice + fee,
        expected_allowance: [],
        expires_at: [BigInt(Date.now() + 15 * 60_000) * 1_000_000n],
        fee: [fee],
        memo: [],
        created_at_time: [],
      });
      if ('Err' in approve) throw new Error(errText(approve.Err));

      setBusy('Крок 2 з 2 · Проводимо оплату…');
      const payments = await wallet.actor<PaymentsService>(PAYMENTS_CANISTER_ID, paymentsIdl);
      const res = await payments.buy_plan(q.plan.id, maxPrice);
      if ('Err' in res) throw new Error(res.Err);
      setSuccess(`Готово! Тариф «${q.plan.name}» активний до ${fmtDate(res.Ok.expires_at_ns)}.`);
      await refreshAll();
    } catch (e) {
      setError(errText(e));
    } finally {
      setBusy(null);
    }
  };

  const fee = pricing?.ledger_fee_e8s ?? 10_000n;
  const planName = (id: string) => plans.find(p => p.plan.id === id)?.plan.name ?? id;
  const fundCmd = wallet ? `./scripts/fund.sh ${wallet.principal}` : '';

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

      {/* Current plan first: it is what a returning customer looks for. */}
      {plan.active && (
        <div className="card" style={{ marginBottom: 16 }}>
          <PlanMeter state={plan} />
          <div className="muted" style={{ fontSize: 12, marginTop: 10 }}>
            Діє до {fmtDate(plan.subscription!.expires_at_ns)} · повторна покупка додає мінти й продовжує строк
          </div>
        </div>
      )}

      <div className="card" style={{ marginBottom: 20 }}>
        {!wallet ? (
          <div style={{ display: 'flex', gap: 12, flexWrap: 'wrap', alignItems: 'center', justifyContent: 'space-between' }}>
            <div>
              <div style={{ fontWeight: 600 }}>Гаманець для оплати</div>
              <div className="muted" style={{ fontSize: 12 }}>Оплата в ICP. Підключи гаманець, щоб обрати тариф.</div>
            </div>
            <WalletConnect onError={setError} />
          </div>
        ) : (
          <>
            <div style={{ display: 'flex', gap: 12, flexWrap: 'wrap', alignItems: 'center', justifyContent: 'space-between' }}>
              <div style={{ display: 'flex', gap: 12, alignItems: 'center', flexWrap: 'wrap' }}>
                <WalletConnect />
                <span>Баланс: <strong>{balance === null ? '…' : fmtIcp(balance, 4)} ICP</strong></span>
              </div>
              <div style={{ display: 'flex', gap: 8 }}>
                <button className="btn" onClick={refreshAll} disabled={!!busy}><Icon.Refresh /> Оновити</button>
                <button className="btn" onClick={disconnectWallet} disabled={!!busy}>Відключити</button>
              </div>
            </div>
            {IS_LOCAL_NETWORK && balance !== null && balance < 5n * 100_000_000n && (
              <div className="notice notice-info" style={{ marginTop: 12, marginBottom: 0 }}>
                <div style={{ marginBottom: 8 }}>
                  Тестова мережа: поповни гаманець безкоштовними ICP. Скопіюй команду, виконай її в папці <code>icp-local</code> і натисни «Оновити».
                </div>
                <div className="copy-row">
                  <code>{fundCmd}</code>
                  <button
                    className="btn"
                    onClick={() => navigator.clipboard.writeText(fundCmd).then(() => { setCopied(true); setTimeout(() => setCopied(false), 1500); })}
                  >
                    {copied ? 'Скопійовано' : 'Копіювати'}
                  </button>
                </div>
              </div>
            )}
          </>
        )}
      </div>

      {plans.length === 0 && !error && <div className="spinner">Завантаження тарифів…</div>}

      <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fit, minmax(240px, 1fr))', gap: 14, paddingTop: 10 }}>
        {plans.map(q => {
          const p = q.plan;
          const featured = p.id === FEATURED_PLAN;
          const current = plan.active && plan.subscription?.plan_id === p.id;
          const need = q.price_e8s + 2n * fee;
          const notEnough = balance !== null && balance < need;
          const label = !wallet ? 'Підключи гаманець'
            : notEnough ? 'Недостатньо ICP'
            : current ? `Продовжити «${p.name}»` : `Обрати «${p.name}»`;
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
                <div className="muted" style={{ fontSize: 12 }}>≈ {fmtIcp(q.price_e8s, 2)} ICP · {perNft(q)} за NFT з фото</div>
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
                onClick={() => buy(q)}
                disabled={!wallet || !!busy || notEnough}
              >
                {label}
              </button>
            </div>
          );
        })}
      </div>

      {pricing && plans[0] && (
        <details className="soft">
          <summary>Як рахується ціна в ICP?</summary>
          <div className="muted" style={{ fontSize: 12, lineHeight: 1.6 }}>
            Ціни встановлені в доларах. У момент оплати вони переводяться в ICP за курсом Internet Computer:
            1 ICP = {(Number(plans[0].xdr_permyriad_per_icp) / 10_000).toFixed(2)} XDR, 1 XDR ≈ ${pricing.xdr_usd}.
            Мережа додає комісію 2 × {fmtIcp(fee)} ICP (дозвіл і списання). Якщо курс зміниться більше ніж на 2 %, оплата не пройде — просто повтори її.
          </div>
        </details>
      )}

      {purchases.length > 0 && (
        <details className="soft">
          <summary>Історія оплат ({purchases.length})</summary>
          <div className="table-scroll">
            <table className="table">
              <thead><tr><th>Дата</th><th>Тариф</th><th>Сума</th></tr></thead>
              <tbody>
                {purchases.map(p => (
                  <tr key={p.id.toString()}>
                    <td>{fmtDate(p.timestamp_ns)}</td>
                    <td>{planName(p.plan_id)}</td>
                    <td>{fmtIcp(p.price_e8s, 4)} ICP</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </details>
      )}
    </div>
  );
}
