fn main() {
    if let Err(error) = cmvs_patch::run_cli(std::env::args_os().skip(1)) {
        eprintln!("Error: {error}\nRun 'cmvs-patch --help' for usage.");
        std::process::exit(1);
    }
}
