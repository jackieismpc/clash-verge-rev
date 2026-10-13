fn main() {
    if let Err(error) = clash_verge_native_core::helper::serve() {
        eprintln!("native helper: {error}");
        std::process::exit(1);
    }
}
