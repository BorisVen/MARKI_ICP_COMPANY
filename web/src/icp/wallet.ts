import { Actor, HttpAgent, type ActorSubclass } from '@dfinity/agent';
import type { IDL } from '@dfinity/candid';
import { Ed25519KeyIdentity } from '@dfinity/identity';

export const ICP_HOST = (import.meta.env.VITE_ICP_HOST as string) || 'https://icp0.io';
export const IS_LOCAL_NETWORK = /localhost|127\.0\.0\.1/.test(ICP_HOST);
export const PAYMENTS_CANISTER_ID = (import.meta.env.VITE_ICP_PAYMENTS_CANISTER_ID as string) || '';
export const LEDGER_CANISTER_ID =
  (import.meta.env.VITE_ICP_LEDGER_CANISTER_ID as string) || 'ryjl3-tyaaa-aaaaa-aaaba-cai';

export type WalletKind = 'plug' | 'dev';

export type Wallet = {
  kind: WalletKind;
  principal: string;
  actor: <T>(canisterId: string, idl: IDL.InterfaceFactory) => Promise<ActorSubclass<T>>;
};

let anonymousAgent: Promise<HttpAgent> | null = null;

/** Actor without a wallet: for queries and public updates. */
export async function anonymousActor<T>(canisterId: string, idl: IDL.InterfaceFactory) {
  anonymousAgent ??= HttpAgent.create({ host: ICP_HOST, shouldFetchRootKey: IS_LOCAL_NETWORK });
  return Actor.createActor<T>(idl, { agent: await anonymousAgent, canisterId });
}

export async function connectPlug(): Promise<Wallet> {
  const plug = (window as any).ic?.plug;
  if (!plug) {
    throw new Error('Plug Wallet не знайдено. Встанови розширення Plug для браузера та перезавантаж сторінку.');
  }
  const whitelist = [PAYMENTS_CANISTER_ID, LEDGER_CANISTER_ID].filter(Boolean);
  // requestConnect asks again only when the whitelist has canisters Plug has not approved yet.
  await plug.requestConnect({ whitelist, host: ICP_HOST });
  if (!plug.agent) await plug.createAgent({ whitelist, host: ICP_HOST });
  if (IS_LOCAL_NETWORK) await plug.agent.fetchRootKey();
  const principal = (await plug.getPrincipal()).toText();
  return {
    kind: 'plug',
    principal,
    actor: (canisterId, idl) => plug.createActor({ canisterId, interfaceFactory: idl }),
  };
}

const DEV_IDENTITY_KEY = 'markicp.devIdentity';

/**
 * Local-network test wallet: a key generated in the browser and kept in localStorage.
 * Only for the local replica, where Plug has no test ICP.
 */
export async function connectDevWallet(): Promise<Wallet> {
  if (!IS_LOCAL_NETWORK) throw new Error('Dev-гаманець працює тільки з локальною мережею ICP.');
  let identity: Ed25519KeyIdentity | null = null;
  try {
    const raw = localStorage.getItem(DEV_IDENTITY_KEY);
    if (raw) identity = Ed25519KeyIdentity.fromJSON(raw);
  } catch { /* corrupted or blocked storage: make a new key */ }
  if (!identity) {
    identity = Ed25519KeyIdentity.generate();
    try { localStorage.setItem(DEV_IDENTITY_KEY, JSON.stringify(identity.toJSON())); } catch { /* noop */ }
  }
  const agent = await HttpAgent.create({ host: ICP_HOST, identity, shouldFetchRootKey: true });
  return {
    kind: 'dev',
    principal: identity.getPrincipal().toText(),
    actor: async (canisterId, idl) => Actor.createActor(idl, { agent, canisterId }),
  };
}
