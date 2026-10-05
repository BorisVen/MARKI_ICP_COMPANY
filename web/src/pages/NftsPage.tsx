import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import QRCode from 'qrcode';
import {
  apiAiGenerateImage,
  apiBatchCreateNFTs,
  apiCreateEditionNFTs,
  apiCreateNFT,
  apiCreatePost,
  apiDeleteNFT,
  apiGenerateMissingQr,
  apiGenerateNFTQr,
  apiRegenerateAllQr,
  apiGetMintInfo,
  apiGetNFTs,
  apiUpdateNFT,
  type NFT,
} from '../api';
import { Icon } from '../icons';
import { useWallet } from '../icp/useWallet';
import EmptyState from '../EmptyState';
import { fmtPrice, plural } from '../format';
import { useSubscription } from '../icp/useSubscription';
import { friendlyIcpError } from '../icp/errors';
import WalletConnect from '../icp/WalletConnect';
import {
  cyclesToUsd,
  ICP_MINT_ENABLED,
  icpMintAddress,
  mergeMints,
  mintPhotoOnChain,
  parseIcpMintAddress,
  type OnChainMint,
} from '../icp/mint';

const CATEGORIES = ['Art', 'Music', 'Photography', 'Gaming', '3D', 'Collectible', 'Sports', 'Meme'];
const CURRENCIES = ['ICP', 'UAH', 'USD', 'USDC'];
const BLOCKCHAINS = [{ id: 'icp', name: 'Internet Computer', icon: '∞', currency: 'ICP', fee: '$0' }];

type Mode = 'list' | 'create' | 'batch';

export const OPEN_CREATE_KEY = 'markicp.nfts.openCreate';
type Step = 1 | 2 | 3;

function compressImage(file: File, maxPx = 1080, quality = 0.8): Promise<File> {
  return new Promise(resolve => {
    const url = URL.createObjectURL(file);
    const img = new Image();
    img.onload = () => {
      URL.revokeObjectURL(url);
      const scale = Math.min(1, maxPx / Math.max(img.width, img.height));
      const canvas = document.createElement('canvas');
      canvas.width = Math.round(img.width * scale);
      canvas.height = Math.round(img.height * scale);
      canvas.getContext('2d')!.drawImage(img, 0, 0, canvas.width, canvas.height);
      canvas.toBlob(
        blob => {
          if (!blob) { resolve(file); return; }
          const name = file.name.replace(/\.[^.]+$/, '.jpg');
          resolve(new File([blob], name, { type: 'image/jpeg' }));
        },
        'image/jpeg',
        quality,
      );
    };
    img.onerror = () => { URL.revokeObjectURL(url); resolve(file); };
    img.src = url;
  });
}

export default function NftsPage() {
  // The dashboard «Створити NFT» shortcut opens the wizard directly.
  const [mode, setMode] = useState<Mode>(() => {
    try {
      if (sessionStorage.getItem(OPEN_CREATE_KEY)) {
        sessionStorage.removeItem(OPEN_CREATE_KEY);
        return 'create';
      }
    } catch { /* noop */ }
    return 'list';
  });
  const [nfts, setNfts] = useState<NFT[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [editing, setEditing] = useState<NFT | null>(null);
  const [qrFor, setQrFor] = useState<NFT | null>(null);
  const [generatingQr, setGeneratingQr] = useState(false);

  const reload = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      setNfts(await apiGetNFTs());
    } catch (e: any) {
      setError(e?.message ?? 'Не вдалося завантажити каталог');
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => { reload(); }, [reload]);

  const onDelete = async (id: string) => {
    if (!confirm('Видалити цей NFT? Відновити його не вийде.')) return;
    try {
      await apiDeleteNFT(id);
      await reload();
    } catch (e: any) {
      alert(e?.message ?? 'Не вдалося видалити');
    }
  };

  const onGenerateMissingQr = async () => {
    if (!confirm('Згенерувати QR-коди для всіх NFT, у яких їх ще немає?')) return;
    setGeneratingQr(true);
    try {
      const result = await apiGenerateMissingQr();
      alert(result.generated ? `Готово: створено ${result.generated} QR.` : 'У всіх товарів уже є QR-коди.');
      await reload();
    } catch (e: any) {
      alert(e?.message ?? 'Не вдалося створити QR');
    } finally {
      setGeneratingQr(false);
    }
  };

  const onRegenerateAllQr = async () => {
    if (!confirm('ПЕРЕгенерувати ВСІ QR-коди заново?\nЦе замінить наявні QR на нові з правильним URL.')) return;
    setGeneratingQr(true);
    try {
      const result = await apiRegenerateAllQr();
      alert(`Готово: перестворено ${result.regenerated} QR.`);
      await reload();
    } catch (e: any) {
      alert(e?.message ?? 'Не вдалося перестворити QR');
    } finally {
      setGeneratingQr(false);
    }
  };

  return (
    <div>
      <div className="page-head">
        <div>
          <h2>NFT</h2>
          <p>{nfts.length > 0 ? `${nfts.length} ${plural(nfts.length, 'товар', 'товари', 'товарів')} у каталозі` : 'Цифрові паспорти твоїх товарів.'}</p>
        </div>
        {mode === 'list' && (
          <div style={{ display: 'flex', gap: 8, flexWrap: 'wrap' }}>
            <button className="btn" onClick={reload} title="Оновити"><Icon.Refresh /></button>
            {nfts.length > 0 && (
              <button className="btn" onClick={onGenerateMissingQr} disabled={generatingQr} title="Створити QR-коди для товарів, у яких їх ще немає">
                <Icon.QrCode /> {generatingQr ? 'Створюємо…' : 'QR для всіх'}
              </button>
            )}
            <button className="btn btn-primary" onClick={() => setMode('create')}>
              <Icon.Plus /> Створити NFT
            </button>
          </div>
        )}
      </div>

      <div className="mode-tabs" role="tablist">
        <button className={`mode-tab ${mode === 'list' ? 'active' : ''}`} onClick={() => setMode('list')}>
          Каталог
        </button>
        <button className={`mode-tab ${mode === 'create' ? 'active' : ''}`} onClick={() => setMode('create')}>
          Новий NFT
        </button>
        <button className={`mode-tab ${mode === 'batch' ? 'active' : ''}`} onClick={() => setMode('batch')}>
          Масовий випуск
        </button>
      </div>

      {mode === 'create' && (
        <CreateNftWizard onDone={async () => { await reload(); setMode('list'); }} />
      )}

      {mode === 'batch' && (
        <BatchNftForm onDone={async () => { await reload(); setMode('list'); }} />
      )}

      {mode === 'list' && (
        <>
          {error && <div className="error-banner">{error}</div>}
          {loading ? <div className="spinner">Завантаження…</div> : (
            nfts.length === 0 ? (
              <EmptyState
                icon="🏷️"
                title="Каталог порожній"
                text="Створи перший NFT: завантаж фото товару — і отримай цифровий паспорт із QR-кодом."
                action={<button className="btn btn-primary" onClick={() => setMode('create')}><Icon.Plus /> Створити NFT</button>}
              />
            ) : (
              <div className="grid grid-3">
                {nfts.map(n => (
                  <NftCard
                    key={n.id}
                    nft={n}
                    onEdit={() => setEditing(n)}
                    onDelete={() => onDelete(n.id)}
                    onShowQr={() => setQrFor(n)}
                  />
                ))}
              </div>
            )
          )}
          {nfts.length > 0 && (
            <div className="muted" style={{ fontSize: 12, marginTop: 16, textAlign: 'center' }}>
              Змінив адресу сайту?{' '}
              <button className="btn" style={{ padding: '2px 8px', fontSize: 12 }} onClick={onRegenerateAllQr} disabled={generatingQr}>
                Перестворити всі QR
              </button>
            </div>
          )}
        </>
      )}

      {editing && (
        <EditNftModal
          nft={editing}
          onClose={() => setEditing(null)}
          onSaved={async () => { setEditing(null); await reload(); }}
        />
      )}

      {qrFor && <QrModal nft={qrFor} onClose={() => setQrFor(null)} />}
    </div>
  );
}

