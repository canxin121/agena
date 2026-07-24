# Agena 单一 Binary、Studio 收口与 Rust 持续拆分执行计划

> 状态：待执行
>
> 规划基线：`fa201e39652e`（`refactor: split runtime, TUI, and Studio implementation owners`）
>
> 规划日期：2026-07-24
>
> 事实来源：`cargo metadata --format-version 1 --locked`、`scripts/rust-architecture-report.py`，以及当前 Rust/Web/CI/发布脚本源码。
>
> 本文件是仓库唯一保留的重构执行计划。此前的 app/TUI/runtime 拆分计划已删除；已落地结构以当前源码、Cargo metadata 和架构报告为准。
>
> 执行模式：一条连续源码迁移列车。先冻结范围，一次性创建目标 crate/module，连续完成所有 `git mv`、模块拆分、路径/API/manifest/Web/脚本/CI/文档改写和静态收口；在 source train 完整关闭之前，不运行 `cargo check`、`cargo test` 或 Clippy。第一次编译发生在所有 ownership 与命名变更完成之后，随后只按错误类别批量修复，最后统一完成测试、lint、构建、打包与 smoke。

## 1. 结论

本轮必须同时完成四件事，不能只做其中一部分：

1. **删除独立的 Agena Studio 产品和 binary**：不再有 `agena-studio` 可执行文件、`agena-studio-server` package、独立 Studio release/service/package 身份。
2. **统一到一个产品 binary**：TUI、普通 CLI、stdio RPC server 和 HTTP/Web server 都由唯一的 `agena` binary 解析并启动。
3. **继续真实拆分大 crate、module tree 和 `.rs` 文件**：不能因为 server 合并进 `agena` 就把 10k–20k 行重新塞进 `main.rs` 或一个新的 `server.rs`；单一 executable 与 library/module 拆分必须同时成立。
4. **先快速连续修改，最后验证**：从第一条移动开始，直到 Rust、Web、CI、发布和文档全部静态收口，禁止编译驱动式来回修改；整个迁移主要依靠目录移动、批量路径改写、脚本化断言和模块级抽取，而不是逐行重写。

本计划中的“只有一个 bin”指 **只有一个 Agena 产品可执行文件**。示例插件和 `tools/agena-e2e` 的 fixture/probe binary 是测试资产，不属于产品 binary，不要求删除。

单一 binary 也不等于单一 crate 或单一源文件。最终必须保持：

- `apps/agena` 只声明一个 `[[bin]] name = "agena"`；
- `main.rs` 只负责构建进程 runtime、解析 launch intent 和顶层 dispatch；
- TUI、CLI presentation、runtime、HTTP API、Git、session、provider 和 plugin 继续由清晰的 library crate/module owner 承担；
- server 的进程级 composition/router 在 `agena` binary module tree 内，复用领域 library，而不是恢复一个第二 server binary；
- 任何新 library crate 都不得产生新的产品 binary。

## 2. 不可妥协的约束

### 2.1 单一产品入口

最终只支持以下产品入口：

```text
agena                              默认启动 TUI
agena tui ...                      显式启动 TUI
agena <command> ...                普通 CLI command
agena rpc-server ...               现有 stdio App Server/IDE RPC 模式
agena server ...                   HTTP API + Web UI + terminal/fs/git/preview server
```

必须删除：

- `agena-studio` binary target；
- `agena-studio-server` Cargo package；
- 独立的 Studio backend archive/service executable；
- 任何调用 `cargo run -p agena-studio-server` 或 `bin/agena-studio` 的脚本、CI、README 和配置文档。

`app-server` 当前实际是 stdio JSON-RPC server，不是旧 HTTP Studio server。为了消除两个含糊的 “server” 名称，本轮将它正式命名为 `rpc-server`；如果产品明确要求一个过渡别名，只能在 `clap` 中保留一个隐藏且带 removal issue 的 alias，不能保留第二 launch implementation。

### 2.2 不把 server 重新做成巨型 bin 文件

禁止以下结果：

- 把 `apps/agena-studio-server/src/**` 复制进 `apps/agena/src/main.rs`；
- 新建一个 10,000 行的 `apps/agena/src/server.rs`；
- 为了减少 module 声明而使用 `include!`；
- 把原 server package 改名成 `agena-server` 后继续产出第二 binary；
- 用 compatibility facade 让旧 `agena-studio-server` package 依赖新 `agena`；
- 同时维护旧 server 和新 server 两套 router/Args/runtime bootstrap。

### 2.3 Source train 期间禁止编译驱动

从第一条 `git mv` 或第一个新 crate/module patch 开始，到第 14 节的静态收口清单全部完成之前，禁止运行：

```text
cargo check
cargo test
cargo clippy
cargo build
cargo run
bun test
bun run typecheck
bun run build
```

允许并要求运行的只读/静态命令包括：

```text
rg / rg --files
git status --short
git diff --name-status
git diff --stat
git diff --check
cargo metadata --format-version 1 --no-deps
python3 scripts/rust-architecture-report.py
```

`cargo metadata` 不编译 Rust，只用于确认 workspace/package/target graph。修改 package graph 后只在所有 manifest 一次性改完时执行一次不带 `--locked` 的受控 lockfile 刷新，随后恢复 `--locked`。

### 2.4 快速不等于复制或无边界替换

禁止：

- 手工逐行重写已有实现；
- 复制源码到新目录后保留旧文件；
- symlink、hard link、alias crate、`include!` 或双实现；
- 无文件白名单、无命中数断言的全仓库字符串替换；
- 为了过编译把所有 item 改成 `pub`；
- 在第一次 check 后继续随意移动 ownership；
- 先建大量空 facade，再等编译器逐条告诉实现应该放在哪里。

必须优先使用：

- `git mv` 移动完整目录或文件；
- `apply_patch` 一次完成 root module、manifest 和有锚点的 item 移动；
- 带精确输入文件、旧字符串计数和新字符串计数断言的一次性转换脚本；
- `rg` 生成受影响文件集，再对固定集合执行机械改写；
- `git diff --find-renames` 审计真实移动；
- 所有源码批量完成后统一 `cargo fmt`，不在每个小步骤反复格式化。

## 3. 当前量化事实

当前 Python 架构报告覆盖：

| 指标 | 当前值 |
| --- | ---: |
| Workspace packages | 54 |
| Rust targets | 59 |
| Binary targets | 7 |
| 第一方 `.rs` 文件 | 1,082 |
| Rust 源码行 | 335,586 |
| 模块解析未找到/歧义 | 0 |
| Lexer 告警 | 0 |
| 第一方 normal dependency cycle | 0 |

与本计划直接相关的 target/package：

