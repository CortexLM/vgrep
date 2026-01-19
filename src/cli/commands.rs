use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use console::style;
use std::path::{Path, PathBuf};

use super::interactive;
use crate::config::{Config, Mode};
use crate::core::{Database, EmbeddingEngine, Indexer, SearchEngine, ServerIndexer};
use crate::server::{self, Client};
use crate::ui::{self, SearchTui};
use crate::watcher::FileWatcher;

#[derive(Parser)]
#[command(name = "vgrep")]
#[command(author, version, about = "Local semantic grep using llama.cpp", long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Search query (shorthand for 'vgrep search <query>')
    #[arg(trailing_var_arg = true)]
    query: Vec<String>,

    /// Maximum number of results
    #[arg(short = 'm', long, global = true, env = "VGREP_MAX_RESULTS")]
    max_results: Option<usize>,

    /// Show content in results
    #[arg(short = 'c', long, global = true, env = "VGREP_CONTENT")]
    content: bool,

    /// Use local mode instead of server
    #[arg(short = 'l', long, global = true)]
    local: bool,

    /// Server host to use
    #[arg(long, global = true, env = "VGREP_HOST")]
    host: Option<String>,

    /// Server port to use
    #[arg(short = 'p', long, global = true, env = "VGREP_PORT")]
    port: Option<u16>,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize vgrep (download models, create config)
    Init {
        /// Force re-initialization
        #[arg(short, long)]
        force: bool,
    },

    /// Index files in the current directory
    Index {
        /// Path to index (defaults to current directory)
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Force re-indexing of all files
        #[arg(short, long)]
        force: bool,

        /// Maximum file size to index (in KB)
        #[arg(long)]
        max_size: Option<u64>,

        /// Dry run (don't actually index)
        #[arg(short = 'd', long)]
        dry_run: bool,
    },

    /// Search for files matching a query
    #[command(visible_alias = "s")]
    Search {
        /// The search query
        query: String,

        /// Path to search in (defaults to current directory)
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Maximum number of results
        #[arg(short = 'm', long)]
        max_results: Option<usize>,

        /// Show file content in results
        #[arg(short = 'c', long)]
        content: bool,

        /// Use interactive TUI mode
        #[arg(short = 'i', long)]
        interactive: bool,

        /// Sync/index before searching
        #[arg(short = 's', long)]
        sync: bool,
    },

    /// Watch directory for changes and auto-index
    Watch {
        /// Path to watch (defaults to current directory)
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Dry run (don't actually index)
        #[arg(short = 'd', long)]
        dry_run: bool,
    },

    /// Start the vgrep server (keeps models loaded in memory)
    Serve {
        /// Host to bind to
        #[arg(long)]
        host: Option<String>,

        /// Port to listen on
        #[arg(short = 'p', long)]
        port: Option<u16>,
    },

    /// Show indexing status
    Status,

    /// Download or manage models
    Models {
        #[command(subcommand)]
        action: ModelsAction,
    },

    /// Configure vgrep (interactive)
    Config {
        #[command(subcommand)]
        action: Option<ConfigAction>,
    },

    /// Install vgrep integration for coding agents
    Install {
        #[command(subcommand)]
        agent: InstallAgent,
    },

    /// Uninstall vgrep integration from coding agents
    Uninstall {
        #[command(subcommand)]
        agent: UninstallAgent,
    },
}

#[derive(Subcommand)]
enum InstallAgent {
    /// Install for Claude Code
    ClaudeCode,
    /// Install for OpenCode
    Opencode,
    /// Install for Codex
    Codex,
    /// Install for Factory Droid
    Droid,
}

#[derive(Subcommand)]
enum UninstallAgent {
    /// Uninstall from Claude Code
    ClaudeCode,
    /// Uninstall from OpenCode
    Opencode,
    /// Uninstall from Codex
    Codex,
    /// Uninstall from Factory Droid
    Droid,
}

#[derive(Subcommand)]
enum ModelsAction {
    /// Download required models
    Download {
        /// Download only embedding model
        #[arg(long)]
        embedding_only: bool,

        /// Download only reranker model
        #[arg(long)]
        reranker_only: bool,
    },

