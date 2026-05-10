//! Byte-level GDSII codec: record framing, primitive types, GDS f64.
//!
//! GDSII uses big-endian byte order and a 64-bit hex-base float format with a
//! 7-bit excess-64 exponent and 56-bit mantissa. Conversion between this
//! format and IEEE-754 f64 is exact within the float's representable range.

use super::super::error::{IoError, Result};

/// Header pulled from a record: combined `(record_type << 8) | data_type`.
#[derive(Copy, Clone, Debug)]
pub struct RecordHeader {
    pub kind: u16,
    /// Total record length in bytes (header + data, padded). Always even.
    pub total_len: u16,
}

impl RecordHeader {
    pub fn data_len(self) -> usize {
        self.total_len as usize - 4
    }
}

pub struct ByteReader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> ByteReader<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    pub fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.pos)
    }

    pub fn is_eof(&self) -> bool {
        self.pos >= self.bytes.len()
    }

    pub fn pos(&self) -> usize {
        self.pos
    }

    pub fn seek(&mut self, p: usize) {
        self.pos = p.min(self.bytes.len());
    }

    pub fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        if self.pos + n > self.bytes.len() {
            return Err(IoError::UnexpectedEof);
        }
        let s = &self.bytes[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }

    pub fn read_u16(&mut self) -> Result<u16> {
        let b = self.take(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }

    pub fn read_record_header(&mut self) -> Result<Option<RecordHeader>> {
        if self.is_eof() {
            return Ok(None);
        }
        let total_len = self.read_u16()?;
        if total_len < 4 || total_len % 2 != 0 {
            return Err(IoError::InvalidRecordLength(total_len));
        }
        let kind = self.read_u16()?;
        Ok(Some(RecordHeader { kind, total_len }))
    }
}

pub fn parse_i16_array(data: &[u8]) -> Vec<i16> {
    data.chunks_exact(2)
        .map(|c| i16::from_be_bytes([c[0], c[1]]))
        .collect()
}

