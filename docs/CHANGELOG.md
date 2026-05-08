# Changelog

格式遵循 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)，
版本号遵循 [SemVer](https://semver.org/lang/zh-CN/)。

## [Unreleased]

### Added
- `[workspace.dependencies]` / `[workspace.lints]` 统一所有子 crate 的版本与 lint
- `rust-toolchain.toml`、`rustfmt.toml`、`clippy.toml`、`deny.toml`
- CI 增加 `cargo fmt --check`、`cargo clippy -D warnings`、`cargo deny`、
  `cargo audit`、macOS runner、lockfile 校验
- `agena-api-server` 增加 `/healthz`、`/readyz`、`/metrics` 端点
- 顶层文档：`docs/ARCHITECTURE.md`、`docs/CONTRIBUTING.md`、
  `docs/SECURITY.md`、`docs/CHANGELOG.md`
- `config.full.toml`：原 `config.example.toml` 的完整版

### Changed
- `config.example.toml` 简化为最小可运行配置
- `HttpProviderConfig`、`GitlabProviderOptions`、`BedrockAuthConfig`、
  `CloudflareAiGatewayProviderOptions`、`WebSearchConfig`、
  `WebSearchBackend` 的 `Debug` 输出对 `api_key` / `secret_*` 字段打码
- `session/manager.rs` 的并发工具调度通过 `Semaphore(32)` 限流，
  防止耗尽 tokio blocking pool
- 工作区 axum 统一到 0.8.8；reqwest 统一到 0.13.2；mcp-client 从 0.12 升级

### Removed
- `agena serve` 子命令的迁移残留提示（HTTP server 已迁至
  `apps/agena-studio-server`）

### Security
- `InMemorySecretStore` 的 RwLock poisoned 路径改用
  `unwrap_or_else(|e| e.into_inner())` 优雅恢复，不再 panic
