//! Validator endpoints.
//!
//! Endpoints for validator operations including job claiming,
//! heartbeats, task logging, and progress tracking.

use axum::{
    extract::{Path, Query, State},
    http::{header, StatusCode},
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{debug, error, info, warn};

use crate::api::ApiState;
use crate::auth::{is_timestamp_valid, is_valid_ss58_hotkey, verify_signature};
use crate::storage::pg::{TaskAssignment, TaskLog, ValidatorReadiness};

// ============================================================================
// CLAIM JOBS
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct ClaimJobsRequest {
    pub validator_hotkey: String,
    pub signature: String,
    pub timestamp: i64,
    pub count: Option<usize>, // Max jobs to claim (default: 5, max: 10)
}

#[derive(Debug, Serialize)]
pub struct ClaimJobsResponse {
    pub success: bool,
    pub jobs: Vec<JobInfo>,
    pub total_available: usize,
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct JobInfo {
    pub pending_id: String,
    pub submission_id: String,
    pub agent_hash: String,
    pub miner_hotkey: String,
    /// Compiled binary (base64 encoded)
    pub binary_base64: String,
    /// Binary size in bytes
    pub binary_size: i32,
    pub window_expires_at: i64,
    pub tasks: Vec<TaskAssignment>,
}

/// POST /api/v1/validator/claim_jobs - Claim pending evaluation jobs
///
/// Each validator must evaluate ALL pending agents.
/// Returns jobs that this validator hasn't evaluated yet.
/// Window expires after 6h - late validators are exempt.
pub async fn claim_jobs(
    State(state): State<Arc<ApiState>>,
    Json(req): Json<ClaimJobsRequest>,
) -> Result<Json<ClaimJobsResponse>, (StatusCode, Json<ClaimJobsResponse>)> {
    // Validate hotkey
    if !is_valid_ss58_hotkey(&req.validator_hotkey) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ClaimJobsResponse {
                success: false,
                jobs: vec![],
                total_available: 0,
                error: Some("Invalid hotkey format".to_string()),
            }),
        ));
    }

    // Validate timestamp
    if !is_timestamp_valid(req.timestamp) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ClaimJobsResponse {
                success: false,
                jobs: vec![],
                total_available: 0,
                error: Some("Timestamp expired".to_string()),
            }),
        ));
    }

    // Verify signature (skip in test mode)
    let message = format!("claim_jobs:{}", req.timestamp);
    #[cfg(debug_assertions)]
    let skip_auth = std::env::var("SKIP_AUTH")
        .map(|v| v == "1")
        .unwrap_or(false);
    #[cfg(not(debug_assertions))]
    let skip_auth = false;
    if !skip_auth && !verify_signature(&req.validator_hotkey, &message, &req.signature) {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(ClaimJobsResponse {
                success: false,
                jobs: vec![],
                total_available: 0,
                error: Some("Invalid signature".to_string()),
            }),
        ));
    }

    // Check if validator is authorized (>= 10000 TAO stake or whitelisted)
    if !skip_auth {
        if !state.is_authorized_validator(&req.validator_hotkey).await {
            warn!(
                "Unauthorized validator claim attempt: {} (insufficient stake)",
                &req.validator_hotkey[..16.min(req.validator_hotkey.len())]
            );
            return Err((
                StatusCode::FORBIDDEN,
                Json(ClaimJobsResponse {
                    success: false,
                    jobs: vec![],
                    total_available: 0,
                    error: Some(
                        "Validator not authorized (requires >= 10000 TAO stake)".to_string(),
                    ),
                }),
            ));
        }
    } else {
        // Auto-add to whitelist in test mode
        state.auth.add_validator(&req.validator_hotkey).await;
    }

    let count = req.count.unwrap_or(5).min(10);

    // Get jobs available for this validator
    let available_jobs = state
        .storage
        .get_jobs_for_validator(&req.validator_hotkey, count as i64)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ClaimJobsResponse {
                    success: false,
                    jobs: vec![],
                    total_available: 0,
                    error: Some(e.to_string()),
                }),
            )
        })?;

    let total_available = available_jobs.len();

    if available_jobs.is_empty() {
        return Ok(Json(ClaimJobsResponse {
            success: true,
            jobs: vec![],
            total_available: 0,
            error: Some("No pending jobs for this validator".to_string()),
        }));
    }

    // Claim the jobs
    let agent_hashes: Vec<String> = available_jobs
        .iter()
        .map(|j| j.agent_hash.clone())
        .collect();
    let _ = state
        .storage
        .claim_jobs(&req.validator_hotkey, &agent_hashes)
        .await;

    let jobs: Vec<JobInfo> = available_jobs
        .into_iter()
        .map(|j| JobInfo {
            pending_id: j.pending_id,
            submission_id: j.submission_id,
            agent_hash: j.agent_hash,
            miner_hotkey: j.miner_hotkey,
            binary_base64: j.binary_base64,
            binary_size: j.binary_size,
            window_expires_at: j.window_expires_at,
            tasks: j.tasks,
        })
        .collect();

    info!(
        "Validator {} claimed {} jobs",
        &req.validator_hotkey[..16.min(req.validator_hotkey.len())],
        jobs.len()
    );

    Ok(Json(ClaimJobsResponse {
        success: true,
        jobs,
        total_available,
        error: None,
    }))
}

