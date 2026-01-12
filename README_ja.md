# Rust + grammers Telegram 転送ツール

**Languages:** [English](README.md) | 日本語 | [한국어](README_ko.md)

## プロジェクト構成

```
rust_tdlib/
├── Cargo.toml              # ワークスペース設定
├── app/                    # メインエントリ
│   ├── Cargo.toml
│   └── src/
│       ├── main.rs         # メインロジック
│       └── thumb_dedup.rs  # サムネイル重複判定
├── core/                   # コアロジック
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── model.rs        # データモデル
│       ├── pipeline.rs     # 処理パイプライン
│       ├── filter.rs       # キーワードフィルタ
│       ├── replace.rs      # テキスト置換
│       ├── dedup.rs        # 多層重複排除
│       ├── album.rs        # アルバム集約
│       ├── append.rs       # ヘッダー/フッター追加
│       ├── dispatch.rs     # ワーカー分配
│       ├── catch_up.rs     # 補漏処理
│       ├── hotreload.rs    # ホットリロード
│       ├── media_filter.rs # メディアフィルタ
│       ├── text_merge.rs   # テキスト結合
│       ├── entity_trim.rs  # エンティティ削除
│       ├── throttle.rs     # FloodWait 制御
│       ├── thumb.rs        # pHash/dHash
│       └── sources.rs      # sources.txt 読み込み
├── tdlib/                  # Telegram アダプタ (grammers)
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── client.rs       # クライアントラッパ
│       ├── model.rs        # モデル変換
│       └── send.rs         # 送信処理
├── storage/                # 永続化
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── state.rs        # 状態管理
│       ├── dedup.rs        # 重複記録
│       ├── log.rs          # ログローテーション
│       └── session.rs      # WAL クリーンアップ
├── config/                 # 設定
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── loader.rs
│       ├── validate.rs
│       └── paths.rs
├── common/                 # 共通ユーティリティ
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── utf16.rs
│       ├── time.rs
│       └── text.rs
├── logging/                # tracing 設定
│   ├── Cargo.toml
│   └── src/
│       └── lib.rs
├── sources.txt             # 送信元一覧 (テンプレート)
├── keywords.txt            # キーワードフィルタ (任意)
├── replacements.json       # 置換ルール (テンプレート)
├── content_addition.json   # Header/Footer (テンプレート)
├── logs.txt                # 実行時ログ
├── state.json              # 実行時状態
├── deduplication.json      # 実行時重複DB
├── user_session*           # 実行時セッション + WAL/SHM
└── data/                   # サムネイルキャッシュ
```

> 注意: `logs.txt`/`state.json`/`deduplication.json`/`user_session*`/`data/` は実行時に生成され `.gitignore` で除外されます。

## 機能概要

### 1. 設定
- 環境変数読み込み (`.env` 対応)
- 設定検証
- パス正規化
- Python 版と同等の設定項目
- サンプル設定 (`config.example.env`)

### 2. 永続化
- `state.json` 管理 (Format 1/Format 2)
- `deduplication.json` (version=2, LRU)
- ログローテーション
- 遅延保存
- Session WAL チェックポイント

### 3. コア処理
- キーワードフィルタ (ホワイト/ブラック)
- テキスト置換 (正規表現/完全一致)
- 多層重複排除
  - SimHash
  - Jaccard (短文/長文)
  - メディア ID シグネチャ
  - メディアメタ情報
  - pHash + dHash
  - アルバム比率判定
  - 短文+単一メディア二重検証
- ヘッダー/フッター追加 (UTF-16 制限)
- UTF-16 オフセット補正
- 長文キャプションのエンティティ除去
- メディアフィルタ
- アルバム前テキスト結合
- FloodWait 再試行

### 4. アルバム集約
- album_id 単位のキャッシュ
- 最大件数で送信 (デフォルト 10)
- 遅延送信
- 欠落補完
- キャプション結合

### 5. Telegram アダプタ (grammers-client)
- 初期化 (API_ID/API_HASH)
- 認証 (電話/コード/2FA)
- 自動再接続
- 更新購読 (Live)
- メッセージ取得
- チャット解決
- テキスト/メディア/アルバム送信
- FloodWait 対応

### 6. 並列処理
- 非同期キュー
- ワーカープール (デフォルト 3)
- グレースフルシャットダウン

