fn main() {
    if let Err(error) = suneiro_core::delegation_bridge::run_stdio() {
        eprintln!("{error:#}");
        std::process::exit(1);
    }
}
