pub use egui::{Color32, Rgba};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

use crate::timestamp::{Interval, Timestamp};

// We encode EntryID as i64 because it allows us to pack Summary into the
// value -1. Users shouldn't need to know about this and interact through the
// methods below, or via EntryIndex.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
pub struct EntryID(Vec<i64>);

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
pub enum EntryIndex {
    Summary,
    Slot(u64),
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DataSourceInfo {
    pub entry_info: EntryInfo,
    pub interval: Interval,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub enum EntryInfo {
    Panel {
        short_name: String,
        long_name: String,
        summary: Option<Box<EntryInfo>>,
        slots: Vec<EntryInfo>,
    },
    Slot {
        short_name: String,
        long_name: String,
        max_rows: u64,
    },
    Summary {
        color: Color32,
    },
}

#[derive(Debug, Copy, Clone, PartialEq, PartialOrd, Default, Deserialize, Serialize)]
pub struct UtilPoint {
    pub time: Timestamp,
    pub util: f32,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub enum Field {
    I64(i64),
    U64(u64),
    String(String),
    Interval(Interval),
    Empty,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
pub struct ItemUID(pub u64);

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Item {
    pub item_uid: ItemUID,
    pub interval: Interval,
    pub color: Color32,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ItemMeta {
    pub item_uid: ItemUID,
    // As opposed to the interval in Item, which may get expanded for
    // visibility, or sliced up into multiple tiles, this interval covers the
    // entire duration of the original item, unexpanded and unsliced.
    pub original_interval: Interval,
    pub title: String,
    pub fields: Vec<(String, Field)>,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
pub struct TileID(pub Interval);

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct TileSet {
    pub tiles: Vec<Vec<TileID>>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SummaryTileData {
    pub utilization: Vec<UtilPoint>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SummaryTile {
    pub entry_id: EntryID,
    pub tile_id: TileID,
    pub data: SummaryTileData,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SlotTileData {
    pub items: Vec<Vec<Item>>, // row -> [item]
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SlotTile {
    pub entry_id: EntryID,
    pub tile_id: TileID,
    pub data: SlotTileData,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SlotMetaTileData {
    pub items: Vec<Vec<ItemMeta>>, // row -> [item]
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SlotMetaTile {
    pub entry_id: EntryID,
    pub tile_id: TileID,
    pub data: SlotMetaTileData,
}

pub trait DataSource {
    fn fetch_info(&self) -> DataSourceInfo;
    fn fetch_tile_set(&self) -> TileSet;
    fn fetch_summary_tile(&self, entry_id: &EntryID, tile_id: TileID) -> SummaryTile;
    fn fetch_slot_tile(&self, entry_id: &EntryID, tile_id: TileID) -> SlotTile;
    fn fetch_slot_meta_tile(&self, entry_id: &EntryID, tile_id: TileID) -> SlotMetaTile;
}

pub trait DataSourceMut {
    fn fetch_info(&mut self) -> DataSourceInfo;
    fn fetch_tile_set(&mut self) -> TileSet;
    fn fetch_summary_tile(&mut self, entry_id: &EntryID, tile_id: TileID) -> SummaryTile;
    fn fetch_slot_tile(&mut self, entry_id: &EntryID, tile_id: TileID) -> SlotTile;
    fn fetch_slot_meta_tile(&mut self, entry_id: &EntryID, tile_id: TileID) -> SlotMetaTile;
}

impl<T: DataSource> DataSourceMut for T {
    fn fetch_info(&mut self) -> DataSourceInfo {
        DataSource::fetch_info(self)
    }
    fn fetch_tile_set(&mut self) -> TileSet {
        DataSource::fetch_tile_set(self)
    }
    fn fetch_summary_tile(&mut self, entry_id: &EntryID, tile_id: TileID) -> SummaryTile {
        DataSource::fetch_summary_tile(self, entry_id, tile_id)
    }
    fn fetch_slot_tile(&mut self, entry_id: &EntryID, tile_id: TileID) -> SlotTile {
        DataSource::fetch_slot_tile(self, entry_id, tile_id)
    }
    fn fetch_slot_meta_tile(&mut self, entry_id: &EntryID, tile_id: TileID) -> SlotMetaTile {
        DataSource::fetch_slot_meta_tile(self, entry_id, tile_id)
    }
}

impl EntryID {
    pub fn root() -> Self {
        Self(Vec::new())
    }

    pub fn summary(&self) -> Self {
        let mut result = self.clone();
        result.0.push(-1);
        result
    }

    pub fn child(&self, index: u64) -> Self {
        let mut result = self.clone();
        result
            .0
            .push(index.try_into().expect("unable to fit in i64"));
        result
    }

    pub fn level(&self) -> u64 {
        self.0.len() as u64
    }

    pub fn last_slot_index(&self) -> Option<u64> {
        let last = self.0.last()?;
        (*last).try_into().ok()
    }

    pub fn slot_index(&self, level: u64) -> Option<u64> {
        let last = self.0.get(level as usize)?;
        (*last).try_into().ok()
    }

    pub fn last_index(&self) -> Option<EntryIndex> {
        let last = self.0.last()?;
        Some(
            (*last)
                .try_into()
                .map_or(EntryIndex::Summary, EntryIndex::Slot),
        )
    }

    pub fn index(&self, level: u64) -> Option<EntryIndex> {
        let last = self.0.get(level as usize)?;
        Some(
            (*last)
                .try_into()
                .map_or(EntryIndex::Summary, EntryIndex::Slot),
        )
    }

    pub fn has_prefix(&self, prefix: &EntryID) -> bool {
        if prefix.0.len() > self.0.len() {
            return false;
        }
        for (a, b) in self.0.iter().zip(prefix.0.iter()) {
            if a != b {
                return false;
            }
        }
        true
    }
}

impl EntryInfo {
    pub fn get(&self, entry_id: &EntryID) -> Option<&EntryInfo> {
        let mut result = self;
        for i in 0..entry_id.level() {
            match (entry_id.index(i)?, result) {
                (EntryIndex::Summary, EntryInfo::Panel { summary, .. }) => {
                    return summary.as_deref();
                }
                (EntryIndex::Slot(j), EntryInfo::Panel { slots, .. }) => {
                    result = slots.get(j as usize)?;
                }
                _ => panic!("EntryID and EntryInfo do not match"),
            }
        }
        Some(result)
    }

    pub fn nodes(&self) -> u64 {
        if let EntryInfo::Panel { slots, .. } = self {
            slots.len() as u64
        } else {
            unreachable!()
        }
    }

    pub fn kinds(&self) -> Vec<String> {
        if let EntryInfo::Panel { slots: nodes, .. } = self {
            let mut result = Vec::new();
            let mut set = BTreeSet::new();
            for node in nodes {
                if let EntryInfo::Panel { slots: kinds, .. } = node {
                    for kind in kinds {
                        if let EntryInfo::Panel { short_name, .. } = kind {
                            if set.insert(short_name) {
                                result.push(short_name.clone());
                            }
                        } else {
                            unreachable!();
                        }
                    }
                } else {
                    unreachable!();
                }
            }
            return result;
        }
        unreachable!()
    }
}
