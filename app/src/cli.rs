use std::ffi::OsString;
use std::io::IsTerminal;
use std::path::Path;

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

pub fn resolve_book_from(args: &mut [OsString], launched_from: &Path) {
    let book = usize::from(wants_cli(args));
    if let Some(path) = args.get_mut(book)
        && Path::new(path).is_relative()
    {
        *path = launched_from.join(&path).into_os_string();
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
    use super::{resolve_book_from, wants_cli};
    use std::ffi::OsString;
    use std::path::Path;

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

    #[test]
    fn an_appimage_resolves_only_the_book_against_the_folder_it_was_started_in() {
        let from = Path::new("/home/reader/books");
        let resolved = |list: &[&str]| {
            let mut list = args(list);
            resolve_book_from(&mut list, from);
            list
        };
        assert_eq!(resolved(&["a.epub"]), args(&["/home/reader/books/a.epub"]));
        assert_eq!(resolved(&["/x/a.epub"]), args(&["/x/a.epub"]));
        assert_eq!(
            resolved(&["extract", "a.epub", "3"]),
            args(&["extract", "/home/reader/books/a.epub", "3"])
        );
        assert_eq!(
            resolved(&["info", "../a.epub"]),
            args(&["info", "/home/reader/books/../a.epub"])
        );
        assert_eq!(resolved(&[]), args(&[]));
    }
}