// ============================================================================
// VALIDATOR READINESS (Heartbeat for broker connectivity)
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct ValidatorHeartbeatRequest {
    pub validator_hotkey: String,
    pub signature: String,
    pub timestamp: i64,
    pub is_ready: bool,
    pub broker_connected: bool,
    pub error_message: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ValidatorHeartbeatResponse {
    pub success: bool,
    pub message: String,
    pub error: Option<String>,
}

/// POST /api/v1/validator/heartbeat - Report validator readiness status
///
/// Validators must call this every 1 minute to report they are ready.
/// If broker is not connected, set broker_connected=false.
/// Validators with stale heartbeats (>2 min) are not used for task assignment.
pub async fn validator_heartbeat(
    State(state): State<Arc<ApiState>>,
    Json(req): Json<ValidatorHeartbeatRequest>,
) -> Result<Json<ValidatorHeartbeatResponse>, (StatusCode, Json<ValidatorHeartbeatResponse>)> {
    // Validate hotkey
    if !is_valid_ss58_hotkey(&req.validator_hotkey) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ValidatorHeartbeatResponse {
                success: false,
                message: String::new(),
                error: Some("Invalid hotkey format".to_string()),
            }),
        ));
    }

    // Validate timestamp
    if !is_timestamp_valid(req.timestamp) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ValidatorHeartbeatResponse {
                success: false,
                message: String::new(),
                error: Some("Timestamp expired".to_string()),
            }),
        ));
    }

    // Verify signature (skip in test mode)
    let message = format!("heartbeat:{}:{}", req.timestamp, req.is_ready);
    #[cfg(debug_assertions)]
    let skip_auth = std::env::var("SKIP_AUTH")
        .map(|v| v == "1")
        .unwrap_or(false);
    #[cfg(not(debug_assertions))]
    let skip_auth = false;
    if !skip_auth && !verify_signature(&req.validator_hotkey, &message, &req.signature) {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(ValidatorHeartbeatResponse {
                success: false,
                message: String::new(),
                error: Some("Invalid signature".to_string()),
            }),
        ));
    }

    // Update readiness status
    state
        .storage
        .update_validator_readiness(
            &req.validator_hotkey,
            req.is_ready,
            req.broker_connected,
            req.error_message.as_deref(),
        )
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ValidatorHeartbeatResponse {
                    success: false,
                    message: String::new(),
                    error: Some(e.to_string()),
                }),
            )
        })?;

    let status = if req.is_ready && req.broker_connected {
        "ready"
    } else if req.broker_connected {
        "broker_ok_not_ready"
    } else {
        "broker_disconnected"
    };

    debug!(
        "Validator {} heartbeat: {} (broker={})",
        &req.validator_hotkey[..16.min(req.validator_hotkey.len())],
        status,
        req.broker_connected
    );

    Ok(Json(ValidatorHeartbeatResponse {
        success: true,
        message: format!("Heartbeat recorded: {}", status),
        error: None,
    }))
}

/// GET /api/v1/validators/readiness - Get all validator readiness statuses
pub async fn get_validators_readiness(
    State(state): State<Arc<ApiState>>,
) -> Result<Json<Vec<ValidatorReadiness>>, (StatusCode, Json<serde_json::Value>)> {
    let readiness = state
        .storage
        .get_all_validator_readiness()
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": e.to_string()})),
            )
        })?;

    Ok(Json(readiness))
}

/// GET /api/v1/validators/ready - Get only ready validators
pub async fn get_ready_validators(
    State(state): State<Arc<ApiState>>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> Result<Json<Vec<ValidatorReadiness>>, (StatusCode, Json<serde_json::Value>)> {
    let limit = params
        .get("limit")
        .and_then(|s| s.parse().ok())
        .unwrap_or(10);

    let ready = state
        .storage
        .get_ready_validators(limit)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": e.to_string()})),
            )
        })?;

    Ok(Json(ready))
}

