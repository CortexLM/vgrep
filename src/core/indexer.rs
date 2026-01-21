use anyhow::{Context, Result};
use console::style;
use ignore::WalkBuilder;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

use super::db::Database;
use super::embeddings::EmbeddingEngine;
use crate::ui;

pub struct Indexer {
    db: Database,
    engine: EmbeddingEngine,
    max_file_size: u64,
    chunk_size: usize,
    chunk_overlap: usize,
}

#[derive(Clone)]
struct FileChunk {
    content: String,
    start_line: i32,
    end_line: i32,
}

struct PendingFile {
    path: PathBuf,
    hash: String,
    chunks: Vec<FileChunk>,
}

impl Indexer {
    pub fn new(db: Database, engine: EmbeddingEngine, max_file_size: u64) -> Self {
        Self {
            db,
            engine,
            max_file_size,
            chunk_size: 512,
            chunk_overlap: 64,
        }
    }

    pub fn index_directory(&self, path: &Path, force: bool) -> Result<()> {
        let abs_path = fs::canonicalize(path).context("Failed to resolve path")?;

        println!(
            "  {}Scanning: {}",
            ui::FOLDER,
            style(abs_path.display()).dim()
        );

        // Collect files to index
        let files: Vec<_> = WalkBuilder::new(&abs_path)
            .hidden(false)
            .git_ignore(true)
            .git_global(true)
            .git_exclude(true)
            .build()
            .filter_map(Result::ok)
            .filter(|e| e.file_type().is_some_and(|ft| ft.is_file()))
            .filter(|e| self.should_index(e.path()))
            .collect();

        if files.is_empty() {
            println!("  {}No files to index.", ui::WARN);
            return Ok(());
        }

        println!("  {}Found {} files", ui::FILE, style(files.len()).cyan());
        println!();

        let mp = MultiProgress::new();
        
        // Phase 1: Collect all chunks from all files
        let pb = mp.add(ProgressBar::new(files.len() as u64));
        pb.set_style(
            ProgressStyle::default_bar()
                .template("{spinner:.green} [{bar:30.cyan/blue}] {pos}/{len} Reading files {msg}")?
                .progress_chars("━━─"),
        );
        pb.enable_steady_tick(std::time::Duration::from_millis(100));

        let mut pending_files: Vec<PendingFile> = Vec::new();
        let mut total_chunks = 0;
        let mut skipped = 0;

        for entry in &files {
            let file_path = entry.path();
            let file_name = file_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();
                
            pb.set_message(file_name);

            match self.prepare_file(file_path, force) {
                Ok(Some(pending)) => {
                    total_chunks += pending.chunks.len();
                    pending_files.push(pending);
                }
                Ok(None) => skipped += 1,
                Err(_) => skipped += 1,
            }

            pb.inc(1);
        }

        pb.finish_with_message(format!(
            "Phase 1: Read {} files, {} chunks",
            pending_files.len(),
            total_chunks
        ));

        if pending_files.is_empty() {
            println!();
            println!("  {}All files up to date ({} skipped)", ui::CHECK, skipped);
            return Ok(());
        }

        // Phase 2: Generate embeddings for all chunks at once
        let all_chunks: Vec<&str> = pending_files
            .iter()
            .flat_map(|f| f.chunks.iter().map(|c| c.content.as_str()))
            .collect();

        let pb = mp.add(ProgressBar::new(all_chunks.len() as u64));
        pb.set_style(
            ProgressStyle::default_bar()
                .template("{spinner:.green} [{bar:30.cyan/blue}] {pos}/{len} Generating embeddings ({eta})")?
                .progress_chars("━━─"),
        );
        pb.enable_steady_tick(std::time::Duration::from_millis(100));

        let all_embeddings = self.engine.embed_batch(&all_chunks)?;
        pb.finish_with_message(format!("Phase 2: Generated {} embeddings", all_embeddings.len()));

        // Phase 3: Store in database
        let pb = mp.add(ProgressBar::new(pending_files.len() as u64));
        pb.set_style(
            ProgressStyle::default_bar()
                .template("{spinner:.green} [{bar:30.cyan/blue}] {pos}/{len} Storing in database ({eta})")?
                .progress_chars("━━─"),
        );
        pb.enable_steady_tick(std::time::Duration::from_millis(100));

        let mut embedding_idx = 0;
        let mut indexed = 0;

        for pending in &pending_files {
            // Insert file
            let file_id = self.db.insert_file(&pending.path, &pending.hash)?;

            // Insert chunks with their embeddings
            for (chunk_idx, chunk) in pending.chunks.iter().enumerate() {
                let embedding = &all_embeddings[embedding_idx];
                self.db.insert_chunk(
                    file_id,
                    chunk_idx as i32,
                    &chunk.content,
                    chunk.start_line,
                    chunk.end_line,
                    embedding,
                )?;
                embedding_idx += 1;
            }

            indexed += 1;
            pb.inc(1);
        }

        pb.finish_with_message(format!("Phase 3: Stored {} files", indexed));

        println!();
        println!("  {}Indexing complete!", ui::SPARKLES);
        println!(
            "    {} {} indexed, {} skipped",
            style("Files:").dim(),
            style(indexed).green(),
            skipped
        );
        println!(
            "    {} {}",
            style("Chunks:").dim(),
            style(total_chunks).green()
        );
        println!();

        Ok(())
    }

