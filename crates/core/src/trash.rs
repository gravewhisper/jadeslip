use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub struct Trash {
    root: PathBuf,
}

pub struct Trashed {
    from: PathBuf,
    file: PathBuf,
    info: PathBuf,
}

impl Trashed {
    pub fn restore(self) -> io::Result<()> {
        rename_new(&self.file, &self.from)?;
        let _ = fs::remove_file(&self.info);
        Ok(())
    }

    pub fn place(&self) -> &Path {
        &self.from
    }

    pub fn in_trash(&self) -> &Path {
        &self.file
    }
}

#[cfg(all(target_os = "linux", target_env = "gnu"))]
fn rename_new(from: &Path, to: &Path) -> io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let from_c = CString::new(from.as_os_str().as_bytes())?;
    let to_c = CString::new(to.as_os_str().as_bytes())?;
    let renamed = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            from_c.as_ptr(),
            libc::AT_FDCWD,
            to_c.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if renamed == 0 {
        return Ok(());
    }
    let error = io::Error::last_os_error();
    match error.raw_os_error() {
        Some(libc::EINVAL | libc::ENOSYS) => rename_if_free(from, to),
        _ => Err(error),
    }
}

#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
fn rename_new(from: &Path, to: &Path) -> io::Result<()> {
    rename_if_free(from, to)
}

fn rename_if_free(from: &Path, to: &Path) -> io::Result<()> {
    if fs::symlink_metadata(to).is_ok() {
        return Err(io::ErrorKind::AlreadyExists.into());
    }
    fs::rename(from, to)
}

const UNRESERVED: &[u8] = b"-_.~/";

fn encoded(path: &Path) -> String {
    #[cfg(unix)]
    let bytes = {
        use std::os::unix::ffi::OsStrExt;
        path.as_os_str().as_bytes().to_vec()
    };
    #[cfg(not(unix))]
    let bytes = path.to_string_lossy().into_owned().into_bytes();
    bytes
        .iter()
        .map(|&byte| {
            if byte.is_ascii_alphanumeric() || UNRESERVED.contains(&byte) {
                char::from(byte).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect()
}

fn civil_date(seconds: u64) -> String {
    let days = (seconds / 86_400) as i64;
    let rest = seconds % 86_400;
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era = (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 { month_index + 3 } else { month_index - 9 };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}

impl Trash {
    pub fn at(root: PathBuf) -> Trash {
        Trash { root }
    }

    pub fn home() -> Option<Trash> {
        let data = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|dir| dir.is_absolute())
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")))?;
        Some(Trash::at(data.join("Trash")))
    }

    pub fn put(&self, path: &Path) -> io::Result<Trashed> {
        let from = std::path::absolute(path)?;
        fs::symlink_metadata(&from)?;
        let files = self.root.join("files");
        let infos = self.root.join("info");
        fs::create_dir_all(&files)?;
        fs::create_dir_all(&infos)?;
        let base = from
            .file_name()
            .map_or_else(|| OsString::from("item"), ToOwned::to_owned);
        let date = civil_date(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |since| since.as_secs()),
        );
        for attempt in 1..10_000 {
            let mut name = base.clone();
            if attempt > 1 {
                name.push(format!(".{attempt}"));
            }
            let mut info_name = name.clone();
            info_name.push(".trashinfo");
            let info = infos.join(&info_name);
            let file = files.join(&name);
            let mut out = match File::options().write(true).create_new(true).open(&info) {
                Ok(out) => out,
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            };
            if fs::symlink_metadata(&file).is_ok() {
                drop(out);
                let _ = fs::remove_file(&info);
                continue;
            }
            let written = write!(
                out,
                "[Trash Info]\nPath={}\nDeletionDate={date}\n",
                encoded(&from)
            )
            .and_then(|()| out.sync_all());
            drop(out);
            let moved = written.and_then(|()| fs::rename(&from, &file));
            if let Err(error) = moved {
                let _ = fs::remove_file(&info);
                return Err(error);
            }
            return Ok(Trashed { from, file, info });
        }
        Err(io::Error::other("the trash has too many items with this name"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(label: &str) -> PathBuf {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/trash-tests")
            .join(format!("{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_trashed_file_has_its_info_and_comes_back() {
        let dir = scratch("put");
        let trash = Trash::at(dir.join("Trash"));
        let path = dir.join("my book%.epub");
        fs::write(&path, b"words").unwrap();
        let first = trash.put(&path).unwrap();
        assert!(!path.exists());
        assert_eq!(fs::read(dir.join("Trash/files/my book%.epub")).unwrap(), b"words");
        let info = fs::read_to_string(dir.join("Trash/info/my book%.epub.trashinfo")).unwrap();
        assert!(info.starts_with("[Trash Info]\nPath=/"), "{info}");
        assert!(info.contains("/my%20book%25.epub\n"), "{info}");
        assert!(info.contains("\nDeletionDate=20"), "{info}");
        fs::write(&path, b"again").unwrap();
        let second = trash.put(&path).unwrap();
        assert_eq!(fs::read(dir.join("Trash/files/my book%.epub.2")).unwrap(), b"again");
        second.restore().unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"again");
        assert!(!dir.join("Trash/info/my book%.epub.2.trashinfo").exists());
        drop(first);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_restored_item_never_replaces_a_file_made_since() {
        let dir = scratch("restore-taken");
        let trash = Trash::at(dir.join("Trash"));
        let path = dir.join("book.epub");
        fs::write(&path, b"old copy").unwrap();
        let trashed = trash.put(&path).unwrap();
        fs::write(&path, b"new work").unwrap();
        let restored = trashed.restore();
        assert_eq!(fs::read_to_string(&path).unwrap(), "new work");
        assert_eq!(restored.unwrap_err().kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(
            fs::read(dir.join("Trash/files/book.epub")).unwrap(),
            b"old copy"
        );
        assert!(dir.join("Trash/info/book.epub.trashinfo").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_item_leaves_no_info_behind() {
        let dir = scratch("missing");
        let trash = Trash::at(dir.join("Trash"));
        assert!(trash.put(&dir.join("gone.epub")).is_err());
        assert!(fs::read_dir(dir.join("Trash/info")).map_or(true, |mut found| found.next().is_none()));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_deletion_date_is_a_calendar_date() {
        assert_eq!(civil_date(0), "1970-01-01T00:00:00");
        assert_eq!(civil_date(1_709_210_096), "2024-02-29T12:34:56");
    }
}
