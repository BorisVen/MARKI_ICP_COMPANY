use axum::{
    extract::{Multipart, Path, State},
    http::StatusCode,
    Extension, Json,
};
use chrono::Utc;
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

use crate::{
    errors::{ApiResult, AppError},
    middleware::auth::AuthenticatedUser,
    models::{BatchItemResult, BatchNftInput, BatchUploadResponse, CreateEditionResponse, CreateNftRequest, Nft, PaymentQuoteRequest, TransferNftRequest, UpdateNftRequest, VerifyNftRequest, VerifyNftResponse},
    notification_helpers,
    services::StorageClient,
    AppState,
};

// ── Helpers ───────────────────────────────────────────────────────────────────

fn ext_from_ct(ct: &str) -> &str {
    match ct {
        "image/png" => "png",
        "image/gif" => "gif",
        "image/webp" => "webp",
        _ => "jpg",
    }
}

fn collection_title_after_sale(title: &str, remaining: usize) -> String {
    if let Some((base, suffix)) = title.rsplit_once(" (") {
        if suffix.ends_with(" editions)") {
            return format!("{} ({} editions)", base, remaining);
        }
    }
    title.to_owned()
}

/// Load the `nfts` array from `marki_wallets/{uid}`.
async fn load_nfts(state: &AppState, uid: &str) -> ApiResult<Vec<Nft>> {
    let doc = state
        .firestore
        .get("marki_wallets", uid)
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?
        .unwrap_or_else(|| json!({ "nfts": [] }));

    let nfts: Vec<Nft> = doc
        .get("nfts")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();

    Ok(nfts)
}

/// Persist the `nfts` array back to `marki_wallets/{uid}`.
async fn save_nfts(state: &AppState, uid: &str, nfts: &[Nft]) -> ApiResult<()> {
    state
        .firestore
        .update("marki_wallets", uid, &json!({ "nfts": nfts }))
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?;
    Ok(())
}

/// Viewer link encoded into an NFT's QR code.
fn viewer_url(state: &AppState, nft_id: &str) -> String {
    let base = &state.config.qr_base_url;
    let sep = if base.contains('?') { '&' } else { '?' };
    format!("{}{}nft={}", base, sep, urlencoding::encode(nft_id))
}

/// Render `url` as a PNG QR code (high error correction, quiet zone, ~320 px).
fn render_qr_png(url: &str) -> ApiResult<Vec<u8>> {
    let code = qrcode::QrCode::with_error_correction_level(url.as_bytes(), qrcode::EcLevel::H)
        .map_err(|e| AppError::BadRequest(format!("QR encode failed: {e}")))?;
    let img = code
        .render::<image::Luma<u8>>()
        .min_dimensions(320, 320)
        .quiet_zone(true)
        .build();
    let mut png = std::io::Cursor::new(Vec::new());
    img.write_to(&mut png, image::ImageFormat::Png)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("QR PNG encode failed: {e}")))?;
    Ok(png.into_inner())
}

/// Generate the QR PNG for one NFT, upload it and return its URL.
/// The `v` query parameter busts browser caches after a regeneration.
async fn upload_nft_qr(state: &AppState, owner_id: &str, nft_id: &str) -> ApiResult<String> {
    let png = render_qr_png(&viewer_url(state, nft_id))?;
    let path = format!("qr/{}/{}.png", owner_id, nft_id);
    let url = state
        .storage
        .upload(&path, png, "image/png")
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?;
    Ok(format!("{}&v={}", url, Utc::now().timestamp()))
}

/// Upload image bytes to Firebase Storage and return the public URL.
async fn upload_nft_image(
    state: &AppState,
    owner_id: &str,
    nft_id: &str,
    bytes: Vec<u8>,
    content_type: &str,
) -> ApiResult<String> {
    let ext = ext_from_ct(content_type);
    let path = StorageClient::nft_path(owner_id, nft_id, ext);
    state
        .storage
        .upload(&path, bytes, content_type)
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))
}

// ── Handlers ──────────────────────────────────────────────────────────────────

/// `GET /api/nfts/mint-info`
/// Returns the caller's lifetime mint count and whether this next mint will
/// incur a platform commission (kicks in from the 4th mint onward).
/// Commission = 1 % of the typical Metaplex mint cost (~0.012 SOL ≈ 12 000 000 lamports).
pub async fn get_mint_info(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedUser>,
) -> ApiResult<Json<serde_json::Value>> {
    let doc = state
        .firestore
        .get("marki_wallets", &auth.uid)
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?
        .unwrap_or_else(|| json!({}));

    let mint_count = doc.get("mintCount").and_then(|v| v.as_u64()).unwrap_or(0);

    // First 3 mints are free (only gas). Commission starts on the 4th.
    let commission_lamports: u64 = if mint_count >= 3 { 120_000 } else { 0 };

    Ok(Json(json!({
        "mintCount":          mint_count,
        "isFree":             mint_count < 3,
        "commissionLamports": commission_lamports,
    })))
}

/// `GET /api/nfts`
pub async fn get_nfts(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedUser>,
) -> ApiResult<Json<Vec<Nft>>> {
    let wallet = state
        .firestore
        .get("marki_wallets", &auth.uid)
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?
        .unwrap_or_else(|| json!({}));
    let mut nfts: Vec<Nft> = wallet
        .get("nfts")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    // Views are counted outside the array; see public::nft_views.
    for n in nfts.iter_mut() {
        n.views = crate::handlers::public::nft_views(&wallet, n);
    }
    Ok(Json(nfts))
}