    /// List configured models
    List,

    /// Show model information and download links
    Info,
}

#[derive(Subcommand)]
enum ConfigAction {
    /// Show current configuration
    Show,

    /// Interactive configuration editor
    Edit,

    /// Set a configuration value
    Set {
        /// Configuration key
        key: ConfigKey,

        /// Configuration value
        value: String,
    },

    /// Get a configuration value
    Get {
        /// Configuration key
        key: ConfigKey,
    },

    /// Reset configuration to defaults
    Reset,

    /// Show config file path
    Path,
}

#[derive(Clone, ValueEnum)]
enum ConfigKey {
    Mode,
    ServerHost,
    ServerPort,
    EmbeddingModel,
    RerankerModel,
    UseReranker,
    ModelsDir,
    MaxFileSize,
    MaxResults,
    ShowContent,
    ChunkSize,
    ChunkOverlap,
    NThreads,
    ContextSize,
    WatchDebounce,
}

impl Cli {
    pub fn run(self) -> Result<()> {
        let mut config = Config::load()?;

        // Apply global overrides
        if self.local {
            config.mode = Mode::Local;
        }
        if let Some(host) = &self.host {
            config.server_host = host.clone();
        }
        if let Some(port) = self.port {
            config.server_port = port;
        }

        // Handle shorthand search (vgrep "query")
        if self.command.is_none() && !self.query.is_empty() {
            let query = self.query.join(" ");
            let max_results = self.max_results.unwrap_or(config.max_results);
            let show_content = self.content || config.show_content;
            return run_search_smart(
                &config,
                &query,
                &PathBuf::from("."),
                max_results,
                show_content,
                false,
                false,
            );
        }

        match self.command {
            Some(Commands::Init { force }) => run_init(&mut config, force),
            Some(Commands::Index {
                path,
                force,
                max_size,
                dry_run,
            }) => {
                let max_size = max_size.map(|s| s * 1024).unwrap_or(config.max_file_size);
                run_index(&config, path, force, max_size, dry_run)
            }
            Some(Commands::Search {
                query,
                path,
                max_results,
                content,
                interactive,
                sync,
            }) => {
                let max_results = max_results
                    .or(self.max_results)
                    .unwrap_or(config.max_results);
                let content = content || self.content || config.show_content;
                run_search_smart(
                    &config,
                    &query,
                    &path,
                    max_results,
                    content,
                    interactive,
                    sync,
                )
            }
            Some(Commands::Watch { path, dry_run }) => {
                if dry_run {
                    println!("Dry run: would watch {}", path.display());
                    return Ok(());
                }
                run_watch(&config, path)
            }
            Some(Commands::Serve { host, port }) => {
                let host = host.unwrap_or_else(|| config.server_host.clone());
                let port = port.unwrap_or(config.server_port);
                run_serve(&config, host, port)
            }
            Some(Commands::Status) => run_status(&config),
            Some(Commands::Models { action }) => run_models(action, &mut config),
            Some(Commands::Config { action }) => run_config(action, &mut config),
            Some(Commands::Install { agent }) => match agent {
                InstallAgent::ClaudeCode => super::install::install_claude_code(),
                InstallAgent::Opencode => super::install::install_opencode(),
                InstallAgent::Codex => super::install::install_codex(),
                InstallAgent::Droid => super::install::install_droid(),
            },
            Some(Commands::Uninstall { agent }) => match agent {
                UninstallAgent::ClaudeCode => super::install::uninstall_claude_code(),
                UninstallAgent::Opencode => super::install::uninstall_opencode(),
                UninstallAgent::Codex => super::install::uninstall_codex(),
                UninstallAgent::Droid => super::install::uninstall_droid(),
            },
            None => {
                print_quick_help();
                Ok(())
            }
        }
    }
}