function NftCard({ nft, onEdit, onDelete, onShowQr }: {
  nft: NFT;
  onEdit: () => void;
  onDelete: () => void;
  onShowQr: () => void;
}) {
  return (
    <div className="card">
      {nft.imageUrl || nft.image ? (
        <img
          src={nft.imageUrl || nft.image}
          alt={nft.title}
          style={{ width: '100%', aspectRatio: '1 / 1', objectFit: 'cover', borderRadius: 10, marginBottom: 12 }}
        />
      ) : (
        <div
          style={{
            aspectRatio: '1 / 1',
            background: 'var(--bg-soft)',
            borderRadius: 10,
            marginBottom: 12,
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
            color: 'var(--muted)',
            fontSize: 12,
          }}
        >
          No image
        </div>
      )}
      <h3>{nft.title}</h3>
      {nft.description && (
        <p className="sub" style={{ fontSize: 12 }}>
          {nft.description.slice(0, 100)}{nft.description.length > 100 ? '…' : ''}
        </p>
      )}
      <div style={{ display: 'flex', gap: 8, marginTop: 10, flexWrap: 'wrap' }}>
        {nft.forSale && <span className="badge badge-success">у продажу · {fmtPrice(nft.price, nft.currency)}</span>}
        {nft.nfcUid && <span className="badge badge-info">NFC</span>}
        {nft.tokenId && <span className="badge badge-muted">у блокчейні</span>}
        {parseIcpMintAddress(nft.mintAddress) && (
          <span className="badge badge-info" title={nft.mintAddress}>ICP #{parseIcpMintAddress(nft.mintAddress)!.tokenId.toString()}</span>
        )}
        {nft.category && <span className="badge badge-muted">{nft.category}</span>}
      </div>
      <div className="card-actions">
        <button className="btn" onClick={onEdit}>Змінити</button>
        <button className="btn" onClick={onShowQr} title="QR-код для упаковки товару">
          <Icon.QrCode /> QR-код
        </button>
        <span className="spacer" />
        <button className="btn ghost-danger" onClick={onDelete} title="Видалити NFT"><Icon.Trash /></button>
      </div>
    </div>
  );
}

// ────────────────────────────────────────────────────────────────────────────
// CREATE WIZARD
// ────────────────────────────────────────────────────────────────────────────

