//! Bookshelf format adapter — read ISPD'15 / ISPD'18 placement
//! benchmarks into [`crate::types::Placement`].
//!
//! The Bookshelf format is the de-facto standard for academic
//! placement benchmarks. A design is split across five files:
//!
//! | Suffix | Holds |
//! |--------|-------|
//! | `.aux` | Manifest pointing at the other four files. |
//! | `.nodes` | Cells: name, width, height, fixed/movable flag. |
//! | `.nets` | Nets: name + connected `(cell, pin_offset)` list. |
//! | `.pl` | Initial placement: per-cell `(x, y, orientation, /FIXED?)`. |
//! | `.scl` | Site grid: rows with `Coordinate / Sitewidth / NumSites`. |
//!
//! ## Coverage
//!
//! v1 reads the cells, nets, initial positions, and rows. We
//! deliberately ignore:
//! * **Pin offsets** within cells (Bookshelf supports per-pin
//!   `(dx, dy)`; our `Net` model treats each pin as the cell's
//!   center, which is the same approximation FastPlace ships with).
//! * **Multi-row cells** — heights `≠ row_height` are imported but
//!   the legalize pass doesn't yet handle them.
//! * **Region constraints** (`.regions` extension).
//!
//! ## Why this matters
//!
//! Without the Bookshelf adapter, every placement-quality claim in
//! this workspace is tested only against synthetic netlists.
//! Reading the canonical benchmarks lets us:
//!
//! 1. Run [`crate::quadratic_place`] / [`crate::eplace`] on
//!    `ispd15_fft.aux` and report the achieved HPWL.
//! 2. Compare directly against published OpenROAD numbers (which
//!    consume the same benchmarks).
//!
//! The reader is intentionally tolerant: unknown sections / unknown
//! flags are skipped so a partial benchmark still imports.

use crate::types::{Cell, Net, Placement, Row};
use klayout_core::Point;
use smol_str::SmolStr;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum BookshelfError {
    #[error("bookshelf: I/O error reading {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("bookshelf: missing required field `{field}` in {file}:{line}")]
    MissingField {
        file: String,
        line: usize,
        field: &'static str,
    },
    #[error("bookshelf: invalid number `{raw}` in {file}:{line}")]
    InvalidNumber {
        file: String,
        line: usize,
        raw: String,
    },
    #[error("bookshelf: aux manifest references unknown file `{name}`")]
    UnknownAuxRef { name: String },
}

pub type Result<T> = std::result::Result<T, BookshelfError>;

#[derive(Clone, Debug, Default)]
pub struct AuxManifest {
    pub design_name: SmolStr,
    pub nodes_file: Option<PathBuf>,
    pub nets_file: Option<PathBuf>,
    pub pl_file: Option<PathBuf>,
    pub scl_file: Option<PathBuf>,
}

/// Read a `.aux` manifest. Format:
/// ```text
/// RowBasedPlacement : design.nodes design.nets design.pl design.scl
/// ```
pub fn read_aux(path: impl AsRef<Path>) -> Result<AuxManifest> {
    let path = path.as_ref();
    let text = std::fs::read_to_string(path).map_err(|e| BookshelfError::Io {
        path: path.to_path_buf(),
        source: e,
    })?;
    let dir = path.parent().unwrap_or(Path::new("."));
    let design = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unnamed")
        .to_string();
    let mut out = AuxManifest {
        design_name: SmolStr::from(design),
        ..Default::default()
    };
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        // Format: "<TAG> : <file1> <file2> ...".
        let after_colon = line.split(':').nth(1).unwrap_or("");
        for tok in after_colon.split_whitespace() {
            let p = dir.join(tok);
            match Path::new(tok)
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or("")
            {
                "nodes" => out.nodes_file = Some(p),
                "nets" => out.nets_file = Some(p),
                "pl" => out.pl_file = Some(p),
                "scl" => out.scl_file = Some(p),
                _ => {}
            }
        }
    }
    Ok(out)
}

/// Parse a single Bookshelf benchmark from its `.aux` file. Returns
/// a fully-populated [`Placement`].
pub fn read_bookshelf(aux_path: impl AsRef<Path>) -> Result<Placement> {
    let aux = read_aux(&aux_path)?;
    let mut p = Placement::new();
    let mut name_to_idx: HashMap<SmolStr, usize> = HashMap::new();

    if let Some(scl) = &aux.scl_file {
        read_scl_into(&mut p, scl)?;
    }
    if let Some(nodes) = &aux.nodes_file {
        read_nodes_into(&mut p, &mut name_to_idx, nodes)?;
    }
    if let Some(pl) = &aux.pl_file {
        read_pl_into(&mut p, &name_to_idx, pl)?;
    }
    if let Some(nets) = &aux.nets_file {
        read_nets_into(&mut p, &name_to_idx, nets)?;
    }
    Ok(p)
}