/// `GET /api/nfts/:id`
pub async fn get_nft(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedUser>,
    Path(nft_id): Path<String>,
) -> ApiResult<Json<Nft>> {
    let nfts = load_nfts(&state, &auth.uid).await?;
    let nft = nfts
        .into_iter()
        .find(|n| n.id == nft_id)
        .ok_or_else(|| AppError::NotFound(format!("NFT {} not found", nft_id)))?;
    Ok(Json(nft))
}

/// Verify that an NFT is present in an Idenity wallet and report whether its
/// issuing brand has passed company verification.
pub async fn verify_nft(
    State(state): State<Arc<AppState>>,
    Json(body): Json<VerifyNftRequest>,
) -> ApiResult<Json<VerifyNftResponse>> {
    let nft = load_nfts(&state, &body.owner_id)
        .await?
        .into_iter()
        .find(|n| n.id == body.nft_id && n.owner_id == body.owner_id)
        .ok_or_else(|| AppError::NotFound("This QR is not registered by Idenity".to_owned()))?;

    let issuer = state
        .firestore
        .get("users", &body.owner_id)
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?
        .ok_or_else(|| AppError::NotFound("Issuer profile not found".to_owned()))?;

    Ok(Json(VerifyNftResponse {
        issued_by_idenity: true,
        issuer_verified: issuer.get("companyApproved").and_then(|v| v.as_bool()).unwrap_or(false),
        issuer_name: issuer.get("companyName")
            .or_else(|| issuer.get("name"))
            .and_then(|v| v.as_str())
            .unwrap_or(&nft.owner_name)
            .to_owned(),
        nft,
    }))
}

/// `POST /api/nfts`
/// Multipart body: `image` (file) + `metadata` (JSON string).
pub async fn create_nft(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedUser>,
    mut multipart: Multipart,
) -> ApiResult<(StatusCode, Json<Nft>)> {
    let mut image_bytes: Option<Vec<u8>> = None;
    let mut image_ct = "image/jpeg".to_owned();
    let mut meta: Option<CreateNftRequest> = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::BadRequest(e.to_string()))?
    {
        let name = field.name().unwrap_or("").to_owned();
        match name.as_str() {
            "image" => {
                image_ct = field
                    .content_type()
                    .unwrap_or("image/jpeg")
                    .to_owned();
                image_bytes = Some(
                    field
                        .bytes()
                        .await
                        .map_err(|e| AppError::BadRequest(e.to_string()))?
                        .to_vec(),
                );
            }
            "metadata" => {
                let text = field
                    .text()
                    .await
                    .map_err(|e| AppError::BadRequest(e.to_string()))?;
                meta = Some(
                    serde_json::from_str(text.trim())
                        .map_err(|e| AppError::BadRequest(format!("Invalid metadata JSON: {}", e)))?,
                );
            }
            _ => {
                // Drain unrecognised fields to keep the multipart stream healthy.
                let _ = field.bytes().await;
            }
        }
    }

    let bytes = image_bytes
        .ok_or_else(|| AppError::BadRequest("Missing 'image' field".to_owned()))?;
    let req = meta.ok_or_else(|| AppError::BadRequest("Missing 'metadata' field".to_owned()))?;

    // Fetch owner name from profile.
    let user_doc = state
        .firestore
        .get("users", &auth.uid)
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?;
    let owner_name = user_doc
        .as_ref()
        .and_then(|d| d.get("name"))
        .and_then(|v| v.as_str())
        .unwrap_or("Unknown")
        .to_owned();

    let nft_id = Uuid::new_v4().to_string();
    let image_url = upload_nft_image(&state, &auth.uid, &nft_id, bytes, &image_ct).await?;

    // Build the off-chain Metaplex metadata JSON and upload it to Firebase Storage
    // so the frontend can hand `metadataUri` straight to Umi's `createNft()`.
    let metadata_json = json!({
        "name":        req.title,
        "symbol":      "",
        "description": req.description,
        "image":       image_url,
        "attributes": [
            { "trait_type": "Category",    "value": req.category.as_deref().unwrap_or("Art") },
            { "trait_type": "Blockchain",  "value": "Solana" },
            { "trait_type": "Idenity Platform Resale Fee", "value": "3%" }
        ],
        "properties": {
            "files":    [{ "uri": image_url, "type": image_ct }],
            "category": "image"
        }
    });
    let metadata_bytes = serde_json::to_vec(&metadata_json)
        .map_err(|e| AppError::BadRequest(format!("Failed to serialise metadata: {}", e)))?;
    let metadata_path = StorageClient::metadata_path(&auth.uid, &nft_id);
    let metadata_uri = state
        .storage
        .upload(&metadata_path, metadata_bytes, "application/json")
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?;

    let nft = Nft {
        id: nft_id,
        title: req.title.clone(),
        description: req.description,
        image: image_url,
        tags: req.tags,
        category: req.category,
        blockchain: Some(req.blockchain),
        royalty: Some(req.royalty),
        owner_id: auth.uid.clone(),
        owner_name,
        price: req.price,
        for_sale: req.for_sale,
        currency: Some(req.currency),
        created_at: Utc::now().to_rfc3339(),
        views: 0,
        metadata_uri: Some(metadata_uri),
        mint_address: None,
        edition_count: None,
        edition_number: None,
        master_nft_id: None,
        batch_id: None,
        batch_name: None,
        batch_index: None,
        batch_size: None,
        qr_image_url: None,
    };

    let mut nfts = load_nfts(&state, &auth.uid).await?;
    nfts.push(nft.clone());
    save_nfts(&state, &auth.uid, &nfts).await?;

    // Increment the lifetime mint counter (freemium gate tracked here).
    // Non-fatal: a failure should not block the successful mint response.
    let wallet_doc = state
        .firestore
        .get("marki_wallets", &auth.uid)
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| json!({}));
    let new_count = wallet_doc.get("mintCount").and_then(|v| v.as_u64()).unwrap_or(0) + 1;
    let _ = state
        .firestore
        .update("marki_wallets", &auth.uid, &json!({ "mintCount": new_count }))
        .await;

    notification_helpers::notify_nft_created(&state.firestore, &auth.uid, &req.title).await;

    Ok((StatusCode::CREATED, Json(nft)))
}