| Owner | 行数 / 文件 | 模块节点 / 引用边 | 主要问题 |
| --- | ---: | ---: | --- |
| `agena-tui-app` | 38,350 / 103 | 138 / 207 | 最大生产 target，37 个 `app_*` 一级模块约 26,641 行 |
| `agena-runtime-provider` | 30,449 / 56 | 78 / 164 | provider tree 26,391 行，vendor adapter 与 shared registry 混合 |
| `agena-runtime-session` | 26,374 / 85 | 120 / 247 | 引用边最高；`session::manager` 单树 10,307 行 |
| `agena-macros` | 15,845 / 35 | 35 / 89 | proc-macro target 承载全部 parse/validation/codegen support |
| `agena-runtime` | 14,845 / 69 | 87 / 134 | 行数已下降，但仍有 25 个第一方直接依赖 |
| `agena-tui-transcript` | 13,146 / 17 | 26 / 55 | renderer 8,367 行，`transcript_ast.rs` 4,067 行 |
| `agena-provider` | 12,328 / 58 | 81 / 122 | `lib.rs` 3,054 行且是高 blast-radius 公共契约根 |
| `agena-runtime-plugins` | 12,167 / 39 | 50 / 66 | `plugins::provided` 8,695 行，package 直接依赖 16 个第一方 crate |
| `agena-studio-git` | 11,308 / 35 | 35 / 49 | 名称仍绑定已取消的 Studio 产品；实际是 Git HTTP vertical slice |
| `agena-studio-server` | 10,020 / 28 | 29 / 58 | 独立 `agena-studio` bin，必须并入唯一 `agena` |
| `apps/agena` | 673 / 2 | lib + bin | 同 package 的 `agena_app` lib 只被自己的 main 使用，可收进唯一 bin module tree |

当前 `agena-studio-server` 的最大文件：

| 文件 | 行数 |
| --- | ---: |
| `terminal.rs` | 1,347 |
| `fs/fs_core.rs` | 1,014 |
| `workspace_preview.rs` | 930 |
| `workspace_preview_registry.rs` | 847 |
| `app.rs` | 835 |
| `ui_auth.rs` | 719 |
| `fs/fs_content.rs` | 719 |
| `config/routes/settings/sanitize.rs` | 474 |
| `terminal_ui_state.rs` | 472 |

Studio 产品身份还散落在：

- `packages/agena-studio-web`；
- `scripts/agena-studio/**`；
- `.github/workflows/agena-studio-release.yml`；
- CI job、README、configuration docs；
- `AGENA_STUDIO_*` 环境变量；
- `agena-studio.db`、`.config/agena-studio`、localStorage namespace；
- `/api/agena-studio/diagnostics`、`agena-studio:fs-changed`、`agena-studio.auth-required`；
- tracing target、tmux prefix、service/plist/systemd name 和 release tag/archive。

因此“删除 Studio”必须是产品面、源码面、运维面和持久化面的一次完整迁移，不能只删一个 Cargo target。

## 4. 最终产品与源码拓扑

### 4.1 唯一 binary

最终 `apps/agena/Cargo.toml`：

```text
[package]
name = "agena"
autobins = false

[[bin]]
name = "agena"
path = "src/main.rs"
```

删除 `[lib] name = "agena_app"`。当前 `apps/agena/src/lib.rs` 中的 TUI process adapter 通过真实移动进入 binary module tree，而不是复制。

目标 module tree：

```text
apps/agena/src/
├── main.rs                         <= 80–120 行；只做 runtime + parse + dispatch
├── error.rs                        AgenaProcessError
├── launch/
│   ├── mod.rs                      LaunchMode 顶层分派
│   ├── tracing.rs                  各 mode 唯一 tracing 初始化边界
│   ├── command.rs                  普通 CLI command adapter
│   ├── tui.rs                      原 agena_app TUI process adapter
│   └── rpc_server/
│       ├── mod.rs                  stdio RPC server lifecycle
│       ├── backend.rs              AppServerBackend 实现
│       └── mapping.rs              request/response/domain mapping
└── server/
    ├── mod.rs                      HTTP server launch API
    ├── bootstrap.rs                runtime/application/server state composition
    ├── state.rs                    ServerState + narrow adapter impl
    ├── router.rs                   顶层 router 合并；不放业务实现
    ├── diagnostics.rs
    ├── cors.rs
    ├── error.rs
    ├── auth/
    ├── attachment/
    ├── config/
    ├── fs/
    ├── persistence/
    ├── preview/
    ├── settings/
    └── terminal/
```

`main.rs` 不拥有任何 HTTP route、session backend method、TUI loop 或 CLI command implementation。

### 4.2 Library crate 方向

目标依赖方向：

```text
agena binary
├── agena-cli
├── agena-runtime
├── agena-api-server / agena-application
├── agena-tui-app / backend / platform
└── agena-git-http

agena-runtime
├── agena-runtime-provider
├── agena-runtime-provider-adapters
├── agena-runtime-session
├── agena-runtime-session-core
├── agena-runtime-plugins
├── agena-bundled-plugins
└── existing config/contracts/tools/etc.

agena-macros
└── agena-macro-core
```

强制无环规则：

- `agena-git-http` 不依赖 `agena` binary；
- `agena-runtime-provider-adapters` 可以依赖 `agena-runtime-provider`，反向禁止；
- `agena-runtime-session` 可以依赖 `agena-runtime-session-core`，反向禁止；
- `agena-bundled-plugins` 可以依赖 plugin runtime contract/core，`agena-runtime-plugins` 不得依赖 bundled implementations；
- `agena-macros` 依赖 `agena-macro-core`，macro core 不得依赖 proc-macro crate；
- 所有 feature/library crate 禁止依赖 `apps/agena`。

### 4.3 CLI launch model

`agena-cli` 只拥有参数 schema 和 typed launch intent，不拥有 Tokio runtime、HTTP listener、TUI terminal 或 process-global tracing。

目标类型：

```text
AgenaCommand::Server(ServerArgs)
AgenaCommand::RpcServer(RpcServerArgs)
AgenaCommand::Tui(TuiArgs)
...ordinary commands...

LaunchMode::Server(ServerLaunchRequest)
LaunchMode::RpcServer(RpcServerLaunchRequest)
LaunchMode::Tui(TuiLaunchRequest)
LaunchMode::Command(AgenaCli)
```

`ServerArgs` 接收原 HTTP server 参数：

- `--host`
- `--port`
- `--ui-password`
- `--workspace`
- `--ui-dir`
- `--cors-origin`
- `--cors-allow-all`
- `--ui-cookie-samesite`

全局 `--set`、`--database-url`、`--database-path` 继续由 `AgenaCli` 统一解析，并在 `into_launch_mode` 中合并进 `ServerLaunchRequest`/`RpcServerLaunchRequest`，不能在 server module 再解析一次环境或 CLI。

## 5. Studio 产品身份的完整移除

### 5.1 必须真实 rename/move 的路径

使用 `git mv`：