fn read_scl_into(p: &mut Placement, path: &Path) -> Result<()> {
    let text = read_text(path)?;
    let mut row_origin_y: Option<i64> = None;
    let mut row_height: Option<i64> = None;
    let mut row_x: Option<i64> = None;
    let mut site_w: Option<i64> = None;
    let mut num_sites: Option<u32> = None;
    let mut in_row = false;

    for (line_no, raw) in text.lines().enumerate() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let upper = line.to_ascii_uppercase();
        if upper.starts_with("CORE") {
            in_row = true;
            row_origin_y = None;
            row_height = None;
            row_x = None;
            site_w = None;
            num_sites = None;
            continue;
        }
        if upper.starts_with("END") {
            if in_row {
                in_row = false;
                if let (Some(y), Some(h), Some(x), Some(sw), Some(n)) =
                    (row_origin_y, row_height, row_x, site_w, num_sites)
                {
                    p.add_row(Row {
                        origin: Point::new(x, y),
                        site_width: sw,
                        num_sites: n,
                        height: h,
                    });
                }
            }
            continue;
        }
        if !in_row {
            continue;
        }
        // Bookshelf SCL keys can be followed by an optional `:`
        // separator (e.g. `Coordinate : 0`). Strip any colons before
        // looking for the numeric value.
        let toks: Vec<&str> = line.split_whitespace().filter(|t| *t != ":").collect();
        if toks.is_empty() {
            continue;
        }
        let key = toks[0].to_ascii_lowercase();
        let parse_i64 = |s: &str| {
            s.parse::<i64>().map_err(|_| BookshelfError::InvalidNumber {
                file: path.display().to_string(),
                line: line_no,
                raw: s.into(),
            })
        };
        match key.as_str() {
            "coordinate" => row_origin_y = Some(parse_i64(toks.get(1).copied().unwrap_or("0"))?),
            "height" => row_height = Some(parse_i64(toks.get(1).copied().unwrap_or("0"))?),
            "subroworigin" => {
                // Format: `SubrowOrigin : <x>  NumSites : <n>` —
                // both keys can appear on the same line.
                row_x = Some(parse_i64(toks.get(1).copied().unwrap_or("0"))?);
                if toks.iter().any(|t| t.eq_ignore_ascii_case("numsites")) {
                    let pos = toks
                        .iter()
                        .position(|t| t.eq_ignore_ascii_case("numsites"))
                        .unwrap();
                    num_sites = Some(toks.get(pos + 1).copied().unwrap_or("0").parse().unwrap_or(0));
                }
            }
            "sitewidth" => site_w = Some(parse_i64(toks.get(1).copied().unwrap_or("0"))?),
            "numsites" => {
                num_sites = Some(toks.get(1).copied().unwrap_or("0").parse().unwrap_or(0))
            }
            _ => {}
        }
    }
    Ok(())
}

fn read_nodes_into(
    p: &mut Placement,
    name_to_idx: &mut HashMap<SmolStr, usize>,
    path: &Path,
) -> Result<()> {
    let text = read_text(path)?;
    for (line_no, raw) in text.lines().enumerate() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        // Skip header lines (UCLA banner / NumNodes / NumTerminals).
        if line.contains(":") || line.to_ascii_uppercase().starts_with("UCLA") {
            continue;
        }
        let mut parts = line.split_whitespace();
        let name = parts.next().unwrap_or("");
        if name.is_empty() {
            continue;
        }
        let parse = |s: &str| {
            s.parse::<i64>().map_err(|_| BookshelfError::InvalidNumber {
                file: path.display().to_string(),
                line: line_no,
                raw: s.into(),
            })
        };
        let w = parse(parts.next().unwrap_or("0"))?;
        let h = parse(parts.next().unwrap_or("0"))?;
        let mut fixed = false;
        for tok in parts {
            if tok.eq_ignore_ascii_case("terminal") || tok.eq_ignore_ascii_case("fixed") {
                fixed = true;
            }
        }
        let idx = p.add_cell(Cell {
            name: SmolStr::from(name),
            width: w,
            height: h,
            position: Point::new(0, 0),
            fixed,
        });
        name_to_idx.insert(SmolStr::from(name), idx);
    }
    Ok(())
}

fn read_pl_into(
    p: &mut Placement,
    name_to_idx: &HashMap<SmolStr, usize>,
    path: &Path,
) -> Result<()> {
    let text = read_text(path)?;
    for (line_no, raw) in text.lines().enumerate() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() || line.starts_with("UCLA") {
            continue;
        }
        let mut parts = line.split_whitespace();
        let name = parts.next().unwrap_or("");
        if name.is_empty() {
            continue;
        }
        let parse = |s: &str| {
            s.parse::<i64>().map_err(|_| BookshelfError::InvalidNumber {
                file: path.display().to_string(),
                line: line_no,
                raw: s.into(),
            })
        };
        let x = parse(parts.next().unwrap_or("0"))?;
        let y = parse(parts.next().unwrap_or("0"))?;
        let mut fixed = false;
        for tok in parts {
            if tok.eq_ignore_ascii_case("/fixed") || tok.eq_ignore_ascii_case("fixed") {
                fixed = true;
            }
        }
        if let Some(&idx) = name_to_idx.get(name) {
            p.cells[idx].position = Point::new(x, y);
            if fixed {
                p.cells[idx].fixed = true;
            }
        }
    }
    Ok(())
}

