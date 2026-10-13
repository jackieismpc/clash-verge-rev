use anyhow::{Context, Result, bail};
use std::{path::Path, process::Command};

fn allowed(path: &Path) -> bool {
    let components: Vec<_> = path.iter().filter_map(|p| p.to_str()).collect();
    if components
        .iter()
        .any(|p| [".work", ".build", "target", "node_modules", "src-tauri"].contains(p) || p.ends_with(".app"))
    {
        return false;
    }
    let filename = path.file_name().and_then(|p| p.to_str()).unwrap_or_default();
    if [
        "package.json",
        "pnpm-lock.yaml",
        "pnpm-workspace.yaml",
        "package-lock.json",
        "yarn.lock",
        "GEMINI.md",
        "CLAUDE.md",
    ]
    .contains(&filename)
    {
        return false;
    }
    let extension = path
        .extension()
        .and_then(|p| p.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if [
        "ts", "tsx", "mts", "cts", "js", "jsx", "mjs", "cjs", "html", "css", "gz", "zip", "log", "dmg", "dylib", "a",
    ]
    .contains(&extension.as_str())
    {
        return false;
    }
    if ["yaml", "yml"].contains(&extension.as_str()) && !components.contains(&"fixtures") {
        return false;
    }
    true
}

pub fn check(root: &Path) -> Result<()> {
    let output = Command::new("/usr/bin/git")
        .args(["ls-files", "--cached", "--others", "--exclude-standard", "-z"])
        .current_dir(root)
        .output()?;
    if !output.status.success() {
        bail!("cannot inventory Git source inputs");
    }
    for path in output.stdout.split(|byte| *byte == 0).filter(|path| !path.is_empty()) {
        let path = Path::new(std::str::from_utf8(path).context("non-UTF-8 Git path")?);
        if root.join(path).is_file() && !allowed(path) {
            bail!(
                "retired source or private/generated artifact must not enter Git: {}",
                path.display()
            );
        }
    }
    let lock = std::fs::read_to_string(root.join("Cargo.lock"))?;
    if lock
        .lines()
        .any(|line| line.starts_with("name = \"tauri") || ["name = \"wry\"", "name = \"tao\""].contains(&line))
    {
        bail!("retired Tauri/WebView dependency remains in Cargo.lock");
    }
    println!("Source audit passed: Swift/Rust inputs, no retired frontend or Tauri packages.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn staged_private_artifacts_and_retired_sources_are_rejected() {
        for path in [
            ".work/integration/run/profile.yaml",
            ".work/engines/mihomo",
            "native/out/libcore.dylib",
            "src-tauri/Cargo.toml",
            "src/view.tsx",
            "scripts/dev.mjs",
            "package.json",
            "profiles/private.yaml",
            "logs/run.log",
            "out/Native.app/Contents/Info.plist",
        ] {
            assert!(!allowed(Path::new(path)), "invalid input accepted: {path}");
        }
        for path in [
            "native/Sources/NativeUI/Pages.swift",
            "crates/core/src/lib.rs",
            "resources/core-artifacts.json",
            "tests/fixtures/artificial.yaml",
            "LICENSE",
        ] {
            assert!(allowed(Path::new(path)), "valid source rejected: {path}");
        }
    }
}
