# Skills

Agena supports [Agent Skills](https://agentskills.io/specification): a directory
containing `SKILL.md` and any supporting scripts, references, or assets. The
Skills settings pages, terminal workbench, REST API, and `agena.skills` plugin
use the same parser and package manager.

## Document format

A minimal package can be:

```text
review-evidence/
  SKILL.md
  references/
    checklist.md
  scripts/
    check.py
  assets/
    example.json
```

```markdown
---
name: review-evidence
description: Review source evidence and identify gaps. Use for research checks and fact verification.
license: MIT
compatibility: Requires Python 3 for scripts/check.py.
metadata:
  author: Example Team
  version: "1.0"
allowed-tools: Read Bash(python:*)
---

# Review evidence

Read references/checklist.md, inspect the supplied sources, and report findings
with their supporting evidence.
```

The parser supports complete YAML syntax, including quoted strings, folded and
literal blocks, nested metadata, UTF-8, and CRLF line endings. The original
document is preserved when read or edited; unknown extension keys are accepted.

| Field | Meaning |
| --- | --- |
| `name` | Required. 1–64 lowercase letters, numbers, or hyphens; no leading, trailing, or consecutive hyphens. Must match the installed directory name. |
| `description` | Required. 1–1024 characters explaining the skill and when to use it. |
| `license` | Optional license identifier or reference. |
| `compatibility` | Optional environment requirements, limited to 1–500 characters. |
| `metadata` | Optional map of string keys and string values. |
| `allowed-tools` | Optional experimental tool metadata. It does not grant permissions or replace Agena's tool policy. |
| `aliases` | Agena compatibility extension: an alternate name or list of names. |

New and edited managed documents are validated before saving. Invalid existing
packages remain visible in the library with diagnostics so they can be repaired.
Legacy command declarations retain their existing naming conventions.

`SKILL.md` should stay focused. More than 500 body lines produces a recommendation
to move detailed material into supporting files, rather than a hard validation
failure. Supporting files can use arbitrary directories; `scripts/`,
`references/`, and `assets/` are conventions.

## Scopes, discovery, and availability

| Source | Location | Management |
| --- | --- | --- |
| Workspace | `<workspace>/.agena/skills/<name>/` | Full create, read, edit, remove, and enable/disable support. |
| Global | `<Agena data directory>/skills/<name>/` | Full management; available across workspaces on the same Agena server. |
| Compatible workspace roots | `.agents/skills`, `.claude/skills`, `.codex/skills` | Discovered read-only; install an editable copy to manage it through Agena. |
| Compatible user roots | The same three directories under the user's home directory | Discovered read-only. |
| Additional configured roots | `agena.commands` settings `additional_roots` | Discovered read-only, relative to the selected workspace. |
| Loaded plugin declarations | Skills contributed by plugin manifests | Read-only instructions; create an editable document copy to customize them. |

The default Agena data directory is `$HOME/agena`, or `%USERPROFILE%/agena`.
`AGENA_DATA_DIR` selects a different data directory. The API returns the actual
root paths, so clients do not need to assume the default.

Workspace packages take precedence over global packages with the same name.
Within each scope, the native Agena location takes precedence over the
compatibility roots. Workspace additional roots precede the native workspace
location. Filesystem packages take precedence over plugin declarations.

Each copy has a stable, opaque ID. Management uses this ID rather than an
ambiguous name, and the library retains shadowed copies for inspection. Disabling
a managed copy keeps its files but removes that copy from discovery; an enabled
lower-priority copy can then become effective.

Invalid packages remain visible for repair but do not hide a valid package with
the same name.

Package availability and installation provenance are stored in
`.agena-skill.json` inside the managed directory. This is internal state, not
part of the Agent Skills document. Import ignores source copies of this state.
The legacy `agena.commands` disabled-name setting is respected when no explicit
package availability setting exists; an explicit managed-package setting wins.

Plain Markdown files under the older command roots remain available through the
command palette. The Skills attachment picker lists enabled, valid packages for
the selected workspace and omits shadowed copies. Selecting a skill attaches its
opaque ID and revision; its instructions are fetched on demand through the
`agena.skills` plugin's `get` tool. Older command references remain supported.

## Web settings

Open **Settings → Skills → Skill library**.

The library supports searching and filtering by scope or status. Select a skill
to inspect its source, validation results, complete `SKILL.md`, rendered Markdown,
and resource inventory. Managed skills can be created, edited, enabled, disabled,
or removed. Text resources can be read, created, and edited without reinstalling the rest of
the package. Individual resources, including binary assets, can be removed.

**Create editable copy** handles read-only entries:

- A filesystem package opens an installation preview for its complete directory,
  preserving supporting files.
- A plugin declaration opens an editable document copy. Choose a valid managed
  name; supporting assets must be supplied separately through a folder or archive.

Open **Install skills** for GitHub, archives, a server folder, or a browser folder
upload. Preview the source, select the desired valid candidates, explicitly allow
replacement if needed, and install into Global or Workspace. The UI passes the
preview's source revision, so a changed source must be previewed again.

Settings use the currently selected workspace, including when the server serves
several workspaces. Unsaved editors and in-flight operations retain their
workspace context.

## Terminal workbench

Open **Settings → Skills → Skill library**.

| Key | Action |
| --- | --- |
| `/` | Search names, descriptions, or paths. |
| `G` | Cycle All, Workspace, and Global filters. |
| `S` | Choose the target scope for creation, installation, and copies. |
| `↑` / `↓` | Select a skill or file. |
| `Tab` | Switch between the library and resource files. |
| `Enter` | Open the selection. |
| `E` | Edit the selected skill document or text resource. |
| `N` | Create a skill. |
| `F` | Create a text resource. |
| `Space` | Enable or disable a managed skill. |
| `D` / `Delete` | Confirm removal of a skill or resource. |
| `C` | Create an editable copy. |
| `I` | Open the installer. |
| `U` | Preview an update from the recorded source when available. |
| `R` | Refresh. |
| `F1` | Show help. |

The multiline editor uses **Ctrl+S** to save. **Esc** protects unsaved changes.
In an installation preview, **Space** selects a candidate, **A** toggles all valid
candidates, **O** allows replacement, and **Enter** confirms installation.

The installer accepts a GitHub URL, archive URL, server folder, or server archive
path. Prefix a path with `@` to upload a folder or archive from the machine running
the TUI, which can differ from the Agena server.

## Installation sources

### GitHub

Use an `owner/repository` shorthand, a repository URL, a tree URL, or a
`SKILL.md` blob URL. The source can specify a branch, tag, commit, and subdirectory.
A reference containing slashes should be supplied explicitly so it can be
distinguished from the repository path.

Direct GitHub installation downloads the public repository archive. For a
private repository, first obtain the files through your authenticated GitHub
workflow, then install the resulting folder or archive.

A repository can contain several skills. Preview identifies candidates by their
source-relative directory, validates each document, and marks conflicts in the
destination scope. Selecting one candidate installs that package and its
supporting files.

### Archives and folders

Supported archives are **ZIP, TAR, TAR.GZ, and TGZ**, from a server path, HTTP(S)
URL, or uploaded archive. Archives with a common repository wrapper directory are
normalized before subdirectory selection. Folders can be read from the server or
uploaded by a client.

Binary resources are copied unchanged. On Unix, files marked executable in
archive, folder, or upload metadata are installed with mode `0700`; the original
permission mode is not copied.

Browser folder uploads carry no Unix permission metadata, so their files are
non-executable. To preserve executable status, upload an archive that stores it
or import a folder from a Unix server or TUI client. Installation does not execute
scripts, install dependencies, or perform package-specific setup.

The importer excludes version-control directories and Agena's internal package
state. It preserves ordinary support directories, including `dist`, `target`,
and `node_modules`, subject to the source limits.

### Replacement and source updates

Existing names are never replaced by default. Set `replace: true` only after
reviewing conflicts. Replacing a package installs the selected source tree as a
whole and preserves the existing enabled/disabled setting.

Source information records the original location, reference, selected subpath,
and content revision. For a reusable source, reopen the installer from that
source and preview the update before replacing the installed package. Browser
uploads must be supplied again when their original source is unavailable.

All selected packages are prepared and validated before the visible library is
changed. A failed commit attempts to restore earlier packages; recovery backups
are retained if restoration itself fails.

## Agent tools

The dedicated plugin is **`agena.skills`**:

| Tool | Purpose |
| --- | --- |
| `list` | Search the catalog. Set `include_inactive: true` for disabled, invalid, or shadowed packages. |
| `get` | Read the complete document, metadata, source, revision, and resource inventory for an exact ID. |
| `create` | Create a validated package in a managed scope. |
| `update` | Edit `SKILL.md` using the current package revision. |
| `remove` | Remove a managed package and all its resources using the current revision. |
| `set_enabled` | Change the package's availability. |
| `read_resource` | Read one bounded UTF-8 resource. |
| `write_resource` | Create a resource or replace its current revision. |
| `remove_resource` | Remove one resource using its content revision or the current package revision. |
| `validate` | Check a document without changing files. |
| `preview_install` | Inspect installation candidates and conflicts. |
| `install` | Install selected complete packages. |

A typical workflow is to list relevant skills, read only the selected skill, and
load its supporting resources when needed. Merely discovering a package does not
inject its full instructions into every conversation.

For management, read the current revision before updating or deleting. Preview
installation sources and pass `expected_source_revision` when installing. Tool
calls use Agena's existing path and network permission checks, including
permissions for concrete resources and redirected download destinations.

The older `agena.commands` tools and composer references remain compatible. Use
`agena.skills` for complete package management.

## REST API

All paths below are under `/api/v1`. Supply `workspace_id` as a query parameter to
select the intended registered workspace; omitting it uses the application's
default workspace.

| Method | Path | Operation |
| --- | --- | --- |
| GET | `/skills` | Full management catalog. |
| POST | `/skills` | Create from `scope` and `document`. |
| GET | `/skills/{id}` | Read a skill and its resources. |
| PUT | `/skills/{id}` | Save `document` with `expected_revision`. |
| DELETE | `/skills/{id}` | Remove using query `expected_revision`. |
| PUT | `/skills/{id}/enabled` | Set `enabled` with `expected_revision`. |
| GET | `/skills/{id}/resource` | Read query `path` relative to the package. |
| PUT | `/skills/{id}/resource` | Write `path`, `content`, and optional `expected_revision`. |
| DELETE | `/skills/{id}/resource` | Remove query `path` with query `expected_revision`. |
| POST | `/skills/validate` | Validate `document` and optional directory `name`. |
| POST | `/skills/install/preview` | Inspect an installation source. |
| POST | `/skills/install` | Install selected candidates. |

A preview request can be:

```json
{
  "scope": "workspace",
  "source": {
    "kind": "github",
    "url": "example/skills",
    "reference": "main",
    "subpath": "skills"
  }
}
```

Send the same source to install, adding candidate paths from the preview:

```json
{
  "scope": "workspace",
  "source": {
    "kind": "github",
    "url": "example/skills",
    "reference": "main",
    "subpath": "skills"
  },
  "selected": ["review-evidence"],
  "replace": false,
  "expected_source_revision": "<revision returned by preview>"
}
```

An empty `selected` list means all valid candidates. Invalid candidates cannot be
explicitly selected. A stale source or editing revision produces a conflict.

For a server folder, use `{"kind":"folder","path":"/path/to/skills"}`. An archive
source uses `kind: "archive"` and exactly one of `path`, `url`, or
`data_base64`; uploaded archives also require `filename`. A folder upload uses
`kind: "files"` with entries containing `path`, `content_base64`, and optional
`executable`.

Omit a resource write's `expected_revision` only when creating a new file.
An empty revision also means create-only. Replacing an existing resource requires
the revision returned by `read_resource`, so a concurrent creation cannot be
silently overwritten.

Resource deletion also accepts the current package revision returned by `get`.
Use this for binary assets or files larger than the text-read limit. A stale
package revision is rejected, and the package directory boundary still applies.

## Limits and file boundaries

- `SKILL.md` and text resource reads/edits: **1 MiB** each.
- Server installation source or downloaded archive: **64 MiB**.
- Expanded package source: **128 MiB**.
- Source files: **10,000**, with bounded directory traversal.
- Web and TUI uploads: **32 MiB** and **2,000 files**; use a server source or
  GitHub for a larger package within server limits.
- Archives reject absolute paths, parent traversal, symbolic links, hard links,
  and special files. Managed resource paths stay within the package and cannot
  traverse symbolic links.
- Binary assets remain in the package and resource inventory; the text editor
  does not treat them as UTF-8 documents.
- Removing a skill removes its complete managed directory. Read-only external
  and plugin-provided copies remain unchanged.