function CreateNftWizard({ onDone }: { onDone: () => Promise<void> }) {
  const wallet = useWallet();
  const walletReady = !!wallet;
  const principal = wallet?.principal ?? '';
  const [chainMint, setChainMint] = useState<OnChainMint | null>(null);
  const plan = useSubscription();
  const fileInputRef = useRef<HTMLInputElement>(null);
  const collectionInputRef = useRef<HTMLInputElement>(null);

  const [step, setStep] = useState<Step>(1);
  const [loading, setLoading] = useState(false);
  const [uploadProgress, setUploadProgress] = useState('');
  const [success, setSuccess] = useState(false);
  const [dragOver, setDragOver] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  const [selectedFile, setSelectedFile] = useState<File | null>(null);
  const [previewUrl, setPreviewUrl] = useState('');
  const [title, setTitle] = useState('');
  const [description, setDescription] = useState('');
  const [tags, setTags] = useState<string[]>([]);
  const [tagInput, setTagInput] = useState('');
  const [category, setCategory] = useState('Art');
  const [forSale, setForSale] = useState(false);
  const [price, setPrice] = useState('');
  const [currency, setCurrency] = useState('ICP');
  const [blockchain, setBlockchain] = useState('icp');
  const [royalty, setRoyalty] = useState('10');
  const [editionCount, setEditionCount] = useState('1');

  const [isCollection, setIsCollection] = useState(false);
  const [collectionName, setCollectionName] = useState('');
  const [collectionFiles, setCollectionFiles] = useState<File[]>([]);

  const [aiPrompt, setAiPrompt] = useState('');
  const [aiGenerating, setAiGenerating] = useState(false);
  const [aiError, setAiError] = useState('');
  const [showAiPanel, setShowAiPanel] = useState(false);

  const handleAiGenerate = async () => {
    const prompt = aiPrompt.trim();
    if (!prompt) return;
    setAiGenerating(true);
    setAiError('');
    try {
      const blob = await apiAiGenerateImage(prompt);
      const seed = Math.floor(Math.random() * 1_000_000);
      const file = new File([blob], `ai-nft-${seed}.jpg`, { type: 'image/jpeg' });
      setSelectedFile(file);
      setPreviewUrl(URL.createObjectURL(file));
      setShowAiPanel(false);
    } catch (e: any) {
      setAiError(e?.message ?? 'Generation failed');
    } finally {
      setAiGenerating(false);
    }
  };

  const processFile = (file: File) => {
    if (!file.type.startsWith('image/')) { alert('Завантаж зображення'); return; }
    if (file.size > 10 * 1024 * 1024) { alert('Файл завеликий (макс 10MB).'); return; }
    setSelectedFile(file);
    setPreviewUrl(URL.createObjectURL(file));
  };

  const handleDrop = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    setDragOver(false);
    const file = e.dataTransfer.files?.[0];
    if (file) processFile(file);
  }, []);

  const addTag = () => {
    const t = tagInput.trim().replace(/^#/, '');
    if (t && !tags.includes(t) && tags.length < 8) {
      setTags([...tags, t]);
      setTagInput('');
    }
  };

  const canGoNext = () => {
    if (step === 1) {
      const hasFile = isCollection ? collectionFiles.length > 0 : !!selectedFile;
      return hasFile && !!title.trim() && !!description.trim();
    }
    if (step === 2) return !!blockchain;
    return true;
  };

  const handleSubmit = async () => {
    if (!wallet) {
      alert('Підключи гаманець (Plug або Dev), щоб мінтити NFT у ICP.');
      return;
    }
    if (isCollection && collectionFiles.length === 0) {
      alert('Вибери файли для колекції');
      return;
    }
    if (!isCollection && !selectedFile) {
      alert('Вибери зображення');
      return;
    }

    setLoading(true);
    setErr(null);
    setUploadProgress('');

    try {
      const baseMetadata: any = {
        title: title.trim(),
        description: description.trim(),
        tags,
        category,
        blockchain,
        royalty: parseFloat(royalty),
        forSale,
        currency,
        creatorPrincipal: principal,
      };
      if (forSale && price) baseMetadata.price = parseFloat(price);

      if (isCollection) {
        const totalItems = collectionFiles.length;
        setUploadProgress(`Крок 1/3 — Стиснення ${totalItems} файлів…`);
        const items = collectionFiles.map((f, i) => ({
          title: f.name.replace(/\.[^.]+$/, '') || `${collectionName.trim() || 'Collection'} #${i + 1}`,
          description: description.trim(),
          ...(forSale && price ? { price: parseFloat(price) } : {}),
        }));
        const compressedFiles = await Promise.all(collectionFiles.map(f => compressImage(f)));

        // On-chain first: the canister takes plan credits, so no plan = no NFT.
        const mints: OnChainMint[] = [];
        if (ICP_MINT_ENABLED) {
          for (let i = 0; i < compressedFiles.length; i++) {
            mints.push(await mintPhotoOnChain(wallet, compressedFiles[i], { ...baseMetadata, ...items[i] }, 1,
              t => setUploadProgress(`ICP ${i + 1}/${totalItems} — ${t}`)));
          }
          setChainMint(mergeMints(mints));
        }

        const form = new FormData();
        compressedFiles.forEach(f => form.append('images[]', f));
        form.append('metadata', JSON.stringify({
          ...baseMetadata,
          batchName: collectionName.trim() || title.trim() || 'Collection',
          items,
        }));

        setUploadProgress(`Крок 2/3 — Завантаження ${totalItems} зображень і мінт у ICP…`);
        const colRes: any = await apiBatchCreateNFTs(form);
        const successful: any[] = (colRes?.results ?? []).filter((r: any) => r.status === 'ok' && r.id);
        if (successful.length === 0) throw new Error('Бекенд не прийняв жодного файлу.');

        for (const r of successful) {
          const tokenId = mints[r.index]?.tokenIds[0];
          if (tokenId !== undefined) await apiUpdateNFT(r.id, { mintAddress: icpMintAddress(tokenId) });
        }

        const collectionImageUrls: string[] = successful.map((r: any) => r.imageUrl);
        const collectionNftIds: string[] = successful.map((r: any) => r.id);

        setUploadProgress('Крок 3/3 — Публікація у стрічку…');
        await apiCreatePost({
          nftImages: collectionImageUrls,
          walletNftIds: collectionNftIds,
          title: collectionName.trim() || title.trim(),
          description: description.trim(),
          tags,
          forSale,
          price: forSale && price ? parseFloat(price) : null,
          currency,
          blockchain,
        });
      } else {
        const numEditions = Math.max(1, parseInt(editionCount) || 1);

        if (numEditions > 1) {
          setUploadProgress('Крок 1/3 — Стиснення та завантаження зображення…');
          const editionImage = await compressImage(selectedFile!);
          const edMint = ICP_MINT_ENABLED
            ? await mintPhotoOnChain(wallet, editionImage, baseMetadata, numEditions, setUploadProgress)
            : null;
          setChainMint(edMint);
          const editionForm = new FormData();
          editionForm.append('image', editionImage);
          editionForm.append('metadata', JSON.stringify({
            ...baseMetadata,
            batchName: collectionName.trim() || title.trim(),
            editionCount: numEditions,
          }));

          setUploadProgress('Крок 2/3 — Мінт editions у ICP (reverse gas)…');
          const edResult: any = await apiCreateEditionNFTs(editionForm);
          const editionIds: string[] = edResult?.editionIds ?? [];
          const imageUrl: string = edResult?.imageUrl ?? '';
          if (edMint) {
            for (let i = 0; i < editionIds.length && i < edMint.tokenIds.length; i++) {
              await apiUpdateNFT(editionIds[i], { mintAddress: icpMintAddress(edMint.tokenIds[i]) });
            }
            if (edResult?.masterId) await apiUpdateNFT(edResult.masterId, { mintAddress: icpMintAddress(edMint.tokenIds[0]) });
          }

          setUploadProgress('Крок 3/3 — Публікація у стрічку…');
          await apiCreatePost({
            nftImages: editionIds.map(() => imageUrl),
            walletNftIds: editionIds,
            title: `${title.trim()} (${editionIds.length}/${numEditions} editions)`,
            description: description.trim(),
            tags,
            forSale,
            price: forSale && price ? parseFloat(price) : null,
            currency,
            blockchain,
          });
        } else {
          setUploadProgress('Крок 1/3 — Стиснення та завантаження…');
          const image = await compressImage(selectedFile!);
          const oneMint = ICP_MINT_ENABLED
            ? await mintPhotoOnChain(wallet, image, baseMetadata, 1, setUploadProgress)
            : null;
          setChainMint(oneMint);
          const form = new FormData();
          form.append('image', image);
          form.append('metadata', JSON.stringify(baseMetadata));

          setUploadProgress('Крок 2/3 — Мінт у ICP (reverse gas)…');
          const result: any = await apiCreateNFT(form);
          const nftId = result?.id;
          if (!nftId) throw new Error('Бекенд не повернув id');
          if (oneMint) await apiUpdateNFT(nftId, { mintAddress: icpMintAddress(oneMint.tokenIds[0]) });

          setUploadProgress('Крок 3/3 — Публікація у стрічку…');
          await apiCreatePost({
            nftImage: result.image,
            title: title.trim(),
            description: description.trim(),
            tags,
            forSale,
            price: forSale && price ? parseFloat(price) : null,
            currency,
            blockchain,
            walletNftId: nftId,
          });
        }
      }

      if (ICP_MINT_ENABLED) plan.refresh();
      setSuccess(true);
      setUploadProgress('');
    } catch (e: any) {
      console.error('[NFTs] mint failed:', e);
      setErr(friendlyIcpError(e?.message ?? 'Не вдалося випустити NFT'));
    } finally {
      setLoading(false);
    }
  };

  const handleReset = () => {
    setStep(1);
    setSelectedFile(null);
    setPreviewUrl('');
    setTitle('');
    setDescription('');
    setTags([]);
    setCategory('Art');
    setForSale(false);
    setPrice('');
    setCurrency('ICP');
    setBlockchain('icp');
    setRoyalty('10');
    setEditionCount('1');
    setIsCollection(false);
    setCollectionName('');
    setCollectionFiles([]);
    setSuccess(false);
    setErr(null);
    setChainMint(null);
  };

  if (success) {
    return (
      <div className="success-box">
        <div className="circle">✨</div>
        <h2 style={{ margin: '0 0 8px' }}>{isCollection ? 'Колекцію випущено!' : 'NFT випущено!'}</h2>
        <p className="sub" style={{ margin: '0 auto 14px', maxWidth: 440 }}>
          «{isCollection ? collectionName.trim() || title.trim() : title.trim()}» додано в гаманець
          {forSale && price ? ` і виставлено за ${price} ${currency}.` : '.'}
        </p>
        {previewUrl && <img src={previewUrl} alt={title} className="img-preview" />}
        {chainMint && <ChainMintSummary mint={chainMint} />}
        <div style={{ display: 'flex', gap: 10, justifyContent: 'center', marginTop: 16 }}>
          <button className="btn btn-primary" onClick={handleReset}>
            <Icon.Plus /> Створити ще
          </button>
          <button className="btn" onClick={onDone}>До списку</button>
        </div>
      </div>
    );
  }

  return (
    <div className="create-grid">
      <div>
        <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 18, flexWrap: 'wrap', gap: 12 }}>
          <div className="stepper">
            <div className={`dot ${step === 1 ? 'active' : step > 1 ? 'done' : ''}`}>{step > 1 ? '✓' : 1}</div>
            <div className={`line ${step > 1 ? 'done' : ''}`} />
            <div className={`dot ${step === 2 ? 'active' : step > 2 ? 'done' : ''}`}>{step > 2 ? '✓' : 2}</div>
            <div className={`line ${step > 2 ? 'done' : ''}`} />
            <div className={`dot ${step === 3 ? 'active' : ''}`}>3</div>
          </div>
          <WalletConnect onError={setErr} />
        </div>

        <div className="step-labels">
          <span className={step === 1 ? 'active' : ''}>Фото й опис</span>
          <span className={step === 2 ? 'active' : ''}>Параметри</span>
          <span className={step === 3 ? 'active' : ''}>Продаж</span>
        </div>

        {loading && uploadProgress && (
          <div className="progress-banner">
            <div className="mini-spin" />
            <div style={{ fontSize: 13 }}>{uploadProgress}</div>
          </div>
        )}

        {err && <div className="error-banner">{err}</div>}

        {step === 1 && (
          <div className="card">
            <h3>Фото й опис товару</h3>

            <div className="toggle-row">
              <div>
                <div className="lbl">Створити як колекцію</div>
                <div className="sub">Група пов’язаних NFT — один пост у стрічці</div>
              </div>
              <div className={`switch ${isCollection ? 'on' : ''}`} onClick={() => setIsCollection(!isCollection)}>
                <div className="knob" />
              </div>
            </div>

            {isCollection && (
              <div className="field">
                <label>Назва колекції</label>
                <input
                  value={collectionName}
                  maxLength={60}
                  placeholder="Cosmic Dreams Series"
                  onChange={e => setCollectionName(e.target.value)}
                />
                <div className="notice notice-warn" style={{ marginTop: 8 }}>
                  ⚠️ До продажів усередині колекції застосовується комісія платформи 1%.
                </div>
              </div>
            )}

            {isCollection ? (
              <div className="field">
                <label>Зображення колекції</label>
                <input
                  ref={collectionInputRef}
                  type="file"
                  accept="image/*"
                  multiple
                  style={{ display: 'none' }}
                  onChange={e => setCollectionFiles(Array.from(e.target.files || []))}
                />
                <div className={`drop-zone ${collectionFiles.length > 0 ? 'has-file' : ''}`}
                     onClick={() => collectionInputRef.current?.click()}>
                  {collectionFiles.length > 0 ? (
                    <div style={{ width: '100%', padding: 14 }}>
                      <div style={{ display: 'grid', gridTemplateColumns: 'repeat(4, 1fr)', gap: 8, marginBottom: 8 }}>
                        {collectionFiles.slice(0, 8).map((f, i) => (
                          <div key={i} style={{ aspectRatio: '1 / 1', background: 'var(--bg-soft)', borderRadius: 8, overflow: 'hidden' }}>
                            <img src={URL.createObjectURL(f)} alt="" style={{ width: '100%', height: '100%', objectFit: 'cover' }} />
                          </div>
                        ))}
                      </div>
                      <div className="sub" style={{ textAlign: 'center' }}>
                        {collectionFiles.length} файл(ів) — натисни, щоб перевибрати
                      </div>
                    </div>
                  ) : (
                    <div>
                      <div className="ph-title">+ Вибери зображення</div>
                      <div className="ph-sub">PNG, JPG, GIF, WebP — до 10 MB кожен</div>
                    </div>
                  )}
                </div>
              </div>
            ) : (
              <div className="field">
                <label>Зображення *</label>
                <input
                  ref={fileInputRef}
                  type="file"
                  accept="image/*"
                  style={{ display: 'none' }}
                  onChange={e => { const f = e.target.files?.[0]; if (f) processFile(f); }}
                />
                <div
                  className={`drop-zone ${dragOver ? 'over' : ''} ${selectedFile ? 'has-file' : ''}`}
                  onClick={() => fileInputRef.current?.click()}
                  onDragOver={e => { e.preventDefault(); setDragOver(true); }}
                  onDragLeave={() => setDragOver(false)}
                  onDrop={handleDrop}
                >
                  {selectedFile && previewUrl ? (
                    <img src={previewUrl} alt="preview" className="preview" />
                  ) : (
                    <div>
                      <div className="ph-title">Перетягни зображення сюди</div>
                      <div className="ph-sub">або клікни, щоб вибрати · PNG / JPG / GIF · до 10 MB</div>
                    </div>
                  )}
                </div>

                <div style={{ display: 'flex', gap: 8, marginTop: 10 }}>
                  <button type="button" className="btn" onClick={() => setShowAiPanel(v => !v)}>
                    ✨ Згенерувати AI
                  </button>
                  {selectedFile && (
                    <button type="button" className="btn" onClick={() => { setSelectedFile(null); setPreviewUrl(''); }}>
                      Прибрати
                    </button>
                  )}
                </div>

                {showAiPanel && (
                  <div style={{ background: 'var(--bg-soft)', border: '1px solid var(--border)', borderRadius: 12, padding: 14, marginTop: 10 }}>
                    <textarea
                      placeholder="Опиши NFT — напр. «cyberpunk astronaut neon city»"
                      value={aiPrompt}
                      onChange={e => setAiPrompt(e.target.value)}
                      style={{ width: '100%', background: 'var(--card)', border: '1px solid var(--border)', borderRadius: 8, padding: '10px 12px', color: 'var(--text)', fontSize: 13, minHeight: 60, resize: 'vertical' }}
                    />
                    {aiError && <div className="error-banner" style={{ marginTop: 8, marginBottom: 0 }}>{aiError}</div>}
                    <button
                      type="button"
                      className="btn btn-primary btn-block"
                      style={{ marginTop: 8 }}
                      disabled={!aiPrompt.trim() || aiGenerating}
                      onClick={handleAiGenerate}
                    >
                      {aiGenerating ? 'Генерація… (~15–30 сек)' : 'Згенерувати'}
                    </button>
                  </div>
                )}
              </div>
            )}

            <div className="field">
              <label>Назва *</label>
              <input value={title} maxLength={60} onChange={e => setTitle(e.target.value)} />
              <div className="char-count">{title.length}/60</div>
            </div>

            <div className="field">
              <label>Опис *</label>
              <textarea value={description} maxLength={300} onChange={e => setDescription(e.target.value)} />
              <div className="char-count">{description.length}/300</div>
            </div>

            <div className="field">
              <label>Категорія</label>
              <div className="chip-row">
                {CATEGORIES.map(c => (
                  <button
                    type="button"
                    key={c}
                    className={`chip ${category === c ? 'active' : ''}`}
                    onClick={() => setCategory(c)}
                  >{c}</button>
                ))}
              </div>
            </div>

            <div className="field">
              <label>Теги (до 8)</label>
              <div className="tag-input-row">
                <input
                  placeholder="#tag"
                  value={tagInput}
                  onChange={e => setTagInput(e.target.value)}
                  onKeyDown={e => { if (e.key === 'Enter' || e.key === ',') { e.preventDefault(); addTag(); } }}
                />
                <button type="button" className="btn" onClick={addTag}>Додати</button>
              </div>
              {tags.length > 0 && (
                <div style={{ marginTop: 6 }}>
                  {tags.map(t => (
                    <span key={t} className="tag-badge">
                      #{t}
                      <button type="button" className="x" onClick={() => setTags(tags.filter(x => x !== t))}>✕</button>
                    </span>
                  ))}
                </div>
              )}
            </div>

            <div className="step-actions">
              <span />
              <button
                type="button"
                className="btn btn-primary"
                disabled={!canGoNext()}
                onClick={() => setStep(2)}
              >Далі <Icon.ChevronRight /></button>
            </div>
          </div>
        )}

        {step === 2 && (
          <div className="card">
            <h3>Параметри</h3>

            {/* One chain today: show it as information instead of a one-option picker. */}
            <div className="notice notice-info" style={{ display: 'flex', gap: 10, alignItems: 'center' }}>
              <span style={{ fontSize: 18 }}>{BLOCKCHAINS[0].icon}</span>
              <span>NFT буде записано в блокчейн <strong>{BLOCKCHAINS[0].name}</strong>. Газ і зберігання оплачує Marki.</span>
            </div>

            <div className="field">
              <label>Відсоток з перепродажу (роялті): <strong style={{ color: 'var(--primary)' }}>{royalty}%</strong></label>
              <input
                type="range"
                min="0"
                max="30"
                step="1"
                value={royalty}
                onChange={e => setRoyalty(e.target.value)}
                className="slider"
              />
              <div className="sub" style={{ fontSize: 11 }}>
                Стільки відсотків ти отримуєш щоразу, коли покупець перепродає цей NFT.
              </div>
            </div>

            {!isCollection && (
              <div className="field">
                <label>Кількість копій</label>
                <input
                  type="number"
                  min="1"
                  max="1000"
                  value={editionCount}
                  onChange={e => setEditionCount(e.target.value)}
                />
                <div className="sub" style={{ fontSize: 11 }}>
                  1 — унікальний товар. Більше — серія однакових NFT (наприклад, партія з 100 пляшок), кожна копія спише 1 NFT з тарифу.
                </div>
              </div>
            )}

            <div className="step-actions">
              <button type="button" className="btn" onClick={() => setStep(1)}>← Назад</button>
              <button type="button" className="btn btn-primary" onClick={() => setStep(3)}>
                Далі <Icon.ChevronRight />
              </button>
            </div>
          </div>
        )}

        {step === 3 && (
          <div className="card">
            <h3>Виставити на продаж</h3>

            <div className="toggle-row">
              <div>
                <div className="lbl">Одразу лістити на маркетплейс</div>
                <div className="sub">Можна увімкнути пізніше, у редагуванні.</div>
              </div>
              <div className={`switch ${forSale ? 'on' : ''}`} onClick={() => setForSale(!forSale)}>
                <div className="knob" />
              </div>
            </div>

            {forSale && (
              <>
                <div className="field">
                  <label>Валюта</label>
                  <div className="chip-row">
                    {CURRENCIES.map(c => (
                      <button
                        type="button"
                        key={c}
                        className={`chip ${currency === c ? 'active' : ''}`}
                        onClick={() => setCurrency(c)}
                      >{c}</button>
                    ))}
                  </div>
                </div>
                <div className="field">
                  <label>Цена ({currency})</label>
                  <input
                    type="number"
                    min="0"
                    step="0.001"
                    placeholder="0.5"
                    value={price}
                    onChange={e => setPrice(e.target.value)}
                  />
                  {price && parseFloat(price) > 0 && (
                    <div className="sub" style={{ fontSize: 11 }}>
                      Ти отримаєш <strong style={{ color: 'var(--primary)' }}>{parseFloat(price)} {currency}</strong>.
                      Покупець доплачує 1% платформний збір зверху.
                    </div>
                  )}
                </div>
              </>
            )}

            {!walletReady && (
              <div className="notice notice-warn">
                Підключи гаманець кнопкою вгорі — NFT буде записано на нього.
              </div>
            )}

            {walletReady && ICP_MINT_ENABLED && (() => {
              const need = isCollection ? collectionFiles.length : Math.max(1, parseInt(editionCount) || 1);
              const left = plan.active ? Number(plan.subscription!.photo_mints_left) : 0;
              if (!plan.active) {
                return <div className="notice notice-warn">Немає активного тарифу. Обери тариф у розділі «Тарифи», щоб випустити NFT.</div>;
              }
              if (left < need) {
                return <div className="notice notice-warn">Потрібно {need} NFT з фото, а в тарифі залишилось {left}. Докупи тариф у розділі «Тарифи».</div>;
              }
              return (
                <div className="notice notice-info">
                  З тарифу «{plan.plan?.name ?? plan.subscription!.plan_id}» буде використано {need} NFT з фото — залишиться {left - need}.
                  Газ і зберігання в блокчейні оплачує Marki.
                </div>
              );
            })()}

            <div className="step-actions">
              <button type="button" className="btn" onClick={() => setStep(2)}>← Назад</button>
              <button
                type="button"
                className="btn btn-primary"
                disabled={loading || !walletReady}
                onClick={handleSubmit}
              >
                {loading ? 'Випускаємо…' : isCollection ? `Випустити колекцію (${collectionFiles.length})` : 'Випустити NFT'}
              </button>
            </div>
          </div>
        )}
      </div>

      <CreatePreview
        title={title}
        description={description}
        category={category}
        tags={tags}
        previewUrl={previewUrl}
        collectionFiles={collectionFiles}
        isCollection={isCollection}
        collectionName={collectionName}
        forSale={forSale}
        price={price}
        currency={currency}
        royalty={royalty}
        editionCount={editionCount}
      />
    </div>
  );
}

