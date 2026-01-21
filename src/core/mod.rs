//! Core search and indexing functionality.

mod db;
mod embeddings;
pub mod filter;
mod indexer;
mod search;

pub use db::{ChunkEntry, Database, DatabaseStats, FileEntry, SearchResult as DbSearchResult};
pub use embeddings::EmbeddingEngine;
pub use filter::FileFilter;
pub use indexer::{Indexer, ServerIndexer};
pub use search::{SearchEngine, SearchResult};
