use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static SERIAL: AtomicU64 = AtomicU64::new(0);

pub struct TempFile {
    path: PathBuf,
    file: File,
}

impl TempFile {
    pub fn beside(dest: &Path) -> io::Result<TempFile> {
        let name = dest.file_name().unwrap_or_default().to_string_lossy();
        let path = dest.with_file_name(format!(
            "{name}.{}-{}.tmp",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        let file = File::options()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)?;
        Ok(TempFile { path, file })
    }

    pub fn file(&self) -> &File {
        &self.file
    }

    pub fn replace(mut self, dest: &Path) -> io::Result<()> {
        fs::rename(&self.path, dest)?;
        self.path = PathBuf::new();
        Ok(())
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        if !self.path.as_os_str().is_empty() {
            let _ = fs::remove_file(&self.path);
        }
    }
}
