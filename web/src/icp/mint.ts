import { paymentsIdl, type PaymentsService, type Subscription } from './idl';
import { anonymousActor, PAYMENTS_CANISTER_ID, type Wallet } from './wallet';

/** On-chain minting is on when the payments/NFT canister is configured. */
export const ICP_MINT_ENABLED = Boolean(PAYMENTS_CANISTER_ID);

/** Below the 2 MB ingress limit, with room for the Candid envelope. */
const CHUNK_BYTES = 1_000_000;
/** 1T cycles = 1 XDR ≈ $1.36643. */
export const XDR_USD = 1.36643;

export type OnChainMint = {
  tokenIds: bigint[];
  imageBytes: number;
  /** Real canister cycle balance drop across the whole upload + mint. */
  cyclesSpent: bigint;
  subscription: Subscription;
};

export const cyclesToUsd = (c: bigint) => (Number(c) / 1e12) * XDR_USD;

/** `icp:<canister>:<token>`, stored in the backend NFT as `mintAddress`. */
export const icpMintAddress = (tokenId: bigint) => `icp:${PAYMENTS_CANISTER_ID}:${tokenId}`;

export function parseIcpMintAddress(addr?: string): { canisterId: string; tokenId: bigint } | null {
  const m = addr?.match(/^icp:([a-z0-9-]+):(\d+)$/);
  return m ? { canisterId: m[1], tokenId: BigInt(m[2]) } : null;
}

function unwrap<T>(r: { Ok: T } | { Err: string }): T {
  if ('Err' in r) throw new Error(r.Err);
  return r.Ok;
}

async function cycleBalance(): Promise<bigint> {
  const payments = await anonymousActor<PaymentsService>(PAYMENTS_CANISTER_ID, paymentsIdl);
  return (await payments.nft_stats()).cycles;
}

/**
 * Upload a photo in chunks and mint `copies` NFTs (editions share one image).
 * The canister takes the plan credits in the mint call; it fails without a plan.
 */
export async function mintPhotoOnChain(
  wallet: Wallet,
  file: Blob,
  metadata: Record<string, unknown>,
  copies = 1,
  onProgress?: (text: string) => void,
): Promise<OnChainMint> {
  const payments = await wallet.actor<PaymentsService>(PAYMENTS_CANISTER_ID, paymentsIdl);
  const bytes = new Uint8Array(await file.arrayBuffer());
  const before = await cycleBalance();

  const uploadId = unwrap(await payments.create_upload());
  for (let off = 0; off < bytes.length; off += CHUNK_BYTES) {
    onProgress?.(`Завантаження фото в канiстру: ${Math.min(100, Math.round(((off + CHUNK_BYTES) / bytes.length) * 100))}%`);
    unwrap(await payments.append_chunk(uploadId, bytes.subarray(off, off + CHUNK_BYTES)));
  }
  onProgress?.(copies > 1 ? `Мінт ${copies} editions у ICP…` : 'Мінт у ICP…');
  const res = unwrap(await payments.mint_uploaded(uploadId, JSON.stringify(metadata), copies));

  const after = await cycleBalance();
  return {
    tokenIds: res.token_ids,
    imageBytes: Number(res.image_bytes),
    cyclesSpent: before > after ? before - after : 0n,
    subscription: res.subscription,
  };
}

/** Sum several mints (a collection = one mint per photo). */
export function mergeMints(mints: OnChainMint[]): OnChainMint | null {
  if (mints.length === 0) return null;
  return {
    tokenIds: mints.flatMap(m => m.tokenIds),
    imageBytes: mints.reduce((a, m) => a + m.imageBytes, 0),
    cyclesSpent: mints.reduce((a, m) => a + m.cyclesSpent, 0n),
    subscription: mints[mints.length - 1].subscription,
  };
}

/** Read an on-chain NFT image as an object URL. */
export async function onChainImageUrl(tokenId: bigint): Promise<string | null> {
  const payments = await anonymousActor<PaymentsService>(PAYMENTS_CANISTER_ID, paymentsIdl);
  const img = await payments.get_image(tokenId);
  if (!img[0]) return null;
  return URL.createObjectURL(new Blob([img[0] as BlobPart], { type: 'image/jpeg' }));
}