/// `POST /api/nfts/batch`
/// Multipart body: multiple `images[]` fields + one `metadata` JSON field.
pub async fn batch_create_nfts(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedUser>,
    mut multipart: Multipart,
) -> ApiResult<Json<BatchUploadResponse>> {
    // Collect all files first, then process.
    let mut files: Vec<(Vec<u8>, String)> = Vec::new(); // (bytes, content_type)
    let mut batch_meta: Option<BatchNftInput> = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::BadRequest(e.to_string()))?
    {
        let name = field.name().unwrap_or("").to_owned();
        match name.as_str() {
            "metadata" => {
                let text = field
                    .text()
                    .await
                    .map_err(|e| AppError::BadRequest(e.to_string()))?;
                batch_meta = Some(
                    serde_json::from_str(text.trim())
                        .map_err(|e| AppError::BadRequest(format!("Invalid metadata: {}", e)))?,
                );
            }
            // Accept both "images[]" (PHP-style) and plain "images"
            n if n == "images[]" || n == "images" => {
                let ct = field
                    .content_type()
                    .unwrap_or("image/jpeg")
                    .to_owned();
                let bytes = field
                    .bytes()
                    .await
                    .map_err(|e| AppError::BadRequest(e.to_string()))?
                    .to_vec();
                if !bytes.is_empty() {
                    files.push((bytes, ct));
                }
            }
            _ => {
                // Drain unrecognised fields to keep the multipart stream healthy.
                let _ = field.bytes().await;
            }
        }
    }

    if files.is_empty() {
        return Err(AppError::BadRequest("No image files provided".to_owned()));
    }

    let shared = batch_meta
        .ok_or_else(|| AppError::BadRequest("Missing 'metadata' field".to_owned()))?;

    // Fetch owner name once.
    let user_doc = state
        .firestore
        .get("users", &auth.uid)
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?;
    let owner_name = user_doc
        .as_ref()
        .and_then(|d| d.get("name"))
        .and_then(|v| v.as_str())
        .unwrap_or("Unknown")
        .to_owned();

    let mut nfts = load_nfts(&state, &auth.uid).await?;
    let mut results: Vec<BatchItemResult> = Vec::with_capacity(files.len());
    let mut created = 0usize;
    let mut failed = 0usize;
    let batch_id = Uuid::new_v4().to_string();
    let batch_size = files.len() as u32;
    let batch_name = shared
        .batch_name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("Collection")
        .to_owned();

    for (index, (bytes, ct)) in files.into_iter().enumerate() {
        let item_meta = shared.items.get(index);
        let title = item_meta
            .and_then(|m| m.title.as_deref())
            .map(str::to_owned)
            .unwrap_or_else(|| format!("NFT #{}", index + 1));
        let description = item_meta
            .and_then(|m| m.description.as_deref())
            .unwrap_or("")
            .to_owned();
        let price = item_meta.and_then(|m| m.price).or(None);

        let nft_id = Uuid::new_v4().to_string();
        match upload_nft_image(&state, &auth.uid, &nft_id, bytes, &ct).await {
            Ok(image_url) => {
                // Generate and upload the off-chain Metaplex metadata JSON so the
                // frontend can pass `metadataUri` straight into Umi's `createNft()`.
                let metadata_json = json!({
                    "name":        title,
                    "symbol":      "",
                    "description": description,
                    "image":       image_url,
                    "attributes":  [
                        { "trait_type": "Blockchain", "value": "Solana" },
                        { "trait_type": "Idenity Platform Resale Fee", "value": "3%" }
                    ],
                    "properties":  {
                        "files":    [{ "uri": image_url, "type": ct }],
                        "category": "image"
                    }
                });
                let metadata_bytes = serde_json::to_vec(&metadata_json)
                    .map_err(|e| AppError::BadRequest(format!("Failed to serialise metadata: {}", e)))?;
                let metadata_path = StorageClient::metadata_path(&auth.uid, &nft_id);
                let metadata_uri = state
                    .storage
                    .upload(&metadata_path, metadata_bytes, "application/json")
                    .await
                    .map_err(|e| AppError::Firebase(e.to_string()))?;

                let nft = Nft {
                    id: nft_id.clone(),
                    title: title.clone(),
                    description,
                    image: image_url.clone(),
                    tags: shared.tags.clone(),
                    category: None,
                    blockchain: Some(shared.blockchain.clone()),
                    royalty: Some(shared.royalty),
                    owner_id: auth.uid.clone(),
                    owner_name: owner_name.clone(),
                    price,
                    for_sale: shared.for_sale,
                    currency: Some(shared.currency.clone()),
                    created_at: Utc::now().to_rfc3339(),
                    views: 0,
                    metadata_uri: Some(metadata_uri.clone()),
                    mint_address: None,
                    edition_count: None,
                    edition_number: None,
                    master_nft_id: None,
                    batch_id: Some(batch_id.clone()),
                    batch_name: Some(batch_name.clone()),
                    batch_index: Some(index as u32),
                    batch_size: Some(batch_size),
                    qr_image_url: None,
                };
                nfts.push(nft);
                results.push(BatchItemResult {
                    index,
                    id: Some(nft_id),
                    status: "ok".to_owned(),
                    message: None,
                    image_url: Some(image_url),
                    metadata_uri: Some(metadata_uri),
                });
                created += 1;
            }
            Err(e) => {
                results.push(BatchItemResult {
                    index,
                    id: None,
                    status: "error".to_owned(),
                    message: Some(e.to_string()),
                    image_url: None,
                    metadata_uri: None,
                });
                failed += 1;
            }
        }
    }

    // Persist all successfully created NFTs in one write.
    if created > 0 {
        save_nfts(&state, &auth.uid, &nfts).await?;
        notification_helpers::notify_batch_created(&state.firestore, &auth.uid, created).await;
    }

    Ok(Json(BatchUploadResponse { created, failed, results }))
}

