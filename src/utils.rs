use std::path::Path;
use std::fs;
use anyhow::Result;

/// Write content to a file atomically by writing to a temp file and renaming it
pub fn write_atomic<P: AsRef<Path>, C: AsRef<[u8]>>(path: P, content: C) -> Result<()> {
    let path = path.as_ref();
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    
    // Create directory if it doesn't exist
    fs::create_dir_all(dir)?;
    
    // Create a temporary file in the same directory to ensure it's on the same filesystem
    // We use a simple strategy: filename.tmp.uuid
    let tmp_file_name = format!("{}.tmp.{}", 
        path.file_name().unwrap_or_default().to_string_lossy(),
        uuid::Uuid::new_v4()
    );
    let tmp_path = dir.join(tmp_file_name);
    
    // Write content to temp file
    match fs::write(&tmp_path, content) {
        Ok(_) => {
            // Rename temp file to destination
            match fs::rename(&tmp_path, path) {
                Ok(_) => Ok(()),
                Err(e) => {
                    // Try to clean up temp file if rename fails
                    let _ = fs::remove_file(&tmp_path);
                    Err(anyhow::anyhow!("Failed to rename temp file to destination: {}", e))
                }
            }
        },
        Err(e) => {
            // Try to clean up temp file if write fails
            let _ = fs::remove_file(&tmp_path);
            Err(anyhow::anyhow!("Failed to write to temp file: {}", e))
        }
    }
}