fn print_quick_help() {
    ui::print_banner();

    println!("  {}", style("Usage:").bold());
    println!(
        "    {} {}           Search for something",
        style("vgrep").cyan(),
        style("<query>").yellow()
    );
    println!(
        "    {} {} {}  Search with options",
        style("vgrep").cyan(),
        style("search").green(),
        style("<query>").yellow()
    );
    println!(
        "    {} {}             Watch and auto-index",
        style("vgrep").cyan(),
        style("watch").green()
    );
    println!(
        "    {} {}             Start server",
        style("vgrep").cyan(),
        style("serve").green()
    );
    println!(
        "    {} {}           Show all commands",
        style("vgrep").cyan(),
        style("--help").dim()
    );
    println!();

    println!("  {}", style("Quick start:").bold());
    println!(
        "    {} {}   Initialize and download models",
        style("1.").dim(),
        style("vgrep init").cyan()
    );
    println!(
        "    {} {}  Start server (in a terminal)",
        style("2.").dim(),
        style("vgrep serve").cyan()
    );
    println!(
        "    {} {}  Index and watch (in another terminal)",
        style("3.").dim(),
        style("vgrep watch").cyan()
    );
    println!(
        "    {} {} Search!",
        style("4.").dim(),
        style("vgrep \"your query\"").cyan()
    );
    println!();

    println!("  {}", style("Configuration:").bold());
    println!(
        "    {} {}    Interactive config editor",
        style("vgrep").cyan(),
        style("config").green()
    );
    println!();

    if let Ok(path) = Config::global_config_path() {
        println!(
            "  {} {}",
            style("Config:").dim(),
            style(path.display()).dim()
        );
    }
    println!();
}

fn run_init(config: &mut Config, force: bool) -> Result<()> {
    ui::print_banner();
    ui::print_header("Initialization");

    let config_dir = Config::global_config_dir()?;

    if !config_dir.exists() || force {
        std::fs::create_dir_all(&config_dir)?;
        ui::print_success(&format!("Config directory: {}", config_dir.display()));
    } else {
        println!(
            "  {} Config directory: {}",
            ui::CHECK,
            style(config_dir.display()).dim()
        );
    }

    if force {
        *config = Config::default();
        config.save()?;
        ui::print_success("Config reset to defaults");
    } else {
        println!(
            "  {} Config file: {}",
            ui::CHECK,
            style(Config::global_config_path()?.display()).dim()
        );
    }

    let models_dir = config.get_models_dir()?;
    std::fs::create_dir_all(&models_dir)?;
    println!(
        "  {} Models directory: {}",
        ui::CHECK,
        style(models_dir.display()).dim()
    );

    ui::print_section("Models Status");

    let has_embedding = config.has_embedding_model();
    let has_reranker = config.has_reranker_model();

    if has_embedding {
        ui::print_key_value_colored("Embedding", &config.embedding_model, true);
    } else {
        ui::print_key_value_colored("Embedding", "NOT FOUND", false);
    }

    if has_reranker {
        ui::print_key_value_colored("Reranker", &config.reranker_model, true);
    } else {
        ui::print_key_value_colored("Reranker", "NOT FOUND (optional)", false);
    }

    println!();
    if !has_embedding {
        ui::print_warning("Download models with: vgrep models download");
    } else {
        ui::print_success("Ready! Start the server with: vgrep serve");
    }
    println!();

    Ok(())
}

