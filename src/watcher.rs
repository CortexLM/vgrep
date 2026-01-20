use anyhow::{Context, Result};
use console::style;
use notify::{
    Config as NotifyConfig, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher,
};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::config::Config;
use crate::core::{Database, Indexer, ServerIndexer};
use crate::server::Client;

pub struct FileWatcher {
    config: Config,
    root_path: PathBuf,
    debounce_ms: u64,
}

impl FileWatcher {
    pub fn new(config: Config, root_path: PathBuf) -> Self {
        let debounce_ms = config.watch_debounce_ms;
        Self {
            config,
            root_path,
            debounce_ms,
        }
    }

    pub fn watch(&self) -> Result<()> {
        let abs_path =
            std::fs::canonicalize(&self.root_path).context("Failed to resolve watch path")?;

        // Clean up Windows path prefix
        let display_path = abs_path.to_string_lossy().replace("\\\\?\\", "");

        println!();
        println!("  {} vgrep watcher", style(">>>").green().bold());
        println!("  {} {}", style("Path:").dim(), style(&display_path).cyan());
        println!(
            "  {} {}",
            style("Mode:").dim(),
            if self.config.mode == crate::config::Mode::Server {
                style("server").green()
            } else {
                style("local").yellow()
            }
        );
        println!();
        println!("  {} to stop", style("Ctrl+C").yellow().bold());
        println!();
        println!("{}", style("─".repeat(50)).dim());
        println!();

        // Set up Ctrl+C handler
        let running = Arc::new(AtomicBool::new(true));
        let r = running.clone();

        ctrlc::set_handler(move || {
            r.store(false, Ordering::SeqCst);
        })
        .expect("Error setting Ctrl+C handler");

        // Initial index
        println!("  {} Initial indexing...", style(">>").dim());
        self.index_all()?;

        println!("{}", style("─".repeat(50)).dim());
        println!();

        // Set up watcher
        let (tx, rx) = channel();

        let mut watcher = RecommendedWatcher::new(
            move |res: Result<Event, notify::Error>| {
                if let Ok(event) = res {
                    let _ = tx.send(event);
                }
            },
            NotifyConfig::default().with_poll_interval(Duration::from_millis(self.debounce_ms)),
        )?;

        watcher.watch(&abs_path, RecursiveMode::Recursive)?;

        println!("  {} Watching for changes...", style("[~]").cyan());
        println!();

        // Process events with debouncing
        let pending_files: Arc<Mutex<HashSet<PathBuf>>> = Arc::new(Mutex::new(HashSet::new()));
        let last_process: Arc<Mutex<Instant>> = Arc::new(Mutex::new(Instant::now()));
        let debounce_duration = Duration::from_millis(self.debounce_ms);

        while running.load(Ordering::SeqCst) {
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(event) => {
                    self.handle_event(&event, &pending_files)?;
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    // Check if we should process pending files
                    let should_process = {
                        let last = last_process.lock().unwrap();
                        let pending = pending_files.lock().unwrap();
                        !pending.is_empty() && last.elapsed() >= debounce_duration
                    };

                    if should_process {
                        let files_to_process: Vec<PathBuf> = {
                            let mut pending = pending_files.lock().unwrap();
                            let files: Vec<_> = pending.drain().collect();
                            files
                        };

                        if !files_to_process.is_empty() {
                            self.process_files(&files_to_process)?;
                            *last_process.lock().unwrap() = Instant::now();
                        }
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    break;
                }
            }
        }

        println!();
        println!("  {} Watcher stopped", style("[x]").yellow());
        println!();

        Ok(())
    }

