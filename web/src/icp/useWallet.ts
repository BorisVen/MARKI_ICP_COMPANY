import { useSyncExternalStore } from 'react';
import { connectDevWallet, connectPlug, type Wallet, type WalletKind } from './wallet';

// One wallet connection shared by every page (Тарифи, NFT).
let current: Wallet | null = null;
const listeners = new Set<() => void>();

function set(w: Wallet | null) {
  current = w;
  listeners.forEach(l => l());
}

function subscribe(l: () => void) {
  listeners.add(l);
  return () => { listeners.delete(l); };
}

const LAST_PRINCIPAL_KEY = 'markicp.lastPrincipal';

/** Principal of the last connected wallet: lets pages show the plan before reconnecting. */
export function lastPrincipal(): string | null {
  try { return localStorage.getItem(LAST_PRINCIPAL_KEY); } catch { return null; }
}

export async function connectWallet(kind: WalletKind): Promise<Wallet> {
  const w = kind === 'plug' ? await connectPlug() : await connectDevWallet();
  try { localStorage.setItem(LAST_PRINCIPAL_KEY, w.principal); } catch { /* noop */ }
  set(w);
  return w;
}

export async function disconnectWallet() {
  if (current?.kind === 'plug') {
    try { await (window as any).ic?.plug?.disconnect?.(); } catch { /* noop */ }
  }
  set(null);
}

export function useWallet(): Wallet | null {
  return useSyncExternalStore(subscribe, () => current);
}
