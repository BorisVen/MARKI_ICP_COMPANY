#!/usr/bin/env node
import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { Connection, PublicKey } from '@solana/web3.js';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(__dirname, '..', '..');
const apiEnvPath = path.join(repoRoot, 'api', '.env');

function parseArgs(argv) {
  const out = { keepIds: [] };
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === '--uid') out.uid = argv[++i];
    else if (arg === '--keep-batch') out.keepBatch = argv[++i];
    else if (arg === '--keep-id') out.keepIds.push(argv[++i]);
    else if (arg === '--apply') out.apply = true;
    else if (arg === '--delete-public') out.deletePublic = true;
    else if (arg === '--summary') out.summary = true;
    else if (arg === '--help') out.help = true;
    else throw new Error(`Unknown argument: ${arg}`);
  }
  return out;
}

function parseEnvFile(file) {
  const env = {};
  const lines = fs.readFileSync(file, 'utf8').split(/\r?\n/);
  for (const line of lines) {
    const trimmed = line.trim();
    if (!trimmed || trimmed.startsWith('#')) continue;
    const eq = trimmed.indexOf('=');
    if (eq === -1) continue;
    const key = trimmed.slice(0, eq);
    let value = trimmed.slice(eq + 1);
    if ((value.startsWith('"') && value.endsWith('"')) || (value.startsWith("'") && value.endsWith("'"))) {
      value = value.slice(1, -1);
    }
    env[key] = value;
  }
  return env;
}

function base64url(input) {
  return Buffer.from(input)
    .toString('base64')
    .replaceAll('+', '-')
    .replaceAll('/', '_')
    .replace(/=+$/u, '');
}

async function accessToken(serviceAccount) {
  const now = Math.floor(Date.now() / 1000);
  const header = { alg: 'RS256', typ: 'JWT' };
  const claims = {
    iss: serviceAccount.client_email,
    scope: 'https://www.googleapis.com/auth/datastore',
    aud: serviceAccount.token_uri || 'https://oauth2.googleapis.com/token',
    iat: now,
    exp: now + 3600,
  };
  const unsigned = `${base64url(JSON.stringify(header))}.${base64url(JSON.stringify(claims))}`;
  const signature = crypto.sign('RSA-SHA256', Buffer.from(unsigned), serviceAccount.private_key);
  const assertion = `${unsigned}.${base64url(signature)}`;
  const body = new URLSearchParams({
    grant_type: 'urn:ietf:params:oauth:grant-type:jwt-bearer',
    assertion,
  });

  const resp = await fetch(claims.aud, {
    method: 'POST',
    headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
    body,
  });
  if (!resp.ok) throw new Error(`Google token error ${resp.status}: ${await resp.text()}`);
  const json = await resp.json();
  return json.access_token;
}

function fromFs(value) {
  if (!value) return null;
  if ('stringValue' in value) return value.stringValue;
  if ('booleanValue' in value) return value.booleanValue;
  if ('integerValue' in value) return Number(value.integerValue);
  if ('doubleValue' in value) return value.doubleValue;
  if ('nullValue' in value) return null;
  if ('arrayValue' in value) return (value.arrayValue.values || []).map(fromFs);
  if ('mapValue' in value) {
    const out = {};
    for (const [key, child] of Object.entries(value.mapValue.fields || {})) out[key] = fromFs(child);
    return out;
  }
  return null;
}

function toFs(value) {
  if (value === null || value === undefined) return { nullValue: null };
  if (typeof value === 'string') return { stringValue: value };
  if (typeof value === 'boolean') return { booleanValue: value };
  if (typeof value === 'number') {
    return Number.isInteger(value) ? { integerValue: String(value) } : { doubleValue: value };
  }
  if (Array.isArray(value)) return { arrayValue: { values: value.map(toFs) } };
  if (typeof value === 'object') {
    const fields = {};
    for (const [key, child] of Object.entries(value)) {
      if (child !== undefined) fields[key] = toFs(child);
    }
    return { mapValue: { fields } };
  }
  return { stringValue: String(value) };
}

function docToJson(doc) {
  const out = {};
  for (const [key, value] of Object.entries(doc.fields || {})) out[key] = fromFs(value);
  out.__docId = doc.name.split('/').pop();
  return out;
}

function jsonToDoc(fields) {
  const out = {};
  for (const [key, value] of Object.entries(fields)) out[key] = toFs(value);
  return { fields: out };
}

class Firestore {
  constructor(projectId, token) {
    this.projectId = projectId;
    this.token = token;
    this.root = `https://firestore.googleapis.com/v1/projects/${projectId}/databases/(default)/documents`;
  }