```text
crates/agena-studio-git
  -> crates/agena-git-http

packages/agena-studio-web
  -> packages/agena-web-ui

scripts/agena-studio
  -> scripts/agena

.github/workflows/agena-studio-release.yml
  -> .github/workflows/agena-release.yml
```

`apps/agena-studio-server/src/**` 不整体改名为另一个 app package，而是按第 7 节的 owner map 真实移动到 `apps/agena/src/server/**`。完成后删除空的 `apps/agena-studio-server/Cargo.toml` 和目录。

### 5.2 Cargo 名称

```text
agena-studio-server        -> 删除
agena-studio binary        -> 删除
agena-studio-git           -> agena-git-http
agena_studio_git Rust root -> agena_git_http
```

根 workspace：

- 删除 member `apps/agena-studio-server`；
- 将 member/dependency `crates/agena-studio-git` 改为 `crates/agena-git-http`；
- `apps/agena` 吸收 HTTP server 所需 direct dependencies；
- 删除 `apps/agena` 的 `[lib]` target；
- 新增本轮拆分的 library members；
- 只受 server 使用的 dependency 不得泄漏到无关 TUI/CLI feature crate。

### 5.3 Web、发布与服务身份

目标：

```text
Web package          agena-web-ui
Release workflow     Agena Release
Release tag          agena-v<version>
Archive              agena-<target>-v<version>.*
Executable           bin/agena[.exe]
Service command      agena server ...
systemd unit         agena.service
launchd label        cn.cxits.agena
Install root         ~/agena
Web dist             packages/agena-web-ui/dist
```

release version 必须从 root workspace/Cargo metadata 读取，不能再用 `sed` 从一个 `version.workspace = true` 的旧 server manifest 猜版本。

### 5.4 Canonical runtime identifiers

全部改为：

| 旧标识 | 新标识 |
| --- | --- |
| `AGENA_STUDIO_HOST` | `AGENA_SERVER_HOST` |
| `AGENA_STUDIO_PORT` | `AGENA_SERVER_PORT` |
| `AGENA_STUDIO_UI_PASSWORD` | `AGENA_SERVER_UI_PASSWORD` |
| `AGENA_STUDIO_UI_DIR` | `AGENA_SERVER_UI_DIR` |
| `AGENA_STUDIO_CORS_ORIGINS` | `AGENA_SERVER_CORS_ORIGINS` |
| `AGENA_STUDIO_CORS_ALLOW_ALL` | `AGENA_SERVER_CORS_ALLOW_ALL` |
| `AGENA_STUDIO_UI_COOKIE_SAMESITE` | `AGENA_SERVER_UI_COOKIE_SAMESITE` |
| `AGENA_STUDIO_DATA_DIR` | `AGENA_SERVER_DATA_DIR` |
| `AGENA_STUDIO_BUN_PATH` | `AGENA_SERVER_BUN_PATH` |
| `AGENA_STUDIO_TERMINAL_IDLE_TIMEOUT_SECS` | `AGENA_SERVER_TERMINAL_IDLE_TIMEOUT_SECS` |
| `AGENA_STUDIO_GIT_*` | `AGENA_GIT_*` |
| `agena_studio.*` tracing target | `agena.server.*` / `agena.git.*` |
| `agena-studio:fs-changed` | `agena:fs-changed` |
| `agena-studio.auth-required` | `agena.auth-required` |
| `/api/agena-studio/diagnostics` | `/api/agena/diagnostics` |
| response `service: agena-studio` | `service: agena` |
| tmux prefix `agena-studio-` | `agena-` |
| localStorage `agena-studio` | `agena-web` |

不保留旧 executable、旧 route 或旧 env 的永久 alias。若必须保护本地持久化数据，只允许第 11 节中的一次性 legacy data migration module 包含旧字符串。

通用 plugin UI schema 中名为 `studio` 的字段和 TUI 中 `provider_studio`/`permission_studio` 等功能域，不自动属于本轮产品 identity rename。禁止用无边界替换把协议字段或用户可见 feature 名意外改坏。静态禁用规则只针对：

```text
agena-studio
agena_studio
AGENA_STUDIO
agena-studio-server
agena_studio_server
```

如需未来统一 plugin “studio” 术语，另立协议迁移，不混入本轮 binary 合并。

## 6. Server 移动与拆分 owner map

所有移动先完成，随后一次性修 module path。不要移动一个文件就运行编译。

| 当前路径 | 目标路径/owner | 操作 |
| --- | --- | --- |
| `src/main.rs` 中 Args | `agena-cli::ServerArgs` | 机械抽取字段/Clap 属性；进程 main 删除 |
| `src/main.rs` 中 issue token | `server/auth/token.rs` | 移动函数，不复制 |
| `src/app.rs` state | `server/state.rs` | `AppState` -> `ServerState` |
| `src/app.rs` bootstrap | `server/bootstrap.rs` | runtime/application/listener lifecycle |
| `src/app.rs` router | `server/router.rs` | 顶层 route merge 与 layers |
| `src/app.rs` CORS | `server/cors.rs` | origin normalize + layer |
| `src/app.rs` diagnostics | `server/diagnostics.rs` | health/diagnostics only |
| `attachment_cache.rs` | `server/attachment/{mod,cache}.rs` | manager + persistence helper |
| `config/**` | `server/config/**` | 目录级 `git mv`，再改 root paths |
| `error.rs` | `server/error.rs` | `AppError` -> `ServerError` |
| `fs.rs` + `fs/**` | `server/fs/**` | 目录级移动；见 8.1 再拆大文件 |
| `path_utils.rs` | 优先复用 `agena_git_http`/共享 path API；剩余进 `server/path.rs` | 去重后移动 |
| `persistence_paths.rs` | `server/persistence/paths.rs` | canonical server paths + legacy migration |
| `providers.rs` | `server/provider_routes.rs` | HTTP adapter |
| `settings.rs` | `server/settings/store.rs` | settings store adapter |
| `settings_events.rs` | `server/settings/events.rs` | SSE hub |
| `studio_db.rs` | `server/persistence/db.rs` | `StudioDb` -> `ServerStateDb` |
| `terminal.rs` | `server/terminal/**` | 必须拆 manager/session/routes/store |
| `terminal_ui_state.rs` | `server/terminal/ui_state/**` | model/store/routes/events |
| `ui_auth.rs` | `server/auth/**` | model/store/middleware/routes |
| `workspace_preview.rs` | `server/preview/**` | proxy/http/ws/discovery/routes |
| `workspace_preview_registry.rs` | `server/preview/registry/**` | model/store/cache/service |
| `workspace_preview_runtime.rs` | `server/preview/runtime.rs` | process lifecycle |

Git HTTP handler 继续由重命名后的 `agena-git-http` library 拥有。`ServerState` 实现其窄 `GitHttpState` port，`agena-git-http` 不得知道 `apps/agena` 的 concrete state。

## 7. `apps/agena` 自身的快速拆分

### 7.1 删除同 package `agena_app` lib target

