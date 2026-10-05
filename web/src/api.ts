import { auth } from './firebase';

const BASE_URL =
  (import.meta.env.VITE_API_BASE_URL as string) ||
  (import.meta.env.VITE_API_URL as string) ||
  'https://idenity.onrender.com';

// console.log('[API] API_BASE_URL:', BASE_URL);

async function token(): Promise<string | null> {
  try {
    return (await auth.currentUser?.getIdToken()) ?? null;
  } catch {
    return null;
  }
}

/**
 * fetch against the API with readable errors: a network failure names the API
 * host, and a JSON error body (`{"error": "..."}`) becomes its message.
 */
async function send(path: string, init: RequestInit): Promise<Response> {
  let res: Response;
  try {
    res = await fetch(`${BASE_URL}${path}`, init);
  } catch {
    throw new Error(`Сервер ${BASE_URL} недоступний. Перевір VITE_API_BASE_URL і чи запущено API.`);
  }
  if (!res.ok) {
    const text = await res.text().catch(() => '');
    let msg = text;
    try {
      const j = JSON.parse(text);
      msg = j?.error ?? j?.message ?? text;
    } catch { /* plain text body */ }
    if (res.status === 401) msg = `Сесія недійсна, увійди знову. ${msg}`.trim();
    if (res.status === 404 && !msg) msg = `Метод API не знайдено: ${path}`;
    throw new Error(msg || `HTTP ${res.status}`);
  }
  return res;
}

async function req<T>(method: string, path: string, body?: unknown, asForm = false): Promise<T> {
  const t = await token();
  const headers: Record<string, string> = {};
  if (t) headers['Authorization'] = `Bearer ${t}`;
  if (body && !asForm) headers['Content-Type'] = 'application/json';

  const res = await send(path, {
    method,
    headers,
    body: asForm ? (body as BodyInit) : body !== undefined ? JSON.stringify(body) : undefined,
  });

  if (res.status === 204) return undefined as T;
  return res.json() as Promise<T>;
}

const get  = <T>(p: string)                 => req<T>('GET',    p);
const post = <T>(p: string, b?: unknown)    => req<T>('POST',   p, b);
const put  = <T>(p: string, b?: unknown)    => req<T>('PUT',    p, b);
const del  = <T>(p: string)                 => req<T>('DELETE', p);

// ── Types ────────────────────────────────────────────────────────────────────

export type User = {
  uid: string;
  name: string;
  username: string;
  email: string;
  phone?: string;
  avatar?: string;
  location?: string;
  bio?: string;
  createdAt: string;
  companyApproved: boolean;
  pendingApproval: boolean;
  banned?: boolean;
  deliveryAddress?: string;
  roles?: string[];
  companyId?: string;
  companyName?: string;
  registrationNumber?: string;
  contactEmail?: string;
  businessDescription?: string;
  approvalStatus?: 'pending' | 'approved' | 'rejected' | 'banned';
  rejectionReason?: string;
  banReason?: string;
};

export type PublicUser = {
  uid: string;
  name: string;
  username: string;
  avatar?: string;
  bio?: string;
  companyApproved: boolean;
};

export type NFT = {
  id: string;
  walletNftId?: string;
  title: string;
  description?: string;
  image?: string;
  imageUrl?: string;
  attributes?: { trait_type: string; value: string }[];
  forSale?: boolean;
  price?: number;
  currency?: string;
  userId?: string;
  tokenId?: string;
  metadataUri?: string;
  nfcUid?: string;
  tags?: string[];
  category?: string;
  blockchain?: string;
  royalty?: number;
  createdAt?: string;
  batchId?: string;
  batchName?: string;
  batchIndex?: number;
  batchSize?: number;
  qrImageUrl?: string;
  /** `icp:<canister>:<token>` after an on-chain mint. */
  mintAddress?: string;
};

export type Post = {
  id: string;
  userId: string;
  authorName?: string;
  authorAvatar?: string;
  text?: string;
  title?: string;
  description?: string;
  nftId?: string;
  walletNftId?: string;
  walletNftIds?: string[];
  nftTitle?: string;
  nftImage?: string;
  nftImages?: string[];
  forSale?: boolean;
  price?: number | null;
  currency?: string;
  blockchain?: string;
  tags?: string[];
  likes?: number;
  likedBy?: string[];
  comments?: { id: string; userId: string; text: string; createdAt: string }[];
  createdAt: string;
};

export type CodOrder = {
  id: string;
  postId: string;
  nftId: string;
  nftTitle: string;
  buyerId: string;
  buyerName: string;
  sellerId: string;
  price: number;
  nftCurrency: string;
  paymentCurrency: string;
  deliveryAddress: string;
  fullName: string;
  phone: string;
  status: 'pending' | 'in_delivery' | 'completed' | 'cancelled';
  createdAt: string;
  deliveryId?: string;
};

