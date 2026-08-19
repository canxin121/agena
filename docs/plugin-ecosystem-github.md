# GitHub-first plugin ecosystem

Agena plugin distribution uses GitHub as the public source, build, release, and
catalog transport. Runtime plugin contracts remain transport-neutral; this
layer defines how independent repositories publish and discover those runtime
contracts without adding a second plugin model.

## Repositories and ownership

There are three repository roles.

### Plugin repository

A plugin repository owns one stable plugin id (`namespace.name`) and its source.
The human-maintained release source is `agena-plugin.toml`. Cargo owns the Rust
package version (`version = "cargo"` keeps the two values from drifting).

A generated repository contains:

- typed SDK source using the same Settings / Operations / Services contracts as
  bundled plugins;
- `Cargo.lock` and a fixed Rust toolchain;
- CI and Release workflows calling Agena reusable workflows by immutable Agena
  commit SHA;
- an Agena SDK git dependency pinned to the same Agena commit;
- contribution, security, pull-request, and issue templates.

`agena plugin init` / `agena-plugin init` create this repository shape.

### Plugin GitHub Release

The reusable Release workflow builds all supported GitHub-hosted target
architectures, creates one archive and one release fragment per target, and
merges them into `agena-plugin-release.json`.

A public GitHub release manifest records:

- canonical GitHub source repository;
- release tag;
- exact 40-character source commit SHA (`GITHUB_SHA`);
- optional GitHub Actions run URL;
- immutable GitHub Release asset URLs;
- SHA-256 for every artifact;
- target, transport kind, archive entrypoint, settings defaults and plugin
  dependencies.

The manifest is validated before publishing. Direct installation from
`owner/repository[@tag]` enables the same strict GitHub provenance policy; a
manifest without immutable source provenance is rejected before it is cached or
installed.

Release publication never mutates a marketplace repository. Publishing and
catalog review are separate trust boundaries.

### Marketplace repository

A marketplace repository has three layers with one responsibility each:

1. `agena-marketplace.toml` -- human-maintained marketplace identity and explicit
   plugin-id rename graph;
2. `releases/<plugin-id>/<version>.json` -- reviewed immutable plugin release
   manifests copied from plugin repositories;
3. `agena-marketplace.json` -- deterministic generated search/install index.

`agena-plugin marketplace add` stores a new immutable release record. Repeating
identical content is a no-op; changing an existing plugin-id/version is rejected
and requires a new plugin version. `marketplace build` regenerates the index
from the project manifest and release records.

Public GitHub-only marketplaces additionally require every version to use a
canonical repository, matching tag/version, exact source SHA, and asset URL from
that repository's GitHub Release.

Marketplace review is an independent human signal stored in
`agena-marketplace.toml`. Each plugin can remain `community`, be marked
`verified` after marketplace review, or `official` when maintained by the
marketplace owner; entries can also be `featured`. These fields never replace
source provenance or digest verification: authenticity and curation are
separate trust axes.

```toml
[plugins."acme.example"]
review_tier = "verified"
featured = true
```

Review policy entries must resolve to an actual plugin release in the generated
catalog, so a typo cannot silently create a badge for a nonexistent slug.

The generated marketplace CI:

- rebuilds and byte-compares the generated index;
- rejects PRs that modify or delete an already-published release record;
- validates GitHub-only distribution rules;
- resolves every source tag through the GitHub API (including annotated tags)
  and checks it equals the recorded source commit;
- verifies the corresponding GitHub Release exists.

This mirrors the stable-slug / independently reviewed catalog boundary used by
mature agent plugin ecosystems while keeping Agena's binary distribution fully
reproducible.

## Installation and upgrades

The public installer accepts either a marketplace plugin id or a GitHub
repository locator. Discovery may use a mutable convenience pointer (the default
marketplace `main` branch or GitHub's `latest` release), but the selected plugin
version is always represented by immutable release provenance and artifact
digests before installation.

Installed records persist their trust policy:

- whether a trusted Ed25519 signature was required;
- whether strict GitHub provenance was required.

`upgrade` and `outdated` reuse that policy. They never silently downgrade a
previously strict install to an unverified registry lookup.

The Web Marketplace panel exposes marketplace identity, release source commit,
and installed trust-policy badges so the supply-chain state is observable rather
than hidden in cache metadata.

## Stable identity and version rules

- Plugin id is a long-lived slug, independent of repository name.
- A published plugin-id/version is immutable.
- Marketplace renames are explicit and acyclic; old ids are never silently
  reassigned.
- Plugin project, Cargo package, release tag and release manifest versions must
  agree.
- Public release/source version remains `0.1.0` until the project intentionally
  changes the workspace version; no parallel `v2` plugin protocol is introduced.
- Reusable workflow, SDK git dependency, developer tool and committed lockfile
  share one verified Agena baseline revision.

## Developer workflow

```bash
agena plugin init ./my-plugin \
  --id acme.my_plugin \
  --repository https://github.com/acme/agena-my-plugin

cd my-plugin
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
agena-plugin validate .
```

Tag the Cargo version (for example `v0.1.0`) to publish. Users can then install
directly:

```bash
agena plugin install acme/agena-my-plugin
agena plugin install acme/agena-my-plugin@v0.1.0
```

or through a marketplace after its release manifest is reviewed.

## Template repositories

Agena maintains independent GitHub template repositories for:

- Rust stdio plugins;
- Rust cdylib plugins;
- GitHub-first marketplace catalogs.

These templates are generated from the same scaffold code exercised by Agena's
own tests. They are not manually maintained forks of the contract. Updating the
baseline is an explicit operation: update the single Agena revision used by the
SDK, reusable workflows and developer tool, regenerate `Cargo.lock`, run the
standalone repository tests, then publish the template update.