  async get(collection, docId) {
    const resp = await fetch(`${this.root}/${collection}/${encodeURIComponent(docId)}`, {
      headers: { Authorization: `Bearer ${this.token}` },
    });
    if (resp.status === 404) return null;
    if (!resp.ok) throw new Error(`Firestore get ${collection}/${docId}: ${await resp.text()}`);
    return docToJson(await resp.json());
  }

  async query(collection) {
    const resp = await fetch(`https://firestore.googleapis.com/v1/projects/${this.projectId}/databases/(default)/documents:runQuery`, {
      method: 'POST',
      headers: {
        Authorization: `Bearer ${this.token}`,
        'Content-Type': 'application/json',
      },
      body: JSON.stringify({ structuredQuery: { from: [{ collectionId: collection }] } }),
    });
    if (!resp.ok) throw new Error(`Firestore query ${collection}: ${await resp.text()}`);
    const rows = await resp.json();
    return rows.filter(row => row.document).map(row => docToJson(row.document));
  }

  async update(collection, docId, fields) {
    const masks = Object.keys(fields)
      .map(key => `updateMask.fieldPaths=${encodeURIComponent(key)}`)
      .join('&');
    const resp = await fetch(`${this.root}/${collection}/${encodeURIComponent(docId)}?${masks}`, {
      method: 'PATCH',
      headers: {
        Authorization: `Bearer ${this.token}`,
        'Content-Type': 'application/json',
      },
      body: JSON.stringify(jsonToDoc(fields)),
    });
    if (!resp.ok) throw new Error(`Firestore update ${collection}/${docId}: ${await resp.text()}`);
  }

  async delete(collection, docId) {
    const resp = await fetch(`${this.root}/${collection}/${encodeURIComponent(docId)}`, {
      method: 'DELETE',
      headers: { Authorization: `Bearer ${this.token}` },
    });
    if (!resp.ok && resp.status !== 404) {
      throw new Error(`Firestore delete ${collection}/${docId}: ${await resp.text()}`);
    }
  }
}

function normalizeMint(nft) {
  return nft.mintAddress || nft.mint_address || '';
}

function groupKey(nft) {
  return nft.batchId || nft.batch_id || nft.masterNftId || nft.master_nft_id || `single:${nft.id}`;
}

function editionNumber(nft) {
  return nft.editionNumber ?? nft.edition_number ?? nft.batchIndex ?? nft.batch_index ?? null;
}

async function checkMints(nfts) {
  const networks = [
    ['mainnet', 'https://api.mainnet-beta.solana.com'],
    ['devnet', 'https://api.devnet.solana.com'],
  ];
  const withMint = nfts.filter(nft => normalizeMint(nft));
  const statuses = new Map(withMint.map(nft => [normalizeMint(nft), { valid: true, mainnet: false, devnet: false }]));
  const pubkeys = [];
  for (const nft of withMint) {
    try {
      pubkeys.push([normalizeMint(nft), new PublicKey(normalizeMint(nft))]);
    } catch {
      statuses.set(normalizeMint(nft), { valid: false, mainnet: false, devnet: false });
    }
  }

  for (const [network, rpc] of networks) {
    const conn = new Connection(rpc, 'confirmed');
    for (let i = 0; i < pubkeys.length; i += 100) {
      const chunk = pubkeys.slice(i, i + 100);
      const infos = await conn.getMultipleAccountsInfo(chunk.map(([, key]) => key));
      infos.forEach((info, idx) => {
        const mint = chunk[idx][0];
        const current = statuses.get(mint) || { valid: true, mainnet: false, devnet: false };
        current[network] = Boolean(info);
        statuses.set(mint, current);
      });
    }
  }
  return statuses;
}

