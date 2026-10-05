use axum::{
    extract::{Path, Query, State},
    Extension, Json,
};
use serde_json::{json, Value};
use std::sync::Arc;

use crate::{
    errors::{ApiResult, AppError},
    middleware::auth::AuthenticatedUser,
    models::{Post, PublicProfile, PublicUser, UserSearchQuery},
    services::firestore::QueryFilter,
    AppState,
};

/// How many user documents one search scans. Firestore has no text search, so
/// matching happens in memory; raise this (or move to a search index) as users grow.
const SCAN_LIMIT: i32 = 1000;
const MAX_RESULTS: usize = 20;

fn text(doc: &Value, key: &str) -> String {
    doc.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

/// Pure toggle helper, kept independent of Firestore for predictable behavior.
pub fn toggle_id(items: &mut Vec<String>, id: &str) -> bool {
    if items.iter().any(|item| item == id) {
        items.retain(|item| item != id);
        false
    } else {
        items.push(id.to_owned());
        true
    }
}

pub async fn likes_count_for_user(state: &AppState, uid: &str) -> ApiResult<u64> {
    let docs = state
        .firestore
        .query("posts", vec![QueryFilter::equal("userId", uid)], None, None)
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?;
    Ok(docs
        .iter()
        .map(|d| d.get("likes").and_then(Value::as_i64).unwrap_or(0).max(0) as u64)
        .sum())
}

fn user_text(doc: &Value, field: &str) -> String {
    text(doc, field)
}

fn user_opt_text(doc: &Value, field: &str) -> Option<String> {
    opt_text(doc, field)
}

pub async fn follow_user(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedUser>,
    Path(uid): Path<String>,
) -> ApiResult<Json<Value>> {
    set_following(state, auth, uid, true).await
}

pub async fn unfollow_user(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedUser>,
    Path(uid): Path<String>,
) -> ApiResult<Json<Value>> {
    set_following(state, auth, uid, false).await
}

async fn set_following(
    state: Arc<AppState>,
    auth: AuthenticatedUser,
    target: String,
    should_follow: bool,
) -> ApiResult<Json<Value>> {
    if target == auth.uid && should_follow {
        return Err(AppError::BadRequest("Cannot follow yourself".into()));
    }
    let caller = state
        .firestore
        .get("users", &auth.uid)
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?
        .ok_or_else(|| AppError::NotFound("User profile not found".into()))?;
    let target_doc = state
        .firestore
        .get("users", &target)
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?
        .ok_or_else(|| AppError::NotFound("User profile not found".into()))?;
    let mut following: Vec<String> = caller
        .get("following")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect();
    let currently = following.iter().any(|uid| uid == &target);
    if should_follow && !currently {
        following.push(target.clone());
    }
    if !should_follow && currently {
        following.retain(|uid| uid != &target);
    }
    let followers = target_doc
        .get("followersCount")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let changed = currently != should_follow;
    let new_followers = if changed {
        if should_follow { followers.saturating_add(1) } else { followers.saturating_sub(1) }
    } else {
        followers
    };
    state
        .firestore
        .update(
            "users",
            &auth.uid,
            &json!({"following": following, "followingCount": following.len()}),
        )
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?;
    state
        .firestore
        .update("users", &target, &json!({"followersCount": new_followers}))
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?;
    Ok(Json(
        json!({"following": should_follow, "followersCount": new_followers, "followingCount": following.len()}),
    ))
}

pub async fn get_public_profile(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedUser>,
    Path(uid): Path<String>,
) -> ApiResult<Json<PublicProfile>> {
    let doc = state
        .firestore
        .get("users", &uid)
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?
        .ok_or_else(|| AppError::NotFound("User profile not found".into()))?;
    let followers = doc
        .get("followersCount")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let following = doc
        .get("followingCount")
        .and_then(Value::as_u64)
        .unwrap_or_else(|| {
            doc.get("following")
                .and_then(Value::as_array)
                .map(|a| a.len() as u64)
                .unwrap_or(0)
        });
    let caller_follows = state
        .firestore
        .get("users", &auth.uid)
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?
        .and_then(|d| d.get("following").and_then(Value::as_array).cloned())
        .unwrap_or_default()
        .iter()
        .any(|v| v.as_str() == Some(uid.as_str()));
    Ok(Json(PublicProfile {
        uid: user_text(&doc, "uid").if_empty_then(uid.clone()),
        name: user_text(&doc, "name"),
        username: user_text(&doc, "username"),
        avatar: user_opt_text(&doc, "avatar"),
        bio: user_opt_text(&doc, "bio"),
        company_approved: doc
            .get("companyApproved")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        followers_count: followers,
        following_count: following,
        likes_count: likes_count_for_user(&state, &uid).await?,
        is_following: caller_follows,
    }))
}

trait EmptyFallback {
    fn if_empty_then(self, fallback: String) -> String;
}
impl EmptyFallback for String {
    fn if_empty_then(self, fallback: String) -> String {
        if self.is_empty() {
            fallback
        } else {
            self
        }
    }
}

async fn profile_tab(state: Arc<AppState>, ids: Vec<String>) -> ApiResult<Json<Vec<Post>>> {
    if ids.is_empty() {
        return Ok(Json(vec![]));
    }
    let docs = state
        .firestore
        .query("posts", vec![], Some(("createdAt", true)), None)
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?;
    let wanted: std::collections::HashSet<String> = ids.into_iter().collect();
    Ok(Json(
        docs.into_iter()
            .filter_map(|d| serde_json::from_value::<Post>(d).ok())
            .filter(|p| wanted.contains(&p.id))
            .collect(),
    ))
}

async fn current_user_ids(state: &AppState, uid: &str, field: &str) -> ApiResult<Vec<String>> {
    let doc = state
        .firestore
        .get("users", uid)
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?
        .ok_or_else(|| AppError::NotFound("User profile not found".into()))?;
    Ok(doc
        .get(field)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect())
}

pub async fn saved_posts(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedUser>,
) -> ApiResult<Json<Vec<Post>>> {
    let ids = current_user_ids(&state, &auth.uid, "savedPosts").await?;
    profile_tab(state, ids).await
}

pub async fn reposted_posts(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedUser>,
) -> ApiResult<Json<Vec<Post>>> {
    let ids = current_user_ids(&state, &auth.uid, "repostedPosts").await?;
    profile_tab(state, ids).await
}

pub async fn liked_posts(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedUser>,
) -> ApiResult<Json<Vec<Post>>> {
    let docs = state
        .firestore
        .query(
            "posts",
            vec![QueryFilter::array_contains("likedBy", auth.uid.clone())],
            Some(("createdAt", true)),
            None,
        )
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?;
    Ok(Json(
        docs.into_iter()
            .filter_map(|d| serde_json::from_value(d).ok())
            .collect(),
    ))
}

pub async fn toggle_saved_post(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedUser>,
    Path(post_id): Path<String>,
) -> ApiResult<Json<Value>> {
    if state
        .firestore
        .get("posts", &post_id)
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?
        .is_none()
    {
        return Err(AppError::NotFound(format!("Post {} not found", post_id)));
    }
    let doc = state
        .firestore
        .get("users", &auth.uid)
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?
        .ok_or_else(|| AppError::NotFound("User profile not found".into()))?;
    let mut ids: Vec<String> = doc
        .get("savedPosts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect();
    let saved = toggle_id(&mut ids, &post_id);
    state
        .firestore
        .update("users", &auth.uid, &json!({"savedPosts": ids}))
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?;
    Ok(Json(json!({"saved": saved})))
}

pub async fn toggle_reposted_post(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedUser>,
    Path(post_id): Path<String>,
) -> ApiResult<Json<Value>> {
    let post = state
        .firestore
        .get("posts", &post_id)
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?
        .ok_or_else(|| AppError::NotFound(format!("Post {} not found", post_id)))?;
    let user = state
        .firestore
        .get("users", &auth.uid)
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?
        .ok_or_else(|| AppError::NotFound("User profile not found".into()))?;
    let mut ids: Vec<String> = user
        .get("repostedPosts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect();
    let reposted = toggle_id(&mut ids, &post_id);
    let old_count = post
        .get("reposts")
        .and_then(Value::as_i64)
        .unwrap_or(0)
        .max(0);
    let count = if reposted {
        old_count + 1
    } else {
        old_count.saturating_sub(1)
    };
    state
        .firestore
        .update("users", &auth.uid, &json!({"repostedPosts": ids}))
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?;
    state
        .firestore
        .update("posts", &post_id, &json!({"reposts": count}))
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?;
    Ok(Json(json!({"reposted": reposted, "reposts": count})))
}

#[cfg(test)]
mod tests {
    use super::toggle_id;
    #[test]
    fn follow_toggle_is_idempotent_and_reversible() {
        let mut ids = vec![];
        assert!(toggle_id(&mut ids, "u1"));
        assert_eq!(ids, ["u1"]);
        assert!(!toggle_id(&mut ids, "u1"));
        assert!(ids.is_empty());
    }
}

fn opt_text(doc: &Value, key: &str) -> Option<String> {
    doc.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

/// `GET /api/users/search?q=` — find users by name or username (case-insensitive,
/// substring). A leading `@` is ignored. Email is never matched or returned.
pub async fn search_users(
    State(state): State<Arc<AppState>>,
    Query(query): Query<UserSearchQuery>,
) -> ApiResult<Json<Vec<PublicUser>>> {
    let q = query.q.trim().trim_start_matches('@').to_lowercase();
    if q.chars().count() < 2 {
        return Err(AppError::BadRequest(
            "Query must be at least 2 characters".into(),
        ));
    }

    let docs = state
        .firestore
        .query("users", vec![], None, Some(SCAN_LIMIT))
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?;

    let mut users: Vec<PublicUser> = docs
        .iter()
        .filter(|d| !d.get("banned").and_then(Value::as_bool).unwrap_or(false))
        .filter_map(|d| {
            let name = text(d, "name");
            let username = text(d, "username");
            let hit = name.to_lowercase().contains(&q) || username.to_lowercase().contains(&q);
            let uid = text(d, "uid");
            (hit && !uid.is_empty()).then(|| PublicUser {
                uid,
                name,
                username,
                avatar: opt_text(d, "avatar"),
                bio: opt_text(d, "bio"),
                company_approved: d
                    .get("companyApproved")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            })
        })
        .collect();

    // Exact and prefix username matches first.
    users.sort_by_key(|u| {
        let un = u.username.to_lowercase();
        (un != q, !un.starts_with(&q), u.name.to_lowercase())
    });
    users.truncate(MAX_RESULTS);
    Ok(Json(users))
}
