# Native business runtime

Tauri-independent Rust library for the Apple Silicon/macOS 26+ presentation.
The proxy protocol implementation remains the pinned external mihomo executable.

```sh
scripts/native-core-build.sh build --offline
scripts/native-core-build.sh test --offline
scripts/native-core-build.sh clippy --offline --all-targets -- -D warnings
```

Uses the existing installed Rust toolchain, with dependencies and output in the
ignored `.work` directory. No global toolchain settings are changed.

## Ownership and ABI

See `include/clash_verge_native_core.h`. UTF-8 JSON schema version 1 carries a
request ID, core generation and optional configuration revision. Each accepted
request gets a result event. The library owns a two-thread Tokio runtime, a
bounded command queue, cancellation tokens, controller streams, durable state
and its child process. A file lock prevents a second owner of the data directory.
Only the owned child and socket are stopped or removed. Consumers free every
returned string exactly once and never access a handle concurrently with destroy.
Create and destroy perform I/O and belong on a background executor.

Mutations serialize in Rust. Delay/media checks run separately from writes;
stop cancels and joins owned reads. Delay requests share eight concurrency slots
across batches, preserve complete test URLs, and publish each result without
random waits or minimum slot occupancy. Generation/revision checks reject late
batch results. Validate and stage configuration before applying
it. Failure before persistence restores the prior runtime; replacement that
already committed stays authoritative even if directory durability confirmation
fails. Generation checks reject callbacks and commands from an older core.
Selection mutations that already reached mihomo remain authoritative if saving
fails, with an explicit error. Stream buffers and pending commands are bounded.

Commands cover profile import/update/select/delete/rename, YAML overrides,
settings, owned core lifecycle, whitelisted controller operations, WebSocket
streams, media checks and local JSON backups. Subscription requests enforce
normal TLS verification, bounded redirects and a 16 MiB response limit. Media
checks use the owned local proxy, with TLS verification enabled.

## Current boundaries

System proxy, TUN and privileged helper commands explicitly return unsupported.
Runtime reconciliation binds to loopback and disables TUN; it never changes OS
DNS, proxy or routes. Subscription schedules, node URI conversion, legacy backup
migration and privileged restoration still require extraction from the old app.
The local backups here contain native state only and do not read legacy ZIPs.
Core startup validates the configuration and waits at most 15 seconds for the
owned Unix controller. Distribution helper/core packaging is a separate step.

Tests use project-local directories and loopback/Unix fixtures. They cover
failed persistence rollback, cancellation without commit, directory ownership,
stale generations, protocol-field preservation and C buffer/shutdown handling.
