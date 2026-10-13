fn main() {
    assert_eq!(
        std::env::var("TARGET").as_deref(),
        Ok("aarch64-apple-darwin"),
        "Only Apple Silicon macOS is supported"
    );
    #[cfg(feature = "clippy")]
    println!("cargo:warning=Skipping tauri_build during Clippy");
    #[cfg(not(feature = "clippy"))]
    tauri_build::build();
}
