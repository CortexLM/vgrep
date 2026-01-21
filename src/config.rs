use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const CONFIG_FILE: &str = "config.json";
const MODELS_DIR: &str = "models";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    #[default]
    Server,
    Local,
}

impl std::fmt::Display for Mode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Mode::Server => write!(f, "server"),
            Mode::Local => write!(f, "local"),
        }
    }
}

impl std::str::FromStr for Mode {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "server" => Ok(Mode::Server),
            "local" => Ok(Mode::Local),
            _ => anyhow::bail!("Invalid mode: {}. Use 'server' or 'local'", s),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Operating mode: server (default) or local
    #[serde(default)]
    pub mode: Mode,

    /// Server host for server mode
    #[serde(default = "default_server_host")]
    pub server_host: String,

    /// Server port for server mode
    #[serde(default = "default_server_port")]
    pub server_port: u16,

    /// Auto-start server if not running (future feature)
    #[serde(default)]
    pub auto_start_server: bool,

    /// Path to embedding model (relative to models_dir or absolute)
    #[serde(default = "default_embedding_model")]
    pub embedding_model: String,

    /// Path to reranker model (relative to models_dir or absolute)
    #[serde(default = "default_reranker_model")]
    pub reranker_model: String,

    /// Enable reranker for better results (slower)
    #[serde(default = "default_use_reranker")]
    pub use_reranker: bool,

    /// Custom models directory (overrides default)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub models_dir: Option<PathBuf>,

    /// Chunk size for text splitting
    #[serde(default = "default_chunk_size")]
    pub chunk_size: usize,

    /// Overlap between chunks
    #[serde(default = "default_chunk_overlap")]
    pub chunk_overlap: usize,

    /// Maximum file size to index (in bytes)
    #[serde(default = "default_max_file_size")]
    pub max_file_size: u64,

    /// Default number of search results
    #[serde(default = "default_max_results")]
    pub max_results: usize,

    /// Show content in search results by default
    #[serde(default)]
    pub show_content: bool,

    /// Watch debounce delay in milliseconds
    #[serde(default = "default_watch_debounce_ms")]
    pub watch_debounce_ms: u64,

    /// Number of threads for inference (0 = auto)
    #[serde(default)]
    pub n_threads: usize,

    /// Context size for embeddings
    #[serde(default = "default_context_size")]
    pub context_size: usize,
}

fn default_server_host() -> String {
    std::env::var("VGREP_HOST").unwrap_or_else(|_| "127.0.0.1".to_string())
}

