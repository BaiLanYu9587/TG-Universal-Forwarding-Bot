# Rust + grammers Telegram 转发器

**Languages:** [English](../../README.md) | 中文 | [日本語](README_ja.md) | [한국어](README_ko.md)

## 项目结构

```
rust_tdlib/
├── Cargo.toml              # 工作区配置
├── app/                    # 主程序入口
│   ├── Cargo.toml
│   └── src/
│       ├── main.rs         # 主逻辑
│       └── thumb_dedup.rs  # 缩略图去重工具
├── core/                   # 核心业务逻辑
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── model.rs        # 数据模型
│       ├── pipeline.rs     # 处理流水线
│       ├── filter.rs       # 关键词过滤
│       ├── replace.rs      # 文本替换
│       ├── dedup.rs        # 多层去重
│       ├── album.rs        # 相册聚合
│       ├── append.rs       # 内容追加
│       ├── dispatch.rs     # 任务调度
│       ├── catch_up.rs     # 补漏逻辑
│       ├── hotreload.rs    # 热更新
│       ├── media_filter.rs # 媒体过滤
│       ├── text_merge.rs   # 文本合并
│       ├── entity_trim.rs  # 实体修剪
│       ├── throttle.rs     # FloodWait 控制
│       ├── thumb.rs        # pHash/dHash
│       └── sources.rs      # 来源解析
├── tdlib/                  # Telegram 适配层（grammers）
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── client.rs       # 客户端封装
│       ├── model.rs        # 模型转换
│       └── send.rs         # 发送逻辑
├── storage/                # 持久化层
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── state.rs        # 状态管理
│       ├── dedup.rs        # 去重记录
│       ├── log.rs          # 日志轮转
│       └── session.rs      # Session WAL 清理
├── config/                 # 配置加载/校验
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── loader.rs
│       ├── validate.rs
│       └── paths.rs
├── common/                 # 通用工具
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── utf16.rs
│       ├── time.rs
│       └── text.rs
├── logging/                # tracing 配置
│   ├── Cargo.toml
│   └── src/
│       └── lib.rs
├── sources.txt             # 来源列表（模板）
├── keywords.txt            # 关键词过滤（可选）
├── replacements.json       # 文本替换规则（模板）
├── content_addition.json   # Header/Footer（模板）
├── logs.txt                # 运行日志
├── state.json              # 运行状态
├── deduplication.json      # 去重数据库
├── user_session*           # Session 数据库与 WAL/SHM
└── data/                   # 缩略图缓存目录
```

> 说明：`logs.txt`/`state.json`/`deduplication.json`/`user_session*`/`data/` 为运行时生成内容，默认被 `.gitignore` 忽略。

## 已实现的功能

### 1. 配置管理
- 环境变量加载（兼容 `.env`）
- 配置验证
- 路径解析与标准化
- 兼容 Python 版配置项
- 配置示例文件（`config.example.env`）

### 2. 持久化层
- `state.json` 管理（Format 1/Format 2）
- `deduplication.json`（version=2, LRU）
- 日志轮转（最大行数）
- 惰性保存
- Session WAL checkpoint（sqlite3）

### 3. 核心处理
- 关键词过滤（白名单/黑名单）
- 文本替换（正则/精确匹配）
- 多层去重
  - SimHash 文本相似度
  - Jaccard 相似度（短/长文本）
  - 媒体 ID 签名
  - 媒体元数据签名
  - 缩略图感知哈希（pHash + dHash）
  - 相册去重比例阈值
  - 短文本 + 单媒体双重验证
- 内容追加（Header/Footer + UTF-16 限制）
- UTF-16 实体偏移修正
- 长 caption 实体裁剪
- 媒体过滤（大小/扩展名/MIME）
- 相册前文本合并
- FloodWait 自动等待与重试

### 4. 相册聚合
- album_id 缓存
- 满员触发（默认 10）
- 延迟触发
- 相册回补
- 相册说明合并