当前 `apps/agena/src/lib.rs` 只被同 package `main.rs` 使用。执行：

```text
apps/agena/src/lib.rs
  -> apps/agena/src/launch/tui.rs
```

移动后：

- 删除 `[lib] name = "agena_app"`；
- `agena_app::TuiLaunchArgs` 等改成 `crate::launch::tui::*`；
- 对外不建立 `agena_app` compatibility library；
- TUI feature implementation 继续位于 `agena-tui-*` crates，binary module 只做 process adapter。

### 7.2 拆现有 `main.rs`

当前 `main.rs` 约 427 行，混合 launch dispatch 与 stdio RPC backend。不得手工重写。

执行方法：

1. 先将整个旧 `main.rs` 真实移动为一个临时受控源，如 `launch/rpc_server/legacy_entry.rs`；
2. 用函数/item 锚点一次抽出 `AgenaAppServerBackend` impl 和 mapping helpers；
3. 将 `run_tui`、`run_command`、`run_app_server` 分别移动到目标 launch module；
4. 新建不足 120 行的 `main.rs`；
5. 静态确认临时 `legacy_entry.rs` 为空后删除；
6. 通过 `git diff --find-renames` 确认代码是移动/拆分，不是重新输入。

### 7.3 Process lifecycle

唯一 `main`：

- 只调用一次 `agena_runtime::build_app_runtime()`；
- 只解析一次 `AgenaCli`；
- 每个 launch mode 只初始化一次 tracing subscriber；
- TUI restore、RPC shutdown、HTTP graceful shutdown 各自保留 mode-specific cleanup；
- server 启动失败、listener 退出或 Ctrl-C 时必须调用 runtime shutdown；
- 不允许 server 内部再创建第二 Tokio runtime；
- 不允许 CLI library 安装全局 subscriber。

## 8. 巨型 `.rs` 与 module tree 拆分清单

本节所有拆分都在同一 source train 中完成，完成前不编译。

### 8.1 合并后的 HTTP server

| 原文件/模块 | 目标拆分 |
| --- | --- |
| `terminal.rs` 1,347 行 | `terminal/model.rs`、`terminal/manager.rs`、`terminal/session.rs`、`terminal/store.rs`、`terminal/routes.rs` |
| `fs/fs_core.rs` 1,014 行 | `fs/read.rs`、`fs/write.rs`、`fs/mutate.rs`、`fs/list.rs`、`fs/path_guard.rs` |
| `workspace_preview.rs` 930 行 | `preview/routes.rs`、`preview/proxy_http.rs`、`preview/proxy_ws.rs`、`preview/rewrite.rs`、`preview/discovery.rs` |
| `workspace_preview_registry.rs` 847 行 | `preview/registry/model.rs`、`store.rs`、`cache.rs`、`service.rs` |
| `app.rs` 835 行 | 第 6 节的 state/bootstrap/router/cors/diagnostics |
| `ui_auth.rs` 719 行 | `auth/model.rs`、`session.rs`、`middleware.rs`、`routes.rs` |
| `fs/fs_content.rs` 719 行 | `fs/content/search.rs`、`replace.rs`、`text.rs` |
| `terminal_ui_state.rs` 472 行 | `terminal/ui_state/{model,store,events,routes}.rs` |

server module 目标：

- `main.rs` <= 120 行；
- server root `mod.rs` <= 200 行；
- router 文件只注册 route/layer，不实现 handler；
- 生产 server 单文件原则上 <= 800 行；
- HTTP request/response DTO 与 handler 可同 vertical slice，但 persistence/process implementation 不塞进 router。

### 8.2 Workspace 最大单文件

| 当前文件 | 当前行数 | 目标拆分 |
| --- | ---: | --- |
| `agena-tui-transcript/.../transcript_ast.rs` | 4,067 | `ast/types.rs`、`parse/{math,html,footnote,block,inline}.rs`、`render/{block,inline,list,table,image}.rs`、独立 tests |
| `agena-provider/src/lib.rs` | 3,054 | `contract/{catalog,completion,config,capability,native_tool,auth}.rs`；root 只显式 re-export |
| `agena-api/src/resource.rs` | 2,455 | `resource/{runtime,plugin,catalog,session,message,permission,provider,auth}.rs` |
| `agena-runtime/src/runtime/builder.rs` | 2,401 | `runtime/services/{bootstrap,auth,catalog,configuration,control,status}.rs` |
| `agena-runtime-session/.../manager/mod.rs` | 2,315 | `manager/{state,guards,api}.rs` + `manager/services/{execution,plugin,session}.rs` |
| `agena-tui-components/src/search_picker.rs` | 2,236 | `search_picker/{model,state,filter,input,render}.rs` + tests |
| `agena-tui-app/src/app_tests.rs` | 2,225 | 按 session/provider/permission/settings/composer/navigation 拆 tests |
| `agena-runtime-session/.../history/store.rs` | 2,217 | `history/store/{read,append,rewrite,rewind,projection}.rs` + tests |
| `agena-tui-media/src/lib.rs` | 2,161 | media model/cache/render/protocol/test_support 模块化，root 只 export |
| `.../replies/replies_execution.rs` | 2,049 | `execution/{request,completion,tools,finalize}.rs` |

拆文件时必须移动完整 item，不改行为。若单个 impl 太大，允许把同一类型的 `impl` 分散到多个 sibling module；禁止为了保持一个 impl block 而继续保留 2k 文件。

### 8.3 次级 1,200–2,000 行文件

同一列车继续处理：

- `prompt_tool_transport.rs`：transport model、request projection、stream state、tool result mapping；
- `registry/completion.rs`：resolution、request、stream、fallback；
- Bedrock adapter：wire/signing/stream/runtime；
- `transcript_state.rs`：neutral state/reducer 移入 `agena-tui-transcript`，App adapter 留在 app；
- `plugin_host_core.rs`/`host_handle.rs`：host lifecycle、registry、invocation、handle API；
- `runtime-config/config/raw.rs`：raw root 与 provider/runtime/plugin sections；
- `session/model.rs`/`prompt_window.rs`：neutral model 与 execution behavior 分离；
- `runtime-plugins` workflow plan/runtime/settings/schema lab；
- `agena-cli/src/cli/mod.rs`：arguments 按 command family 分文件，root 只声明/re-export。

### 8.4 文件大小验收

本轮完成后：

- 不得存在 > 2,000 行的生产 `.rs` 文件；
- > 1,200 行文件必须有明确例外说明；
- generated code、compile-test fixture 或集中式 schema 可以例外，但必须在计划执行记录中列出；
- 不能通过删除测试、压缩换行或合并声明来“达标”。

## 9. 大 crate 的真实拆分

### 9.1 `agena-runtime-session` -> execution + core

新增 `agena-runtime-session-core`，目标拥有：