// ============================================================================
// LOG TASK (Real-time task logging)
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct LogTaskRequest {
    pub validator_hotkey: String,
    pub signature: String,
    pub timestamp: i64,
    pub agent_hash: String,
    pub task_id: String,
    pub task_name: String,
    pub passed: bool,
    pub score: f64,
    pub execution_time_ms: i64,
    pub steps: i32,
    pub cost_usd: f64,
    pub error: Option<String>,
    pub execution_log: Option<String>,
    pub trajectory: Option<serde_json::Value>,
    pub started_at: i64,
    // Verbose logging fields for debugging agent failures
    #[serde(default)]
    pub agent_stderr: Option<String>,
    #[serde(default)]
    pub agent_stdout: Option<String>,
    #[serde(default)]
    pub test_output: Option<String>,
    #[serde(default)]
    pub steps_executed: Option<i32>,
    /// For global failures (before tasks run): "download", "container_create", "binary_exec", etc.
    #[serde(default)]
    pub failure_stage: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct LogTaskResponse {
    pub success: bool,
    pub tasks_logged: i32,
    pub tasks_total: i32,
    pub error: Option<String>,
}

/// POST /api/v1/validator/log_task - Log individual task result (real-time)
///
/// Validators call this endpoint after completing each task.
/// This allows real-time tracking and ensures all task data is saved.
pub async fn log_task(
    State(state): State<Arc<ApiState>>,
    Json(req): Json<LogTaskRequest>,
) -> Result<Json<LogTaskResponse>, (StatusCode, Json<LogTaskResponse>)> {
    // Validate hotkey
    if !is_valid_ss58_hotkey(&req.validator_hotkey) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(LogTaskResponse {
                success: false,
                tasks_logged: 0,
                tasks_total: 0,
                error: Some("Invalid hotkey format".to_string()),
            }),
        ));
    }

    // Validate timestamp
    if !is_timestamp_valid(req.timestamp) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(LogTaskResponse {
                success: false,
                tasks_logged: 0,
                tasks_total: 0,
                error: Some("Timestamp expired".to_string()),
            }),
        ));
    }

    // Verify signature (skip in test mode)
    let message = format!(
        "log_task:{}:{}:{}",
        req.agent_hash, req.task_id, req.timestamp
    );
    #[cfg(debug_assertions)]
    let skip_auth = std::env::var("SKIP_AUTH")
        .map(|v| v == "1")
        .unwrap_or(false);
    #[cfg(not(debug_assertions))]
    let skip_auth = false;
    if !skip_auth && !verify_signature(&req.validator_hotkey, &message, &req.signature) {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(LogTaskResponse {
                success: false,
                tasks_logged: 0,
                tasks_total: 0,
                error: Some("Invalid signature".to_string()),
            }),
        ));
    }

    // Check if validator is authorized (>= 10000 TAO stake or whitelisted)
    if !skip_auth && !state.is_authorized_validator(&req.validator_hotkey).await {
        return Err((
            StatusCode::FORBIDDEN,
            Json(LogTaskResponse {
                success: false,
                tasks_logged: 0,
                tasks_total: 0,
                error: Some("Validator not authorized (requires >= 10000 TAO stake)".to_string()),
            }),
        ));
    }

    // Check if validator is assigned to this agent (skip in test mode)
    let is_assigned = if skip_auth {
        true // In test mode, allow any validator
    } else {
        state
            .storage
            .is_validator_assigned(&req.agent_hash, &req.validator_hotkey)
            .await
            .unwrap_or(false)
    };

    if !is_assigned {
        return Err((
            StatusCode::FORBIDDEN,
            Json(LogTaskResponse {
                success: false,
                tasks_logged: 0,
                tasks_total: 0,
                error: Some("Validator not assigned to this agent".to_string()),
            }),
        ));
    }

    // Create task log
    let task_log = TaskLog {
        id: uuid::Uuid::new_v4().to_string(),
        agent_hash: req.agent_hash.clone(),
        validator_hotkey: req.validator_hotkey.clone(),
        task_id: req.task_id.clone(),
        task_name: req.task_name.clone(),
        passed: req.passed,
        score: req.score,
        execution_time_ms: req.execution_time_ms,
        steps: req.steps,
        cost_usd: req.cost_usd,
        error: req.error,
        execution_log: req.execution_log,
        trajectory: req.trajectory,
        started_at: req.started_at,
        completed_at: chrono::Utc::now().timestamp(),
        // Verbose logging fields
        agent_stderr: req.agent_stderr,
        agent_stdout: req.agent_stdout,
        test_output: req.test_output,
        steps_executed: req.steps_executed,
        failure_stage: req.failure_stage,
    };

    // Store task log
    if let Err(e) = state.storage.store_task_log(&task_log).await {
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(LogTaskResponse {
                success: false,
                tasks_logged: 0,
                tasks_total: 0,
                error: Some(format!("Failed to store task log: {}", e)),
            }),
        ));
    }

    // Calculate and update cost from llm_usage table
    // This aggregates all LLM calls made during this task execution
    match state
        .storage
        .get_task_llm_cost(&req.agent_hash, &req.validator_hotkey, &req.task_id)
        .await
    {
        Ok(calculated_cost) if calculated_cost > 0.0 => {
            if let Err(e) = state
                .storage
                .update_task_log_cost(
                    &req.agent_hash,
                    &req.validator_hotkey,
                    &req.task_id,
                    calculated_cost,
                )
                .await
            {
                warn!(
                    "Failed to update task cost for {}/{}: {}",
                    &req.agent_hash[..16.min(req.agent_hash.len())],
                    &req.task_id,
                    e
                );
            } else {
                debug!(
                    "Updated task {} cost to ${:.4} from llm_usage",
                    &req.task_id, calculated_cost
                );
            }
        }
        Ok(_) => {
            // No LLM usage recorded for this task (agent might not use LLM)
        }
        Err(e) => {
            warn!("Failed to get task LLM cost: {}", e);
        }
    }

    // Get current progress
    let summary = state
        .storage
        .get_task_log_summary(&req.agent_hash, &req.validator_hotkey)
        .await
        .unwrap_or_default();

    // Remove from real-time cache now that task is persisted to DB
    if let Some(ref cache) = state.task_stream_cache {
        cache.remove(&req.agent_hash, &req.validator_hotkey, &req.task_id);
    }

    info!(
        "Task logged: {} {} task={} ({}/{} complete)",
        &req.validator_hotkey[..16.min(req.validator_hotkey.len())],
        &req.agent_hash[..16.min(req.agent_hash.len())],
        req.task_name,
        summary.completed_tasks,
        summary.total_tasks
    );

    // Auto-detect completion: when all tasks are logged, auto-complete the evaluation
    // This replaces the need for validators to call submit_result
    if summary.completed_tasks == summary.total_tasks && summary.total_tasks > 0 {
        info!(
            "Validator {} completed all {} tasks for agent {}, auto-completing evaluation",
            &req.validator_hotkey[..16.min(req.validator_hotkey.len())],
            summary.total_tasks,
            &req.agent_hash[..16.min(req.agent_hash.len())]
        );

        match state
            .storage
            .auto_complete_validator_evaluation(&req.agent_hash, &req.validator_hotkey, &summary)
            .await
        {
            Ok((consensus_reached, final_score)) => {
                if consensus_reached {
                    info!(
                        "Consensus reached for agent {}: final score = {:.4}",
                        &req.agent_hash[..16.min(req.agent_hash.len())],
                        final_score.unwrap_or(0.0)
                    );
                }
            }
            Err(e) => {
                warn!(
                    "Failed to auto-complete evaluation for {} on {}: {}",
                    &req.validator_hotkey[..16.min(req.validator_hotkey.len())],
                    &req.agent_hash[..16.min(req.agent_hash.len())],
                    e
                );
            }
        }
    }

    Ok(Json(LogTaskResponse {
        success: true,
        tasks_logged: summary.completed_tasks,
        tasks_total: summary.total_tasks,
        error: None,
    }))
}

