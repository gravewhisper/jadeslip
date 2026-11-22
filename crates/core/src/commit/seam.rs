use std::fs::File;
use std::io;
use std::path::Path;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum LedgerWrite {
    Created,
    Pending,
    Head,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Step {
    BackupCopied,
    BackupSynced,
    BackupRenamed,
    BackupDirSynced,
    LedgerTempWritten(LedgerWrite),
    LedgerSynced(LedgerWrite),
    LedgerRenamed(LedgerWrite),
    LedgerDirSynced(LedgerWrite),
    TempCreated,
    TempWriting,
    TempWritten,
    TempSynced,
    TempVerified,
    AliasWritten,
    DeletedSynced,
    StampRechecked,
    BookRenamed,
    BookDirSynced,
    RecoveryDirSynced,
    LibrarySynced,
    LibraryDirSynced,
    HistoryWriting,
    HistoryWritten,
    HistorySynced,
    CopyWritten,
    CopySynced,
    CopyLinked,
    CopyDirSynced,
    PruneWriting,
    PruneWritten,
    PruneSynced,
    PruneRenamed,
    PruneDirSynced,
}

#[cfg(not(test))]
#[inline(always)]
pub(crate) fn hit(_step: Step) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
pub(crate) use hook::hit;
#[cfg(test)]
pub(crate) use hook::install;

pub(crate) fn sync_file(file: &File, step: Step) -> io::Result<()> {
    file.sync_all()?;
    hit(step)
}

pub(crate) fn sync_dir(dir: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        match File::open(dir)?.sync_all() {
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::InvalidInput | io::ErrorKind::Unsupported
                ) =>
            {
                Ok(())
            }
            other => other,
        }
    }
    #[cfg(not(unix))]
    {
        let _ = dir;
        Ok(())
    }
}

#[cfg(test)]
mod hook {
    use super::*;
    use std::cell::RefCell;

    type Hook = Box<dyn FnMut(Step) -> io::Result<()>>;

    thread_local! {
        static HOOK: RefCell<Option<Hook>> = const { RefCell::new(None) };
    }

    pub(crate) fn hit(step: Step) -> io::Result<()> {
        HOOK.with_borrow_mut(|hook| hook.as_mut().map_or(Ok(()), |hook| hook(step)))
    }

    pub(crate) fn install(hook: impl FnMut(Step) -> io::Result<()> + 'static) {
        HOOK.with_borrow_mut(|slot| *slot = Some(Box::new(hook)));
    }
}