- session value/model；
- history/event projection；
- DB entities/crud；
- session store/history store；
- prompt window/cache/cost 等不依赖 execution manager 的 neutral state；
- 与 persistence 直接相关的测试。

保留在 `agena-runtime-session`：

- `SessionManager`；
- processor/reply/run/compact execution；
- plugin/provider/tool orchestration；
- execution/query/maintenance service adapters；
- task control 与 runtime composition-facing API。

方向：

```text
agena-runtime-session -> agena-runtime-session-core
agena-runtime-session-core -X-> agena-runtime-session
```

移动 gate：任何准备进入 core 的文件若直接引用 manager、plugin runtime、provider adapter 或 tool executor，先把引用改为 contracts/port，不能把 execution dependency 一起复制进 core。

目标趋势：

- execution crate <= 18k 行；
- core crate <= 14k 行；
- `session::manager` 不再是一棵 10k 单树；
- 两个 crate 均无反向依赖和 facade cycle。

### 9.2 `agena-runtime-provider` -> shared runtime + adapters

新增 `agena-runtime-provider-adapters`，移动 vendor implementation：

- OpenAI tree；
- Amazon Bedrock runtime adapter；
- Gemini adapter；
- Anthropic adapter；
- GitLab/Ollama 等 vendor-specific transport；
- vendor adapter registration/factory table。

`agena-runtime-provider` 保留：

- shared registry；
- shared auth/credential/config support；
- core model runtime traits；
- shared wire/projection/transport helpers；
- provider selection/catalog decoration；
- adapter factory/registration port。

方向：

```text
agena-runtime-provider-adapters -> agena-runtime-provider
agena-runtime -> agena-runtime-provider + agena-runtime-provider-adapters
agena-runtime-provider -X-> agena-runtime-provider-adapters
```

将 builtin adapter registration 从 shared registry 移到 parent runtime composition。不得通过 shared crate re-export 整个 adapters crate，否则拆分无效。

目标趋势：

- shared provider runtime <= 20k 行；
- adapters crate 每个 vendor 独立 module tree；
- vendor-specific dependencies/features 不进入纯 shared registry；
- OpenAI 6k 子树不再埋在 shared owner 内。

### 9.3 `agena-runtime-plugins` -> runtime core + bundled plugins

新增 `agena-bundled-plugins`，移动：

- `plugins/provided/**`；
- bundled plugin descriptors/factories；
- workflow/settings/schema-lab/MCP/LSP/shell/fs/tasks 等内置 plugin implementation；
- 与 bundled plugin 直接绑定的测试。

`agena-runtime-plugins` 保留：

- plugin runtime lifecycle；
- slot/shutdown/callback guard；
- source/storage/config abstraction；
- web/memory runtime glue 中真正属于 runtime core 的部分；
- bundled factory 所消费的窄 registration contract。

方向：

```text
agena-bundled-plugins -> agena-runtime-plugins/contracts/tools/sdk
agena-runtime -> agena-runtime-plugins + agena-bundled-plugins
agena-runtime-plugins -X-> agena-bundled-plugins
```

目标：把 `plugins::provided` 8,695 行和大部分 16 个第一方依赖从 plugin lifecycle core 中移出。

### 9.4 `agena-macros` -> proc-macro shell + macro core

新增普通 library `agena-macro-core`，移动所有不要求 `proc_macro::TokenStream` 的：

- syn parsing；
- validation；
- metadata/schema building；
- quote/proc_macro2 codegen support；
- helper tests。

`agena-macros` 只保留 proc-macro attributes/derive 入口、`proc_macro` 转换和少量 dispatch。

方向：

```text
agena-macros -> agena-macro-core
agena-macro-core -X-> agena-macros
```

目标：proc-macro target 从 15,845 行下降到薄入口；macro core 可作为普通 library 独立单测。

### 9.5 `agena-tui-app` 继续收口到已有 feature crates

本轮不为每个 `app_*` 文件再建微型 crate。使用已有 owner：

- transcript neutral state/reducer/render input -> `agena-tui-transcript`；
- plugin schema/model/presentation -> `agena-tui-plugin-workbench`；
- session controller/value projection -> `agena-tui-session`；
- provider form/model helper -> `agena-tui-provider-studio`；
- permission model/normalization/editor helper -> `agena-tui-permission-studio`；
- settings schema/choice/model helper -> `agena-tui-settings`。

留在 app 的只能是：

- `App` concrete state；
- route/overlay composition；
- backend/runtime/platform/persistence adapter；
- async task/message dispatch；
- process-independent但 App-specific 的生命周期 glue。

任何移动 API 禁止接收 `&mut App`、`Backend` 或 root `AppMessage` 巨型 envelope。必须形成 feature state/action/effect/snapshot 或窄参数。

目标趋势：`agena-tui-app` 下降到约 30k–32k 行；如果仍超过该值，必须用实际 `impl App`/adapter 证据解释，不能仅声明“application shell”。

## 10. 快速机械化实施方法

### 10.1 先冻结精确文件集和命中数

开始前保存/记录：

```text
rg --files apps/agena apps/agena-studio-server crates/agena-studio-git
rg --files packages/agena-studio-web scripts/agena-studio
rg -n 'agena-studio|agena_studio|AGENA_STUDIO|studio-server'
python3 scripts/rust-architecture-report.py --output <temporary-report>
```

每个 bulk rename 都必须记录：

- 输入文件白名单；
- 旧字符串预期命中数；
- 新字符串执行前预期命中数；
- 替换后旧字符串剩余数；
- 允许保留的 legacy migration 文件。

若命中数不符，转换脚本立即失败，不做部分写入。

### 10.2 目录/文件优先 `git mv`

适合整棵移动的直接移动：

- Web package；
- scripts 目录；
- Git HTTP crate；
- server 的 `config/` 和 `fs/`；
- Rust 大模块拆成目录时，先 `git mv file.rs module/mod.rs`，再抽 sibling files；
- tests 与 owner 一起移动。

不要先创建目标副本再删除源。

### 10.3 大文件拆分使用 item 边界，不重输代码

对每个大文件：

1. 用架构报告骨架、`rg` 和 rust-analyzer/ast-grep（若可用）列出 top-level item；
2. 按 item 连续范围决定目标 module；
3. 先把原文件 `git mv` 成目标目录的 `mod.rs`；
4. 用一次 `apply_patch` 或带锚点的 extraction script 移动完整 item；
5. 自动/机械生成 `mod` 与必要的显式 `pub(crate) use`；
6. 通过 `rg` 确认 item 名只存在一次；
7. 所有文件完成后统一 rustfmt。

禁止复制函数体后手工删除旧版本。

### 10.4 路径批量改写

优先按以下批次，每批只改固定文件集：

1. Cargo package/dependency key；
2. Rust crate root (`agena_studio_git` -> `agena_git_http`)；
3. `crate::` server module paths；
4. CLI type/LaunchMode 名；
5. env/tracing/event/API canonical identifier；
6. Web package/storage/auth event；
7. scripts/workflow/docs。

