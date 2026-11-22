use std::ffi::c_int;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

const PREFIX: &str = "jadeslip-test-";

unsafe extern "C" {
    fn atexit(callback: extern "C" fn()) -> c_int;
}

fn root_of(pid: u32) -> PathBuf {
    std::env::temp_dir().join(format!("{PREFIX}{pid}"))
}

extern "C" fn remove_root() {
    let _ = fs::remove_dir_all(root_of(std::process::id()));
}

fn remove_roots_of_dead_processes() {
    let Ok(entries) = fs::read_dir(std::env::temp_dir()) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let pid = name
            .to_str()
            .and_then(|name| name.strip_prefix(PREFIX))
            .and_then(|pid| pid.parse::<u32>().ok());
        if let Some(pid) = pid
            && !Path::new(&format!("/proc/{pid}")).exists()
        {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}

pub fn root() -> &'static Path {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| {
        remove_roots_of_dead_processes();
        let root = root_of(std::process::id());
        fs::create_dir_all(&root).expect("scratch root");
        assert_eq!(
            unsafe { atexit(remove_root) },
            0,
            "scratch cleanup registered"
        );
        root
    })
}

pub fn scratch(name: &str) -> PathBuf {
    let dir = root().join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("scratch directory");
    dir
}
