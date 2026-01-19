//! Platform API Interface for Challenge Containers
//!
//! This module provides the interface between challenge containers and platform-server.
//!
//! IMPORTANT SECURITY MODEL:
//! - Challenge containers NEVER have access to validator keypairs
//! - All authentication is handled by platform-server
//! - Challenge containers receive data via HTTP from platform-server
//! - Results are sent back to platform-server which handles signing
//!
//! Architecture:
//! ```text
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                    Platform Server                               │
//! │  (handles all auth, keypairs, WebSocket to validators)          │
//! │                                                                  │
//! │  ┌──────────────┐    ┌──────────────┐    ┌──────────────┐      │
//! │  │  Validator   │◄──►│   Platform   │◄──►│  Challenge   │      │
//! │  │  (keypair)   │ WS │   Server     │HTTP│  Container   │      │
//! │  └──────────────┘    └──────────────┘    └──────────────┘      │
//! └─────────────────────────────────────────────────────────────────┘
//! ```
//!
//! The challenge container:
//! 1. Receives submissions via HTTP POST from platform-server
//! 2. Evaluates the agent
//! 3. Returns results via HTTP response
//! 4. Platform-server handles signing and broadcasting

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{debug, error, info, warn};

// ============================================================================
// TYPES FOR CHALLENGE CONTAINER <-> PLATFORM COMMUNICATION
// ============================================================================
//
// NOTE: The authoritative EvaluateRequest/Response definitions are in server.rs
// This file only contains types used by PlatformClient for querying platform-server.
//
// See server.rs for:
// - EvaluateRequest (POST /evaluate input)
// - EvaluateResponse (POST /evaluate output)
// - TaskResultResponse (per-task results)

/// Network state info (read-only for challenge)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkState {
    pub current_epoch: u64,
    pub current_block: u64,
    pub active_validators: u32,
}

/// Leaderboard entry (read-only)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeaderboardEntry {
    pub agent_hash: String,
    pub miner_hotkey: String,
    pub name: Option<String>,
    pub consensus_score: f64,
    pub evaluation_count: u32,
    pub rank: u32,
}

// ============================================================================
// CHALLENGE CONTAINER ROUTES (exposed by term-challenge in server mode)
// ============================================================================

// Routes that the challenge container must expose for platform-server to call:
//
// POST /evaluate
//   - Receives: EvaluateRequest
//   - Returns: EvaluateResponse
//   - Platform-server calls this when a validator needs to evaluate an agent
//
// GET /health
//   - Returns: "OK" or health status
//   - Platform-server uses this to check container is alive
//
// GET /config
//   - Returns: Challenge-specific configuration schema
//   - Used by platform-server to know what config options are available
//
// POST /validate
//   - Receives: { "source_code": "..." }
//   - Returns: { "valid": bool, "errors": [...] }
//   - Quick validation without full evaluation

// ============================================================================
// HELPER FOR CHALLENGE CONTAINERS
// ============================================================================

/// Simple HTTP client for challenge containers to query platform-server.
/// Read-only operations only, no auth needed for public data.
pub struct PlatformClient {
    base_url: String,
    client: reqwest::Client,
}