### 5. Telegram 适配（grammers-client）
- 客户端初始化（API_ID/API_HASH）
- 授权流程（手机号/验证码/2FA）
- 自动重连
- 更新订阅（Live）
- 消息获取（分页/按 ID）
- 聊天解析
- 文本/媒体/相册发送
- FloodWait 处理

### 6. 调度与并发
- 异步任务队列
- Worker 池（默认 3）
- 优雅关闭（超时等待）

### 7. 运行模式
- **Live**：实时更新 + 定期补漏
- **Past**：历史轮询 + 相册回补

### 8. 热更新
- 配置文件监控（`sources.txt`/`keywords.txt`/`replacements.json`/`content_addition.json`）
- 规则自动重载
- 优雅重启

### 9. 补漏机制
- 定期扫描遗漏
- 按来源分页
- 相册感知回补
- 缺口超时处理

### 10. 日志系统
- 结构化日志（tracing）
- 日志级别控制
- 日志轮转（`logs.txt`）
- 控制台输出

## 已知限制

### 功能增强
- 性能指标（CPU/内存/吞吐量）
- 更精细的重试/退避策略
- 单元测试覆盖率提升
- 集成/端到端测试

### 优化项
- 大文件/相册内存优化
- 去重数据库索引优化
- 并发参数自适应

### 兼容性
- TDLib 原生适配（当前仅 grammers）

## 依赖项

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

## 配置说明

所有配置通过环境变量或 `.env` 提供：

- `TG_API_ID`: Telegram API ID
- `TG_API_HASH`: Telegram API Hash
- `TG_TARGET`: 目标聊天
- `TG_SOURCE_FILE`: 来源文件路径（默认 `sources.txt`）
- `TG_MODE`: 运行模式（`live`/`past`）
- `TG_PAST_LIMIT`: 历史分页大小（默认 2000）
- `TG_CATCH_UP_INTERVAL`: 补漏间隔秒（默认 300）
- `TG_ALBUM_DELAY`: 相册延迟（默认 5.0）
- `TG_ALBUM_MAX_ITEMS`: 相册最大条数（默认 10，上限 10）
- `TG_ALBUM_BACKFILL_MAX_RANGE`: 相册回补范围（默认 20）
- `TG_FORWARD_DELAY`: 转发间隔（默认 0）
- `TG_FLOOD_WAIT_MAX_RETRIES`: FloodWait 最大重试（默认 5）
- `TG_FLOOD_WAIT_MAX_TOTAL_WAIT`: FloodWait 最大等待秒（默认 3600）
- `TG_REQUEST_TIMEOUT`: 请求超时秒（默认 30）
- `TG_DISPATCHER_IDLE_TIMEOUT`: 空闲超时秒（默认 300）
- `TG_UPDATES_IDLE_TIMEOUT`: 更新空闲超时秒（默认 0 表示禁用）
- `TG_WORKER_COUNT`: Worker 数量（默认 3）
- `TG_ENABLE_FILE_FORWARD`: 允许文件转发（默认 true）
- `TG_MEDIA_SIZE_LIMIT`: 媒体大小上限 MB（默认 100）
- `TG_MEDIA_EXT_ALLOWLIST`: 媒体扩展名白名单
- `TG_KEYWORD_FILE`: 关键词文件（设置后启用）
- `TG_KEYWORD_CASE_SENSITIVE`: 关键词大小写敏感（默认 false）
- `TG_KEYWORD_RELOAD_INTERVAL`: 关键词刷新秒（默认 2）
- `TG_REPLACEMENT_FILE`: 替换规则文件（设置后启用）
- `TG_CONTENT_ADDITION_FILE`: 内容追加文件（设置后启用）
- `TG_DEDUP_FILE`: 去重文件路径（默认 `deduplication.json`）
- `TG_DEDUP_LIMIT`: 文本指纹上限（默认 5000）
- `TG_MEDIA_DEDUP_LIMIT`: 媒体指纹上限（默认 15000）
- `TG_DEDUP_SIMHASH_THRESHOLD`: SimHash 阈值（默认 3）
- `TG_DEDUP_JACCARD_SHORT_THRESHOLD`: Jaccard 短文本阈值（默认 0.7）
- `TG_DEDUP_JACCARD_LONG_THRESHOLD`: Jaccard 长文本阈值（默认 0.5）
- `TG_DEDUP_RECENT_TEXT_LIMIT`: 近期文本数量（默认 100）
- `TG_DEDUP_PHASH_THRESHOLD`: pHash 阈值（默认 10）
- `TG_DEDUP_DHASH_THRESHOLD`: dHash 阈值（默认 10）
- `TG_DEDUP_ALBUM_THUMB_RATIO`: 相册缩略图比例（默认 0.34）
- `TG_DEDUP_SHORT_TEXT_THRESHOLD`: 短文本阈值（默认 50）
- `TG_THUMB_DIR`: 缩略图缓存目录（默认 `data`）
- `TG_APPEND_LIMIT_WITH_MEDIA`: 带媒体追加上限（默认 1024）
- `TG_APPEND_LIMIT_TEXT`: 文本追加上限（默认 4096）
- `TG_TEXT_MERGE_WINDOW`: 文本合并窗口（默认 0）
- `TG_TEXT_MERGE_MIN_LEN`: 文本合并最小长度（默认 0）
- `TG_TEXT_MERGE_MAX_ID_GAP`: 文本合并最大 ID 间隔（默认 5）
- `TG_STATE_FLUSH_INTERVAL`: 状态/去重落盘间隔秒（默认 5）
- `TG_SHUTDOWN_DRAIN_TIMEOUT`: 退出等待秒（默认 30）
- `TG_STATE_GAP_TIMEOUT`: 缺口等待秒（默认 300）
- `TG_STATE_PENDING_LIMIT`: Pending 上限（默认 2000）
- `TG_HOTRELOAD_INTERVAL`: 热更新检查秒（默认 2）
- `TG_LOG_LEVEL`: 日志级别（默认 INFO）
- `TG_LOG_MAX_LINES`: 日志最大行数（默认 100）
- `TG_SESSION_NAME`: Session 文件名（默认 `user_session`）
- `TG_TDLIB_DB_DIR`: TDLib 数据库目录（可选）
- `TG_TDLIB_FILES_DIR`: TDLib 文件目录（可选）
- `TG_DEVICE_MODEL`: 设备型号（可选）
- `TG_SYSTEM_VERSION`: 系统版本（可选）
- `TG_APP_VERSION`: 应用版本（可选）
- `TG_SYSTEM_LANG`: 系统语言（可选）
- `TG_USE_TEST_DC`: 使用测试 DC（可选）

