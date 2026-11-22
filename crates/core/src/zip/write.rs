use super::*;
use crate::FileHash;
use sha2::{Digest, Sha256};
use std::io::Write;
use std::time::{Duration, Instant};

const CHUNK: usize = 1 << 20;
const MIMETYPE: &str = "mimetype";
const MIMETYPE_BODY: &[u8] = b"application/epub+zip";
const UTF8_FLAG: u16 = 1 << 11;
const VERSION_DEFLATE: u16 = 20;
const VERSION_ZIP64: u16 = 45;
const DOS_DATE_1980: u16 = 0x0021;
pub(crate) const RAW_DEFLATE_LEVEL_6: zlib_rs::DeflateConfig = zlib_rs::DeflateConfig {
    level: 6,
    method: zlib_rs::Method::Deflated,
    window_bits: -15,
    mem_level: 8,
    strategy: zlib_rs::Strategy::Default,
};

pub struct NewEntry {
    name: EntryName,
    method: u16,
    crc32: u32,
    inflated: u64,
    body: Vec<u8>,
}

impl Packed {
    pub(crate) fn of(bytes: &[u8]) -> Packed {
        let mut body = vec![0; zlib_rs::compress_bound(bytes.len())];
        let (written, code) = zlib_rs::compress_slice(&mut body, bytes, RAW_DEFLATE_LEVEL_6);
        let written = written.len();
        let (method, body) = if code == zlib_rs::ReturnCode::Ok {
            body.truncate(written);
            (METHOD_DEFLATE, body)
        } else {
            (METHOD_STORED, bytes.to_vec())
        };
        Packed {
            method,
            crc32: crc32fast::hash(bytes),
            inflated: bytes.len() as u64,
            body,
        }
    }
}

impl NewEntry {
    pub fn deflate(name: EntryName, bytes: &[u8]) -> NewEntry {
        NewEntry::from_packed(name, Packed::of(bytes))
    }

    fn stored(name: EntryName, bytes: Vec<u8>) -> NewEntry {
        NewEntry {
            name,
            method: METHOD_STORED,
            crc32: crc32fast::hash(&bytes),
            inflated: bytes.len() as u64,
            body: bytes,
        }
    }

    pub(crate) fn from_packed(name: EntryName, packed: Packed) -> NewEntry {
        NewEntry {
            name,
            method: packed.method,
            crc32: packed.crc32,
            inflated: packed.inflated,
            body: packed.body,
        }
    }

    pub fn name(&self) -> &EntryName {
        &self.name
    }
}

pub(crate) enum EntryFate {
    RawCopy,
    Drop,
    Write(NewEntry),
}

pub(crate) trait Changes {
    fn take(&mut self, entry: &ZipEntry) -> Result<EntryFate, Error>;
    fn added(&mut self) -> Vec<NewEntry>;
    fn spine_delta(&self) -> i64 {
        0
    }
}

pub(crate) struct RewrittenEntry {
    pub(crate) name: EntryName,
    pub(crate) crc32: u32,
    pub(crate) inflated: u64,
}

pub(crate) struct CopiedEntry {
    pub(crate) source_ix: u32,
    pub(crate) offset: u64,
}

pub(crate) struct Rewritten {
    pub(crate) rewritten: Vec<RewrittenEntry>,
    pub(crate) copied: Vec<CopiedEntry>,
    pub(crate) mimetype_normalised: bool,
}

#[derive(Default, Clone, Copy)]
pub(crate) struct OutTimes {
    pub(crate) read: Duration,
    pub(crate) hash: Duration,
    pub(crate) write: Duration,
}

pub(crate) struct HashingOut<W> {
    out: W,
    hasher: Sha256,
    buf: Vec<u8>,
    filled: usize,
    flushed: u64,
    pub(crate) times: OutTimes,
}

