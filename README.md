# Rust + grammers Telegram Forwarder

**Languages:** English | [中文](docs/i18n/README_zh.md) | [日本語](docs/i18n/README_ja.md) | [한국어](docs/i18n/README_ko.md)

## Project Structure

```
rust_tdlib/
├── Cargo.toml              # Workspace configuration
├── app/                    # Main entry
│   ├── Cargo.toml
│   └── src/
│       ├── main.rs         # Main application logic
│       └── thumb_dedup.rs  # Thumbnail dedup helper
├── core/                   # Core business logic
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── model.rs        # Data models
│       ├── pipeline.rs     # Processing pipeline
│       ├── filter.rs       # Keyword filter
│       ├── replace.rs      # Text replacement rules
│       ├── dedup.rs        # Multi-layer deduplication
│       ├── album.rs        # Album aggregation
│       ├── append.rs       # Header/Footer appending
│       ├── dispatch.rs     # Worker dispatcher
│       ├── catch_up.rs     # Catch-up logic
│       ├── hotreload.rs    # Hot reload
│       ├── media_filter.rs # Media filter
│       ├── text_merge.rs   # Text merge
│       ├── entity_trim.rs  # Entity trimming
│       ├── throttle.rs     # FloodWait throttle
│       ├── thumb.rs        # pHash/dHash
│       └── sources.rs      # Source parsing
├── tdlib/                  # Telegram adapter (grammers)
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── client.rs       # Client wrapper
│       ├── model.rs        # Telegram model mapping
│       └── send.rs         # Send helpers
├── storage/                # Persistence
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── state.rs        # State store
│       ├── dedup.rs        # Dedup store
│       ├── log.rs          # Log rotation
│       └── session.rs      # Session WAL cleanup
├── config/                 # Config loader/validator
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── loader.rs
│       ├── validate.rs
│       └── paths.rs
├── common/                 # Utilities
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── utf16.rs
│       ├── time.rs
│       └── text.rs
├── logging/                # Tracing setup
│   ├── Cargo.toml
│   └── src/
│       └── lib.rs
├── sources.txt             # Source list (template)
├── keywords.txt            # Keyword filter (optional)
├── replacements.json       # Replacement rules (template)
├── content_addition.json   # Header/Footer (template)
├── logs.txt                # Runtime log output
├── state.json              # Runtime state
├── deduplication.json      # Runtime dedup database
├── user_session*           # Runtime session + WAL/SHM
└── data/                   # Thumbnail cache (default)
```

> Note: `logs.txt`/`state.json`/`deduplication.json`/`user_session*`/`data/` are runtime outputs and are ignored by `.gitignore`.

## Features

### 1. Configuration
- Environment variable loading (`.env` compatible)
- Configuration validation
- Path normalization (relative → absolute)
- Full compatibility with the Python config schema
- Example config file (`config.example.env`)

### 2. Persistence
- `state.json` tracking (Format 1/Format 2)
- `deduplication.json` (version=2, LRU)
- Log rotation (max line count)
- Lazy save strategy
- Session WAL checkpoint (sqlite3)

### 3. Core Processing
- Keyword filtering (white/black list)
- Text replacement (regex/exact with entity offset adjustment)
- Multi-layer deduplication
  - SimHash text similarity
  - Jaccard similarity (short/long)
  - Media ID signature
  - Media metadata signature
  - Thumbnail perceptual hashes (pHash + dHash)
  - Album-aware dedup ratio
  - Dual verification for short text + single media
- Content appending (header/footer with UTF-16 limits)
- UTF-16 entity offset handling
- Entity trimming for long captions
- Media filtering (size, extension allowlist, MIME validation)
- Text merge before albums
- FloodWait throttle with retry

### 4. Album Aggregation
- Album cache by `album_id`
- Trigger on max items (default 10)
- Trigger on delay
- Album backfill for missing items
- Album caption merge

### 5. Telegram Adapter (grammers-client)
- Client initialization (API_ID/API_HASH)
- Auth flow (phone, code, 2FA)
- Auto reconnect
- Update subscription (live mode)
- Message fetch (pagination / by ID)
- Chat resolve (username/link/ID)
- Send text/media/album
- FloodWait handling