fn read_nets_into(
    p: &mut Placement,
    name_to_idx: &HashMap<SmolStr, usize>,
    path: &Path,
) -> Result<()> {
    let text = read_text(path)?;
    let mut current_net: Option<(SmolStr, Vec<usize>)> = None;
    let mut net_seq = 0usize;

    let flush = |p: &mut Placement, taken: Option<(SmolStr, Vec<usize>)>| {
        if let Some((name, indices)) = taken {
            if indices.len() >= 2 {
                p.add_net(Net {
                    name,
                    cell_indices: indices,
                    weight: 1.0,
                });
            }
        }
    };

    for raw in text.lines() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let upper_first = line.split_whitespace().next().unwrap_or("");
        if upper_first.eq_ignore_ascii_case("NetDegree") {
            // Flush previous net.
            flush(p, current_net.take());
            // "NetDegree : <degree> [name]" — degree is informational
            // (we count from connections); the optional name is what
            // we record.
            let mut parts = line.split_whitespace();
            parts.next(); // NetDegree
            parts.next(); // ":"
            parts.next(); // degree
            let name = parts
                .next()
                .map(SmolStr::from)
                .unwrap_or_else(|| {
                    let n = SmolStr::from(format!("net_{net_seq}"));
                    n
                });
            net_seq += 1;
            current_net = Some((name, Vec::new()));
            continue;
        }
        // Other lines reference connected cells: "<cell_name> <pin>".
        let mut parts = line.split_whitespace();
        if let Some(cell_name) = parts.next() {
            if let Some(&idx) = name_to_idx.get(cell_name) {
                if let Some((_, ref mut indices)) = current_net {
                    if !indices.contains(&idx) {
                        indices.push(idx);
                    }
                }
            }
        }
    }
    flush(p, current_net.take());
    Ok(())
}

fn read_text(path: &Path) -> Result<String> {
    std::fs::read_to_string(path).map_err(|e| BookshelfError::Io {
        path: path.to_path_buf(),
        source: e,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Synthesize a tiny Bookshelf benchmark in a temp dir; verify
    /// `read_bookshelf` reconstructs the placement we wrote.
    #[test]
    fn round_trips_synthetic_benchmark() {
        let dir = std::env::temp_dir().join(format!(
            "klayout_rs_bookshelf_test_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let aux = "RowBasedPlacement : tiny.nodes tiny.nets tiny.pl tiny.scl\n";
        std::fs::write(dir.join("tiny.aux"), aux).unwrap();
        std::fs::write(
            dir.join("tiny.nodes"),
            "UCLA nodes 1.0\nNumNodes : 3\nNumTerminals : 1\n\
             c1 10 5\nc2 10 5\npad1 5 5 terminal\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("tiny.pl"),
            "UCLA pl 1.0\nc1 0 0 : N\nc2 20 0 : N\npad1 100 0 : N /FIXED\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("tiny.nets"),
            "UCLA nets 1.0\nNumNets : 1\nNumPins : 3\n\
             NetDegree : 3 net1\n  c1 O\n  c2 I\n  pad1 I\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("tiny.scl"),
            "UCLA scl 1.0\nNumRows : 1\n\
             CoreRow Horizontal\n  Coordinate : 0\n  Height : 5\n\
             Sitewidth : 1\n  Sitespacing : 1\n  Siteorient : 1\n\
             Sitesymmetry : 1\n  SubrowOrigin : 0  NumSites : 200\n\
             End\n",
        )
        .unwrap();

        let p = read_bookshelf(dir.join("tiny.aux")).unwrap();
        assert_eq!(p.cells.len(), 3);
        assert_eq!(p.nets.len(), 1);
        assert_eq!(p.rows.len(), 1);
        // Pad must be fixed.
        let pad = p.cells.iter().find(|c| c.name.as_str() == "pad1").unwrap();
        assert!(pad.fixed);
        // c1 / c2 not fixed.
        for n in ["c1", "c2"] {
            let c = p.cells.iter().find(|c| c.name.as_str() == n).unwrap();
            assert!(!c.fixed);
        }
        // Net connects all three.
        assert_eq!(p.nets[0].cell_indices.len(), 3);
        // Row covers 200 sites.
        assert_eq!(p.rows[0].num_sites, 200);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_aux_returns_err() {
        let r = read_aux("/nonexistent/path/missing.aux");
        assert!(r.is_err());
    }
}
