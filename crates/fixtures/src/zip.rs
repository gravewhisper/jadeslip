use std::fs::File;
use std::io::{self, BufWriter, Write};

pub const STORED: u16 = 0;
pub const DEFLATE: u16 = 8;
const DOS_TIME: u16 = 0;
const DOS_DATE: u16 = 0x0021;
const UTF8_FLAG: u16 = 0x0800;

#[derive(Clone)]
pub struct Record {
    pub name: Vec<u8>,
    pub method: u16,
    pub crc32: u32,
    pub compressed: u64,
    pub inflated: u64,
    pub offset: u64,
}

impl Record {
    pub fn honest(name: &[u8], method: u16, crc32: u32, compressed: u64, inflated: u64) -> Self {
        Record {
            name: name.to_vec(),
            method,
            crc32,
            compressed,
            inflated,
            offset: 0,
        }
    }
}

pub fn deflate(data: &[u8]) -> Vec<u8> {
    miniz_oxide::deflate::compress_to_vec(data, 6)
}

pub fn deflate_max(data: &[u8]) -> Vec<u8> {
    miniz_oxide::deflate::compress_to_vec(data, 9)
}

pub struct ZipWriter {
    out: BufWriter<File>,
    pos: u64,
    central: Vec<Record>,
}

impl ZipWriter {
    pub fn new(file: File) -> Self {
        ZipWriter {
            out: BufWriter::with_capacity(1 << 20, file),
            pos: 0,
            central: Vec::new(),
        }
    }

    pub fn add(&mut self, name: &[u8], data: &[u8], method: u16) -> io::Result<()> {
        let crc = crc32fast::hash(data);
        let body = match method {
            STORED => None,
            _ => Some(deflate(data)),
        };
        let bytes = body.as_deref().unwrap_or(data);
        let record = Record::honest(name, method, crc, bytes.len() as u64, data.len() as u64);
        let record = self.write_local(record, bytes)?;
        self.push_central(record);
        Ok(())
    }

    pub fn write_local(&mut self, mut record: Record, body: &[u8]) -> io::Result<Record> {
        record.offset = self.pos;
        let header = local_header(&record);
        self.write(&header)?;
        self.write(body)?;
        Ok(record)
    }

    pub fn push_central(&mut self, record: Record) {
        self.central.push(record);
    }

    pub fn finish(mut self) -> io::Result<()> {
        let cd_start = self.pos;
        let records = std::mem::take(&mut self.central);
        for record in &records {
            let header = central_header(record);
            self.write(&header)?;
        }
        let cd_len = self.pos - cd_start;
        let count = records.len() as u64;
        if count >= u64::from(u16::MAX) {
            let eocd64_at = self.pos;
            let mut eocd64 = Vec::with_capacity(76);
            eocd64.extend_from_slice(b"PK\x06\x06");
            eocd64.extend_from_slice(&44u64.to_le_bytes());
            eocd64.extend_from_slice(&45u16.to_le_bytes());
            eocd64.extend_from_slice(&45u16.to_le_bytes());
            eocd64.extend_from_slice(&[0u8; 8]);
            eocd64.extend_from_slice(&count.to_le_bytes());
            eocd64.extend_from_slice(&count.to_le_bytes());
            eocd64.extend_from_slice(&cd_len.to_le_bytes());
            eocd64.extend_from_slice(&cd_start.to_le_bytes());
            eocd64.extend_from_slice(b"PK\x06\x07");
            eocd64.extend_from_slice(&0u32.to_le_bytes());
            eocd64.extend_from_slice(&eocd64_at.to_le_bytes());
            eocd64.extend_from_slice(&1u32.to_le_bytes());
            self.write(&eocd64)?;
        }
        let short_count = count.min(u64::from(u16::MAX)) as u16;
        let mut eocd = Vec::with_capacity(22);
        eocd.extend_from_slice(b"PK\x05\x06");
        eocd.extend_from_slice(&[0u8; 4]);
        eocd.extend_from_slice(&short_count.to_le_bytes());
        eocd.extend_from_slice(&short_count.to_le_bytes());
        eocd.extend_from_slice(&(cd_len as u32).to_le_bytes());
        eocd.extend_from_slice(&(cd_start as u32).to_le_bytes());
        eocd.extend_from_slice(&0u16.to_le_bytes());
        self.write(&eocd)?;
        self.out.flush()
    }

    fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.out.write_all(bytes)?;
        self.pos += bytes.len() as u64;
        Ok(())
    }
}

pub fn local_header(record: &Record) -> Vec<u8> {
    let mut h = Vec::with_capacity(30 + record.name.len());
    h.extend_from_slice(b"PK\x03\x04");
    h.extend_from_slice(&20u16.to_le_bytes());
    h.extend_from_slice(&UTF8_FLAG.to_le_bytes());
    h.extend_from_slice(&record.method.to_le_bytes());
    h.extend_from_slice(&DOS_TIME.to_le_bytes());
    h.extend_from_slice(&DOS_DATE.to_le_bytes());
    h.extend_from_slice(&record.crc32.to_le_bytes());
    h.extend_from_slice(&(record.compressed as u32).to_le_bytes());
    h.extend_from_slice(&(record.inflated as u32).to_le_bytes());
    h.extend_from_slice(&(record.name.len() as u16).to_le_bytes());
    h.extend_from_slice(&0u16.to_le_bytes());
    h.extend_from_slice(&record.name);
    h
}

fn central_header(record: &Record) -> Vec<u8> {
    let mut h = Vec::with_capacity(46 + record.name.len());
    h.extend_from_slice(b"PK\x01\x02");
    h.extend_from_slice(&20u16.to_le_bytes());
    h.extend_from_slice(&20u16.to_le_bytes());
    h.extend_from_slice(&UTF8_FLAG.to_le_bytes());
    h.extend_from_slice(&record.method.to_le_bytes());
    h.extend_from_slice(&DOS_TIME.to_le_bytes());
    h.extend_from_slice(&DOS_DATE.to_le_bytes());
    h.extend_from_slice(&record.crc32.to_le_bytes());
    h.extend_from_slice(&(record.compressed as u32).to_le_bytes());
    h.extend_from_slice(&(record.inflated as u32).to_le_bytes());
    h.extend_from_slice(&(record.name.len() as u16).to_le_bytes());
    h.extend_from_slice(&[0u8; 12]);
    h.extend_from_slice(&(record.offset as u32).to_le_bytes());
    h.extend_from_slice(&record.name);
    h
}
