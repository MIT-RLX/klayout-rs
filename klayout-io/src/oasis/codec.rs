//! OASIS variable-length integer + real codecs.
//!
//! Reference: SEMI P39-1109 OASIS Stream Format, section 7.2.
//!
//! * `unsigned-integer` — base-128 little-endian; bit 7 is the continuation
//!   flag, bits 6..0 are 7 payload bits.
//! * `signed-integer` — same encoding, but the LSB of the decoded value is
//!   the sign bit and the upper bits are the magnitude.
//! * `real` — encoded as a type byte + payload, with several formats
//!   (positive integer, negative integer, ratio, IEEE float32/float64).

use crate::error::{IoError, Result};

pub fn encode_unsigned(v: u64, out: &mut Vec<u8>) {
    let mut v = v;
    loop {
        let byte = (v & 0x7F) as u8;
        v >>= 7;
        if v == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

pub fn decode_unsigned(r: &mut &[u8]) -> Result<u64> {
    let mut v: u64 = 0;
    let mut shift: u32 = 0;
    loop {
        if r.is_empty() {
            return Err(IoError::UnexpectedEof);
        }
        let byte = r[0];
        *r = &r[1..];
        v |= ((byte & 0x7F) as u64) << shift;
        if (byte & 0x80) == 0 {
            return Ok(v);
        }
        shift += 7;
        if shift >= 64 {
            return Err(IoError::OasisVarIntOverflow);
        }
    }
}

pub fn encode_signed(v: i64, out: &mut Vec<u8>) {
    let (mag, sign) = if v < 0 {
        (v.unsigned_abs(), 1u64)
    } else {
        (v as u64, 0u64)
    };
    let combined = (mag << 1) | sign;
    encode_unsigned(combined, out);
}

pub fn decode_signed(r: &mut &[u8]) -> Result<i64> {
    let combined = decode_unsigned(r)?;
    let sign = combined & 1;
    let mag = (combined >> 1) as i64;
    Ok(if sign == 1 { -mag } else { mag })
}

/// OASIS real number. The spec defines 8 type codes; v1 supports the
/// integer-friendly variants and IEEE float64.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum Real {
    PositiveWhole(u64),
    NegativeWhole(u64),
    PositiveRatio { num: u64, den: u64 },
    NegativeRatio { num: u64, den: u64 },
    Float32(f32),
    Float64(f64),
}

impl Real {
    pub fn to_f64(self) -> f64 {
        match self {
            Real::PositiveWhole(v) => v as f64,
            Real::NegativeWhole(v) => -(v as f64),
            Real::PositiveRatio { num, den } => num as f64 / den as f64,
            Real::NegativeRatio { num, den } => -(num as f64 / den as f64),
            Real::Float32(f) => f as f64,
            Real::Float64(f) => f,
        }
    }
}

pub fn encode_real(r: Real, out: &mut Vec<u8>) {
    match r {
        Real::PositiveWhole(v) => {
            out.push(0);
            encode_unsigned(v, out);
        }
        Real::NegativeWhole(v) => {
            out.push(1);
            encode_unsigned(v, out);
        }
        Real::PositiveRatio { num, den } => {
            out.push(2);
            encode_unsigned(num, out);
            encode_unsigned(den, out);
        }
        Real::NegativeRatio { num, den } => {
            out.push(3);
            encode_unsigned(num, out);
            encode_unsigned(den, out);
        }
        Real::Float32(f) => {
            out.push(6);
            out.extend_from_slice(&f.to_le_bytes());
        }
        Real::Float64(f) => {
            out.push(7);
            out.extend_from_slice(&f.to_le_bytes());
        }
    }
}

pub fn decode_real(r: &mut &[u8]) -> Result<Real> {
    if r.is_empty() {
        return Err(IoError::UnexpectedEof);
    }
    let kind = r[0];
    *r = &r[1..];
    match kind {
        0 => Ok(Real::PositiveWhole(decode_unsigned(r)?)),
        1 => Ok(Real::NegativeWhole(decode_unsigned(r)?)),
        2 => {
            let num = decode_unsigned(r)?;
            let den = decode_unsigned(r)?;
            Ok(Real::PositiveRatio { num, den })
        }
        3 => {
            let num = decode_unsigned(r)?;
            let den = decode_unsigned(r)?;
            Ok(Real::NegativeRatio { num, den })
        }
        6 => {
            if r.len() < 4 {
                return Err(IoError::UnexpectedEof);
            }
            let mut a = [0u8; 4];
            a.copy_from_slice(&r[..4]);
            *r = &r[4..];
            Ok(Real::Float32(f32::from_le_bytes(a)))
        }
        7 => {
            if r.len() < 8 {
                return Err(IoError::UnexpectedEof);
            }
            let mut a = [0u8; 8];
            a.copy_from_slice(&r[..8]);
            *r = &r[8..];
            Ok(Real::Float64(f64::from_le_bytes(a)))
        }
        other => Err(IoError::OasisUnsupportedReal(other)),
    }
}

pub fn encode_string(s: &str, out: &mut Vec<u8>) {
    let bytes = s.as_bytes();
    encode_unsigned(bytes.len() as u64, out);
    out.extend_from_slice(bytes);
}

pub fn decode_string(r: &mut &[u8]) -> Result<String> {
    let len = decode_unsigned(r)? as usize;
    if r.len() < len {
        return Err(IoError::UnexpectedEof);
    }
    let s = std::str::from_utf8(&r[..len])
        .map_err(|_| IoError::OasisInvalidUtf8)?
        .to_string();
    *r = &r[len..];
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rt_unsigned(v: u64) {
        let mut buf = Vec::new();
        encode_unsigned(v, &mut buf);
        let mut s = buf.as_slice();
        let got = decode_unsigned(&mut s).unwrap();
        assert_eq!(got, v);
        assert!(s.is_empty(), "leftover bytes after decode");
    }

    fn rt_signed(v: i64) {
        let mut buf = Vec::new();
        encode_signed(v, &mut buf);
        let mut s = buf.as_slice();
        let got = decode_signed(&mut s).unwrap();
        assert_eq!(got, v);
        assert!(s.is_empty());
    }

    #[test]
    fn unsigned_roundtrip() {
        for v in [0, 1, 127, 128, 255, 256, 16383, 16384, u64::MAX] {
            rt_unsigned(v);
        }
    }

    #[test]
    fn signed_roundtrip() {
        for v in [
            0,
            1,
            -1,
            127,
            -127,
            128,
            -128,
            16384,
            -16384,
            i64::MAX,
            i64::MIN + 1, // i64::MIN abs would overflow
        ] {
            rt_signed(v);
        }
    }

    #[test]
    fn unsigned_known_encodings() {
        let mut buf = Vec::new();
        encode_unsigned(127, &mut buf);
        assert_eq!(buf, vec![0x7F]);

        buf.clear();
        encode_unsigned(128, &mut buf);
        assert_eq!(buf, vec![0x80, 0x01]);

        buf.clear();
        encode_unsigned(16384, &mut buf);
        assert_eq!(buf, vec![0x80, 0x80, 0x01]);
    }

    #[test]
    fn real_roundtrip() {
        for r in [
            Real::PositiveWhole(1000),
            Real::NegativeWhole(42),
            Real::PositiveRatio { num: 1, den: 1000 },
            Real::Float64(0.001),
            Real::Float32(1.5),
        ] {
            let mut buf = Vec::new();
            encode_real(r, &mut buf);
            let mut s = buf.as_slice();
            let got = decode_real(&mut s).unwrap();
            assert_eq!(got, r);
        }
    }

    #[test]
    fn string_roundtrip() {
        for s in ["", "hello", "with unicode: π", "longer string of text"] {
            let mut buf = Vec::new();
            encode_string(s, &mut buf);
            let mut sl = buf.as_slice();
            let got = decode_string(&mut sl).unwrap();
            assert_eq!(got, s);
        }
    }
}
