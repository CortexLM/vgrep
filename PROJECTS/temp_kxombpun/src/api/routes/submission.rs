//! Submission endpoints.
//!
//! Handles agent submission from miners.

use axum::{extract::State, http::StatusCode, Json};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use tracing::{info, warn};

use crate::api::ApiState;
use crate::auth::{create_submit_message, is_valid_ss58_hotkey, verify_signature};
use crate::storage::pg::{
    Submission, DEFAULT_COST_LIMIT_USD, MAX_COST_LIMIT_USD, SUBMISSION_COOLDOWN_SECS,
};
use crate::validation::package::PackageValidator;
use crate::validation::whitelist::{PythonWhitelist, WhitelistConfig};

// ============================================================================
// REQUEST/RESPONSE STRUCTS
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct SubmitAgentRequest {
    // ========================================================================
    // Mode 1: Single file submission (existing, backwards compatible)
    // ========================================================================
    /// Python source code (for single-file submissions)
    pub source_code: Option<String>,

    // ========================================================================
    // Mode 2: Package submission (new, multi-file)
    // ========================================================================
    /// Base64-encoded package archive (ZIP or TAR.GZ)
    pub package: Option<String>,
    /// Package format: "zip" or "tar.gz" (default: "zip")
    pub package_format: Option<String>,
    /// Entry point file within the package (default: "agent.py")
    pub entry_point: Option<String>,

    // ========================================================================
    // Common fields
    // ========================================================================
    pub miner_hotkey: String,
    pub signature: String,
    pub name: Option<String>,
    /// User's API key for LLM inferences (optional, serves as bridge for agent requests)
    pub api_key: Option<String>,
    /// API provider: openrouter, chutes, openai, anthropic, grok (default: openrouter)
    pub api_provider: Option<String>,
    /// Cost limit per validator in USD (0-100, default: 10)
    pub cost_limit_usd: Option<f64>,
}

#[derive(Debug, Serialize)]
pub struct SubmitAgentResponse {
    pub success: bool,
    pub submission_id: Option<String>,
    pub agent_hash: Option<String>,
    pub version: Option<i32>,
    pub cost_limit_usd: Option<f64>,
    pub error: Option<String>,
}

// ============================================================================
// HELPER FUNCTIONS
// ============================================================================

/// Get active validator count from platform-server with limited retries
const MAX_VALIDATOR_FETCH_RETRIES: u64 = 10;
const DEFAULT_VALIDATOR_COUNT: i32 = 3;

#[allow(dead_code)]
pub async fn get_active_validator_count(platform_url: &str) -> i32 {
    let url = format!("{}/api/v1/validators", platform_url);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .expect("Failed to create HTTP client");

    #[derive(serde::Deserialize)]
    struct ValidatorInfo {
        #[allow(dead_code)]
        hotkey: String,
    }

    for attempt in 1..=MAX_VALIDATOR_FETCH_RETRIES {
        match client.get(&url).send().await {
            Ok(response) => {
                if response.status().is_success() {
                    if let Ok(validators) = response.json::<Vec<ValidatorInfo>>().await {
                        let count = validators.len() as i32;
                        info!("Got {} active validators from platform-server", count);
                        return count.max(1);
                    }
                } else {
                    warn!(
                        "Failed to get validators from platform-server: {} (attempt {}/{})",
                        response.status(),
                        attempt,
                        MAX_VALIDATOR_FETCH_RETRIES
                    );
                }
            }
            Err(e) => {
                warn!(
                    "Platform-server not reachable: {} (attempt {}/{})",
                    e, attempt, MAX_VALIDATOR_FETCH_RETRIES
                );
            }
        }

        if attempt < MAX_VALIDATOR_FETCH_RETRIES {
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        }
    }

    warn!(
        "Failed to get validator count after {} attempts, using default: {}",
        MAX_VALIDATOR_FETCH_RETRIES, DEFAULT_VALIDATOR_COUNT
    );
    DEFAULT_VALIDATOR_COUNT
}

// ============================================================================
// SUBMISSION ENDPOINT
// ============================================================================

