fn main() {
    if std::env::args().nth(1).as_deref() == Some("__worker") {
        std::process::exit(soma::worker::run());
    }
    std::process::exit(soma::cli::run());
}
