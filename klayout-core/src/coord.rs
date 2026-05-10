//! Integer coordinates and exact orthogonal transforms.
//!
//! All layout coordinates are i64 DBU. The float `DTrans` exists only as an
//! escape hatch for arbitrary-angle work (mask rotation, angled photonics);
//! conversion back to grid is explicit and lossy.

use std::ops::{Add, Neg, Sub};

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Point {
    pub x: i64,
    pub y: i64,
}

impl Point {
    pub const ZERO: Self = Self { x: 0, y: 0 };
    #[inline]
    pub const fn new(x: i64, y: i64) -> Self {
        Self { x, y }
    }
}

impl Add<Vec2> for Point {
    type Output = Point;
    #[inline]
    fn add(self, v: Vec2) -> Point {
        Point::new(self.x + v.x, self.y + v.y)
    }
}

impl Sub for Point {
    type Output = Vec2;
    #[inline]
    fn sub(self, o: Point) -> Vec2 {
        Vec2::new(self.x - o.x, self.y - o.y)
    }
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Vec2 {
    pub x: i64,
    pub y: i64,
}

impl Vec2 {
    pub const ZERO: Self = Self { x: 0, y: 0 };
    #[inline]
    pub const fn new(x: i64, y: i64) -> Self {
        Self { x, y }
    }
}

impl Add for Vec2 {
    type Output = Vec2;
    #[inline]
    fn add(self, o: Vec2) -> Vec2 {
        Vec2::new(self.x + o.x, self.y + o.y)
    }
}

impl Sub for Vec2 {
    type Output = Vec2;
    #[inline]
    fn sub(self, o: Vec2) -> Vec2 {
        Vec2::new(self.x - o.x, self.y - o.y)
    }
}

impl Neg for Vec2 {
    type Output = Vec2;
    #[inline]
    fn neg(self) -> Vec2 {
        Vec2::new(-self.x, -self.y)
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Bbox {
    pub min: Point,
    pub max: Point,
}

impl Bbox {
    pub const EMPTY: Self = Self {
        min: Point::new(i64::MAX, i64::MAX),
        max: Point::new(i64::MIN, i64::MIN),
    };

    #[inline]
    pub const fn new(min: Point, max: Point) -> Self {
        Self { min, max }
    }

    #[inline]
    pub fn from_point(p: Point) -> Self {
        Self { min: p, max: p }
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.min.x > self.max.x || self.min.y > self.max.y
    }

    #[inline]
    pub fn width(&self) -> i64 {
        if self.is_empty() {
            0
        } else {
            self.max.x - self.min.x
        }
    }

    #[inline]
    pub fn height(&self) -> i64 {
        if self.is_empty() {
            0
        } else {
            self.max.y - self.min.y
        }
    }

    pub fn contains(&self, p: Point) -> bool {
        !self.is_empty()
            && p.x >= self.min.x
            && p.x <= self.max.x
            && p.y >= self.min.y
            && p.y <= self.max.y
    }

    pub fn intersects(&self, other: &Bbox) -> bool {
        !self.is_empty()
            && !other.is_empty()
            && self.min.x <= other.max.x
            && self.max.x >= other.min.x
            && self.min.y <= other.max.y
            && self.max.y >= other.min.y
    }

    pub fn intersection(&self, other: &Bbox) -> Bbox {
        if self.is_empty() || other.is_empty() {
            return Bbox::EMPTY;
        }
        let min = Point::new(self.min.x.max(other.min.x), self.min.y.max(other.min.y));
        let max = Point::new(self.max.x.min(other.max.x), self.max.y.min(other.max.y));
        if min.x > max.x || min.y > max.y {
            Bbox::EMPTY
        } else {
            Bbox::new(min, max)
        }
    }

    pub fn union(&self, other: &Bbox) -> Bbox {
        if self.is_empty() {
            return *other;
        }
        if other.is_empty() {
            return *self;
        }
        Bbox::new(
            Point::new(self.min.x.min(other.min.x), self.min.y.min(other.min.y)),
            Point::new(self.max.x.max(other.max.x), self.max.y.max(other.max.y)),
        )
    }

    pub fn expand_to(&mut self, p: Point) {
        if self.is_empty() {
            self.min = p;
            self.max = p;
        } else {
            self.min.x = self.min.x.min(p.x);
            self.min.y = self.min.y.min(p.y);
            self.max.x = self.max.x.max(p.x);
            self.max.y = self.max.y.max(p.y);
        }
    }

    pub fn corners(&self) -> [Point; 4] {
        [
            self.min,
            Point::new(self.max.x, self.min.y),
            self.max,
            Point::new(self.min.x, self.max.y),
        ]
    }
}

impl Default for Bbox {
    fn default() -> Self {
        Self::EMPTY
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Rot4 {
    R0 = 0,
    R90 = 1,
    R180 = 2,
    R270 = 3,
}

impl Rot4 {
    #[inline]
    pub fn apply(self, p: Point) -> Point {
        match self {
            Rot4::R0 => p,
            Rot4::R90 => Point::new(-p.y, p.x),
            Rot4::R180 => Point::new(-p.x, -p.y),
            Rot4::R270 => Point::new(p.y, -p.x),
        }
    }

    #[inline]
    pub fn apply_vec(self, v: Vec2) -> Vec2 {
        match self {
            Rot4::R0 => v,
            Rot4::R90 => Vec2::new(-v.y, v.x),
            Rot4::R180 => Vec2::new(-v.x, -v.y),
            Rot4::R270 => Vec2::new(v.y, -v.x),
        }
    }

    #[inline]
    pub fn compose(self, other: Rot4) -> Rot4 {
        let s = self as u8;
        let o = other as u8;
        match (s + o) % 4 {
            0 => Rot4::R0,
            1 => Rot4::R90,
            2 => Rot4::R180,
            _ => Rot4::R270,
        }
    }

    #[inline]
    pub fn inverse(self) -> Rot4 {
        match self {
            Rot4::R0 => Rot4::R0,
            Rot4::R90 => Rot4::R270,
            Rot4::R180 => Rot4::R180,
            Rot4::R270 => Rot4::R90,
        }
    }
}

/// Exact orthogonal transform: optional mirror about X axis, then rotation,
/// then translation. Composition is exact; coordinates stay on grid.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Trans {
    pub rot: Rot4,
    pub mirror: bool,
    pub disp: Vec2,
}

impl Trans {
    pub const IDENTITY: Self = Self {
        rot: Rot4::R0,
        mirror: false,
        disp: Vec2::ZERO,
    };

    #[inline]
    pub const fn new(rot: Rot4, mirror: bool, disp: Vec2) -> Self {
        Self { rot, mirror, disp }
    }

    #[inline]
    pub fn translate(disp: Vec2) -> Self {
        Self::new(Rot4::R0, false, disp)
    }

    #[inline]
    pub fn rotate(rot: Rot4) -> Self {
        Self::new(rot, false, Vec2::ZERO)
    }

    #[inline]
    pub fn mirror_x() -> Self {
        Self::new(Rot4::R0, true, Vec2::ZERO)
    }

    #[inline]
    pub fn apply(self, p: Point) -> Point {
        let p = if self.mirror { Point::new(p.x, -p.y) } else { p };
        let p = self.rot.apply(p);
        Point::new(p.x + self.disp.x, p.y + self.disp.y)
    }

    pub fn apply_bbox(self, b: Bbox) -> Bbox {
        if b.is_empty() {
            return b;
        }
        let mut out = Bbox::EMPTY;
        for c in b.corners() {
            out.expand_to(self.apply(c));
        }
        out
    }

    /// Composition: `(self * other).apply(p) == self.apply(other.apply(p))`.
    pub fn compose(self, other: Trans) -> Trans {
        let mirror = self.mirror ^ other.mirror;
        // Mirror across X commutes with rotation by negating the rotation.
        let inner_rot = if self.mirror {
            other.rot.inverse()
        } else {
            other.rot
        };
        let rot = self.rot.compose(inner_rot);
        // Apply self to other.disp, then add self.disp.
        let other_disp = if self.mirror {
            Vec2::new(other.disp.x, -other.disp.y)
        } else {
            other.disp
        };
        let rotated = self.rot.apply_vec(other_disp);
        let disp = Vec2::new(rotated.x + self.disp.x, rotated.y + self.disp.y);
        Trans { rot, mirror, disp }
    }

    pub fn inverse(self) -> Trans {
        // self(p) = R(M(p)) + d  =>  inverse: M'(R'(q - d)) where M' = M, R' = R^-1
        let neg_d = -self.disp;
        let rot_inv = self.rot.inverse();
        let r_neg_d = rot_inv.apply_vec(neg_d);
        let disp = if self.mirror {
            Vec2::new(r_neg_d.x, -r_neg_d.y)
        } else {
            r_neg_d
        };
        Trans {
            rot: if self.mirror { self.rot } else { rot_inv },
            mirror: self.mirror,
            disp,
        }
    }
}

impl Default for Trans {
    fn default() -> Self {
        Self::IDENTITY
    }
}

/// Floating-point complex transform. Use only for non-orthogonal rotation
/// or magnification. Conversion back to integer DBU is explicit and lossy.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct DTrans {
    pub angle_deg: f64,
    pub mag: f64,
    pub mirror: bool,
    pub disp: Vec2,
}

impl DTrans {
    pub const IDENTITY: Self = Self {
        angle_deg: 0.0,
        mag: 1.0,
        mirror: false,
        disp: Vec2::ZERO,
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rot90_apply() {
        let p = Point::new(10, 0);
        assert_eq!(Rot4::R90.apply(p), Point::new(0, 10));
        assert_eq!(Rot4::R180.apply(p), Point::new(-10, 0));
        assert_eq!(Rot4::R270.apply(p), Point::new(0, -10));
    }

    #[test]
    fn trans_inverse_roundtrip() {
        let t = Trans::new(Rot4::R90, true, Vec2::new(7, -3));
        let inv = t.inverse();
        for p in [
            Point::new(0, 0),
            Point::new(100, 200),
            Point::new(-50, 75),
        ] {
            assert_eq!(inv.apply(t.apply(p)), p, "inverse failed for {p:?}");
        }
    }

    #[test]
    fn trans_compose_matches_apply() {
        let a = Trans::new(Rot4::R90, false, Vec2::new(10, 20));
        let b = Trans::new(Rot4::R180, true, Vec2::new(-5, 3));
        let ab = a.compose(b);
        for p in [Point::new(1, 2), Point::new(-7, 8), Point::new(0, 0)] {
            assert_eq!(ab.apply(p), a.apply(b.apply(p)));
        }
    }

    #[test]
    fn bbox_ops() {
        let a = Bbox::new(Point::new(0, 0), Point::new(10, 10));
        let b = Bbox::new(Point::new(5, 5), Point::new(15, 15));
        assert!(a.intersects(&b));
        assert_eq!(
            a.union(&b),
            Bbox::new(Point::new(0, 0), Point::new(15, 15))
        );
        assert!(Bbox::EMPTY.is_empty());
    }
}
