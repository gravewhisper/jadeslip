use jadeslip_fixtures::{CATALOG, Options, generate, generated_dir, lock_path, lock_text};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

const USAGE: &str =
    "gen-fixtures [--out DIR] [--only BOOK]... [--probe-port PORT] [--write-lock] [--list]";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("gen-fixtures: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let mut out = generated_dir();
    let mut only = Vec::new();
    let mut options = Options::default();
    let mut write_lock = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = || args.next().ok_or(format!("{arg} needs a value\n{USAGE}"));
        match arg.as_str() {
            "--out" => out = PathBuf::from(value()?),
            "--only" => only.push(value()?),
            "--probe-port" => {
                options.probe_port = value()?.parse().map_err(|e| format!("--probe-port: {e}"))?;
            }
            "--write-lock" => write_lock = true,
            "--list" => {
                CATALOG.iter().for_each(|def| println!("{}", def.name));
                return Ok(());
            }
            _ => return Err(format!("unknown argument {arg}\n{USAGE}")),
        }
    }
    let defs: Vec<_> = CATALOG
        .iter()
        .filter(|def| only.is_empty() || only.iter().any(|name| name == def.name))
        .copied()
        .collect();
    if defs.len() < only.len() {
        return Err(format!("unknown book in {only:?}"));
    }
    let started = Instant::now();
    generate(&defs, &out, options).map_err(|e| e.to_string())?;
    println!(
        "generated {} books in {:.2}s into {}",
        defs.len(),
        started.elapsed().as_secs_f64(),
        out.display()
    );
    if write_lock {
        if defs.len() != CATALOG.len() || options.probe_port != Options::default().probe_port {
            return Err("--write-lock needs the full catalog and the default probe port".into());
        }
        let text = lock_text(&out).map_err(|e| e.to_string())?;
        std::fs::write(lock_path(), text).map_err(|e| e.to_string())?;
        println!("wrote {}", lock_path().display());
    }
    Ok(())
}