/// `PUT /api/nfts/:id`
pub async fn update_nft(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedUser>,
    Path(nft_id): Path<String>,
    Json(body): Json<UpdateNftRequest>,
) -> ApiResult<Json<Nft>> {
    let mut nfts = load_nfts(&state, &auth.uid).await?;

    let nft = nfts
        .iter_mut()
        .find(|n| n.id == nft_id)
        .ok_or_else(|| AppError::NotFound(format!("NFT {} not found", nft_id)))?;

    if let Some(v) = body.title        { nft.title = v; }
    if let Some(v) = body.description  { nft.description = v; }
    if let Some(v) = body.tags         { nft.tags = v; }
    if let Some(v) = body.category     { nft.category = Some(v); }
    if let Some(v) = body.price        { nft.price = Some(v); }
    if let Some(v) = body.for_sale     { nft.for_sale = v; }
    if let Some(v) = body.currency     { nft.currency = Some(v); }
    if let Some(v) = body.mint_address { nft.mint_address = Some(v); }

    let updated = nft.clone();
    save_nfts(&state, &auth.uid, &nfts).await?;
    Ok(Json(updated))
}

/// `POST /api/nfts/:id/qr` — create (or recreate) the QR code of one NFT.
pub async fn generate_nft_qr(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedUser>,
    Path(nft_id): Path<String>,
) -> ApiResult<Json<Nft>> {
    let mut nfts = load_nfts(&state, &auth.uid).await?;
    let idx = nfts
        .iter()
        .position(|n| n.id == nft_id)
        .ok_or_else(|| AppError::NotFound(format!("NFT {} not found", nft_id)))?;
    nfts[idx].qr_image_url = Some(upload_nft_qr(&state, &auth.uid, &nft_id).await?);
    save_nfts(&state, &auth.uid, &nfts).await?;
    Ok(Json(nfts[idx].clone()))
}

/// Generate QR codes for the caller's NFTs: all of them when `regenerate`,
/// otherwise only those without one. Saves once at the end.
async fn generate_qr_codes(state: &AppState, uid: &str, regenerate: bool) -> ApiResult<usize> {
    let mut nfts = load_nfts(state, uid).await?;
    let mut count = 0;
    for nft in nfts.iter_mut().filter(|n| regenerate || n.qr_image_url.is_none()) {
        nft.qr_image_url = Some(upload_nft_qr(state, uid, &nft.id).await?);
        count += 1;
    }
    if count > 0 {
        save_nfts(state, uid, &nfts).await?;
    }
    Ok(count)
}

/// `POST /api/nfts/qr-missing` — QR codes for NFTs that have none yet.
pub async fn generate_missing_qr(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedUser>,
) -> ApiResult<Json<serde_json::Value>> {
    let generated = generate_qr_codes(&state, &auth.uid, false).await?;
    Ok(Json(json!({ "generated": generated })))
}

/// `POST /api/nfts/regenerate-qr` — recreate every QR code, e.g. after
/// `QR_BASE_URL` changed.
pub async fn regenerate_all_qr(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedUser>,
) -> ApiResult<Json<serde_json::Value>> {
    let regenerated = generate_qr_codes(&state, &auth.uid, true).await?;
    Ok(Json(json!({ "regenerated": regenerated })))
}

