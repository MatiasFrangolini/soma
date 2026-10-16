fn main() {
    if std::env::args().nth(1).as_deref() == Some("__maven_child") {
        std::process::exit(soma::jobs::maven_child());
    }
    if std::env::args().nth(1).as_deref() == Some("__worker") {
        std::process::exit(soma::worker::run());
    }
    std::process::exit(soma::cli::run());
}
