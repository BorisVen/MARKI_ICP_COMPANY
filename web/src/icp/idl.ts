import type { IDL as IDLType } from '@dfinity/candid';
import type { Principal } from '@dfinity/principal';

// ── ICRC-1 / ICRC-2 ledger (ICP ledger or any ICRC-2 token) ─────────────────

export type Account = { owner: Principal; subaccount: [] | [Uint8Array] };

export type LedgerError = Record<string, unknown>;

export interface LedgerService {
  icrc1_balance_of: (a: Account) => Promise<bigint>;
  icrc1_fee: () => Promise<bigint>;
  icrc2_approve: (args: {
    from_subaccount: [] | [Uint8Array];
    spender: Account;
    amount: bigint;
    expected_allowance: [] | [bigint];
    expires_at: [] | [bigint];
    fee: [] | [bigint];
    memo: [] | [Uint8Array];
    created_at_time: [] | [bigint];
  }) => Promise<{ Ok: bigint } | { Err: LedgerError }>;
}

export const ledgerIdl: IDLType.InterfaceFactory = ({ IDL }) => {
  const Account = IDL.Record({ owner: IDL.Principal, subaccount: IDL.Opt(IDL.Vec(IDL.Nat8)) });
  const ApproveError = IDL.Variant({
    BadFee: IDL.Record({ expected_fee: IDL.Nat }),
    InsufficientFunds: IDL.Record({ balance: IDL.Nat }),
    AllowanceChanged: IDL.Record({ current_allowance: IDL.Nat }),
    Expired: IDL.Record({ ledger_time: IDL.Nat64 }),
    TooOld: IDL.Null,
    CreatedInFuture: IDL.Record({ ledger_time: IDL.Nat64 }),
    Duplicate: IDL.Record({ duplicate_of: IDL.Nat }),
    TemporarilyUnavailable: IDL.Null,
    GenericError: IDL.Record({ error_code: IDL.Nat, message: IDL.Text }),
  });
  return IDL.Service({
    icrc1_balance_of: IDL.Func([Account], [IDL.Nat], ['query']),
    icrc1_fee: IDL.Func([], [IDL.Nat], ['query']),
    icrc2_approve: IDL.Func(
      [IDL.Record({
        from_subaccount: IDL.Opt(IDL.Vec(IDL.Nat8)),
        spender: Account,
        amount: IDL.Nat,
        expected_allowance: IDL.Opt(IDL.Nat),
        expires_at: IDL.Opt(IDL.Nat64),
        fee: IDL.Opt(IDL.Nat),
        memo: IDL.Opt(IDL.Vec(IDL.Nat8)),
        created_at_time: IDL.Opt(IDL.Nat64),
      })],
      [IDL.Variant({ Ok: IDL.Nat, Err: ApproveError })],
      [],
    ),
  });
};

// ── MarkICP payments canister (icp-local/payments.did) ──────────────────────

export type Plan = {
  id: string;
  name: string;
  price_usd_cents: bigint;
  period_days: number;
  photo_mints: bigint;
  metadata_mints: bigint;
  max_photo_kb: bigint;
  storage_years: number;
};

export type PlanQuote = {
  plan: Plan;
  price_e8s: bigint;
  xdr_permyriad_per_icp: bigint;
  rate_timestamp_seconds: bigint;
  max_cost_cycles: bigint;
};

export type Subscription = {
  owner: Principal;
  plan_id: string;
  photo_mints_left: bigint;
  metadata_mints_left: bigint;
  max_photo_kb: bigint;
  expires_at_ns: bigint;
};

export type PlanPurchase = {
  id: bigint;
  buyer: Principal;
  plan_id: string;
  price_e8s: bigint;
  block: bigint;
  timestamp_ns: bigint;
};

export type Pricing = {
  xdr_usd: number;
  xdr_permyriad_per_icp: bigint;
  ledger: Principal;
  ledger_fee_e8s: bigint;
};

export type MintResult = {
  token_ids: bigint[];
  image_bytes: bigint;
  subscription: Subscription;
};

export type NftStats = {
  cycles: bigint;
  tokens: bigint;
  images: bigint;
  image_bytes: bigint;
  heap_bytes: bigint;
};

type Res<T> = { Ok: T } | { Err: string };

