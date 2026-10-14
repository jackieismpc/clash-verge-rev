fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let mut child = std::process::Command::new(&args[0])
        .arg("--supervise").arg(&args[1])
        .stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null())
        .spawn().unwrap();
    println!("{}", child.id());
    // A killed parent cannot run destructors; its pipe still closes in the kernel.
    let _lifetime = child.stdin.take().unwrap();
    std::thread::sleep(std::time::Duration::from_secs(60));
}