### 6. Dispatch & Concurrency
- Async task queue
- Worker pool (default 3)
- Graceful shutdown with timeout

### 7. Run Modes
- **Live mode**: realtime updates + periodic catch-up
- **Past mode**: historical polling + album backfill

### 8. Hot Reload
- Config file monitoring (`sources.txt`/`keywords.txt`/`replacements.json`/`content_addition.json`)
- Auto reload rules
- Graceful restart

### 9. Catch-up
- Periodic gap scan
- Pagination by source
- Album-aware backfill
- Gap timeout handling

### 10. Logging
- Structured logging (tracing)
- Level control (DEBUG/INFO/WARNING/ERROR)
- Log rotation (`logs.txt`)
- Console output

## Known Limitations

### Enhancements
- Performance metrics (CPU/memory/throughput)
- Smarter retry/backoff strategy
- Higher unit test coverage
- Integration/E2E tests

### Optimization
- Memory optimization for large media/albums
- Dedup database indexing
- Adaptive concurrency tuning

### Compatibility
- TDLib native adapter (currently only grammers)

## Dependencies

```toml
[workspace.dependencies]
tokio = "1.35"              # Async runtime
serde = "1.0"               # Serialization
serde_json = "1.0"
anyhow = "1.0"              # Error handling
thiserror = "1.0"
tracing = "0.1"             # Logging
tracing-subscriber = "0.3"
tracing-appender = "0.2"
dotenv = "0.15"             # Environment variables
regex = "1.10"              # Regex
image = "0.25"              # Image processing

[workspace.dependencies.grammers-client]
version = "0.8"

[workspace.dependencies.grammers-session]
version = "0.8"

[workspace.dependencies.grammers-mtsender]
version = "0.8"

siphasher = "1.0"           # SimHash
```

## Configuration

All settings are provided via environment variables or `.env`:

