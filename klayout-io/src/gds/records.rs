//! GDSII record-type constants. Encoded as `(record_type << 8) | data_type`.
//!
//! Reference: GDSII Stream Format Manual, release 6.0 (Calma).

#![allow(dead_code)]

pub const HEADER: u16 = 0x0002; // i16
pub const BGNLIB: u16 = 0x0102; // 12 i16 (mod date + acc date)
pub const LIBNAME: u16 = 0x0206; // string
pub const UNITS: u16 = 0x0305; // 2 f64
pub const ENDLIB: u16 = 0x0400;
pub const BGNSTR: u16 = 0x0502; // 12 i16
pub const STRNAME: u16 = 0x0606; // string
pub const ENDSTR: u16 = 0x0700;
pub const BOUNDARY: u16 = 0x0800;
pub const PATH: u16 = 0x0900;
pub const SREF: u16 = 0x0A00;
pub const AREF: u16 = 0x0B00;
pub const TEXT: u16 = 0x0C00;
pub const LAYER: u16 = 0x0D02;
pub const DATATYPE: u16 = 0x0E02;
pub const WIDTH: u16 = 0x0F03; // i32
pub const XY: u16 = 0x1003; // i32 array
pub const ENDEL: u16 = 0x1100;
pub const SNAME: u16 = 0x1206;
pub const COLROW: u16 = 0x1302; // 2 i16
pub const TEXTNODE: u16 = 0x1400;
pub const NODE: u16 = 0x1500;
pub const TEXTTYPE: u16 = 0x1602;
pub const PRESENTATION: u16 = 0x1701; // bit array
pub const STRING: u16 = 0x1906;
pub const STRANS: u16 = 0x1A01; // bit array
pub const MAG: u16 = 0x1B05; // f64
pub const ANGLE: u16 = 0x1C05; // f64
pub const REFLIBS: u16 = 0x1F06;
pub const FONTS: u16 = 0x2006;
pub const PATHTYPE: u16 = 0x2102;
pub const GENERATIONS: u16 = 0x2202;
pub const ATTRTABLE: u16 = 0x2306;
pub const EFLAGS: u16 = 0x2601;
pub const NODETYPE: u16 = 0x2A02;
pub const PROPATTR: u16 = 0x2B02;
pub const PROPVALUE: u16 = 0x2C06;
pub const BOX_REC: u16 = 0x2D00;
pub const BOXTYPE: u16 = 0x2E02;
pub const PLEX: u16 = 0x2F03;
pub const BGNEXTN: u16 = 0x3003;
pub const ENDEXTN: u16 = 0x3103;

/// Render a record kind for error messages.
pub fn kind_name(kind: u16) -> &'static str {
    match kind & 0xFF00 {
        0x0000 => "HEADER",
        0x0100 => "BGNLIB",
        0x0200 => "LIBNAME",
        0x0300 => "UNITS",
        0x0400 => "ENDLIB",
        0x0500 => "BGNSTR",
        0x0600 => "STRNAME",
        0x0700 => "ENDSTR",
        0x0800 => "BOUNDARY",
        0x0900 => "PATH",
        0x0A00 => "SREF",
        0x0B00 => "AREF",
        0x0C00 => "TEXT",
        0x0D00 => "LAYER",
        0x0E00 => "DATATYPE",
        0x0F00 => "WIDTH",
        0x1000 => "XY",
        0x1100 => "ENDEL",
        0x1200 => "SNAME",
        0x1300 => "COLROW",
        0x1600 => "TEXTTYPE",
        0x1700 => "PRESENTATION",
        0x1900 => "STRING",
        0x1A00 => "STRANS",
        0x1B00 => "MAG",
        0x1C00 => "ANGLE",
        0x2100 => "PATHTYPE",
        0x2A00 => "NODETYPE",
        0x2B00 => "PROPATTR",
        0x2C00 => "PROPVALUE",
        0x2D00 => "BOX",
        0x2E00 => "BOXTYPE",
        0x3000 => "BGNEXTN",
        0x3100 => "ENDEXTN",
        _ => "?",
    }
}
