# Rust + grammers Telegram 转发器

## 项目结构

```
rust_tdlib/
├── Cargo.toml              # 工作区配置
├── app/                    # 主程序入口
│   ├── Cargo.toml
│   └── src/
│       ├── main.rs         # 应用主逻辑（1868行）
│       └── thumb_dedup.rs  # 缩略图去重工具
├── core/                   # 核心业务逻辑
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── model.rs        # 数据模型
│       ├── pipeline.rs     # 处理流水线（编排所有处理步骤）
│       ├── filter.rs       # 关键词过滤
│       ├── replace.rs      # 文本替换（正则/精确匹配）
│       ├── dedup.rs        # 多层去重（SimHash/Jaccard/媒体签名）
│       ├── album.rs        # 相册聚合与缓存
│       ├── append.rs       # 内容追加（Header/Footer）
│       ├── dispatch.rs     # 任务调度（Worker池）
│       ├── catch_up.rs     # 补漏逻辑（定期扫描遗漏消息）
│       ├── hotreload.rs    # 热更新（配置文件监控）
│       ├── media_filter.rs # 媒体过滤（大小/类型/扩展名）
│       ├── text_merge.rs   # 文本合并（相册前文本合并）
│       ├── entity_trim.rs  # 实体修剪（移除过长实体）
│       ├── throttle.rs     # 发送节流（FloodWait控制）
│       ├── thumb.rs        # 缩略图哈希（pHash/dHash）
│       └── sources.rs      # 源解析（读取sources.txt）
├── tdlib/                  # Telegram客户端适配层（基于grammers）
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── client.rs       # 客户端封装（连接/授权/订阅）
│       ├── model.rs        # Telegram类型到业务模型转换
│       └── send.rs         # 消息发送（文本/媒体/相册）
├── storage/                # 持久化层
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── state.rs        # 状态管理（last_committed/pending）
│       ├── dedup.rs        # 去重记录（deduplication.json）
│       ├── log.rs          # 日志轮转
│       └── session.rs      # Session WAL 文件自动清理
├── config/                 # 配置管理
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── loader.rs       # 配置加载（环境变量）
│       ├── validate.rs     # 配置验证
│       └── paths.rs        # 路径解析（相对→绝对）
├── common/                 # 通用工具
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── utf16.rs        # UTF-16编码处理（Telegram长度限制）
│       ├── time.rs         # 时间工具
│       └── text.rs         # 文本工具
├── logging/                # 日志基础设施
│   ├── Cargo.toml
│   └── src/
│       └── lib.rs          # Tracing配置
└── data/                   # 运行时数据（自动创建）
    ├── logs.txt            # 应用日志
    ├── state.json          # 处理状态（per-source last_committed）
    ├── deduplication.json  # 去重数据库（version=2）
    ├── user_session        # Telegram Session 主文件
    ├── user_session-wal    # SQLite WAL 文件（定期 checkpoint 缩小）
    └── user_session-shm    # SQLite 共享内存文件（由 SQLite 管理）
```

## 已实现的功能

### 1. 配置管理
- 环境变量加载（兼容 .env 文件）
- 配置验证
- 路径解析与标准化（相对→绝对路径）
- 兼容 Python 版本的所有配置项
- 配置示例文件（config.example.env）

### 2. 持久化层
- state.json 管理（支持两种格式，Format 1/Format 2）
- deduplication.json 管理（version=2，支持 LRU 淘汰）
- 日志文件轮转（可配置保留行数）
- 惰性保存策略（减少 IO 开销）
- Session WAL 文件定期 checkpoint（通过 sqlite3，防止 WAL 文件无限增长）

### 3. 核心业务逻辑
- 关键词过滤（白名单/黑名单，可配置大小写敏感）
- 文本替换（正则/精确匹配，支持实体偏移调整）
- 多层去重策略：
  - SimHash 文本相似度（可配置汉明距离阈值）
  - Jaccard 词组相似度（短/长文本动态阈值）
  - 媒体 ID 签名
  - 媒体元数据签名（尺寸、MIME类型）
  - 缩略图感知哈希（pHash + dHash）
  - 智能相册去重（单图严格，多图宽松，可配置比例阈值）
  - 双重验证机制（单媒体+短文本需文本和媒体同时匹配）