export interface PaymentsService {
  create_upload: () => Promise<Res<bigint>>;
  append_chunk: (uploadId: bigint, chunk: Uint8Array) => Promise<Res<bigint>>;
  mint_uploaded: (uploadId: bigint, metadata: string, copies: number) => Promise<Res<MintResult>>;
  mint_metadata: (items: string[]) => Promise<Res<MintResult>>;
  get_image: (tokenId: bigint) => Promise<[] | [Uint8Array]>;
  nft_stats: () => Promise<NftStats>;
  refresh_rate: () => Promise<Res<bigint>>;
  plans: () => Promise<Res<PlanQuote[]>>;
  buy_plan: (planId: string, maxPriceE8s: bigint) => Promise<Res<Subscription>>;
  subscription_of: (owner: Principal) => Promise<[] | [Subscription]>;
  purchases_of: (owner: Principal) => Promise<PlanPurchase[]>;
  pricing: () => Promise<Pricing>;
}

export const paymentsIdl: IDLType.InterfaceFactory = ({ IDL }) => {
  const Plan = IDL.Record({
    id: IDL.Text,
    name: IDL.Text,
    price_usd_cents: IDL.Nat64,
    period_days: IDL.Nat32,
    photo_mints: IDL.Nat64,
    metadata_mints: IDL.Nat64,
    max_photo_kb: IDL.Nat64,
    storage_years: IDL.Nat16,
  });
  const PlanQuote = IDL.Record({
    plan: Plan,
    price_e8s: IDL.Nat64,
    xdr_permyriad_per_icp: IDL.Nat64,
    rate_timestamp_seconds: IDL.Nat64,
    max_cost_cycles: IDL.Nat,
  });
  const Subscription = IDL.Record({
    owner: IDL.Principal,
    plan_id: IDL.Text,
    photo_mints_left: IDL.Nat64,
    metadata_mints_left: IDL.Nat64,
    max_photo_kb: IDL.Nat64,
    expires_at_ns: IDL.Nat64,
  });
  const PlanPurchase = IDL.Record({
    id: IDL.Nat64,
    buyer: IDL.Principal,
    plan_id: IDL.Text,
    price_e8s: IDL.Nat64,
    block: IDL.Nat,
    timestamp_ns: IDL.Nat64,
  });
  // Candid decodes records by field hash, so extra canister fields are skipped.
  const Pricing = IDL.Record({
    xdr_usd: IDL.Float64,
    xdr_permyriad_per_icp: IDL.Nat64,
    ledger: IDL.Principal,
    ledger_fee_e8s: IDL.Nat64,
  });
  const MintResult = IDL.Record({
    token_ids: IDL.Vec(IDL.Nat64),
    image_bytes: IDL.Nat64,
    subscription: Subscription,
  });
  const NftStats = IDL.Record({
    cycles: IDL.Nat,
    tokens: IDL.Nat64,
    images: IDL.Nat64,
    image_bytes: IDL.Nat64,
    heap_bytes: IDL.Nat64,
  });
  const res = (t: IDLType.Type) => IDL.Variant({ Ok: t, Err: IDL.Text });
  return IDL.Service({
    create_upload: IDL.Func([], [res(IDL.Nat64)], []),
    append_chunk: IDL.Func([IDL.Nat64, IDL.Vec(IDL.Nat8)], [res(IDL.Nat64)], []),
    mint_uploaded: IDL.Func([IDL.Nat64, IDL.Text, IDL.Nat32], [res(MintResult)], []),
    mint_metadata: IDL.Func([IDL.Vec(IDL.Text)], [res(MintResult)], []),
    get_image: IDL.Func([IDL.Nat64], [IDL.Opt(IDL.Vec(IDL.Nat8))], ['query']),
    nft_stats: IDL.Func([], [NftStats], ['query']),
    refresh_rate: IDL.Func([], [res(IDL.Nat64)], []),
    plans: IDL.Func([], [res(IDL.Vec(PlanQuote))], ['query']),
    buy_plan: IDL.Func([IDL.Text, IDL.Nat64], [res(Subscription)], []),
    subscription_of: IDL.Func([IDL.Principal], [IDL.Opt(Subscription)], ['query']),
    purchases_of: IDL.Func([IDL.Principal], [IDL.Vec(PlanPurchase)], ['query']),
    pricing: IDL.Func([], [Pricing], ['query']),
  });
};