- `TG_API_ID`: Telegram API ID
- `TG_API_HASH`: Telegram API Hash
- `TG_TARGET`: Target chat
- `TG_SOURCE_FILE`: Source file path (default `sources.txt`)
- `TG_MODE`: Run mode (`live`/`past`)
- `TG_PAST_LIMIT`: History page size (default 2000)
- `TG_CATCH_UP_INTERVAL`: Catch-up interval in seconds (default 300)
- `TG_ALBUM_DELAY`: Album delay (default 5.0)
- `TG_ALBUM_MAX_ITEMS`: Max album items (default 10, max 10)
- `TG_ALBUM_BACKFILL_MAX_RANGE`: Album backfill range (default 20)
- `TG_FORWARD_DELAY`: Forward delay (default 0)
- `TG_FLOOD_WAIT_MAX_RETRIES`: FloodWait max retries (default 5)
- `TG_FLOOD_WAIT_MAX_TOTAL_WAIT`: FloodWait max total wait seconds (default 3600)
- `TG_REQUEST_TIMEOUT`: Request timeout seconds (default 30)
- `TG_DISPATCHER_IDLE_TIMEOUT`: Dispatcher idle timeout seconds (default 300)
- `TG_UPDATES_IDLE_TIMEOUT`: Updates idle timeout seconds (default 0 disables)
- `TG_WORKER_COUNT`: Worker count (default 3)
- `TG_ENABLE_FILE_FORWARD`: Enable file forward (default true)
- `TG_MEDIA_SIZE_LIMIT`: Media size limit in MB (default 100)
- `TG_MEDIA_EXT_ALLOWLIST`: Media extension allowlist (comma/semicolon/space)
- `TG_KEYWORD_FILE`: Keyword file (enable only when set)
- `TG_KEYWORD_CASE_SENSITIVE`: Keyword case sensitivity (default false)
- `TG_KEYWORD_RELOAD_INTERVAL`: Keyword reload interval seconds (default 2)
- `TG_REPLACEMENT_FILE`: Replacement rules file (enable only when set)
- `TG_CONTENT_ADDITION_FILE`: Content append file (enable only when set)
- `TG_DEDUP_FILE`: Dedup file path (default `deduplication.json`)
- `TG_DEDUP_LIMIT`: Text fingerprint limit (default 5000)
- `TG_MEDIA_DEDUP_LIMIT`: Media fingerprint limit (default 15000)
- `TG_DEDUP_SIMHASH_THRESHOLD`: SimHash threshold (default 3)
- `TG_DEDUP_JACCARD_SHORT_THRESHOLD`: Jaccard short threshold (default 0.7)
- `TG_DEDUP_JACCARD_LONG_THRESHOLD`: Jaccard long threshold (default 0.5)
- `TG_DEDUP_RECENT_TEXT_LIMIT`: Recent text feature limit (default 100)
- `TG_DEDUP_PHASH_THRESHOLD`: pHash threshold (default 10)
- `TG_DEDUP_DHASH_THRESHOLD`: dHash threshold (default 10; `TG_DEDUP_THUMB_DHASH_THRESHOLD` also supported)
- `TG_DEDUP_ALBUM_THUMB_RATIO`: Album thumb ratio (default 0.34)
- `TG_DEDUP_SHORT_TEXT_THRESHOLD`: Short text threshold (default 50)
- `TG_THUMB_DIR`: Thumbnail cache directory (default `data`)
- `TG_APPEND_LIMIT_WITH_MEDIA`: Append limit with media (default 1024)
- `TG_APPEND_LIMIT_TEXT`: Append limit for text (default 4096)
- `TG_TEXT_MERGE_WINDOW`: Text merge window (default 0)
- `TG_TEXT_MERGE_MIN_LEN`: Text merge min length (default 0)
- `TG_TEXT_MERGE_MAX_ID_GAP`: Text merge max ID gap (default 5)
- `TG_STATE_FLUSH_INTERVAL`: State/dedup flush interval seconds (default 5)
- `TG_SHUTDOWN_DRAIN_TIMEOUT`: Shutdown drain timeout seconds (default 30)
- `TG_STATE_GAP_TIMEOUT`: State gap timeout seconds (default 300)
- `TG_STATE_PENDING_LIMIT`: State pending limit (default 2000)
- `TG_HOTRELOAD_INTERVAL`: Hot reload interval seconds (default 2)
- `TG_LOG_LEVEL`: Log level (default INFO)
- `TG_LOG_MAX_LINES`: Log max lines (default 100)
- `TG_SESSION_NAME`: Session file name (default `user_session`)
- `TG_TDLIB_DB_DIR`: TDLib DB directory (optional)
- `TG_TDLIB_FILES_DIR`: TDLib files directory (optional)
- `TG_DEVICE_MODEL`: Device model (optional)
- `TG_SYSTEM_VERSION`: System version (optional)
- `TG_APP_VERSION`: App version (optional)
- `TG_SYSTEM_LANG`: System language (optional)
- `TG_USE_TEST_DC`: Use test DC (optional)

## Python Compatibility

### Compatible File Formats
- `sources.txt` (source list)
- `state.json` (Format 1/Format 2)
- `deduplication.json` (version=2)
- `keywords.txt` (whitelist/blacklist)
- `replacements.json` (regex/exact)
- `content_addition.json` (header/footer)

### Compatible Config Items
- All environment variables

### Compatible Logic
- Keyword filtering
- Text replacement (entity offsets)
- Dedup strategies (SimHash/Jaccard/album ratio/short text)
- Album aggregation logic

## Build & Run

```bash
# Build
cargo build --release

# Copy config
cp config.example.env .env
# Fill TG_API_ID and TG_API_HASH
# Update sources.txt / replacements.json / content_addition.json / keywords.txt as needed

# Live mode
cargo run --release

# Past mode
TG_MODE=past cargo run --release

# Check build
cargo check

# Format check
cargo fmt --all --check

# Clippy
cargo clippy --all-targets --all-features -- -D warnings

# Tests
cargo test --workspace
```

## Logging

Logs are written to `logs.txt` (levels: DEBUG/INFO/WARNING/ERROR). Use `TG_LOG_LEVEL` to control log level.

## Performance

- Async concurrency (Tokio)
- Worker pool forwarding
- Album-aware batching
- Dedup priority: media ID → thumbnail hash → text similarity
- Dual verification to reduce false positives
- Lazy persistence to reduce IO

## Security Notes

- Do not commit `.env` to git
- Protect API hash and session files
- Backup `state.json` and `deduplication.json`
