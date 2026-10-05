/// <reference types="vite/client" />

interface ImportMetaEnv {
  readonly VITE_API_BASE_URL?: string;
  readonly VITE_API_URL?: string;
  readonly VITE_ICP_HOST?: string;
  readonly VITE_ICP_NFT_CANISTER_ID?: string;
  readonly VITE_ICP_PAYMENTS_CANISTER_ID?: string;
  readonly VITE_ICP_LEDGER_CANISTER_ID?: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}
