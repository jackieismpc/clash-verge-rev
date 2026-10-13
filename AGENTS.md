# Native Mac Client Guidelines

This is an independently maintained descendant of Clash Verge Rev. The approved
native migration is the change scope; an upstream issue or pull request is not
required. Preserve the GPL license, source attribution, and Git history.

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
- Keep the existing UI usable until all retained workflows work in SwiftUI.
  Label demonstration data. A visual sample is not a functioning proxy client.
- Swift owns native presentation. Rust owns persistent configuration, command
  ordering, core lifecycle, cancellation, and events. Keep the proxy engine
  version fixed during the interface migration.
- Preserve configuration transactions, authoritative runtime state, owner
  identity, bounded queues, proxy/DNS restoration, and cancellation semantics.
  An older snapshot or callback must never replace a newer core generation.
- Run meaningful existing checks and focused regression tests. Do not add
  unrelated formatting, dependency upgrades, speculative frameworks, or reports
  to commits. Document copyable build and usage instructions.
  Use `scripts/check-local.sh` for native checks; legacy checks remain in README.md.
- Generated contracts remain owned by their producer. Update all consumers when
  changing a command, event, field, or configuration format.
- Code and comments are English. User-facing text may be Chinese. Keep comments
  limited to constraints that the code cannot express.

The `src` and `src-tauri` guardrails continue to apply while the legacy shell is
present. The native business library must build without Tauri.