每批改完运行 `rg` 与 `git diff --check`，但不编译。

### 10.5 可审阅的一次性 refactor script

当改写超过约 20 个文件时，允许创建一个临时、可审阅、带断言的 Python refactor script。脚本必须：

- 使用显式相对路径列表；
- 每个 replacement 声明 expected count；
- 写入前验证所有输入；
- 任何 mismatch 时零写入退出；
- 不遍历 `.git`、`target` 或 workspace 外路径；
- 执行后由 `git diff` 审阅；
- 若只服务本次迁移，在 source train 关闭前删除；若保留，则补测试和说明。

简单的单文件/manifest 修改继续使用 `apply_patch`，不为了“自动化”制造不必要脚本。

## 11. 数据与兼容迁移

### 11.1 产品/CLI 是硬切换

不保留：

- `agena-studio` executable wrapper；
- `agena-studio-server` Cargo alias package；
- 旧 HTTP route 永久 alias；
- 旧 release tag/service 的并行发布；
- 旧 env 作为长期 fallback。

文档必须清楚说明新命令 `agena server`。

### 11.2 持久化数据不能静默丢失

允许唯一的 legacy 范围：

```text
apps/agena/src/server/persistence/legacy_studio.rs
packages/agena-web-ui/src/lib/persistence/legacyStudio.ts
```

它们只做一次性数据发现/迁移：

- 旧 `agena-studio.db` -> 新 server state DB；
- 旧 `.config/agena-studio`/`~/agena/studio` -> canonical `~/agena/server`；
- 旧 localStorage namespace -> `agena-web`；
- 旧 `studio_kv` 数据复制/表迁移到 canonical schema；
- 迁移成功后写入 version marker，后续不重复覆盖新数据；
- 新路径已有数据时以新路径为准，不自动覆盖；
- migration failure 返回明确错误，不静默创建空状态。

旧字符串只能出现在这两个 migration owner 和迁移测试中。生产 active path 的 `rg` 必须为零；历史背景通过 Git history 查询，不再保留多份过期重构计划。

### 11.3 HTTP/Web 一起切换

后端 route/event/auth/storage namespace 与 Web consumer 在同一 source train 修改，避免先发布一边导致协议不匹配。

JSON response shape、auth cookie security、CORS、terminal/fs/git/preview 行为保持不变；只改变产品命名和入口。任何业务行为变更另开任务。

## 12. 一条连续 Source Train

### Train 0：冻结与保护

1. 确认工作树状态并记录用户既有改动；不得覆盖或清理无关修改。
2. 生成当前 Cargo metadata、架构报告和 Studio identifier inventory。
3. 记录 Rust/Web/CI/scripts 目标文件清单和行数。
4. 确认所有新 crate/path 不存在，防止覆盖。
5. 记录当前 CLI help、HTTP routes、env、service/package contracts，作为最终 smoke 对照。

### Train 1：一次创建所有目标骨架

一次创建：

- `apps/agena/src/{launch,server}` module roots；
- `crates/agena-git-http` rename target；
- `crates/agena-runtime-session-core`；
- `crates/agena-runtime-provider-adapters`；
- `crates/agena-bundled-plugins`；
- `crates/agena-macro-core`；
- `packages/agena-web-ui` rename target；
- `scripts/agena` rename target。

同时一次修改 root workspace member/path dependency。新 manifest 先写真实最小依赖，不复制 parent 全量 dependency。

### Train 2：统一 CLI schema 与 launch intent

一次完成：

- `ServerArgs`/`ServerLaunchRequest`；
- `RpcServerArgs`/`RpcServerLaunchRequest` rename；
- `LaunchMode::{Server,RpcServer,Tui,Command}`；
- help/examples/env canonical names；
- 普通 command dispatch 的 unreachable arms。

此时不要求编译，只要求类型/命名设计完整。

### Train 3：移动唯一 binary 的 launch modules

连续完成：

- `apps/agena/src/lib.rs` -> `launch/tui.rs`；
- 旧 `main.rs` 按 item 拆到 launch/rpc_server/command；
- 创建薄 `main.rs`；
- 删除 `[lib] agena_app`；
- 保留唯一 runtime 与 tracing 生命周期。

### Train 4：移动 HTTP server tree

按第 6 节一次移动所有 28 个 Rust 文件，不在中间 check。移动后立即：

- 重建 server module tree；
- 批量修 `crate::` paths；
- `AppState`/`StudioDb` 等 active product symbols 改为 canonical server names；
- 合并 Args 到 `agena-cli`；
- 删除旧 server main/runtime；
- 删除旧 package manifest/member。

### Train 5：rename Git/Web/scripts/workflow 产品面

连续执行所有 `git mv`，然后按白名单机械改写 Cargo、Rust、Web、shell、PowerShell、workflow、README 和 configuration docs。

release/package/service 必须改为构建/运行 `agena server`，不能继续寻找 `agena-studio` binary。

### Train 6：拆 server 大文件

按 8.1 完成 router/state/bootstrap、terminal、fs、preview、auth、persistence 拆分。完成后 server 不存在 > 1,200 行文件。

### Train 7：拆四个大 crate

按顺序连续移动，期间不编译：

1. `agena-macro-core`；
2. `agena-runtime-session-core`；
3. `agena-runtime-provider-adapters`；
4. `agena-bundled-plugins`。

顺序原因：macro core 最叶子；session/provider/bundled 需要先有 contracts/port，再由 parent runtime 最后统一 composition。

每棵树移动后只做静态 `rg`：旧路径归零、新 crate 不反向依赖 parent、item 单一存在。

### Train 8：拆 workspace 巨型 `.rs`

按 8.2/8.3 批量完成。每个文件只移动 item，不改业务逻辑。测试随 owner 移动，但不运行。

### Train 9：TUI app pure owner 收口

用 `impl App`、backend/runtime/route/overlay 引用作为负面过滤器：

- 无这些引用的 state/reducer/mapper 移入现有 feature crates；
- 有 concrete adapter 引用的保留 app；
- 跨 crate API 一次性设计为窄 action/effect/snapshot；
- 同步移动对应测试。

### Train 10：manifest、feature 与 composition 收口

一次完成：

- 所有新旧 Cargo dependencies；
- workspace members；
- feature forwarding；
- parent runtime adapter registration；
- server dependency 吸收；
- 移除旧/unused dependencies；
- 更新 lockfile 一次；
- `cargo metadata --locked` 验证 package graph。

### Train 11：Web、CI、release、service 和 docs 收口

一次完成：

- Web package name、storage/auth events、backend endpoint；
- CI job/path/test package；
- unified release workflow；
- Unix/Windows build/install/uninstall scripts；
- README、configuration、架构/执行记录；
- Python 架构报告重生成；
- 确认仓库只保留本文件这一份重构执行计划，已完成历史通过 Git history 查询。

