use anyhow::Result;
use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tower_http::cors::{Any, CorsLayer};

use crate::config::Config;
use crate::core::{Database, EmbeddingEngine};

pub struct ServerState {
    embedding_engine: Mutex<EmbeddingEngine>,
    config: Config,
}

#[derive(Debug, Deserialize)]
pub struct SearchRequest {
    pub query: String,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default = "default_max_results")]
    pub max_results: usize,
}

fn default_max_results() -> usize {
    10
}

#[derive(Debug, Serialize)]
pub struct SearchResponse {
    pub results: Vec<SearchResultJson>,
    pub query: String,
    pub total: usize,
}

#[derive(Debug, Serialize)]
pub struct SearchResultJson {
    pub path: String,
    pub score: f32,
    pub score_percent: String,
    pub preview: Option<String>,
    pub start_line: i32,
    pub end_line: i32,
}

#[derive(Debug, Deserialize)]
pub struct EmbedRequest {
    pub text: String,
}

#[derive(Debug, Deserialize)]
pub struct EmbedBatchRequest {
    pub texts: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct EmbedBatchResponse {
    pub embeddings: Vec<Vec<f32>>,
    #[allow(dead_code)]
    pub count: usize,
}

#[derive(Debug, Serialize)]
pub struct EmbedResponse {
    pub embedding: Vec<f32>,
    pub dimensions: usize,
}

#[derive(Debug, Serialize)]
pub struct StatusResponse {
    pub status: String,
    pub indexed_files: usize,
    pub total_chunks: usize,
    pub embedding_model: Option<String>,
    pub reranker_model: Option<String>,
}

type SharedState = Arc<ServerState>;

pub async fn run_server(config: &Config, host: &str, port: u16) -> Result<()> {
    let config = config.clone();

    if !config.has_embedding_model() {
        anyhow::bail!("Embedding model not found. Please run: vgrep models download");
    }

    crate::ui::print_banner();

    println!("  {}Loading embedding model...", crate::ui::BRAIN);
    let engine = EmbeddingEngine::new(&config)?;
    println!("  {}Model loaded successfully!", crate::ui::CHECK);
    println!();

    let state = Arc::new(ServerState {
        embedding_engine: Mutex::new(engine),
        config,
    });

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let app = Router::new()
        .route("/", get(root))
        .route("/health", get(health))
        .route("/status", get(status))
        .route("/search", post(search))
        .route("/embed", post(embed))
        .route("/embed_batch", post(embed_batch))
        .layer(cors)
        .with_state(state);

    let addr: SocketAddr = format!("{}:{}", host, port).parse()?;

    crate::ui::print_server_banner(host, port);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

async fn root() -> impl IntoResponse {
    Json(serde_json::json!({
        "name": "vgrep",
        "version": env!("CARGO_PKG_VERSION"),
        "description": "Local semantic grep server"
    }))
}

async fn health() -> impl IntoResponse {
    Json(serde_json::json!({
        "status": "ok"
    }))
}

async fn status(State(state): State<SharedState>) -> impl IntoResponse {
    let db = match Database::new(&state.config.db_path().unwrap_or_default()) {
        Ok(db) => db,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": format!("Failed to open database: {}", e)
                })),
            )
                .into_response();
        }
    };

    let stats = match db.get_stats() {
        Ok(stats) => stats,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": format!("Failed to get stats: {}", e)
                })),
            )
                .into_response();
        }
    };

    Json(StatusResponse {
        status: "ok".to_string(),
        indexed_files: stats.file_count,
        total_chunks: stats.chunk_count,
        embedding_model: state
            .config
            .embedding_model_path()
            .ok()
            .map(|p| p.to_string_lossy().to_string()),
        reranker_model: state
            .config
            .reranker_model_path()
            .ok()
            .map(|p| p.to_string_lossy().to_string()),
    })
    .into_response()
}

async fn search(
    State(state): State<SharedState>,
    Json(req): Json<SearchRequest>,
) -> impl IntoResponse {
    let path = req
        .path
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    let abs_path = std::fs::canonicalize(&path).unwrap_or(path);

    // Generate query embedding
    let query_embedding = {
        let engine = match state.embedding_engine.lock() {
            Ok(e) => e,
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({
                        "error": format!("Failed to lock engine: {}", e)
                    })),
                )
                    .into_response();
            }
        };

        match engine.embed(&req.query) {
            Ok(emb) => emb,
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({
                        "error": format!("Failed to generate embedding: {}", e)
                    })),
                )
                    .into_response();
            }
        }
    };

    // Search in database
    let db = match Database::new(&state.config.db_path().unwrap_or_default()) {
        Ok(db) => db,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": format!("Failed to open database: {}", e)
                })),
            )
                .into_response();
        }
    };

    let candidates = match db.search_similar(&query_embedding, &abs_path, req.max_results * 3) {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": format!("Search failed: {}", e)
                })),
            )
                .into_response();
        }
    };

    // Deduplicate by file (keep best chunk per file)
    use std::collections::HashMap;
    let mut best_per_file: HashMap<PathBuf, _> = HashMap::new();

    for result in candidates {
        let entry = best_per_file
            .entry(result.path.clone())
            .or_insert(result.clone());
        if result.similarity > entry.similarity {
            *entry = result;
        }
    }

    // Convert to final results
    let mut results: Vec<SearchResultJson> = best_per_file
        .into_values()
        .map(|r| SearchResultJson {
            path: r.path.to_string_lossy().to_string(),
            score: r.similarity,
            score_percent: format!("{:.2}%", r.similarity * 100.0),
            preview: Some(r.content),
            start_line: r.start_line + 1,
            end_line: r.end_line + 1,
        })
        .collect();

    // Sort by score descending
    results.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    results.truncate(req.max_results);

    let total = results.len();

    Json(SearchResponse {
        results,
        query: req.query,
        total,
    })
    .into_response()
}

async fn embed(
    State(state): State<SharedState>,
    Json(req): Json<EmbedRequest>,
) -> impl IntoResponse {
    let engine = match state.embedding_engine.lock() {
        Ok(e) => e,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": format!("Failed to lock engine: {}", e)
                })),
            )
                .into_response();
        }
    };

    match engine.embed(&req.text) {
        Ok(embedding) => {
            let dimensions = embedding.len();
            Json(EmbedResponse {
                embedding,
                dimensions,
            })
            .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": format!("Embedding failed: {}", e)
            })),
        )
            .into_response(),
    }
}

async fn embed_batch(
    State(state): State<SharedState>,
    Json(req): Json<EmbedBatchRequest>,
) -> impl IntoResponse {
    let engine = match state.embedding_engine.lock() {
        Ok(e) => e,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": format!("Failed to lock engine: {}", e)
                })),
            )
                .into_response();
        }
    };

    let texts: Vec<&str> = req.texts.iter().map(|s| s.as_str()).collect();

    match engine.embed_batch(&texts) {
        Ok(embeddings) => {
            let count = embeddings.len();
            Json(EmbedBatchResponse { embeddings, count }).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": format!("Batch embedding failed: {}", e)
            })),
        )
            .into_response(),
    }
}
