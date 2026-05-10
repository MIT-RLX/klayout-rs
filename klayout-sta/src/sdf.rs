//! SDF (Standard Delay Format) reader and back-annotation.
//!
//! SDF is the ASCII s-expression format that post-route flows emit to
//! capture per-instance, per-arc, per-corner delays after layout
//! parasitics are extracted. Reading it lets a downstream STA pass
//! consume those delays directly (rather than recomputing from
//! Liberty + SPEF — which is the design intent but not always
//! reliable across tool versions).
//!
//! ## Coverage
//!
//! v1 covers the **common case** that drives ~99% of real flows:
//!
//! ```text
//! (DELAYFILE
//!   (SDFVERSION "OVI 3.0")
//!   (DESIGN "top")
//!   (TIMESCALE 1ns)
//!   (CELL
//!     (CELLTYPE "AND2X1")
//!     (INSTANCE u_top/u_inst)
//!     (DELAY (ABSOLUTE
//!       (IOPATH A Y (0.123:0.123:0.123) (0.135:0.135:0.135))
//!       (IOPATH B Y (0.118:0.118:0.118) (0.130:0.130:0.130))
//!     ))
//!   )
//!   ...
//! )
//! ```
//!
//! Each `IOPATH` carries a rising and a falling delay triplet
//! `(min:typ:max)`. The reader normalizes triplets into a typed
//! [`Triplet`] struct; back-annotation maps them onto the
//! [`TimingGraph`]'s `CellArc` edges by `(instance, input_pin →
//! output_pin)` lookup.
//!
//! ## What's not in v1
//!
//! * **`(COND ...)` and `(CONDELSE ...)`** — conditional delays.
//! * **`(CHECK ...)` constraints** — setup/hold check timings.
//! * **`(NEGEDGE / POSEDGE ...)`** — clock-edge-conditional arcs.
//! * **Interconnect delays** (`PORT`, `INTERCONNECT`, `NETDELAY`).
//! * **`INCREMENTAL`** delays (we treat all SDF as `ABSOLUTE`).
//!
//! These don't affect read tolerance — unknown blocks are skipped —
//! but the back-annotation pass ignores them. A v2 needs proper
//! handling, especially for hold-check sign-off.

use crate::graph::{Edge, EdgeKind, NodeId, TimingGraph};
use smol_str::SmolStr;
use std::collections::HashMap;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SdfError {
    #[error("sdf: unexpected end of input")]
    UnexpectedEof,

    #[error("sdf: parse error at byte {pos}: {message}")]
    Parse { pos: usize, message: String },

    #[error("sdf: invalid number `{raw}`")]
    InvalidNumber { raw: String },
}

pub type Result<T> = std::result::Result<T, SdfError>;

/// `(min:typ:max)` triplet. SDF allows two-element shorthand
/// (`(typ:max)`) and single-element (`(typ)`); we widen them to
/// triplets at parse time.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Triplet {
    pub min: f64,
    pub typ: f64,
    pub max: f64,
}

impl Triplet {
    pub fn from_typ(t: f64) -> Self {
        Self {
            min: t,
            typ: t,
            max: t,
        }
    }
}

#[derive(Clone, Debug)]
pub struct SdfFile {
    pub design: Option<SmolStr>,
    pub sdf_version: Option<SmolStr>,
    pub timescale: Option<SmolStr>,
    pub cells: Vec<SdfCell>,
}

#[derive(Clone, Debug)]
pub struct SdfCell {
    pub cell_type: SmolStr,
    pub instance: SmolStr,
    pub iopaths: Vec<SdfIoPath>,
}

#[derive(Clone, Debug)]
pub struct SdfIoPath {
    pub from_pin: SmolStr,
    pub to_pin: SmolStr,
    pub rise: Triplet,
    pub fall: Triplet,
}

// ---------- Tokenizer + parser ----------

struct Tokenizer<'a> {
    src: &'a [u8],
    pos: usize,
}

impl<'a> Tokenizer<'a> {
    fn new(src: &'a [u8]) -> Self {
        Self { src, pos: 0 }
    }

