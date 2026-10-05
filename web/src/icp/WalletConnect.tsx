import { useState } from 'react';
import { Icon } from '../icons';
import { connectWallet, useWallet } from './useWallet';
import { IS_LOCAL_NETWORK, type WalletKind } from './wallet';

/** Connect buttons (Plug, plus the dev wallet on a local network) or the connected principal. */
export default function WalletConnect({ onError }: { onError?: (msg: string) => void }) {
  const wallet = useWallet();
  const [busy, setBusy] = useState(false);

  if (wallet) {
    const p = wallet.principal;
    return (
      <span className="wallet-pill connected" title={p}>
        <span className="dot-ind" />
        {wallet.kind === 'dev' ? 'Dev · ' : ''}{p.slice(0, 5)}…{p.slice(-3)}
      </span>
    );
  }

  const connect = async (kind: WalletKind) => {
    setBusy(true);
    try {
      await connectWallet(kind);
    } catch (e: any) {
      const msg = e?.message ?? 'Не вдалося підключити гаманець';
      if (onError) onError(msg); else alert(msg);
    } finally {
      setBusy(false);
    }
  };

  return (
    <span style={{ display: 'inline-flex', gap: 8, flexWrap: 'wrap' }}>
      <button type="button" className="btn btn-primary" disabled={busy} onClick={() => connect('plug')}>
        <Icon.Wallet /> {busy ? 'Підключення…' : 'Підключити Plug'}
      </button>
      {IS_LOCAL_NETWORK && (
        <button type="button" className="btn" disabled={busy} onClick={() => connect('dev')}>
          Dev-гаманець
        </button>
      )}
    </span>
  );
}