/// `DELETE /api/nfts/:id`
pub async fn delete_nft(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedUser>,
    Path(nft_id): Path<String>,
) -> ApiResult<StatusCode> {
    let mut nfts = load_nfts(&state, &auth.uid).await?;
    let len_before = nfts.len();
    nfts.retain(|n| n.id != nft_id);

    if nfts.len() == len_before {
        return Err(AppError::NotFound(format!("NFT {} not found", nft_id)));
    }

    save_nfts(&state, &auth.uid, &nfts).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ─────────────────────────────────────────────────────────────────────────────

/// `POST /api/nfts/editions`
///
/// Creates one Master Edition NFT record plus N print-edition placeholder
/// records in Firestore, all sharing a single uploaded image and metadata JSON.
///
/// The frontend is responsible for the on-chain work:
///   1. `createNft(…, { maxSupply: some(N) })` — registers the Master Edition.
///   2. `printV1(…)` × N                       — mints each print edition.
///   3. `PUT /api/nfts/:id { mintAddress }`     — saves each on-chain address.
///
/// Multipart body: `image` (file) + `metadata` (JSON, must include `editionCount`).
pub async fn create_edition_nfts(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedUser>,
    mut multipart: Multipart,
) -> ApiResult<(StatusCode, Json<CreateEditionResponse>)> {
    let mut image_bytes: Option<Vec<u8>> = None;
    let mut image_ct = "image/jpeg".to_owned();
    let mut meta: Option<CreateNftRequest> = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::BadRequest(e.to_string()))?
    {
        let name = field.name().unwrap_or("").to_owned();
        match name.as_str() {
            "image" => {
                image_ct    = field.content_type().unwrap_or("image/jpeg").to_owned();
                image_bytes = Some(
                    field.bytes().await
                        .map_err(|e| AppError::BadRequest(e.to_string()))?.to_vec(),
                );
            }
            "metadata" => {
                let text = field.text().await
                    .map_err(|e| AppError::BadRequest(e.to_string()))?;
                meta = Some(
                    serde_json::from_str(text.trim())
                        .map_err(|e| AppError::BadRequest(format!("Invalid metadata JSON: {}", e)))?,
                );
            }
            _ => { let _ = field.bytes().await; }
        }
    }

    let bytes = image_bytes
        .ok_or_else(|| AppError::BadRequest("Missing 'image' field".into()))?;
    let req   = meta
        .ok_or_else(|| AppError::BadRequest("Missing 'metadata' field".into()))?;

    // edition_count must be ≥ 2 for this endpoint to make sense.
    let edition_count = req.edition_count
        .filter(|&n| n >= 2)
        .ok_or_else(|| AppError::BadRequest(
            "Field 'editionCount' must be an integer ≥ 2".into()
        ))?
        .min(100); // hard cap: 100 editions per request

    // Fetch owner display name.
    let user_doc  = state.firestore.get("users", &auth.uid).await
        .map_err(|e| AppError::Firebase(e.to_string()))?;
    let owner_name = user_doc.as_ref()
        .and_then(|d| d.get("name")).and_then(|v| v.as_str())
        .unwrap_or("Unknown").to_owned();

    // Upload the shared image once.
    let master_id = Uuid::new_v4().to_string();
    let image_url = upload_nft_image(&state, &auth.uid, &master_id, bytes, &image_ct).await?;

    // Build and upload the shared off-chain metadata JSON.
    let metadata_json = json!({
        "name":        req.title,
        "symbol":      "",
        "description": req.description,
        "image":       image_url,
        "attributes": [
            { "trait_type": "Category",    "value": req.category.as_deref().unwrap_or("Art") },
            { "trait_type": "Blockchain",  "value": "Solana" },
            { "trait_type": "Edition",     "value": format!("1 of {}", edition_count) },
            { "trait_type": "Idenity Platform Resale Fee", "value": "3%" }
        ],
        "properties": {
            "files":    [{ "uri": image_url, "type": image_ct }],
            "category": "image"
        }
    });
    let metadata_bytes = serde_json::to_vec(&metadata_json)
        .map_err(|e| AppError::BadRequest(e.to_string()))?;
    let metadata_path = StorageClient::metadata_path(&auth.uid, &master_id);
    let metadata_uri  = state.storage
        .upload(&metadata_path, metadata_bytes, "application/json")
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?;

    let now = Utc::now().to_rfc3339();
    let batch_id = Uuid::new_v4().to_string();
    let batch_size = edition_count + 1;
    let batch_name = req
        .batch_name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(req.title.as_str())
        .to_owned();

    // ── Master Edition record (edition_number = 0) ─────────────────────────────
    let master = Nft {
        id:             master_id.clone(),
        title:          req.title.clone(),
        description:    req.description.clone(),
        image:          image_url.clone(),
        tags:           req.tags.clone(),
        category:       req.category.clone(),
        blockchain:     Some(req.blockchain.clone()),
        royalty:        Some(req.royalty),
        owner_id:       auth.uid.clone(),
        owner_name:     owner_name.clone(),
        price:          req.price,
        for_sale:       req.for_sale,
        currency:       Some(req.currency.clone()),
        created_at:     now.clone(),
        views:          0,
        metadata_uri:   Some(metadata_uri.clone()),
        mint_address:   None,
        edition_count:  Some(edition_count),
        edition_number: Some(0),
        master_nft_id:  None,
        batch_id:       Some(batch_id),
        batch_name:     Some(batch_name),
        batch_index:    Some(0),
        batch_size:     Some(batch_size),
        qr_image_url: None,
    };

    // ── Print-edition placeholder records (edition_number = 1..N) ─────────────
    let mut edition_ids: Vec<String> = Vec::with_capacity(edition_count as usize);
    let mut all_nfts = load_nfts(&state, &auth.uid).await?;
    all_nfts.push(master.clone());

    for n in 1..=edition_count {
        let eid = Uuid::new_v4().to_string();
        edition_ids.push(eid.clone());
        all_nfts.push(Nft {
            id:             eid,
            title:          format!("{} #{}", req.title, n),
            edition_number: Some(n),
            master_nft_id:  Some(master_id.clone()),
            batch_index:    Some(n),
            // Inherit everything else from master
            mint_address:   None,
            edition_count:  None, // only master carries the total
            ..master.clone()
        });
    }

    save_nfts(&state, &auth.uid, &all_nfts).await?;

    // Increment lifetime mint counter (non-fatal).
    let wallet_doc = state.firestore.get("marki_wallets", &auth.uid).await
        .ok().flatten().unwrap_or_else(|| json!({}));
    let new_count = wallet_doc.get("mintCount").and_then(|v| v.as_u64()).unwrap_or(0) + 1;
    let _ = state.firestore
        .update("marki_wallets", &auth.uid, &json!({ "mintCount": new_count }))
        .await;

    notification_helpers::notify_nft_created(&state.firestore, &auth.uid, &req.title).await;

    Ok((StatusCode::CREATED, Json(CreateEditionResponse {
        master_id,
        metadata_uri,
        image_url,
        edition_ids,
        edition_count,
    })))
}

