# Native business runtime

Framework-independent Rust for Apple Silicon/macOS 26+; the protocol engine remains
external pinned mihomo. Outputs/dependencies use `.work` and installed toolchains.

```sh
scripts/native-core-build.sh build --offline
scripts/native-core-build.sh test --offline
scripts/native-core-build.sh clippy --offline --all-targets -- -D warnings
```

## Ownership and ABI

`include/clash_verge_native_core.h` defines schema-version-1 UTF-8 JSON, request ID,
core generation and optional revision. Every accepted request receives a result.
Rust owns a two-thread Tokio runtime, bounded queues, cancellations, configuration,
background subscriptions, controller streams and its core. A directory lock rejects
a second owner. Consumers free buffers once and never destroy a handle concurrently
with calls; creation/destruction belong on a background executor.

Writes serialize. Delay/media checks share eight slots and run outside the write
queue; each result publishes without artificial waits. Generation/revision checks
reject stale results. Configuration is validated/staged before apply and rolled back
on persistence failure. Post-rename durability warnings preserve authoritative state.
A controller selection already applied stays authoritative if saving fails.

## Additional commands

| Command | Arguments |
| --- | --- |
| `subscription.configure` | `id`, `intervalMinutes` (0 disables, null uses default) |
| `rules.overrides.get` / `rules.overrides.set` | empty / arrays `prepend`, `append`, `delete` |
| `legacy.import` | absolute `path` to legacy data directory or ZIP |
| `core.update.check` / `core.update.stage` | `channel`: stable or alpha |
| `core.update.activate` | private `candidatePath` |
| `core.update.reset` | empty, restore bundled baseline |
| `systemProxy.set` / `tun.set` | Boolean `enabled` |
| `systemDns.set` | IP array `servers`; empty restores owned DNS |
| `network.restore` / `helper.status` | empty |

Existing commands cover profile import/update/select/delete/rename, YAML overrides,
settings, owned lifecycle, whitelisted controller requests, WebSocket subscriptions,
media checks and native JSON backups. `profile.import` accepts exactly one of YAML,
URL or URI. HTTP(S) subscriptions use normal TLS, bounded redirects and a 16 MiB
limit. Scheduling preserves local rules and cancels stale tickets. Legacy imports
copy bounded original directory/ZIP bytes before conversion, reject traversal and
symlinks, preserve selections and never execute custom JS.

Update activation validates private path, architecture, digest, signature and config,
then restores the prior engine on failed persistence. The pinned bundle is retained.
Privileged TUN cannot use a user-writable candidate. Streams reconnect with bounded
backoff; rotating logs and owned stream tasks remain bounded and are joined on quit.

## Platform and isolation

Mac proxy/DNS uses authorized SystemConfiguration and a durable field ownership
journal. TUN uses a standalone root-owned SMAppService helper that authenticates
the GUI audit token/code hash, owns only its child/cache/socket and exposes a
controller firewall instead of arbitrary privileged config paths. Its engine is
fixed inside the signed installed bundle. Actual OS validation is skipped by user
request; automated checks do not install/register/authorize this service.

Initialization accepts `executionMode: integrationTest` with absolute
`integrationRoot` ending in `.work/integration` and a private child `dataDir`.
Rust rejects foreign/symlink paths, forces loopback/Unix endpoints, disables TUN,
DNS listeners, NTP and external listeners/UI, and rewrites provider caches.
All network/helper/login operations are forbidden regardless of YAML overrides.
Subprocesses clear inherited environment and apply a child-only private mask;
files are 0600, directories 0700. Interactive mode is the default.

Tests cover persistence/activation rollback, cancellation, stale results, stream
reconnection, rule retention, automatic updates, legacy ZIP, corrupt backup, mock
network ownership, private paths/ports and C allocation/shutdown. The explicit
integration binary separately tests real private nodes and records anonymous
summaries; raw credentials and logs never enter Git. See root README for invocation.
