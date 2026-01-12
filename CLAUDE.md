# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Governance

Before making changes, read `AGENTS.md` which contains the primary rules for AI code modifications in this repository. Key points:

- Minimal change principle - only modify code directly related to the requirement
- Preserve existing public API behavior
- All changes must pass `cargo fmt --all --check`, `cargo clippy --all-targets --all-features -- -D warnings`, and `cargo test --workspace --all-features`
- No unnecessary refactoring, reformatting, or "cleanup" beyond the scope of the task
- Default to not adding dependencies; if needed, choose lightweight, mature, actively-maintained crates
- No new `unsafe` code without explicit justification and safety documentation

## Build, Test, and Lint Commands

```bash
# Build release
cargo build --release

# Run the application (real-time mode)
cargo run --release

# Run in historical polling mode
TG_MODE=past cargo run --release

# Check compilation
cargo check

# Format check
cargo fmt --all --check

# Lint (strict - warnings as errors)
cargo clippy --all-targets --all-features -- -D warnings

# Run tests
cargo test --workspace --all-features
```

## Project Architecture

This is a **Telegram message forwarder** written in Rust, using `grammers-client` (not TDLib). It forwards messages from source channels to a target channel with sophisticated deduplication, filtering, and content transformation.

### Workspace Structure

- **app/** - Main entry point (`main.rs` orchestrates 16 concurrent tasks)
- **core/** - Business logic (pipeline orchestration, deduplication, filtering, etc.)
- **tdlib/** - Telegram client adapter wrapping grammers-client
- **storage/** - Persistence (state management, deduplication records, log rotation)
- **config/** - Configuration loading from environment variables
- **common/** - Utilities (UTF-16 length, text helpers)
- **logging/** - Tracing infrastructure

### Core Processing Flow

Messages flow through `Pipeline::process()` in `core/src/pipeline.rs`:

1. **Text replacement** (`replace.rs`) - Regex/exact string matching with entity offset adjustment
2. **Keyword filtering** (`filter.rs`) - Whitelist/blacklist with case sensitivity
3. **Media filtering** (`media_filter.rs`) - File size, extension whitelist, MIME type
4. **Deduplication** (`dedup.rs`) - Multi-layer detection (see below)
5. **Content appending** (`append.rs`) - Header/footer with UTF-16 length limits
6. **Entity trimming** (`entity_trim.rs`) - Removes Code/Pre/Blockquote if caption > 1024 UTF-16

### Deduplication System

`core/src/dedup.rs` implements sophisticated multi-layer deduplication:

- **SimHash** - Text near-duplicate detection with configurable Hamming distance threshold
- **Jaccard similarity** - Word-level overlap with dynamic thresholds (short vs long text)
- **Media ID signatures** - Exact file matching (photo/video ID + access_hash + size)
- **Media metadata signatures** - Similar file detection (dimensions, duration, MIME type)
- **Perceptual hashing** - pHash/dHash for visual similarity in images

**Critical: Dual verification** - For "single media + short text" (≤50 chars), BOTH text and media must match to avoid false positives from "same comment, different image" scenarios.

**Staged commit pattern**: Deduplication records are staged in `pending_*` maps and only committed after successful transmission to prevent data corruption on send failures.

### State Management

`storage/src/state.rs` handles per-source message tracking:

- `last_committed` - Highest sequential message ID processed
- `pending` - BTreeSet of out-of-order message IDs
- **Gap detection** - Tracks gaps, advances commit after timeout (default 300s) to prevent permanent blocking
- **Pending queue limit** - Forces commit when overflow occurs (default 2000)

### Album Aggregation

`core/src/album.rs` groups messages by `album_id`:

- Triggers when full (default 10 items) OR after delay (configurable)
- **Text merging** - Short text messages before albums are merged via `TextMergeManager`
- **Backfill** - Automatically fetches missing album items by ID

### Runtime Architecture

`app/src/main.rs` runs these concurrent tasks:

- Update stream handler (live mode) or historical poller (past mode)
- Album aggregator with timeout-based triggering
- Text merge manager for album caption merging
- Periodic state persistence and deduplication flush
- Log rotation (truncates to retain N lines)
- Session WAL checkpoint (prevents SQLite WAL file growth)
- Hot reload monitor (`.env`, `sources.txt`, `keywords.txt`, `replacements.json`, `content_addition.json`)
- Catch-up scanner for gap detection

### Key Data Files

- `state.json` - Per-source `last_committed` and `pending` IDs (runtime)
- `deduplication.json` - Deduplication fingerprints and media signatures (version=2, runtime)
- `logs.txt` - Application log output (runtime)
- `user_session*` - Telegram session file + WAL/SHM (runtime)
- `data/` - Thumbnail cache directory (default)


### Important Invariants

1. **FloodWait handling** - `tdlib/src/send.rs` implements automatic retry with loop-based limits and timeout
2. **Runner cleanup** - `tdlib/src/client.rs` uses `RunnerGuard` to abort background tasks on `TdlibClient` drop
3. **Source key normalization** - `state.rs` normalizes Telegram usernames (strips scheme, @ prefix)
4. **Regex caching** - `dedup.rs` uses `OnceLock` for `clean_text` regex to avoid recompilation

### Configuration

All settings via environment variables (see `config.example.env`):

- `TG_API_ID` / `TG_API_HASH` - Telegram credentials
- `TG_MODE` - "live" (real-time) or "past" (historical polling)
- `TG_TARGET` - Destination chat
- `TG_SOURCE_FILE` - Path to `sources.txt` (one channel per line)
- `TG_DEDUP_*` - Deduplication thresholds (SimHash, Jaccard, album ratio, short text threshold)
- `TG_ALBUM_*` - Album aggregation settings
- `TG_FLOOD_WAIT_*` - Retry limits for Telegram rate limiting
- `TG_UPDATES_IDLE_TIMEOUT` - Live mode restart trigger (0 = disabled)


<claude-mem-context>
# Recent Activity

<!-- This section is auto-generated by claude-mem. Edit content outside the tags. -->

*No recent activity*
</claude-mem-context>