fn run_index(
    config: &Config,
    path: PathBuf,
    force: bool,
    max_size: u64,
    dry_run: bool,
) -> Result<()> {
    if dry_run {
        println!(
            "  {} Dry run: would index {} (max size: {} KB)",
            ui::FOLDER,
            style(path.display()).cyan(),
            max_size / 1024
        );
        return Ok(());
    }

    println!();
    println!("  {}Indexing {}", ui::FOLDER, style(path.display()).cyan());
    println!();

    match config.mode {
        Mode::Server => {
            let client = Client::new(&config.server_host, config.server_port);
            if !client.health().unwrap_or(false) {
                ui::print_error(&format!(
                    "Server not running at {}:{}",
                    config.server_host, config.server_port
                ));
                println!();
                println!("  Start the server first:");
                println!("    {}", style("vgrep serve").cyan());
                println!();
                println!("  Or use local mode:");
                println!("    {}", style("vgrep --local index").cyan());
                return Ok(());
            }

            let db = Database::new(&config.db_path()?)?;
            let indexer = ServerIndexer::new(db, client, max_size);
            indexer.index_directory(&path, force)?;
        }
        Mode::Local => {
            if !config.has_embedding_model() {
                ui::print_error(&format!(
                    "Embedding model not found: {}",
                    config.embedding_model
                ));
                println!("  Run: {}", style("vgrep models download").cyan());
                return Ok(());
            }

            let db = Database::new(&config.db_path()?)?;
            let engine = EmbeddingEngine::new(config)?;
            let indexer = Indexer::new(db, engine, max_size);
            indexer.index_directory(&path, force)?;
        }
    }

    Ok(())
}

fn run_search_smart(
    config: &Config,
    query: &str,
    path: &Path,
    max_results: usize,
    show_content: bool,
    interactive: bool,
    sync: bool,
) -> Result<()> {
    if sync {
        run_index(
            config,
            path.to_path_buf(),
            false,
            config.max_file_size,
            false,
        )?;
    }

    match config.mode {
        Mode::Server => {
            let client = Client::new(&config.server_host, config.server_port);

            if !client.health().unwrap_or(false) {
                ui::print_warning(&format!(
                    "Server not running at {}:{}",
                    config.server_host, config.server_port
                ));
                println!("  Falling back to local mode...");
                println!();
                return run_search_local(
                    config,
                    query,
                    path,
                    max_results,
                    show_content,
                    interactive,
                );
            }

            run_search_server(&client, query, path, max_results, show_content)
        }
        Mode::Local => {
            run_search_local(config, query, path, max_results, show_content, interactive)
        }
    }
}

fn run_search_server(
    client: &Client,
    query: &str,
    path: &Path,
    max_results: usize,
    show_content: bool,
) -> Result<()> {
    use std::time::Instant;

    println!();
    println!(
        "  {}Searching for: {}",
        ui::SEARCH,
        style(query).yellow().bold()
    );
    println!();

    let start = Instant::now();
    let response = client.search(query, Some(path), max_results)?;
    let elapsed = start.elapsed();

    if response.results.is_empty() {
        ui::print_warning("No results found");
        return Ok(());
    }

    let cwd = std::env::current_dir()?;

    for (i, result) in response.results.iter().enumerate() {
        let clean_path = result.path.replace("\\\\?\\", "");
        let result_path = PathBuf::from(&clean_path);
        let rel_path = result_path.strip_prefix(&cwd).unwrap_or(&result_path);

        ui::print_search_result(
            &format!("./{}", rel_path.display()),
            &result.score_percent,
            i,
        );

        if show_content {
            if let Some(preview) = &result.preview {
                ui::print_search_preview(preview);
            }
            println!();
        }
    }

    println!();
    println!(
        "  {} Found {} results in {}",
        style("→").dim(),
        style(response.results.len()).green(),
        style(format!("{:.0}ms", elapsed.as_millis())).yellow()
    );
    println!();

    Ok(())
}

