import { useCallback, useEffect, useState } from 'react';
import { Principal } from '@dfinity/principal';
import { apiGetNFTs, apiOnchainPending, apiWithdrawOnchain } from '../api';
import { useAuth } from '../auth';
import { custodyAccount, internetIdentityPrincipal, sameAccount } from './custody';
import { paymentsIdl, type Account, type PaymentsService } from './idl';
import { parseIcpMintAddress } from './mint';
import { useWallet } from './useWallet';
import WalletConnect from './WalletConnect';
import { anonymousActor, PAYMENTS_CANISTER_ID } from './wallet';

type Gift = { tokenId: bigint; title: string; toName: string; to: Account };
type State = {
  custody: Account | null;
  inCustody: bigint[];
  inPlug: bigint[];
  inOwnWallet: number;
  gifts: Gift[];
};

const plain = (p: string): Account => ({ owner: Principal.fromText(p), subaccount: [] });
const tokenIds = (addrs: (string | undefined)[]) =>
  addrs.flatMap(a => { const m = parseIcpMintAddress(a); return m && m.canisterId === PAYMENTS_CANISTER_ID ? [m.tokenId] : []; });

/**
 * Where my tokens are on-chain (Marki custody, my Plug, my own wallet) and the
 * moves between them. Gifts move between custody accounts automatically; Plug
 * holdings are moved in once, and anyone can withdraw to their own wallet.
 */
export default function OnChainHandover() {
  const { user } = useAuth();
  const wallet = useWallet();
  const [st, setSt] = useState<State | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [note, setNote] = useState<string | null>(null);

  const load = useCallback(async () => {
    if (!user || !PAYMENTS_CANISTER_ID) return;
    const custody = await custodyAccount(user.uid);
    const payments = await anonymousActor<PaymentsService>(PAYMENTS_CANISTER_ID, paymentsIdl);
    const mine = tokenIds((await apiGetNFTs().catch(() => [])).map(n => n.mintAddress));
    const owners = mine.length ? await payments.icrc7_owner_of(mine) : [];
    const plug = wallet ? plain(wallet.principal) : null;
    const inCustody = mine.filter((_, i) => sameAccount(owners[i]?.[0], custody));
    const inPlug = mine.filter((_, i) => sameAccount(owners[i]?.[0], plug));

    // Old gifts still held by my Plug: they go to the receiver's wallet or Marki custody.
    let gifts: Gift[] = [];
    if (plug) {
      const pending = (await apiOnchainPending().catch(() => [])).flatMap(p => {
        const m = parseIcpMintAddress(p.mintAddress);
        return m && m.canisterId === PAYMENTS_CANISTER_ID ? [{ ...p, tokenId: m.tokenId }] : [];
      });
      const held = pending.length ? await payments.icrc7_owner_of(pending.map(p => p.tokenId)) : [];
      for (const [i, p] of pending.entries()) {
        if (!sameAccount(held[i]?.[0], plug)) continue;
        const to = p.toPrincipal ? plain(p.toPrincipal) : await custodyAccount(p.toUid);
        if (to) gifts.push({ tokenId: p.tokenId, title: p.nftTitle, toName: p.toName, to });
      }
    }
    setSt({ custody, inCustody, inPlug, inOwnWallet: mine.length - inCustody.length - inPlug.length, gifts });
  }, [user, wallet]);

  useEffect(() => { load().catch(() => {}); }, [load]);

  const plugTransfer = async (items: { tokenId: bigint; to: Account }[], label: string) => {
    if (!wallet) return;
    setBusy(label);
    setNote(null);
    try {
      const payments = await wallet.actor<PaymentsService>(PAYMENTS_CANISTER_ID, paymentsIdl);
      let ok = 0;
      for (let i = 0; i < items.length; i += 100) {
        const res = await payments.icrc7_transfer(items.slice(i, i + 100).map(x => ({
          from_subaccount: [], memo: [], created_at_time: [], to: x.to, token_id: x.tokenId,
        })));
        ok += res.filter(r => r[0] && 'Ok' in r[0]).length;
      }
      setNote(`Готово: ${ok} з ${items.length}.`);
      await load();
    } catch (e: any) {
      setNote(e?.message ?? 'Не вдалося');
    } finally {
      setBusy(null);
    }
  };

  const withdraw = async (via: 'ii' | 'plug') => {
    setBusy('withdraw');
    setNote(null);
    try {
      const principal = via === 'plug' ? wallet!.principal : await internetIdentityPrincipal();
      const r = await apiWithdrawOnchain(principal);
      setNote(`Забрано у ваш гаманець: ${r.moved} з ${r.total}. Адреса: ${principal.slice(0, 5)}…${principal.slice(-3)}`);
      await load();
    } catch (e: any) {
      setNote(e?.message ?? 'Не вдалося забрати');
    } finally {
      setBusy(null);
    }
  };

  if (!PAYMENTS_CANISTER_ID || !user) return null;

  return (
    <div className="card onchain-card">
      <div className="oc-row">
        <div>
          <div className="oc-title">⛓️ NFT у блокчейні ICP</div>
          {st && (
            <div className="muted" style={{ fontSize: 12 }}>
              У сховищі Marki: <strong>{st.inCustody.length}</strong>
              {' · '}у вашому Plug: <strong>{wallet ? st.inPlug.length : '—'}</strong>
              {' · '}в інших гаманцях: <strong>{Math.max(0, st.inOwnWallet)}</strong>
            </div>
          )}
        </div>
        {!wallet && <WalletConnect />}
      </div>
      {!st?.custody && st && <div className="muted" style={{ fontSize: 12, marginTop: 8 }}>Сховище Marki ще не ввімкнене на сервері.</div>}

      <div className="onchain-actions">
        {wallet && st?.custody && st.inPlug.length > 0 && (
          <button className="btn btn-primary" disabled={!!busy} onClick={() => plugTransfer(st.inPlug.map(tokenId => ({ tokenId, to: st.custody! })), 'deposit')}>
            {busy === 'deposit' ? 'Підтвердіть у Plug…' : `Перенести з Plug у сховище Marki (${st.inPlug.length})`}
          </button>
        )}
        {st && st.gifts.length > 0 && (
          <button className="btn btn-primary" disabled={!!busy} onClick={() => plugTransfer(st.gifts, 'gifts')}>
            {busy === 'gifts' ? 'Підтвердіть у Plug…' : `Передати подаровані отримувачам (${st.gifts.length})`}
          </button>
        )}
        {st && st.inCustody.length > 0 && (
          <>
            <button className="btn" disabled={!!busy} onClick={() => withdraw('ii')}>
              {busy === 'withdraw' ? 'Забираємо…' : '🔐 Забрати у свій гаманець (Face ID)'}
            </button>
            {wallet && <button className="btn" disabled={!!busy} onClick={() => withdraw('plug')}>Забрати в підключений Plug</button>}
          </>
        )}
      </div>
      <div className="muted" style={{ fontSize: 11, marginTop: 8 }}>
        Подарунки зі сховища Marki переходять у блокчейні автоматично. «Забрати у свій гаманець» — повний контроль у вас (Internet Identity: вхід за Face ID, без застосунків).
      </div>
      {note && <div className="notice notice-info" style={{ marginTop: 10, marginBottom: 0 }}>{note}</div>}
    </div>
  );
}
