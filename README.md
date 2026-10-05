# MARKI ICP COMPANY — кабінет компаній (Internet Computer)

Веб-кабінет для брендів: випуск NFT з мінтом у ICP за тарифом, CRM (замовлення, доставки, NFC), тарифи з оплатою в ICP, панель «Потребує уваги», профіль.

- `web/` — Vite + React + TypeScript, Firebase Auth, ICP (@dfinity/agent, Plug).
- `icp-local/` — канiстра `payments` (тарифи в ICP через ICRC-2, мінт ICRC-7) і локальна мережа.
- `api/` — спільний Rust-бекенд.

## Локальний запуск усього

```bash
cd icp-local && icp network start -d && icp deploy   # потрібен icp-cli
cd ../api && cargo run                               # :8090
cd ../web && cp .env.example .env.local && npm ci && npm run dev:icp   # :3002
```

Деталі — `icp-local/README.md`.

## Бекенд (`api/`)

Rust + Axum, дані у Firebase (Firestore, Storage, Auth).

```bash
cp api/.env.example api/.env   # FIREBASE_SERVICE_ACCOUNT_JSON, FIREBASE_PROJECT_ID, ...
cd api && cargo run            # http://localhost:8090
```

Docker: `docker build -t marki-api .` (Dockerfile у корені). `render.yaml` — приклад деплою на Render.