### 7. 実行モード
- **Live**: リアルタイム更新 + 補漏
- **Past**: 履歴ポーリング + アルバム補完

### 8. ホットリロード
- 設定ファイル監視
- 自動再読み込み
- グレースフル再起動

### 9. 補漏
- 定期スキャン
- ソース単位のページング
- アルバム考慮の補完
- ギャップタイムアウト

### 10. ログ
- tracing ログ
- レベル制御 (DEBUG/INFO/WARNING/ERROR)
- `logs.txt` ローテーション
- コンソール出力

## 既知の制限

### 機能強化
- パフォーマンス指標
- リトライ/バックオフ最適化
- テスト拡充
- 統合/End-to-End テスト

### 最適化
- 大容量メディアのメモリ削減
- 重複DBのインデックス
- 並列数の自動調整

### 互換性
- TDLib ネイティブ対応 (現状 grammers のみ)

## 依存関係

```toml
[workspace.dependencies]
tokio = "1.35"
serde = "1.0"
serde_json = "1.0"
anyhow = "1.0"
thiserror = "1.0"
tracing = "0.1"
tracing-subscriber = "0.3"
tracing-appender = "0.2"
dotenv = "0.15"
regex = "1.10"
image = "0.25"

[workspace.dependencies.grammers-client]
version = "0.8"

[workspace.dependencies.grammers-session]
version = "0.8"

[workspace.dependencies.grammers-mtsender]
version = "0.8"

siphasher = "1.0"
```

## 設定

環境変数または `.env` で設定します。

