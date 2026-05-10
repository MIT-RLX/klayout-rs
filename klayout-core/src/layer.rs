//! Layers: local indexing inside a Library, GDS (layer, datatype) backing.
//!
//! `LayerIndex` is opaque and only meaningful with the Library that issued it.
//! Crossing libraries goes through `LayerInfo` and an explicit remap.

use smol_str::SmolStr;
use std::collections::HashMap;
use std::num::NonZeroU16;

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LayerIndex(NonZeroU16);

impl LayerIndex {
    pub(crate) fn from_slot(slot: usize) -> Self {
        // LayerIndex packs a 1-based slot into a NonZeroU16. The 64K
        // ceiling is the deliberate design budget — exceeding it is a
        // hard error rather than a recoverable condition because
        // every consumer assumes a stable LayerIndex domain.
        let n = u16::try_from(slot + 1)
            .expect("LayerIndex arena exceeded u16::MAX (65535 layers)");
        Self(NonZeroU16::new(n).expect("LayerIndex: slot+1 ≥ 1 by construction"))
    }

    pub(crate) fn slot(self) -> usize {
        self.0.get() as usize - 1
    }

    pub fn raw(self) -> u16 {
        self.0.get()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct LayerInfo {
    pub layer: u16,
    pub datatype: u16,
    pub name: SmolStr,
    pub purpose: Option<SmolStr>,
}

impl LayerInfo {
    pub fn gds(layer: u16, datatype: u16) -> Self {
        Self {
            layer,
            datatype,
            name: SmolStr::default(),
            purpose: None,
        }
    }

    pub fn named(name: impl Into<SmolStr>, layer: u16, datatype: u16) -> Self {
        Self {
            layer,
            datatype,
            name: name.into(),
            purpose: None,
        }
    }

    pub fn key(&self) -> (u16, u16) {
        (self.layer, self.datatype)
    }
}

#[derive(Clone, Debug, Default)]
pub struct LayerTable {
    by_index: Vec<LayerInfo>,
    by_gds: HashMap<(u16, u16), LayerIndex>,
    by_name: HashMap<SmolStr, LayerIndex>,
}

impl LayerTable {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get_or_insert(&mut self, info: LayerInfo) -> LayerIndex {
        if let Some(&idx) = self.by_gds.get(&info.key()) {
            return idx;
        }
        let idx = LayerIndex::from_slot(self.by_index.len());
        self.by_gds.insert(info.key(), idx);
        if !info.name.is_empty() {
            self.by_name.insert(info.name.clone(), idx);
        }
        self.by_index.push(info);
        idx
    }

    pub fn info(&self, idx: LayerIndex) -> &LayerInfo {
        &self.by_index[idx.slot()]
    }

    pub fn by_gds(&self, layer: u16, datatype: u16) -> Option<LayerIndex> {
        self.by_gds.get(&(layer, datatype)).copied()
    }

    pub fn by_name(&self, name: &str) -> Option<LayerIndex> {
        self.by_name.get(name).copied()
    }

    pub fn len(&self) -> usize {
        self.by_index.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_index.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (LayerIndex, &LayerInfo)> {
        self.by_index
            .iter()
            .enumerate()
            .map(|(i, info)| (LayerIndex::from_slot(i), info))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dedup_by_gds() {
        let mut t = LayerTable::new();
        let a = t.get_or_insert(LayerInfo::named("WG", 1, 0));
        let b = t.get_or_insert(LayerInfo::gds(1, 0));
        assert_eq!(a, b);
        assert_eq!(t.len(), 1);
        assert_eq!(t.by_name("WG"), Some(a));
    }
}