/// `POST /api/nfts/:id/payment-quote`
///
/// Returns a short-lived, server-authoritative SOL amount for a marketplace
/// listing. Fiat prices are converted here rather than in the app, so the
/// amount later verified on-chain cannot be manipulated by the client.
pub async fn create_payment_quote(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedUser>,
    Path(nft_id): Path<String>,
    Json(body): Json<PaymentQuoteRequest>,
) -> ApiResult<Json<serde_json::Value>> {
    let post_doc = state
        .firestore
        .get("posts", &body.post_id)
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?
        .ok_or_else(|| AppError::NotFound(format!("Post {} not found", body.post_id)))?;

    if !post_doc
        .get("forSale")
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
    {
        return Err(AppError::BadRequest("This NFT is no longer for sale".to_owned()));
    }

    let seller_id = post_doc
        .get("userId")
        .and_then(|value| value.as_str())
        .ok_or_else(|| AppError::BadRequest("Listing has no seller".to_owned()))?;
    if seller_id == auth.uid {
        return Err(AppError::BadRequest("Cannot buy your own NFT".to_owned()));
    }

    let single_nft_id = post_doc.get("walletNftId").and_then(|value| value.as_str());
    let collection_contains_nft = post_doc
        .get("walletNftIds")
        .and_then(|value| value.as_array())
        .map(|ids| ids.iter().any(|id| id.as_str() == Some(nft_id.as_str())))
        .unwrap_or(false);
    if single_nft_id != Some(nft_id.as_str()) && !collection_contains_nft {
        return Err(AppError::BadRequest(
            "NFT does not belong to this marketplace listing".to_owned(),
        ));
    }

    let price = post_doc
        .get("price")
        .and_then(|value| value.as_f64())
        .filter(|value| value.is_finite() && *value > 0.0)
        .ok_or_else(|| AppError::BadRequest("Listing has no valid price".to_owned()))?;
    let currency = post_doc
        .get("currency")
        .and_then(|value| value.as_str())
        .unwrap_or("SOL")
        .trim()
        .to_ascii_uppercase();

    let price_sol = state
        .solana
        .listing_price_in_sol(price, &currency)
        .await
        .map_err(|error| AppError::BadRequest(format!("Unable to quote SOL payment: {error}")))?;

    const LAMPORTS_PER_SOL: f64 = 1_000_000_000.0;
    const PLATFORM_FEE: f64 = 0.01;
    let seller_lamports = (price_sol * LAMPORTS_PER_SOL).round() as u64;
    let fee_lamports = (price_sol * PLATFORM_FEE * LAMPORTS_PER_SOL).round() as u64;
    if seller_lamports == 0 {
        return Err(AppError::BadRequest("SOL quote is too small to pay".to_owned()));
    }

    let quote_id = Uuid::new_v4().to_string();
    let created_at = Utc::now();
    let expires_at_unix = (created_at + chrono::Duration::minutes(10)).timestamp();
    state
        .firestore
        .set(
            "payment_quotes",
            &quote_id,
            &json!({
                "buyerId": auth.uid,
                "sellerId": seller_id,
                "postId": body.post_id,
                "nftId": nft_id,
                "sourcePrice": price,
                "sourceCurrency": currency,
                "sellerLamports": seller_lamports,
                "feeLamports": fee_lamports,
                "expiresAtUnix": expires_at_unix,
                "used": false,
                "createdAt": created_at.to_rfc3339(),
            }),
        )
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?;

    Ok(Json(json!({
        "quoteId": quote_id,
        "sourcePrice": price,
        "sourceCurrency": currency,
        "sellerLamports": seller_lamports,
        "feeLamports": fee_lamports,
        "totalLamports": seller_lamports + fee_lamports,
        "expiresAtUnix": expires_at_unix,
    })))
}

