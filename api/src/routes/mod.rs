use axum::{
    extract::DefaultBodyLimit,
    middleware,
    routing::{delete, get, post, put},
    Router,
};
use std::sync::Arc;

use crate::{
    handlers::{admin, ai, auth, cod_orders, billing, deliveries, gift_requests, onchain, gifts, goods, marketplace, nfc, notifications, nfts, posts, profile, public, users, wallets},
    middleware::auth::auth_middleware,
    AppState,
};

/// Build the full `/api` router.
/// Public routes (no auth required): `POST /api/auth/register`
/// All other routes require a valid Firebase Bearer token.
pub fn api_router(state: Arc<AppState>) -> Router<Arc<AppState>> {
    // ── Protected routes ──────────────────────────────────────────────────────
    let protected = Router::new()
        // Auth
        .route("/auth/me", get(auth::me))
        // Profile
        .route("/profile/:uid", get(profile::get_profile))
        .route("/profile/:uid", put(profile::update_profile))
        .route("/profile/:uid/avatar", post(profile::upload_avatar)
            .layer(DefaultBodyLimit::max(5 * 1024 * 1024)))   // 5 MB for avatar
        .route("/profile/:uid/password", put(profile::change_password))
        .route("/profile/:uid/approval", put(profile::set_approval))
        .route("/profile/:uid/request-approval", post(profile::request_approval))
        // Users
        .route("/users/search", get(users::search_users))
        .route("/users/me/saved", get(users::saved_posts))
        .route("/users/me/reposts", get(users::reposted_posts))
        .route("/users/me/likes", get(users::liked_posts))
        .route("/users/me/analytics", get(public::analytics))
        .route("/users/:uid/follow", post(users::follow_user))
        .route("/users/:uid/follow", delete(users::unfollow_user))
        .route("/users/:uid", get(users::get_public_profile))
        // Marki wallet
        .route("/wallets/marki", get(wallets::get_marki_wallet))
        .route("/wallets/marki/fingerprint", put(wallets::update_fingerprint))
        .route("/wallets/marki/email", put(wallets::update_email))
        // Crypto wallets (Phantom only)
        .route("/wallets/crypto", get(wallets::get_crypto_wallets))
        .route("/wallets/crypto", post(wallets::add_crypto_wallet))
        .route("/wallets/crypto/:id", delete(wallets::remove_crypto_wallet))
        .route("/wallets/crypto/:id/balance", put(wallets::refresh_wallet_balance))
        // NFTs — isolated in their own sub-router so matchit's radix tree
        // resolves static segments (/mint-info, /batch) in a self-contained
        // scope and can never conflate them with the dynamic /:id parameter.
        .nest("/nfts", Router::new()
            .route("/",          get(nfts::get_nfts))
            .route("/gift-collection", post(nfts::set_gift_collection))
            .route("/",          post(nfts::create_nft)
                .layer(DefaultBodyLimit::max(20 * 1024 * 1024)))   // 20 MB per NFT
            .route("/mint-info", get(nfts::get_mint_info))
            .route("/batch",     post(nfts::batch_create_nfts)
                .layer(DefaultBodyLimit::max(100 * 1024 * 1024)))  // 100 MB for batch
            .route("/editions",  post(nfts::create_edition_nfts)   // static — must stay above /:id
                .layer(DefaultBodyLimit::max(20 * 1024 * 1024)))   // 20 MB image
            .route("/verify",       post(nfts::verify_nft))
            .route("/qr-missing",   post(nfts::generate_missing_qr))    // static — above /:id
            .route("/regenerate-qr", post(nfts::regenerate_all_qr))
            .route("/:id",          get(nfts::get_nft))
            .route("/:id",          put(nfts::update_nft))
            .route("/:id",          delete(nfts::delete_nft))
            .route("/:id/payment-quote", post(nfts::create_payment_quote))
            .route("/:id/transfer", post(nfts::transfer_nft))
            .route("/:id/qr",       post(nfts::generate_nft_qr))
        )
        // Posts / feed
        .route("/posts", get(posts::get_posts))
        .route("/posts", post(posts::create_post))
        .route("/posts/:id", delete(posts::delete_post))
        .route("/posts/:id/like", post(posts::like_post))
        .route("/posts/:id/save", post(users::toggle_saved_post))
        .route("/posts/:id/repost", post(users::toggle_reposted_post))
        .route("/posts/:id/comments", post(posts::add_comment))
        // Gifts
        .route("/gifts", get(gifts::my_gifts))
        .route("/gifts", post(gifts::create_gift))
        .route("/gifts/split", post(gifts::create_gift_group))
        .route("/me/icp-principal", put(onchain::link_principal))
        .route("/me/icp-wallet", get(onchain::wallet_seed))
        .route("/me/push-token", put(onchain::register_push_token))
        .route("/onchain/pending", get(onchain::pending))
        .route("/billing/checkout", post(billing::checkout))
        .route("/billing/orders", get(billing::my_orders))
        .route("/onchain/account", get(onchain::account))
        .route("/onchain/withdraw", post(onchain::withdraw))
        .route("/gift-requests", get(gift_requests::mine).post(gift_requests::create))
        .route("/gift-requests/:id/approve", post(gift_requests::approve))
        .route("/gift-requests/:id/decline", post(gift_requests::decline))
        .route("/gifts/:code", delete(gifts::cancel_gift))
        .route("/gifts/:code/claim", post(gifts::claim_gift))
        // Goods authenticity (products → batches → units)
        .route("/goods/products", get(goods::my_products).post(goods::create_product))
        .route("/goods/products/:id/batches", post(goods::create_batch))
        .route("/goods/batches/:id", get(goods::get_batch))
        .route("/goods/batches/:id/events", post(goods::add_event))
        .route("/goods/verify/:code", get(goods::verify))
        .route("/goods/verify/:code/claim", post(goods::claim))
        .route("/goods/verify/:code/transfer", post(goods::create_transfer).delete(goods::cancel_transfer))
        .route("/goods/verify/:code/accept", post(goods::accept_transfer))
        .route("/goods/batches/:id/units/:serial/flag", delete(goods::clear_flag))
        .route("/goods/mine", get(goods::my_goods))
        // Marketplace
        .route("/marketplace/buy", post(marketplace::buy_nft))
        .route("/marketplace/cod", post(marketplace::buy_nft_cod))
        // COD orders — CRM inbox
        .route("/cod-orders",            get(cod_orders::list_orders))
        .route("/cod-orders/:id/accept", post(cod_orders::accept_order))
        // Deliveries — CRM
        .route("/deliveries", get(deliveries::list_deliveries))
        .route("/deliveries", post(deliveries::create_delivery))
        .route("/deliveries/:id", get(deliveries::get_delivery))
        .route("/deliveries/:id/carrier",         put(deliveries::update_carrier))
        .route("/deliveries/:id/status",          put(deliveries::update_status))
        .route("/deliveries/:id/checkpoints",     post(deliveries::add_checkpoint))
        .route("/deliveries/:id/sync-novaposhta", post(deliveries::sync_novaposhta))
        .route("/deliveries/:id/confirm-receipt", post(deliveries::confirm_receipt))
        // NFC binding & verification
        .route("/nfc/bind",   post(nfc::bind_nfc))
        .route("/nfc/verify", post(nfc::verify_nfc))
        // Notifications
        .route("/notifications", get(notifications::get_notifications))
        .route("/notifications/read-all", put(notifications::mark_all_read))
        .route("/notifications/:id/read", put(notifications::mark_read))
        .route("/notifications/:id", delete(notifications::delete_notification))
        // Admin — gated inside each handler via ADMIN_UIDS check
        .route("/admin/me",                       get(admin::me))
        .route("/admin/companies",                get(admin::list_companies))
        .route("/admin/companies/:uid",           get(admin::get_company))
        .route("/admin/companies/:uid/approve",   post(admin::approve))
        .route("/admin/companies/:uid/reject",    post(admin::reject))
        .route("/admin/companies/:uid/ban",       post(admin::ban))
        .route("/admin/companies/:uid/unban",     post(admin::unban))
        // Attach auth middleware to ALL routes in this sub-router.
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ));

    // ── Public routes (no auth required) ─────────────────────────────────────
    let public = Router::new()
        .route("/auth/register", post(auth::register))
        .route("/public/users/:username", get(public::get_profile))
        .route("/public/users/:username/nfts", get(public::get_user_nfts))
        .route("/public/users/:username/view", post(public::view_profile))
        .route("/public/nfts/:owner_id/:nft_id", get(public::get_passport))
        .route("/public/nfts/:owner_id/:nft_id/view", post(public::view_passport))
        .route("/public/gifts/:code", get(gifts::preview_gift))
        .route("/public/goods/:code", get(goods::public_verify))
        .route("/public/billing/wayforpay", post(billing::wayforpay_notice))
        .route("/public/ii-relay/:key", get(onchain::ii_relay_take).post(onchain::ii_relay_put))
        .route("/public/billing/return/:id", get(billing::back_to_app).post(billing::back_to_app))
        .route("/public/billing/pay/:id", get(billing::pay_page))
        .route("/public/billing/price", get(billing::current_price))
        .route("/public/goods/:code/locate", post(goods::locate))
        .route("/ai/generate",   post(ai::generate_image));

    Router::new().merge(protected).merge(public)
}
