use jadeslip_core::cli::run;
use std::io::IsTerminal;

fn main() {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let stdout = std::io::stdout();
    let is_terminal = stdout.is_terminal();
    let code = run(
        &args,
        &mut stdout.lock(),
        &mut std::io::stderr().lock(),
        is_terminal,
    );
    std::process::exit(code);
}
