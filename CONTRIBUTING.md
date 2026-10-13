# Development

This repository is maintained independently for Apple Silicon Macs running
macOS 26 or later. The original GPL license and attribution are retained.

Changes are made directly on `main`, validated, and committed with:

```sh
git commit -s -S -m "fix(component): describe the change"
git verify-commit HEAD
```

Use repository-local signing settings and an existing signing identity. A
failed signature must not be bypassed. There is no upstream issue or PR workflow.

Keep downloaded tools, package caches, build outputs, fixtures and reports under
`.work`. Set tool environment variables only for the command being run. Do not
change global toolchain or shell settings.

The legacy React/Tauri shell remains a behavior reference until the native
client passes the retained workflows. Its existing checks are:

```sh
pnpm typecheck
pnpm test
pnpm arch:check
pnpm knip:check
pnpm gen:contract --check
cargo fmt --all --check
```

Use isolated fixtures for development. Never run integration tests against the
daily proxy, change its system proxy/DNS/TUN, or kill processes by name.