    fn skip_ws_and_comments(&mut self) {
        while self.pos < self.src.len() {
            let c = self.src[self.pos];
            if c.is_ascii_whitespace() {
                self.pos += 1;
                continue;
            }
            if self.pos + 1 < self.src.len() && c == b'/' && self.src[self.pos + 1] == b'/' {
                // Line comment.
                while self.pos < self.src.len() && self.src[self.pos] != b'\n' {
                    self.pos += 1;
                }
                continue;
            }
            if self.pos + 1 < self.src.len() && c == b'/' && self.src[self.pos + 1] == b'*' {
                self.pos += 2;
                while self.pos + 1 < self.src.len() {
                    if self.src[self.pos] == b'*' && self.src[self.pos + 1] == b'/' {
                        self.pos += 2;
                        break;
                    }
                    self.pos += 1;
                }
                continue;
            }
            break;
        }
    }

    fn peek(&mut self) -> Option<u8> {
        self.skip_ws_and_comments();
        self.src.get(self.pos).copied()
    }

    fn bump(&mut self) -> Option<u8> {
        self.skip_ws_and_comments();
        let c = self.src.get(self.pos).copied()?;
        self.pos += 1;
        Some(c)
    }

    fn expect(&mut self, c: u8) -> Result<()> {
        match self.bump() {
            Some(x) if x == c => Ok(()),
            Some(other) => Err(SdfError::Parse {
                pos: self.pos,
                message: format!("expected `{}`, got `{}`", c as char, other as char),
            }),
            None => Err(SdfError::UnexpectedEof),
        }
    }

    /// A "word" is any run of non-whitespace, non-paren bytes. SDF
    /// identifiers can include `/`, `\`, `[`, `]`, etc., so we keep
    /// the alphabet permissive.
    fn read_word(&mut self) -> Option<&'a [u8]> {
        self.skip_ws_and_comments();
        let start = self.pos;
        while self.pos < self.src.len() {
            let c = self.src[self.pos];
            if c.is_ascii_whitespace() || c == b'(' || c == b')' {
                break;
            }
            self.pos += 1;
        }
        if self.pos == start {
            None
        } else {
            Some(&self.src[start..self.pos])
        }
    }

    fn read_quoted(&mut self) -> Result<&'a [u8]> {
        self.skip_ws_and_comments();
        if self.peek() != Some(b'"') {
            return Err(SdfError::Parse {
                pos: self.pos,
                message: "expected quoted string".into(),
            });
        }
        self.pos += 1;
        let start = self.pos;
        while self.pos < self.src.len() && self.src[self.pos] != b'"' {
            self.pos += 1;
        }
        if self.pos >= self.src.len() {
            return Err(SdfError::UnexpectedEof);
        }
        let s = &self.src[start..self.pos];
        self.pos += 1;
        Ok(s)
    }

    /// Skip the body of an unrecognized s-expression up to and
    /// including its closing paren. Caller is positioned just AFTER
    /// the opening paren.
    fn skip_paren_body(&mut self) -> Result<()> {
        let mut depth = 1;
        while depth > 0 {
            self.skip_ws_and_comments();
            match self.src.get(self.pos).copied() {
                Some(b'(') => {
                    self.pos += 1;
                    depth += 1;
                }
                Some(b')') => {
                    self.pos += 1;
                    depth -= 1;
                }
                Some(b'"') => {
                    // Skip string in entirety.
                    self.pos += 1;
                    while self.pos < self.src.len() && self.src[self.pos] != b'"' {
                        self.pos += 1;
                    }
                    if self.pos < self.src.len() {
                        self.pos += 1;
                    }
                }
                Some(_) => {
                    self.pos += 1;
                }
                None => return Err(SdfError::UnexpectedEof),
            }
        }
        Ok(())
    }
}

fn parse_number(raw: &[u8]) -> Result<f64> {
    let s = std::str::from_utf8(raw).map_err(|_| SdfError::InvalidNumber {
        raw: format!("{raw:?}"),
    })?;
    s.parse::<f64>().map_err(|_| SdfError::InvalidNumber { raw: s.into() })
}