- 内容追加（Header/Footer，UTF-16 长度限制处理）
- UTF-16 实体偏移计算与调整
- 过长实体裁剪（媒体 caption 超 1024 UTF-16 时自动移除 Code/Pre/Blockquote）
- 媒体过滤（文件大小、扩展名白名单、MIME 类型验证）
- 文本合并（相册前的文本消息缓存与合并）
- 发送节流（FloodWait 自动等待与重试）

### 4. 相册聚合
- 相册缓存机制（按 album_id 分组）
- 满员立即触发（可配置，默认10张）
- 定时延迟触发（可配置延迟时间）
- 相册回补（自动获取缺失项目）
- 文本合并（相册前的短文本合并为统一说明）

### 5. Telegram 客户端适配（基于 grammers-client）
- 客户端初始化（API_ID/API_HASH 授权）
- 授权流程（手机号、验证码、二步验证）
- 连接管理（自动重连）
- 更新流订阅（实时模式）
- 消息获取（分页、按ID获取）
- 聊天实体解析（用户名/链接/ID）
- 消息发送（文本/媒体/相册）
- FloodWait 处理（自动等待与重试）

### 6. 调度与并发
- 任务队列（异步通道）
- 并发 Worker 池（可配置数量，默认4-10）
- 优雅关闭（等待任务完成 + 超时提示）

### 7. 运行模式
- **Live 模式**：
  - 实时订阅 Telegram 更新
  - 即时处理新消息
  - 定期补漏扫描（可配置间隔）
  - 相册智能聚合（延迟 + 满员触发）
- **Past 模式**：
  - 历史消息轮询
  - 分页拉取（可配置页面大小）
  - 相册回补与聚合
  - 持续轮询直到停止

### 8. 热更新
- 配置文件监控（sources.txt/keywords.txt/replacements.json/content_addition.json）
- 自动重载关键词/替换规则
- 优雅重启（保存状态 + 重新初始化）

### 9. 补漏机制
- 定期扫描遗漏消息（可配置间隔）
- 按来源分页获取
- 相册感知回补
- 缺口检测与超时处理

### 10. 日志系统
- 结构化日志（tracing）
- 日志级别配置（DEBUG/INFO/WARNING/ERROR）
- 文件日志轮转（data/logs.txt）
- 控制台日志输出

## 已知限制与待优化

### 功能增强
- 性能监控与指标收集（CPU/内存/吞吐量）
- 更精细的错误重试策略（指数退避）
- 单元测试覆盖率提升
- 集成测试与端到端测试

### 优化项
- 大文件/相册的内存使用优化
- 去重数据库的索引优化
- 并发控制参数的自适应调整

### 兼容性
- 支持 TDLib 原生库（当前仅支持 grammers-client）

## 依赖项

```toml
[workspace.dependencies]
tokio = "1.35"              # 异步运行时
serde = "1.0"               # 序列化
serde_json = "1.0"
anyhow = "1.0"              # 错误处理
thiserror = "1.0"
tracing = "0.1"             # 日志
tracing-subscriber = "0.3"
tracing-appender = "0.2"
dotenv = "0.15"             # 环境变量
regex = "1.10"              # 正则表达式
grammers-client = "0.8"     # Telegram MTProto 客户端
grammers-session = "0.8"    # Session 管理（SQLite）
grammers-mtsender = "0.8"   # MTProto sender
siphasher = "1.0"           # SimHash 算法
image = "0.25"              # 图像处理（缩略图哈希）
```

## 配置说明

所有配置项兼容 Python 版本，通过环境变量或 `.env` 文件设置：

