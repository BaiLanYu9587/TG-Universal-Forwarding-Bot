0. 文件目的与适用范围

本文件用于约束 AI 代码编辑/编码代理在本仓库内的行为与产出，确保改动可控、可审计、可回滚。

适用范围：对整个仓库生效。若子目录存在更具体的 AGENTS.md，子目录规则优先（更严格者优先）。

冲突处理：当本文件与仓库其他规则（README/CI/SECURITY 等）冲突时，以更严格、更安全、更保守的要求为准。

Cursor/Copilot 规则：未发现 .cursor/rules、.cursorrules 或 .github/copilot-instructions.md。

1. 总体目标与非目标

目标：
- 优先保证正确性与可维护性，改动尽量小且聚焦。
- 保持现有公共 API 行为稳定，避免无关重构。
- 变更必须能通过既定检查（fmt/clippy/test），或说明无法通过的原因。

非目标：
- 未经明确指示，不做大规模重构/架构重写/目录迁移。
- 未经明确指示，不升级 Rust edition、MSRV、关键依赖大版本。
- 不为“看起来更优雅”改变既有可工作逻辑。

2. 修改前必须做的事（理解优先）

动手改代码前必须先完成：
- 阅读：README.md、Cargo.toml（workspace 配置）、app/src/main.rs 或 core/src/lib.rs。
- 识别：目标 crate、目标模块、已有实现方式与错误处理风格（anyhow/thiserror）。
- 查找：是否已有类似功能/工具函数/宏，优先复用而非重复实现。

缺少必要上下文时：采取保守策略，最小修改并在输出中列出假设与风险点。

3. 构建/格式化/静态检查/测试

构建与运行（workspace 根目录执行）：
- 构建（发布版）：cargo build --release
- 本地运行：cargo run --release
- 模式切换：TG_MODE=past cargo run --release
- 快速编译检查：cargo check

格式化与静态检查：
- 格式化检查：cargo fmt --all --check
- 自动格式化：cargo fmt --all
- Clippy（严格）：cargo clippy --all-targets --all-features -- -D warnings

测试：
- 全量测试：cargo test --workspace --all-features
- 仅工作区默认：cargo test --workspace
- 仅某 crate：cargo test -p core
- 单个测试函数：cargo test -p core test_name
- 单个测试模块（lib/unit）：cargo test -p core --lib test_name
- 单个集成测试文件：cargo test -p core --test test_file
- 过滤包含关键字的测试：cargo test -p core keyword

如需最小验证顺序建议：fmt -> clippy -> test。

运行前准备：
- 复制示例配置：cp config.example.env .env
- 填写 TG_API_ID/TG_API_HASH 后再运行。
- .env 与 logs.txt/state.json/deduplication.json/user_session* 为运行时产物，不提交。
- data/ 目录自动创建用于缩略图缓存，日志写入 logs.txt。

- 使用 TG_LOG_LEVEL 控制日志级别。
- 运行时依赖 Telegram 授权流程（手机号/验证码/二步验证）。

4. 代码风格与结构约定

Rust 版本与 edition：
- 使用 workspace 定义的 edition = 2021。
- 不使用高于 MSRV 的特性或不稳定 feature。

模块与组织：
- 以现有模块结构为准（app/core/tdlib/storage/config/common/logging）。
- 新功能优先放入对应 crate 的现有模块层级。
- 不跨 crate 重复实现，优先共享 common/config 等工具。

命名规范：
- 类型/结构体/枚举：PascalCase。
- 函数/变量/模块：snake_case。
- 常量：SCREAMING_SNAKE_CASE。
- 布尔命名倾向使用 is_/has_/should_ 前缀。

类型与接口：
- 公共接口尽量显式类型（避免过度 type inference）。
- 避免无意义的泛型；优先清晰、可读的签名。
- 使用 Arc/Mutex/RwLock 时说明所有权与并发语义是否合理。

Imports：
- 建议分组：std -> 第三方 crate -> workspace 内部 crate。
- 每组之间空行分隔，避免无序混排。
- 尽量避免 glob 引入；只在明确需要时使用。

错误处理：
- 二进制（app）使用 anyhow::Result，并通过 Context 补充上下文。
- 库 crate 使用 thiserror 定义稳定错误类型，避免字符串拼接错误。
- 不吞错；仅在明确意图时使用 let _ = 并说明原因。
- 优先使用 ? 传播错误，减少手动 match。
- 非测试代码避免 unwrap/expect。

日志与可观测性：
- 使用 tracing（debug/info/warn/error），遵循现有字段与风格。
- 日志字段命名保持一致，优先结构化字段而非拼接字符串。
- 日志不得包含密钥、令牌、密码、个人信息。
- 对外错误信息需可定位问题但避免泄露敏感数据。

格式化与风格：
- 使用 rustfmt 默认风格，避免手动对齐。
- 维持现有换行/缩进方式，减少无关格式变化。
- 不新增 inline 注释，除非需求明确或涉及 unsafe 说明。

5. 依赖与供应链安全

- 默认不新增依赖；如需新增，必须说明必要性与替代方案。
- 变更依赖后更新 Cargo.lock（若存在且应提交）。
- 禁止大范围依赖升级（cargo update 全量）除非明确要求。

6. 并发、异步与性能

- 不引入新的异步运行时/并发模型（例如改用 async-std）。
- async 代码避免阻塞调用，必要时使用 spawn_blocking。
- 避免在热路径引入不必要分配/锁；需要时说明原因与影响。
- 长循环中尽量复用缓冲区，减少频繁分配。
- 禁止新增 unsafe；如必须，需注明不变量并补充测试。

7. 文档与示例

- 公共 API 变更必须更新对应文档注释或 README。
- 新增/变更环境变量时同步更新 README 与 config.example.env。
- 示例与测试优先，长注释其次。

8. 输出与交付要求

每次完成修改后输出必须包含：
- 变更摘要（做了什么、为什么）。
- 涉及文件（主要修改路径）。
- 行为变化（哪些输入/场景改变输出）。
- 验证方式（执行/建议执行的命令）。
- 风险与假设（不确定点与潜在回归）。
- 回滚建议（最小回滚点）。

9. 需要人工确认的高风险改动（遇到必须停）

以下任一情况必须停止并请求人工决策：
- 数据格式/协议/数据库 schema 的破坏性变化。
- 认证、鉴权、权限模型调整。
- 引入或升级关键依赖的大版本。
- 需要新增 unsafe，或需要放宽 clippy/安全检查。
- 更改 MSRV、edition，或启用 nightly 特性。