/// Parse `(min:typ:max)` or shorthand. The opening `(` has already
/// been consumed.
fn parse_triplet(t: &mut Tokenizer) -> Result<Triplet> {
    let body = t.read_word().ok_or(SdfError::UnexpectedEof)?;
    // body is everything up to the next paren — could be
    // "1.23:1.45:1.67" or "1.23". Split on `:`.
    let parts: Vec<&[u8]> = body.split(|&c| c == b':').collect();
    let nums: Vec<f64> = parts
        .iter()
        .map(|p| parse_number(p))
        .collect::<Result<Vec<_>>>()?;
    let trip = match nums.len() {
        1 => Triplet::from_typ(nums[0]),
        2 => Triplet {
            min: nums[0],
            typ: nums[0],
            max: nums[1],
        },
        3 => Triplet {
            min: nums[0],
            typ: nums[1],
            max: nums[2],
        },
        _ => {
            return Err(SdfError::Parse {
                pos: t.pos,
                message: format!("invalid triplet: {nums:?}"),
            })
        }
    };
    t.expect(b')')?;
    Ok(trip)
}

fn parse_iopath(t: &mut Tokenizer) -> Result<SdfIoPath> {
    let from = t
        .read_word()
        .ok_or(SdfError::UnexpectedEof)?
        .to_vec();
    let to = t.read_word().ok_or(SdfError::UnexpectedEof)?.to_vec();
    let from_pin = SmolStr::from(std::str::from_utf8(&from).unwrap_or(""));
    let to_pin = SmolStr::from(std::str::from_utf8(&to).unwrap_or(""));

    // Two triplets: rise + fall. Some tools emit only one (then both
    // sides share it).
    t.expect(b'(')?;
    let rise = parse_triplet(t)?;
    let fall = if t.peek() == Some(b'(') {
        t.bump();
        parse_triplet(t)?
    } else {
        rise
    };
    t.expect(b')')?;
    Ok(SdfIoPath {
        from_pin,
        to_pin,
        rise,
        fall,
    })
}

fn parse_delay(t: &mut Tokenizer) -> Result<Vec<SdfIoPath>> {
    let mut iopaths = Vec::new();
    // Body is one or more `(ABSOLUTE ...)` or `(INCREMENTAL ...)`
    // sub-blocks. We treat both as ABSOLUTE for v1.
    while t.peek() == Some(b'(') {
        t.bump();
        let kw = t.read_word().ok_or(SdfError::UnexpectedEof)?;
        let upper = kw.to_ascii_uppercase();
        match upper.as_slice() {
            b"ABSOLUTE" | b"INCREMENTAL" => {
                while t.peek() == Some(b'(') {
                    t.bump();
                    let inner_kw = t.read_word().ok_or(SdfError::UnexpectedEof)?;
                    if inner_kw.eq_ignore_ascii_case(b"IOPATH") {
                        iopaths.push(parse_iopath(t)?);
                    } else {
                        // Skip CONDIDELAY, COND, etc.
                        t.skip_paren_body()?;
                    }
                }
                t.expect(b')')?;
            }
            _ => {
                t.skip_paren_body()?;
            }
        }
    }
    t.expect(b')')?;
    Ok(iopaths)
}

fn parse_cell(t: &mut Tokenizer) -> Result<SdfCell> {
    let mut cell_type: SmolStr = SmolStr::default();
    let mut instance: SmolStr = SmolStr::default();
    let mut iopaths: Vec<SdfIoPath> = Vec::new();
    while t.peek() == Some(b'(') {
        t.bump();
        let kw = t.read_word().ok_or(SdfError::UnexpectedEof)?;
        let upper = kw.to_ascii_uppercase();
        match upper.as_slice() {
            b"CELLTYPE" => {
                let v = t.read_quoted()?;
                cell_type = SmolStr::from(std::str::from_utf8(v).unwrap_or(""));
                t.expect(b')')?;
            }
            b"INSTANCE" => {
                if let Some(name) = t.read_word() {
                    instance = SmolStr::from(std::str::from_utf8(name).unwrap_or(""));
                }
                t.expect(b')')?;
            }
            b"DELAY" => {
                let mut new_paths = parse_delay(t)?;
                iopaths.append(&mut new_paths);
            }
            _ => {
                t.skip_paren_body()?;
            }
        }
    }
    t.expect(b')')?;
    Ok(SdfCell {
        cell_type,
        instance,
        iopaths,
    })
}

