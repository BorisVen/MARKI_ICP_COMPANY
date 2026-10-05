/// Convenience wrappers that create Notification documents in Firestore.
/// All functions are fire-and-forget — failures are logged but not propagated
/// to the caller so that a notification error never breaks the primary action.
use anyhow::Result;
use chrono::Utc;
use uuid::Uuid;

use crate::{
    models::{Notification, NotificationMeta, NotificationType},
    services::FirestoreClient,
};

async fn create(
    db: &FirestoreClient,
    user_id: &str,
    notification_type: NotificationType,
    title: String,
    text: String,
    metadata: Option<NotificationMeta>,
) -> Result<()> {
    let id = Uuid::new_v4().to_string();
    let notification = Notification {
        id: id.clone(),
        user_id: user_id.to_owned(),
        notification_type,
        title,
        text,
        read: false,
        created_at: Utc::now().to_rfc3339(),
        metadata,
    };
    let value = serde_json::to_value(&notification)?;
    db.set("notifications", &id, &value).await
}

pub async fn notify_purchase(
    db: &FirestoreClient,
    buyer_id: &str,
    nft_title: &str,
    price: f64,
    currency: &str,
) {
    let res = create(
        db,
        buyer_id,
        NotificationType::Purchase,
        "Purchase successful 🛒".to_owned(),
        format!("You bought \"{}\" for {} {}", nft_title, price, currency),
        Some(NotificationMeta {
            nft_title: Some(nft_title.to_owned()),
            price: Some(price),
            currency: Some(currency.to_owned()),
            ..Default::default()
        }),
    )
    .await;
    if let Err(e) = res {
        tracing::warn!(error = %e, "Failed to create purchase notification");
    }
}

pub async fn notify_sale(
    db: &FirestoreClient,
    seller_id: &str,
    nft_title: &str,
    price: f64,
    currency: &str,
    buyer_name: &str,
) {
    let res = create(
        db,
        seller_id,
        NotificationType::Sale,
        "Your NFT was sold 💰".to_owned(),
        format!(
            "\"{}\" was sold to {} for {} {}",
            nft_title, buyer_name, price, currency
        ),
        Some(NotificationMeta {
            nft_title: Some(nft_title.to_owned()),
            price: Some(price),
            currency: Some(currency.to_owned()),
            from_user: Some(buyer_name.to_owned()),
            ..Default::default()
        }),
    )
    .await;
    if let Err(e) = res {
        tracing::warn!(error = %e, "Failed to create sale notification");
    }
}

pub async fn notify_like(
    db: &FirestoreClient,
    owner_id: &str,
    nft_title: &str,
    from_user_name: &str,
) {
    let res = create(
        db,
        owner_id,
        NotificationType::Like,
        "Someone liked your NFT ❤️".to_owned(),
        format!("{} liked \"{}\"", from_user_name, nft_title),
        Some(NotificationMeta {
            nft_title: Some(nft_title.to_owned()),
            from_user: Some(from_user_name.to_owned()),
            ..Default::default()
        }),
    )
    .await;
    if let Err(e) = res {
        tracing::warn!(error = %e, "Failed to create like notification");
    }
}

pub async fn notify_comment(
    db: &FirestoreClient,
    owner_id: &str,
    nft_title: &str,
    from_user_name: &str,
) {
    let res = create(
        db,
        owner_id,
        NotificationType::Comment,
        "New comment on your NFT 💬".to_owned(),
        format!("{} commented on \"{}\"", from_user_name, nft_title),
        Some(NotificationMeta {
            nft_title: Some(nft_title.to_owned()),
            from_user: Some(from_user_name.to_owned()),
            ..Default::default()
        }),
    )
    .await;
    if let Err(e) = res {
        tracing::warn!(error = %e, "Failed to create comment notification");
    }
}

pub async fn notify_welcome(db: &FirestoreClient, user_id: &str, user_name: &str) {
    let res = create(
        db,
        user_id,
        NotificationType::Welcome,
        "Welcome to Idenity 🎉".to_owned(),
        format!(
            "Hi {}! Your wallet has been created. Start creating and trading NFTs.",
            user_name
        ),
        None,
    )
    .await;
    if let Err(e) = res {
        tracing::warn!(error = %e, "Failed to create welcome notification");
    }
}

pub async fn notify_nft_created(db: &FirestoreClient, user_id: &str, nft_title: &str) {
    let res = create(
        db,
        user_id,
        NotificationType::NftCreated,
        "NFT created 🎨".to_owned(),
        format!("Your NFT \"{}\" has been minted.", nft_title),
        Some(NotificationMeta {
            nft_title: Some(nft_title.to_owned()),
            ..Default::default()
        }),
    )
    .await;
    if let Err(e) = res {
        tracing::warn!(error = %e, "Failed to create nft_created notification");
    }
}

pub async fn notify_cod_buyer(
    db: &FirestoreClient,
    buyer_id: &str,
    nft_title: &str,
    price: f64,
    currency: &str,
    delivery_address: &str,
) {
    let res = create(
        db,
        buyer_id,
        NotificationType::Purchase,
        "Order placed 📦".to_owned(),
        format!(
            "Your order for \"{}\" ({} {}) has been placed. It will be delivered to: {}",
            nft_title, price, currency, delivery_address
        ),
        Some(NotificationMeta {
            nft_title: Some(nft_title.to_owned()),
            price: Some(price),
            currency: Some(currency.to_owned()),
            ..Default::default()
        }),
    )
    .await;
    if let Err(e) = res {
        tracing::warn!(error = %e, "Failed to create COD buyer notification");
    }
}

pub async fn notify_cod_seller(
    db: &FirestoreClient,
    seller_id: &str,
    order_id: &str,
    nft_title: &str,
    price: f64,
    currency: &str,
    buyer_name: &str,
    full_name: &str,
    phone: &str,
    delivery_address: &str,
) {
    let res = create(
        db,
        seller_id,
        NotificationType::Sale,
        "New COD order 🚚".to_owned(),
        format!(
            "{} ordered \"{}\" for {} {}.\nShip to: {} ({}), {}",
            buyer_name, nft_title, price, currency,
            full_name, phone, delivery_address
        ),
        Some(NotificationMeta {
            nft_title: Some(nft_title.to_owned()),
            price: Some(price),
            currency: Some(currency.to_owned()),
            from_user: Some(buyer_name.to_owned()),
            order_id: Some(order_id.to_owned()),
            delivery_address: Some(delivery_address.to_owned()),
            buyer_full_name: Some(full_name.to_owned()),
            buyer_phone: Some(phone.to_owned()),
            ..Default::default()
        }),
    )
    .await;
    if let Err(e) = res {
        tracing::warn!(error = %e, "Failed to create COD seller notification");
    }
}

pub async fn notify_batch_created(db: &FirestoreClient, user_id: &str, count: usize) {
    let res = create(
        db,
        user_id,
        NotificationType::NftCreated,
        format!("{} NFTs created 🎨", count),
        format!("Your batch of {} NFTs has been minted successfully.", count),
        Some(NotificationMeta {
            batch_count: Some(count),
            ..Default::default()
        }),
    )
    .await;
    if let Err(e) = res {
        tracing::warn!(error = %e, "Failed to create batch_created notification");
    }
}