impl PlatformClient {
    pub fn new(base_url: &str) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            client: reqwest::Client::new(),
        }
    }

    /// Get the base URL
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Get current network state (public endpoint)
    pub async fn get_network_state(&self) -> Result<NetworkState> {
        let resp = self
            .client
            .get(format!("{}/api/v1/network/state", self.base_url))
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err(anyhow!("Failed to get network state: {}", resp.status()));
        }

        Ok(resp.json().await?)
    }

    /// Check if the platform server is healthy
    pub async fn check_health(&self) -> Result<bool> {
        let resp = self
            .client
            .get(format!("{}/health", self.base_url))
            .send()
            .await?;
        
        // Fix for issue #46: Ensure we check the HTTP status code
        // Previously this might have returned true just because the connection succeeded
        if !resp.status().is_success() {
            return Ok(false);
        }

        Ok(true)
    }

    /// Get leaderboard (public endpoint)
    pub async fn get_leaderboard(&self, limit: usize) -> Result<Vec<LeaderboardEntry>> {
        let resp = self
            .client
            .get(format!(
                "{}/api/v1/leaderboard?limit={}",
                self.base_url, limit
            ))
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err(anyhow!("Failed to get leaderboard: {}", resp.status()));
        }

        Ok(resp.json().await?)
    }

    /// Get challenge config (public endpoint)
    pub async fn get_config(&self) -> Result<serde_json::Value> {
        let resp = self
            .client
            .get(format!("{}/api/v1/config", self.base_url))
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err(anyhow!("Failed to get config: {}", resp.status()));
        }

        Ok(resp.json().await?)
    }

    /// Get database snapshot for deterministic weight calculation
    /// Used by /get_weights endpoint
    pub async fn get_snapshot(&self, epoch: Option<u64>) -> Result<SnapshotResponse> {
        let url = match epoch {
            Some(e) => format!("{}/api/v1/data/snapshot?epoch={}", self.base_url, e),
            None => format!("{}/api/v1/data/snapshot", self.base_url),
        };

        let resp = self.client.get(url).send().await?;

        if !resp.status().is_success() {
            return Err(anyhow!("Failed to get snapshot: {}", resp.status()));
        }

        Ok(resp.json().await?)
    }

    /// Claim a task for exclusive processing (Data API)
    pub async fn claim_task(
        &self,
        task_id: &str,
        validator_hotkey: &str,
        ttl_seconds: u64,
    ) -> Result<ClaimTaskResponse> {
        let resp = self
            .client
            .post(format!("{}/api/v1/data/tasks/claim", self.base_url))
            .json(&serde_json::json!({
                "task_id": task_id,
                "validator_hotkey": validator_hotkey,
                "signature": "internal", // Platform-server handles auth for internal calls
                "ttl_seconds": ttl_seconds,
            }))
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err(anyhow!("Failed to claim task: {}", resp.status()));
        }

        Ok(resp.json().await?)
    }

    /// Acknowledge task completion
    pub async fn ack_task(&self, task_id: &str, validator_hotkey: &str) -> Result<bool> {
        let resp = self
            .client
            .post(format!(
                "{}/api/v1/data/tasks/{}/ack",
                self.base_url, task_id
            ))
            .json(&serde_json::json!({
                "validator_hotkey": validator_hotkey,
                "signature": "internal", // Platform-server handles auth for internal calls
            }))
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err(anyhow!("Failed to ack task: {}", resp.status()));
        }

        let result: serde_json::Value = resp.json().await?;
        Ok(result
            .get("success")
            .and_then(|v| v.as_bool())
            .unwrap_or(false))
    }

    /// Write evaluation result to platform server
    pub async fn write_result(&self, result: &WriteResultRequest) -> Result<serde_json::Value> {
        let resp = self
            .client
            .post(format!("{}/api/v1/data/results", self.base_url))
            .json(result)
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err(anyhow!("Failed to write result: {}", resp.status()));
        }

        Ok(resp.json().await?)
    }
}

