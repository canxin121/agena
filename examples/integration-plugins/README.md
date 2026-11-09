# Reference integration plugins

These are external Python **stdio plugins** connected to Agena's integration
dispatcher. They use only the Python standard library and the SDK-generated
`agena.integration` service contract.

| Plugin | Export | Purpose |
|---|---|---|
| `example.github_issues` | `describe`, `poll` | Read new GitHub repository issues, excluding PRs; conditional requests, bounded pagination and rate-limit backoff. |
| `example.webhook_chat` | `describe`, `poll`, `send` | Verify signed normalized chat messages, persist them, and return real replies to a fixed HTTPS gateway. |

Install/configuration, grant examples, signing rules and limitations are in
[`docs/integrations.md`](../../docs/integrations.md). Use absolute paths in
`package.args`; Python imports `protocol.py` and reads `manifests.json` alongside
the plugin, so keep this directory together.

The gateway must authenticate the original platform user before signing a
normalized `actor`. Do not hand an external chat app the Agena administrator
token. Place credentials in the referenced service environment, not a model
prompt or an integration rule.

Installing a source does **not** authorize work. Save a proposal and explicitly
enable its finite grant in Web/TUI `/integrations`. New matches can then run
automatically in the selected existing project session within that grant. Choose
`mode: "session"` and a finite `max_execution_seconds` budget (30–3600 seconds).
The event is untrusted data inside a typed integration instruction; the grant
supplies authority, and the session retains its current tools, model and native
Subagents. People can continue the same visible conversation. Pause, revocation
and expiry cancel only the event’s recorded execution and generation.

`spool_only=true` in the chat plugin is an explicit local demonstration mode.
It does not send a message to a real app. The end-to-end product test uses an
actual HTTP reply gateway instead of treating the local spool as delivery.

## Validate

From the repository root:

```sh
python3 examples/integration-plugins/test_adapters.py -v
cargo test -p agena --test system scenarios::integrations --offline \
  -- --test-threads=1 --nocapture
```

Regenerate the contract after changing Rust SDK types:

```sh
cargo run -p agena-bundled-plugins --example generate_integration_manifests \
  --offline > examples/integration-plugins/manifests.json
```

CI compares the checked-in manifest against fresh SDK generation. The Python
tests use owned loopback servers and temporary SQLite files, including protocol
framing, signature/replay validation, pagination, conditional fetches, rate
limits, fixed destinations and idempotent send receipts.

To add another platform, replace the provider-specific I/O while keeping this
versioned service contract. Rich media, streaming edits and remote native
approval UI require new typed capabilities; do not encode them as arbitrary
instructions in inbound chat text.


## Multi-user security contract

Channels now require a verified `channel` envelope (tenant, bot account, direct/
group/public/private-channel kind, membership and mention evidence) and a local
`channel_policy`. A signed username alone does not assign a role. The Runtime
uses local member roles and immutable per-session capability limits; old sender
allowlists are not silently upgraded to owner privileges. See
[`docs/channel-access.md`](../../docs/channel-access.md).

Send a stable, signed `membership.revoked` event when the platform removes a
member. Set `actor_verified=true`, `is_member=false` and a fresh verification
time. This path only revokes privileges and is processed ahead of messages and
outside chat budgets. Without a platform event the host cannot know a member
left; configure membership expirations and reliable membership-event forwarding.
