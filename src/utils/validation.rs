use anyhow::{Result, Context};
use std::fs;
use std::path::Path;

// Helper function to validate a model file
pub fn validate_model_file(path: &Path, expected_size_min: u64) -> Result<()> {
    // 1. Check if file exists
    if !path.exists() {
        anyhow::bail!("Model file does not exist at {}", path.display());
    }

    // 2. Check file size
    let metadata = fs::metadata(path)?;
    if metadata.len() < expected_size_min {
        anyhow::bail!(
            "Model file size too small: {} bytes (expected at least {})",
            metadata.len(),
            expected_size_min
        );
    }
    
    // 3. Try to verify GGUF header (first 4 bytes should be 'GGUF')
    let mut file = fs::File::open(path)?;
    use std::io::Read;
    let mut buffer = [0u8; 4];
    file.read_exact(&mut buffer).context("Failed to read file header")?;
    
    if &buffer != b"GGUF" {
         anyhow::bail!("Invalid file format: Header is not 'GGUF'");
    }

    Ok(())
}
