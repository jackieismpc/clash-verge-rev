# Native macOS presentation

This Swift package is the independently maintained native presentation layer.
It targets Apple Silicon and macOS 26 or later. The default transport loads the
bundled Rust business library and uses a project-local development data directory,
controller socket and port. The core starts only after an explicit action. It
never changes OS proxy, DNS or TUN settings in this stage. `--demo` selects the
clearly labelled visual fixture instead.

## Build and run

From the repository root, using the installed Command Line Tools (Swift 6.2+):

```sh
node scripts/native-core-assets.mjs
scripts/native-core-build.sh build
scripts/native-ui-build.sh
open '.work/native-ui/Clash Native.app'
scripts/native-ui-test.sh
# Visual fixture only:
open -n '.work/native-ui/Clash Native.app' --args --demo
```

The scripts scope temporary files, SwiftPM configuration, caches, and generated
app bundles to `.work/native-ui`. Rust outputs and the verified pinned mihomo
archive live in `.work/target` and `.work/engines`; they are never Git inputs. The bundle receives an ad-hoc local code
signature; distribution signing and notarization are separate release tasks.
No global Swift, Cargo, Xcode, shell, or proxy configuration is modified.

SDK 27 introduces a SwiftUI `State` macro whose plugin is absent from the current
Command Line Tools distribution. `ViewState<Value>` explicitly aliases the
stable SwiftUI property wrapper so a full Xcode installation is not required.

## Presentation

The default window is 1280 × 820 points and its minimum is 980 × 680. The native
split sidebar is 224 points by default, adjustable between 208 and 256. Content
uses 24-point margins, 16-point gaps, and raised 20-point-radius cards with
20-point padding. System navigation and toolbar controls use Liquid Glass;
content cards remain opaque. Settings follow system colors, contrast, reduced
transparency, and reduced motion, with an app-only light/dark override.

The six pages are overview, profiles, nodes, rules, activity, and media unlock
checks. The overview uses a 2:1:1 latency/upload/download row, then connections
and a five-minute, one-Hertz traffic chart. Missing values or intervals above
1.8 seconds start a new chart segment. Only core-provided metrics are represented;
there are no invented application, router/DNS, or day/month totals.

Nodes use a native searchable and sortable `Table`. A single click inspects;
double-click, Return, or the explicit use action sends a selection command.
`⌘O` opens the import sheet and `⌘R` refreshes. The sheet accepts YAML text or an
HTTP/HTTPS subscription address. Demo import only appends a labelled in-memory
sample; parsing and remote download belong to the Rust transport.

## Transport boundary

`TransportModels.swift` defines Codable, Sendable presentation records and the
`CoreTransport` protocol. `AppModel` runs on the main actor and applies snapshots
only when the schema is supported and `(generation, sequence)` is no older than
the current state. Transport implementation and command ordering belong to Rust.
All UI commands carry a request ID and an expected core generation.

UI command names and payloads:

| Command | Payload |
| --- | --- |
| `refresh` | empty |
| `selectNode` | `nodeID` |
| `importProfile` | `name`, `source`, and `yaml` or `url` |
| `selectProfile` | `profileID` |
| `setMode` | `mode` |
| `startCore`, `stopCore` | empty |
| `closeConnection` | `connectionID` |
| `clearLogs` | empty; clear the current UI log buffer |
| `checkMedia` | `service` |

`NativeCoreTransport.swift` loads the versioned C ABI on its actor executor, frees
returned buffers, drains ordered events and projects runtime/controller data.
Rust serializes writes. Swift preserves distinct commands instead of dropping
new intents behind a global busy flag. Snapshots coalesce ordinary refreshes.
Quit waits for pending operations and owned core cleanup; closing the window
keeps the core and menu bar alive. `CVN_LIBRARY_PATH`, `CVN_MIHOMO_PATH` and
`CVN_DATA_DIRECTORY` can select isolated development inputs per child process.

Subscriptions support URL/YAML/file import, update, selection, editing and
delete. Nodes support individual and batch tests, cancellation and stable row
ordering during tests. The single shared Rust scheduler bounds tests to eight
requests, with no random or minimum slot delay. Late results are rejected after
core generation or configuration revision changes. The original media checker
list is used for individual and bounded parallel whole-list checks, with normal
TLS verification. Native JSON backups can be listed, created and restored.

Network settings currently expose core start/stop/mode and persistent YAML
overrides. System proxy/TUN/helper remain explicitly unavailable pending their
Mac adapter. Old ZIP backup import and subscription schedules are unfinished.
Login startup uses `SMAppService` only when the user changes its toggle;
registration, distribution signing and notarization require product validation.

## Validation

Swift tests verify snapshot generation/sequence ordering and schema rejection,
missing-sample chart segments, unavailable-node and stale-command rejection, and
JSON round trips. Native accessibility and visual inspection cover the overview,
table single-click/double-click behavior, search empty state, and demo import.
A real Swift → C ABI → Rust → pinned arm64 mihomo integration test validates
YAML import, lifecycle, groups containing spaces, node selection, rules, backup
restore and persistence after reopen. Integration uses temporary project-local
data and an ephemeral loopback port. OS proxy/TUN/helper and legacy migration
workflows remain unvalidated. AppKit content bitmap exports cannot represent
composited glass/sidebars, so they are not treated as final visual acceptance.
Review the runnable native window for materials and accessibility.