fn run_search_local(
    config: &Config,
    query: &str,
    path: &Path,
    max_results: usize,
    show_content: bool,
    interactive: bool,
) -> Result<()> {
    use std::time::Instant;

    if !config.has_embedding_model() {
        ui::print_error(&format!(
            "Embedding model not found: {}",
            config.embedding_model
        ));
        println!("  Run: {}", style("vgrep models download").cyan());
        return Ok(());
    }

    println!();
    println!("  {}Loading model...", ui::BRAIN);

    let db = Database::new(&config.db_path()?)?;
    let engine = EmbeddingEngine::new(config)?;
    let search = SearchEngine::new(db, engine, config, config.use_reranker)?;

    if interactive {
        let mut tui = SearchTui::new(search)?;
        tui.run()?;
    } else {
        println!(
            "  {}Searching for: {}",
            ui::SEARCH,
            style(query).yellow().bold()
        );
        println!();

        let start = Instant::now();
        let results = search.search(query, path, max_results)?;
        let elapsed = start.elapsed();

        if results.is_empty() {
            ui::print_warning("No results found");
            return Ok(());
        }

        let cwd = std::env::current_dir()?;

        for (i, result) in results.iter().enumerate() {
            let clean_path = result.path.to_string_lossy().replace("\\\\?\\", "");
            let clean_path = PathBuf::from(clean_path);
            let rel_path = clean_path.strip_prefix(&cwd).unwrap_or(&clean_path);

            ui::print_search_result(
                &format!("./{}", rel_path.display()),
                &format!("{:.1}%", result.score * 100.0),
                i,
            );

            if show_content {
                if let Some(preview) = &result.preview {
                    ui::print_search_preview(preview);
                }
                println!();
            }
        }

        println!();
        println!(
            "  {} Found {} results in {}",
            style("→").dim(),
            style(results.len()).green(),
            style(format!("{:.0}ms", elapsed.as_millis())).yellow()
        );
        println!();
    }

    Ok(())
}

fn run_watch(config: &Config, path: PathBuf) -> Result<()> {
    if config.mode == Mode::Server {
        let client = Client::new(&config.server_host, config.server_port);
        if !client.health().unwrap_or(false) {
            ui::print_error(&format!(
                "Server not running at {}:{}",
                config.server_host, config.server_port
            ));
            println!();
            println!("  Start the server first:");
            println!("    {}", style("vgrep serve").cyan());
            println!();
            println!("  Or use local mode:");
            println!("    {}", style("vgrep --local watch").cyan());
            return Ok(());
        }
    }

    let watcher = FileWatcher::new(config.clone(), path);
    watcher.watch()
}

fn run_serve(config: &Config, host: String, port: u16) -> Result<()> {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(server::run_server(config, &host, port))
}

fn run_status(config: &Config) -> Result<()> {
    ui::print_banner();
    ui::print_header("Status");

    ui::print_section("Configuration");
    ui::print_key_value(
        "Config file",
        &Config::global_config_path()?.to_string_lossy(),
    );
    ui::print_key_value("Mode", &config.mode.to_string());
    ui::print_key_value(
        "Server",
        &format!("{}:{}", config.server_host, config.server_port),
    );

    let client = Client::new(&config.server_host, config.server_port);
    let server_running = client.health().unwrap_or(false);
    ui::print_key_value_colored(
        "Server status",
        if server_running {
            "Running"
        } else {
            "Not running"
        },
        server_running,
    );

    ui::print_section("Models");
    ui::print_key_value("Directory", &config.get_models_dir()?.to_string_lossy());
    ui::print_key_value_colored(
        "Embedding",
        &config.embedding_model,
        config.has_embedding_model(),
    );

    let reranker_status = format!(
        "{} ({})",
        config.reranker_model,
        if config.use_reranker {
            "enabled"
        } else {
            "disabled"
        }
    );
    ui::print_key_value_colored("Reranker", &reranker_status, config.has_reranker_model());

    ui::print_section("Index");
    let db_path = config.db_path()?;
    if db_path.exists() {
        let db = Database::new(&db_path)?;
        let stats = db.get_stats()?;
        ui::print_key_value("Database", &db_path.to_string_lossy());
        ui::print_key_value("Files indexed", &stats.file_count.to_string());
        ui::print_key_value("Total chunks", &stats.chunk_count.to_string());
        ui::print_key_value(
            "Last indexed",
            &stats.last_indexed.map_or_else(
                || "Never".to_string(),
                |t| t.format("%Y-%m-%d %H:%M:%S").to_string(),
            ),
        );
    } else {
        ui::print_key_value("Database", "Not created");
        println!(
            "  {} Run '{}' to create the index",
            style("→").dim(),
            style("vgrep index").cyan()
        );
    }

    println!();
    Ok(())
}