- `TG_API_ID`: Telegram API ID
- `TG_API_HASH`: Telegram API Hash
- `TG_TARGET`: 送信先チャット
- `TG_SOURCE_FILE`: 送信元ファイル (デフォルト `sources.txt`)
- `TG_MODE`: 実行モード (`live`/`past`)
- `TG_PAST_LIMIT`: 履歴ページサイズ (デフォルト 2000)
- `TG_CATCH_UP_INTERVAL`: 補漏間隔秒 (デフォルト 300)
- `TG_ALBUM_DELAY`: アルバム遅延 (デフォルト 5.0)
- `TG_ALBUM_MAX_ITEMS`: 最大件数 (デフォルト 10, 上限 10)
- `TG_ALBUM_BACKFILL_MAX_RANGE`: 補完範囲 (デフォルト 20)
- `TG_FORWARD_DELAY`: 送信間隔 (デフォルト 0)
- `TG_FLOOD_WAIT_MAX_RETRIES`: FloodWait リトライ (デフォルト 5)
- `TG_FLOOD_WAIT_MAX_TOTAL_WAIT`: FloodWait 総待ち時間秒 (デフォルト 3600)
- `TG_REQUEST_TIMEOUT`: リクエストタイムアウト秒 (デフォルト 30)
- `TG_DISPATCHER_IDLE_TIMEOUT`: Idle タイムアウト秒 (デフォルト 300)
- `TG_UPDATES_IDLE_TIMEOUT`: 更新 idle 秒 (デフォルト 0 で無効)
- `TG_WORKER_COUNT`: ワーカー数 (デフォルト 3)
- `TG_ENABLE_FILE_FORWARD`: ファイル転送 (デフォルト true)
- `TG_MEDIA_SIZE_LIMIT`: メディア上限 MB (デフォルト 100)
- `TG_MEDIA_EXT_ALLOWLIST`: 拡張子許可リスト
- `TG_KEYWORD_FILE`: キーワードファイル (設定時のみ有効)
- `TG_KEYWORD_CASE_SENSITIVE`: 大小区別 (デフォルト false)
- `TG_KEYWORD_RELOAD_INTERVAL`: キーワード再読込秒 (デフォルト 2)
- `TG_REPLACEMENT_FILE`: 置換ファイル (設定時のみ有効)
- `TG_CONTENT_ADDITION_FILE`: 追加ファイル (設定時のみ有効)
- `TG_DEDUP_FILE`: 重複ファイル (デフォルト `deduplication.json`)
- `TG_DEDUP_LIMIT`: テキスト指紋上限 (デフォルト 5000)
- `TG_MEDIA_DEDUP_LIMIT`: メディア指紋上限 (デフォルト 15000)
- `TG_DEDUP_SIMHASH_THRESHOLD`: SimHash 閾値 (デフォルト 3)
- `TG_DEDUP_JACCARD_SHORT_THRESHOLD`: Jaccard 短文 (デフォルト 0.7)
- `TG_DEDUP_JACCARD_LONG_THRESHOLD`: Jaccard 長文 (デフォルト 0.5)
- `TG_DEDUP_RECENT_TEXT_LIMIT`: 直近テキスト数 (デフォルト 100)
- `TG_DEDUP_PHASH_THRESHOLD`: pHash 閾値 (デフォルト 10)
- `TG_DEDUP_DHASH_THRESHOLD`: dHash 閾値 (デフォルト 10)
- `TG_DEDUP_ALBUM_THUMB_RATIO`: アルバム比率 (デフォルト 0.34)
- `TG_DEDUP_SHORT_TEXT_THRESHOLD`: 短文閾値 (デフォルト 50)
- `TG_THUMB_DIR`: サムネイルディレクトリ (デフォルト `data`)
- `TG_APPEND_LIMIT_WITH_MEDIA`: メディア追加上限 (デフォルト 1024)
- `TG_APPEND_LIMIT_TEXT`: テキスト追加上限 (デフォルト 4096)
- `TG_TEXT_MERGE_WINDOW`: テキスト結合ウィンドウ (デフォルト 0)
- `TG_TEXT_MERGE_MIN_LEN`: テキスト最小長 (デフォルト 0)
- `TG_TEXT_MERGE_MAX_ID_GAP`: ID ギャップ (デフォルト 5)
- `TG_STATE_FLUSH_INTERVAL`: フラッシュ間隔秒 (デフォルト 5)
- `TG_SHUTDOWN_DRAIN_TIMEOUT`: 終了待機秒 (デフォルト 30)
- `TG_STATE_GAP_TIMEOUT`: ギャップタイムアウト秒 (デフォルト 300)
- `TG_STATE_PENDING_LIMIT`: Pending 上限 (デフォルト 2000)
- `TG_HOTRELOAD_INTERVAL`: ホットリロード秒 (デフォルト 2)
- `TG_LOG_LEVEL`: ログレベル (デフォルト INFO)
- `TG_LOG_MAX_LINES`: ログ最大行数 (デフォルト 100)
- `TG_SESSION_NAME`: セッション名 (デフォルト `user_session`)
- `TG_TDLIB_DB_DIR`: TDLib DB ディレクトリ (任意)
- `TG_TDLIB_FILES_DIR`: TDLib ファイルディレクトリ (任意)
- `TG_DEVICE_MODEL`: デバイス名 (任意)
- `TG_SYSTEM_VERSION`: OS バージョン (任意)
- `TG_APP_VERSION`: アプリバージョン (任意)
- `TG_SYSTEM_LANG`: システム言語 (任意)
- `TG_USE_TEST_DC`: テスト DC を使用 (任意)

## Python 互換性

### 互換ファイル
- `sources.txt`
- `state.json` (Format 1/Format 2)
- `deduplication.json` (version=2)
- `keywords.txt`
- `replacements.json`
- `content_addition.json`

### 互換設定
- すべての環境変数

### 互換ロジック
- キーワードフィルタ
- テキスト置換
- 重複判定 (SimHash/Jaccard/アルバム比率)
- アルバム集約

## ビルドと実行

```bash
# ビルド
cargo build --release

# 設定コピー
cp config.example.env .env
# TG_API_ID / TG_API_HASH を入力
# sources.txt / replacements.json / content_addition.json / keywords.txt を必要に応じて更新

# Live モード
cargo run --release

# Past モード
TG_MODE=past cargo run --release

# ビルドチェック
cargo check

# フォーマット
cargo fmt --all --check

# Clippy
cargo clippy --all-targets --all-features -- -D warnings

# テスト
cargo test --workspace
```

## ログ

ログは `logs.txt` に出力されます。`TG_LOG_LEVEL` でレベル制御します。

## パフォーマンス

- 非同期並列 (Tokio)
- ワーカープール
- アルバム集約
- 重複判定優先度: メディアID → サムネイル → テキスト
- 二重検証で誤判定を抑制
- 遅延保存で IO を削減

## セキュリティ

- `.env` を Git にコミットしない
- API Hash とセッションファイルを保護
- `state.json` と `deduplication.json` をバックアップ
