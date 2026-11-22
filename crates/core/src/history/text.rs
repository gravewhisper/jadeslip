use crate::zip::RAW_DEFLATE_LEVEL_6;
use zlib_rs::{Deflate, DeflateFlush, Inflate, InflateFlush, Status};

const PIECE: usize = 8 << 10;
const WINDOW: usize = 24 << 10;
const GROW: usize = 64 << 10;

fn window(base: &[u8], at: usize, len: usize) -> &[u8] {
    let middle = (at as u128 * base.len() as u128 / len.max(1) as u128) as usize;
    let start = middle
        .saturating_sub(PIECE)
        .min(base.len().saturating_sub(WINDOW));
    &base[start..(start + WINDOW).min(base.len())]
}

pub(super) fn squeeze(text: &[u8], base: Option<&[u8]>) -> Option<Vec<u8>> {
    let mut deflate = Deflate::new_with_config(RAW_DEFLATE_LEVEL_6);
    let Some(base) = base else {
        return stream(&mut deflate, text, &[]);
    };
    let mut body = Vec::new();
    for (ix, piece) in text.chunks(PIECE).enumerate() {
        let packed = stream(&mut deflate, piece, window(base, ix * PIECE, text.len()))?;
        body.extend_from_slice(&u32::try_from(packed.len()).ok()?.to_le_bytes());
        body.extend_from_slice(&packed);
    }
    Some(body)
}

fn stream(deflate: &mut Deflate, input: &[u8], dictionary: &[u8]) -> Option<Vec<u8>> {
    deflate.reset();
    if !dictionary.is_empty() {
        deflate.set_dictionary(dictionary).ok()?;
    }
    let mut out = vec![0; zlib_rs::compress_bound(input.len())];
    let status = deflate
        .compress(input, &mut out, DeflateFlush::Finish)
        .ok()?;
    out.truncate(usize::try_from(deflate.total_out()).ok()?);
    (status == Status::StreamEnd).then_some(out)
}

pub(super) fn expand(body: &[u8], base: Option<&[u8]>, len: u64) -> Option<Vec<u8>> {
    let len = usize::try_from(len).ok()?;
    let mut inflate = Inflate::new(false, 15);
    let mut text = Vec::new();
    let Some(base) = base else {
        unstream(&mut inflate, body, &[], len, &mut text)?;
        return Some(text);
    };
    let mut rest = body;
    for at in (0..len).step_by(PIECE) {
        let (size, after) = rest.split_first_chunk::<4>()?;
        let (packed, after) = after.split_at_checked(u32::from_le_bytes(*size) as usize)?;
        let piece = PIECE.min(len - at);
        unstream(
            &mut inflate,
            packed,
            window(base, at, len),
            piece,
            &mut text,
        )?;
        rest = after;
    }
    rest.is_empty().then_some(text)
}

fn unstream(
    inflate: &mut Inflate,
    input: &[u8],
    dictionary: &[u8],
    size: usize,
    out: &mut Vec<u8>,
) -> Option<()> {
    inflate.reset(false);
    if !dictionary.is_empty() {
        inflate.set_dictionary(dictionary).ok()?;
    }
    let end = out.len().checked_add(size)?;
    loop {
        let filled = out.len();
        let room = (end - filled).min(GROW) + 1;
        out.resize(filled + room, 0);
        let (read, wrote) = (inflate.total_in(), inflate.total_out());
        let status = inflate
            .decompress(
                input.get(usize::try_from(read).ok()?..)?,
                &mut out[filled..],
                InflateFlush::NoFlush,
            )
            .ok()?;
        let made = usize::try_from(inflate.total_out() - wrote).ok()?;
        out.truncate(filled + made);
        if out.len() > end {
            return None;
        }
        if status == Status::StreamEnd {
            let whole = inflate.total_in() == input.len() as u64;
            return (whole && out.len() == end).then_some(());
        }
        if made < room {
            return None;
        }
    }
}
