//! Generic typed key-value store for round-tripping GDS/OASIS user properties
//! and tagging Component build args on cells.
//!
//! Backed by a `Vec` so iteration order is deterministic and cheap to hash.

use smol_str::SmolStr;

#[derive(Clone, Debug, PartialEq)]
pub enum PropertyValue {
    Int(i64),
    Float(f64),
    String(SmolStr),
    Bytes(Vec<u8>),
    List(Vec<PropertyValue>),
}

impl Eq for PropertyValue {}

impl std::hash::Hash for PropertyValue {
    fn hash<H: std::hash::Hasher>(&self, h: &mut H) {
        match self {
            PropertyValue::Int(i) => {
                h.write_u8(0);
                i.hash(h);
            }
            PropertyValue::Float(f) => {
                h.write_u8(1);
                h.write_u64(f.to_bits());
            }
            PropertyValue::String(s) => {
                h.write_u8(2);
                s.hash(h);
            }
            PropertyValue::Bytes(b) => {
                h.write_u8(3);
                b.hash(h);
            }
            PropertyValue::List(l) => {
                h.write_u8(4);
                l.hash(h);
            }
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Properties {
    inner: Vec<(SmolStr, PropertyValue)>,
}

impl Properties {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn set(&mut self, key: impl Into<SmolStr>, value: PropertyValue) {
        let key = key.into();
        if let Some(slot) = self.inner.iter_mut().find(|(k, _)| *k == key) {
            slot.1 = value;
        } else {
            self.inner.push((key, value));
        }
    }

    pub fn get(&self, key: &str) -> Option<&PropertyValue> {
        self.inner.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&SmolStr, &PropertyValue)> {
        self.inner.iter().map(|(k, v)| (k, v))
    }

    pub(crate) fn sorted_iter(&self) -> Vec<(&SmolStr, &PropertyValue)> {
        let mut v: Vec<_> = self.inner.iter().map(|(k, v)| (k, v)).collect();
        v.sort_by(|a, b| a.0.cmp(b.0));
        v
    }
}