/// Snapshot response from Data API
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotResponse {
    pub epoch: u64,
    pub snapshot_time: i64,
    pub leaderboard: Vec<SnapshotLeaderboardEntry>,
    pub validators: Vec<SnapshotValidator>,
    pub total_stake: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotLeaderboardEntry {
    pub agent_hash: String,
    pub miner_hotkey: String,
    pub name: Option<String>,
    pub consensus_score: f64,
    pub evaluation_count: u32,
    pub rank: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotValidator {
    pub hotkey: String,
    pub stake: u64,
    pub is_active: bool,
}

/// Claim task response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaimTaskResponse {
    pub success: bool,
    pub lease: Option<TaskLease>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskLease {
    pub task_id: String,
    pub validator_hotkey: String,
    pub claimed_at: i64,
    pub expires_at: i64,
}

/// Write result request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WriteResultRequest {
    pub agent_hash: String,
    pub validator_hotkey: String,
    pub signature: String,
    pub score: f64,
    pub task_results: Option<serde_json::Value>,
    pub execution_time_ms: Option<i64>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::prelude::*;
    use serde_json::json;

    fn client_for(server: &MockServer) -> PlatformClient {
        PlatformClient::new(&server.base_url())
    }

    #[test]
    fn test_base_url_trims_trailing_slash() {
        let client = PlatformClient::new("http://example.com/");
        assert_eq!(client.base_url(), "http://example.com");

        let client2 = PlatformClient::new("http://example.com");
        assert_eq!(client2.base_url(), "http://example.com");
    }

    #[test]
    fn test_snapshot_response_serialization() {
        let resp = SnapshotResponse {
            epoch: 100,
            snapshot_time: 1234567890,
            leaderboard: vec![],
            validators: vec![],
            total_stake: 1000000,
        };

        let json = serde_json::to_string(&resp).unwrap();
        let parsed: SnapshotResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.epoch, 100);
    }

    #[test]
    fn test_network_state_serialization() {
        let state = NetworkState {
            current_epoch: 50,
            current_block: 12345,
            active_validators: 10,
        };

        let json = serde_json::to_string(&state).unwrap();
        let parsed: NetworkState = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.current_epoch, 50);
    }

    #[tokio::test]
    async fn test_get_network_state_success_and_error() {
        let server = MockServer::start();
        let _ok = server.mock(|when, then| {
            when.method(GET).path("/api/v1/network/state");
            then.status(200).json_body(json!({
                "current_epoch": 2,
                "current_block": 42,
                "active_validators": 7
            }));
        });

        let client = client_for(&server);
        let state = client.get_network_state().await.unwrap();
        assert_eq!(state.current_block, 42);

        let err_server = MockServer::start();
        let _err = err_server.mock(|when, then| {
            when.method(GET).path("/api/v1/network/state");
            then.status(503);
        });

        let err_client = client_for(&err_server);
        let err = err_client.get_network_state().await.unwrap_err();
        assert!(err.to_string().contains("Failed to get network state"));
    }

    #[tokio::test]
    async fn test_get_leaderboard_paths() {
        let server = MockServer::start();
        let _ok = server.mock(|when, then| {
            when.method(GET)
                .path("/api/v1/leaderboard")
                .query_param("limit", "5");
            then.status(200).json_body(json!([
                {
                    "agent_hash": "0xabc",
                    "miner_hotkey": "hot",
                    "name": "Agent",
                    "consensus_score": 0.5,
                    "evaluation_count": 10,
                    "rank": 1
                }
            ]));
        });

        let client = client_for(&server);
        let entries = client.get_leaderboard(5).await.unwrap();
        assert_eq!(entries.len(), 1);

        let err_server = MockServer::start();
        let _err = err_server.mock(|when, then| {
            when.method(GET).path("/api/v1/leaderboard");
            then.status(404);
        });

        let err_client = client_for(&err_server);
        let err = err_client.get_leaderboard(5).await.unwrap_err();
        assert!(err.to_string().contains("Failed to get leaderboard"));
    }

    #[tokio::test]
    async fn test_get_config_success_and_error() {
        let server = MockServer::start();
        let _ok = server.mock(|when, then| {
            when.method(GET).path("/api/v1/config");
            then.status(200).json_body(json!({"fields": []}));
        });

        let client = client_for(&server);
        let cfg = client.get_config().await.unwrap();
        assert!(cfg.get("fields").is_some());

        let err_server = MockServer::start();
        let _err = err_server.mock(|when, then| {
            when.method(GET).path("/api/v1/config");
            then.status(401);
        });

        let err_client = client_for(&err_server);
        let err = err_client.get_config().await.unwrap_err();
        assert!(err.to_string().contains("Failed to get config"));
    }

    #[tokio::test]
    async fn test_get_snapshot_with_and_without_epoch() {
        let server = MockServer::start();
        let _with_epoch = server.mock(|when, then| {
            when.method(GET)
                .path("/api/v1/data/snapshot")
                .query_param("epoch", "3");
            then.status(200).json_body(json!({
                "epoch": 3,
                "snapshot_time": 10,
                "leaderboard": [],
                "validators": [],
                "total_stake": 0
            }));
        });

        let client = client_for(&server);
        let snap = client.get_snapshot(Some(3)).await.unwrap();
        assert_eq!(snap.epoch, 3);

        let err_server = MockServer::start();
        let _without_epoch = err_server.mock(|when, then| {
            when.method(GET).path("/api/v1/data/snapshot");
            then.status(500);
        });

        let err_client = client_for(&err_server);
        let err = err_client.get_snapshot(None).await.unwrap_err();
        assert!(err.to_string().contains("Failed to get snapshot"));
    }

    #[tokio::test]
    async fn test_claim_task_success_and_error() {
        let server = MockServer::start();
        let _ok = server.mock(|when, then| {
            when.method(POST)
                .path("/api/v1/data/tasks/claim")
                .json_body(json!({
                    "task_id": "t1",
                    "validator_hotkey": "hotkey",
                    "signature": "internal",
                    "ttl_seconds": 30
                }));
            then.status(200).json_body(json!({
                "success": true,
                "lease": {
                    "task_id": "t1",
                    "validator_hotkey": "hotkey",
                    "claimed_at": 0,
                    "expires_at": 30
                },
                "error": null
            }));
        });

        let client = client_for(&server);
        let resp = client.claim_task("t1", "hotkey", 30).await.unwrap();
        assert!(resp.success);

        let err_server = MockServer::start();
        let _err = err_server.mock(|when, then| {
            when.method(POST).path("/api/v1/data/tasks/claim");
            then.status(429);
        });

        let err_client = client_for(&err_server);
        let err = err_client.claim_task("t1", "hotkey", 30).await.unwrap_err();
        assert!(err.to_string().contains("Failed to claim task"));
    }

    #[tokio::test]
    async fn test_ack_task_success_and_error() {
        let server = MockServer::start();
        let path = "/api/v1/data/tasks/task123/ack";
        let _ok = server.mock(|when, then| {
            when.method(POST).path(path).json_body(json!({
                "validator_hotkey": "hk",
                "signature": "internal"
            }));
            then.status(200).json_body(json!({"success": true}));
        });

        let client = client_for(&server);
        assert!(client.ack_task("task123", "hk").await.unwrap());

        let err_server = MockServer::start();
        let _err = err_server.mock(|when, then| {
            when.method(POST).path(path);
            then.status(400);
        });

        let err_client = client_for(&err_server);
        let err = err_client.ack_task("task123", "hk").await.unwrap_err();
        assert!(err.to_string().contains("Failed to ack task"));
    }

    #[tokio::test]
    async fn test_write_result_success_and_error() {
        let server = MockServer::start();
        let _ok = server.mock(|when, then| {
            when.method(POST)
                .path("/api/v1/data/results")
                .json_body(json!({
                    "agent_hash": "hash",
                    "validator_hotkey": "hk",
                    "signature": "sig",
                    "score": 0.8,
                    "task_results": null,
                    "execution_time_ms": 10
                }));
            then.status(200).json_body(json!({"stored": true}));
        });

        let client = client_for(&server);
        let payload = WriteResultRequest {
            agent_hash: "hash".into(),
            validator_hotkey: "hk".into(),
            signature: "sig".into(),
            score: 0.8,
            task_results: None,
            execution_time_ms: Some(10),
        };
        let resp = client.write_result(&payload).await.unwrap();
        assert_eq!(resp.get("stored").and_then(|v| v.as_bool()), Some(true));

        let err_server = MockServer::start();
        let _err = err_server.mock(|when, then| {
            when.method(POST).path("/api/v1/data/results");
            then.status(502);
        });

        let err_client = client_for(&err_server);
        let err = err_client.write_result(&payload).await.unwrap_err();
        assert!(err.to_string().contains("Failed to write result"));
    }
}
