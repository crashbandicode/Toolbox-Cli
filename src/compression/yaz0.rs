//! Yaz0 / Yaz1 run-length LZ codec (the `.szs` container of BOTW and older
//! Nintendo titles), implemented natively from the public format spec — no
//! GPL code consulted.
//!
//! Header (16 bytes, big-endian): `Yaz0`/`Yaz1` magic, decompressed size
//! (`u32`), then an alignment field and a reserved word (both usually 0,
//! preserved as 0 here). The body is a sequence of *groups*: one code byte
//! whose 8 bits (MSB first) each describe the next chunk — a `1` bit is a
//! literal byte, a `0` bit is a back-reference (`u16` of a 12-bit distance
//! and a 4-bit length; length `0` reads one more byte for lengths up to
//! 0x111).
//!
//! Decoding is byte-exact. Encoding is *lossless but not byte-identical* to
//! Nintendo's encoder (match-finding differs); the project's round-trip
//! discipline for compressed data is `decompress(compress(x)) == x`, with
//! unmodified files passed through verbatim at the archive layer.

use crate::error::{Error, Result};

/// Sliding-window size: back-references reach up to 4096 bytes behind.
const WINDOW: usize = 0x1000;
/// Longest encodable match (4-bit length 0 + extra byte: `0xFF + 0x12`).
const MAX_MATCH: usize = 0x111;
/// Shortest match worth encoding (a back-reference costs two bytes).
const MIN_MATCH: usize = 3;
const HASH_BITS: usize = 15;
const HASH_SIZE: usize = 1 << HASH_BITS;
/// Cap on hash-chain traversal per position (bounds encode time).
const MAX_CHAIN: usize = 128;
const NIL: i32 = -1;
const HEADER_SIZE: usize = 0x10;

/// True if `bytes` begins with a Yaz0 or Yaz1 header.
pub fn is_yaz(bytes: &[u8]) -> bool {
    bytes.len() >= 4 && (&bytes[0..4] == b"Yaz0" || &bytes[0..4] == b"Yaz1")
}

/// Decompress a Yaz0/Yaz1 stream. The output length is taken from the
/// header's decompressed-size field; every input access is bounds-checked.
pub fn decompress(data: &[u8]) -> Result<Vec<u8>> {
    if data.len() < HEADER_SIZE || !is_yaz(data) {
        return Err(Error::Compression(
            "not a Yaz0/Yaz1 stream (bad magic or truncated header)".into(),
        ));
    }
    let dec_size = u32::from_be_bytes([data[4], data[5], data[6], data[7]]) as usize;
    let mut out = Vec::with_capacity(dec_size);

    let mut src = HEADER_SIZE;
    while out.len() < dec_size {
        let code = *data
            .get(src)
            .ok_or_else(|| Error::Compression("Yaz0: truncated before group code".into()))?;
        src += 1;

        for bit in 0..8 {
            if out.len() >= dec_size {
                break;
            }
            if code & (0x80 >> bit) != 0 {
                // Literal byte.
                let b = *data
                    .get(src)
                    .ok_or_else(|| Error::Compression("Yaz0: truncated literal".into()))?;
                out.push(b);
                src += 1;
            } else {
                // Back-reference: 12-bit distance + length.
                let hi = *data
                    .get(src)
                    .ok_or_else(|| Error::Compression("Yaz0: truncated back-ref".into()))?;
                let lo = *data
                    .get(src + 1)
                    .ok_or_else(|| Error::Compression("Yaz0: truncated back-ref".into()))?;
                src += 2;
                let word = ((hi as usize) << 8) | lo as usize;
                let dist = (word & 0x0FFF) + 1;
                let mut count = word >> 12;
                if count == 0 {
                    let ext = *data
                        .get(src)
                        .ok_or_else(|| Error::Compression("Yaz0: truncated length byte".into()))?;
                    src += 1;
                    count = ext as usize + 0x12;
                } else {
                    count += 2;
                }
                if dist > out.len() {
                    return Err(Error::Compression(format!(
                        "Yaz0: back-reference distance {dist} precedes output start ({})",
                        out.len()
                    )));
                }
                // Copy byte-by-byte so overlapping runs (RLE) expand
                // correctly: when `count > dist`, `out[start + i]` reads
                // bytes written earlier in this same loop.
                let start = out.len() - dist;
                for i in 0..count {
                    if out.len() >= dec_size {
                        break;
                    }
                    let b = out[start + i];
                    out.push(b);
                }
            }
        }
    }
    Ok(out)
}