export type DeliveryCheckpoint = {
  id: string;
  status: string;
  location: string;
  timestamp: string;
  recordedBy: string;
  recordedByName?: string;
  note?: string;
};

export type Delivery = {
  id: string;
  orderId?: string;
  nftId: string;
  nftTitle: string;
  sellerId: string;
  buyerId: string;
  buyerName: string;
  deliveryAddress: string;
  carrierType: 'self' | 'nova_poshta';
  courierId?: string;
  courierName?: string;
  controllerId?: string;
  controllerName?: string;
  npTrackingNumber?: string;
  npLastSyncedAt?: string;
  status: string;
  checkpoints: DeliveryCheckpoint[];
  customerReceived: boolean;
  receivedAt?: string;
  nfcUid?: string;
  nfcVerified: boolean;
  nfcVerifiedAt?: string;
  createdAt: string;
  updatedAt: string;
};

export type CreateDeliveryInput = {
  orderId?: string;
  nftId: string;
  buyerId: string;
  deliveryAddress: string;
  carrierType: 'self' | 'nova_poshta';
  npTrackingNumber?: string;
  courierId?: string;
  controllerId?: string;
  nfcUid?: string;
  createdAt?: string;
  status?: string;
  initialLocation?: string;
  initialNote?: string;
};

export type MarkiWallet = {
  uid: string;
  email: string;
  balance: Record<string, number>;
  fingerprintEnabled: boolean;
};

export type CryptoWallet = {
  id: string;
  address: string;
  label?: string;
  balance?: number;
  lastBalanceAt?: string;
};

export type Notification = {
  id: string;
  type: string;
  title: string;
  body: string;
  read: boolean;
  createdAt: string;
  data?: Record<string, unknown>;
};

// ── Auth + profile ───────────────────────────────────────────────────────────

export const apiMe = () => get<User>('/api/auth/me');

export const apiGetProfile = (uid: string) => get<User>(`/api/profile/${uid}`);

export const apiUpdateProfile = (uid: string, d: Partial<User>) =>
  put<User>(`/api/profile/${uid}`, d);

export const apiUploadAvatar = async (uid: string, file: File): Promise<{ avatar: string }> => {
  const t = await token();
  const res = await send(`/api/profile/${uid}/avatar`, {
    method: 'POST',
    headers: {
      Authorization: `Bearer ${t}`,
      'Content-Type': file.type || 'image/jpeg',
    },
    body: file,
  });
  return res.json();
};

export const apiChangePassword = (uid: string, newPassword: string) =>
  put<void>(`/api/profile/${uid}/password`, { newPassword });

export const apiRequestApproval = (uid: string, d: {
  companyName: string;
  registrationNumber: string;
  contactEmail: string;
  description: string;
}) => post<void>(`/api/profile/${uid}/request-approval`, d);

export const apiSearchUsers = (q: string) =>
  get<PublicUser[]>(`/api/users/search?q=${encodeURIComponent(q)}`);

// ── NFTs ─────────────────────────────────────────────────────────────────────

export const apiGetNFTs = () => get<NFT[]>('/api/nfts');
export const apiGetNFT  = (id: string) => get<NFT>(`/api/nfts/${id}`);
export const apiUpdateNFT = (id: string, d: Partial<NFT>) => put<NFT>(`/api/nfts/${id}`, d);
export const apiDeleteNFT = (id: string) => del<void>(`/api/nfts/${id}`);

export const apiGetMintInfo = () => get<{
  mintCount: number;
  isFree: boolean;
  /** Estimated cycles cost per mint (display only; backend pays reverse gas). */
  estimatedCyclesPerMint?: number;
}>('/api/nfts/mint-info');

export const apiCreateNFT = async (form: FormData) => {
  const t = await token();
  const res = await send(`/api/nfts`, {
    method: 'POST',
    headers: { Authorization: `Bearer ${t}` },
    body: form,
  });
  return res.json() as Promise<NFT & { metadataUri?: string }>;
};

export const apiBatchCreateNFTs = async (form: FormData) => {
  const t = await token();
  const res = await send(`/api/nfts/batch`, {
    method: 'POST',
    headers: { Authorization: `Bearer ${t}` },
    body: form,
  });
  return res.json();
};

export const apiCreateEditionNFTs = async (form: FormData) => {
  const t = await token();
  const res = await send(`/api/nfts/editions`, {
    method: 'POST',
    headers: { Authorization: `Bearer ${t}` },
    body: form,
  });
  return res.json();
};

export const apiGenerateNFTQr = async (nftId: string) => {
  const t = await token();
  const res = await send(`/api/nfts/${encodeURIComponent(nftId)}/qr`, {
    method: 'POST',
    headers: { Authorization: `Bearer ${t}` },
  });
  return res.json() as Promise<NFT>;
};