pub fn read_sdf(src: &[u8]) -> Result<SdfFile> {
    let mut t = Tokenizer::new(src);
    let mut file = SdfFile {
        design: None,
        sdf_version: None,
        timescale: None,
        cells: Vec::new(),
    };

    t.expect(b'(')?;
    let header_kw = t.read_word().ok_or(SdfError::UnexpectedEof)?;
    if !header_kw.eq_ignore_ascii_case(b"DELAYFILE") {
        return Err(SdfError::Parse {
            pos: t.pos,
            message: format!(
                "expected DELAYFILE, got {:?}",
                std::str::from_utf8(header_kw).unwrap_or("<bad utf8>")
            ),
        });
    }

    while t.peek() == Some(b'(') {
        t.bump();
        let kw = t.read_word().ok_or(SdfError::UnexpectedEof)?;
        let upper = kw.to_ascii_uppercase();
        match upper.as_slice() {
            b"SDFVERSION" => {
                let v = t.read_quoted()?;
                file.sdf_version = Some(SmolStr::from(std::str::from_utf8(v).unwrap_or("")));
                t.expect(b')')?;
            }
            b"DESIGN" => {
                let v = t.read_quoted()?;
                file.design = Some(SmolStr::from(std::str::from_utf8(v).unwrap_or("")));
                t.expect(b')')?;
            }
            b"TIMESCALE" => {
                if let Some(w) = t.read_word() {
                    file.timescale = Some(SmolStr::from(std::str::from_utf8(w).unwrap_or("")));
                }
                t.expect(b')')?;
            }
            b"CELL" => {
                file.cells.push(parse_cell(&mut t)?);
            }
            _ => {
                t.skip_paren_body()?;
            }
        }
    }
    t.expect(b')')?;
    Ok(file)
}

// ---------- Back-annotation ----------

/// Apply `sdf` delays onto `graph`'s `CellArc` edges. Edges are
/// matched by `(instance/from_pin → instance/to_pin)` node names —
/// the graph builder's convention.
///
/// Returns the number of edges actually annotated. Use the result for
/// diagnostics (a count near zero usually means the graph's instance
/// names don't match the SDF's; check the divider character).
///
/// Picks the **typ** value of each triplet. A multi-corner flow
/// would call this once per corner with the corresponding (min/max)
/// view; the [`crate::multi_corner`] module composes those.
pub fn back_annotate(sdf: &SdfFile, graph: &mut TimingGraph) -> usize {
    // Build a lookup: edge `(from_node_name, to_node_name)` → edge
    // index, restricted to CellArc edges.
    let mut by_endpoints: HashMap<(SmolStr, SmolStr), usize> = HashMap::new();
    for (i, e) in graph.edges.iter().enumerate() {
        if !matches!(e.kind, EdgeKind::CellArc) {
            continue;
        }
        let from_name = graph.nodes[e.from.0 as usize].name.clone();
        let to_name = graph.nodes[e.to.0 as usize].name.clone();
        by_endpoints.insert((from_name, to_name), i);
    }

    let mut annotated = 0usize;
    for cell in &sdf.cells {
        for io in &cell.iopaths {
            let from_full = SmolStr::from(format!("{}/{}", cell.instance, io.from_pin));
            let to_full = SmolStr::from(format!("{}/{}", cell.instance, io.to_pin));
            if let Some(&idx) = by_endpoints.get(&(from_full, to_full)) {
                // Use the average of rise+fall typ delays. STA tools
                // can split rising / falling propagation; v1 uses a
                // single per-edge delay so we average.
                let typ = 0.5 * (io.rise.typ + io.fall.typ);
                graph.edges[idx].delay = typ;
                annotated += 1;
            }
        }
    }
    annotated
}

