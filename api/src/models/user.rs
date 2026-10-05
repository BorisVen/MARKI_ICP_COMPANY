use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserData {
    pub uid: String,
    pub name: String,
    pub username: String,
    pub email: String,
    pub phone: Option<String>,
    pub avatar: Option<String>,
    pub location: Option<String>,
    pub bio: Option<String>,
    pub created_at: String,
    #[serde(default)]
    pub company_approved: bool,
    pub delivery_address: Option<String>,
    #[serde(default)]
    pub pending_approval: bool,
    /// Any combination of: `owner`, `manager`, `controller`, `courier`, `customer`.
    /// Empty array == plain customer (legacy users default to this).
    #[serde(default)]
    pub roles: Vec<String>,
    /// uid of the company-owner this user reports to (when role is courier/manager/controller).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub company_id: Option<String>,
    #[serde(default)]
    pub rating_overall: f64,
    #[serde(default)]
    pub rating_by_category: std::collections::HashMap<String, f64>,

    #[serde(default)]
    pub followers_count: u64,
    #[serde(default)]
    pub following_count: u64,
    #[serde(default)]
    pub likes_count: u64,
    #[serde(default)]
    pub saved_posts: Vec<String>,
    #[serde(default)]
    pub reposted_posts: Vec<String>,

    // ── Company application + admin moderation ───────────────────────────────
    #[serde(default)]
    pub banned: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub company_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub registration_number: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact_email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub business_description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approval_requested_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reviewed_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reviewed_by: Option<String>,
    /// "pending" | "approved" | "rejected" | "banned" — convenience for admin filtering.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approval_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rejection_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ban_reason: Option<String>,
}

/// Public profile fields returned by `GET /api/users/search`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicUser {
    pub uid: String,
    pub name: String,
    pub username: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bio: Option<String>,
    pub company_approved: bool,
}

/// Public profile fields returned by `GET /api/users/:uid`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicProfile {
    pub uid: String,
    pub name: String,
    pub username: String,
    pub avatar: Option<String>,
    pub bio: Option<String>,
    pub company_approved: bool,
    pub followers_count: u64,
    pub following_count: u64,
    pub likes_count: u64,
    pub is_following: bool,
}

#[derive(Debug, Deserialize)]
pub struct UserSearchQuery {
    #[serde(default)]
    pub q: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisterRequest {
    pub uid: String,
    pub name: String,
    pub username: String,
    pub email: String,
    pub phone: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateProfileRequest {
    pub name: Option<String>,
    pub username: Option<String>,
    pub bio: Option<String>,
    pub location: Option<String>,
    pub delivery_address: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangePasswordRequest {
    pub new_password: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetApprovalRequest {
    pub approved: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateRolesRequest {
    pub roles: Vec<String>,
    pub company_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestApprovalRequest {
    pub company_name: String,
    pub registration_number: String,
    pub contact_email: String,
    #[serde(default)]
    pub description: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModerationRequest {
    /// Optional reason for reject/ban actions.
    #[serde(default)]
    pub reason: Option<String>,
}
