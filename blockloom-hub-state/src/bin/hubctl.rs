fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    std::process::exit(blockloom_hub_state::cli::run(&args));
}