export const apiGenerateMissingQr = async () => {
  const t = await token();
  const res = await send(`/api/nfts/qr-missing`, {
    method: 'POST',
    headers: { Authorization: `Bearer ${t}` },
  });
  return res.json() as Promise<{ generated: number }>;
};

export const apiRegenerateAllQr = async () => {
  const t = await token();
  const res = await send(`/api/nfts/regenerate-qr`, {
    method: 'POST',
    headers: { Authorization: `Bearer ${t}` },
  });
  return res.json() as Promise<{ regenerated: number }>;
};

// ── Posts (marketplace + feed) ───────────────────────────────────────────────

export const apiGetPosts   = () => get<Post[]>('/api/posts');
export const apiCreatePost = (d: Partial<Post>) => post<Post>('/api/posts', d);
export const apiDeletePost = (id: string) => del<void>(`/api/posts/${id}`);
export const apiLikePost   = (id: string) => post<Post>(`/api/posts/${id}/like`);
export const apiAddComment = (id: string, text: string) =>
  post<Post>(`/api/posts/${id}/comments`, { text });

// ── COD orders ───────────────────────────────────────────────────────────────

export const apiListCodOrders = () => get<CodOrder[]>('/api/cod-orders');
export const apiAcceptCodOrder = (id: string, d: {
  carrierType: 'self' | 'nova_poshta';
  npTrackingNumber?: string;
  courierId?: string;
  controllerId?: string;
  nfcUid?: string;
}) => post<Delivery>(`/api/cod-orders/${id}/accept`, d);

// ── Deliveries ───────────────────────────────────────────────────────────────

export const apiListDeliveries = () => get<Delivery[]>('/api/deliveries');
export const apiGetDelivery    = (id: string) => get<Delivery>(`/api/deliveries/${id}`);
export const apiCreateDelivery = (d: CreateDeliveryInput) =>
  post<Delivery>('/api/deliveries', d);
export const apiUpdateCarrier  = (id: string, d: {
  carrierType: 'self' | 'nova_poshta';
  npTrackingNumber?: string;
  courierId?: string;
  controllerId?: string;
}) => put<Delivery>(`/api/deliveries/${id}/carrier`, d);
export const apiUpdateDeliveryStatus = (id: string, status: string) =>
  put<Delivery>(`/api/deliveries/${id}/status`, { status });
export const apiAddCheckpoint = (id: string, d: { status: string; location: string; timestamp?: string; note?: string }) =>
  post<Delivery>(`/api/deliveries/${id}/checkpoints`, d);
export const apiSyncNovaPoshta = (id: string) =>
  post<Delivery>(`/api/deliveries/${id}/sync-novaposhta`);
export const apiConfirmReceipt = (id: string) =>
  post<Delivery>(`/api/deliveries/${id}/confirm-receipt`);

// ── NFC ──────────────────────────────────────────────────────────────────────

export const apiBindNfc   = (d: { nftId: string; nfcUid: string }) =>
  post<{ success: boolean; nfcUid: string; nftId: string }>('/api/nfc/bind', d);

export const apiVerifyNfc = (nfcUid: string) =>
  post<{
    nftId: string;
    nftTitle: string;
    ownerId: string;
    ownerName: string;
    tokenId?: string;
    deliveryId?: string;
    autoConfirmedReceipt: boolean;
  }>('/api/nfc/verify', { nfcUid });

// ── Wallets ──────────────────────────────────────────────────────────────────

export const apiGetMarkiWallet    = () => get<MarkiWallet>('/api/wallets/marki');
export const apiUpdateMarkiEmail  = (newEmail: string) =>
  put<void>('/api/wallets/marki/email', { newEmail });
export const apiUpdateFingerprint = (enabled: boolean) =>
  put<void>('/api/wallets/marki/fingerprint', { enabled });

export const apiGetCryptoWallets = () => get<CryptoWallet[]>('/api/wallets/crypto');
export const apiAddCryptoWallet  = (d: { address: string; label?: string }) =>
  post<CryptoWallet>('/api/wallets/crypto', d);
export const apiRemoveCryptoWallet = (id: string) => del<void>(`/api/wallets/crypto/${id}`);
export const apiRefreshWalletBalance = (id: string) =>
  put<CryptoWallet>(`/api/wallets/crypto/${id}/balance`);

// ── Notifications ────────────────────────────────────────────────────────────

export const apiGetNotifications   = () => get<Notification[]>('/api/notifications');
export const apiMarkRead           = (id: string) => put<void>(`/api/notifications/${id}/read`);
export const apiMarkAllRead        = () => put<void>('/api/notifications/read-all');
export const apiDeleteNotification = (id: string) => del<void>(`/api/notifications/${id}`);

// ── AI image generation ──────────────────────────────────────────────────────

export const apiAiGenerateImage = async (prompt: string): Promise<Blob> => {
  const res = await send(`/api/ai/generate`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ prompt }),
  });
  return res.blob();
};
