//! Instance: placement of a child cell with a transform, optional repetition,
//! and provenance.

use crate::cell::CellId;
use crate::coord::{Bbox, Trans, Vec2};
use crate::properties::Properties;

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct SourceTag(pub u32);

impl SourceTag {
    pub const NONE: Self = Self(0);
}

/// OASIS-style placement repetition. Generalizes KLayout's `CellInstArray`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Repetition {
    Regular {
        row: Vec2,
        col: Vec2,
        n_rows: u32,
        n_cols: u32,
    },
    Irregular {
        offsets: Vec<Vec2>,
    },
}

impl Repetition {
    pub fn count(&self) -> usize {
        match self {
            Repetition::Regular { n_rows, n_cols, .. } => {
                (*n_rows as usize) * (*n_cols as usize)
            }
            Repetition::Irregular { offsets } => offsets.len() + 1,
        }
    }

    pub fn placement_bbox(&self) -> Bbox {
        let mut b = Bbox::from_point(crate::coord::Point::ZERO);
        match self {
            Repetition::Regular {
                row,
                col,
                n_rows,
                n_cols,
            } => {
                if *n_rows == 0 || *n_cols == 0 {
                    return Bbox::EMPTY;
                }
                let r = (*n_rows as i64).saturating_sub(1);
                let c = (*n_cols as i64).saturating_sub(1);
                let p = crate::coord::Point::new(
                    col.x * c + row.x * r,
                    col.y * c + row.y * r,
                );
                b.expand_to(p);
            }
            Repetition::Irregular { offsets } => {
                for o in offsets {
                    b.expand_to(crate::coord::Point::new(o.x, o.y));
                }
            }
        }
        b
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Instance {
    pub cell: CellId,
    pub trans: Trans,
    pub repetition: Option<Repetition>,
    pub properties: Properties,
    pub source: SourceTag,
}

impl Instance {
    pub fn new(cell: CellId, trans: Trans) -> Self {
        Self {
            cell,
            trans,
            repetition: None,
            properties: Properties::new(),
            source: SourceTag::NONE,
        }
    }

    pub fn with_repetition(mut self, r: Repetition) -> Self {
        self.repetition = Some(r);
        self
    }
}
