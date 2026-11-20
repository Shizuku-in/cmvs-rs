fn main() {
    if let Err(error) = cmvs_disassembler::run(&std::env::args_os().skip(1).collect::<Vec<_>>()) {
        eprintln!("Error: {error}\nRun 'cmvs-disassembler --help' for usage.");
        std::process::exit(1);
    }
}
