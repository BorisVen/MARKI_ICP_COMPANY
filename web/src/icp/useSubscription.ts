import { useCallback, useEffect, useState } from 'react';
import { Principal } from '@dfinity/principal';
import { paymentsIdl, type PaymentsService, type Plan, type Subscription } from './idl';
import { lastPrincipal, useWallet } from './useWallet';
import { anonymousActor, PAYMENTS_CANISTER_ID } from './wallet';

export type PlanState = {
  enabled: boolean;
  principal: string | null;
  loading: boolean;
  subscription: Subscription | null;
  plan: Plan | null;
  active: boolean;
  daysLeft: number;
  refresh: () => Promise<void>;
};

/** Plan of the connected wallet (or the last one used on this device). */
export function useSubscription(): PlanState {
  const wallet = useWallet();
  const principal = wallet?.principal ?? lastPrincipal();
  const [loading, setLoading] = useState(false);
  const [subscription, setSubscription] = useState<Subscription | null>(null);
  const [plan, setPlan] = useState<Plan | null>(null);

  const refresh = useCallback(async () => {
    if (!PAYMENTS_CANISTER_ID || !principal) return;
    setLoading(true);
    try {
      const payments = await anonymousActor<PaymentsService>(PAYMENTS_CANISTER_ID, paymentsIdl);
      const [sub, plans] = await Promise.all([
        payments.subscription_of(Principal.fromText(principal)),
        payments.plans().catch(() => ({ Err: '' }) as const),
      ]);
      const s = sub[0] ?? null;
      setSubscription(s);
      setPlan(s && 'Ok' in plans ? plans.Ok.find(q => q.plan.id === s.plan_id)?.plan ?? null : null);
    } catch {
      setSubscription(null);
    } finally {
      setLoading(false);
    }
  }, [principal]);

  useEffect(() => { refresh(); }, [refresh]);

  const nowNs = BigInt(Date.now()) * 1_000_000n;
  const active = !!subscription && subscription.expires_at_ns > nowNs;
  const daysLeft = subscription && active
    ? Math.ceil(Number((subscription.expires_at_ns - nowNs) / 1_000_000n) / 86_400_000)
    : 0;

  return { enabled: !!PAYMENTS_CANISTER_ID, principal, loading, subscription, plan, active, daysLeft, refresh };
}
