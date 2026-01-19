use anyhow::Result;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::db::{Database, SearchResult as DbSearchResult};
use super::embeddings::EmbeddingEngine;
use crate::config::Config;

pub struct SearchResult {
    pub path: PathBuf,
    pub score: f32,
    pub preview: Option<String>,
    pub start_line: i32,
    pub end_line: i32,
}

pub struct SearchEngine {
    db: Database,
    embedding_engine: EmbeddingEngine,
}

impl SearchEngine {
    pub fn new(
        db: Database,
        embedding_engine: EmbeddingEngine,
        _config: &Config,
        _use_reranker: bool,
    ) -> Result<Self> {
        // Note: Reranker disabled for now as it requires a separate backend
        // which conflicts with the embedding engine's backend
        Ok(Self {
            db,
            embedding_engine,
        })
    }

    pub fn search(
        &self,
        query: &str,
        path: &Path,
        max_results: usize,
    ) -> Result<Vec<SearchResult>> {
        let abs_path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());

        // Generate query embedding
        let query_embedding = self.embedding_engine.embed(query)?;

        // Search for similar chunks
        let candidates = self
            .db
            .search_similar(&query_embedding, &abs_path, max_results * 3)?;

        if candidates.is_empty() {
            return Ok(Vec::new());
        }

        // Deduplicate by file (keep best chunk per file)
        let mut best_per_file: HashMap<PathBuf, DbSearchResult> = HashMap::new();

        for result in candidates {
            let entry = best_per_file
                .entry(result.path.clone())
                .or_insert(result.clone());
            if result.similarity > entry.similarity {
                *entry = result;
            }
        }

        // Convert to final results
        let mut results: Vec<SearchResult> = best_per_file
            .into_values()
            .map(|r| SearchResult {
                path: r.path,
                score: r.similarity,
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

        results.truncate(max_results);

        Ok(results)
    }

    pub fn search_interactive(&self, query: &str, max_results: usize) -> Result<Vec<SearchResult>> {
        let cwd = std::env::current_dir()?;
        self.search(query, &cwd, max_results)
    }

    pub fn embed(&self, text: &str) -> Result<Vec<f32>> {
        self.embedding_engine.embed(text)
    }
}
