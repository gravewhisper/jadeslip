use crate::{Error, Limits};
use memchr::memmem;
use std::collections::BTreeMap;
use std::fs::File;
use std::ops::Range;
use std::sync::Mutex;

const EOCD_SIG: &[u8; 4] = b"PK\x05\x06";
const EOCD_LEN: u64 = 22;
const MAX_COMMENT: u64 = 0xFFFF;
const ZIP64_LOCATOR_SIG: &[u8; 4] = b"PK\x06\x07";
const ZIP64_LOCATOR_LEN: u64 = 20;
const ZIP64_EOCD_SIG: &[u8; 4] = b"PK\x06\x06";
const ZIP64_EOCD_LEN: u64 = 56;
const CENTRAL_SIG: &[u8; 4] = b"PK\x01\x02";
const CENTRAL_LEN: usize = 46;
const LOCAL_SIG: &[u8; 4] = b"PK\x03\x04";
const LOCAL_LEN: u64 = 30;
const ZIP64_EXTRA: u16 = 0x0001;
const FLAG_ENCRYPTED: u16 = 1;
const FLAG_DATA_DESCRIPTOR: u16 = 1 << 3;
const METHOD_STORED: u16 = 0;
const METHOD_DEFLATE: u16 = 8;
const U16_MARK: u64 = 0xFFFF;
const U32_MARK: u64 = 0xFFFF_FFFF;

mod write;

pub(crate) use write::{
    Changes, EntryFate, HashingOut, RAW_DEFLATE_LEVEL_6, Rewritten, RewrittenEntry, directory_diff,
    rewrite,
};
pub use write::{NewEntry, RevertKind, RevertRow};
#[cfg(test)]
pub(crate) use write::{end_records as end_records_for_test, push_kept_central_for_test};

#[allow(clippy::len_without_is_empty)]
pub trait ReadAt: Send + Sync {
    fn read_exact_at(&self, buf: &mut [u8], offset: u64) -> std::io::Result<()>;
    fn len(&self) -> std::io::Result<u64>;
}

impl ReadAt for File {
    #[cfg(unix)]
    fn read_exact_at(&self, buf: &mut [u8], offset: u64) -> std::io::Result<()> {
        std::os::unix::fs::FileExt::read_exact_at(self, buf, offset)
    }

    #[cfg(windows)]
    fn read_exact_at(&self, mut buf: &mut [u8], mut offset: u64) -> std::io::Result<()> {
        use std::os::windows::fs::FileExt;
        while !buf.is_empty() {
            let read = self.seek_read(buf, offset)?;
            if read == 0 {
                return Err(std::io::ErrorKind::UnexpectedEof.into());
            }
            buf = &mut buf[read..];
            offset += read as u64;
        }
        Ok(())
    }

    fn len(&self) -> std::io::Result<u64> {
        Ok(self.metadata()?.len())
    }
}

impl<T: ReadAt> ReadAt for &T {
    fn read_exact_at(&self, buf: &mut [u8], offset: u64) -> std::io::Result<()> {
        (**self).read_exact_at(buf, offset)
    }

    fn len(&self) -> std::io::Result<u64> {
        (**self).len()
    }
}

pub struct RecordingReader<R> {
    inner: R,
    seen: Mutex<Vec<Range<u64>>>,
}

impl<R: ReadAt> RecordingReader<R> {
    pub fn new(inner: R) -> Self {
        RecordingReader {
            inner,
            seen: Mutex::new(Vec::new()),
        }
    }

