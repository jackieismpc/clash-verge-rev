# Native macOS presentation

This Swift package is the independently maintained native presentation layer.
It targets Apple Silicon and macOS 26 or later. The existing Tauri client remains
available while retained workflows migrate. The default transport is explicitly
labelled demonstration data: it does not start a proxy, download subscriptions,
or change operating-system network settings.

## Build and run

From the repository root, using the installed Command Line Tools (Swift 6.2+):

```sh
scripts/native-ui-build.sh
open '.work/native-ui/Clash Native.app'
scripts/native-ui-test.sh
```

The scripts scope temporary files, SwiftPM configuration, caches, and generated
app bundles to `.work/native-ui`. The bundle receives an ad-hoc local code
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

The real Rust adapter must map these presentation commands to the native core's
versioned C ABI and decode runtime, profile, controller, and stream data. Demo
transport rejects unsupported actions and stale generation commands. Capability
availability must come from the real transport, and operating-system actions
must never be inferred from the appearance of a toggle.

## Validation

Swift tests verify snapshot generation/sequence ordering and schema rejection,
missing-sample chart segments, unavailable-node and stale-command rejection, and
JSON round trips. Native accessibility and visual inspection cover the overview,
table single-click/double-click behavior, search empty state, and demo import.
Full privileged proxy/TUN/helper functionality is outside the demo transport and
is not claimed by this presentation package.