#[allow(dead_code)]
fn _silence_unused(_e: &Edge, _n: NodeId) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_minimal_delayfile() {
        let src = br#"
            (DELAYFILE
              (SDFVERSION "OVI 3.0")
              (DESIGN "top")
              (TIMESCALE 1ns)
            )
        "#;
        let f = read_sdf(src).unwrap();
        assert_eq!(f.sdf_version.as_deref(), Some("OVI 3.0"));
        assert_eq!(f.design.as_deref(), Some("top"));
        assert_eq!(f.timescale.as_deref(), Some("1ns"));
        assert!(f.cells.is_empty());
    }

    #[test]
    fn parse_one_cell_one_iopath() {
        let src = br#"
            (DELAYFILE
              (SDFVERSION "OVI 3.0")
              (CELL
                (CELLTYPE "AND2X1")
                (INSTANCE u_top/u_inst)
                (DELAY (ABSOLUTE
                  (IOPATH A Y (0.123:0.130:0.135) (0.118:0.122:0.130))
                ))
              )
            )
        "#;
        let f = read_sdf(src).unwrap();
        assert_eq!(f.cells.len(), 1);
        let c = &f.cells[0];
        assert_eq!(c.cell_type.as_str(), "AND2X1");
        assert_eq!(c.instance.as_str(), "u_top/u_inst");
        assert_eq!(c.iopaths.len(), 1);
        let io = &c.iopaths[0];
        assert_eq!(io.from_pin.as_str(), "A");
        assert_eq!(io.to_pin.as_str(), "Y");
        assert!((io.rise.typ - 0.130).abs() < 1e-9);
        assert!((io.fall.typ - 0.122).abs() < 1e-9);
    }

    #[test]
    fn shorthand_triplet_widens_to_full() {
        // Single-element triplet: `(0.5)`.
        let src = br#"
            (DELAYFILE
              (CELL
                (CELLTYPE "INV")
                (INSTANCE i)
                (DELAY (ABSOLUTE (IOPATH A Y (0.5) (0.4))))
              )
            )
        "#;
        let f = read_sdf(src).unwrap();
        let io = &f.cells[0].iopaths[0];
        assert_eq!(io.rise, Triplet::from_typ(0.5));
        assert_eq!(io.fall, Triplet::from_typ(0.4));
    }

    #[test]
    fn unknown_blocks_are_tolerated() {
        // `(VOLTAGE ...)`, `(PROCESS ...)` etc. should be skipped.
        let src = br#"
            (DELAYFILE
              (SDFVERSION "3.0")
              (VOLTAGE 1.080:1.080:1.080)
              (PROCESS "tt")
              (TEMPERATURE 25.0:25.0:25.0)
              (CELL
                (CELLTYPE "INV")
                (INSTANCE u)
                (DELAY (ABSOLUTE (IOPATH A Y (0.1) (0.1))))
              )
            )
        "#;
        let f = read_sdf(src).unwrap();
        assert_eq!(f.cells.len(), 1);
    }

    #[test]
    fn comments_are_ignored() {
        let src = br#"
            // A comment.
            (DELAYFILE
              /* block comment */
              (SDFVERSION "3.0")
            )
        "#;
        let f = read_sdf(src).unwrap();
        assert_eq!(f.sdf_version.as_deref(), Some("3.0"));
    }

    #[test]
    fn back_annotation_updates_matching_edges() {
        use crate::graph::NodeKind;

        let src = br#"
            (DELAYFILE
              (CELL
                (CELLTYPE "AND2X1")
                (INSTANCE u_inst)
                (DELAY (ABSOLUTE
                  (IOPATH A Y (0.10:0.10:0.10) (0.10:0.10:0.10))
                ))
              )
            )
        "#;
        let sdf = read_sdf(src).unwrap();

        let mut g = TimingGraph::new();
        let a = g.add_node("u_inst/A", NodeKind::CellInput);
        let y = g.add_node("u_inst/Y", NodeKind::CellOutput);
        g.add_edge(a, y, EdgeKind::CellArc, 0.0);
        g.finalize();

        assert_eq!(g.edges[0].delay, 0.0);
        let n = back_annotate(&sdf, &mut g);
        assert_eq!(n, 1);
        assert!((g.edges[0].delay - 0.10).abs() < 1e-9);
    }

    #[test]
    fn back_annotation_skips_nonmatching() {
        use crate::graph::NodeKind;
        let src = br#"
            (DELAYFILE
              (CELL
                (CELLTYPE "X")
                (INSTANCE foo)
                (DELAY (ABSOLUTE (IOPATH A Y (0.1) (0.1))))
              )
            )
        "#;
        let sdf = read_sdf(src).unwrap();
        let mut g = TimingGraph::new();
        let a = g.add_node("bar/A", NodeKind::CellInput);
        let y = g.add_node("bar/Y", NodeKind::CellOutput);
        g.add_edge(a, y, EdgeKind::CellArc, 0.0);
        g.finalize();
        assert_eq!(back_annotate(&sdf, &mut g), 0);
        assert_eq!(g.edges[0].delay, 0.0);
    }

    #[test]
    fn malformed_input_returns_err() {
        // Missing closing paren.
        let src = br#"(DELAYFILE (SDFVERSION "3.0")"#;
        assert!(read_sdf(src).is_err());
    }
}