    pub fn ranges(&self) -> Vec<Range<u64>> {
        self.seen.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

impl<R: ReadAt> ReadAt for RecordingReader<R> {
    fn read_exact_at(&self, buf: &mut [u8], offset: u64) -> std::io::Result<()> {
        self.seen
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(offset..offset + buf.len() as u64);
        self.inner.read_exact_at(buf, offset)
    }

    fn len(&self) -> std::io::Result<u64> {
        self.inner.len()
    }
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
pub struct EntryName(String);

impl EntryName {
    pub fn parse(name: &str) -> Result<EntryName, Error> {
        let directory = name.strip_suffix('/').unwrap_or(name);
        let unsafe_segment = directory
            .split('/')
            .any(|segment| matches!(segment, "" | "." | ".."));
        let drive = directory.as_bytes().get(1) == Some(&b':');
        if unsafe_segment || drive || name.contains(['\\', '\0']) {
            return Err(Error::Book(format!(
                "The book contains an unsafe path, {}.",
                printable(name)
            )));
        }
        Ok(EntryName(name.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn printable(name: &str) -> String {
    name.chars().take(80).flat_map(char::escape_debug).collect()
}

pub(crate) struct ZipIndex {
    entries: Vec<ZipEntry>,
    by_name: BTreeMap<EntryName, u32>,
    comment: Box<[u8]>,
}

pub(crate) struct ZipEntry {
    pub(crate) name: EntryName,
    method: u16,
    flags: u16,
    pub(crate) crc32: u32,
    compressed: u64,
    pub(crate) inflated: u64,
    pub(crate) record: Range<u64>,
    raw_central: Box<[u8]>,
    name_len: u16,
}

struct DirectoryLocation {
    count: u64,
    size: u64,
    start: u64,
    end_bound: u64,
    comment: Box<[u8]>,
}

fn le16(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
}

fn le32(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

fn le64(bytes: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(bytes[at..at + 8].try_into().expect("eight bytes"))
}

fn not_a_zip() -> Error {
    Error::Book(
        "This file is not a readable EPUB, because its ZIP directory is missing.".to_owned(),
    )
}

fn io_as_truncated(error: std::io::Error) -> Error {
    if error.kind() == std::io::ErrorKind::UnexpectedEof {
        Error::Book("The book's ZIP directory points past the end of the file.".to_owned())
    } else {
        Error::Io(error)
    }
}

fn locate_directory<R: ReadAt>(source: &R, len: u64) -> Result<DirectoryLocation, Error> {
    if len < EOCD_LEN {
        return Err(not_a_zip());
    }
    let tail_len = len.min(EOCD_LEN + MAX_COMMENT);
    let tail_start = len - tail_len;
    let mut tail = vec![0u8; tail_len as usize];
    source.read_exact_at(&mut tail, tail_start)?;
    let mut found = None;
    for at in memmem::rfind_iter(&tail, EOCD_SIG) {
        if eocd_reaches_end(&tail, at) {
            if found.is_some() {
                return Err(ambiguous_directory());
            }
            found = Some(at);
        }
    }
    let at = found.ok_or_else(not_a_zip)?;
    directory_from_eocd(
        source,
        &tail[at..at + EOCD_LEN as usize],
        &tail[at + EOCD_LEN as usize..],
        tail_start + at as u64,
    )
}

fn eocd_reaches_end(window: &[u8], at: usize) -> bool {
    let fixed_end = at + EOCD_LEN as usize;
    fixed_end <= window.len() && fixed_end + le16(window, at + 20) as usize == window.len()
}

fn ambiguous_directory() -> Error {
    Error::Book("This file has more than one ZIP directory, so it is not safe to open.".to_owned())
}

fn directory_from_eocd<R: ReadAt>(
    source: &R,
    eocd: &[u8],
    comment: &[u8],
    eocd_at: u64,
) -> Result<DirectoryLocation, Error> {
    if le16(eocd, 4) != 0 || le16(eocd, 6) != 0 || le16(eocd, 8) != le16(eocd, 10) {
        return Err(Error::Book(
            "Multi-part ZIP files are not supported.".to_owned(),
        ));
    }
    let count = u64::from(le16(eocd, 10));
    let size = u64::from(le32(eocd, 12));
    let start = u64::from(le32(eocd, 16));
    if count != U16_MARK && size != U32_MARK && start != U32_MARK {
        return Ok(DirectoryLocation {
            count,
            size,
            start,
            end_bound: eocd_at,
            comment: comment.into(),
        });
    }
    let locator_at = eocd_at
        .checked_sub(ZIP64_LOCATOR_LEN)
        .ok_or_else(not_a_zip)?;
    let mut locator = [0u8; ZIP64_LOCATOR_LEN as usize];
    source.read_exact_at(&mut locator, locator_at)?;
    if &locator[..4] != ZIP64_LOCATOR_SIG || le32(&locator, 4) != 0 || le32(&locator, 16) != 1 {
        return Err(not_a_zip());
    }
    let eocd64_at = le64(&locator, 8);
    if eocd64_at.saturating_add(ZIP64_EOCD_LEN) > locator_at {
        return Err(not_a_zip());
    }
    let mut eocd64 = [0u8; ZIP64_EOCD_LEN as usize];
    source.read_exact_at(&mut eocd64, eocd64_at)?;
    if &eocd64[..4] != ZIP64_EOCD_SIG || le32(&eocd64, 16) != 0 || le32(&eocd64, 20) != 0 {
        return Err(not_a_zip());
    }
    Ok(DirectoryLocation {
        count: le64(&eocd64, 32),
        size: le64(&eocd64, 40),
        start: le64(&eocd64, 48),
        end_bound: eocd64_at,
        comment: comment.into(),
    })
}

fn zip64_field(declared: u64, extra: &mut Option<&[u8]>) -> Result<u64, Error> {
    if declared != U32_MARK {
        return Ok(declared);
    }
    let bad = || Error::Book("The book's ZIP directory has a broken size record.".to_owned());
    let fields = extra.as_mut().ok_or_else(bad)?;
    if fields.len() < 8 {
        return Err(bad());
    }
    let value = le64(fields, 0);
    *fields = &fields[8..];
    Ok(value)
}

fn zip64_extra(extra: &[u8]) -> Option<Range<usize>> {
    let mut at = 0;
    while extra.len() >= at + 4 {
        let id = le16(extra, at);
        let size = usize::from(le16(extra, at + 2));
        let body = at + 4..at + 4 + size;
        extra.get(body.clone())?;
        if id == ZIP64_EXTRA {
            return Some(body);
        }
        at = body.end;
    }
    None
}

fn parse_directory(directory: &[u8], count: u64) -> Result<Vec<(ZipEntry, u64)>, Error> {
    let broken = || Error::Book("The book's ZIP directory is damaged.".to_owned());
    let mut entries = Vec::with_capacity(count as usize);
    let mut at = 0usize;
    for _ in 0..count {
        let fixed = directory.get(at..at + CENTRAL_LEN).ok_or_else(broken)?;
        if &fixed[..4] != CENTRAL_SIG {
            return Err(broken());
        }
        let name_len = le16(fixed, 28);
        let extra_len = usize::from(le16(fixed, 30));
        let comment_len = usize::from(le16(fixed, 32));
        let name_end = at + CENTRAL_LEN + usize::from(name_len);
        let extra_end = name_end + extra_len;
        let total_end = extra_end + comment_len;
        let raw = directory.get(at..total_end).ok_or_else(broken)?;
        let name_bytes = &directory[at + CENTRAL_LEN..name_end];
        let name = match std::str::from_utf8(name_bytes) {
            Ok(name) => EntryName::parse(name)?,
            Err(_) => {
                let latin1: String = name_bytes.iter().map(|&byte| byte as char).collect();
                EntryName::parse(&latin1)?
            }
        };
        let extra_bytes = &directory[name_end..extra_end];
        let mut extra = zip64_extra(extra_bytes).map(|body| &extra_bytes[body]);
        let inflated = zip64_field(u64::from(le32(fixed, 24)), &mut extra)?;
        let compressed = zip64_field(u64::from(le32(fixed, 20)), &mut extra)?;
        let offset = zip64_field(u64::from(le32(fixed, 42)), &mut extra)?;
        if le16(fixed, 34) != 0 {
            return Err(Error::Book(
                "Multi-part ZIP files are not supported.".to_owned(),
            ));
        }
        entries.push((
            ZipEntry {
                name,
                method: le16(fixed, 10),
                flags: le16(fixed, 8),
                crc32: le32(fixed, 16),
                compressed,
                inflated,
                record: 0..0,
                raw_central: raw.into(),
                name_len,
            },
            offset,
        ));
        at = total_end;
    }
    if at != directory.len() {
        return Err(broken());
    }
    Ok(entries)
}

impl ZipIndex {
    pub(crate) fn empty() -> ZipIndex {
        ZipIndex {
            entries: Vec::new(),
            by_name: BTreeMap::new(),
            comment: Box::default(),
        }
    }

    pub(crate) fn parse<R: ReadAt>(source: &R, limits: &Limits) -> Result<ZipIndex, Error> {
        let len = source.len()?;
        let location = locate_directory(source, len)?;
        if location.count > limits.max_entries as u64 {
            return Err(Error::Limit(format!(
                "This book lists {} files, more than the limit of {}.",
                location.count, limits.max_entries
            )));
        }
        if location.size > limits.max_central_dir {
            return Err(Error::Limit(format!(
                "This book's ZIP directory is {} bytes, more than the limit of {}.",
                location.size, limits.max_central_dir
            )));
        }
        if location.start.checked_add(location.size) != Some(location.end_bound) {
            return Err(Error::Book(
                "The book's ZIP directory does not line up with its end record.".to_owned(),
            ));
        }
        if location.count.saturating_mul(CENTRAL_LEN as u64) > location.size {
            return Err(Error::Book(
                "The book's ZIP directory is damaged.".to_owned(),
            ));
        }
        let mut directory = vec![0u8; location.size as usize];
        source
            .read_exact_at(&mut directory, location.start)
            .map_err(io_as_truncated)?;
        let parsed = parse_directory(&directory, location.count)?;
        Self::from_parsed(parsed, location.start, location.comment, limits)
    }

    fn from_parsed(
        mut parsed: Vec<(ZipEntry, u64)>,
        cd_start: u64,
        comment: Box<[u8]>,
        limits: &Limits,
    ) -> Result<ZipIndex, Error> {
        let past_end =
            || Error::Book("A file in this book points past the end of the book.".to_owned());
        let mut declared_total = 0u64;
        for (entry, offset) in &parsed {
            declared_total = declared_total.saturating_add(entry.inflated);
            let extent = offset
                .checked_add(LOCAL_LEN)
                .and_then(|end| end.checked_add(entry.compressed))
                .ok_or_else(past_end)?;
            if extent > cd_start {
                return Err(past_end());
            }
        }
        if declared_total > limits.max_book_inflated {
            return Err(Error::Limit(format!(
                "This book would unpack to {declared_total} bytes, more than the limit of {}.",
                limits.max_book_inflated
            )));
        }
        parsed.sort_unstable_by_key(|(_, offset)| *offset);
        let offsets: Vec<u64> = parsed.iter().map(|(_, offset)| *offset).collect();
        let mut entries = Vec::with_capacity(parsed.len());
        let mut by_name = BTreeMap::new();
        for (ix, (mut entry, offset)) in parsed.into_iter().enumerate() {
            let next = offsets.get(ix + 1).copied().unwrap_or(cd_start);
            if offset + LOCAL_LEN + u64::from(entry.name_len) + entry.compressed > next {
                return Err(Error::Book(
                    "Files inside this book overlap, which is a known ZIP attack.".to_owned(),
                ));
            }
            entry.record = offset..next;
            if by_name.insert(entry.name.clone(), ix as u32).is_some() {
                return Err(Error::Book(format!(
                    "This book has two files named {}.",
                    printable(entry.name.as_str())
                )));
            }
            entries.push(entry);
        }
        Ok(ZipIndex {
            entries,
            by_name,
            comment,
        })
    }

    pub(crate) fn entry(&self, name: &EntryName) -> Option<&ZipEntry> {
        self.by_name.get(name).map(|&ix| &self.entries[ix as usize])
    }

    pub(crate) fn entries(&self) -> &[ZipEntry] {
        &self.entries
    }

    pub(crate) fn read_entry<R: ReadAt>(
        &self,
        source: &R,
        entry: &ZipEntry,
        limits: &Limits,
    ) -> Result<Vec<u8>, Error> {
        self.read_packed(source, entry, limits)?
            .unpack(entry.name.as_str())
    }

    pub(crate) fn read_packed<R: ReadAt>(
        &self,
        source: &R,
        entry: &ZipEntry,
        limits: &Limits,
    ) -> Result<Packed, Error> {
        let label = printable(entry.name.as_str());
        if entry.flags & FLAG_ENCRYPTED != 0 {
            return Err(Error::Book(format!(
                "{label} is encrypted, so it cannot be read."
            )));
        }
        if entry.method != METHOD_STORED && entry.method != METHOD_DEFLATE {
            return Err(Error::Book(format!(
                "{label} uses a compression method this app does not support."
            )));
        }
        if entry.inflated > limits.max_entry_inflated {
            return Err(entry_too_large(&label, limits));
        }
        let most_compressed = limits.max_entry_inflated + limits.max_entry_inflated / 1000 + 1024;
        if entry.compressed > most_compressed {
            return Err(entry_too_large(&label, limits));
        }
        let data_start = self.check_local_header(source, entry, &label)?;
        let mut body = vec![0u8; entry.compressed as usize];
        source.read_exact_at(&mut body, data_start)?;
        Ok(Packed {
            method: entry.method,
            crc32: entry.crc32,
            inflated: entry.inflated,
            body,
        })
    }

    fn check_local_header<R: ReadAt>(
        &self,
        source: &R,
        entry: &ZipEntry,
        label: &str,
    ) -> Result<u64, Error> {
        let name_len = usize::from(entry.name_len);
        let mut header = vec![0u8; LOCAL_LEN as usize + name_len];
        source.read_exact_at(&mut header, entry.record.start)?;
        let disagrees = || Error::Book(format!("{label} has ZIP records that disagree."));
        let central_name = &entry.raw_central[CENTRAL_LEN..CENTRAL_LEN + name_len];
        if &header[..4] != LOCAL_SIG
            || usize::from(le16(&header, 26)) != name_len
            || &header[LOCAL_LEN as usize..] != central_name
        {
            return Err(disagrees());
        }
        let local_flags = le16(&header, 6);
        if local_flags & FLAG_DATA_DESCRIPTOR == 0 {
            let compressed = u64::from(le32(&header, 18));
            let inflated = u64::from(le32(&header, 22));
            let zip64 = compressed == U32_MARK || inflated == U32_MARK;
            let same_sizes = compressed == entry.compressed && inflated == entry.inflated;
            if le32(&header, 14) != entry.crc32 || (!zip64 && !same_sizes) {
                return Err(disagrees());
            }
        }
        let data_start = entry.record.start
            + LOCAL_LEN
            + u64::from(entry.name_len)
            + u64::from(le16(&header, 28));
        if data_start + entry.compressed > entry.record.end {
            return Err(disagrees());
        }
        Ok(data_start)
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct Packed {
    pub(crate) method: u16,
    pub(crate) crc32: u32,
    pub(crate) inflated: u64,
    pub(crate) body: Vec<u8>,
}

impl Packed {
    pub(crate) fn unpack(self, name: &str) -> Result<Vec<u8>, Error> {
        let label = printable(name);
        let output = match self.method {
            METHOD_STORED => self.body,
            METHOD_DEFLATE => miniz_oxide::inflate::decompress_to_vec_with_limit(
                &self.body,
                self.inflated as usize,
            )
            .map_err(|failure| match failure.status {
                miniz_oxide::inflate::TINFLStatus::HasMoreOutput => Error::Book(format!(
                    "{label} unpacks to more than the size it declares."
                )),
                _ => Error::Book(format!("{label} is damaged and could not be unpacked.")),
            })?,
            _ => {
                return Err(Error::Book(format!(
                    "{label} uses a compression method this app does not support."
                )));
            }
        };
        if output.len() as u64 != self.inflated {
            return Err(Error::Book(format!(
                "{label} does not match the size it declares."
            )));
        }
        if crc32fast::hash(&output) != self.crc32 {
            return Err(Error::Book(format!(
                "{label} failed its checksum, so the book is damaged."
            )));
        }
        Ok(output)
    }
}

fn entry_too_large(label: &str, limits: &Limits) -> Error {
    Error::Limit(format!(
        "{label} is larger than the limit of {} bytes for one file.",
        limits.max_entry_inflated
    ))
}

#[cfg(test)]
mod tests;