// ============================================================================
// REAL-TIME TASK STREAMING
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct TaskStreamUpdateRequest {
    pub validator_hotkey: String,
    pub signature: String,
    pub timestamp: i64,
    pub agent_hash: String,
    pub task_id: String,
    pub task_name: Option<String>,
    pub status: Option<String>,
    pub stdout_chunk: Option<String>,
    pub stderr_chunk: Option<String>,
    pub current_step: Option<i32>,
}

#[derive(Debug, Serialize)]
pub struct TaskStreamUpdateResponse {
    pub success: bool,
    pub error: Option<String>,
}

/// POST /api/v1/validator/task_stream_update - Push real-time task progress
///
/// Validators call this during task execution to stream live stdout/stderr.
/// Data is stored in memory cache and evicted when task is persisted to DB.
pub async fn task_stream_update(
    State(state): State<Arc<ApiState>>,
    Json(req): Json<TaskStreamUpdateRequest>,
) -> Result<Json<TaskStreamUpdateResponse>, (StatusCode, Json<TaskStreamUpdateResponse>)> {
    // Check if cache is available and enabled
    let cache = match &state.task_stream_cache {
        Some(c) if c.is_enabled() => c,
        _ => {
            return Ok(Json(TaskStreamUpdateResponse {
                success: true,
                error: None,
            }));
        }
    };

    // Validate hotkey
    if !is_valid_ss58_hotkey(&req.validator_hotkey) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(TaskStreamUpdateResponse {
                success: false,
                error: Some("Invalid hotkey format".to_string()),
            }),
        ));
    }

    // Validate timestamp
    if !is_timestamp_valid(req.timestamp) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(TaskStreamUpdateResponse {
                success: false,
                error: Some("Timestamp expired".to_string()),
            }),
        ));
    }

    // Verify signature
    let message = format!(
        "task_stream:{}:{}:{}",
        req.agent_hash, req.task_id, req.timestamp
    );
    #[cfg(debug_assertions)]
    let skip_auth = std::env::var("SKIP_AUTH")
        .map(|v| v == "1")
        .unwrap_or(false);
    #[cfg(not(debug_assertions))]
    let skip_auth = false;
    if !skip_auth && !verify_signature(&req.validator_hotkey, &message, &req.signature) {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(TaskStreamUpdateResponse {
                success: false,
                error: Some("Invalid signature".to_string()),
            }),
        ));
    }

    // Push update to cache
    let update = crate::cache::task_stream::TaskStreamUpdate {
        agent_hash: req.agent_hash,
        validator_hotkey: req.validator_hotkey,
        task_id: req.task_id,
        task_name: req.task_name,
        status: req.status,
        stdout_chunk: req.stdout_chunk,
        stderr_chunk: req.stderr_chunk,
        current_step: req.current_step,
    };

    cache.push_update(update);

    Ok(Json(TaskStreamUpdateResponse {
        success: true,
        error: None,
    }))
}

#[derive(Debug, Serialize)]
pub struct LiveTasksResponse {
    pub agent_hash: String,
    pub tasks: Vec<crate::cache::task_stream::LiveTaskProgress>,
    pub cache_stats: Option<crate::cache::task_stream::TaskStreamStats>,
}

/// GET /api/v1/agent/:agent_hash/tasks/live - Get all live task progress for an agent
///
/// Returns real-time streaming progress from the in-memory cache.
/// No authentication required.
pub async fn get_live_tasks(
    State(state): State<Arc<ApiState>>,
    Path(agent_hash): Path<String>,
) -> Result<Json<LiveTasksResponse>, (StatusCode, String)> {
    let cache = match &state.task_stream_cache {
        Some(c) => c,
        None => {
            return Ok(Json(LiveTasksResponse {
                agent_hash,
                tasks: vec![],
                cache_stats: None,
            }));
        }
    };

    let entries = cache.get_agent_tasks(&agent_hash);
    let tasks: Vec<_> = entries
        .into_iter()
        .map(crate::cache::task_stream::LiveTaskProgress::from)
        .collect();

    Ok(Json(LiveTasksResponse {
        agent_hash,
        tasks,
        cache_stats: Some(cache.stats()),
    }))
}

#[derive(Debug, Serialize)]
pub struct LiveTaskDetailResponse {
    pub agent_hash: String,
    pub task_id: String,
    pub validators: Vec<crate::cache::task_stream::LiveTaskProgress>,
}

