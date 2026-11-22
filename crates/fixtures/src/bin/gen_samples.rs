use jadeslip_fixtures::samples::{generate, samples_dir};
use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let out = match std::env::args().skip(1).collect::<Vec<_>>().as_slice() {
        [] => samples_dir(),
        [flag, dir] if flag == "--out" => PathBuf::from(dir),
        _ => {
            eprintln!("usage: gen-samples [--out DIR]");
            return ExitCode::from(2);
        }
    };
    match generate(&out) {
        Ok(paths) => {
            paths.iter().for_each(|path| println!("{}", path.display()));
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("gen-samples: {error}");
            ExitCode::FAILURE
        }
    }
}
