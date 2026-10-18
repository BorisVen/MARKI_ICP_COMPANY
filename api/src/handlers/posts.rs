use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};
use chrono::Utc;
use serde_json::{json, Value};
use std::sync::Arc;
use uuid::Uuid;

use crate::{
    errors::{ApiResult, AppError},
    middleware::auth::AuthenticatedUser,
    models::{AddCommentRequest, Comment, CreatePostRequest, Post},
    notification_helpers,
    services::firestore::QueryFilter,
    AppState,
};

/// `GET /api/posts`
/// Returns all feed posts sorted newest-first.
/// Add `?for_sale=true` query param to get marketplace listings only.
pub async fn get_posts(
    State(state): State<Arc<AppState>>,
    Extension(_auth): Extension<AuthenticatedUser>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> ApiResult<Json<Vec<Post>>> {
    let mut filters = vec![];
    if params.get("for_sale").map(|v| v == "true").unwrap_or(false) {
        filters.push(QueryFilter::equal("forSale", true));
    }

    let docs = state
        .firestore
        .query("posts", filters, Some(("createdAt", true)), None)
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?;

    let mut posts: Vec<Post> = docs
        .into_iter()
        .filter_map(|d| serde_json::from_value(d).ok())
        .collect();
    crate::handlers::names::refresh_posts(&state, &mut posts).await;

    Ok(Json(posts))
}

/// `POST /api/posts`
pub async fn create_post(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedUser>,
    Json(body): Json<CreatePostRequest>,
) -> ApiResult<(StatusCode, Json<Post>)> {
    // Fetch author's profile for display info.
    let user_doc = state
        .firestore
        .get("users", &auth.uid)
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?;
    let user_name = user_doc
        .as_ref()
        .and_then(|d| d.get("name"))
        .and_then(|v| v.as_str())
        .unwrap_or("Unknown")
        .to_owned();
    let user_avatar = user_doc
        .as_ref()
        .and_then(|d| d.get("avatar"))
        .and_then(|v| v.as_str())
        .map(str::to_owned);

    // Fetch the seller's primary connected Phantom wallet address so BuyModal
    // can execute the on-chain SOL transfer without an extra API round-trip.
    let seller_address: Option<String> = state
        .firestore
        .get("crypto_wallets", &auth.uid)
        .await
        .ok()
        .flatten()
        .and_then(|doc| {
            doc.get("wallets")
                .and_then(|v| v.as_array())
                .and_then(|arr| {
                    // Prefer the wallet explicitly marked as connected; fall
                    // back to the first entry if none is flagged yet.
                    arr.iter()
                        .find(|w| {
                            w.get("isConnected")
                                .and_then(|c| c.as_bool())
                                .unwrap_or(false)
                        })
                        .or_else(|| arr.first())
                        .and_then(|w| w.get("address").and_then(|a| a.as_str()).map(str::to_owned))
                })
        });

    // For collection posts the caller may omit nft_image and rely on nft_images[0].
    let primary_image = body.nft_image
        .or_else(|| body.nft_images.first().cloned())
        .unwrap_or_default();

    let post = Post {
        id: String::new(), // will be set after Firestore assigns ID
        user_id: auth.uid.clone(),
        user_name,
        user_avatar,
        nft_image: primary_image,
        title: body.title,
        description: body.description,
        tags: body.tags,
        likes: 0,
        reposts: 0,
        liked_by: vec![],
        comments: vec![],
        created_at: Utc::now().to_rfc3339(),
        for_sale: body.for_sale,
        price: body.price,
        currency: body.currency,
        wallet_nft_id: body.wallet_nft_id,
        nft_images: body.nft_images,
        wallet_nft_ids: body.wallet_nft_ids,
        seller_address,
    };

    let mut value = serde_json::to_value(&post).map_err(|e| AppError::Internal(e.into()))?;

    let id = state
        .firestore
        .create("posts", &value)
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?;

    // Inject the generated ID into the returned object.
    value["id"] = json!(id);
    let mut post = post;
    post.id = id;

    Ok((StatusCode::CREATED, Json(post)))
}

/// `DELETE /api/posts/:id`
pub async fn delete_post(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedUser>,
    Path(post_id): Path<String>,
) -> ApiResult<StatusCode> {
    let doc = state
        .firestore
        .get("posts", &post_id)
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?
        .ok_or_else(|| AppError::NotFound(format!("Post {} not found", post_id)))?;

    let owner_id = doc
        .get("userId")
        .and_then(|v| v.as_str())
        .unwrap_or_default();

    if owner_id != auth.uid {
        return Err(AppError::Forbidden("Cannot delete another user's post".to_owned()));
    }

    state
        .firestore
        .delete("posts", &post_id)
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?;

    Ok(StatusCode::NO_CONTENT)
}

/// `POST /api/posts/:id/like`
/// Toggles the authenticated user's like on a post (add if not present, remove if present).
pub async fn like_post(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedUser>,
    Path(post_id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let doc = state
        .firestore
        .get("posts", &post_id)
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?
        .ok_or_else(|| AppError::NotFound(format!("Post {} not found", post_id)))?;

    let post: Post =
        serde_json::from_value(doc.clone()).map_err(|e| AppError::Internal(e.into()))?;

    let (new_likes, new_liked_by, is_like) = if post.liked_by.contains(&auth.uid) {
        // Unlike
        let liked_by: Vec<String> = post
            .liked_by
            .into_iter()
            .filter(|id| id != &auth.uid)
            .collect();
        let count = liked_by.len() as i64;
        (count, liked_by, false)
    } else {
        // Like
        let mut liked_by = post.liked_by;
        liked_by.push(auth.uid.clone());
        let count = liked_by.len() as i64;

        // Notify the post owner (non-blocking, skip if liker == owner).
        if post.user_id != auth.uid {
            let liker_name = state
                .firestore
                .get("users", &auth.uid)
                .await
                .ok()
                .flatten()
                .and_then(|d| d.get("name").and_then(|v| v.as_str()).map(str::to_owned))
                .unwrap_or_else(|| "Someone".to_owned());

            notification_helpers::notify_like(
                &state.firestore,
                &post.user_id,
                &auth.uid,
                &post_id,
                &post.title,
                &liker_name,
            )
            .await;
        }

        (count, liked_by, true)
    };

    state
        .firestore
        .update(
            "posts",
            &post_id,
            &json!({ "likes": new_likes, "likedBy": new_liked_by }),
        )
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?;

    Ok(Json(json!({ "likes": new_likes, "liked": is_like })))
}

/// `POST /api/posts/:id/comments`
pub async fn add_comment(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedUser>,
    Path(post_id): Path<String>,
    Json(body): Json<AddCommentRequest>,
) -> ApiResult<Json<Comment>> {
    let body_text = body.text.trim().to_owned();
    if body_text.is_empty() {
        return Err(AppError::BadRequest("Comment text cannot be empty".to_owned()));
    }
    if body_text.chars().count() > 500 {
        return Err(AppError::BadRequest("Коментар задовгий: до 500 символів".to_owned()));
    }

    let doc = state
        .firestore
        .get("posts", &post_id)
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?
        .ok_or_else(|| AppError::NotFound(format!("Post {} not found", post_id)))?;

    let post: Post =
        serde_json::from_value(doc).map_err(|e| AppError::Internal(e.into()))?;

    // Fetch commenter info.
    let user_doc = state
        .firestore
        .get("users", &auth.uid)
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?;
    let user_name = user_doc
        .as_ref()
        .and_then(|d| d.get("name").and_then(|v| v.as_str()))
        .unwrap_or("Unknown")
        .to_owned();
    let user_avatar = user_doc
        .as_ref()
        .and_then(|d| d.get("avatar").and_then(|v| v.as_str()))
        .map(str::to_owned);

    let comment = Comment {
        id: Uuid::new_v4().to_string(),
        user_id: auth.uid.clone(),
        user_name: user_name.clone(),
        user_avatar,
        text: body_text,
        created_at: Utc::now().to_rfc3339(),
    };

    let mut comments = post.comments;
    comments.push(comment.clone());

    state
        .firestore
        .update("posts", &post_id, &json!({ "comments": comments }))
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?;

    // Notify post owner (non-blocking).
    if post.user_id != auth.uid {
        notification_helpers::notify_comment(
            &state.firestore,
            &post.user_id,
            &post_id,
            &post.title,
            &user_name,
            &comment.text,
        )
        .await;
    }

    Ok(Json(comment))
}

// ── Discover: real NFTs of everyone, beside the posts ────────────────────────
//
// The market only had posts, so it looked empty. Every minted NFT is already
// public on its owner's profile; here they come grouped by collection (one
// card for "Water ×12"), newest first, without the ones a post already shows.

static DISCOVER: std::sync::LazyLock<std::sync::Mutex<Option<(std::time::Instant, Vec<Value>)>>> =
    std::sync::LazyLock::new(Default::default);

/// `GET /api/discover` — collections of minted NFTs, shaped like posts (`discover: true`).
pub async fn discover(
    State(state): State<Arc<AppState>>,
    Extension(_auth): Extension<AuthenticatedUser>,
) -> ApiResult<Json<Vec<Value>>> {
    if let Some((at, items)) = DISCOVER.lock().unwrap().clone() {
        if at.elapsed() < std::time::Duration::from_secs(60) {
            return Ok(Json(items));
        }
    }
    let fs = |e: anyhow::Error| AppError::Firebase(e.to_string());
    let in_posts: std::collections::HashSet<String> = state.firestore.query("posts", vec![], None, None).await.map_err(fs)?
        .iter()
        .flat_map(|p| {
            let mut ids: Vec<String> = p["walletNftIds"].as_array().into_iter().flatten().filter_map(|v| v.as_str().map(str::to_owned)).collect();
            ids.extend(p["walletNftId"].as_str().map(str::to_owned));
            ids
        })
        .collect();

    let mut groups: Vec<(String, Vec<crate::models::Nft>)> = vec![];
    for wallet in state.firestore.query("marki_wallets", vec![], None, None).await.map_err(fs)? {
        let uid = wallet["userId"].as_str().or(wallet["id"].as_str()).unwrap_or_default().to_owned();
        if uid.is_empty() { continue; }
        let nfts = crate::handlers::nfts::nfts_in_wallet(state.as_ref(), &uid, &wallet).await.unwrap_or_default();
        let mut by_collection: std::collections::BTreeMap<String, Vec<crate::models::Nft>> = Default::default();
        for n in nfts.into_iter().filter(|n| n.mint_address.as_deref().is_some_and(|m| !m.is_empty()) && !n.image.is_empty()) {
            let key = format!("{}|{}", n.batch_name.clone().unwrap_or_else(|| n.title.clone()), n.image);
            by_collection.entry(key).or_default().push(n);
        }
        for (_, list) in by_collection {
            if list.iter().any(|n| in_posts.contains(&n.id)) { continue; }
            groups.push((uid.clone(), list));
        }
    }

    let people = crate::handlers::names::lookup(&state, groups.iter().map(|(u, _)| u.clone()).collect()).await;
    let mut items: Vec<Value> = groups.into_iter().map(|(uid, list)| {
        let first = &list[0];
        let who = people.get(&uid);
        let title = first.batch_name.clone().filter(|b| !b.is_empty()).unwrap_or_else(|| first.title.clone());
        json!({
            "id": format!("nft-{}", first.id),
            "discover": true,
            "count": list.len(),
            "userId": uid,
            "userName": who.map(|p| p.name.clone()).unwrap_or_else(|| first.owner_name.clone()),
            "userAvatar": who.and_then(|p| p.avatar.clone()),
            "title": title,
            "description": first.description,
            "nftImage": first.image,
            "nftImages": [],
            "walletNftId": first.id,
            "walletNftIds": list.iter().take(50).map(|n| n.id.clone()).collect::<Vec<_>>(),
            "createdAt": list.iter().map(|n| n.created_at.clone()).max().unwrap_or_default(),
            "forSale": false,
            "likes": 0, "likedBy": [], "reposts": 0, "comments": [], "tags": first.tags,
        })
    }).collect();
    items.sort_by(|a, b| b["createdAt"].as_str().cmp(&a["createdAt"].as_str()));
    *DISCOVER.lock().unwrap() = Some((std::time::Instant::now(), items.clone()));
    Ok(Json(items))
}
