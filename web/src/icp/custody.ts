import { AuthClient } from '@dfinity/auth-client';
import { Principal } from '@dfinity/principal';
import { paymentsIdl, type Account, type PaymentsService } from './idl';
import { anonymousActor, PAYMENTS_CANISTER_ID } from './wallet';

/**
 * Marki custody: tokens of people without their own wallet live under one
 * custody principal, one sub-account per Marki user — sha256("marki-custody:" + uid),
 * the same derivation as the server. Gifts then move on-chain automatically.
 */
let custodian: Promise<Principal | null> | null = null;

/** The custody principal from the canister (null when custody is off). */
export function custodianPrincipal(): Promise<Principal | null> {
  custodian ??= (async () => {
    if (!PAYMENTS_CANISTER_ID) return null;
    const payments = await anonymousActor<PaymentsService>(PAYMENTS_CANISTER_ID, paymentsIdl);
    return (await payments.config()).custodian[0] ?? null;
  })().catch(() => null);
  return custodian;
}

export async function custodySubaccount(uid: string): Promise<Uint8Array> {
  return new Uint8Array(await crypto.subtle.digest('SHA-256', new TextEncoder().encode(`marki-custody:${uid}`)));
}

/** Where `uid`'s tokens live in Marki custody, or null when custody is off. */
export async function custodyAccount(uid: string): Promise<Account | null> {
  const owner = await custodianPrincipal();
  return owner ? { owner, subaccount: [await custodySubaccount(uid)] } : null;
}

export const sameAccount = (a: Account | undefined, b: Account | null) =>
  !!a && !!b && a.owner.toText() === b.owner.toText() &&
  (a.subaccount[0] ?? new Uint8Array(32)).every((x, i) => x === ((b.subaccount[0] ?? new Uint8Array(32))[i] ?? 0));

/** Sign in with Internet Identity (Face ID / fingerprint) and return the principal. */
export async function internetIdentityPrincipal(): Promise<string> {
  const client = await AuthClient.create();
  if (await client.isAuthenticated()) return client.getIdentity().getPrincipal().toText();
  await new Promise<void>((resolve, reject) => client.login({
    identityProvider: 'https://identity.ic0.app',
    maxTimeToLive: BigInt(7 * 24 * 3600) * 1_000_000_000n,
    onSuccess: () => resolve(),
    onError: e => reject(new Error(e ?? 'Вхід через Internet Identity скасовано')),
  }));
  return client.getIdentity().getPrincipal().toText();
}