/// Compress `data` into a Yaz0 stream (lossless; ratio is secondary). Uses a
/// hash-chained greedy longest-match search within the 4096-byte window.
pub fn compress(data: &[u8]) -> Vec<u8> {
    let n = data.len();
    let mut out = Vec::with_capacity(n / 2 + HEADER_SIZE);
    out.extend_from_slice(b"Yaz0");
    out.extend_from_slice(&(n as u32).to_be_bytes());
    out.extend_from_slice(&[0u8; 8]); // alignment + reserved

    let mut head = vec![NIL; HASH_SIZE];
    let mut prev = vec![NIL; n.max(1)];

    let mut pos = 0usize;
    let mut code_byte = 0u8;
    let mut group: Vec<u8> = Vec::with_capacity(8 * 3);
    let mut ops = 0u8;

    while pos < n {
        let (best_len, best_dist) = find_match(data, pos, &head, &prev);
        if best_len >= MIN_MATCH {
            let dist_field = (best_dist - 1) as u16; // 0..=0x0FFF
            if best_len < 0x12 {
                let word = (((best_len - 2) as u16) << 12) | dist_field;
                group.extend_from_slice(&word.to_be_bytes());
            } else {
                group.extend_from_slice(&dist_field.to_be_bytes());
                group.push((best_len - 0x12) as u8);
            }
            let end = pos + best_len;
            while pos < end {
                insert(data, pos, &mut head, &mut prev);
                pos += 1;
            }
        } else {
            code_byte |= 0x80u8 >> ops; // literal bit
            group.push(data[pos]);
            insert(data, pos, &mut head, &mut prev);
            pos += 1;
        }
        ops += 1;
        if ops == 8 {
            out.push(code_byte);
            out.extend_from_slice(&group);
            code_byte = 0;
            group.clear();
            ops = 0;
        }
    }
    if ops > 0 {
        out.push(code_byte);
        out.extend_from_slice(&group);
    }
    out
}

#[inline]
fn hash3(d: &[u8], p: usize) -> usize {
    let v = ((d[p] as u32) << 16) | ((d[p + 1] as u32) << 8) | (d[p + 2] as u32);
    ((v.wrapping_mul(0x9E37_79B1)) >> (32 - HASH_BITS)) as usize & (HASH_SIZE - 1)
}

#[inline]
fn insert(d: &[u8], pos: usize, head: &mut [i32], prev: &mut [i32]) {
    if pos + MIN_MATCH <= d.len() {
        let h = hash3(d, pos);
        prev[pos] = head[h];
        head[h] = pos as i32;
    }
}

/// Greedy longest match for `pos` within the window. Returns `(len, dist)`
/// with `len < MIN_MATCH` meaning "no usable match".
fn find_match(d: &[u8], pos: usize, head: &[i32], prev: &[i32]) -> (usize, usize) {
    let n = d.len();
    if pos + MIN_MATCH > n {
        return (0, 0);
    }
    let max_len = (n - pos).min(MAX_MATCH);
    let limit = pos.saturating_sub(WINDOW);
    let mut cand = head[hash3(d, pos)];
    let mut chain = MAX_CHAIN;
    let mut best_len = 0usize;
    let mut best_dist = 0usize;

    while cand != NIL && chain > 0 {
        let c = cand as usize;
        if c < limit {
            break;
        }
        // Skip candidates that can't extend past the current best (the
        // byte one past the best match must at least match).
        if best_len == 0 || d[c + best_len] == d[pos + best_len] {
            let mut l = 0usize;
            while l < max_len && d[c + l] == d[pos + l] {
                l += 1;
            }
            if l > best_len {
                best_len = l;
                best_dist = pos - c;
                if l >= max_len {
                    break;
                }
            }
        }
        cand = prev[c];
        chain -= 1;
    }
    (best_len, best_dist)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(data: &[u8]) {
        let packed = compress(data);
        assert_eq!(&packed[0..4], b"Yaz0");
        assert_eq!(
            u32::from_be_bytes([packed[4], packed[5], packed[6], packed[7]]) as usize,
            data.len(),
            "header size field"
        );
        let back = decompress(&packed).expect("decompress");
        assert_eq!(back, data, "round-trip mismatch (len {})", data.len());
    }

    #[test]
    fn empty() {
        round_trip(&[]);
    }

    #[test]
    fn short_literal() {
        round_trip(b"hi");
    }

    #[test]
    fn highly_repetitive_runs() {
        // Exercises overlapping back-references (RLE) and the long-match
        // extra-length-byte path.
        round_trip(&[0x5Au8; 5000]);
        round_trip(b"abcabcabcabcabcabcabcabcabcabcabcabcabcabc");
    }

    #[test]
    fn mixed_and_pseudo_random() {
        let mut v = Vec::new();
        let mut x = 0x1234_5678u32;
        for i in 0..4096 {
            // Deterministic pseudo-random with embedded repeats.
            x = x.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            v.push((x >> 16) as u8);
            if i % 64 == 0 {
                v.extend_from_slice(b"REPEATED_PATTERN_REPEATED_PATTERN");
            }
        }
        round_trip(&v);
    }

    #[test]
    fn text_like() {
        let s = "The quick brown fox jumps over the lazy dog. ".repeat(100);
        round_trip(s.as_bytes());
    }

    #[test]
    fn yaz1_magic_decodes() {
        // Build a Yaz1 stream by hand: 3 literal bytes.
        let mut packed = Vec::new();
        packed.extend_from_slice(b"Yaz1");
        packed.extend_from_slice(&3u32.to_be_bytes());
        packed.extend_from_slice(&[0u8; 8]);
        packed.push(0b1110_0000); // 3 literal bits
        packed.extend_from_slice(b"abc");
        assert_eq!(decompress(&packed).unwrap(), b"abc");
    }

    #[test]
    fn rejects_bad_magic() {
        assert!(decompress(b"NOPE........................").is_err());
        assert!(decompress(&[]).is_err());
    }

    #[test]
    fn detects_truncation() {
        let packed = compress(&[7u8; 200]);
        // Lop off the tail; decompression must error, not panic.
        assert!(decompress(&packed[..packed.len() - 3]).is_err());
    }
}