/// `POST /api/nfts/:id/transfer`
///
/// Off-chain ownership sync called by the buyer immediately after an on-chain
/// Solana transaction confirms. Does NOT touch balances — the SOL already moved
/// on-chain. Steps:
///   a) Remove the NFT from `marki_wallets/{seller_id}`.
///   b) Stamp the record with the buyer's uid/name and `for_sale = false`.
///   c) Append it to `marki_wallets/{buyer_uid}`.
///   d) Set `forSale = false` on the corresponding `posts` document so the
///      listing can no longer be purchased.
pub async fn transfer_nft(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedUser>,
    Path(nft_id): Path<String>,
    Json(body): Json<TransferNftRequest>,
) -> ApiResult<Json<serde_json::Value>> {
    let buyer_uid = &auth.uid;

    if buyer_uid == &body.seller_id {
        return Err(AppError::BadRequest("Cannot transfer an NFT to yourself".to_owned()));
    }

    // The server, not the client, is authoritative for seller, price,
    // destination wallet and the NFT IDs included in the listing.
    let post_doc = state
        .firestore
        .get("posts", &body.post_id)
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?
        .ok_or_else(|| AppError::NotFound(format!("Post {} not found", body.post_id)))?;

    let post_seller = post_doc
        .get("userId")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::BadRequest("Listing has no seller".to_owned()))?;
    if post_seller != body.seller_id {
        return Err(AppError::BadRequest("Listing seller does not match".to_owned()));
    }

    let single_nft_id = post_doc.get("walletNftId").and_then(|v| v.as_str());
    let collection_nft_ids: Vec<String> = post_doc
        .get("walletNftIds")
        .and_then(|v| v.as_array())
        .map(|ids| {
            ids.iter()
                .filter_map(|id| id.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    if single_nft_id != Some(nft_id.as_str())
        && !collection_nft_ids.iter().any(|id| id == &nft_id)
    {
        return Err(AppError::BadRequest(
            "NFT does not belong to this marketplace listing".to_owned(),
        ));
    }

    let currency = post_doc
        .get("currency")
        .and_then(|v| v.as_str())
        .unwrap_or("SOL")
        .trim()
        .to_ascii_uppercase();
    let price = post_doc
        .get("price")
        .and_then(|v| v.as_f64())
        .filter(|price| *price > 0.0)
        .ok_or_else(|| AppError::BadRequest("Listing has no valid price".to_owned()))?;
    let seller_address = post_doc
        .get("sellerAddress")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::BadRequest("Seller has no connected Phantom wallet".to_owned()))?;
    if !state.solana.is_valid_address(seller_address)
        || !state.solana.is_valid_address(&body.payer_address)
    {
        return Err(AppError::BadRequest(
            "Buyer or seller wallet address is invalid; reconnect and relist the wallet".to_owned(),
        ));
    }

    let buyer_wallet_doc = state
        .firestore
        .get("crypto_wallets", buyer_uid)
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?;
    let payer_is_connected = buyer_wallet_doc
        .as_ref()
        .and_then(|doc| doc.get("wallets"))
        .and_then(|v| v.as_array())
        .map(|wallets| {
            wallets.iter().any(|wallet| {
                wallet.get("address").and_then(|v| v.as_str())
                    == Some(body.payer_address.as_str())
            })
        })
        .unwrap_or(false);
    if !payer_is_connected {
        return Err(AppError::BadRequest(
            "Payment wallet is not connected to this account".to_owned(),
        ));
    }

    let quote = state
        .firestore
        .get("payment_quotes", &body.quote_id)
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?
        .ok_or_else(|| AppError::BadRequest("Payment quote was not found".to_owned()))?;
    let quote_matches = quote.get("buyerId").and_then(|v| v.as_str()) == Some(buyer_uid)
        && quote.get("sellerId").and_then(|v| v.as_str()) == Some(body.seller_id.as_str())
        && quote.get("postId").and_then(|v| v.as_str()) == Some(body.post_id.as_str())
        && quote.get("nftId").and_then(|v| v.as_str()) == Some(nft_id.as_str())
        && quote.get("sourceCurrency").and_then(|v| v.as_str()) == Some(currency.as_str())
        && quote
            .get("sourcePrice")
            .and_then(|v| v.as_f64())
            .map(|quoted| (quoted - price).abs() < f64::EPSILON)
            .unwrap_or(false);
    if !quote_matches {
        return Err(AppError::BadRequest(
            "Payment quote does not match this listing".to_owned(),
        ));
    }
    if quote.get("used").and_then(|v| v.as_bool()).unwrap_or(false) {
        return Err(AppError::Conflict("Payment quote was already used".to_owned()));
    }
    if quote
        .get("expiresAtUnix")
        .and_then(|v| v.as_i64())
        .unwrap_or(0)
        <= Utc::now().timestamp()
    {
        return Err(AppError::BadRequest(
            "Payment quote expired; request a new quote".to_owned(),
        ));
    }
    let seller_lamports = quote
        .get("sellerLamports")
        .and_then(|v| v.as_u64())
        .filter(|value| *value > 0)
        .ok_or_else(|| AppError::BadRequest("Payment quote has no valid amount".to_owned()))?;
    let fee_lamports = quote
        .get("feeLamports")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    if state
        .firestore
        .get("processed_payments", &body.signature)
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?
        .is_some()
    {
        return Err(AppError::Conflict(
            "This Solana payment was already used".to_owned(),
        ));
    }

    const PLATFORM_TREASURY: &str = "2wZ2vKzRzY7ZxkRTRgTKVBDBVTqk1NfvGbQFgDxJAr9X";
    state
        .solana
        .verify_payment(
            &body.signature,
            &body.payer_address,
            seller_address,
            seller_lamports,
            PLATFORM_TREASURY,
            fee_lamports,
        )
        .await
        .map_err(|e| AppError::BadRequest(format!("Payment verification failed: {e}")))?;

    // ── a) Remove NFT from seller's wallet ────────────────────────────────────
    let mut seller_nfts = load_nfts(&state, &body.seller_id).await?;
    let pos = seller_nfts
        .iter()
        .position(|n| n.id == nft_id)
        .ok_or_else(|| AppError::NotFound(format!("NFT {} not found in seller wallet", nft_id)))?;
    let seller_nft = seller_nfts.remove(pos);
    save_nfts(&state, &body.seller_id, &seller_nfts).await?;

    // ── b) Fetch buyer's display name ─────────────────────────────────────────
    let buyer_doc = state
        .firestore
        .get("users", buyer_uid)
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?;
    let buyer_name = buyer_doc
        .as_ref()
        .and_then(|d| d.get("name").and_then(|v| v.as_str()))
        .unwrap_or("Unknown")
        .to_owned();

    // Build the new record with updated ownership.
    let new_id = Uuid::new_v4().to_string();
    let transferred = Nft {
        id:         new_id.clone(),
        owner_id:   buyer_uid.clone(),
        owner_name: buyer_name,
        for_sale:   false,
        price:      None,
        ..seller_nft
    };

    // ── c) Append to buyer's wallet ───────────────────────────────────────────
    let mut buyer_nfts = load_nfts(&state, buyer_uid).await?;
    buyer_nfts.push(transferred);
    save_nfts(&state, buyer_uid, &buyer_nfts).await?;

    // ── d) Remove the purchased item from the listing. Collection posts stay
    // live until their final edition is sold.
    let nft_title = post_doc
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("NFT")
        .to_owned();
    if collection_nft_ids.is_empty() {
        state
            .firestore
            .delete("posts", &body.post_id)
            .await
            .map_err(|e| AppError::Firebase(e.to_string()))?;
    } else {
        let purchased_index = collection_nft_ids
            .iter()
            .position(|id| id == &nft_id)
            .ok_or_else(|| AppError::BadRequest("Collection item not found".to_owned()))?;
        let mut remaining_ids = collection_nft_ids;
        remaining_ids.remove(purchased_index);

        let mut remaining_images: Vec<String> = post_doc
            .get("nftImages")
            .and_then(|v| v.as_array())
            .map(|images| {
                images
                    .iter()
                    .filter_map(|image| image.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();
        if purchased_index < remaining_images.len() {
            remaining_images.remove(purchased_index);
        }

        if remaining_ids.is_empty() {
            state
                .firestore
                .delete("posts", &body.post_id)
                .await
                .map_err(|e| AppError::Firebase(e.to_string()))?;
        } else {
            let updated_title = collection_title_after_sale(&nft_title, remaining_ids.len());
            let primary_image = remaining_images.first().cloned().unwrap_or_default();
            state
                .firestore
                .update(
                    "posts",
                    &body.post_id,
                    &json!({
                        "walletNftIds": remaining_ids,
                        "nftImages": remaining_images,
                        "nftImage": primary_image,
                        "title": updated_title,
                    }),
                )
                .await
                .map_err(|e| AppError::Firebase(e.to_string()))?;
        }
    }

    state
        .firestore
        .set(
            "processed_payments",
            &body.signature,
            &json!({
                "buyerId": buyer_uid,
                "sellerId": body.seller_id,
                "postId": body.post_id,
                "nftId": nft_id,
                "payerAddress": body.payer_address,
                "createdAt": Utc::now().to_rfc3339(),
            }),
        )
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?;

    state
        .firestore
        .update(
            "payment_quotes",
            &body.quote_id,
            &json!({
                "used": true,
                "signature": body.signature,
                "usedAt": Utc::now().to_rfc3339(),
            }),
        )
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?;

    // ── e) Notifications: buyer "you bought it", seller "you sold it" ─────────
    let buyer_display = state.firestore.get("users", buyer_uid).await.ok().flatten()
        .as_ref()
        .and_then(|d| d.get("name").and_then(|v| v.as_str()).map(str::to_owned))
        .unwrap_or_else(|| "Buyer".to_owned());

    crate::notification_helpers::notify_purchase(
        &state.firestore, buyer_uid, &nft_title, price, &currency,
    ).await;
    crate::notification_helpers::notify_sale(
        &state.firestore, &body.seller_id, &nft_title, price, &currency, &buyer_display,
    ).await;

    Ok(Json(json!({
        "success":  true,
        "newNftId": new_id,
    })))
}

#[cfg(test)]
mod tests {
    use super::render_qr_png;

    #[test]
    fn qr_is_png() {
        let png = render_qr_png("https://example.com/idenity/?nft=abc").unwrap();
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        let img = image::load_from_memory(&png).unwrap();
        assert!(img.width() >= 320);
    }
}