## 与 Python 版本兼容性

### 兼容文件格式
- `sources.txt`
- `state.json`（Format 1/Format 2）
- `deduplication.json`（version=2）
- `keywords.txt`
- `replacements.json`
- `content_addition.json`

### 兼容配置项
- 所有环境变量

### 兼容逻辑
- 关键词过滤
- 文本替换（实体偏移）
- 去重策略（SimHash/Jaccard/相册比例/短文本阈值）
- 相册聚合逻辑

## 构建与运行

```bash
# 构建
cargo build --release

# 复制配置
cp config.example.env .env
# 填写 TG_API_ID / TG_API_HASH
# 按需更新 sources.txt / replacements.json / content_addition.json / keywords.txt

# Live 模式
cargo run --release

# Past 模式
TG_MODE=past cargo run --release

# 编译检查
cargo check

# 格式检查
cargo fmt --all --check

# Clippy
cargo clippy --all-targets --all-features -- -D warnings

# 测试
cargo test --workspace
```

## 日志与调试

日志输出到 `logs.txt`，可通过 `TG_LOG_LEVEL` 控制级别。

## 性能特点

- Tokio 异步并发
- Worker 池并发转发
- 相册智能聚合
- 去重优先级：媒体 ID → 缩略图哈希 → 文本相似度
- 双重验证降低误判
- 惰性保存减少 IO

## 安全建议

- 不要提交 `.env`
- 保护 API Hash 与会话文件
- 定期备份 `state.json` 和 `deduplication.json`
