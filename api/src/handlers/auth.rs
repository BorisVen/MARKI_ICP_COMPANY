use axum::{extract::State, http::StatusCode, Extension, Json};
use chrono::Utc;
use std::{collections::HashMap, sync::Arc};

use crate::{
    errors::{ApiResult, AppError},
    middleware::auth::AuthenticatedUser,
    models::{MarkiWallet, RegisterRequest, UserData},
    notification_helpers,
    AppState,
};

// ── Word list for recovery phrase (256 BIP-39-style words) ───────────────────

const WORDS: &[&str] = &[
    "abandon","ability","able","about","above","absent","absorb","abstract",
    "absurd","abuse","access","accident","account","accuse","achieve","acid",
    "acoustic","acquire","across","act","action","actor","actress","actual",
    "adapt","add","addict","address","adjust","admit","adult","advance",
    "advice","aerobic","afford","afraid","again","age","agent","agree",
    "ahead","aim","air","airport","aisle","alarm","album","alcohol",
    "alert","alien","all","alley","allow","almost","alone","alpha",
    "already","also","alter","always","amateur","amazing","among","amount",
    "amused","analyst","anchor","ancient","anger","angle","angry","animal",
    "ankle","announce","annual","another","answer","antenna","antique","anxiety",
    "any","apart","apology","appear","apple","approve","april","arch",
    "arctic","area","arena","argue","arm","armor","army","around",
    "arrange","arrest","arrive","arrow","art","artefact","artist","artwork",
    "ask","aspect","assault","asset","assist","assume","asthma","athlete",
    "atom","attack","attend","attitude","attract","auction","audit","august",
    "aunt","author","auto","autumn","average","avocado","avoid","awake",
    "aware","away","awesome","awful","awkward","axis","baby","balance",
    "bamboo","banana","banner","barely","bargain","barrel","base","basic",
    "basket","battle","beach","beauty","because","become","beef","before",
    "begin","behave","behind","believe","below","belt","bench","benefit",
    "best","betray","better","between","beyond","bicycle","bid","bike",
    "bind","biology","bird","birth","bitter","black","blade","blame",
    "blanket","blast","bleak","bless","blind","blood","blossom","blouse",
    "blue","blur","blush","board","boat","body","boil","bomb",
    "bone","book","boost","border","boring","borrow","boss","bottom",
    "bounce","box","boy","bracket","brain","brand","brave","bread",
    "breeze","brick","bridge","brief","bright","bring","brisk","broccoli",
    "broken","bronze","broom","brother","brown","brush","bubble","buddy",
    "budget","buffalo","build","bulb","bulk","bullet","bundle","bunker",
    "burden","burger","burst","bus","business","busy","butter","buyer",
    "buzz","cabbage","cabin","cable","cactus","cage","cake","call",
    "calm","camera","camp","canal","cancel","candy","cannon","canvas",
    "canyon","capable","capital","captain","carbon","card","cargo","carpet",
    "carry","cart","case","cash","castle","casual","catalog","catch",
    "category","cattle","caught","cause","caution","cave","ceiling","celery",
    "cement","census","century","cereal","certain","chair","chalk","champion",
    "change","chaos","chapter","charge","chase","chat","cheap","check",
    "cheese","chef","cherry","chest","chicken","chief","child","chimney",
    "choice","choose","chronic","chuckle","chunk","cigar","cinema","circle",
    "citizen","city","civil","claim","clap","clarify","claw","clay",
];

// ── Helpers ───────────────────────────────────────────────────────────────────

fn generate_wallet_address() -> String {
    let a = uuid::Uuid::new_v4().simple().to_string();
    let b = uuid::Uuid::new_v4().simple().to_string();
    format!("0x{}", &format!("{}{}", a, b)[..40])
}

fn generate_recovery_phrase() -> String {
    uuid::Uuid::new_v4()
        .as_bytes()
        .iter()
        .take(12)
        .map(|&b| WORDS[b as usize])
        .collect::<Vec<_>>()
        .join(" ")
}

// ── Handlers ──────────────────────────────────────────────────────────────────

/// `POST /api/auth/register`
/// Called immediately after Firebase Auth signup to persist the user profile
/// and create the Marki wallet in Firestore.
pub async fn register(
    State(state): State<Arc<AppState>>,
    Json(body): Json<RegisterRequest>,
) -> ApiResult<(StatusCode, Json<UserData>)> {
    // Idempotent: if user already exists return them.
    if let Some(existing) = state.firestore.get("users", &body.uid).await? {
        let user: UserData = serde_json::from_value(existing)
            .map_err(|e| AppError::Internal(e.into()))?;
        return Ok((StatusCode::OK, Json(user)));
    }

    let now = Utc::now().to_rfc3339();

    let user = UserData {
        uid: body.uid.clone(),
        name: body.name.clone(),
        username: body.username.clone(),
        email: body.email.clone(),
        phone: body.phone.clone(),
        avatar: None,
        location: None,
        bio: None,
        created_at: now.clone(),
        company_approved: false,
        delivery_address: None,
        pending_approval: false,
        roles: Vec::new(),
        company_id: None,
        rating_overall: 0.0,
        rating_by_category: std::collections::HashMap::new(),
        followers_count: 0,
        following_count: 0,
        likes_count: 0,
        saved_posts: Vec::new(),
        reposted_posts: Vec::new(),
        banned: false,
        company_name: None,
        registration_number: None,
        contact_email: None,
        business_description: None,
        approval_requested_at: None,
        reviewed_at: None,
        reviewed_by: None,
        approval_status: None,
        rejection_reason: None,
        ban_reason: None,
    };

    let user_value = serde_json::to_value(&user).map_err(|e| AppError::Internal(e.into()))?;
    state
        .firestore
        .set("users", &body.uid, &user_value)
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?;

    // Create Marki wallet.
    let mut balance = HashMap::new();
    balance.insert("ICP".to_owned(), 0.0_f64);
    balance.insert("POLYGON".to_owned(), 0.0_f64);
    balance.insert("SOLANA".to_owned(), 0.0_f64);

    let wallet = MarkiWallet {
        address: generate_wallet_address(),
        recovery_phrase: generate_recovery_phrase(),
        balance,
        fingerprint_enabled: false,
    };
    let wallet_value =
        serde_json::to_value(&wallet).map_err(|e| AppError::Internal(e.into()))?;
    state
        .firestore
        .set("wallets", &body.uid, &wallet_value)
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?;

    // Bootstrap empty NFT and crypto-wallet documents.
    state
        .firestore
        .set(
            "marki_wallets",
            &body.uid,
            &serde_json::json!({ "userId": body.uid, "nfts": [], "createdAt": now }),
        )
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?;

    state
        .firestore
        .set(
            "crypto_wallets",
            &body.uid,
            &serde_json::json!({ "userId": body.uid, "wallets": [], "createdAt": now }),
        )
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?;

    // Send welcome notification (non-blocking).
    notification_helpers::notify_welcome(&state.firestore, &body.uid, &body.name).await;

    Ok((StatusCode::CREATED, Json(user)))
}

/// `GET /api/auth/me`
pub async fn me(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUser>,
) -> ApiResult<Json<UserData>> {
    let doc = state
        .firestore
        .get("users", &user.uid)
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?
        .ok_or_else(|| AppError::NotFound("User profile not found".to_owned()))?;

    let mut user_data: UserData =
        serde_json::from_value(doc).map_err(|e| AppError::Internal(e.into()))?;

    user_data.likes_count = crate::handlers::users::likes_count_for_user(&state, &user.uid).await?;

    Ok(Json(user_data))
}
