# 贡献指南

## 开发环境

- Rust toolchain 由 `rust-toolchain.toml` 锁定（当前 1.93.0）。
  首次进入仓库时 rustup 会自动安装。
- 前端构建需要 `bun`（见 `ops/agena-studio/scripts/`）。

## 提交前清单

1. `cargo fmt --all`
2. `cargo clippy --workspace --all-targets -- -D warnings`
3. `cargo test --workspace`
4. 若改了 public API 或新增 crate：更新 `docs/ARCHITECTURE.md`
5. 若改了配置字段：同步更新 `config.example.toml` 与 `config.full.toml`
6. 若是面向用户的变化：在 `CHANGELOG.md` 顶部「Unreleased」段添加一条

## 代码规范

- 禁止在库 crate 里 `unwrap()` / `expect()` / `panic!()`，除非
  - 在 `#[cfg(test)]` 中
  - 用 `unreachable!` 标注的结构不变量
- 错误类型：库 crate 用 `thiserror` 自定义 enum；二进制 crate 用 `anyhow`
- 注释只写 *为什么*，不解释 *做什么* —— 命名应已传达后者
- 不要为了"将来可能"添加抽象层；三处相似代码胜过一个早产抽象
- 涉密字段（API key / secret）必须自定义 `Debug` 实现以打码

## 依赖管理

- 公共依赖在 workspace 根 `Cargo.toml` 的 `[workspace.dependencies]` 声明，
  子 crate 用 `foo = { workspace = true, features = [...] }` 引用
- 新增直接依赖前先 grep 看是否已有等价依赖
- `cargo-deny`、`cargo-audit` 在 CI 强制运行；若需 ignore advisory，
  在 `deny.toml` 留出原因注释

## 提交信息

- 标题用祈使句，≤ 72 字（中文 ≤ 36 字）
- 正文说明 *动机* 与 *取舍*，不必复述 diff
- 用脚注关联 issue：`Fixes #123` / `Refs #123`

## 拉取请求

- 单 PR 只做一件事；大重构拆为多 PR 串行评审
- CI 全绿后才会被 review；本地先跑 `cargo clippy` 再 push
- Reviewer 会优先关注：错误处理、并发安全、配置兼容性