    fn handle_event(
        &self,
        event: &Event,
        pending_files: &Arc<Mutex<HashSet<PathBuf>>>,
    ) -> Result<()> {
        match &event.kind {
            EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_) => {
                let mut pending = pending_files.lock().unwrap();
                for path in &event.paths {
                    if self.should_index(path) {
                        pending.insert(path.clone());
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn should_index(&self, path: &Path) -> bool {
        // Skip directories
        if path.is_dir() {
            return false;
        }

        // Skip hidden files and directories
        if path
            .components()
            .any(|c| c.as_os_str().to_string_lossy().starts_with('.'))
        {
            // But allow .vgrepignore
            if !path.ends_with(".vgrepignore") {
                return false;
            }
        }

        crate::core::index_filter::should_index_path(path, self.config.max_file_size)
    }

    fn index_all(&self) -> Result<()> {
        use crate::config::Mode;

        match self.config.mode {
            Mode::Server => {
                let db = Database::new(&self.config.db_path()?)?;
                let client = Client::new(&self.config.server_host, self.config.server_port);
                let indexer = ServerIndexer::new(db, client, self.config.max_file_size);
                indexer.index_directory(&self.root_path, false)?;
            }
            Mode::Local => {
                if !self.config.has_embedding_model() {
                    anyhow::bail!("Embedding model not found. Run: vgrep models download");
                }
                let db = Database::new(&self.config.db_path()?)?;
                let engine = crate::core::EmbeddingEngine::new(&self.config)?;
                let indexer = Indexer::new(db, engine, self.config.max_file_size);
                indexer.index_directory(&self.root_path, false)?;
            }
        }

        Ok(())
    }

    fn process_files(&self, files: &[PathBuf]) -> Result<()> {
        use crate::config::Mode;

        let db = Database::new(&self.config.db_path()?)?;

        for path in files {
            let filename = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown");

            if !path.exists() {
                // File was deleted
                if let Some(entry) = db.get_file_by_path(path)? {
                    db.delete_file(entry.id)?;
                    println!(
                        "  {} {} {}",
                        style("[-]").red(),
                        style("removed").red(),
                        style(filename).dim()
                    );
                }
                continue;
            }

            // File was created or modified
            let content = match std::fs::read_to_string(path) {
                Ok(c) => c,
                Err(_) => continue, // Skip binary or unreadable files
            };

            if content.is_empty() {
                continue;
            }

            // Delete old entry if exists
            if let Some(entry) = db.get_file_by_path(path)? {
                db.delete_file(entry.id)?;
            }

            // Re-index
            match self.config.mode {
                Mode::Server => {
                    let client = Client::new(&self.config.server_host, self.config.server_port);
                    self.index_file_server(&db, &client, path, &content)?;
                    println!(
                        "  {} {} {}",
                        style("[+]").green(),
                        style("indexed").green(),
                        style(filename).cyan()
                    );
                }
                Mode::Local => {
                    println!(
                        "  {} {} {} {}",
                        style("[~]").yellow(),
                        style("modified").yellow(),
                        style(filename).cyan(),
                        style("(pending)").dim()
                    );
                }
            }
        }

        Ok(())
    }

    fn index_file_server(
        &self,
        db: &Database,
        client: &Client,
        path: &Path,
        content: &str,
    ) -> Result<()> {
        use sha2::{Digest, Sha256};

        let mut hasher = Sha256::new();
        hasher.update(content.as_bytes());
        let hash = hex::encode(hasher.finalize());

        // Chunk the content
        let chunks = self.chunk_content(content);
        if chunks.is_empty() {
            return Ok(());
        }

        let file_id = db.insert_file(path, &hash)?;

        // Get embeddings from server
        let chunk_texts: Vec<&str> = chunks.iter().map(|c| c.content.as_str()).collect();
        let embeddings = client.embed_batch(&chunk_texts)?;

        for (idx, (chunk, embedding)) in chunks.iter().zip(embeddings.iter()).enumerate() {
            db.insert_chunk(
                file_id,
                idx as i32,
                &chunk.content,
                chunk.start_line,
                chunk.end_line,
                embedding,
            )?;
        }

        Ok(())
    }

    fn chunk_content(&self, content: &str) -> Vec<FileChunk> {
        let lines: Vec<&str> = content.lines().collect();

        if lines.is_empty() {
            return Vec::new();
        }

        let mut chunks = Vec::new();
        let mut current_chunk = String::new();
        let mut chunk_start_line = 0;
        let mut char_count = 0;

        for (line_idx, line) in lines.iter().enumerate() {
            let line_len = line.len() + 1;

            if char_count + line_len > self.config.chunk_size && !current_chunk.is_empty() {
                chunks.push(FileChunk {
                    content: current_chunk.clone(),
                    start_line: chunk_start_line as i32,
                    end_line: (line_idx - 1) as i32,
                });

                let overlap_start = if line_idx > 0 {
                    line_idx.saturating_sub(self.config.chunk_overlap / 40)
                } else {
                    0
                };

                current_chunk = lines[overlap_start..line_idx].join("\n");
                if !current_chunk.is_empty() {
                    current_chunk.push('\n');
                }
                chunk_start_line = overlap_start;
                char_count = current_chunk.len();
            }

            if !current_chunk.is_empty() || !line.is_empty() {
                current_chunk.push_str(line);
                current_chunk.push('\n');
                char_count += line_len;
            }
        }

        if !current_chunk.trim().is_empty() {
            chunks.push(FileChunk {
                content: current_chunk,
                start_line: chunk_start_line as i32,
                end_line: (lines.len() - 1) as i32,
            });
        }

        chunks
    }
}

struct FileChunk {
    content: String,
    start_line: i32,
    end_line: i32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_not_index_extensionless_files() {
        let watcher = FileWatcher::new(Config::default(), PathBuf::from("."));
        assert!(!watcher.should_index(Path::new("my_binary")));
    }

    #[test]
    fn should_index_known_extensionless_filenames() {
        let watcher = FileWatcher::new(Config::default(), PathBuf::from("."));
        assert!(watcher.should_index(Path::new("Makefile")));
    }
}
