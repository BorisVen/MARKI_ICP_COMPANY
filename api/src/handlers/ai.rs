use axum::{
    body::Body,
    http::{header, StatusCode},
    response::Response,
    Json,
};
use serde::Deserialize;

#[derive(Deserialize)]
pub struct AiGenerateRequest {
    pub prompt: String,
}

/// `POST /api/ai/generate`
///
/// Server-side proxy to Pollinations.ai.
/// The browser cannot call Pollinations directly due to CORS,
/// but the server has no such restriction.
///
/// Returns the generated image as `image/jpeg`.
pub async fn generate_image(
    Json(body): Json<AiGenerateRequest>,
) -> Result<Response<Body>, (StatusCode, String)> {
    let prompt = body.prompt.trim().to_string();
    if prompt.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "prompt is required".into()));
    }

    let seed: u64  = rand_seed();
    let full_prompt = format!("{}, digital art, NFT, highly detailed", prompt);
    let encoded     = urlencoding::encode(&full_prompt);
    let url         = format!(
        "https://image.pollinations.ai/prompt/{}?width=1024&height=1024&model=flux&seed={}",
        encoded, seed
    );

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(90))
        .build()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let upstream = client
        .get(&url)
        .send()
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("Pollinations error: {}", e)))?;

    if !upstream.status().is_success() {
        return Err((
            StatusCode::BAD_GATEWAY,
            format!("Pollinations returned HTTP {}", upstream.status()),
        ));
    }

    let bytes = upstream
        .bytes()
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?;

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "image/jpeg")
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from(bytes))
        .unwrap())
}

/// Simple non-crypto seed based on current time (no extra dependency needed).
fn rand_seed() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64 ^ d.as_secs().wrapping_mul(6_364_136_223_846_793_005))
        .unwrap_or(42)
}
