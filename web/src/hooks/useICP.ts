import { useEffect, useMemo, useState } from 'react';
import { Principal } from '@dfinity/principal';

declare global {
  interface Window {
    ic?: {
      plug?: {
        requestConnect: (opts?: {
          whitelist?: string[];
          host?: string;
        }) => Promise<string>;
        isConnected: () => Promise<boolean>;
        disconnect: () => Promise<void>;
        createAgent: (opts?: { whitelist?: string[]; host?: string }) => Promise<any>;
        agent?: any;
        principalId?: string;
        getPrincipal: () => Promise<{ toText: () => string }>;
      };
    };
  }
}

const ICP_HOST = (import.meta.env.VITE_ICP_HOST as string) || 'https://icp0.io';
const NFT_CANISTER_ID = (import.meta.env.VITE_ICP_NFT_CANISTER_ID as string) || '';

type PlugApi = NonNullable<NonNullable<Window['ic']>['plug']>;

function getPlug(): PlugApi | null {
  return (window as any).ic?.plug ?? null;
}

export function useICP() {
  const plug = useMemo(() => getPlug(), []);

  const [principal, setPrincipal] = useState<string>('');
  const [connected, setConnected] = useState<boolean>(false);

  useEffect(() => {
    let mounted = true;
    const check = async () => {
      const p = getPlug();
      if (!p) return;
      try {
        const ok = await p.isConnected();
        if (!mounted || !ok) return;
        const pk = await p.getPrincipal();
        setPrincipal(pk.toText());
        setConnected(true);
      } catch {
        // ignore
      }
    };
    check();
    return () => { mounted = false; };
  }, []);

  const connect = async (): Promise<string> => {
    const p = getPlug();
    if (!p) {
      throw new Error(
        'Plug Wallet не знайдено. Встанови розширення Plug для браузера та перезавантаж сторінку.'
      );
    }
    const whitelist = NFT_CANISTER_ID ? [NFT_CANISTER_ID] : [];
    await p.requestConnect({ whitelist, host: ICP_HOST });
    const pk = await p.getPrincipal();
    const text = pk.toText();
    setPrincipal(text);
    setConnected(true);
    return text;
  };

  const disconnect = async () => {
    const p = getPlug();
    try { await p?.disconnect?.(); } catch { /* noop */ }
    setPrincipal('');
    setConnected(false);
  };

  const agent = useMemo(() => {
    return plug?.agent ?? null;
  }, [plug, connected]);

  const isReady = Boolean(plug && connected && principal);

  return {
    plug,
    agent,
    principal,
    isReady,
    connected,
    connect,
    disconnect,
    host: ICP_HOST,
    canisterId: NFT_CANISTER_ID,
  };
}

export function principalToText(p: Principal | string | undefined): string {
  if (!p) return '';
  return typeof p === 'string' ? p : p.toText();
}