function CreatePreview(props: {
  title: string;
  description: string;
  category: string;
  tags: string[];
  previewUrl: string;
  collectionFiles: File[];
  isCollection: boolean;
  collectionName: string;
  forSale: boolean;
  price: string;
  currency: string;
  royalty: string;
  editionCount: string;
}) {
  const {
    title, description, category, tags, previewUrl, collectionFiles,
    isCollection, collectionName, forSale, price, currency, royalty, editionCount,
  } = props;

  const firstFile = collectionFiles[0];
  const firstUrl = useMemo(
    () => firstFile ? URL.createObjectURL(firstFile) : '',
    [firstFile]
  );

  const showImg = isCollection ? firstUrl : previewUrl;
  const showTitle = isCollection ? (collectionName || title || 'Без назви') : (title || 'Без назви');

  return (
    <div className="card" style={{ position: 'sticky', top: 80 }}>
      <div className="sub" style={{ marginBottom: 8 }}>Попередній перегляд</div>
      {showImg ? (
        <img
          src={showImg}
          alt={showTitle}
          style={{ width: '100%', aspectRatio: '1 / 1', objectFit: 'cover', borderRadius: 10, marginBottom: 12 }}
        />
      ) : (
        <div style={{
          aspectRatio: '1 / 1', background: 'var(--bg-soft)', borderRadius: 10, marginBottom: 12,
          display: 'flex', alignItems: 'center', justifyContent: 'center', color: 'var(--muted)', fontSize: 12,
        }}>
          Без зображення
        </div>
      )}
      <h3 style={{ margin: '0 0 6px' }}>{showTitle}</h3>
      {description && <p className="sub" style={{ fontSize: 12, margin: '0 0 8px' }}>{description.slice(0, 140)}{description.length > 140 ? '…' : ''}</p>}
      <div style={{ display: 'flex', gap: 6, flexWrap: 'wrap', marginBottom: 8 }}>
        {!isCollection && <span className="badge badge-muted">{category}</span>}
        {isCollection && <span className="badge badge-info">collection · {collectionFiles.length}</span>}
        {parseInt(editionCount) > 1 && !isCollection && <span className="badge badge-info">{editionCount} копій</span>}
        <span className="badge badge-muted">{royalty}% royalty</span>
        {forSale && price && <span className="badge badge-success">{price} {currency}</span>}
      </div>
      {tags.length > 0 && (
        <div>
          {tags.map(t => <span key={t} className="tag-badge" style={{ background: 'var(--bg-soft)', color: 'var(--muted)' }}>#{t}</span>)}
        </div>
      )}
    </div>
  );
}

function ChainMintSummary({ mint }: { mint: OnChainMint }) {
  const sub = mint.subscription;
  const ids = mint.tokenIds.map(String);
  const usd = cyclesToUsd(mint.cyclesSpent);
  const tokens = ids.length === 1 ? `№ ${ids[0]}` : ids.length > 6 ? `№ ${ids[0]}–${ids[ids.length - 1]}` : ids.map(i => `№ ${i}`).join(', ');
  return (
    <div className="card mint-done">
      <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: 6, gap: 8 }}>
        <span className="chain-badge">✓ Записано в блокчейн</span>
        <span className="muted" style={{ fontSize: 12 }}>Internet Computer</span>
      </div>
      <div className="md-row">
        <span className="muted">{ids.length === 1 ? 'Номер NFT' : `Номери NFT (${ids.length})`}</span>
        <strong>{tokens}</strong>
      </div>
      <div className="md-row">
        <span className="muted">Вартість запису</span>
        <span title={`${mint.cyclesSpent.toString()} циклів`}>
          <strong>{usd < 0.01 ? 'менше 1 цента' : `$${usd.toFixed(2)}`}</strong>
          <span className="muted"> · оплачує Marki</span>
        </span>
      </div>
      <div className="md-row">
        <span className="muted">Залишилось у тарифі</span>
        <span><strong>{sub.photo_mints_left.toString()}</strong> NFT з фото</span>
      </div>
      <details style={{ marginTop: 8 }}>
        <summary className="muted" style={{ fontSize: 12, cursor: 'pointer' }}>Технічні деталі</summary>
        <div className="muted" style={{ fontSize: 12, lineHeight: 1.7, marginTop: 6 }}>
          Фото в канiстрі: {(mint.imageBytes / 1024).toFixed(1)} KB<br />
          Списано циклів: {(Number(mint.cyclesSpent) / 1e6).toFixed(1)}M (≈ ${usd.toLocaleString('en-US', { maximumSignificantDigits: 3 })})<br />
          Мінтів метаданих у тарифі: {sub.metadata_mints_left.toString()}
        </div>
      </details>
    </div>
  );
}