fn run_models(action: ModelsAction, config: &mut Config) -> Result<()> {
    match action {
        ModelsAction::Download {
            embedding_only,
            reranker_only,
        } => {
            use hf_hub::api::sync::ApiBuilder;
            use indicatif::{ProgressBar, ProgressStyle};

            ui::print_banner();
            ui::print_header("Downloading Models");

            let api = ApiBuilder::new().with_progress(true).build()?;
            let models_dir = config.get_models_dir()?;
            std::fs::create_dir_all(&models_dir)?;

            println!("  {} {}", ui::FOLDER, style(models_dir.display()).dim());
            println!();

            if !reranker_only {
                println!("  {}Downloading embedding model...", ui::DOWNLOAD);
                println!(
                    "    {} Qwen3-Embedding-0.6B-Q8_0.gguf",
                    style("Model:").dim()
                );
                println!(
                    "    {} huggingface.co/Qwen/Qwen3-Embedding-0.6B-GGUF",
                    style("From:").dim()
                );
                println!();

                let pb = ProgressBar::new_spinner();
                pb.set_style(ProgressStyle::default_spinner().template("{spinner:.green} {msg}")?);
                pb.set_message("Downloading...");

                let embedding_path = api
                    .model("Qwen/Qwen3-Embedding-0.6B-GGUF".to_string())
                    .get("Qwen3-Embedding-0.6B-Q8_0.gguf")?;

                pb.finish_and_clear();
                ui::print_success(&format!("Downloaded: {}", embedding_path.display()));

                config.set_embedding_model(embedding_path.to_string_lossy().to_string())?;
            }

            if !embedding_only {
                println!();
                println!("  {}Downloading reranker model...", ui::DOWNLOAD);
                println!(
                    "    {} Qwen3-Reranker-0.6B-Q4_K_M.gguf",
                    style("Model:").dim()
                );
                println!(
                    "    {} huggingface.co/sinjab/Qwen3-Reranker-0.6B-Q4_K_M-GGUF",
                    style("From:").dim()
                );
                println!();

                let pb = ProgressBar::new_spinner();
                pb.set_style(ProgressStyle::default_spinner().template("{spinner:.green} {msg}")?);
                pb.set_message("Downloading...");

                let reranker_path = api
                    .model("sinjab/Qwen3-Reranker-0.6B-Q4_K_M-GGUF".to_string())
                    .get("Qwen3-Reranker-0.6B-Q4_K_M.gguf")?;

                pb.finish_and_clear();
                ui::print_success(&format!("Downloaded: {}", reranker_path.display()));

                config.set_reranker_model(reranker_path.to_string_lossy().to_string())?;
            }

            println!();
            ui::print_success("Models downloaded successfully!");
            println!();
            println!("  {}Next steps:", ui::ROCKET);
            println!(
                "    {} Start the server: {}",
                style("1.").dim(),
                style("vgrep serve").cyan()
            );
            println!(
                "    {} Index your project: {}",
                style("2.").dim(),
                style("vgrep index").cyan()
            );
            println!(
                "    {} Search: {}",
                style("3.").dim(),
                style("vgrep \"your query\"").cyan()
            );
            println!();
        }
        ModelsAction::List => {
            ui::print_header("Configured Models");

            println!(
                "  {} {}",
                ui::FOLDER,
                style(config.get_models_dir()?.display()).dim()
            );
            println!();

            let emb_ok = config.has_embedding_model();
            println!(
                "  {} {} {}",
                if emb_ok {
                    style("●").green()
                } else {
                    style("○").red()
                },
                style("Embedding:").bold(),
                config.embedding_model
            );

            let rerank_ok = config.has_reranker_model();
            println!(
                "  {} {} {} ({})",
                if rerank_ok {
                    style("●").green()
                } else {
                    style("○").red()
                },
                style("Reranker:").bold(),
                config.reranker_model,
                if config.use_reranker {
                    "enabled"
                } else {
                    "disabled"
                }
            );
            println!();
        }
        ModelsAction::Info => {
            ui::print_header("Available Models");

            println!();
            println!(
                "  {} {}",
                style("1.").cyan(),
                style("Embedding Model (required)").bold()
            );
            println!(
                "     {} Qwen3-Embedding-0.6B-Q8_0.gguf",
                style("Name:").dim()
            );
            println!("     {} ~600MB", style("Size:").dim());
            println!(
                "     {} {}",
                style("URL:").dim(),
                ui::hyperlink("https://huggingface.co/Qwen/Qwen3-Embedding-0.6B-GGUF", "https://huggingface.co/Qwen/Qwen3-Embedding-0.6B-GGUF")
            );
            println!();
            println!(
                "  {} {}",
                style("2.").cyan(),
                style("Reranker Model (recommended)").bold()
            );
            println!(
                "     {} Qwen3-Reranker-0.6B-Q4_K_M.gguf",
                style("Name:").dim()
            );
            println!("     {} ~400MB", style("Size:").dim());
            println!(
                "     {} {}",
                style("URL:").dim(),
                ui::hyperlink("https://huggingface.co/sinjab/Qwen3-Reranker-0.6B-Q4_K_M-GGUF", "https://huggingface.co/sinjab/Qwen3-Reranker-0.6B-Q4_K_M-GGUF")
            );
            println!();
            println!("  Download with: {}", style("vgrep models download").cyan());
            println!();
        }
    }

    Ok(())
}

