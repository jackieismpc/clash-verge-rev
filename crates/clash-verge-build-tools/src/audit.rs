use anyhow::{Context, Result, bail};
use std::{io::Read, path::Path, process::Command};

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
    if extension == "json" && path != Path::new("resources/core-artifacts.json") && !components.contains(&"fixtures") {
        return false;
    }
    if ["yaml", "yml"].contains(&extension.as_str()) && !components.contains(&"fixtures") {
        return false;
    }
    true
}
fn executable_magic(bytes: [u8; 4]) -> bool {
    matches!(
        bytes,
        [0xcf, 0xfa, 0xed, 0xfe]
            | [0xfe, 0xed, 0xfa, 0xcf]
            | [0xce, 0xfa, 0xed, 0xfe]
            | [0xfe, 0xed, 0xfa, 0xce]
            | [0xca, 0xfe, 0xba, 0xbe]
            | [0xbe, 0xba, 0xfe, 0xca]
            | [0x7f, b'E', b'L', b'F']
    ) || bytes[..2] == *b"MZ"
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
        if root.join(path).is_file() {
            let mut magic = [0; 4];
            if std::fs::File::open(root.join(path))?.read(&mut magic)? == 4 && executable_magic(magic) {
                bail!("generated executable must not enter Git: {}", path.display());
            }
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
    fn executable_inputs_are_rejected_by_content() {
        assert!(executable_magic([0xcf, 0xfa, 0xed, 0xfe]));
        assert!(executable_magic(*b"\x7fELF"));
        assert!(!executable_magic(*b"icns"));
        assert!(!executable_magic(*b"use "));
    }
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
            "state.json",
            "logs/events.private.json",
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
