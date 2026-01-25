fn main() {
    if let Err(err) = xtask::run(std::env::args().skip(1)) {
        eprintln!("xtask failed: {err}");
        std::process::exit(1);
    }
}
