# Shell environment fidelity

Agena's `shell.exec`, `shell.spawn`, `shell.watch` and `shell.open` run model-authored command
strings, so those strings should behave the way
they do in the user's own terminal. Two things decide that: which shell program
runs them, and which startup state is loaded before they do.

## Which shell runs a command

`agena_process::shell` resolves one program per launch, and `agena-tool`'s
`ShellLaunchSpec` turns it into argv. The resolution order is:

1. `AGENA_SHELL` (path or bare name on `PATH`) - a supported dialect only.
2. The login shell from the password database on Unix (`getpwuid_r`; never the
   non-reentrant `getpwuid`), or `COMSPEC` on Windows.
3. A fallback: `sh`, then `bash`, then `zsh`.

Anything Agena cannot run POSIX command strings with - `fish`, `tcsh`, `nu`, ... -
is **reported and replaced**, never executed with a POSIX command string. A
missing or unusable candidate never aborts the command; the launch records why
in tool metadata instead (`shell`, `shell_program`, `shell_source`,
`login_shell`, `shell_snapshot`). Names come from the dialects Agena supports:
`bash`, `zsh`, `sh`, `powershell`, `cmd`.

The `shell` input selects the existing `bash`/`powershell` command dialect, not
an arbitrary executable. Program resolution belongs to trusted process
configuration, matching the `AGENA_SHELL_SANDBOX` surface: model input cannot
change it, and `ProcessShell`'s `bash` value still means "the POSIX dialect the
user's shell speaks".

| Variable | Values | Default | Effect |
|---|---|---|---|
| `AGENA_SHELL` | path or bare name | unset | Pins the shell program; unsupported or missing values are ignored with a diagnostic |
| `AGENA_SHELL_LOGIN` | `1/true/yes/on`, `0/false/no/off` | `1` | Login startup (`-lc`) versus plain (`-c`) |
| `AGENA_SHELL_SNAPSHOT` | `disabled`, `auto`, `required` | `auto` | Interactive startup capture policy |

Invalid values are fail-open for these switches and reported in metadata,
because none of them is a protection. `required` is the exception: a capture that
fails refuses the launch rather than silently running with less state.

To keep the historical behavior exactly, set
`AGENA_SHELL_SNAPSHOT=disabled AGENA_SHELL=/bin/sh`.

## Interactive startup capture

A non-interactive login shell never reads the interactive startup file, so
`-lc` alone still misses what shapes the user's terminal: `PATH` additions from
`~/.zshrc` or `~/.bashrc`, aliases, and shell options. Agena captures that state
once per process and replays it as data:

- `zsh` sources `$ZDOTDIR/.zshrc` or `~/.zshrc`.
- `bash` sources `~/.bashrc`, and only when `BASH_ENV` is unset.
- `sh` sources `$ENV` when the non-evaluating whitelist expander below accepts
  it.

The capture runs the shell with stdin closed, a 5-second deadline, and a 256 KiB
per-stream bound. Environment entries are replayed through the child
environment; alias and option declarations are replayed as source prepended to
the command. Function declarations are **not** replayed: they need their own
literal decoder before prepended source can be trusted, and that is separate
work.

Environment precedence for a launch is
**inherited < snapshot < plugin `shell.env`/`command.before` overrides**, so a
snapshot supplies what the user's terminal supplies, while trusted runtime
patches still win. A snapshot never supplies `AGENA_*` names, linker variables
(`LD_*`, `DYLD_*`), startup injections (`BASH_ENV`, `ENV`, `BASH_FUNC_*`), or the
variables the shell owns itself (`PWD`, `OLDPWD`, `SHLVL`, `PS1`-`PS4`, `_`).

Alias and option records are replayed only when they match the shape the shell
itself emits for a declaration (`alias name='value'` with balanced quotes,
`setopt name`, `shopt -s|-u name`). Anything else is dropped and counted in
`shell_snapshot=used(env=N,dropped=M)` rather than guessed at.

### Credential handling

- The snapshot exists **in memory only**. It is never written to disk, so there
  is no secret at rest and no stale snapshot file to reason about.
- Captured values never appear in metadata, logs, or diagnostics - only counts
  and provenance.
- Captured environment entries pass through the same filter that already guards
  every shell subprocess, so a snapshot cannot re-introduce a startup-injection
  variable that the launch layer removes.
- POSIX `sh` startup files are located through the whitelist expander below,
  which interprets the path without evaluating it.

### `ENV` path expansion

An interactive POSIX `sh` finds its startup file through `ENV`, whose value is
ordinary shell text. Agena never evaluates that text. It accepts an absolute
path, `~`, `~/...`, `$NAME`, `${NAME}`, and `${NAME:-default}`, followed by
`/...` segments, and refuses command substitution, backticks, quotes, globs,
redirections, pipeline characters, whitespace-separated lists, `~user`, and
`..` traversal. A refused value simply means the file is not sourced.

`ENV` stays blocked in the child environment; only the capture script uses this
expansion.

## Windows input

ConPTY input is normalized before it is queued: a line feed becomes a carriage
return with CRLF collapsing across writes, and a `0x08` backspace becomes DEL.
The PTY driver still reports the caller's own input bytes as acknowledged, so a
partially delivered write cannot desynchronize a session. See
[interactive-terminals.md](interactive-terminals.md) for the terminal contract
itself.

## Deliberate divergences

Agena's version is smaller than Codex's equivalent on purpose:

| Codex | Agena | Why |
|---|---|---|
| Snapshot files under `$CODEX_HOME/shell_snapshots/` | In-memory cache with startup-file mtime invalidation | Agena has no shell credential broker; no secrets at rest |
| Credential virtualization with broker markers | Existing environment filter plus count-only metadata | Matches Agena's existing environment/permission model |
| Replays environment, aliases, functions, and options | Replays environment, aliases, and options | Function replay needs a literal decoder; tracked as separate work |
| Richer `ENV` expansion helper | Literal-only whitelist expander | Keeps the "never evaluate" posture |
| Model-supplied shell program/login parameters | Existing dialect selector plus trusted environment switches | Models cannot select an arbitrary shell executable or alter startup policy |

## Limits

| Resource | Limit |
|---|---|
| Capture deadline | 5 s |
| Capture output per stream | 256 KiB |
| Replayed environment entries | Every captured entry that passes the filter |
| Replayed alias/option declarations | Shape-checked; others dropped and counted |
| Persistence | None (process lifetime only) |

A capture failure is reported, not fatal: `auto` falls back to the login startup
and records `shell_snapshot=fallback: <reason>`, while `required` refuses the
launch.

## Validation

```sh
cargo test --locked -p agena-process
cargo test --locked -p agena-tool
cargo test --locked -p agena-runtime-tools --lib shell_tools::
cargo test --locked -p agena-runtime-tools --lib terminal::
cargo check --locked -p agena-process --target x86_64-pc-windows-msvc
```

The `agena-process` suite captures real startup state from `/bin/sh` and asserts
the whitelist refusals; the runtime suite asserts the resolution, login toggling,
and provenance projection. Windows input normalization is unit-tested on every
platform, and its wiring is compiled for the Windows target; a real ConPTY
session must still be exercised on a Windows host, where
`cargo test --locked -p agena-process` covers the normalizer and
`shell.write`-driven sessions cover the driver.