/// GET /api/v1/agent/:agent_hash/tasks/:task_id/live - Get live progress for specific task
///
/// Returns real-time progress for a specific task across all validators.
pub async fn get_live_task_detail(
    State(state): State<Arc<ApiState>>,
    Path((agent_hash, task_id)): Path<(String, String)>,
) -> Result<Json<LiveTaskDetailResponse>, (StatusCode, String)> {
    let cache = match &state.task_stream_cache {
        Some(c) => c,
        None => {
            return Ok(Json(LiveTaskDetailResponse {
                agent_hash,
                task_id,
                validators: vec![],
            }));
        }
    };

    let entries = cache.get_task_by_id(&agent_hash, &task_id);
    let validators: Vec<_> = entries
        .into_iter()
        .map(crate::cache::task_stream::LiveTaskProgress::from)
        .collect();

    Ok(Json(LiveTaskDetailResponse {
        agent_hash,
        task_id,
        validators,
    }))
}

// ============================================================================
// SUBMIT RESULT - DEPRECATED
// ============================================================================
// NOTE: submit_result has been removed. Validator evaluation completion is now
// automatically detected when all tasks are logged via log_task().
// The server auto-creates ValidatorEvaluation records when a validator logs
// all their assigned tasks (completed_tasks == total_tasks).
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct GetMyJobsRequest {
    pub validator_hotkey: String,
    pub signature: String,
    pub timestamp: i64,
}

#[derive(Debug, Serialize)]
pub struct GetMyJobsResponse {
    pub success: bool,
    pub pending_jobs: Vec<ValidatorJob>,
    pub completed_count: usize,
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ValidatorJob {
    pub agent_hash: String,
    pub miner_hotkey: String,
    pub submission_id: String,
    pub assigned_at: i64,
    pub compile_status: String, // "pending", "compiling", "success", "failed"
    pub binary_ready: bool,     // true if compile_status == "success"
    /// Task IDs assigned to this validator for this agent (10 tasks each)
    pub assigned_task_ids: Vec<String>,
}

/// POST /api/v1/validator/my_jobs - Get validator's pending jobs
pub async fn get_my_jobs(
    State(state): State<Arc<ApiState>>,
    Json(req): Json<GetMyJobsRequest>,
) -> Result<Json<GetMyJobsResponse>, (StatusCode, Json<GetMyJobsResponse>)> {
    // Validate hotkey
    if !is_valid_ss58_hotkey(&req.validator_hotkey) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(GetMyJobsResponse {
                success: false,
                pending_jobs: vec![],
                completed_count: 0,
                error: Some("Invalid hotkey format".to_string()),
            }),
        ));
    }

    // Validate timestamp
    if !is_timestamp_valid(req.timestamp) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(GetMyJobsResponse {
                success: false,
                pending_jobs: vec![],
                completed_count: 0,
                error: Some("Timestamp expired".to_string()),
            }),
        ));
    }

    // Verify signature (skip in test mode)
    let message = format!("get_my_jobs:{}", req.timestamp);
    #[cfg(debug_assertions)]
    let skip_auth = std::env::var("SKIP_AUTH")
        .map(|v| v == "1")
        .unwrap_or(false);
    #[cfg(not(debug_assertions))]
    let skip_auth = false;
    if !skip_auth && !verify_signature(&req.validator_hotkey, &message, &req.signature) {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(GetMyJobsResponse {
                success: false,
                pending_jobs: vec![],
                completed_count: 0,
                error: Some("Invalid signature".to_string()),
            }),
        ));
    }

    // Check if validator is authorized (>= 10000 TAO stake or whitelisted)
    if !state.is_authorized_validator(&req.validator_hotkey).await {
        return Err((
            StatusCode::FORBIDDEN,
            Json(GetMyJobsResponse {
                success: false,
                pending_jobs: vec![],
                completed_count: 0,
                error: Some("Validator not authorized (requires >= 10000 TAO stake)".to_string()),
            }),
        ));
    }

    // Get pending jobs for this validator with compile status
    let jobs = state
        .storage
        .get_validator_jobs_with_status(&req.validator_hotkey, 100)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(GetMyJobsResponse {
                    success: false,
                    pending_jobs: vec![],
                    completed_count: 0,
                    error: Some(e.to_string()),
                }),
            )
        })?;

    // Get claims (jobs in progress)
    let claims = state
        .storage
        .get_validator_claims(&req.validator_hotkey)
        .await
        .unwrap_or_default();

    // Build pending jobs with assigned task IDs for each
    let mut pending_jobs: Vec<ValidatorJob> = Vec::new();
    for j in jobs {
        // Get assigned task IDs for this validator/agent pair
        let assigned_task_ids = state
            .storage
            .get_validator_tasks(&j.agent_hash, &req.validator_hotkey)
            .await
            .map(|tasks| tasks.into_iter().map(|t| t.task_id).collect())
            .unwrap_or_else(|_| Vec::new());

        pending_jobs.push(ValidatorJob {
            agent_hash: j.agent_hash,
            miner_hotkey: j.miner_hotkey,
            submission_id: j.submission_id,
            assigned_at: j.assigned_at,
            compile_status: j.compile_status.clone(),
            binary_ready: j.compile_status == "success",
            assigned_task_ids,
        });
    }

    Ok(Json(GetMyJobsResponse {
        success: true,
        pending_jobs,
        completed_count: claims.iter().filter(|c| c.status == "completed").count(),
        error: None,
    }))
}

// ============================================================================
// GET ASSIGNED TASKS ENDPOINT (for live refresh)
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct GetAssignedTasksRequest {
    pub validator_hotkey: String,
    pub agent_hash: String,
    pub signature: String,
    pub timestamp: i64,
}

#[derive(Debug, Serialize)]
pub struct GetAssignedTasksResponse {
    pub success: bool,
    pub task_ids: Vec<String>,
    pub error: Option<String>,
}

