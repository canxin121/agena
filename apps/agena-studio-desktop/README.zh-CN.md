## Desktop Packaging (Tauri)

[English](README.md) | 简体中文

该目录包含 `agena-studio` 的 Tauri 桌面封装。

当前包含两个变体：

- `src-tauri`：标准 WebView2/Wry 桌面版
- `src-tauri-cef`：实验性的 CEF 桌面版

桌面端不再依赖旧的代理流程，而是直接启动并连接内置的
`agena-studio` sidecar。

## 本地快速开始

```bash
bun install --cwd ../../packages/agena-studio-web
bun run --cwd ../../packages/agena-studio-web build
../../ops/agena-studio/desktop/prepare-sidecar.sh
cargo check --manifest-path src-tauri/Cargo.toml
```

如果要检查 CEF 变体：

```bash
../../ops/agena-studio/desktop/prepare-sidecar.sh --cef
cargo check --manifest-path src-tauri-cef/Cargo.toml
```