// ────────────────────────────────────────────────────────────────────────────
// BATCH MODE
// ────────────────────────────────────────────────────────────────────────────

function BatchNftForm({ onDone }: { onDone: () => Promise<void> }) {
  const fileRef = useRef<HTMLInputElement>(null);
  const [files, setFiles] = useState<File[]>([]);
  const [blockchain, setBlockchain] = useState('icp');
  const [currency, setCurrency] = useState('ICP');
  const [royalty, setRoyalty] = useState('10');
  const [tags, setTags] = useState<string[]>([]);
  const [tagInput, setTagInput] = useState('');
  const [forSale, setForSale] = useState(false);
  const [price, setPrice] = useState('');
  const [loading, setLoading] = useState(false);
  const [result, setResult] = useState<any | null>(null);
  const [err, setErr] = useState<string | null>(null);

  const addTag = () => {
    const t = tagInput.trim().replace(/^#/, '');
    if (t && !tags.includes(t) && tags.length < 8) { setTags([...tags, t]); setTagInput(''); }
  };

  const submit = async () => {
    if (files.length === 0) { alert('Вибери файли'); return; }
    setLoading(true);
    setErr(null);
    try {
      const items = files.map((f, i) => ({
        title: f.name.replace(/\.[^.]+$/, '') || `Batch NFT #${i + 1}`,
        description: `Batch upload item ${i + 1}`,
        price: forSale && price ? parseFloat(price) : undefined,
      }));
      const metadata = {
        batchName: 'Batch',
        blockchain,
        currency,
        royalty: parseFloat(royalty),
        forSale,
        tags,
        items,
      };
      const compressed = await Promise.all(files.map(f => compressImage(f)));
      const form = new FormData();
      compressed.forEach(f => form.append('images[]', f));
      form.append('metadata', JSON.stringify(metadata));

      const res: any = await apiBatchCreateNFTs(form);

      const successful = (res?.results ?? []).filter((r: any) => r.status === 'ok' && r.id && r.imageUrl);
      for (let i = 0; i < successful.length; i++) {
        const r = successful[i];
        const meta = items[r.index ?? i];
        await apiCreatePost({
          nftImage: r.imageUrl,
          title: meta?.title ?? `Batch NFT #${i + 1}`,
          description: meta?.description ?? '',
          tags,
          forSale,
          price: forSale && price ? parseFloat(price) : null,
          currency,
          walletNftId: r.id,
        });
      }
      setResult(res);
    } catch (e: any) {
      setErr(e?.message ?? 'Batch failed');
    } finally {
      setLoading(false);
    }
  };

  if (result) {
    return (
      <div className="success-box">
        <div className="circle">📦</div>
        <h2 style={{ margin: '0 0 8px' }}>Масовий випуск завершено</h2>
        <p className="sub">
          ✅ Створено: <strong style={{ color: 'var(--text)' }}>{result.created}</strong>
          {result.failed > 0 && <> &nbsp; ❌ Помилок: <strong style={{ color: 'var(--danger)' }}>{result.failed}</strong></>}
        </p>
        {result.failed > 0 && (
          <div style={{ maxWidth: 560, margin: '14px auto 0', textAlign: 'left' }}>
            {result.results?.filter((r: any) => r.status === 'error').map((r: any) => (
              <div key={r.index} className="error-banner" style={{ marginBottom: 6 }}>
                Item #{r.index + 1}: {r.message || 'Unknown error'}
              </div>
            ))}
          </div>
        )}
        <div style={{ display: 'flex', gap: 10, justifyContent: 'center', marginTop: 18 }}>
          <button className="btn btn-primary" onClick={() => { setResult(null); setFiles([]); }}>
            <Icon.Plus /> Завантажити ще партію
          </button>
          <button className="btn" onClick={onDone}>До списку</button>
        </div>
      </div>
    );
  }

  return (
    <div className="card" style={{ maxWidth: 720 }}>
      <h3>📦 Масовий випуск</h3>
      <p className="sub" style={{ marginBottom: 16 }}>
        Завантаження кількох NFT однією відправкою — для брендів і компаній.
      </p>

      <div className="notice notice-warn">
        ⚠️ До продажів кожного NFT у цій партії застосовується комісія платформи 1%.
      </div>

      <div className="field">
        <label>Файли *</label>
        <input
          ref={fileRef}
          type="file"
          accept="image/*"
          multiple
          style={{ display: 'none' }}
          onChange={e => setFiles(Array.from(e.target.files || []))}
        />
        <div className={`drop-zone ${files.length > 0 ? 'has-file' : ''}`} onClick={() => fileRef.current?.click()}>
          {files.length > 0 ? (
            <div style={{ width: '100%', padding: 14 }}>
              <div style={{ display: 'grid', gridTemplateColumns: 'repeat(6, 1fr)', gap: 8, marginBottom: 8 }}>
                {files.slice(0, 12).map((f, i) => (
                  <div key={i} style={{ aspectRatio: '1 / 1', background: 'var(--bg-soft)', borderRadius: 8, overflow: 'hidden' }}>
                    <img src={URL.createObjectURL(f)} alt="" style={{ width: '100%', height: '100%', objectFit: 'cover' }} />
                  </div>
                ))}
              </div>
              <div className="sub" style={{ textAlign: 'center' }}>
                {files.length} файл(ов){files.length > 12 ? ` (показано 12)` : ''}
              </div>
            </div>
          ) : (
            <div>
              <div className="ph-title">+ Вибери кілька зображень</div>
              <div className="ph-sub">PNG / JPG / GIF · до 10 MB кожен</div>
            </div>
          )}
        </div>
      </div>

      <div className="field">
        <label>Блокчейн</label>
        <div className="chip-row">
          {BLOCKCHAINS.map(b => (
            <button
              type="button"
              key={b.id}
              className={`chip ${blockchain === b.id ? 'active' : ''}`}
              onClick={() => { setBlockchain(b.id); setCurrency(b.currency); }}
            >{b.icon} {b.name}</button>
          ))}
        </div>
      </div>

      <div className="field">
        <label>Валюта</label>
        <div className="chip-row">
          {CURRENCIES.map(c => (
            <button
              type="button"
              key={c}
              className={`chip ${currency === c ? 'active' : ''}`}
              onClick={() => setCurrency(c)}
            >{c}</button>
          ))}
        </div>
      </div>

      <div className="field">
        <label>Роялті: <strong style={{ color: 'var(--primary)' }}>{royalty}%</strong></label>
        <input
          type="range"
          min="0"
          max="30"
          step="1"
          value={royalty}
          onChange={e => setRoyalty(e.target.value)}
          className="slider"
        />
      </div>

      <div className="field">
        <label>Теги (застосовуються до всіх)</label>
        <div className="tag-input-row">
          <input
            placeholder="#tag"
            value={tagInput}
            onChange={e => setTagInput(e.target.value)}
            onKeyDown={e => { if (e.key === 'Enter' || e.key === ',') { e.preventDefault(); addTag(); } }}
          />
          <button type="button" className="btn" onClick={addTag}>Додати</button>
        </div>
        {tags.length > 0 && (
          <div style={{ marginTop: 6 }}>
            {tags.map(t => (
              <span key={t} className="tag-badge">
                #{t}
                <button type="button" className="x" onClick={() => setTags(tags.filter(x => x !== t))}>✕</button>
              </span>
            ))}
          </div>
        )}
      </div>

      <div className="toggle-row">
        <div>
          <div className="lbl">Виставити всіх на продаж</div>
          <div className="sub">Одна ціна застосовується до всіх NFT у партії.</div>
        </div>
        <div className={`switch ${forSale ? 'on' : ''}`} onClick={() => setForSale(!forSale)}>
          <div className="knob" />
        </div>
      </div>

      {forSale && (
        <div className="field">
          <label>Цена ({currency})</label>
          <input type="number" min="0" step="0.001" value={price} onChange={e => setPrice(e.target.value)} placeholder="0.00" />
        </div>
      )}

      {err && <div className="error-banner">{err}</div>}

      <button
        type="button"
        className="btn btn-primary btn-block"
        onClick={submit}
        disabled={files.length === 0 || loading}
      >
        {loading ? `Завантаження ${files.length}…` : `🚀 Випустити ${files.length || 0} NFT`}
      </button>
    </div>
  );
}

// ────────────────────────────────────────────────────────────────────────────
// QR MODAL
// ────────────────────────────────────────────────────────────────────────────

function QrModal({ nft, onClose }: { nft: NFT; onClose: () => void }) {
  const [src, setSrc] = useState<string>('');
  const [copied, setCopied] = useState(false);
  const [generating, setGenerating] = useState(false);

  const url = useMemo(() => {
    const base = (import.meta.env.VITE_QR_BASE_URL as string) || 'https://your-domain.github.io/idenity/';
    if (nft.nfcUid) return `${base}?nfc=${encodeURIComponent(nft.nfcUid)}`;
    return `${base}?nft=${encodeURIComponent(nft.id)}`;
  }, [nft.id, nft.nfcUid]);

  useEffect(() => {
    let cancelled = false;
    if (nft.qrImageUrl) {
      setSrc(nft.qrImageUrl);
      return;
    }
    setGenerating(true);
    apiGenerateNFTQr(nft.id)
      .then((updated: any) => {
        if (!cancelled) {
          setSrc(updated.qrImageUrl || '');
          setGenerating(false);
        }
      })
      .catch(() => {
        // Fallback: generate client-side if server generation fails
        if (!cancelled) {
          QRCode.toDataURL(url, {
            width: 320,
            margin: 3,
            color: { dark: '#0b0f14', light: '#ffffff' },
            errorCorrectionLevel: 'H',
          }).then(dataUrl => {
            if (!cancelled) {
              setSrc(dataUrl);
              setGenerating(false);
            }
          });
        }
      });
    return () => { cancelled = true; };
  }, [url, nft.qrImageUrl, nft.id]);

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(url);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch { /* noop */ }
  };

  const download = () => {
    if (!src) return;
    const a = document.createElement('a');
    a.href = src;
    a.download = `qr-${nft.title.replace(/[^a-zа-я0-9_-]+/gi, '_')}.png`;
    a.click();
  };

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div className="modal" onClick={e => e.stopPropagation()} style={{ maxWidth: 440 }}>
        <h3>QR-код товару</h3>
        <p className="sub">
          Скан відкриває сторінку з фото та метаданими «{nft.title}».
          {nft.nfcUid ? ' Код прив’язано до NFC.' : ''}
        </p>

        <div style={{ background: 'white', borderRadius: 14, padding: 16, display: 'flex', justifyContent: 'center', margin: '6px 0 14px' }}>
          {src ? (
            <img src={src} alt="QR" style={{ width: 280, height: 280, display: 'block' }} />
          ) : (
            <div style={{ width: 280, height: 280, display: 'flex', alignItems: 'center', justifyContent: 'center', color: '#0b0f14' }}>
              Генерація…
            </div>
          )}
        </div>

        <div className="field">
          <label>Посилання</label>
          <input value={url} readOnly onFocus={e => e.currentTarget.select()} />
        </div>

        <div className="actions">
          <button className="btn" onClick={copy}>{copied ? 'Скопійовано ✓' : 'Скопіювати'}</button>
          <button className="btn" onClick={download} disabled={!src}>Завантажити PNG</button>
          <button className="btn btn-primary" onClick={onClose}>Готово</button>
        </div>
      </div>
    </div>
  );
}