function summarizeWallet(wallet, user, mintStatuses, options = {}) {
  const nfts = wallet.nfts || [];
  const groups = new Map();
  for (const nft of nfts) {
    const key = groupKey(nft);
    if (!groups.has(key)) groups.set(key, []);
    groups.get(key).push(nft);
  }

  const mintCounts = new Map();
  for (const nft of nfts) {
    const mint = normalizeMint(nft);
    if (!mint) continue;
    mintCounts.set(mint, (mintCounts.get(mint) || 0) + 1);
  }

  console.log('\n============================================================');
  console.log(`Wallet ${wallet.__docId} · ${user?.name || user?.companyName || user?.email || 'unknown user'}`);
  console.log(`NFT count: ${nfts.length}`);
  console.log(`Groups: ${groups.size}`);
  console.log('Groups by batch/master:');
  [...groups.entries()]
    .sort((a, b) => b[1].length - a[1].length)
    .forEach(([key, items]) => {
      const minted = items.filter(nft => {
        const status = mintStatuses.get(normalizeMint(nft));
        return status?.mainnet || status?.devnet;
      }).length;
      const noMint = items.filter(nft => !normalizeMint(nft)).length;
      const duplicateMints = items.filter(nft => mintCounts.get(normalizeMint(nft)) > 1).length;
      console.log(`- ${key}: ${items.length} items, minted=${minted}, noMint=${noMint}, duplicateMintRefs=${duplicateMints}`);
    });

  if (options.summary) return { groups, mintCounts };

  console.log('\nNFT rows:');
  nfts
    .slice()
    .sort((a, b) => String(groupKey(a)).localeCompare(String(groupKey(b))) || (editionNumber(a) ?? 9999) - (editionNumber(b) ?? 9999))
    .forEach((nft, idx) => {
      const mint = normalizeMint(nft);
      const status = mint ? mintStatuses.get(mint) : null;
      const where = !mint ? 'no-mint' : !status?.valid ? 'bad-mint' : status.mainnet ? 'mainnet' : status.devnet ? 'devnet' : 'missing';
      const dup = mint && mintCounts.get(mint) > 1 ? ' DUP-MINT' : '';
      console.log(
        `${String(idx + 1).padStart(2, '0')}. ${nft.title || 'Untitled'} | id=${nft.id} | group=${groupKey(nft)} | edition=${editionNumber(nft) ?? '-'} | mint=${mint || '-'} | ${where}${dup}`,
      );
    });

  return { groups, mintCounts };
}

function pickKeepSet(wallet, args) {
  const nfts = wallet.nfts || [];
  const keep = new Set(args.keepIds || []);
  if (args.keepBatch) {
    for (const nft of nfts) {
      if (groupKey(nft) === args.keepBatch || nft.batchId === args.keepBatch || nft.batch_id === args.keepBatch) {
        keep.add(nft.id);
      }
    }
  }
  return keep;
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  if (args.help) {
    console.log('Usage: node scripts/audit-wallet-nfts.mjs [--uid UID] [--keep-batch BATCH] [--keep-id ID ...] [--apply] [--delete-public]');
    return;
  }

  const env = parseEnvFile(apiEnvPath);
  const serviceAccount = JSON.parse(env.FIREBASE_SERVICE_ACCOUNT_JSON);
  const token = await accessToken(serviceAccount);
  const db = new Firestore(env.FIREBASE_PROJECT_ID, token);
  const wallets = args.uid ? [await db.get('marki_wallets', args.uid)] : await db.query('marki_wallets');
  const users = new Map((await db.query('users')).map(user => [user.uid || user.__docId, user]));
  const allNfts = wallets.filter(Boolean).flatMap(wallet => wallet.nfts || []);
  const mintStatuses = await checkMints(allNfts);

  for (const wallet of wallets.filter(Boolean)) {
    summarizeWallet(wallet, users.get(wallet.__docId), mintStatuses, { summary: args.summary });

    if (!args.apply) continue;
    const keep = pickKeepSet(wallet, args);
    if (keep.size === 0) {
      throw new Error('Refusing to apply cleanup without --keep-batch or --keep-id.');
    }
    const nfts = wallet.nfts || [];
    const kept = nfts.filter(nft => keep.has(nft.id));
    const removed = nfts.filter(nft => !keep.has(nft.id));
    if (kept.length === 0) throw new Error('Refusing to leave wallet with zero NFTs.');

    const backup = {
      walletId: wallet.__docId,
      keptCount: kept.length,
      removedCount: removed.length,
      kept,
      removed,
      createdAt: new Date().toISOString(),
    };
    const backupPath = path.join('/tmp', `marki-wallet-${wallet.__docId}-nft-cleanup-${Date.now()}.json`);
    fs.writeFileSync(backupPath, JSON.stringify(backup, null, 2));
    await db.update('marki_wallets', wallet.__docId, { nfts: kept });
    if (args.deletePublic) {
      for (const nft of removed) await db.delete('public_nfts', nft.id);
    }
    console.log(`\nAPPLIED cleanup for ${wallet.__docId}: kept=${kept.length}, removed=${removed.length}`);
    console.log(`Backup: ${backupPath}`);
  }
}

main().catch(err => {
  console.error(err?.stack || err?.message || err);
  process.exit(1);
});