### Train 12：静态闭环

完成第 14 节全部 gate。只有所有 gate 通过，source train 才关闭，随后进入第一次格式化/编译。

## 13. Manifest 与 lockfile 规则

### 13.1 `apps/agena`

吸收旧 server 的真实 direct dependencies，但要审计：

- Git implementation-only dependency 留在 `agena-git-http`；
- macro/provider/session/plugin implementation dependency 留在对应 library；
- binary 只直接依赖 launch/composition/HTTP server 真正引用的 crate；
- 不为了省事把旧 server manifest 整段粘贴进 `apps/agena/Cargo.toml`。

### 13.2 新 crate 最小依赖

每个新 crate 通过移动后源码的 crate roots 生成 direct dependency 候选，再人工审查 features。禁止复制 parent 的完整 manifest。

### 13.3 Lockfile

所有 manifest 一次改完后：

1. 执行一次受控 metadata/update 让 Cargo.lock 删除 `agena-studio-server`、rename Git package并加入新 library packages；
2. 之后所有 metadata/check/test 均使用 `--locked`；
3. lockfile 中 `name = "agena-studio-server"` 和 `name = "agena-studio-git"` 必须归零。

## 14. 第一次编译前的静态收口 Gate

以下全部完成前不得运行 `cargo check`。

### 14.1 Product identity

- [ ] `apps/agena-studio-server` 不存在；
- [ ] Cargo metadata 不存在 `agena-studio-server` package；
- [ ] Cargo metadata 不存在 `agena-studio` target；
- [ ] `apps/agena` 只有一个 target：`agena` bin；
- [ ] Web/scripts/workflow 路径已 rename；
- [ ] active source 中旧 product identifiers 为 0；
- [ ] 旧字符串只存在于允许的 legacy migration/tests/history docs。

建议断言：

```text
rg -n 'agena-studio|agena_studio|AGENA_STUDIO|studio-server' \
  apps crates packages scripts .github README.md docs/configuration.md Cargo.toml
```

输出必须只包含预先批准的 legacy migration 项。

### 14.2 Target 与 dependency graph

- [ ] `apps/agena/Cargo.toml` 无 `[lib]`；
- [ ] 唯一产品 bin 为 `agena`；
- [ ] examples/e2e bin exemption 已列出；
- [ ] 新 library crate 都不依赖 `apps/agena`；
- [ ] provider/session/plugins/macro 方向符合第 4.2 节；
- [ ] 第一方 normal graph 无 cycle；
- [ ] `agena-runtime` 完成所有新 implementation owner composition。

### 14.3 Module/path

- [ ] 所有移动文件只存在于新 owner；
- [ ] 临时 `legacy_entry.rs` 等 extraction 文件已删除；
- [ ] 没有 `#[path = ...]` 指回旧目录；
- [ ] 没有 `include!`、symlink、hard link 或源码复制；
- [ ] `crate::`/crate-root 旧路径归零；
- [ ] Python module resolver 未解析项为 0；
- [ ] Lexer 告警为 0。

### 14.4 大文件与 owner

- [ ] 无 > 2,000 行生产 `.rs`；
- [ ] > 1,200 行例外有记录；
- [ ] `main.rs` <= 120 行；
- [ ] server router 不含业务 handler implementation；
- [ ] `agena-tui-app` pure owner 已迁移，adapter 没有伪搬家；
- [ ] 大 crate 行数/依赖趋势符合第 9 节。

### 14.5 Web/CI/release/docs

- [ ] Web package/CI job 使用 `agena-web-ui`；
- [ ] release workflow 构建 `agena --bin agena`；
- [ ] package archive 只包含 `agena` executable；
- [ ] service ExecStart 使用 `agena server`；
- [ ] README/configuration 不再指导运行旧 binary；
- [ ] frontend/backed route/event identifier 同步；
- [ ] dependency scripts 使用新 Web 路径。

### 14.6 Patch hygiene

- [ ] `git status --short` 仅包含计划内文件；
- [ ] `git diff --check` 通过；
- [ ] `git diff --find-renames --summary` 显示大部分源码为 rename/move；
- [ ] 没有无关格式化或 lockfile 噪声；
- [ ] 没有覆盖用户既有改动。

## 15. Source Train 关闭后的统一验证

只有第 14 节完成后才运行。

### 15.1 格式和 metadata

```bash
cargo fmt --all
cargo fmt --all -- --check
cargo metadata --format-version 1 --locked
python3 scripts/rust-architecture-report.py --output docs/rust-workspace-analysis.md
git diff --check
```

### 15.2 第一次编译

第一次直接收集完整 workspace 错误集，不按 crate 一边移动一边试：

```bash
cargo check --workspace --all-targets --locked
```

保存完整错误输出，按第 16 节分组。第一次 check 后禁止新增 owner、移动整棵目录或恢复 compatibility facade；只允许修静态收口遗漏。如果编译揭示依赖方向根本错误，明确退出 stabilization、重新打开 source train，完成新一轮静态收口后再 check。

### 15.3 Tests

所有 check 错误清零后再运行：

```bash
cargo test -p agena-macro-core --all-targets --locked
cargo test -p agena-macros --all-targets --locked
cargo test -p agena-runtime-session-core --all-targets --locked
cargo test -p agena-runtime-session --all-targets --locked
cargo test -p agena-runtime-provider --all-targets --locked
cargo test -p agena-runtime-provider-adapters --all-targets --locked
cargo test -p agena-runtime-plugins --all-targets --locked
cargo test -p agena-bundled-plugins --all-targets --locked
cargo test -p agena-git-http --all-targets --locked
cargo test -p agena --all-targets --locked
cargo test --workspace --all-targets --locked
```

如 macOS linker 资源限制要求串行或特定 flags，记录后串行执行，不能省略 workspace coverage。

### 15.4 Clippy 与依赖验证

```bash
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo deny check
cargo machete
```

### 15.5 Web UI

```bash
bun install --cwd packages/agena-web-ui --frozen-lockfile
bun run --cwd packages/agena-web-ui check:imports
bun run --cwd packages/agena-web-ui typecheck
bun test --cwd packages/agena-web-ui
bun run --cwd packages/agena-web-ui build
```

若 Bun 的 test CLI 需要项目脚本，使用 package 已定义的等价命令，不跳过已有 `*.test.ts`。

### 15.6 Binary/CLI/server smoke

必须覆盖：

1. `agena --help` 只展示一个产品；
2. `agena` 在 PTY 中启动并退出 TUI；
3. `agena tui` 显式模式；
4. 一条普通 CLI 命令，例如 `agena config resolve`；
5. `agena rpc-server` stdio 协议最小 create/list/request；
6. `agena server --host 127.0.0.1 --port <ephemeral>` 启动；
7. HTTP health/native API；
8. Web static assets/fallback；
9. UI auth/CORS/cookie；
10. fs read/write/search；
11. Git status/diff/branch/commit 中至少一条只读和一条临时仓库写操作；
12. terminal create/input/resize/stop；
13. preview HTTP/WebSocket proxy；
14. Ctrl-C graceful shutdown；
15. legacy data migration 与新数据优先级。

