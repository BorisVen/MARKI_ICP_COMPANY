#!/usr/bin/env node
import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(__dirname, '..', '..');
const apiEnvPath = path.join(repoRoot, 'api', '.env');

const DEFAULT_UID = 'bUN9mGgABxXyLDiHsqtPP876rgY2';
const DEFAULT_BATCH_ID = 'da7b264c-f5a7-4948-920c-d6c97850f5a2';
const ODESSA = 'Склад Одеса';
const KYIV = 'Склад Київ';
const EMIL = 'Склад Еміль';
const NATALIA = 'Склад Наталья';

function parseArgs(argv) {
  const out = {};
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === '--uid') out.uid = argv[++i];
    else if (arg === '--batch') out.batchId = argv[++i];
    else if (arg === '--apply') out.apply = true;
    else if (arg === '--inspect') out.inspect = true;
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

  async set(collection, docId, fields) {
    if (!docId || docId === 'undefined') {
      throw new Error(`Refusing to write invalid document id for ${collection}: ${docId}`);
    }
    const cleanFields = { ...fields };
    delete cleanFields.__docId;
    const resp = await fetch(`${this.root}/${collection}/${encodeURIComponent(docId)}`, {
      method: 'PATCH',
      headers: {
        Authorization: `Bearer ${this.token}`,
        'Content-Type': 'application/json',
      },
      body: JSON.stringify(jsonToDoc(cleanFields)),
    });
    if (!resp.ok) throw new Error(`Firestore set ${collection}/${docId}: ${await resp.text()}`);
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

function editionNumber(nft) {
  return nft.editionNumber ?? nft.edition_number ?? nft.batchIndex ?? nft.batch_index ?? null;
}

function locationOf(checkpoint, fallback) {
  const value = String(checkpoint?.location || '').trim();
  return value && value !== '—' ? value : fallback;
}

function hasLocation(delivery, target) {
  return (delivery.checkpoints || []).some(checkpoint => locationOf(checkpoint, delivery.deliveryAddress) === target);
}

function latestLocation(delivery) {
  const checkpoints = [...(delivery.checkpoints || [])].sort((a, b) => String(a.timestamp).localeCompare(String(b.timestamp)));
  const latest = checkpoints.at(-1);
  return latest ? locationOf(latest, delivery.deliveryAddress) : delivery.deliveryAddress;
}

function checkpoint(timestamp, status, location, delivery, note) {
  return {
    id: crypto.randomUUID(),
    status,
    location,
    timestamp,
    recordedBy: delivery.sellerId,
    recordedByName: 'water',
    note,
  };
}

function isoPlus(base, seconds) {
  return new Date(base.getTime() + seconds * 1000).toISOString();
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  if (args.help) {
    console.log('Usage: node scripts/apply-water-route-plan.mjs [--apply] [--inspect] [--uid UID] [--batch BATCH_ID]');
    return;
  }

  const uid = args.uid || DEFAULT_UID;
  const batchId = args.batchId || DEFAULT_BATCH_ID;
  const env = parseEnvFile(apiEnvPath);
  const serviceAccount = JSON.parse(env.FIREBASE_SERVICE_ACCOUNT_JSON);
  const token = await accessToken(serviceAccount);
  const firestore = new Firestore(env.FIREBASE_PROJECT_ID || serviceAccount.project_id, token);

  const wallet = await firestore.get('marki_wallets', uid);
  if (!wallet) throw new Error(`Wallet not found: ${uid}`);

  const editions = (wallet.nfts || [])
    .filter(nft => (nft.batchId || nft.batch_id) === batchId)
    .filter(nft => editionNumber(nft) > 0)
    .sort((a, b) => editionNumber(a) - editionNumber(b));

  if (editions.length !== 12) {
    throw new Error(`Expected 12 edition NFTs, found ${editions.length}`);
  }

  const deliveries = await firestore.query('deliveries');
  const validDeliveries = deliveries.filter(delivery => delivery.__docId && delivery.__docId !== 'undefined');
  const latestDeliveryByNft = new Map();
  for (const nft of editions) {
    const rows = validDeliveries
      .filter(delivery => delivery.nftId === nft.id)
      .sort((a, b) => String(b.updatedAt).localeCompare(String(a.updatedAt)));
    if (!rows[0]) throw new Error(`Delivery not found for ${nft.title} · ${nft.id}`);
    latestDeliveryByNft.set(nft.id, rows[0]);
  }

  if (args.inspect) {
    for (const nft of editions.slice(0, 6)) {
      const rows = deliveries
        .filter(delivery => delivery.nftId === nft.id)
        .sort((a, b) => String(b.updatedAt).localeCompare(String(a.updatedAt)));
      console.log(`\n#${editionNumber(nft)} ${nft.title} ${nft.id}`);
      rows.forEach(delivery => {
        const locations = [...(delivery.checkpoints || [])]
          .sort((a, b) => String(a.timestamp).localeCompare(String(b.timestamp)))
          .map(checkpoint => `${checkpoint.status}:${locationOf(checkpoint, delivery.deliveryAddress)}@${checkpoint.timestamp}`)
          .join(' | ');
        console.log(`- doc=${delivery.__docId} fieldId=${delivery.id} status=${delivery.status} updated=${delivery.updatedAt} latest=${latestLocation(delivery)}`);
        console.log(`  ${locations}`);
      });
    }
    return;
  }

  const base = new Date();
  const changes = [];
  const initialCurrentByNft = new Map();
  const kyivIds = new Set(editions.slice(0, 6).map(nft => nft.id));
  const emilIds = new Set(editions.slice(0, 2).map(nft => nft.id));
  const nataliaIds = new Set(editions.slice(2, 4).map(nft => nft.id));

  editions.forEach((nft, index) => {
    const delivery = latestDeliveryByNft.get(nft.id);
    initialCurrentByNft.set(nft.id, latestLocation(delivery));
    const steps = [ODESSA];
    if (kyivIds.has(nft.id)) steps.push(KYIV);
    if (emilIds.has(nft.id)) steps.push(EMIL);
    if (nataliaIds.has(nft.id)) steps.push(NATALIA);

    for (const target of steps) {
      if (hasLocation(delivery, target)) continue;
      const from = latestLocation(delivery);
      const timestamp = isoPlus(base, changes.length * 20 + index);
      delivery.checkpoints = delivery.checkpoints || [];
      delivery.checkpoints.push(checkpoint(
        timestamp,
        target === ODESSA ? 'pending' : 'in_transit',
        target,
        delivery,
        `${from} → ${target}`,
      ));
      delivery.status = target === ODESSA ? delivery.status : 'in_transit';
      delivery.updatedAt = timestamp;
      changes.push({ nft, delivery, from, to: target, timestamp });
    }
  });

  console.log(`Plan for ${uid}, batch ${batchId}`);
  console.log(`Edition NFTs: ${editions.length}`);
  console.log(`Changes: ${changes.length}`);
  editions.forEach(nft => {
    const delivery = latestDeliveryByNft.get(nft.id);
    console.log(`- #${editionNumber(nft)} ${nft.title}: before=${initialCurrentByNft.get(nft.id)} -> planned=${latestLocation(delivery)}`);
  });
  if (changes.length) {
    console.log('\nTo append:');
    changes.forEach(change => {
      console.log(`- #${editionNumber(change.nft)} ${change.nft.title}: ${change.from} -> ${change.to} @ ${change.timestamp}`);
    });
  }

  if (!args.apply) {
    console.log('\nDry-run only. Re-run with --apply to write Firestore.');
    return;
  }

  if (deliveries.some(delivery => delivery.__docId === 'undefined')) {
    await firestore.delete('deliveries', 'undefined');
    console.log('Deleted stray deliveries/undefined document.');
  }

  for (const { delivery } of changes) {
    await firestore.set('deliveries', delivery.__docId, delivery);
  }
  console.log(`\nApplied ${changes.length} checkpoint changes.`);
}

main().catch(err => {
  console.error(err);
  process.exit(1);
});
