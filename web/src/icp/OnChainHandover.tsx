import { useCallback, useEffect, useState } from 'react';
import { Principal } from '@dfinity/principal';
import { apiLinkIcpPrincipal, apiOnchainPending, type PendingOnchain } from '../api';
import { useAuth } from '../auth';
import { paymentsIdl, type PaymentsService } from './idl';
import { parseIcpMintAddress } from './mint';
import { useWallet } from './useWallet';
import WalletConnect from './WalletConnect';
import { anonymousActor, PAYMENTS_CANISTER_ID } from './wallet';

type Row = PendingOnchain & { tokenId: bigint };

/**
 * Gifts move NFTs in Marki at once; the ICP token follows when the sender signs
 * a transfer in Plug. Also lets anyone link their Plug, so gifts reach them on-chain.
 */
export default function OnChainHandover() {
  const { user, reload } = useAuth();
  const wallet = useWallet();
  const [rows, setRows] = useState<Row[] | null>(null);
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState<string | null>(null);

  const linked = user?.icpPrincipal;
  const isLinked = !!wallet && linked === wallet.principal;

  // Tokens I gave away that the canister still shows in my connected wallet.
  const load = useCallback(async () => {
    if (!wallet || !PAYMENTS_CANISTER_ID) { setRows(null); return; }
    const list = (await apiOnchainPending().catch(() => [])).flatMap(p => {
      const m = parseIcpMintAddress(p.mintAddress);
      return m && m.canisterId === PAYMENTS_CANISTER_ID ? [{ ...p, tokenId: m.tokenId }] : [];
    });
    if (list.length === 0) { setRows([]); return; }
    const payments = await anonymousActor<PaymentsService>(PAYMENTS_CANISTER_ID, paymentsIdl);
    const owners = await payments.icrc7_owner_of(list.map(r => r.tokenId));
    setRows(list.filter((_, i) => owners[i]?.[0]?.owner.toText() === wallet.principal));
  }, [wallet]);

  useEffect(() => { load(); }, [load]);

  const link = async () => {
    if (!wallet) return;
    setBusy(true);
    try { await apiLinkIcpPrincipal(wallet.principal); await reload(); setNote('Гаманець прив’язано: подарунки надходитимуть сюди в блокчейні.'); }
    catch (e: any) { setNote(e?.message ?? 'Не вдалося прив’язати'); }
    finally { setBusy(false); }
  };

  const ready = rows?.filter(r => r.toPrincipal) ?? [];
  const waiting = rows?.filter(r => !r.toPrincipal) ?? [];

  const send = async () => {
    if (!wallet || ready.length === 0) return;
    setBusy(true);
    setNote(null);
    try {
      const payments = await wallet.actor<PaymentsService>(PAYMENTS_CANISTER_ID, paymentsIdl);
      let ok = 0;
      for (let i = 0; i < ready.length; i += 100) {
        const chunk = ready.slice(i, i + 100);
        const res = await payments.icrc7_transfer(chunk.map(r => ({
          from_subaccount: [], memo: [], created_at_time: [],
          to: { owner: Principal.fromText(r.toPrincipal!), subaccount: [] },
          token_id: r.tokenId,
        })));
        ok += res.filter(x => x[0] && 'Ok' in x[0]).length;
      }
      setNote(`Передано в блокчейні: ${ok} з ${ready.length}.`);
      await load();
    } catch (e: any) {
      setNote(e?.message ?? 'Не вдалося передати');
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="card onchain-card">
      <div className="oc-row">
        <div>
          <div className="oc-title">⛓️ Подарунки в блокчейні ICP</div>
          <div className="muted" style={{ fontSize: 12 }}>
            {!wallet ? 'Підключіть Plug: щоб отримувати подарунки в блокчейн і передавати свої.'
              : isLinked ? `Ваш гаманець прив’язано · ${wallet.principal.slice(0, 5)}…${wallet.principal.slice(-3)}`
                : 'Цей гаманець ще не прив’язано до профілю — подарунки не прийдуть на нього.'}
          </div>
        </div>
        {!wallet ? <WalletConnect /> : !isLinked && <button className="btn" onClick={link} disabled={busy}>Прив’язати до профілю</button>}
      </div>

      {wallet && rows && rows.length > 0 && (
        <div style={{ marginTop: 12 }}>
          {ready.length > 0 && (
            <button className="btn btn-primary btn-block" onClick={send} disabled={busy}>
              {busy ? 'Підтвердіть у Plug…' : `Передати в блокчейні (${ready.length})`}
            </button>
          )}
          <div className="onchain-list">
            {ready.map(r => <div key={r.mintAddress}><span>«{r.nftTitle}» → {r.toName || 'отримувач'}</span><span className="muted">№ {r.tokenId.toString()}</span></div>)}
            {waiting.map(r => <div key={r.mintAddress} className="muted"><span>«{r.nftTitle}» → {r.toName || 'отримувач'}</span><span>чекає, поки підключить Plug</span></div>)}
          </div>
        </div>
      )}
      {note && <div className="notice notice-info" style={{ marginTop: 10, marginBottom: 0 }}>{note}</div>}
    </div>
  );
}
