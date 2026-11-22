use std::ffi::OsString;
use std::io::IsTerminal;

const COMMANDS: [&str; 3] = ["info", "extract", "clean-report"];

pub fn wants_cli(args: &[OsString]) -> bool {
    match args {
        [] => false,
        [one] if !COMMANDS.iter().any(|command| one == command) => {
            one.to_string_lossy().starts_with('-')
        }
        _ => true,
    }
}

pub fn run(args: &[OsString]) -> i32 {
    let stdout = std::io::stdout();
    let is_terminal = stdout.is_terminal();
    jadeslip_core::cli::run(
        args,
        &mut stdout.lock(),
        &mut std::io::stderr().lock(),
        is_terminal,
    )
}

#[cfg(test)]
mod tests {
    use super::wants_cli;
    use std::ffi::OsString;

    fn args(list: &[&str]) -> Vec<OsString> {
        list.iter().map(OsString::from).collect()
    }

    #[test]
    fn only_a_single_path_or_nothing_starts_the_window() {
        assert!(!wants_cli(&args(&[])));
        assert!(!wants_cli(&args(&["book.epub"])));
        assert!(wants_cli(&args(&["info"])));
        assert!(wants_cli(&args(&["info", "book.epub"])));
        assert!(wants_cli(&args(&["extract", "book.epub", "3"])));
        assert!(wants_cli(&args(&["--help"])));
        assert!(wants_cli(&args(&["one.epub", "two.epub"])));
    }
}
