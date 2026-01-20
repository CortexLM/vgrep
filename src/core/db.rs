use anyhow::Result;
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};
use std::path::{Path, PathBuf};

pub struct Database {
    conn: Connection,
}

#[derive(Debug, Clone)]
pub struct FileEntry {
    pub id: i64,
    pub path: PathBuf,
    pub hash: String,
    pub indexed_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct ChunkEntry {
    pub id: i64,
    pub file_id: i64,
    pub chunk_index: i32,
    pub content: String,
    pub start_line: i32,
    pub end_line: i32,
    pub embedding: Vec<f32>,
}

#[derive(Debug, Clone)]
pub struct SearchResult {
    pub file_id: i64,
    pub chunk_id: i64,
    pub path: PathBuf,
    pub content: String,
    pub start_line: i32,
    pub end_line: i32,
    pub similarity: f32,
}

#[derive(Debug, Default)]
pub struct DatabaseStats {
    pub file_count: usize,
    pub chunk_count: usize,
    pub last_indexed: Option<DateTime<Utc>>,
}

impl Database {
    pub fn new(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        let db = Self { conn };
        db.init_schema()?;
        Ok(db)
    }

    fn init_schema(&self) -> Result<()> {
        self.conn.execute_batch(
            r"
            CREATE TABLE IF NOT EXISTS files (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                path TEXT UNIQUE NOT NULL,
                hash TEXT NOT NULL,
                indexed_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS chunks (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                file_id INTEGER NOT NULL,
                chunk_index INTEGER NOT NULL,
                content TEXT NOT NULL,
                start_line INTEGER NOT NULL,
                end_line INTEGER NOT NULL,
                embedding BLOB NOT NULL,
                FOREIGN KEY (file_id) REFERENCES files(id) ON DELETE CASCADE,
                UNIQUE(file_id, chunk_index)
            );

            CREATE INDEX IF NOT EXISTS idx_files_path ON files(path);
            CREATE INDEX IF NOT EXISTS idx_files_hash ON files(hash);
            CREATE INDEX IF NOT EXISTS idx_chunks_file_id ON chunks(file_id);
            ",
        )?;
        Ok(())
    }

    pub fn get_file_by_path(&self, path: &Path) -> Result<Option<FileEntry>> {
        let path_str = path.to_string_lossy();
        let mut stmt = self
            .conn
            .prepare("SELECT id, path, hash, indexed_at FROM files WHERE path = ?")?;

        let result = stmt.query_row([path_str.as_ref()], |row| {
            Ok(FileEntry {
                id: row.get(0)?,
                path: PathBuf::from(row.get::<_, String>(1)?),
                hash: row.get(2)?,
                indexed_at: row.get::<_, String>(3)?.parse().unwrap_or_default(),
            })
        });

        match result {
            Ok(entry) => Ok(Some(entry)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn insert_file(&self, path: &Path, hash: &str) -> Result<i64> {
        let path_str = path.to_string_lossy();
        let now = Utc::now().to_rfc3339();

        self.conn.execute(
            "INSERT OR REPLACE INTO files (path, hash, indexed_at) VALUES (?, ?, ?)",
            params![path_str.as_ref(), hash, now],
        )?;

        Ok(self.conn.last_insert_rowid())
    }

    pub fn delete_file(&self, file_id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM chunks WHERE file_id = ?", params![file_id])?;
        self.conn
            .execute("DELETE FROM files WHERE id = ?", params![file_id])?;
        Ok(())
    }

    pub fn insert_chunk(
        &self,
        file_id: i64,
        chunk_index: i32,
        content: &str,
        start_line: i32,
        end_line: i32,
        embedding: &[f32],
    ) -> Result<i64> {
        let embedding_bytes = embedding_to_bytes(embedding);

        self.conn.execute(
            r"INSERT OR REPLACE INTO chunks 
              (file_id, chunk_index, content, start_line, end_line, embedding) 
              VALUES (?, ?, ?, ?, ?, ?)",
            params![
                file_id,
                chunk_index,
                content,
                start_line,
                end_line,
                embedding_bytes
            ],
        )?;

        Ok(self.conn.last_insert_rowid())
    }

    pub fn search_similar(
        &self,
        query_embedding: &[f32],
        path_prefix: &Path,
        limit: usize,
    ) -> Result<Vec<SearchResult>> {
        let path_prefix_str = path_prefix.to_string_lossy();
        
        // Ensure the prefix ends with a directory separator to avoid partial directory matching
        // e.g. "/home/user/proj" should become "/home/user/proj/" to avoid matching "/home/user/project"
        let like_pattern = if path_prefix_str.ends_with(std::path::MAIN_SEPARATOR) {
            format!("{}%", path_prefix_str)
        } else {
            format!("{}{}%", path_prefix_str, std::path::MAIN_SEPARATOR)
        };

        let mut stmt = self.conn.prepare(
            r"SELECT c.id, c.file_id, f.path, c.content, c.start_line, c.end_line, c.embedding
              FROM chunks c
              JOIN files f ON c.file_id = f.id
              WHERE f.path LIKE ?",
        )?;

        let mut results: Vec<SearchResult> = stmt
            .query_map([&like_pattern], |row| {
                let embedding_blob: Vec<u8> = row.get(6)?;
                let embedding = bytes_to_embedding(&embedding_blob);
                let similarity = cosine_similarity(query_embedding, &embedding);

                Ok(SearchResult {
                    chunk_id: row.get(0)?,
                    file_id: row.get(1)?,
                    path: PathBuf::from(row.get::<_, String>(2)?),
                    content: row.get(3)?,
                    start_line: row.get(4)?,
                    end_line: row.get(5)?,
                    similarity,
                })
            })?
            .filter_map(Result::ok)
            .collect();

        // Sort by similarity (highest first)
        results.sort_by(|a, b| {
            b.similarity
                .partial_cmp(&a.similarity)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        results.truncate(limit * 3); // Get more for reranking
        Ok(results)
    }

    pub fn get_all_chunks_for_file(&self, file_id: i64) -> Result<Vec<ChunkEntry>> {
        let mut stmt = self.conn.prepare(
            r"SELECT id, file_id, chunk_index, content, start_line, end_line, embedding
              FROM chunks WHERE file_id = ? ORDER BY chunk_index",
        )?;

        let results = stmt
            .query_map([file_id], |row| {
                let embedding_blob: Vec<u8> = row.get(6)?;
                Ok(ChunkEntry {
                    id: row.get(0)?,
                    file_id: row.get(1)?,
                    chunk_index: row.get(2)?,
                    content: row.get(3)?,
                    start_line: row.get(4)?,
                    end_line: row.get(5)?,
                    embedding: bytes_to_embedding(&embedding_blob),
                })
            })?
            .filter_map(Result::ok)
            .collect();

        Ok(results)
    }

    pub fn get_stats(&self) -> Result<DatabaseStats> {
        let file_count: usize = self
            .conn
            .query_row("SELECT COUNT(*) FROM files", [], |row| row.get(0))?;

        let chunk_count: usize = self
            .conn
            .query_row("SELECT COUNT(*) FROM chunks", [], |row| row.get(0))?;

        let last_indexed: Option<String> = self
            .conn
            .query_row(
                "SELECT indexed_at FROM files ORDER BY indexed_at DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .ok();

        Ok(DatabaseStats {
            file_count,
            chunk_count,
            last_indexed: last_indexed.and_then(|s| s.parse().ok()),
        })
    }

    pub fn get_all_files(&self) -> Result<Vec<FileEntry>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, path, hash, indexed_at FROM files")?;

        let results = stmt
            .query_map([], |row| {
                Ok(FileEntry {
                    id: row.get(0)?,
                    path: PathBuf::from(row.get::<_, String>(1)?),
                    hash: row.get(2)?,
                    indexed_at: row.get::<_, String>(3)?.parse().unwrap_or_default(),
                })
            })?
            .filter_map(Result::ok)
            .collect();

        Ok(results)
    }
}

fn embedding_to_bytes(embedding: &[f32]) -> Vec<u8> {
    embedding.iter().flat_map(|f| f.to_le_bytes()).collect()
}

fn bytes_to_embedding(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|chunk| {
            let arr: [u8; 4] = chunk.try_into().unwrap();
            f32::from_le_bytes(arr)
        })
        .collect()
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }

    let (dot, norm_a, norm_b) = a.iter().zip(b.iter()).fold(
        (0.0f64, 0.0f64, 0.0f64),
        |(dot, norm_a, norm_b), (&x, &y)| {
            let x = f64::from(x);
            let y = f64::from(y);
            (dot + x * y, norm_a + x * x, norm_b + y * y)
        },
    );

    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }

    (dot / (norm_a.sqrt() * norm_b.sqrt())) as f32
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_path_prefix_search_bug_reproduction() -> Result<()> {
        let dir = tempdir()?;
        let db_path = dir.path().join("test.db");
        let db = Database::new(&db_path)?;

        // Setup paths
        let base_path = PathBuf::from("/home/user");
        let path1 = base_path.join("proj/file1.txt");
        let path2 = base_path.join("project/file2.txt");

        // Insert files
        let id1 = db.insert_file(&path1, "hash1")?;
        let id2 = db.insert_file(&path2, "hash2")?;

        // Insert dummy chunks
        let embedding = vec![0.1; 128];
        db.insert_chunk(id1, 0, "content1", 1, 1, &embedding)?;
        db.insert_chunk(id2, 0, "content2", 1, 1, &embedding)?;

        // Search with prefix "/home/user/proj"
        // This should match ONLY "/home/user/proj/file1.txt"
        // But due to the bug, it likely matches "/home/user/project/file2.txt" as well
        let prefix = base_path.join("proj");
        let results = db.search_similar(&embedding, &prefix, 10)?;

        let found_paths: Vec<String> = results.iter()
            .map(|r| r.path.to_string_lossy().to_string())
            .collect();
            
        println!("Found paths: {:?}", found_paths);
        
        // Assert that we DO NOT match "project" when searching for "proj"
        let matched_project = found_paths.iter().any(|p| p.contains("project"));
        assert!(!matched_project, "Should not match sibling directory 'project' when searching for 'proj'");
        
        Ok(())
    }
}