impl<W: Write> HashingOut<W> {
    pub(crate) fn new(out: W) -> Self {
        HashingOut {
            out,
            hasher: Sha256::new(),
            buf: vec![0; CHUNK],
            filled: 0,
            flushed: 0,
            times: OutTimes::default(),
        }
    }

    pub(crate) fn position(&self) -> u64 {
        self.flushed + self.filled as u64
    }

    fn flush(&mut self) -> std::io::Result<()> {
        let chunk = &self.buf[..self.filled];
        let started = Instant::now();
        self.hasher.update(chunk);
        let hashed = Instant::now();
        self.out.write_all(chunk)?;
        self.times.hash += hashed - started;
        self.times.write += hashed.elapsed();
        self.flushed += self.filled as u64;
        self.filled = 0;
        Ok(())
    }

    fn push(&mut self, mut bytes: &[u8]) -> std::io::Result<()> {
        while !bytes.is_empty() {
            if self.filled == CHUNK {
                self.flush()?;
            }
            let take = bytes.len().min(CHUNK - self.filled);
            self.buf[self.filled..self.filled + take].copy_from_slice(&bytes[..take]);
            self.filled += take;
            bytes = &bytes[take..];
        }
        Ok(())
    }

    pub(crate) fn copy_from<R: ReadAt>(
        &mut self,
        source: &R,
        range: Range<u64>,
    ) -> std::io::Result<()> {
        let mut at = range.start;
        while at < range.end {
            if self.filled == CHUNK {
                self.flush()?;
            }
            let take = (range.end - at).min((CHUNK - self.filled) as u64) as usize;
            let started = Instant::now();
            source.read_exact_at(&mut self.buf[self.filled..self.filled + take], at)?;
            self.times.read += started.elapsed();
            self.filled += take;
            at += take as u64;
        }
        Ok(())
    }

    pub(crate) fn finish(mut self) -> std::io::Result<(W, FileHash, u64, OutTimes)> {
        self.flush()?;
        Ok((
            self.out,
            FileHash(self.hasher.finalize().into()),
            self.flushed,
            self.times,
        ))
    }
}

fn too_large(what: &str) -> Error {
    Error::Limit(format!(
        "This save would make {what} too large for this app's ZIP writer, so nothing was changed."
    ))
}

fn write_new<W: Write>(
    new: NewEntry,
    template: Option<&ZipEntry>,
    out: &mut HashingOut<W>,
    central: &mut Vec<u8>,
) -> Result<RewrittenEntry, Error> {
    let compressed = u32::try_from(new.body.len()).map_err(|_| too_large("one file"))?;
    let inflated = u32::try_from(new.inflated).map_err(|_| too_large("one file"))?;
    let offset = out.position();
    if offset >= U32_MARK {
        return Err(too_large("the book"));
    }
    let name_bytes = match template {
        Some(entry) => &entry.raw_central[CENTRAL_LEN..CENTRAL_LEN + usize::from(entry.name_len)],
        None => new.name.as_str().as_bytes(),
    };
    let name_len = u16::try_from(name_bytes.len()).map_err(|_| too_large("a file name"))?;
    let flags = match template {
        Some(entry) => entry.flags & UTF8_FLAG,
        None if new.name.as_str().is_ascii() => 0,
        None => UTF8_FLAG,
    };
    let (time, date, made_by, external, comment): (&[u8], &[u8], u16, u32, &[u8]) = match template {
        Some(entry) => (
            &entry.raw_central[12..14],
            &entry.raw_central[14..16],
            le16(&entry.raw_central, 4),
            le32(&entry.raw_central, 38),
            &entry.raw_central[entry.raw_central.len() - comment_len(entry)..],
        ),
        None => (
            &[0, 0],
            &DOS_DATE_1980.to_le_bytes(),
            VERSION_DEFLATE,
            0,
            &[],
        ),
    };

    let mut shared = Vec::with_capacity(26);
    shared.extend_from_slice(&VERSION_DEFLATE.to_le_bytes());
    shared.extend_from_slice(&flags.to_le_bytes());
    shared.extend_from_slice(&new.method.to_le_bytes());
    shared.extend_from_slice(time);
    shared.extend_from_slice(date);
    shared.extend_from_slice(&new.crc32.to_le_bytes());
    shared.extend_from_slice(&compressed.to_le_bytes());
    shared.extend_from_slice(&inflated.to_le_bytes());
    shared.extend_from_slice(&name_len.to_le_bytes());
    shared.extend_from_slice(&0u16.to_le_bytes());

    out.push(LOCAL_SIG)?;
    out.push(&shared)?;
    out.push(name_bytes)?;
    out.push(&new.body)?;

    central.extend_from_slice(CENTRAL_SIG);
    central.extend_from_slice(&made_by.to_le_bytes());
    central.extend_from_slice(&shared);
    central.extend_from_slice(&(comment.len() as u16).to_le_bytes());
    central.extend_from_slice(&0u16.to_le_bytes());
    central.extend_from_slice(&0u16.to_le_bytes());
    central.extend_from_slice(&external.to_le_bytes());
    central.extend_from_slice(&(offset as u32).to_le_bytes());
    central.extend_from_slice(name_bytes);
    central.extend_from_slice(comment);

    Ok(RewrittenEntry {
        name: new.name,
        crc32: new.crc32,
        inflated: new.inflated,
    })
}

