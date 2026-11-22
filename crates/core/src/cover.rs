use crate::Limits;
use crate::book::Reader;
use crate::package::OpfDoc;
use crate::zip::{EntryName, ReadAt, ZipIndex};
use image::codecs::jpeg::JpegEncoder;
use image::{ImageFormat, ImageReader};
use std::collections::HashMap;
use std::io::Cursor;
use std::panic::{AssertUnwindSafe, catch_unwind};

pub(crate) const MAX_COVER_BYTES: u64 = 16 << 20;
const MAX_SIDE: u32 = 6000;
const MAX_ALLOC: u64 = 128 << 20;
const BOX_WIDTH: u32 = 400;
const BOX_HEIGHT: u32 = 600;
const QUALITY: u8 = 85;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CoverFormat {
    Jpeg,
    Png,
}

impl CoverFormat {
    fn of(media_type: &str) -> Option<CoverFormat> {
        match media_type.trim().to_ascii_lowercase().as_str() {
            "image/jpeg" | "image/jpg" => Some(CoverFormat::Jpeg),
            "image/png" => Some(CoverFormat::Png),
            _ => None,
        }
    }

    fn image_format(self) -> ImageFormat {
        match self {
            CoverFormat::Jpeg => ImageFormat::Jpeg,
            CoverFormat::Png => ImageFormat::Png,
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CoverRef {
    pub entry: EntryName,
    pub format: CoverFormat,
}

pub(crate) fn has_property(properties: &str, wanted: &str) -> bool {
    properties
        .split_ascii_whitespace()
        .any(|property| property == wanted)
}

fn named_cover(id: &str, href: &str) -> bool {
    let file = href.rsplit('/').next().unwrap_or(href);
    [id, file]
        .iter()
        .any(|name| name.to_ascii_lowercase().contains("cover"))
}

pub(crate) fn find_cover(
    opf: &OpfDoc,
    hrefs: &[Option<EntryName>],
    by_id: &HashMap<&str, usize>,
    zip: &ZipIndex,
) -> Option<CoverRef> {
    let usable = |ix: usize| {
        let format = CoverFormat::of(&opf.manifest[ix].media_type)?;
        let entry = hrefs[ix]
            .as_ref()
            .filter(|name| zip.entry(name).is_some())?;
        Some(CoverRef {
            entry: entry.clone(),
            format,
        })
    };
    let items = || opf.manifest.iter().enumerate();
    let by_property = || {
        items()
            .filter(|(_, item)| has_property(&item.properties, "cover-image"))
            .find_map(|(ix, _)| usable(ix))
    };
    let by_meta = || {
        let id = opf.cover_id.as_deref()?;
        usable(*by_id.get(id)?)
    };
    let by_name = || {
        items()
            .filter(|(_, item)| {
                item.media_type
                    .trim()
                    .to_ascii_lowercase()
                    .starts_with("image/")
                    && named_cover(&item.id, &item.href)
            })
            .find_map(|(ix, _)| usable(ix))
    };
    by_property().or_else(by_meta).or_else(by_name)
}

fn shrink(bytes: &[u8], format: CoverFormat) -> Option<Vec<u8>> {
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_SIDE);
    limits.max_image_height = Some(MAX_SIDE);
    limits.max_alloc = Some(MAX_ALLOC);
    let open = || {
        let mut reader = ImageReader::with_format(Cursor::new(bytes), format.image_format());
        reader.limits(limits.clone());
        reader
    };
    let (width, height) = open().into_dimensions().ok()?;
    if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE {
        return None;
    }
    let picture = open().decode().ok()?;
    let picture = if width > BOX_WIDTH || height > BOX_HEIGHT {
        picture.thumbnail(BOX_WIDTH, BOX_HEIGHT)
    } else {
        picture
    };
    let mut out = Vec::new();
    JpegEncoder::new_with_quality(&mut out, QUALITY)
        .encode_image(&picture.into_rgb8())
        .ok()?;
    Some(out)
}

impl<R: ReadAt> Reader<R> {
    pub fn cover_thumbnail(&self) -> Option<Vec<u8>> {
        let cover = self.summary().cover.clone()?;
        let entry = self.zip().entry(&cover.entry)?;
        let limits = Limits {
            max_entry_inflated: self.limits().max_entry_inflated.min(MAX_COVER_BYTES),
            ..self.limits()
        };
        let bytes = self.zip().read_entry(self.source(), entry, &limits).ok()?;
        catch_unwind(AssertUnwindSafe(|| shrink(&bytes, cover.format))).ok()?
    }
}

#[cfg(test)]
mod tests;
