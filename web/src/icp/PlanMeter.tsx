import type { PlanState } from './useSubscription';

const fmt = (n: bigint | number) => Number(n).toLocaleString('uk-UA');

function Meter({ label, left, total }: { label: string; left: bigint; total: bigint }) {
  const max = total > left ? total : left;
  const pct = max > 0n ? Number((left * 100n) / max) : 0;
  const tone = pct <= 10 ? 'danger' : pct <= 25 ? 'warn' : 'ok';
  return (
    <div className="meter">
      <div className="meter-head">
        <span>{label}</span>
        <span><strong>{fmt(left)}</strong> <span className="muted">з {fmt(max)}</span></span>
      </div>
      <div className="meter-track"><div className={`meter-fill ${tone}`} style={{ width: `${pct}%` }} /></div>
    </div>
  );
}

/** Plan name, remaining mints as bars, and days left. */
export default function PlanMeter({ state }: { state: PlanState }) {
  const { subscription: sub, plan, active, daysLeft } = state;
  if (!sub || !active) return null;
  return (
    <div className="plan-meter">
      <div className="plan-meter-head">
        <div>
          <div className="muted" style={{ fontSize: 12 }}>Ваш тариф</div>
          <div style={{ fontSize: 18, fontWeight: 600 }}>{plan?.name ?? sub.plan_id}</div>
        </div>
        <span className={`badge ${daysLeft <= 5 ? 'badge-pending' : 'badge-success'}`}>
          {daysLeft <= 1 ? 'останній день' : `ще ${daysLeft} дн.`}
        </span>
      </div>
      <Meter label="NFT з фото" left={sub.photo_mints_left} total={plan?.photo_mints ?? sub.photo_mints_left} />
      <Meter label="NFT з метаданими" left={sub.metadata_mints_left} total={plan?.metadata_mints ?? sub.metadata_mints_left} />
    </div>
  );
}