fn comment_len(entry: &ZipEntry) -> usize {
    usize::from(le16(&entry.raw_central, 32))
}

fn push_kept_central(central: &mut Vec<u8>, raw: &[u8], offset: u64) -> Result<(), Error> {
    let at = central.len();
    central.extend_from_slice(raw);
    let row = &mut central[at..];
    if u64::from(le32(row, 42)) != U32_MARK {
        let short = u32::try_from(offset)
            .ok()
            .filter(|short| u64::from(*short) != U32_MARK)
            .ok_or_else(|| too_large("the book"))?;
        row[42..46].copy_from_slice(&short.to_le_bytes());
        return Ok(());
    }
    let name_end = CENTRAL_LEN + usize::from(le16(row, 28));
    let extra_end = name_end + usize::from(le16(row, 30));
    let body = zip64_extra(&row[name_end..extra_end]).ok_or_else(|| too_large("the book"))?;
    let sizes = [24, 20]
        .into_iter()
        .filter(|&field| u64::from(le32(row, field)) == U32_MARK)
        .count();
    let field = name_end + body.start + sizes * 8;
    row[field..field + 8].copy_from_slice(&offset.to_le_bytes());
    Ok(())
}

pub(crate) fn end_records(count: u64, size: u64, start: u64, comment: &[u8]) -> Vec<u8> {
    let mut end = Vec::with_capacity(98 + comment.len());
    if count >= U16_MARK || size >= U32_MARK || start >= U32_MARK {
        end.extend_from_slice(ZIP64_EOCD_SIG);
        end.extend_from_slice(&(ZIP64_EOCD_LEN - 12).to_le_bytes());
        end.extend_from_slice(&VERSION_ZIP64.to_le_bytes());
        end.extend_from_slice(&VERSION_ZIP64.to_le_bytes());
        end.extend_from_slice(&[0; 8]);
        end.extend_from_slice(&count.to_le_bytes());
        end.extend_from_slice(&count.to_le_bytes());
        end.extend_from_slice(&size.to_le_bytes());
        end.extend_from_slice(&start.to_le_bytes());
        end.extend_from_slice(ZIP64_LOCATOR_SIG);
        end.extend_from_slice(&[0; 4]);
        end.extend_from_slice(&(start + size).to_le_bytes());
        end.extend_from_slice(&1u32.to_le_bytes());
    }
    let short_count = count.min(U16_MARK) as u16;
    end.extend_from_slice(EOCD_SIG);
    end.extend_from_slice(&[0; 4]);
    end.extend_from_slice(&short_count.to_le_bytes());
    end.extend_from_slice(&short_count.to_le_bytes());
    end.extend_from_slice(&(size.min(U32_MARK) as u32).to_le_bytes());
    end.extend_from_slice(&(start.min(U32_MARK) as u32).to_le_bytes());
    end.extend_from_slice(&(comment.len() as u16).to_le_bytes());
    end.extend_from_slice(comment);
    end
}