/// POST /api/v1/submit - Submit a new agent
///
/// Supports two submission modes:
/// 1. Single file: `source_code` field with Python code
/// 2. Package: `package` field with base64-encoded ZIP/TAR.GZ archive
///
/// Requires:
/// - Valid SS58 miner_hotkey
/// - Valid signature of "submit_agent:<sha256_of_content>"
/// - Rate limit: 1 submission per 3.6 hours per miner
/// - Unique agent name (or auto-version if same miner reuses name)
pub async fn submit_agent(
    State(state): State<Arc<ApiState>>,
    Json(req): Json<SubmitAgentRequest>,
) -> Result<Json<SubmitAgentResponse>, (StatusCode, Json<SubmitAgentResponse>)> {
    // Helper to create error response
    let err_response = |msg: String| SubmitAgentResponse {
        success: false,
        submission_id: None,
        agent_hash: None,
        version: None,
        cost_limit_usd: None,
        error: Some(msg),
    };

    // Validate miner_hotkey is a valid SS58 address
    if !is_valid_ss58_hotkey(&req.miner_hotkey) {
        warn!(
            "Invalid miner_hotkey format: {}",
            &req.miner_hotkey[..32.min(req.miner_hotkey.len())]
        );
        return Err((
            StatusCode::BAD_REQUEST,
            Json(err_response(format!(
                "Invalid miner_hotkey: must be a valid SS58 address. Received: {}",
                &req.miner_hotkey[..32.min(req.miner_hotkey.len())]
            ))),
        ));
    }

    // ========================================================================
    // Determine submission mode and validate content
    // ========================================================================

    let (is_package, source_code, package_data, package_format, entry_point, content_for_hash) =
        match (&req.source_code, &req.package) {
            // Mode 1: Single file submission
            (Some(code), None) => {
                // Validate with Python whitelist
                let whitelist = PythonWhitelist::new(WhitelistConfig::default());
                let validation = whitelist.verify(code);
                if !validation.valid {
                    return Err((
                        StatusCode::BAD_REQUEST,
                        Json(err_response(format!(
                            "Code validation failed: {}",
                            validation.errors.join(", ")
                        ))),
                    ));
                }

                (false, code.clone(), None, None, None, code.clone())
            }

            // Mode 2: Package submission
            (None, Some(pkg_base64)) => {
                // Decode base64
                let pkg_data = match base64::Engine::decode(
                    &base64::engine::general_purpose::STANDARD,
                    pkg_base64,
                ) {
                    Ok(data) => data,
                    Err(e) => {
                        return Err((
                            StatusCode::BAD_REQUEST,
                            Json(err_response(format!("Invalid base64 package: {}", e))),
                        ));
                    }
                };

                let format = req.package_format.as_deref().unwrap_or("zip");
                let entry = req.entry_point.as_deref().unwrap_or("agent.py");

                // Validate package
                let validator = PackageValidator::new();
                let validation = match validator.validate(&pkg_data, format, entry) {
                    Ok(v) => v,
                    Err(e) => {
                        return Err((
                            StatusCode::BAD_REQUEST,
                            Json(err_response(format!("Package validation error: {}", e))),
                        ));
                    }
                };

                if !validation.valid {
                    return Err((
                        StatusCode::BAD_REQUEST,
                        Json(err_response(format!(
                            "Package validation failed: {}",
                            validation.errors.join(", ")
                        ))),
                    ));
                }

                // Log warnings
                for warning in &validation.warnings {
                    warn!("Package warning: {}", warning);
                }

                (
                    true,
                    String::new(), // Empty source_code for packages
                    Some(pkg_data),
                    Some(format.to_string()),
                    Some(entry.to_string()),
                    pkg_base64.clone(), // Hash the base64 for signature
                )
            }

            // Error: Both provided
            (Some(_), Some(_)) => {
                return Err((
                    StatusCode::BAD_REQUEST,
                    Json(err_response(
                        "Cannot provide both source_code and package. Choose one.".to_string(),
                    )),
                ));
            }

            // Error: Neither provided
            (None, None) => {
                return Err((
                    StatusCode::BAD_REQUEST,
                    Json(err_response(
                        "Must provide either source_code (single file) or package (multi-file archive).".to_string(),
                    )),
                ));
            }
        };

    // Verify signature
    let expected_message = create_submit_message(&content_for_hash);
    
    #[cfg(debug_assertions)]
    let skip_auth = std::env::var("SKIP_AUTH")
        .map(|v| v == "1")
        .unwrap_or(false);
    #[cfg(not(debug_assertions))]
    let skip_auth = false;

    if !skip_auth && !verify_signature(&req.miner_hotkey, &expected_message, &req.signature) {
        warn!(
            "Invalid signature for submission from {}",
            &req.miner_hotkey[..16.min(req.miner_hotkey.len())]
        );
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(err_response(format!(
                "Invalid signature. Message to sign: '{}'. Use sr25519 signature.",
                expected_message
            ))),
        ));
    }

    // Check rate limit: 1 agent per 3.6 hours (skip in test mode)
    if !skip_auth {
        match state.storage.can_miner_submit(&req.miner_hotkey).await {
            Ok((can_submit, reason)) => {
                if !can_submit {
                    warn!(
                        "Rate limit exceeded for miner {}: {:?}",
                        &req.miner_hotkey[..16.min(req.miner_hotkey.len())],
                        reason
                    );
                    return Err((
                        StatusCode::TOO_MANY_REQUESTS,
                        Json(err_response(reason.unwrap_or_else(|| {
                            format!(
                                "Rate limit: 1 submission per {} hours",
                                SUBMISSION_COOLDOWN_SECS / 3600
                            )
                        }))),
                    ));
                }
            }
            Err(e) => {
                warn!("Failed to check rate limit: {:?}", e);
                return Err((
                    StatusCode::SERVICE_UNAVAILABLE,
                    Json(err_response(
                        "Rate limit check unavailable. Please retry later.".to_string(),
                    )),
                ));
            }
        }
    }

    // Get current epoch
    let epoch = state.storage.get_current_epoch().await.unwrap_or(0);

    // Check agent name uniqueness
    if let Some(ref name) = req.name {
        match state
            .storage
            .is_name_taken_by_other(name, &req.miner_hotkey)
            .await
        {
            Ok(taken) => {
                if taken {
                    warn!("Agent name '{}' already taken by another miner", name);
                    return Err((
                        StatusCode::CONFLICT,
                        Json(err_response(format!(
                            "Agent name '{}' is already taken by another miner.",
                            name
                        ))),
                    ));
                }
            }
            Err(e) => {
                warn!("Failed to check name uniqueness: {:?}", e);
            }
        }
    }

    // Get next version
    let version = state
        .storage
        .get_next_version(&req.miner_hotkey, req.name.as_deref())
        .await
        .unwrap_or(1);

    // Validate and clamp cost limit
    let cost_limit = req
        .cost_limit_usd
        .unwrap_or(DEFAULT_COST_LIMIT_USD)
        .clamp(0.0, MAX_COST_LIMIT_USD);

    // Compute hashes
    let source_hash = hex::encode(Sha256::digest(content_for_hash.as_bytes()));
    let agent_hash = format!(
        "{}{}",
        &hex::encode(Sha256::digest(req.miner_hotkey.as_bytes()))[..16],
        &source_hash[..16]
    );

    // Get active checkpoint for this submission
    let checkpoint_id = state
        .storage
        .get_active_checkpoint()
        .await
        .unwrap_or_else(|_| "checkpoint1".to_string());

    // Create submission
    let submission_id = uuid::Uuid::new_v4().to_string();
    let submission = Submission {
        id: submission_id.clone(),
        agent_hash: agent_hash.clone(),
        miner_hotkey: req.miner_hotkey.clone(),
        source_code,
        source_hash,
        name: req.name.clone(),
        version,
        epoch,
        status: "pending".to_string(),
        api_key: req.api_key,
        api_provider: req.api_provider,
        cost_limit_usd: cost_limit,
        total_cost_usd: 0.0,
        created_at: chrono::Utc::now().timestamp(),
        // Compilation fields
        binary: None,
        binary_size: 0,
        compile_status: "pending".to_string(),
        compile_error: None,
        compile_time_ms: 0,
        flagged: false,
        flag_reason: None,
        // Package fields
        is_package,
        package_data,
        package_format,
        entry_point,
        // Code visibility & decay (defaults)
        disable_public_code: false,
        disable_decay: false,
        // Checkpoint assignment
        checkpoint_id,
    };

    // Store submission
    if let Err(e) = state.storage.create_submission(&submission).await {
        warn!("Failed to create submission: {:?}", e);
        tracing::error!(
            "Submission error - id: {}, agent_hash: {}, is_package: {}, error: {:?}",
            submission.id,
            submission.agent_hash,
            submission.is_package,
            e
        );
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(err_response(format!("Failed to store submission: {}", e))),
        ));
    }

    // Add test validators in test mode
    if skip_auth {
        let test_validators = [
            "5FHneW46xGXgs5mUiveU4sbTyGBzmstUspZC92UhjJM694ty",
            "5FLSigC9HGRKVhB9FiEo4Y3koPsNmBmLJbpXg2mp1hXcS59Y",
            "5DAAnrj7VHTznn2AWBemMuyBwZWs6FNFjdyVXUeYum3PTXFy",
            "5HGjWAeFDfFCWPsjFQdVV2Msvz2XtMktvgocEZcCj68kUMaw",
        ];
        for v in test_validators {
            state.auth.add_validator(v).await;
        }
    }

    // Queue submission for evaluation (requires 2 validators)
    if let Err(e) = state
        .storage
        .queue_submission_for_evaluation(&submission_id, &agent_hash, &req.miner_hotkey, 2)
        .await
    {
        warn!("Failed to queue submission for evaluation: {:?}", e);
    }

    let submission_type = if is_package { "package" } else { "single-file" };
    info!(
        "Agent submitted: {} v{} ({}) from {} (epoch {}, cost: ${:.2})",
        &agent_hash[..16],
        version,
        submission_type,
        &req.miner_hotkey[..16.min(req.miner_hotkey.len())],
        epoch,
        cost_limit
    );

    // Broadcast "new_submission" event to validators
    {
        let platform_url = state.platform_url.clone();
        let challenge_id = state.challenge_id.clone();
        let broadcast_submission_id = submission_id.clone();
        let broadcast_agent_hash = agent_hash.clone();
        let broadcast_miner_hotkey = req.miner_hotkey.clone();
        let broadcast_name = req.name.clone();
        let broadcast_epoch = epoch;
        let broadcast_is_package = is_package;

        tokio::spawn(async move {
            let client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_default();

            let event_payload = serde_json::json!({
                "submission_id": broadcast_submission_id,
                "agent_hash": broadcast_agent_hash,
                "miner_hotkey": broadcast_miner_hotkey,
                "name": broadcast_name,
                "epoch": broadcast_epoch,
                "is_package": broadcast_is_package,
            });

            let broadcast_request = serde_json::json!({
                "challenge_id": challenge_id,
                "event_name": "new_submission",
                "payload": event_payload,
            });

            let broadcast_secret = std::env::var("BROADCAST_SECRET").unwrap_or_default();

            match client
                .post(format!("{}/api/v1/events/broadcast", platform_url))
                .header("X-Broadcast-Secret", broadcast_secret)
                .json(&broadcast_request)
                .send()
                .await
            {
                Ok(response) => {
                    if response.status().is_success() {
                        info!(
                            "Broadcast new_submission event for agent {}",
                            &broadcast_agent_hash[..16]
                        );
                    } else {
                        warn!("Failed to broadcast event: {}", response.status());
                    }
                }
                Err(e) => {
                    warn!("Failed to broadcast event: {}", e);
                }
            }
        });
    }

    Ok(Json(SubmitAgentResponse {
        success: true,
        submission_id: Some(submission_id),
        agent_hash: Some(agent_hash),
        version: Some(version),
        cost_limit_usd: Some(cost_limit),
        error: None,
    }))
}