// ────────────────────────────────────────────────────────────────────────────
// EDIT MODAL
// ────────────────────────────────────────────────────────────────────────────

function EditNftModal({ nft, onClose, onSaved }: { nft: NFT; onClose: () => void; onSaved: () => Promise<void> }) {
  const [title, setTitle] = useState(nft.title);
  const [description, setDescription] = useState(nft.description ?? '');
  const [forSale, setForSale] = useState(!!nft.forSale);
  const [price, setPrice] = useState(nft.price?.toString() ?? '');
  const [currency, setCurrency] = useState(nft.currency ?? 'USDC');
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  const submit = async () => {
    setBusy(true);
    setErr(null);
    try {
      await apiUpdateNFT(nft.id, {
        title: title.trim(),
        description: description.trim(),
        forSale,
        price: forSale ? parseFloat(price) || 0 : undefined,
        currency: forSale ? currency : undefined,
      });
      await onSaved();
    } catch (e: any) {
      setErr(e?.message ?? 'Save failed');
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="modal-backdrop" onClick={busy ? undefined : onClose}>
      <div className="modal" onClick={e => e.stopPropagation()}>
        <h3>Змінити NFT</h3>

        <div className="field">
          <label>Назва</label>
          <input value={title} onChange={e => setTitle(e.target.value)} />
        </div>
        <div className="field">
          <label>Опис</label>
          <textarea value={description} onChange={e => setDescription(e.target.value)} />
        </div>

        <div className="field">
          <label style={{ display: 'flex', alignItems: 'center', gap: 8, cursor: 'pointer' }}>
            <input type="checkbox" checked={forSale} onChange={e => setForSale(e.target.checked)} />
            Виставити на продаж
          </label>
        </div>

        {forSale && (
          <div style={{ display: 'grid', gridTemplateColumns: '1fr 120px', gap: 12 }}>
            <div className="field">
              <label>Ціна</label>
              <input value={price} onChange={e => setPrice(e.target.value)} inputMode="decimal" />
            </div>
            <div className="field">
              <label>Валюта</label>
              <select value={currency} onChange={e => setCurrency(e.target.value)}>
                <option value="USDC">USDC</option>
                <option value="ICP">ICP</option>
                <option value="USD">USD</option>
                <option value="UAH">UAH</option>
              </select>
            </div>
          </div>
        )}

        {err && <div className="error-banner">{err}</div>}

        <div className="actions">
          <button className="btn" onClick={onClose} disabled={busy}>Скасувати</button>
          <button className="btn btn-primary" onClick={submit} disabled={busy}>
            {busy ? 'Збереження…' : 'Зберегти'}
          </button>
        </div>
      </div>
    </div>
  );
}
