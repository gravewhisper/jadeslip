use crate::rng::Rng;

pub fn page(seed: u64, width: u32, height: u32) -> Vec<u8> {
    let mut rng = Rng::new(seed);
    let mut raw = Vec::with_capacity(((width + 1) * height) as usize);
    let shade = 180 + rng.below(60) as u32;
    for y in 0..height {
        raw.push(0);
        for x in 0..width {
            let band = (x / 16 + y / 16) % 2 * 24;
            raw.push((shade - band) as u8);
        }
    }
    encoded(width, height, GRAY, &raw)
}

const GRAY: u8 = 0;
const RGB: u8 = 2;

pub fn cover(seed: u64, width: u32, height: u32) -> Vec<u8> {
    let mut rng = Rng::new(seed);
    let mut color = || {
        [
            rng.below(256) as i64,
            rng.below(256) as i64,
            rng.below(256) as i64,
        ]
    };
    let (top, bottom, sun) = (color(), color(), color());
    let (w, h) = (i64::from(width), i64::from(height.max(1)));
    let (cx, cy, r) = (w / 2, h * 35 / 100, w * 22 / 100);
    let mut raw = Vec::with_capacity(((width * 3 + 1) * height) as usize);
    for y in 0..h {
        raw.push(0);
        for x in 0..w {
            let inside = (x - cx).pow(2) + (y - cy).pow(2) <= r * r;
            let band = y * 100 / h >= 74 && y * 100 / h < 84;
            for c in 0..3 {
                let sky = top[c] + (bottom[c] - top[c]) * y / h;
                let value = if inside {
                    sun[c]
                } else if band {
                    sky / 3
                } else {
                    sky
                };
                raw.push(value as u8);
            }
        }
    }
    encoded(width, height, RGB, &raw)
}

pub fn claiming(width: u32, height: u32) -> Vec<u8> {
    encoded(width, height, GRAY, &[0, 0, 0])
}

fn encoded(width: u32, height: u32, color: u8, raw: &[u8]) -> Vec<u8> {
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, color, 0, 0, 0]);
    chunk(&mut png, b"IHDR", &ihdr);
    chunk(
        &mut png,
        b"IDAT",
        &miniz_oxide::deflate::compress_to_vec_zlib(raw, 6),
    );
    chunk(&mut png, b"IEND", &[]);
    png
}

fn chunk(png: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    png.extend_from_slice(&(data.len() as u32).to_be_bytes());
    png.extend_from_slice(kind);
    png.extend_from_slice(data);
    let mut hasher = crc32fast::Hasher::new();
    hasher.update(kind);
    hasher.update(data);
    png.extend_from_slice(&hasher.finalize().to_be_bytes());
}