- `TG_API_ID`: Telegram API ID
- `TG_API_HASH`: Telegram API Hash
- `TG_TARGET`: 目标聊天
- `TG_SOURCE_FILE`: 来源文件路径
- `TG_MODE`: 运行模式（live/past，past 为历史轮询）
- `TG_PAST_LIMIT`: 历史轮询分页大小（单次请求上限 100，首轮用于回补窗口）
- `TG_CATCH_UP_INTERVAL`: 轮询/补漏间隔（秒，past 模式轮询频率，live 模式补漏周期）
- `TG_ALBUM_DELAY`: 相册聚合延迟
- `TG_ALBUM_MAX_ITEMS`: 相册聚合最大条数
- `TG_ALBUM_BACKFILL_MAX_RANGE`: 相册补漏回拉最大 ID 范围
- `TG_FORWARD_DELAY`: 转发间隔
- `TG_FLOOD_WAIT_MAX_RETRIES`: FloodWait 最大重试次数
- `TG_FLOOD_WAIT_MAX_TOTAL_WAIT`: FloodWait 最大累计等待时间（秒）
- `TG_REQUEST_TIMEOUT`: 请求超时（秒，用于发送与历史拉取）
- `TG_DISPATCHER_IDLE_TIMEOUT`: 等待发送任务完成的超时时间（秒，超时会触发重连）
- `TG_UPDATES_IDLE_TIMEOUT`: 更新流空闲超时（秒，0 表示禁用，超时会触发重连）
- `TG_WORKER_COUNT`: Worker 数量
- `TG_ENABLE_FILE_FORWARD`: 是否允许文档/文件转发
- `TG_KEYWORD_FILE`: 关键词文件
- `TG_REPLACEMENT_FILE`: 替换规则文件
- `TG_CONTENT_ADDITION_FILE`: 内容追加文件
- `TG_DEDUP_FILE`: 去重记录文件路径（默认项目根 `deduplication.json`）
- `TG_STATE_FLUSH_INTERVAL`: 状态/去重落盘与日志截断间隔
- `TG_SHUTDOWN_DRAIN_TIMEOUT`: 退出/热重启等待任务完成的超时阈值（超时会告警；热重启超时后不再等待任务完成；退出信号下仍可能强制退出）
- `TG_STATE_GAP_TIMEOUT`: 状态缺口等待超时（秒）
- `TG_STATE_PENDING_LIMIT`: 状态待补队列上限
- `TG_HOTRELOAD_INTERVAL`: 热更新检查间隔
- `TG_DEDUP_SIMHASH_THRESHOLD` / `TG_DEDUP_JACCARD_SHORT_THRESHOLD` / `TG_DEDUP_JACCARD_LONG_THRESHOLD` / `TG_DEDUP_RECENT_TEXT_LIMIT`: 去重阈值
- `TG_DEDUP_ALBUM_THUMB_RATIO`: 多图片去重比例阈值（0.0-1.0，默认0.34）
- `TG_DEDUP_SHORT_TEXT_THRESHOLD`: 短文本阈值（字符数，默认50，用于双重验证）
- `TG_APPEND_LIMIT_WITH_MEDIA` / `TG_APPEND_LIMIT_TEXT`: 文本追加长度上限
- `TG_TEXT_MERGE_WINDOW` / `TG_TEXT_MERGE_MIN_LEN` / `TG_TEXT_MERGE_MAX_ID_GAP`: 文本合并窗口与阈值
- 等等...

## 与 Python 版本的兼容性

### 兼容的文件格式
- `state.json`（两种格式）
- `deduplication.json`（version=2）
- `keywords.txt`（白名单/黑名单）
- `replacements.json`（正则/精确替换）
- `content_addition.json`（header/footer）

### 兼容的配置项
- 所有环境变量配置

### 兼容的业务逻辑
- 关键词过滤规则
- 文本替换规则（包括实体偏移调整）
- 去重策略（SimHash 阈值=3，Jaccard 动态阈值，多图片比例阈值=0.34，短文本阈值=50）
- 双重验证机制（单媒体+短文本需文本和媒体同时匹配才判重）
- 相册聚合逻辑（满员触发、定时触发、动态续时）

## 构建与运行

```bash
# 构建
cargo build --release

# 首次运行需要配置 .env 文件
cp config.example.env .env
# 编辑 .env 文件，填入 TG_API_ID 和 TG_API_HASH

# 运行（实时模式）
cargo run --release

# 运行（历史轮询模式）
TG_MODE=past cargo run --release

# 检查编译
cargo check

# 格式化检查
cargo fmt --all --check

# 静态检查
cargo clippy --all-targets --all-features -- -D warnings

# 运行测试
cargo test --workspace
```

## 日志与调试

日志输出到 `data/logs.txt`，支持以下级别：
- DEBUG
- INFO
- WARNING
- ERROR

通过 `TG_LOG_LEVEL` 环境变量设置。


## 性能特点

- 异步并发处理（Tokio）
- 多 Worker 并发转发
- 相册智能聚合
- 去重优化（媒体 ID 优先，缩略图哈希次之，文本相似度兜底）
- 智能多图去重（单图严格，多图宽松，避免误判）
- 双重验证机制（单媒体+短文本场景，防止"相同评论+不同图片"误判）
- 惰性保存（减少 IO 开销）

## 安全建议

- 不要将 `.env` 文件提交到版本控制
- 保护好 API Hash 和会话文件
- 定期备份 `state.json` 和 `deduplication.json`
