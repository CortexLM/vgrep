<div align="center">

# νgrεp
<pre>
██╗   ██╗ ██████╗ ██████╗ ███████╗██████╗ 
██║   ██║██╔════╝ ██╔══██╗██╔════╝██╔══██╗
██║   ██║██║  ███╗██████╔╝█████╗  ██████╔╝
╚██╗ ██╔╝██║   ██║██╔══██╗██╔══╝  ██╔═══╝ 
 ╚████╔╝ ╚██████╔╝██║  ██║███████╗██║     
  ╚═══╝   ╚═════╝ ╚═╝  ╚═╝╚══════╝╚═╝     
</pre>
**Search code by meaning, not just keywords. 100% offline. Zero cloud dependencies.**

[![CI](https://github.com/CortexLM/vgrep/actions/workflows/ci.yml/badge.svg)](https://github.com/CortexLM/vgrep/actions/workflows/ci.yml)
[![License](https://img.shields.io/github/license/CortexLM/vgrep)](https://github.com/CortexLM/vgrep/blob/main/LICENSE)
[![GitHub stars](https://img.shields.io/github/stars/CortexLM/vgrep)](https://github.com/CortexLM/vgrep/stargazers)
[![Rust](https://img.shields.io/badge/rust-1.75+-orange.svg)](https://www.rust-lang.org/)
[![Discord](https://img.shields.io/badge/Discord-Join%20Server-5865F2?logo=discord&logoColor=white)](https://discord.cortex.foundation)


</div>

---

## Installation

```bash
curl -fsSL https://vgrep.dev/install.sh | sh
```

Or with wget:

```bash
wget -qO- https://vgrep.dev/install.sh | sh
```

After installation, initialize vgrep:

```bash
vgrep init
vgrep models download
```

---

## Introduction

**νgrεp** is a semantic code search tool that uses local LLM embeddings to find code by intent rather than exact text matches. Unlike traditional grep which searches for literal strings, νgrεp understands the *meaning* behind your query and finds semantically related code across your entire codebase.

> **Quick Start**: `vgrep init && vgrep serve` then `vgrep "where is authentication handled?"`

### Key Features

- **Semantic Search**: Find code by intent - search "error handling" to find try/catch blocks, Result types, and exception handlers
- **100% Local**: All processing happens on your machine using llama.cpp - no API keys, no cloud, your code stays private
- **Server Mode**: Keep models loaded in memory for instant sub-100ms searches
- **File Watcher**: Automatically re-index files as they change
- **Cross-Platform**: Native binaries for Windows, Linux, and macOS
- **GPU Acceleration**: Optional CUDA, Metal, and Vulkan support for faster embeddings

---

## System Overview

νgrεp uses a client-server architecture optimized for fast repeated searches:

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                              USER QUERIES                                    │
│                        "where is auth handled?"                              │
│                        "database connection logic"                           │
│                        "error handling patterns"                             │
└─────────────────────────────────┬───────────────────────────────────────────┘
                                  │
                                  ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                            νgrεp CLIENT                                      │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐        │
│  │   Search    │  │    Index    │  │    Watch    │  │   Config    │        │
│  │  Command    │  │   Command   │  │   Command   │  │   Editor    │        │
│  └─────────────┘  └─────────────┘  └─────────────┘  └─────────────┘        │
└─────────────────────────────────┬───────────────────────────────────────────┘
                                  │ HTTP API
                                  ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                            νgrεp SERVER                                      │
│  ┌──────────────────────────────────────────────────────────────────────┐  │
│  │                    Embedding Engine (llama.cpp)                       │  │
│  │              Qwen3-Embedding-0.6B • Always Loaded • Fast              │  │
│  └──────────────────────────────────────────────────────────────────────┘  │
│                                                                              │
│  ┌──────────────────────────────────────────────────────────────────────┐  │
│  │                     SQLite Vector Database                            │  │
│  │         File Hashes • Code Chunks • Embeddings • Metadata             │  │
│  └──────────────────────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────────────────────┘
```

### Processing Pipeline

```
┌──────────┐    ┌──────────┐    ┌──────────┐    ┌──────────┐    ┌──────────┐
│  Source  │───▶│  Chunk   │───▶│  Embed   │───▶│  Store   │───▶│  Search  │
│  Files   │    │  (512b)  │    │  (LLM)   │    │ (SQLite) │    │ (Cosine) │
└──────────┘    └──────────┘    └──────────┘    └──────────┘    └──────────┘
     │               │               │               │               │
     ▼               ▼               ▼               ▼               ▼
  .rs .py        Split into      Generate       Vector DB       Similarity
  .js .ts        overlapping     768-dim        with fast       ranking +
  .go .c         text chunks     vectors        retrieval       results
```

---

## Installation

### From Source

```bash
# Prerequisites: Rust 1.75+, LLVM/Clang, CMake
git clone https://github.com/CortexLM/vgrep.git
cd vgrep
cargo build --release

# Binary at target/release/vgrep
```

### GPU Acceleration

```bash
cargo build --release --features cuda    # NVIDIA GPUs
cargo build --release --features metal   # Apple Silicon
cargo build --release --features vulkan  # Cross-platform GPU
```

### System Requirements

| Component | Minimum | Recommended |
|-----------|---------|-------------|
| RAM | 2 GB | 4+ GB |
| Disk | 1 GB (models) | 2+ GB |
| CPU | 4 cores | 8+ cores |
| GPU | Optional | CUDA/Metal for 10x speedup |

---

## Quick Start

### 1. Initialize

```bash
# Download models and create config (~1GB download)
vgrep init
vgrep models download
```

### 2. Start Server

```bash
# Keep this running - loads model once for fast searches
vgrep serve
```

Output:
```
  >>> vgrep server
  Server: http://127.0.0.1:7777
  
  Loading embedding model...
  Model loaded successfully!
  
  Endpoints:
    • GET  /health   - Health check
    • GET  /status   - Index status
    • POST /search   - Semantic search
    • POST /embed    - Generate embeddings
  
  → Press Ctrl+C to stop
```

### 3. Index & Watch

```bash
# In another terminal - index and auto-update on changes
vgrep watch
```

Output:
```
  >>> vgrep watcher
  Path: /home/user/myproject
  Mode: server

  Ctrl+C to stop

──────────────────────────────────────────────────

  >> Initial indexing...
  Phase 1: Reading files...
    Read 45 files, 312 chunks
  Phase 2: Generating embeddings via server...
    Generated 312 embeddings
  Phase 3: Storing in database...
    Stored 45 files

  Indexing complete!
    Files: 45 indexed, 12 skipped
    Chunks: 312

──────────────────────────────────────────────────

  [~] Watching for changes...

  [+] indexed auth.rs
  [+] indexed db.rs
```

### 4. Search

```bash
# Semantic search - finds by meaning
vgrep "where is authentication handled?"
vgrep "database connection pooling"
vgrep "error handling for network requests"
```

Output:
```
  Searching for: where is authentication handled?

  1. ./src/auth/middleware.rs (87.3%)
  2. ./src/handlers/login.rs (82.1%)
  3. ./src/utils/jwt.rs (76.8%)
  4. ./src/config/security.rs (71.2%)

  → Found 4 results in 45ms
```

---

## Commands

### Search

| Command | Description |
|---------|-------------|
| `vgrep "query"` | Quick semantic search |
| `vgrep search "query" -m 20` | Search with max 20 results |
| `vgrep search "query" -c` | Show code snippets in results |
| `vgrep search "query" --sync` | Re-index before searching |

### Server & Indexing

| Command | Description |
|---------|-------------|
| `vgrep serve` | Start server (keeps model loaded) |
| `vgrep serve -p 8080` | Custom port |
| `vgrep index` | Manual one-time index |
| `vgrep index --force` | Force re-index all files |
| `vgrep watch` | Watch and auto-index on changes |
| `vgrep status` | Show index statistics |

### Configuration

| Command | Description |
|---------|-------------|
| `vgrep config` | Interactive configuration editor |
| `vgrep config show` | Display all settings |
| `vgrep config set mode local` | Set config value |
| `vgrep config reset` | Reset to defaults |

### Models

| Command | Description |
|---------|-------------|
| `vgrep init` | Initialize vgrep |
| `vgrep models download` | Download embedding models |
| `vgrep models list` | Show configured models |

---

## Agent Integrations

νgrεp supports assisted installation for popular coding agents:

```bash
vgrep install <agent>     # Install integration
vgrep uninstall <agent>   # Remove integration
```

| Agent | Command |
|-------|---------|
| Claude Code | `vgrep install claude-code` |
| OpenCode | `vgrep install opencode` |
| Codex | `vgrep install codex` |
| Factory Droid | `vgrep install droid` |

### Usage with Claude Code

```bash
vgrep install claude-code
vgrep serve   # Start server
vgrep watch   # Index your project
# Claude Code can now use vgrep for semantic search
```

### Usage with Factory Droid

```bash
vgrep install droid
# vgrep auto-starts when you begin a Droid session
```

To uninstall: `vgrep uninstall <agent>` (e.g., `vgrep uninstall droid`).

---

## How It Works

### Embedding Generation

νgrεp converts code into high-dimensional vectors that capture semantic meaning:

```
Input:  "fn authenticate(user: &str, pass: &str) -> Result<Token>"
        ↓
        Tokenize → Qwen3-Embedding → Normalize
        ↓
Output: [0.023, -0.156, 0.891, ..., 0.045]  (768 dimensions)
```

### Similarity Search

Queries are embedded and compared using cosine similarity:

$$\text{similarity}(q, d) = \frac{q \cdot d}{\|q\| \|d\|} = \frac{\sum_{i=1}^{n} q_i d_i}{\sqrt{\sum_{i=1}^{n} q_i^2} \sqrt{\sum_{i=1}^{n} d_i^2}}$$

Where:
- $q$ = query embedding vector
- $d$ = document (code chunk) embedding vector
- Result in range $[-1, 1]$, higher = more similar

### Chunking Strategy

Files are split into overlapping chunks for granular search:

```
┌─────────────────────────────────────────────────────┐
│                    Source File                       │
├─────────────────────────────────────────────────────┤
│ Chunk 1 (512 chars)                                 │
│          ├── Overlap (64 chars) ──┤                │
│                    Chunk 2 (512 chars)              │
│                             ├── Overlap ──┤        │
│                                      Chunk 3 ...    │
└─────────────────────────────────────────────────────┘
```

- **Chunk Size**: 512 characters (configurable)
- **Overlap**: 64 characters to preserve context at boundaries
- **Deduplication**: Results grouped by file, best chunk shown

---

## Configuration

### Config Location

| Platform | Path |
|----------|------|
| Linux | `~/.vgrep/config.json` |
| macOS | `~/.vgrep/config.json` |
| Windows | `C:\Users\<user>\.vgrep\config.json` |

### Settings

| Setting | Default | Description |
|---------|---------|-------------|
| `mode` | `server` | `server` (recommended) or `local` |
| `server_host` | `127.0.0.1` | Server bind address |
| `server_port` | `7777` | Server port |
| `max_results` | `10` | Default search results |
| `max_file_size` | `524288` | Max file size to index (512KB) |
| `chunk_size` | `512` | Characters per chunk |
| `chunk_overlap` | `64` | Overlap between chunks |
| `n_threads` | `0` | CPU threads (0 = auto) |
| `use_reranker` | `false` | Enable result reranking (not yet implemented) |

### Environment Variables

All settings can be overridden via environment:

```bash
VGREP_HOST=0.0.0.0      # Bind to all interfaces
VGREP_PORT=8080         # Custom port
VGREP_MAX_RESULTS=20    # More results
VGREP_CONTENT=true      # Always show snippets
```

---

## Server API

νgrεp server exposes a REST API for programmatic access:

### Endpoints

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/health` | GET | Health check |
| `/status` | GET | Index statistics |
| `/search` | POST | Semantic search |
| `/embed` | POST | Generate single embedding |
| `/embed_batch` | POST | Batch embeddings |

### Search Example

```bash
curl -X POST http://127.0.0.1:7777/search \
  -H 'Content-Type: application/json' \
  -d '{
    "query": "authentication middleware",
    "max_results": 5
  }'
```

Response:
```json
{
  "results": [
    {
      "path": "/project/src/auth/middleware.rs",
      "score": 0.873,
      "score_percent": "87.3%",
      "preview": "pub async fn auth_middleware...",
      "start_line": 15,
      "end_line": 45
    }
  ],
  "query": "authentication middleware",
  "total": 1
}
```

---

## Project Structure

```
vgrep/
├── src/
│   ├── cli/              # Command-line interface
│   │   ├── commands.rs   # CLI argument handling
│   │   └── interactive.rs # Config editor
│   ├── core/             # Core functionality
│   │   ├── db.rs         # SQLite vector storage
│   │   ├── embeddings.rs # llama.cpp integration
│   │   ├── indexer.rs    # File chunking & indexing
│   │   └── search.rs     # Similarity search
│   ├── server/           # HTTP server
│   │   ├── api.rs        # Axum endpoints
│   │   └── client.rs     # HTTP client
│   ├── ui/               # User interface
│   │   ├── console.rs    # Colored output
│   │   └── search_tui.rs # Interactive TUI
│   ├── config.rs         # Configuration
│   ├── watcher.rs        # File system watcher
│   ├── lib.rs            # Library root
│   └── main.rs           # Entry point
├── tests/                # Integration tests
├── .github/
│   ├── workflows/        # CI/CD (test, build, release)
│   └── hooks/            # Git hooks (pre-commit, pre-push)
└── scripts/              # Development utilities
```

---

## Models

νgrεp uses quantized models from HuggingFace for efficient local inference:

| Model | Size | Purpose |
|-------|------|---------|
| Qwen3-Embedding-0.6B-Q8_0 | ~600 MB | Text → Vector embeddings |
| Qwen3-Reranker-0.6B-Q4_K_M | ~400 MB | Result reranking (not yet implemented) |

The embedding model is downloaded by default with `vgrep models download`. The reranker model is reserved for future use and can be downloaded separately with `vgrep models download --reranker-only`.

Models are downloaded to `~/.cache/huggingface/` and cached automatically.

---

## Performance


### Optimization Tips

1. **Use Server Mode**: 10-50x faster than local mode for repeated searches
2. **Enable GPU**: CUDA/Metal provides 5-10x speedup for embedding generation
3. **Watch Mode**: Auto-indexes only changed files, not entire codebase
4. **Tune Chunk Size**: Larger chunks = fewer embeddings but less granular results

---

## Development

### Setup

```bash
# Clone with submodules (llama.cpp)
git clone https://github.com/CortexLM/vgrep.git
cd vgrep

# Setup git hooks
./scripts/setup-hooks.sh   # Unix
./scripts/setup-hooks.ps1  # Windows

# Build
cargo build

# Test
cargo test

# Lint
cargo clippy --all-targets --all-features
cargo fmt --check
```

### Git Hooks

Pre-commit and pre-push hooks ensure code quality:

| Hook | Checks |
|------|--------|
| `pre-commit` | Format, Clippy, Tests |
| `pre-push` | Full test suite, Release build |

Enable with: `git config core.hooksPath .github/hooks`

---

## Comparison

### vs Traditional Grep

| Feature | grep/ripgrep | νgrεp |
|---------|-------------|-------|
| Search type | Exact text / regex | Semantic meaning |
| "auth" finds "authentication" | ❌ | ✅ |
| "error handling" finds try/catch | ❌ | ✅ |
| Speed | Instant | 30-100ms |
| Setup | None | Model download |

### vs Cloud Semantic Search (mgrep, etc.)

| Feature | Cloud Tools | νgrεp |
|---------|-------------|-------|
| Privacy | Code sent to servers | 100% local |
| Cost | API fees | Free |
| Offline | ❌ | ✅ |
| Latency | 200-500ms | 30-100ms |
| Rate limits | Yes | None |

---

## Troubleshooting

### Server won't start

```bash
# Check if port is in use
netstat -an | grep 7777

# Try different port
vgrep serve -p 8080
```

### Slow indexing

```bash
# Use server mode for batch embeddings
vgrep serve &
vgrep index
```

### Model download fails

```bash
# Manual download
vgrep models download --force

# Check disk space
df -h ~/.cache/huggingface
```

### Out of memory

```bash
# Reduce threads
vgrep config set n-threads 2

# Use quantized model (default)
```

---

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for development guidelines.

1. Fork the repository
2. Create feature branch (`git checkout -b feature/amazing`)
3. Run tests (`cargo test`)
4. Run lints (`cargo fmt && cargo clippy`)
5. Submit Pull Request

---

## License

Apache 2.0 - see [LICENSE](LICENSE)

---

<div align="center">

**νgrεp** - *Search code by meaning*

Built with 🦀 Rust and powered by [llama.cpp](https://github.com/ggerganov/llama.cpp)

[Report Bug](https://github.com/CortexLM/vgrep/issues) · [Request Feature](https://github.com/CortexLM/vgrep/issues)

</div>