/// POST /api/v1/validator/get_assigned_tasks - Get current assigned tasks for an agent
/// Allows validators to refresh their task list during evaluation (for live reassignments)
pub async fn get_assigned_tasks(
    State(state): State<Arc<ApiState>>,
    Json(req): Json<GetAssignedTasksRequest>,
) -> Result<Json<GetAssignedTasksResponse>, (StatusCode, Json<GetAssignedTasksResponse>)> {
    // Validate hotkey
    if !is_valid_ss58_hotkey(&req.validator_hotkey) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(GetAssignedTasksResponse {
                success: false,
                task_ids: vec![],
                error: Some("Invalid hotkey format".to_string()),
            }),
        ));
    }

    // Validate timestamp
    if !is_timestamp_valid(req.timestamp) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(GetAssignedTasksResponse {
                success: false,
                task_ids: vec![],
                error: Some("Timestamp expired".to_string()),
            }),
        ));
    }

    // Verify signature (skip in test mode)
    let message = format!("get_assigned_tasks:{}:{}", req.agent_hash, req.timestamp);
    #[cfg(debug_assertions)]
    let skip_auth = std::env::var("SKIP_AUTH")
        .map(|v| v == "1")
        .unwrap_or(false);
    #[cfg(not(debug_assertions))]
    let skip_auth = false;

    if !skip_auth && !verify_signature(&req.validator_hotkey, &message, &req.signature) {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(GetAssignedTasksResponse {
                success: false,
                task_ids: vec![],
                error: Some("Invalid signature".to_string()),
            }),
        ));
    }

    // Get assigned tasks from DB
    let task_ids = state
        .storage
        .get_validator_tasks(&req.agent_hash, &req.validator_hotkey)
        .await
        .map(|tasks| tasks.into_iter().map(|t| t.task_id).collect())
        .unwrap_or_default();

    Ok(Json(GetAssignedTasksResponse {
        success: true,
        task_ids,
        error: None,
    }))
}

// ============================================================================
// AGENT CLEANUP ENDPOINT
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct GetAgentsToCleanupRequest {
    pub validator_hotkey: String,
    pub signature: String,
    pub timestamp: i64,
}

#[derive(Debug, Serialize)]
pub struct GetAgentsToCleanupResponse {
    pub success: bool,
    pub agents: Vec<String>,
    pub error: Option<String>,
}

/// POST /api/v1/validator/agents_to_cleanup - Get agents that need cleanup
/// Returns agents where submission status is failed/completed/banned/rejected
/// Validators should kill containers and clean up resources for these agents
pub async fn get_agents_to_cleanup(
    State(state): State<Arc<ApiState>>,
    Json(req): Json<GetAgentsToCleanupRequest>,
) -> Result<Json<GetAgentsToCleanupResponse>, (StatusCode, Json<GetAgentsToCleanupResponse>)> {
    // Validate hotkey
    if !is_valid_ss58_hotkey(&req.validator_hotkey) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(GetAgentsToCleanupResponse {
                success: false,
                agents: vec![],
                error: Some("Invalid hotkey format".to_string()),
            }),
        ));
    }

    // Validate timestamp
    if !is_timestamp_valid(req.timestamp) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(GetAgentsToCleanupResponse {
                success: false,
                agents: vec![],
                error: Some("Timestamp expired".to_string()),
            }),
        ));
    }

    // Verify signature
    let message = format!("agents_to_cleanup:{}", req.timestamp);
    #[cfg(debug_assertions)]
    let skip_auth = std::env::var("SKIP_AUTH")
        .map(|v| v == "1")
        .unwrap_or(false);
    #[cfg(not(debug_assertions))]
    let skip_auth = false;
    if !skip_auth && !verify_signature(&req.validator_hotkey, &message, &req.signature) {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(GetAgentsToCleanupResponse {
                success: false,
                agents: vec![],
                error: Some("Invalid signature".to_string()),
            }),
        ));
    }

    // Get agents needing cleanup
    let agents = state
        .storage
        .get_agents_needing_cleanup(&req.validator_hotkey)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(GetAgentsToCleanupResponse {
                    success: false,
                    agents: vec![],
                    error: Some(e.to_string()),
                }),
            )
        })?;

    if !agents.is_empty() {
        info!(
            "Validator {} has {} agents to cleanup: {:?}",
            &req.validator_hotkey[..16.min(req.validator_hotkey.len())],
            agents.len(),
            agents
                .iter()
                .map(|a| &a[..16.min(a.len())])
                .collect::<Vec<_>>()
        );
    }

    Ok(Json(GetAgentsToCleanupResponse {
        success: true,
        agents,
        error: None,
    }))
}

#[derive(Debug, Deserialize)]
pub struct NotifyCleanupCompleteRequest {
    pub validator_hotkey: String,
    pub signature: String,
    pub timestamp: i64,
    pub agent_hash: String,
}

#[derive(Debug, Serialize)]
pub struct NotifyCleanupCompleteResponse {
    pub success: bool,
    pub error: Option<String>,
}