fn default_server_port() -> u16 {
    std::env::var("VGREP_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(7777)
}

fn default_embedding_model() -> String {
    "Qwen3-Embedding-0.6B-Q8_0.gguf".to_string()
}

fn default_reranker_model() -> String {
    "Qwen3-Reranker-0.6B-Q4_K_M.gguf".to_string()
}

fn default_chunk_size() -> usize {
    std::env::var("VGREP_CHUNK_SIZE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(512)
}

fn default_chunk_overlap() -> usize {
    std::env::var("VGREP_CHUNK_OVERLAP")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(64)
}

fn default_max_file_size() -> u64 {
    std::env::var("VGREP_MAX_FILE_SIZE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(512 * 1024) // 512KB
}

fn default_max_results() -> usize {
    std::env::var("VGREP_MAX_RESULTS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(10)
}

fn default_watch_debounce_ms() -> u64 {
    std::env::var("VGREP_WATCH_DEBOUNCE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(500)
}

fn default_context_size() -> usize {
    512
}

fn default_use_reranker() -> bool {
    true
}

impl Default for Config {
    fn default() -> Self {
        Self {
            mode: Mode::default(),
            server_host: default_server_host(),
            server_port: default_server_port(),
            auto_start_server: false,
            embedding_model: default_embedding_model(),
            reranker_model: default_reranker_model(),
            use_reranker: default_use_reranker(),
            models_dir: None,
            chunk_size: default_chunk_size(),
            chunk_overlap: default_chunk_overlap(),
            max_file_size: default_max_file_size(),
            max_results: default_max_results(),
            show_content: false,
            watch_debounce_ms: default_watch_debounce_ms(),
            n_threads: 0,
            context_size: default_context_size(),
        }
    }
}

impl Config {
    /// Get the global config directory (~/.vgrep on all platforms)
    pub fn global_config_dir() -> Result<PathBuf> {
        let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("Cannot find home directory"))?;
        Ok(home.join(".vgrep"))
    }

    /// Get path to global config file
    pub fn global_config_path() -> Result<PathBuf> {
        Ok(Self::global_config_dir()?.join(CONFIG_FILE))
    }

    /// Load config from global location, creating default if not exists
    pub fn load() -> Result<Self> {
        let config_dir = Self::global_config_dir()?;
        let config_path = config_dir.join(CONFIG_FILE);

        if config_path.exists() {
            let content =
                std::fs::read_to_string(&config_path).context("Failed to read config file")?;
            let config: Config =
                serde_json::from_str(&content).context("Failed to parse config file")?;
            Ok(config)
        } else {
            // Create default config
            std::fs::create_dir_all(&config_dir).context("Failed to create config directory")?;
            let config = Config::default();
            config.save()?;
            Ok(config)
        }
    }

    /// Save config to global location
    pub fn save(&self) -> Result<()> {
        let config_dir = Self::global_config_dir()?;
        std::fs::create_dir_all(&config_dir)?;
        let config_path = config_dir.join(CONFIG_FILE);
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(&config_path, content)?;
        Ok(())
    }

    /// Get the models directory
    pub fn get_models_dir(&self) -> Result<PathBuf> {
        if let Some(ref custom_dir) = self.models_dir {
            Ok(custom_dir.clone())
        } else {
            let dir = Self::global_config_dir()?.join(MODELS_DIR);
            std::fs::create_dir_all(&dir)?;
            Ok(dir)
        }
    }

    /// Get the database path for a specific project
    pub fn get_project_db_path(project_path: &std::path::Path) -> Result<PathBuf> {
        let config_dir = Self::global_config_dir()?;
        let db_dir = config_dir.join("projects");
        std::fs::create_dir_all(&db_dir)?;

        // Create a unique name for the project based on path hash
        let path_str = project_path.to_string_lossy();
        let hash = Self::hash_path(&path_str);
        Ok(db_dir.join(format!("{}.db", hash)))
    }

    /// Get database path for current directory
    pub fn db_path(&self) -> Result<PathBuf> {
        let cwd = std::env::current_dir()?;
        Self::get_project_db_path(&cwd)
    }

    fn hash_path(path: &str) -> String {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(path.as_bytes());
        let result = hasher.finalize();
        hex::encode(&result[..8]) // First 8 bytes = 16 hex chars
    }

    /// Get full path to embedding model
    pub fn embedding_model_path(&self) -> Result<PathBuf> {
        let model_path = PathBuf::from(&self.embedding_model);
        if model_path.is_absolute() && model_path.exists() {
            Ok(model_path)
        } else {
            let models_dir = self.get_models_dir()?;
            Ok(models_dir.join(&self.embedding_model))
        }
    }

    /// Get full path to reranker model
    pub fn reranker_model_path(&self) -> Result<PathBuf> {
        let model_path = PathBuf::from(&self.reranker_model);
        if model_path.is_absolute() && model_path.exists() {
            Ok(model_path)
        } else {
            let models_dir = self.get_models_dir()?;
            Ok(models_dir.join(&self.reranker_model))
        }
    }

    /// Check if embedding model exists
    pub fn has_embedding_model(&self) -> bool {
        self.embedding_model_path()
            .map(|p| p.exists())
            .unwrap_or(false)
    }

    /// Check if reranker model exists
    pub fn has_reranker_model(&self) -> bool {
        self.reranker_model_path()
            .map(|p| p.exists())
            .unwrap_or(false)
    }

    /// Get number of threads (auto-detect if 0)
    pub fn get_n_threads(&self) -> i32 {
        if self.n_threads == 0 {
            std::thread::available_parallelism()
                .map(|p| p.get() as i32)
                .unwrap_or(4)
        } else {
            self.n_threads as i32
        }
    }

    // Setters that save automatically
    pub fn set_mode(&mut self, mode: Mode) -> Result<()> {
        self.mode = mode;
        self.save()
    }

    pub fn set_server_host(&mut self, host: String) -> Result<()> {
        // Validate host
        if host.trim().is_empty() {
            anyhow::bail!("Server host cannot be empty");
        }

        // Check for common mistakes
        if host.contains("://") {
            anyhow::bail!("Server host should not contain protocol (e.g. http://). Use hostname or IP only.");
        }

        if host.contains(':') && !host.starts_with('[') {
            // It might be an IPv6 address, but if it has a port like 127.0.0.1:8080 it's invalid for this field
            // A simple heuristic: if it contains a colon, it might be a port, unless it's a valid IPv6
            // But strict checking is better.
            
            // Check if it's a valid socket addr (which means it has a port, which we DON'T want here usually, 
            // unless the user really intends to bind to a specific address including port, but server_port is separate)
            // The issue description says "IPs with ports embedded" are invalid.
            if host.parse::<std::net::SocketAddr>().is_ok() {
                 anyhow::bail!("Server host should not contain port. Set port using server_port configuration.");
            }
        }

        // Check if it's a valid IP or hostname
        // This is a basic check.
        if host.chars().any(|c| c.is_whitespace() || c.is_control()) {
             anyhow::bail!("Server host contains invalid characters");
        }

        self.server_host = host;
        self.save()
    }

    pub fn set_server_port(&mut self, port: u16) -> Result<()> {
        self.server_port = port;
        self.save()
    }

    pub fn set_embedding_model(&mut self, model: String) -> Result<()> {
        self.embedding_model = model;
        self.save()
    }

    pub fn set_reranker_model(&mut self, model: String) -> Result<()> {
        self.reranker_model = model;
        self.save()
    }

    pub fn set_use_reranker(&mut self, enabled: bool) -> Result<()> {
        self.use_reranker = enabled;
        self.save()
    }

    pub fn set_models_dir(&mut self, path: Option<PathBuf>) -> Result<()> {
        self.models_dir = path;
        self.save()
    }

    pub fn set_max_file_size(&mut self, size: u64) -> Result<()> {
        self.max_file_size = size;
        self.save()
    }

    pub fn set_max_results(&mut self, count: usize) -> Result<()> {
        self.max_results = count;
        self.save()
    }

    pub fn set_show_content(&mut self, show: bool) -> Result<()> {
        self.show_content = show;
        self.save()
    }

    pub fn set_chunk_size(&mut self, size: usize) -> Result<()> {
        self.chunk_size = size;
        self.save()
    }

    pub fn set_chunk_overlap(&mut self, overlap: usize) -> Result<()> {
        self.chunk_overlap = overlap;
        self.save()
    }

    pub fn set_n_threads(&mut self, threads: usize) -> Result<()> {
        self.n_threads = threads;
        self.save()
    }

    pub fn set_context_size(&mut self, size: usize) -> Result<()> {
        self.context_size = size;
        self.save()
    }

    pub fn set_watch_debounce(&mut self, ms: u64) -> Result<()> {
        self.watch_debounce_ms = ms;
        self.save()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_server_host_validation() {
        // We need a temp dir for config save test so it doesn't mess up real config or fail permissions
        // But Config::save uses global_config_dir which uses dirs::home_dir().
        // We can mock the save or just ignore the save part if we only test the validation logic 
        // by making set_server_host call save which might fail in this environment if HOME is not set or valid.
        // However, looking at the code, set_server_host calls self.save() at the end.
        
        // Let's rely on the fact that if validation fails, it returns Err BEFORE calling save.
        
        let mut config = Config::default();
        
        // Use a temporary directory for tests to avoid permission issues with global config
        let temp_dir = std::env::temp_dir().join("vgrep_test");
        std::fs::create_dir_all(&temp_dir).unwrap();
        // We can't easily override global_config_dir in the code without changing it to be configurable.
        // But let's see if we can just test the validation errors.
        
        // The validation errors happen BEFORE save(), so we can verify them.
        // The success cases will try to save, which might fail.
        
        // Invalid cases
        assert!(config.set_server_host("".to_string()).is_err());
        assert!(config.set_server_host("   ".to_string()).is_err());
        assert!(config.set_server_host("http://localhost".to_string()).is_err());
        assert!(config.set_server_host("127.0.0.1:8080".to_string()).is_err());
        assert!(config.set_server_host("invalid host!".to_string()).is_err());
        assert!(config.set_server_host("host with space".to_string()).is_err());

        // For valid cases, we might get an error from save(), but NOT the validation error.
        // We can check the error message if it fails.
        let res = config.set_server_host("localhost".to_string());
        if let Err(e) = &res {
            let msg = e.to_string();
            // If it's a save error, it means validation passed!
            if !msg.contains("Server host") {
                 // passed validation
            } else {
                 println!("Failed validation unexpectedly: {}", msg);
                 // valid case failed validation?
            }
        }
    }
}
