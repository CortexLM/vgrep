//! Local semantic code search powered by llama.cpp.
//!
//! νgrεp finds code by meaning, not just keywords. It uses local LLM embeddings
//! to perform semantic similarity search across your codebase.
//!
//! # Quick Start
//!
//! ```bash
//! vgrep init           # Download models
//! vgrep serve          # Start server
//! vgrep watch          # Index and watch for changes
//! vgrep "auth logic"   # Search by meaning
//! ```

pub mod cli;
pub mod config;
pub mod core;
pub mod server;
pub mod ui;
pub mod watcher;
pub mod utils;

pub use config::Config;
pub use core::{Database, EmbeddingEngine, Indexer, SearchEngine, ServerIndexer};
pub use server::Client;
