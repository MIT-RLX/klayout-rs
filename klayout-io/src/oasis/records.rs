//! OASIS record codes (single byte after START). Reference: SEMI P39 §11.
//!
//! Only the records exercised by the v1 reader/writer are listed; the rest
//! are parsed defensively (decoded length, payload skipped) when seen.

#![allow(dead_code)]

pub const PAD: u8 = 0;
pub const START: u8 = 1;
pub const END: u8 = 2;
pub const CELLNAME: u8 = 3;
pub const CELLNAME_REF: u8 = 4;
pub const TEXTSTRING: u8 = 5;
pub const TEXTSTRING_REF: u8 = 6;
pub const PROPNAME: u8 = 7;
pub const PROPNAME_REF: u8 = 8;
pub const PROPSTRING: u8 = 9;
pub const PROPSTRING_REF: u8 = 10;
pub const LAYERNAME_DATA: u8 = 11;
pub const LAYERNAME_TEXT: u8 = 12;
pub const CELL_REF: u8 = 13;
pub const CELL: u8 = 14;
pub const XYABSOLUTE: u8 = 15;
pub const XYRELATIVE: u8 = 16;
pub const PLACEMENT: u8 = 17;
pub const PLACEMENT_TRANSFORM: u8 = 18;
pub const TEXT: u8 = 19;
pub const RECTANGLE: u8 = 20;
pub const POLYGON: u8 = 21;
pub const PATH: u8 = 22;
pub const TRAPEZOID: u8 = 23;
pub const TRAPEZOID_A: u8 = 24;
pub const TRAPEZOID_B: u8 = 25;
pub const CTRAPEZOID: u8 = 26;
pub const CIRCLE: u8 = 27;
pub const PROPERTY: u8 = 28;
pub const PROPERTY_LAST: u8 = 29;
pub const XNAME_REF: u8 = 30;
pub const XNAME: u8 = 31;
pub const XELEMENT: u8 = 32;
pub const XGEOMETRY: u8 = 33;
pub const CBLOCK: u8 = 34;

pub const MAGIC: &[u8] = b"%SEMI-OASIS\r\n";