/// POST /api/v1/validator/cleanup_complete - Notify server that cleanup is done
pub async fn notify_cleanup_complete(
    State(state): State<Arc<ApiState>>,
    Json(req): Json<NotifyCleanupCompleteRequest>,
) -> Result<Json<NotifyCleanupCompleteResponse>, (StatusCode, Json<NotifyCleanupCompleteResponse>)>
{
    // Validate hotkey
    if !is_valid_ss58_hotkey(&req.validator_hotkey) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(NotifyCleanupCompleteResponse {
                success: false,
                error: Some("Invalid hotkey format".to_string()),
            }),
        ));
    }

    // Validate timestamp
    if !is_timestamp_valid(req.timestamp) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(NotifyCleanupCompleteResponse {
                success: false,
                error: Some("Timestamp expired".to_string()),
            }),
        ));
    }

    // Verify signature
    let message = format!("cleanup_complete:{}:{}", req.agent_hash, req.timestamp);
    #[cfg(debug_assertions)]
    let skip_auth = std::env::var("SKIP_AUTH")
        .map(|v| v == "1")
        .unwrap_or(false);
    #[cfg(not(debug_assertions))]
    let skip_auth = false;
    if !skip_auth && !verify_signature(&req.validator_hotkey, &message, &req.signature) {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(NotifyCleanupCompleteResponse {
                success: false,
                error: Some("Invalid signature".to_string()),
            }),
        ));
    }

    // Mark assignment as cancelled
    state
        .storage
        .mark_assignment_cancelled(&req.agent_hash, &req.validator_hotkey)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(NotifyCleanupCompleteResponse {
                    success: false,
                    error: Some(e.to_string()),
                }),
            )
        })?;

    info!(
        "Cleanup complete for agent {} by validator {}",
        &req.agent_hash[..16.min(req.agent_hash.len())],
        &req.validator_hotkey[..16.min(req.validator_hotkey.len())]
    );

    Ok(Json(NotifyCleanupCompleteResponse {
        success: true,
        error: None,
    }))
}

// ============================================================================
// AGENT EVALUATION STATUS
// ============================================================================

#[derive(Debug, Serialize)]
pub struct AgentEvalStatusResponse {
    pub agent_hash: String,
    pub status: String,
    pub validators_completed: i32,
    pub total_validators: i32,
    pub window_expires_at: Option<i64>,
    pub evaluations: Vec<ValidatorEvalInfo>,
}

#[derive(Debug, Serialize)]
pub struct ValidatorEvalInfo {
    pub validator_hotkey: String,
    pub score: f64,
    pub tasks_passed: i32,
    pub tasks_total: i32,
}

/// GET /api/v1/validator/agent_status/:agent_hash - Check if agent has been evaluated
pub async fn get_agent_eval_status(
    State(state): State<Arc<ApiState>>,
    Path(agent_hash): Path<String>,
) -> Result<Json<AgentEvalStatusResponse>, (StatusCode, String)> {
    let pending = state
        .storage
        .get_pending_status(&agent_hash)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let evaluations = state
        .storage
        .get_validator_evaluations(&agent_hash)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(AgentEvalStatusResponse {
        agent_hash,
        status: pending
            .as_ref()
            .map(|p| p.status.clone())
            .unwrap_or_else(|| "not_found".to_string()),
        validators_completed: pending
            .as_ref()
            .map(|p| p.validators_completed)
            .unwrap_or(0),
        total_validators: pending.as_ref().map(|p| p.total_validators).unwrap_or(0),
        window_expires_at: pending.as_ref().map(|p| p.window_expires_at),
        evaluations: evaluations
            .into_iter()
            .map(|e| ValidatorEvalInfo {
                validator_hotkey: e.validator_hotkey,
                score: e.score,
                tasks_passed: e.tasks_passed,
                tasks_total: e.tasks_total,
            })
            .collect(),
    }))
}

// ============================================================================
// GET EVALUATION PROGRESS (Resume support)
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct GetProgressRequest {
    pub validator_hotkey: String,
    pub signature: String,
    pub timestamp: i64,
    pub agent_hash: String,
}

#[derive(Debug, Serialize)]
pub struct GetProgressResponse {
    pub success: bool,
    pub agent_hash: String,
    pub total_tasks: i32,
    pub completed_tasks: Vec<CompletedTaskInfo>,
    pub remaining_task_ids: Vec<String>,
    pub partial_score: f64,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CompletedTaskInfo {
    pub task_id: String,
    pub passed: bool,
    pub score: f64,
}

/// POST /api/v1/validator/get_evaluation_progress - Get progress for resuming evaluation
///
/// Returns which tasks have already been completed for this agent by this validator,
/// allowing the validator to skip already-evaluated tasks and resume from where it left off.
pub async fn get_evaluation_progress(
    State(state): State<Arc<ApiState>>,
    Json(req): Json<GetProgressRequest>,
) -> Result<Json<GetProgressResponse>, (StatusCode, Json<GetProgressResponse>)> {
    // Validate hotkey
    if !is_valid_ss58_hotkey(&req.validator_hotkey) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(GetProgressResponse {
                success: false,
                agent_hash: req.agent_hash.clone(),
                total_tasks: 0,
                completed_tasks: vec![],
                remaining_task_ids: vec![],
                partial_score: 0.0,
                error: Some("Invalid hotkey format".to_string()),
            }),
        ));
    }

    // Validate timestamp
    if !is_timestamp_valid(req.timestamp) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(GetProgressResponse {
                success: false,
                agent_hash: req.agent_hash.clone(),
                total_tasks: 0,
                completed_tasks: vec![],
                remaining_task_ids: vec![],
                partial_score: 0.0,
                error: Some("Timestamp expired".to_string()),
            }),
        ));
    }

    // Verify signature
    let message = format!("get_progress:{}:{}", req.agent_hash, req.timestamp);
    #[cfg(debug_assertions)]
    let skip_auth = std::env::var("SKIP_AUTH")
        .map(|v| v == "1")
        .unwrap_or(false);
    #[cfg(not(debug_assertions))]
    let skip_auth = false;
    if !skip_auth && !verify_signature(&req.validator_hotkey, &message, &req.signature) {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(GetProgressResponse {
                success: false,
                agent_hash: req.agent_hash.clone(),
                total_tasks: 0,
                completed_tasks: vec![],
                remaining_task_ids: vec![],
                partial_score: 0.0,
                error: Some("Invalid signature".to_string()),
            }),
        ));
    }

    // Check if validator is authorized
    if !skip_auth && !state.is_authorized_validator(&req.validator_hotkey).await {
        return Err((
            StatusCode::FORBIDDEN,
            Json(GetProgressResponse {
                success: false,
                agent_hash: req.agent_hash.clone(),
                total_tasks: 0,
                completed_tasks: vec![],
                remaining_task_ids: vec![],
                partial_score: 0.0,
                error: Some("Validator not authorized (requires >= 10000 TAO stake)".to_string()),
            }),
        ));
    }

    // Get evaluation progress from storage
    match state
        .storage
        .get_evaluation_progress(&req.agent_hash, &req.validator_hotkey)
        .await
    {
        Ok(progress) => {
            info!(
                "Progress for {} by {}: {}/{} tasks completed",
                &req.agent_hash[..16.min(req.agent_hash.len())],
                &req.validator_hotkey[..16.min(req.validator_hotkey.len())],
                progress.completed_tasks.len(),
                progress.total_tasks
            );
            Ok(Json(GetProgressResponse {
                success: true,
                agent_hash: req.agent_hash,
                total_tasks: progress.total_tasks,
                completed_tasks: progress.completed_tasks,
                remaining_task_ids: progress.remaining_task_ids,
                partial_score: progress.partial_score,
                error: None,
            }))
        }
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(GetProgressResponse {
                success: false,
                agent_hash: req.agent_hash,
                total_tasks: 0,
                completed_tasks: vec![],
                remaining_task_ids: vec![],
                partial_score: 0.0,
                error: Some(format!("Failed to get progress: {}", e)),
            }),
        )),
    }
}

