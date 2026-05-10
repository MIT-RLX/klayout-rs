//! Geometry primitives, as data only.
//!
//! No boolean ops, no sizing, no point-in-polygon. Algorithms live in the
//! `klayout-geom` crate so the geometry backend can be swapped without
//! touching the data model.

use crate::coord::{Bbox, Point, Trans};
use smallvec::SmallVec;
use smol_str::SmolStr;

/// Sort holes by their first vertex (which after normalization is the
/// lowest-y/lowest-x point of each ring). Stable for identical first
/// vertices using vertex count as the tiebreaker.
fn sort_holes(holes: &mut [SmallVec<[Point; 8]>]) {
    holes.sort_by(|a, b| {
        let ka = a.first().copied().unwrap_or(Point::ZERO);
        let kb = b.first().copied().unwrap_or(Point::ZERO);
        (ka.x, ka.y, a.len()).cmp(&(kb.x, kb.y, b.len()))
    });
}

/// Canonicalize a polygon ring in place: lowest-y/lowest-x first.
/// `want_ccw = false` (default for hulls) normalizes to CW;
/// `want_ccw = true` (for holes) normalizes to CCW.
fn normalize_ring(pts: &mut SmallVec<[Point; 8]>, want_ccw: bool) {
    if pts.len() < 3 {
        return;
    }
    let mut start = 0usize;
    for i in 1..pts.len() {
        let a = pts[i];
        let s = pts[start];
        if a.y < s.y || (a.y == s.y && a.x < s.x) {
            start = i;
        }
    }
    if start != 0 {
        pts.rotate_left(start);
    }
    let mut area2: i128 = 0;
    let n = pts.len();
    for i in 0..n {
        let a = pts[i];
        let b = pts[(i + 1) % n];
        area2 += (a.x as i128) * (b.y as i128) - (b.x as i128) * (a.y as i128);
    }
    let is_ccw = area2 > 0;
    if is_ccw != want_ccw {
        pts[1..].reverse();
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Polygon {
    pub hull: SmallVec<[Point; 8]>,
    pub holes: Vec<SmallVec<[Point; 8]>>,
}

impl Polygon {
    pub fn rect(b: Bbox) -> Self {
        let mut p = Self {
            hull: b.corners().into_iter().collect(),
            holes: Vec::new(),
        };
        p.normalize();
        p
    }

    /// Build a polygon from a hull. The hull is canonicalized:
    /// clockwise winding, starting from the lowest-y/lowest-x vertex.
    /// This matches KLayout's canonical form so cross-tool round-trips
    /// preserve byte-equal vertex sequences.
    pub fn from_hull(hull: impl IntoIterator<Item = Point>) -> Self {
        let mut p = Self {
            hull: hull.into_iter().collect(),
            holes: Vec::new(),
        };
        p.normalize();
        p
    }

    /// Like `from_hull` but skips canonicalization. Use when you specifically
    /// need to preserve a particular vertex order (extremely rare; mainly for
    /// tests that probe normalization itself).
    pub fn from_hull_raw(hull: impl IntoIterator<Item = Point>) -> Self {
        Self {
            hull: hull.into_iter().collect(),
            holes: Vec::new(),
        }
    }

    /// Canonicalize in place: hull → lowest-y/lowest-x first, CW winding;
    /// holes → lowest-y/lowest-x first, CCW winding; hole list sorted by
    /// the first vertex of each hole so polygons that differ only in
    /// hole-construction order hash identically.
    pub fn normalize(&mut self) {
        normalize_ring(&mut self.hull, false);
        for hole in &mut self.holes {
            normalize_ring(hole, true);
        }
        sort_holes(&mut self.holes);
    }

    /// Append a hole, normalizing winding and re-sorting the hole list.
    pub fn add_hole(&mut self, hole: impl IntoIterator<Item = Point>) {
        let mut pts: SmallVec<[Point; 8]> = hole.into_iter().collect();
        normalize_ring(&mut pts, true);
        self.holes.push(pts);
        sort_holes(&mut self.holes);
    }

    pub fn bbox(&self) -> Bbox {
        let mut b = Bbox::EMPTY;
        for p in &self.hull {
            b.expand_to(*p);
        }
        b
    }

    pub fn transform(&self, t: Trans) -> Polygon {
        Polygon {
            hull: self.hull.iter().map(|p| t.apply(*p)).collect(),
            holes: self
                .holes
                .iter()
                .map(|h| h.iter().map(|p| t.apply(*p)).collect())
                .collect(),
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum PathCap {
    Flat,
    Round,
    Extended,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Path {
    pub points: SmallVec<[Point; 4]>,
    pub width: i64,
    pub begin_ext: i64,
    pub end_ext: i64,
    pub cap: PathCap,
}

impl Path {
    pub fn new(points: impl IntoIterator<Item = Point>, width: i64) -> Self {
        Self {
            points: points.into_iter().collect(),
            width,
            begin_ext: 0,
            end_ext: 0,
            cap: PathCap::Flat,
        }
    }

    pub fn bbox(&self) -> Bbox {
        let mut b = Bbox::EMPTY;
        let half = self.width / 2;
        for p in &self.points {
            b.expand_to(Point::new(p.x - half, p.y - half));
            b.expand_to(Point::new(p.x + half, p.y + half));
        }
        b
    }

    pub fn transform(&self, t: Trans) -> Path {
        Path {
            points: self.points.iter().map(|p| t.apply(*p)).collect(),
            width: self.width,
            begin_ext: self.begin_ext,
            end_ext: self.end_ext,
            cap: self.cap,
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Rect {
    pub bbox: Bbox,
}

impl Rect {
    pub fn new(b: Bbox) -> Self {
        Self { bbox: b }
    }
    pub fn transform(self, t: Trans) -> Rect {
        Rect {
            bbox: t.apply_bbox(self.bbox),
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum HAlign {
    Left,
    Center,
    Right,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum VAlign {
    Bottom,
    Middle,
    Top,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Text {
    pub string: SmolStr,
    pub anchor: Point,
    pub size: i32,
    pub halign: HAlign,
    pub valign: VAlign,
}

impl Text {
    pub fn new(s: impl Into<SmolStr>, anchor: Point) -> Self {
        Self {
            string: s.into(),
            anchor,
            size: 0,
            halign: HAlign::Left,
            valign: VAlign::Bottom,
        }
    }

    pub fn bbox(&self) -> Bbox {
        Bbox::from_point(self.anchor)
    }

    pub fn transform(&self, t: Trans) -> Text {
        Text {
            string: self.string.clone(),
            anchor: t.apply(self.anchor),
            size: self.size,
            halign: self.halign,
            valign: self.valign,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Shape {
    Polygon(Polygon),
    Path(Path),
    Box(Rect),
    Text(Text),
}

impl Shape {
    pub fn bbox(&self) -> Bbox {
        match self {
            Shape::Polygon(p) => p.bbox(),
            Shape::Path(p) => p.bbox(),
            Shape::Box(r) => r.bbox,
            Shape::Text(t) => t.bbox(),
        }
    }

    pub fn transform(&self, t: Trans) -> Shape {
        match self {
            Shape::Polygon(p) => Shape::Polygon(p.transform(t)),
            Shape::Path(p) => Shape::Path(p.transform(t)),
            Shape::Box(r) => Shape::Box(r.transform(t)),
            Shape::Text(x) => Shape::Text(x.transform(t)),
        }
    }

    pub(crate) fn discriminant(&self) -> u8 {
        match self {
            Shape::Polygon(_) => 0,
            Shape::Path(_) => 1,
            Shape::Box(_) => 2,
            Shape::Text(_) => 3,
        }
    }
}

impl From<Polygon> for Shape {
    fn from(p: Polygon) -> Self {
        Shape::Polygon(p)
    }
}
impl From<Path> for Shape {
    fn from(p: Path) -> Self {
        Shape::Path(p)
    }
}
impl From<Rect> for Shape {
    fn from(r: Rect) -> Self {
        Shape::Box(r)
    }
}
impl From<Bbox> for Shape {
    fn from(b: Bbox) -> Self {
        Shape::Box(Rect::new(b))
    }
}
impl From<Text> for Shape {
    fn from(t: Text) -> Self {
        Shape::Text(t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn polygon_rect_bbox() {
        let b = Bbox::new(Point::new(0, 0), Point::new(10, 5));
        let p = Polygon::rect(b);
        assert_eq!(p.bbox(), b);
    }

    #[test]
    fn shape_from_bbox() {
        let s: Shape = Bbox::new(Point::new(0, 0), Point::new(1, 1)).into();
        assert!(matches!(s, Shape::Box(_)));
    }
}