    fn prepare_file(&self, path: &Path, force: bool) -> Result<Option<PendingFile>> {
        let content = fs::read_to_string(path).context("Failed to read file")?;

        if content.is_empty() {
            return Ok(None);
        }

        let hash = compute_hash(&content);

        // Check if file already indexed with same hash
        if !force {
            if let Some(existing) = self.db.get_file_by_path(path)? {
                if existing.hash == hash {
                    return Ok(None); // Skip, unchanged
                }
                self.db.delete_file(existing.id)?;
            }
        } else if let Some(existing) = self.db.get_file_by_path(path)? {
            self.db.delete_file(existing.id)?;
        }

        let chunks = self.chunk_content(&content);

        if chunks.is_empty() {
            return Ok(None);
        }

        Ok(Some(PendingFile {
            path: path.to_path_buf(),
            hash,
            chunks,
        }))
    }

    fn should_index(&self, path: &Path) -> bool {
        if let Ok(metadata) = fs::metadata(path) {
            if metadata.len() > self.max_file_size {
                return false;
            }
        }

        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();

        matches!(
            ext.as_str(),
            "rs" | "py"
                | "js"
                | "ts"
                | "tsx"
                | "jsx"
                | "go"
                | "c"
                | "cpp"
                | "h"
                | "hpp"
                | "java"
                | "kt"
                | "swift"
                | "rb"
                | "php"
                | "cs"
                | "fs"
                | "scala"
                | "clj"
                | "ex"
                | "exs"
                | "erl"
                | "hs"
                | "ml"
                | "lua"
                | "r"
                | "jl"
                | "dart"
                | "vue"
                | "svelte"
                | "astro"
                | "html"
                | "htm"
                | "css"
                | "scss"
                | "sass"
                | "less"
                | "json"
                | "yaml"
                | "yml"
                | "toml"
                | "xml"
                | "md"
                | "markdown"
                | "txt"
                | "rst"
                | "tex"
                | "sh"
                | "bash"
                | "zsh"
                | "fish"
                | "ps1"
                | "bat"
                | "cmd"
                | "sql"
                | "graphql"
                | "proto"
                | ""
        ) || path.file_name().is_some_and(|n| {
            let name = n.to_string_lossy().to_lowercase();
            matches!(
                name.as_str(),
                "dockerfile"
                    | "makefile"
                    | "cmakelists.txt"
                    | "rakefile"
                    | "gemfile"
                    | "podfile"
                    | "vagrantfile"
                    | ".gitignore"
                    | ".dockerignore"
                    | ".env.example"
                    | "readme"
                    | "license"
                    | "changelog"
            )
        })
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

            if char_count + line_len > self.chunk_size && !current_chunk.is_empty() {
                chunks.push(FileChunk {
                    content: current_chunk.clone(),
                    start_line: chunk_start_line as i32,
                    end_line: (line_idx - 1) as i32,
                });

                let overlap_start = if line_idx > 0 {
                    line_idx.saturating_sub(self.chunk_overlap / 40)
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

fn compute_hash(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    hex::encode(hasher.finalize())
}

// Server-based indexer that uses HTTP API for embeddings
pub struct ServerIndexer {
    db: Database,
    client: crate::server::Client,
    max_file_size: u64,
    chunk_size: usize,
    chunk_overlap: usize,
}

impl ServerIndexer {
    pub fn new(db: Database, client: crate::server::Client, max_file_size: u64) -> Self {
        Self {
            db,
            client,
            max_file_size,
            chunk_size: 512,
            chunk_overlap: 64,
        }
    }

    pub fn index_directory(&self, path: &Path, force: bool) -> Result<()> {
        let abs_path = fs::canonicalize(path).context("Failed to resolve path")?;

        println!(
            "  {}Scanning: {}",
            ui::FOLDER,
            style(abs_path.display()).dim()
        );
        println!("  {}Using server for embeddings", ui::SERVER);

        let files: Vec<_> = WalkBuilder::new(&abs_path)
            .hidden(false)
            .git_ignore(true)
            .git_global(true)
            .git_exclude(true)
            .build()
            .filter_map(Result::ok)
            .filter(|e| e.file_type().is_some_and(|ft| ft.is_file()))
            .filter(|e| self.should_index(e.path()))
            .collect();

        if files.is_empty() {
            println!("  {}No files to index.", ui::WARN);
            return Ok(());
        }

        println!();

        let mp = MultiProgress::new();
        
        // Phase 1: Collect all chunks
        let pb = mp.add(ProgressBar::new(files.len() as u64));
        pb.set_style(
            ProgressStyle::default_bar()
                .template("{spinner:.green} [{bar:30.cyan/blue}] {pos}/{len} Reading files {msg}")?
                .progress_chars("━━─"),
        );
        pb.enable_steady_tick(std::time::Duration::from_millis(100));

        let mut pending_files: Vec<PendingFile> = Vec::new();
        let mut total_chunks = 0;
        let mut skipped = 0;

        for entry in &files {
            let file_path = entry.path();
            let file_name = file_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();
                
            pb.set_message(file_name);

            match self.prepare_file(file_path, force) {
                Ok(Some(pending)) => {
                    total_chunks += pending.chunks.len();
                    pending_files.push(pending);
                }
                Ok(None) => skipped += 1,
                Err(_) => skipped += 1,
            }

            pb.inc(1);
        }

        pb.finish_with_message(format!(
            "Phase 1: Read {} files, {} chunks",
            pending_files.len(),
            total_chunks
        ));

        if pending_files.is_empty() {
            println!();
            println!("  {}All files up to date ({} skipped)", ui::CHECK, skipped);
            return Ok(());
        }

        // Phase 2: Generate embeddings via server
        let all_chunks: Vec<String> = pending_files
            .iter()
            .flat_map(|f| f.chunks.iter().map(|c| c.content.clone()))
            .collect();

        let batch_size = 50; 
        let mut all_embeddings: Vec<Vec<f32>> = Vec::with_capacity(all_chunks.len());

        let pb = mp.add(ProgressBar::new(all_chunks.len() as u64));
        pb.set_style(
            ProgressStyle::default_bar()
                .template("{spinner:.green} [{bar:30.cyan/blue}] {pos}/{len} Generating embeddings ({eta})")?
                .progress_chars("━━─"),
        );
        pb.enable_steady_tick(std::time::Duration::from_millis(100));

        for batch in all_chunks.chunks(batch_size) {
            let batch_refs: Vec<&str> = batch.iter().map(|s| s.as_str()).collect();
            let embeddings = self.client.embed_batch(&batch_refs)?;
            all_embeddings.extend(embeddings);
            pb.inc(batch.len() as u64);
        }

        pb.finish_with_message(format!("Phase 2: Generated {} embeddings", all_embeddings.len()));

        // Phase 3: Store in database
        let pb = mp.add(ProgressBar::new(pending_files.len() as u64));
        pb.set_style(
            ProgressStyle::default_bar()
                .template("{spinner:.green} [{bar:30.cyan/blue}] {pos}/{len} Storing in database ({eta})")?
                .progress_chars("━━─"),
        );
        pb.enable_steady_tick(std::time::Duration::from_millis(100));

        let mut embedding_idx = 0;
        let mut indexed = 0;

        for pending in &pending_files {
            let file_id = self.db.insert_file(&pending.path, &pending.hash)?;

            for (chunk_idx, chunk) in pending.chunks.iter().enumerate() {
                let embedding = &all_embeddings[embedding_idx];
                self.db.insert_chunk(
                    file_id,
                    chunk_idx as i32,
                    &chunk.content,
                    chunk.start_line,
                    chunk.end_line,
                    embedding,
                )?;
                embedding_idx += 1;
            }

            indexed += 1;
            pb.inc(1);
        }

        pb.finish_with_message(format!("Phase 3: Stored {} files", indexed));

        println!();
        println!("  {}Indexing complete!", ui::SPARKLES);
        println!(
            "    {} {} indexed, {} skipped",
            style("Files:").dim(),
            style(indexed).green(),
            skipped
        );
        println!(
            "    {} {}",
            style("Chunks:").dim(),
            style(total_chunks).green()
        );
        println!();

        Ok(())
    }

    fn prepare_file(&self, path: &Path, force: bool) -> Result<Option<PendingFile>> {
        let content = fs::read_to_string(path).context("Failed to read file")?;

        if content.is_empty() {
            return Ok(None);
        }

        let hash = compute_hash(&content);

        if !force {
            if let Some(existing) = self.db.get_file_by_path(path)? {
                if existing.hash == hash {
                    return Ok(None);
                }
                self.db.delete_file(existing.id)?;
            }
        } else if let Some(existing) = self.db.get_file_by_path(path)? {
            self.db.delete_file(existing.id)?;
        }

        let chunks = self.chunk_content(&content);

        if chunks.is_empty() {
            return Ok(None);
        }

        Ok(Some(PendingFile {
            path: path.to_path_buf(),
            hash,
            chunks,
        }))
    }

    fn should_index(&self, path: &Path) -> bool {
        if let Ok(metadata) = fs::metadata(path) {
            if metadata.len() > self.max_file_size {
                return false;
            }
        }

        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();

        matches!(
            ext.as_str(),
            "rs" | "py"
                | "js"
                | "ts"
                | "tsx"
                | "jsx"
                | "go"
                | "c"
                | "cpp"
                | "h"
                | "hpp"
                | "java"
                | "kt"
                | "swift"
                | "rb"
                | "php"
                | "cs"
                | "fs"
                | "scala"
                | "clj"
                | "ex"
                | "exs"
                | "erl"
                | "hs"
                | "ml"
                | "lua"
                | "r"
                | "jl"
                | "dart"
                | "vue"
                | "svelte"
                | "astro"
                | "html"
                | "htm"
                | "css"
                | "scss"
                | "sass"
                | "less"
                | "json"
                | "yaml"
                | "yml"
                | "toml"
                | "xml"
                | "md"
                | "markdown"
                | "txt"
                | "rst"
                | "tex"
                | "sh"
                | "bash"
                | "zsh"
                | "fish"
                | "ps1"
                | "bat"
                | "cmd"
                | "sql"
                | "graphql"
                | "proto"
                | ""
        ) || path.file_name().is_some_and(|n| {
            let name = n.to_string_lossy().to_lowercase();
            matches!(
                name.as_str(),
                "dockerfile"
                    | "makefile"
                    | "cmakelists.txt"
                    | "rakefile"
                    | "gemfile"
                    | "podfile"
                    | "vagrantfile"
                    | ".gitignore"
                    | ".dockerignore"
                    | ".env.example"
                    | "readme"
                    | "license"
                    | "changelog"
            )
        })
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

            if char_count + line_len > self.chunk_size && !current_chunk.is_empty() {
                chunks.push(FileChunk {
                    content: current_chunk.clone(),
                    start_line: chunk_start_line as i32,
                    end_line: (line_idx - 1) as i32,
                });

                let overlap_start = if line_idx > 0 {
                    line_idx.saturating_sub(self.chunk_overlap / 40)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_hash() {
        let hash1 = compute_hash("hello");
        let hash2 = compute_hash("hello");
        let hash3 = compute_hash("world");

        assert_eq!(hash1, hash2);
        assert_ne!(hash1, hash3);
    }
}