// ============================================================================
// BINARY DOWNLOAD ENDPOINT
// ============================================================================

/// Request for binary download - uses POST for authentication
#[derive(Debug, Deserialize)]
pub struct DownloadBinaryRequest {
    pub validator_hotkey: String,
    pub signature: String,
    pub timestamp: i64,
}

/// POST /api/v1/validator/download_binary/:agent_hash
///
/// Allows assigned validators to download the compiled binary for evaluation.
/// Only validators who are assigned to this agent can download the binary.
///
/// Authentication:
/// - validator_hotkey: SS58 format validator hotkey
/// - signature: sr25519 signature of "download_binary:{agent_hash}:{timestamp}"
/// - timestamp: Unix timestamp (must be within 5 minutes)
///
/// Returns:
/// - Binary file with Content-Type: application/octet-stream
/// - 403 Forbidden if validator is not assigned to this agent
/// - 404 Not Found if binary not compiled yet
pub async fn download_binary(
    State(state): State<Arc<ApiState>>,
    Path(agent_hash): Path<String>,
    Json(req): Json<DownloadBinaryRequest>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    // Validate hotkey format
    if !is_valid_ss58_hotkey(&req.validator_hotkey) {
        return Err((StatusCode::BAD_REQUEST, "Invalid hotkey format".to_string()));
    }

    // Validate timestamp (5 min window)
    if !is_timestamp_valid(req.timestamp) {
        return Err((StatusCode::BAD_REQUEST, "Timestamp expired".to_string()));
    }

    // Verify signature
    let message = format!("download_binary:{}:{}", agent_hash, req.timestamp);
    #[cfg(debug_assertions)]
    let skip_auth = std::env::var("SKIP_AUTH")
        .map(|v| v == "1")
        .unwrap_or(false);
    #[cfg(not(debug_assertions))]
    let skip_auth = false;

    if !skip_auth && !verify_signature(&req.validator_hotkey, &message, &req.signature) {
        warn!(
            "Invalid signature for binary download from {}",
            &req.validator_hotkey[..16.min(req.validator_hotkey.len())]
        );
        return Err((StatusCode::UNAUTHORIZED, "Invalid signature".to_string()));
    }

    // Check if validator is assigned to this agent
    if !skip_auth {
        let is_assigned = state
            .storage
            .is_validator_assigned(&agent_hash, &req.validator_hotkey)
            .await
            .unwrap_or(false);

        if !is_assigned {
            warn!(
                "Validator {} not assigned to agent {}, denying binary download",
                &req.validator_hotkey[..16.min(req.validator_hotkey.len())],
                &agent_hash[..16.min(agent_hash.len())]
            );
            return Err((
                StatusCode::FORBIDDEN,
                "Validator not assigned to this agent".to_string(),
            ));
        }
    }

    // Get binary from database
    let binary = state
        .storage
        .get_binary(&agent_hash)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                "Binary not found or not compiled yet".to_string(),
            )
        })?;

    info!(
        "Validator {} downloading binary for agent {} ({} bytes)",
        &req.validator_hotkey[..16.min(req.validator_hotkey.len())],
        &agent_hash[..16.min(agent_hash.len())],
        binary.len()
    );

    // Return raw binary with appropriate headers
    Ok((
        [
            (header::CONTENT_TYPE, "application/octet-stream".to_string()),
            (header::CONTENT_LENGTH, binary.len().to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!(
                    "attachment; filename=\"{}.bin\"",
                    &agent_hash[..16.min(agent_hash.len())]
                ),
            ),
        ],
        binary,
    ))
}