fn flush_run<R: ReadAt, W: Write>(
    run: &mut Option<Range<u64>>,
    source: &R,
    out: &mut HashingOut<W>,
) -> std::io::Result<()> {
    if let Some(range) = run.take() {
        out.copy_from(source, range)?;
    }
    Ok(())
}

pub(crate) fn rewrite<R: ReadAt, W: Write>(
    source: &R,
    index: &ZipIndex,
    changes: &mut dyn Changes,
    limits: &Limits,
    out: &mut HashingOut<W>,
) -> Result<Rewritten, Error> {
    let mimetype_name = EntryName(MIMETYPE.to_owned());
    let mimetype = index.entry(&mimetype_name);
    let mimetype_ok = mimetype.is_some_and(|entry| {
        entry.method == METHOD_STORED && index.entries[0].name == mimetype_name
    });
    let mut central = Vec::new();
    let mut rewritten = Vec::new();
    let mut copied = Vec::with_capacity(index.entries.len());
    let mut count = 0u64;
    if !mimetype_ok {
        let body = match mimetype {
            Some(entry) => index.read_entry(source, entry, limits)?,
            None => MIMETYPE_BODY.to_vec(),
        };
        let new = NewEntry::stored(mimetype_name.clone(), body);
        rewritten.push(write_new(new, mimetype, out, &mut central)?);
        count += 1;
    }

    let mut run: Option<Range<u64>> = None;
    for (ix, entry) in index.entries.iter().enumerate() {
        let is_mimetype = entry.name == mimetype_name;
        if is_mimetype && !mimetype_ok {
            continue;
        }
        let fate = if is_mimetype {
            EntryFate::RawCopy
        } else {
            changes.take(entry)?
        };
        match fate {
            EntryFate::RawCopy => {
                let pending = run.as_ref().map_or(0, |range| range.end - range.start);
                let offset = out.position() + pending;
                push_kept_central(&mut central, &entry.raw_central, offset)?;
                copied.push(CopiedEntry {
                    source_ix: ix as u32,
                    offset,
                });
                count += 1;
                match &mut run {
                    Some(range) if range.end == entry.record.start => {
                        range.end = entry.record.end;
                    }
                    _ => {
                        flush_run(&mut run, source, out)?;
                        run = Some(entry.record.clone());
                    }
                }
            }
            EntryFate::Drop => {
                flush_run(&mut run, source, out)?;
            }
            EntryFate::Write(new) => {
                flush_run(&mut run, source, out)?;
                rewritten.push(write_new(new, Some(entry), out, &mut central)?);
                count += 1;
            }
        }
    }
    flush_run(&mut run, source, out)?;
    for new in changes.added() {
        rewritten.push(write_new(new, None, out, &mut central)?);
        count += 1;
    }
    let start = out.position();
    out.push(&central)?;
    out.push(&end_records(
        count,
        central.len() as u64,
        start,
        &index.comment,
    ))?;
    Ok(Rewritten {
        rewritten,
        copied,
        mimetype_normalised: !mimetype_ok,
    })
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RevertKind {
    Changed,
    Added,
    Removed,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RevertRow {
    pub entry: EntryName,
    pub kind: RevertKind,
}

pub(crate) fn directory_diff(current: &ZipIndex, backup: &ZipIndex) -> Vec<RevertRow> {
    let mut rows = Vec::new();
    for (name, &ix) in &current.by_name {
        let entry = &current.entries[ix as usize];
        let kind = match backup.entry(name) {
            None => RevertKind::Added,
            Some(old) if old.crc32 != entry.crc32 || old.inflated != entry.inflated => {
                RevertKind::Changed
            }
            Some(_) => continue,
        };
        rows.push(RevertRow {
            entry: name.clone(),
            kind,
        });
    }
    for name in backup.by_name.keys() {
        if current.entry(name).is_none() {
            rows.push(RevertRow {
                entry: name.clone(),
                kind: RevertKind::Removed,
            });
        }
    }
    rows.sort_by(|a, b| a.entry.cmp(&b.entry));
    rows
}

impl ZipIndex {
    pub(crate) fn check_copied_records<R: ReadAt, S: ReadAt>(
        &self,
        new_source: &R,
        old_index: &ZipIndex,
        old_source: &S,
        copied: &[CopiedEntry],
    ) -> Result<(), String> {
        let mut new_header = Vec::new();
        let mut old_header = Vec::new();
        for copy in copied {
            let old = &old_index.entries[copy.source_ix as usize];
            let new = self
                .entry(&old.name)
                .ok_or_else(|| "a kept file is missing".to_owned())?;
            if new.record.start != copy.offset
                || new.crc32 != old.crc32
                || new.compressed != old.compressed
                || new.inflated != old.inflated
            {
                return Err("a kept file's directory record is wrong".to_owned());
            }
            let header_len = (LOCAL_LEN + u64::from(old.name_len))
                .min(old.record.end - old.record.start) as usize;
            new_header.resize(header_len, 0);
            old_header.resize(header_len, 0);
            new_source
                .read_exact_at(&mut new_header, new.record.start)
                .map_err(|error| error.to_string())?;
            old_source
                .read_exact_at(&mut old_header, old.record.start)
                .map_err(|error| error.to_string())?;
            if new_header != old_header {
                return Err("a kept file's record moved or changed".to_owned());
            }
        }
        Ok(())
    }

    pub(crate) fn mimetype_is_first_and_stored(&self) -> bool {
        self.entries.first().is_some_and(|entry| {
            entry.name.as_str() == MIMETYPE
                && entry.method == METHOD_STORED
                && entry.record.start == 0
        })
    }
}

#[cfg(test)]
impl ZipEntry {
    pub(crate) fn raw_central_for_test(&self) -> &[u8] {
        &self.raw_central
    }
}

#[cfg(test)]
pub(crate) fn push_kept_central_for_test(offset: u64) -> Result<(), Error> {
    let mut raw = vec![0; CENTRAL_LEN];
    raw[..4].copy_from_slice(CENTRAL_SIG);
    push_kept_central(&mut Vec::new(), &raw, offset)
}

#[cfg(test)]
mod deflate_tests {
    use super::*;

    #[test]
    fn deflated_entries_inflate_back_byte_equal_for_every_shape_of_input() {
        let mut noise = Vec::with_capacity(300_000);
        let mut state = 0x2545_F491_4F6C_DD1Du64;
        while noise.len() < 300_000 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            noise.extend_from_slice(&state.to_le_bytes());
        }
        let flat = vec![b'a'; 3_000_000];
        let markup = "<p>caf\u{E9} \u{5929}\u{5730}</p>\n".repeat(20_000);
        let samples: [&[u8]; 5] = [b"", b"x", &noise, &flat, markup.as_bytes()];
        for sample in samples {
            let name = EntryName::parse("OEBPS/x.xhtml").unwrap();
            let entry = NewEntry::deflate(name, sample);
            assert_eq!(entry.method, METHOD_DEFLATE);
            assert_eq!(entry.inflated, sample.len() as u64);
            assert_eq!(entry.crc32, crc32fast::hash(sample));
            let back = miniz_oxide::inflate::decompress_to_vec(&entry.body).unwrap();
            assert_eq!(back, sample);
        }
        let repetitive = NewEntry::deflate(EntryName::parse("a").unwrap(), &vec![b'a'; 1_000_000]);
        assert!(repetitive.body.len() < 10_000);
    }
}
