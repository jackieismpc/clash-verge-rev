# Native macOS presentation

SwiftUI is the only presentation, targeting Apple Silicon and macOS 26+.
The default transport loads the bundled Rust business library and uses project-local
data and an independent loopback port/socket. The core starts through an explicit
action; OS proxy/DNS/TUN require separate explicit controls. `--demo` selects
clearly labelled visual data instead. No legacy UI or Node toolchain remains.

## Build and run

From the repository root with installed Rust and Swift 6.2+:

```sh
scripts/native-core-assets.sh
scripts/native-ui-build.sh
open '.work/native-ui/Clash Native.app'
scripts/native-ui-test.sh
# Visual fixture only:
open -n '.work/native-ui/Clash Native.app' --args --demo
```

Outputs and caches stay in `.work`. Bundles default to release optimization with
local ad-hoc signatures; `NATIVE_BUILD_PROFILE=debug` selects debugging. Scripts
use existing tools and do not change global Xcode/Cargo/shell settings. Asset tools
validate HTTPS downloads, SHA256 and arm64 Mach-O; offline failures preserve the
previous core. The stable/alpha update library is shared by the UI and CLI.
Settings can stage/activate a private candidate or restore the pinned baseline;
TUN uses only the installed root-owned signed bundled core.

SDK 27 also exposes a State macro absent from Command Line Tools. `ViewState`
explicitly aliases the stable SwiftUI property wrapper, so full Xcode is not required.

## Presentation and behavior

The main window is 1280 × 820, minimum 980 × 680, sidebar 208–256 (default 224).
Content uses 24-point margins, 16-point gaps, and 20-point card radius/padding.
Navigation and controls use Liquid Glass; content surfaces retain readable contrast.
System colors, appearance, reduced motion/transparency and increased contrast apply.

Six pages cover overview, profiles, nodes, rules, activity and media unlock.
The overview uses a 2:1:1 latency/upload/download row, connection summary and
five-minute traffic chart. Missing samples create gaps; no synthetic totals appear.
Nodes use a searchable/sortable native Table; double-click or Return selects a node.
⌘O imports, ⌘R refreshes. URI parsing and subscription download belong to Rust.

Rules show providers, remote updates, persistent prepend/append/delete rules and
advanced YAML overrides. Activity exposes connection details/close and log filters,
search/pause; buffers remain bounded. Latency progress uses request IDs and shared
Rust concurrency slots; stale batches cannot replace current results.

Subscriptions update in the background (default 1440 minutes, 0 disables), with
bounded retries and stale-ticket cancellation. Legacy directory/ZIP import saves
original copies and reports unsupported JS/YAML overlays; scripts never execute.
Settings contains native backup restore, update candidates and network controls.

Hidden windows unsubscribe metrics and stop projection refresh; bounded logs and
Rust subscription scheduling continue. WebSocket streams reconnect with bounded
backoff. Closing the window preserves the menu bar; quit joins owned tasks, stops
the owned core and restores still-owned network fields.

## Platform setup

SystemConfiguration uses explicit Authorization Services rights. TUN uses the
independent authenticated SMAppService root helper; integration mode forbids all
OS mutations before framework/IPC calls. Build, then manually run:

```sh
sudo scripts/install-local.sh
open '/Applications/Clash Native.app'
```

Register the helper from Settings → Network and approve it in System Settings.
The installed bundle must be root-owned and non-writable to other users. Installation
alone does not register services or modify networking. Quit before updating an
installation; reopen both application and service after replacement. Login startup
uses SMAppService only when its toggle changes. Read the root README for test limits.

## Transport and validation

Codable/Sendable models project the versioned JSON C ABI. AppModel applies only
supported schema and monotonically ordered `(generation, sequence)` snapshots.
Rust serializes mutations and rejects stale generation/revision requests. Consumers
free buffers and destroy the runtime only after ordered shutdown.
`CVN_LIBRARY_PATH`, `CVN_MIHOMO_PATH`, `CVN_DATA_DIRECTORY` select development inputs.

Swift tests exercise real isolated Swift → Rust → mihomo lifecycle, import, providers,
selection, rules, backups, reopen and snapshot ordering. Rust tests cover automatic
updates/rule retention, legacy ZIP imports, corrupt backups, update rollback, stream
reconnection, bounded logs and mocked field ownership restoration. Actual Mac
proxy/DNS/TUN/helper installation and OS writes are skipped by user request;
simulation does not prove them. Real-node evidence and remaining acceptance work
are tracked only in MIGRATION.md. Existing visual fixtures remain available:

```sh
open -n '.work/native-ui/Clash Native.app' --args --demo --appearance=dark
open -n '.work/native-ui/Clash Native.app' --args --demo --page=nodes --scenario=largeList --minimum-window
```
