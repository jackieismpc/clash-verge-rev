fn main() {
    let arguments: Vec<_> = std::env::args().skip(1).collect();
    let outcome = if arguments.len() == 2 && arguments[0] == "--supervise" {
        clash_verge_native_core::helper::supervise(&arguments[1])
    } else if arguments == ["--recover-for-uninstall"] {
        clash_verge_native_core::helper::recover_for_uninstall()
    } else {
        clash_verge_native_core::helper::serve()
    };
    if let Err(error) = outcome {
        eprintln!("native helper: {error}");
        std::process::exit(1);
    }
}