pub fn parse_i32_array(data: &[u8]) -> Vec<i32> {
    data.chunks_exact(4)
        .map(|c| i32::from_be_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

pub fn parse_f64_array(data: &[u8]) -> Vec<f64> {
    data.chunks_exact(8)
        .map(|c| {
            let mut a = [0u8; 8];
            a.copy_from_slice(c);
            gds_f64_decode(a)
        })
        .collect()
}

pub fn parse_string(data: &[u8]) -> String {
    let end = data.iter().rposition(|&b| b != 0).map(|p| p + 1).unwrap_or(0);
    String::from_utf8_lossy(&data[..end]).into_owned()
}

// ---------- GDS f64 codec ----------

/// Decode an 8-byte GDSII real to native f64.
///
/// Format (big-endian): sign(1) | excess-64 exponent(7) | mantissa(56).
/// `value = (-1)^sign * (mantissa / 2^56) * 16^(exponent - 64)`.
pub fn gds_f64_decode(bytes: [u8; 8]) -> f64 {
    let v = u64::from_be_bytes(bytes);
    let sign = (v >> 63) & 1;
    let exp_biased = ((v >> 56) & 0x7F) as i32;
    let mantissa = v & 0x00FF_FFFF_FFFF_FFFF;
    if mantissa == 0 && exp_biased == 0 {
        return 0.0;
    }
    let exp = exp_biased - 64;
    let m = (mantissa as f64) / ((1u64 << 56) as f64);
    let val = m * 16f64.powi(exp);
    if sign == 1 {
        -val
    } else {
        val
    }
}

/// Encode a native f64 as 8-byte GDSII real.
pub fn gds_f64_encode(v: f64) -> [u8; 8] {
    if v == 0.0 || !v.is_finite() {
        return [0u8; 8];
    }
    let sign: u64 = if v.is_sign_negative() { 1 } else { 0 };
    let mut a = v.abs();
    let mut exp: i32 = 0;
    while a >= 1.0 {
        a /= 16.0;
        exp += 1;
    }
    while a < 1.0 / 16.0 {
        a *= 16.0;
        exp -= 1;
    }
    // a in [1/16, 1)
    let mut m = (a * ((1u64 << 56) as f64)).round() as u64;
    if m >= (1u64 << 56) {
        m >>= 4;
        exp += 1;
    }
    let biased = ((exp + 64).clamp(0, 0x7F)) as u64;
    let bits = (sign << 63) | (biased << 56) | (m & 0x00FF_FFFF_FFFF_FFFF);
    bits.to_be_bytes()
}

// ---------- Writers ----------

pub fn write_record(buf: &mut Vec<u8>, kind: u16, data: &[u8]) {
    let pad = data.len() % 2;
    let total = 4 + data.len() + pad;
    assert!(total <= 0xFFFF, "GDS record exceeds 65535 bytes");
    buf.extend_from_slice(&(total as u16).to_be_bytes());
    buf.extend_from_slice(&kind.to_be_bytes());
    buf.extend_from_slice(data);
    if pad == 1 {
        buf.push(0);
    }
}

pub fn write_record_i16(buf: &mut Vec<u8>, kind: u16, vals: &[i16]) {
    let mut data = Vec::with_capacity(vals.len() * 2);
    for v in vals {
        data.extend_from_slice(&v.to_be_bytes());
    }
    write_record(buf, kind, &data);
}

pub fn write_record_i32(buf: &mut Vec<u8>, kind: u16, vals: &[i32]) {
    let mut data = Vec::with_capacity(vals.len() * 4);
    for v in vals {
        data.extend_from_slice(&v.to_be_bytes());
    }
    write_record(buf, kind, &data);
}

pub fn write_record_f64(buf: &mut Vec<u8>, kind: u16, vals: &[f64]) {
    let mut data = Vec::with_capacity(vals.len() * 8);
    for v in vals {
        data.extend_from_slice(&gds_f64_encode(*v));
    }
    write_record(buf, kind, &data);
}

pub fn write_record_str(buf: &mut Vec<u8>, kind: u16, s: &str) {
    write_record(buf, kind, s.as_bytes());
}

pub fn write_record_bits(buf: &mut Vec<u8>, kind: u16, bits: u16) {
    write_record(buf, kind, &bits.to_be_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn f64_roundtrip_specials() {
        // Common DBU values
        for v in [0.0, 1e-3, 1e-9, 1e-6, 1.0, -1.0, 1e-12] {
            let enc = gds_f64_encode(v);
            let dec = gds_f64_decode(enc);
            let rel_err = if v == 0.0 { dec.abs() } else { (dec - v).abs() / v.abs() };
            assert!(rel_err < 1e-15, "v={v} dec={dec}");
        }
    }

    #[test]
    fn f64_roundtrip_random() {
        // Pseudo-random sweep across exponents.
        for i in -50..50 {
            let v = 1.234567 * 10f64.powi(i);
            let dec = gds_f64_decode(gds_f64_encode(v));
            let rel_err = (dec - v).abs() / v.abs();
            assert!(rel_err < 1e-15, "v={v} dec={dec} err={rel_err}");
        }
    }

    #[test]
    fn record_framing() {
        let mut buf = Vec::new();
        write_record_i16(&mut buf, 0x0102, &[1, 2, 3, 4, 5, 6]);
        // total length = 4 header + 12 bytes = 16
        assert_eq!(&buf[..2], &[0x00, 0x10]);
        assert_eq!(&buf[2..4], &[0x01, 0x02]);
        assert_eq!(buf.len(), 16);
    }

    #[test]
    fn odd_data_padded() {
        let mut buf = Vec::new();
        write_record_str(&mut buf, 0x0606, "abc"); // 3 bytes -> pad to 4
        assert_eq!(&buf[..2], &[0x00, 0x08]);
        assert_eq!(buf.len(), 8);
        assert_eq!(buf[7], 0);
    }
}
