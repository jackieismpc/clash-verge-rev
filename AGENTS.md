# Native Mac Client Guidelines

This is an independently maintained descendant of Clash Verge Rev. The approved
native migration is the change scope; an upstream issue or pull request is not
required. Preserve the GPL license, source attribution, and Git history.

Read `MIGRATION.md` before every development task. Select an unfinished task ID,
and update its progress and validation evidence in the same signed commit as the
implementation. Never mark partial or unvalidated work complete. It is the sole
migration progress document; README and interface documents contain usage only.

- Support Apple Silicon and macOS 26 or later only.
- Work directly on `main`. Validate each coherent change before
  `git commit -s -S`. Use English Conventional Commit subjects. Never fall back to
  an unsigned commit, rewrite published history, or open an upstream PR.
- Use the existing machine Rust and Swift toolchains; do not install duplicate
  project toolchains. Keep build outputs, fixtures, and reports in `.work`.
  Configure Git locally and build environments per child process; do not change
  global shell or Xcode settings.
- Development and builds are local only. Do not add containers, cross-compilation,
  hosted CI, release bots, or duplicated AI-tool instruction files. Keep the
  mihomo update implementation until a separate core replacement is approved.
- Do not stop, reconfigure, or take ownership of the user's daily proxy. Use
  separate application data, controller sockets, ports, and service channels.
- SwiftUI is the only interface. Do not add TS/JS, Node, Tauri or a WebView shell.
  The user approved removal of unmigrated legacy business code; rebuild missing
  functions on the Rust/mihomo boundary. Label demonstration data.
- Swift owns native presentation. Rust owns persistent configuration, command
  ordering, core lifecycle, cancellation, and events. Keep the proxy engine
  version fixed during the interface migration.
- Preserve configuration transactions, authoritative runtime state, owner
  identity, bounded queues, proxy/DNS restoration, and cancellation semantics.
  An older snapshot or callback must never replace a newer core generation.
- Run meaningful existing checks and focused regression tests. Do not add
  unrelated formatting, dependency upgrades, speculative frameworks, or reports
  to commits. Document copyable build and usage instructions.
  Use `scripts/check-local.sh` for source auditing, Rust and native Swift checks.
- Generated contracts remain owned by their producer. Update all consumers when
  changing a command, event, field, or configuration format.
- Code and comments are English. User-facing text may be Chinese. Keep comments
  limited to constraints that the code cannot express.

The Rust business library must remain independent of an application framework.
Native build inputs are Swift, Rust, system tools and the pinned mihomo binary.