所有 smoke 使用临时 workspace、临时 DB/data dir 和 ephemeral port，不污染用户真实配置。

### 15.7 Release/package smoke

- Unix package script生成只含 `bin/agena` 的 archive；
- Windows package script路径和 `.exe` 名正确；
- archive 中 Web dist 可由 `agena server --ui-dir` 服务；
- systemd/launchd/Windows service 命令均为 `agena server`；
- release artifact/tag/name 不包含 Studio product identity。

## 16. 第一次 Check 后的批量错误修复策略

严格按类别处理，不按单条错误随机修改。

### 16.1 Package/module/path

一次修完：

- workspace member/dependency key；
- crate root rename；
- module visibility/path；
- test target path；
- cfg/feature forwarding。

### 16.2 类型与 public API

一次修完：

- moved type import；
- narrow port/trait；
- explicit re-export；
- error conversion；
- async Send/Sync/lifetime。

禁止用 root glob re-export 恢复旧 namespace。

### 16.3 Composition

一次修完：

- provider adapter registration；
- bundled plugin factory registration；
- session core store injection；
- server state/runtime shutdown；
- tracing initialization。

### 16.4 Tests/features/platform

一次修完：

- moved unit/integration tests；
- dev-dependencies；
- macOS/Linux/Windows cfg；
- service/package paths；
- Web/backend schema consumers。

每组修完可以重跑同一 workspace check 确认该类别清空；不重新开始源码移动列车。全部 check 通过后才进入 tests。

## 17. 最终验收标准

### 17.1 产品结构

- [ ] 只有 `agena` 产品 executable；
- [ ] TUI、CLI、RPC server、HTTP server 均由 `agena` launch mode 启动；
- [ ] `agena-studio`/`agena-studio-server` 不再是 active product/package/target/release/service；
- [ ] `apps/agena` 无 library target，只有一个 bin target；
- [ ] examples/e2e binary 明确为测试例外。

### 17.2 架构

- [ ] 单一 binary 没有变成单一巨型 `main.rs`；
- [ ] server composition 在 bin，Git/runtime/session/provider/plugin implementation 在独立 library owner；
- [ ] 新 crate 依赖图无环；
- [ ] parent runtime 组合 implementation，不全量 re-export implementation；
- [ ] TUI feature crates 不反向依赖 app；
- [ ] server/Git library 不反向依赖 `apps/agena`。

### 17.3 量化趋势

| 项目 | 目标 |
| --- | --- |
| `apps/agena/src/main.rs` | <= 120 行 |
| 产品 binary targets | 1 (`agena`) |
| `agena-studio-server` package/target | 0 |
| unresolved Rust modules / lexer warnings | 0 / 0 |
| first-party normal cycles | 0 |
| >2,000 行生产 `.rs` | 0 |
| >1,200 行生产 `.rs` | 仅有书面例外 |
| `agena-runtime-session` | execution/core 分离；各自约 <=18k/14k |
| `agena-runtime-provider` | shared/adapters 分离；shared <=20k 趋势 |
| `agena-runtime-plugins` | provided/bundled 8.7k tree 已移出 core |
| `agena-macros` proc-macro target | 薄入口，support 位于 macro core |
| `agena-tui-app` | 约 30k–32k 或有 concrete adapter 证据解释 |

这些数字用于发现伪拆分，不能通过删测试、压缩格式或复制到非 Rust 文件来达标。

### 17.4 行为与质量

- [ ] CLI/TUI/RPC/HTTP 行为通过 smoke；
- [ ] HTTP route/auth/CORS/fs/git/terminal/preview 行为保持；
- [ ] legacy persisted data 安全迁移；
- [ ] Rust workspace check/test/clippy 通过；
- [ ] Web imports/typecheck/test/build 通过；
- [ ] release/service/package smoke 通过；
- [ ] 架构报告已重生成并审阅；
- [ ] `git diff --check` 通过；
- [ ] 工作树没有计划外修改。

## 18. 安全、回滚与共享工作树规则

- 不运行 `git reset --hard`、`git checkout --` 或广目录删除；
- 每次目录移动前确认目标不存在；
- 删除旧 package 只在所有文件已被 Git 识别为 rename 且静态引用归零后执行；
- 用户既有修改与本计划重叠时先保留并绕开，无法安全合并时暂停；
- 可以使用逻辑 checkpoint/diff snapshot，但未经明确要求不自动提交或推送；
- 任何临时脚本、inventory 或生成报告使用明确临时目录，不覆盖仓库外数据；
- 数据迁移 smoke 只使用临时路径，不触碰真实 `~/agena`。

## 19. 推荐的实际执行顺序摘要

1. 冻结 Git/Cargo/Rust/Web/CI/release/product identifier 基线。
2. 一次创建/rename 所有目标 crate、module、Web、script 和 workflow 路径。
3. 一次设计并写完 `ServerArgs`、`RpcServerArgs` 和四种 `LaunchMode`。
4. 将 `agena_app` lib 和旧 `main.rs` 真实移动/拆成薄 binary launch modules。
5. 将 28 个 HTTP server Rust 文件连续移动进 `apps/agena/src/server/**`。
6. 删除旧 server package/binary，rename Git HTTP crate。
7. rename Web package、scripts、release workflow、service/package identity。
8. 批量改 canonical env/tracing/event/API/storage identifiers。
9. 拆 server 的 app/terminal/fs/preview/auth/persistence 大文件。
10. 连续抽出 macro core、session core、provider adapters、bundled plugins 四个 library owner。
11. 拆 workspace 所有 >2k 及重点 >1.2k `.rs` 文件。
12. 将 TUI app 的 neutral owner 迁入已有 feature crates，保留 concrete adapter。
13. 一次收口 Cargo manifests、features、runtime composition 和 lockfile。
14. 一次收口 Web/CI/release/service/README/configuration/architecture docs。
15. 完成所有静态 gate；确认旧 product identity、旧路径、反向依赖和临时 extraction 文件归零。
16. Source train 关闭后第一次统一 fmt/metadata/report/check。
17. 按错误类别批量修复，check 清零后统一 test、Clippy、Web build、smoke 和 release/package 验证。

本计划的完成标准不是“把 `agena-studio` 改了一个名字”，而是形成一个真正的 `agena` 产品入口：用户只安装和运行一个 executable；内部仍由清晰、无环、可独立测试的 library crate/module 提供 TUI、CLI、RPC、HTTP、Git、provider、session 和 plugin 能力；迁移过程依靠大粒度移动和机械化改写快速完成，并且直到所有源码所有权静态闭环后才开始编译和测试。
