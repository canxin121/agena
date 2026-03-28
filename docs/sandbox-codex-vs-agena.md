# Codex Sandbox vs Agena Sandbox（阶段性详细对比）

> 目标：持续做行为级 parity 对照，并记录 Agena 的增强方向与本轮已落地修补。

## 1) 总体架构对照

| 维度 | Codex | Agena | 结论 |
|---|---|---|---|
| 策略模型 | `DangerFullAccess` / `ReadOnly` / `WorkspaceWrite` | 同三态，并额外有 `enforce_world_writable_audit`、`reject_reparse_points` | Agena 在 Windows 安全开关上更细粒度 |
| 可写根默认 | `cwd` + `/tmp` + `TMPDIR`（可排除） | `workspace_root` + `/tmp` + `TMPDIR` + Windows `TEMP/TMP`（可排除） | Agena 对 Windows 临时目录兼容更强 |
| Linux 沙盒 | Landlock + seccomp | Landlock + seccomp | 基本同路线 |
| macOS 沙盒 | `sandbox-exec` + Seatbelt policy | `sandbox-exec` + Seatbelt policy | 基本同路线 |
| Windows 沙盒 | 受限 token + ACL + env no-network（实验性） | 受限 token + ACL + 审计 + env no-network | Agena 增加了审计与重解析点限制 |

## 2) 平台细节对照

### Linux

- 两者都在无网络策略下安装 seccomp 过滤器，限制 `connect/bind/listen/send*` 等关键 syscall。
- 两者都使用 Landlock 对文件系统做“全局只读 + 可写根白名单 + `/dev/null` 可写”。
- Agena 额外在超时时尝试按进程组终止（`setpgid + kill(-pgid, SIGKILL)`），减少子进程泄露。

### macOS

- 两者都调用固定路径 `/usr/bin/sandbox-exec`，避免 PATH 注入。
- 两者都支持 `workspace-write` 下按可写根拼装写策略，并保留 `.git` 子路径只读语义。

### Windows

- 两者都通过受限 token + capability SID + ACL 控制可写范围。
- Agena 增加：
  - 启动前 world-writable 审计（可配置开关）。
  - 可拒绝 reparse point（防止路径跳转类绕过）。
- 无网络策略方面，Agena 与 Codex 都使用 env hardening + denybin 组合；Agena默认 denybin 工具集更宽（`ssh/scp/sftp/ftp/telnet/nc/ncat`）。

## 3) 本轮已落地修补（Agena）

### 3.1 环境变量过滤增强（跨平台）

- 文件：`src/sandbox/manager.rs`
- 变更：将 `DYLD_` / `LD_` / `BASH_FUNC_` 前缀过滤改为**ASCII 大小写不敏感**匹配。
- 目的：避免通过混合大小写变量名绕过注入类环境变量过滤。

### 3.2 Windows 无网络硬化增强

- 文件：`src/sandbox/platform/windows/env.rs`
- 关键增强：
  - 对代理与关键 Git 变量采用**大小写不敏感清理+重写**，消除 `Http_Proxy`、`NO_proxy` 等混合大小写绕过。
  - 新增并强制写入：
    - `GIT_ALLOW_PROTOCOL`（补齐正确变量名）
    - `GIT_ALLOW_PROTOCOLS`（兼容保留）
    - `GIT_PROTOCOL_FROM_USER=0`
    - `GIT_TERMINAL_PROMPT=0`
  - `PATH` / `PATHEXT` 处理改为大小写不敏感读取与归一化写回，降低环境块重复键造成的行为漂移。

### 3.3 fallback 适配提示修复

- 文件：`src/sandbox/platform/fallback.rs`
- 变更：错误提示从“仅 Windows 实现”修正为“当前仅 windows/linux/macos 有适配器”。
- 目的：避免误导排障。

## 4) 本轮新增测试点

- `src/sandbox/manager.rs`
  - 新增用例：验证前缀过滤大小写不敏感。
- `src/sandbox/platform/windows/env.rs`
  - 新增用例：
    - mixed-case 代理/Git 变量被覆盖并归一化。
    - `PATH` 前缀在大小写差异下不重复注入。
    - `PATHEXT` 重排将 `.BAT/.CMD` 前置。

## 5) 下一阶段建议

1. 补充 Linux/macOS 的沙盒集成测试（最小 smoke）：网络阻断、`.git` 只读、可写根生效。
2. 将 `SandboxPolicy` 暴露为可序列化配置输入（与 CLI/服务配置对齐），把增强开关（如审计、reparse）纳入可控项。
3. 增加 Windows ACL 变更审计日志（debug 级），便于诊断“为何此路径被拒绝”。