fn run_config(action: Option<ConfigAction>, config: &mut Config) -> Result<()> {
    match action {
        None => {
            // Interactive config by default
            interactive::run_interactive_config(config)
        }
        Some(ConfigAction::Show) => {
            ui::print_header("Configuration");

            ui::print_section("General");
            ui::print_key_value("Mode", &config.mode.to_string());

            ui::print_section("Server");
            ui::print_key_value("Host", &config.server_host);
            ui::print_key_value("Port", &config.server_port.to_string());

            ui::print_section("Models");
            ui::print_key_value("Embedding", &config.embedding_model);
            ui::print_key_value("Reranker", &config.reranker_model);
            ui::print_key_value("Use reranker", &config.use_reranker.to_string());
            ui::print_key_value(
                "Models dir",
                &config
                    .models_dir
                    .as_ref()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_else(|| "(default)".to_string()),
            );

            ui::print_section("Search");
            ui::print_key_value("Max results", &config.max_results.to_string());
            ui::print_key_value("Show content", &config.show_content.to_string());

            ui::print_section("Indexing");
            ui::print_key_value(
                "Max file size",
                &format!("{} KB", config.max_file_size / 1024),
            );
            ui::print_key_value("Chunk size", &config.chunk_size.to_string());
            ui::print_key_value("Chunk overlap", &config.chunk_overlap.to_string());

            ui::print_section("Performance");
            ui::print_key_value(
                "Threads",
                &format!(
                    "{}{}",
                    config.n_threads,
                    if config.n_threads == 0 { " (auto)" } else { "" }
                ),
            );
            ui::print_key_value("Context size", &config.context_size.to_string());
            ui::print_key_value(
                "Watch debounce",
                &format!("{} ms", config.watch_debounce_ms),
            );

            println!();
            println!(
                "  {} {}",
                style("Config file:").dim(),
                Config::global_config_path()?.display()
            );
            println!(
                "  {} {}",
                style("Edit with:").dim(),
                style("vgrep config").cyan()
            );
            println!();
            Ok(())
        }
        Some(ConfigAction::Edit) => interactive::run_interactive_config(config),
        Some(ConfigAction::Set { key, value }) => {
            let result = match key {
                ConfigKey::Mode => {
                    let mode: Mode = value.parse()?;
                    config.set_mode(mode)?;
                    format!("mode = {}", mode)
                }
                ConfigKey::ServerHost => {
                    config.set_server_host(value.clone())?;
                    format!("server_host = {}", value)
                }
                ConfigKey::ServerPort => {
                    let port: u16 = value.parse().context("Invalid port")?;
                    config.set_server_port(port)?;
                    format!("server_port = {}", port)
                }
                ConfigKey::EmbeddingModel => {
                    config.set_embedding_model(value.clone())?;
                    format!("embedding_model = {}", value)
                }
                ConfigKey::RerankerModel => {
                    config.set_reranker_model(value.clone())?;
                    format!("reranker_model = {}", value)
                }
                ConfigKey::UseReranker => {
                    let enabled = matches!(value.to_lowercase().as_str(), "true" | "yes" | "1");
                    config.set_use_reranker(enabled)?;
                    format!("use_reranker = {}", enabled)
                }
                ConfigKey::ModelsDir => {
                    if value.is_empty() || value == "default" {
                        config.set_models_dir(None)?;
                        "models_dir = (default)".to_string()
                    } else {
                        config.set_models_dir(Some(PathBuf::from(&value)))?;
                        format!("models_dir = {}", value)
                    }
                }
                ConfigKey::MaxFileSize => {
                    let size: u64 = value.parse().context("Invalid size")?;
                    config.set_max_file_size(size * 1024)?;
                    format!("max_file_size = {} KB", size)
                }
                ConfigKey::MaxResults => {
                    let count: usize = value.parse().context("Invalid number")?;
                    config.set_max_results(count)?;
                    format!("max_results = {}", count)
                }
                ConfigKey::ShowContent => {
                    let show = matches!(value.to_lowercase().as_str(), "true" | "yes" | "1");
                    config.set_show_content(show)?;
                    format!("show_content = {}", show)
                }
                ConfigKey::ChunkSize => {
                    let size: usize = value.parse().context("Invalid size")?;
                    config.set_chunk_size(size)?;
                    format!("chunk_size = {}", size)
                }
                ConfigKey::ChunkOverlap => {
                    let overlap: usize = value.parse().context("Invalid overlap")?;
                    config.set_chunk_overlap(overlap)?;
                    format!("chunk_overlap = {}", overlap)
                }
                ConfigKey::NThreads => {
                    let threads: usize = value.parse().context("Invalid number")?;
                    config.set_n_threads(threads)?;
                    format!("n_threads = {}", threads)
                }
                ConfigKey::ContextSize => {
                    let size: usize = value.parse().context("Invalid size")?;
                    config.set_context_size(size)?;
                    format!("context_size = {}", size)
                }
                ConfigKey::WatchDebounce => {
                    let ms: u64 = value.parse().context("Invalid milliseconds")?;
                    config.set_watch_debounce(ms)?;
                    format!("watch_debounce_ms = {}", ms)
                }
            };
            ui::print_success(&format!("Set {}", result));
            Ok(())
        }
        Some(ConfigAction::Get { key }) => {
            let value = match key {
                ConfigKey::Mode => config.mode.to_string(),
                ConfigKey::ServerHost => config.server_host.clone(),
                ConfigKey::ServerPort => config.server_port.to_string(),
                ConfigKey::EmbeddingModel => config.embedding_model.clone(),
                ConfigKey::RerankerModel => config.reranker_model.clone(),
                ConfigKey::UseReranker => config.use_reranker.to_string(),
                ConfigKey::ModelsDir => config
                    .models_dir
                    .as_ref()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_else(|| "(default)".to_string()),
                ConfigKey::MaxFileSize => format!("{}", config.max_file_size / 1024),
                ConfigKey::MaxResults => config.max_results.to_string(),
                ConfigKey::ShowContent => config.show_content.to_string(),
                ConfigKey::ChunkSize => config.chunk_size.to_string(),
                ConfigKey::ChunkOverlap => config.chunk_overlap.to_string(),
                ConfigKey::NThreads => config.n_threads.to_string(),
                ConfigKey::ContextSize => config.context_size.to_string(),
                ConfigKey::WatchDebounce => config.watch_debounce_ms.to_string(),
            };
            println!("{}", value);
            Ok(())
        }
        Some(ConfigAction::Reset) => {
            *config = Config::default();
            config.save()?;
            ui::print_success("Configuration reset to defaults");
            Ok(())
        }
        Some(ConfigAction::Path) => {
            println!("{}", Config::global_config_path()?.display());
            Ok(())
        }
    }
}
