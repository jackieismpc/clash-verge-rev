# Native business runtime

Framework-independent Rust for Apple Silicon/macOS 26+; the protocol engine remains
external pinned mihomo. Outputs/dependencies use `.work` and installed toolchains.

```sh
scripts/native-core-build.sh build --offline
scripts/native-core-build.sh test --offline
scripts/native-core-build.sh clippy --offline --all-targets -- -D warnings
```

## Ownership and ABI

`include/clash_verge_native_core.h` is the single **ABI 2** contract, imported by
Swift's NativeCoreC module. JSON schema remains version 1, with request IDs, core
generation and revision. Inputs are explicit UTF-8 spans (no NUL scanning); outputs
carry byte length, allocation ID and status. Handle tokens are never dereferenced.

Concurrent calls acquire owned client leases. Stale handles and forged buffer frees
are rejected; release/destroy are idempotent. `cvn_destroy` returns 2 while its cleanup
thread is joining the runtime, then 0. A failed cleanup remains failed on repeated
calls. Never unload the library before every handle reports 0 and every buffer is
released. Swift retains the mapped code on cleanup failure/timeout. Request deadlines
are 120 seconds (180 for all media checks), shutdown observation is 30 seconds;
a timeout is not proof that a submitted mutation failed, so refresh before retrying.

Rust owns a two-thread Tokio runtime, configuration, cancellation and owned tasks.
The queue limits accepted commands by count and 64 MiB of input; output reservations
and pending-result limits apply backpressure. Outbound C buffers are limited to 64
allocations and 128 MiB total. A full allocation pool preserves the queued event for
retry. Metrics coalesce, streams cap at 4 MiB, HTTP responses cap at 8 MiB and
WebSocket messages/frames at 2 MiB. Oversized results become an explicit
`resultUnavailable` response carrying `operationSucceeded`, not fabricated success.
The authoritative snapshot remains available; state notifications may coalesce or
be omitted during congestion. Worker failure cancels and completes accepted IDs.
A private directory lock rejects a second runtime owner.

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
media checks and native JSON backups. `profile.import` accepts exactly one of `yaml`, `url`, `uri` or `source`.
The generic `source` input is classified by Rust, including pasted share links. HTTP(S) subscriptions use normal TLS, bounded redirects and a 16 MiB
limit. Scheduling preserves local rules and cancels stale tickets. Legacy imports
copy bounded original directory/ZIP bytes before conversion, reject traversal and
symlinks, preserve selections and never execute custom JS.

Update activation validates private path, architecture, digest, signature and config,
then restores the prior engine on failed persistence. The pinned bundle is retained.
Privileged TUN cannot use a user-writable candidate. Streams reconnect with bounded
backoff; rotating logs and owned stream tasks remain bounded and are joined on quit.

## Platform and isolation

All Mac proxy/DNS/TUN writes use a standalone root-owned SMAppService helper and
durable field ownership journals. The GUI no longer writes SystemConfiguration
using a transient authorization prompt. The helper authenticates
the GUI audit token/code hash, owns only its child/cache/socket and exposes a
controller firewall instead of arbitrary privileged config paths. Its engine is
fixed inside the signed installed bundle. Proxy/DNS leases monitor both the GUI audit
token and the owned core's PID/UID/start time. Restore compares only owned fields,
retains partial ApplyChanges failures for retry and never recreates deleted services.
Unavailable helper connections before submission cannot create phantom recovery
state; a lost reply after submission or an existing lease keeps recovery required.
Service restart recovers journals and exact orphaned engine identities first. Legacy
GUI journals from the signed bundle's default data directory migrate to root storage
before restoration; unfinished legacy recovery is surfaced by the GUI. Ordinary
engines run through the bundled unprivileged supervisor (`hostPath`) and its lifetime
pipe; GUI death closes that pipe. Privileged engines are recorded before TUN routes
are enabled and use redir-host instead of persistent fake IPs.

Installation/uninstallation scripts use fixed identities, refuse live/redirected
installations and delete owned files only after service unregistration and recovery.
Actual OS validation is skipped by user
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
