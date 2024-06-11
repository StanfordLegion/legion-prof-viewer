#![warn(clippy::all, rust_2018_idioms)]
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")] // hide console window on Windows in release

use egui::{Color32, NumExt};
use rand::Rng;
use std::collections::BTreeMap;

use legion_prof_viewer::data::{
    DataSourceDescription, DataSourceInfo, DataSourceMut, EntryID, EntryInfo, Field, FieldID,
    FieldSchema, Item, ItemMeta, ItemUID, SlotMetaTile, SlotMetaTileData, SlotTile, SlotTileData,
    SummaryTile, SummaryTileData, TileID, TileSet, UtilPoint,
};

#[cfg(not(target_arch = "wasm32"))]
use legion_prof_viewer::deferred_data::DeferredDataSourceWrapper;
use legion_prof_viewer::timestamp::{Interval, Timestamp};

#[cfg(target_arch = "wasm32")]
use legion_prof_viewer::http::client::HTTPClientDataSource;
#[cfg(target_arch = "wasm32")]
use url::Url;

#[cfg(target_arch = "wasm32")]
const DEFAULT_URL: &str = "http://127.0.0.1:8080";

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    legion_prof_viewer::app::start(vec![Box::new(DeferredDataSourceWrapper::new(
        RandomDataSource::new(),
    ))]);
}

#[cfg(target_arch = "wasm32")]
fn main() {
    let loc: web_sys::Location = web_sys::window().unwrap().location();
    let href: String = loc.href().expect("Unable to get window URL");
    let browser_url = Url::parse(&href).expect("Unable to parse location URL");

    let url = Url::parse(
        browser_url
            .query_pairs()
            .find(|(key, _)| key == "url")
            .map(|(_, value)| value)
            .as_deref()
            .unwrap_or(DEFAULT_URL),
    )
    .expect("Unable to parse query URL");

    legion_prof_viewer::app::start(vec![Box::new(HTTPClientDataSource::new(url))]);
}

type SlotCacheTile = (Vec<Vec<Item>>, Vec<Vec<ItemMeta>>);

struct ItemUIDGenerator {
    next: ItemUID,
}

impl Default for ItemUIDGenerator {
    fn default() -> Self {
        Self { next: ItemUID(0) }
    }
}

impl ItemUIDGenerator {
    fn next(&mut self) -> ItemUID {
        let result = self.next;
        self.next.0 += 1;
        result
    }
}

struct RandomDataSource {
    info: DataSourceInfo,
    item_uid_field: FieldID,
    interval_field: FieldID,
    summary_cache: BTreeMap<EntryID, Vec<UtilPoint>>,
    slot_cache: BTreeMap<EntryID, SlotCacheTile>,
    rng: rand::rngs::ThreadRng,
    item_uid_generator: ItemUIDGenerator,
}

impl RandomDataSource {
    fn new() -> Self {
        let mut rng = rand::rngs::ThreadRng::default();
        let entry_info = Self::entry_info(&mut rng);
        let mut field_schema = FieldSchema::new();
        let item_uid_field = field_schema.insert("Item UID".to_owned(), false);
        let interval_field = field_schema.insert("Interval".to_owned(), false);

        let info = DataSourceInfo {
            entry_info,
            interval: Self::interval(&mut rng),
            tile_set: TileSet::default(),
            field_schema,
            warning_message: Some("Demo only. The data in this profile is synthetic.".to_string()),
        };

        Self {
            info,
            item_uid_field,
            interval_field,
            summary_cache: BTreeMap::new(),
            slot_cache: BTreeMap::new(),
            rng,
            item_uid_generator: ItemUIDGenerator::default(),
        }
    }

    fn interval(rng: &mut rand::rngs::ThreadRng) -> Interval {
        Interval::new(Timestamp(0), Timestamp(rng.gen_range(1_000_000..2_000_000)))
    }

    fn generate_point(
        &mut self,
        first: UtilPoint,
        last: UtilPoint,
        level: i32,
        max_level: i32,
        utilization: &mut Vec<UtilPoint>,
    ) {
        let time = Timestamp((first.time.0 + last.time.0) / 2);
        let util = (first.util + last.util) * 0.5;
        let diff = (self.rng.gen::<f32>() - 0.5) / 1.2_f32.powi(max_level - level);
        let util = (util + diff).at_least(0.0).at_most(1.0);
        let point = UtilPoint { time, util };
        if level > 0 {
            self.generate_point(first, point, level - 1, max_level, utilization);
        }
        utilization.push(point);
        if level > 0 {
            self.generate_point(point, last, level - 1, max_level, utilization);
        }
    }

    fn generate_summary(&mut self, entry_id: &EntryID) -> &Vec<UtilPoint> {
        if !self.summary_cache.contains_key(entry_id) {
            const LEVELS: i32 = 8;
            let first = UtilPoint {
                time: self.info.interval.start,
                util: self.rng.gen(),
            };
            let last = UtilPoint {
                time: self.info.interval.stop,
                util: self.rng.gen(),
            };
            let mut utilization = Vec::new();
            utilization.push(first);
            self.generate_point(first, last, LEVELS, LEVELS, &mut utilization);
            utilization.push(last);

            self.summary_cache.insert(entry_id.clone(), utilization);
        }
        self.summary_cache.get(entry_id).unwrap()
    }

    fn generate_slot(&mut self, entry_id: &EntryID) -> &SlotCacheTile {
        if !self.slot_cache.contains_key(entry_id) {
            let entry = self.info.entry_info.get(entry_id);

            let max_rows = if let EntryInfo::Slot { max_rows, .. } = entry.unwrap() {
                max_rows
            } else {
                panic!("trying to fetch tile on something that is not a slot")
            };

            let mut items = Vec::new();
            let mut item_metas = Vec::new();
            for row in 0..*max_rows {
                let mut row_items = Vec::new();
                let mut row_item_metas = Vec::new();
                const N: u64 = 1000;
                for i in 0..N {
                    let start = self.info.interval.lerp((i as f32 + 0.05) / (N as f32));
                    let stop = self.info.interval.lerp((i as f32 + 0.95) / (N as f32));

                    let color = match (row * N + i) % 7 {
                        0 => Color32::BLUE,
                        1 => Color32::GREEN,
                        2 => Color32::RED,
                        3 => Color32::YELLOW,
                        4 => Color32::KHAKI,
                        5 => Color32::DARK_GREEN,
                        6 => Color32::DARK_BLUE,
                        _ => Color32::WHITE,
                    };

                    let item_uid = self.item_uid_generator.next();
                    row_items.push(Item {
                        item_uid,
                        interval: Interval::new(start, stop),
                        color,
                    });
                    row_item_metas.push(ItemMeta {
                        item_uid,
                        original_interval: Interval::new(start, stop),
                        title: "Test Item".to_owned(),
                        fields: vec![
                            (
                                self.interval_field,
                                Field::Interval(Interval::new(start, stop)),
                            ),
                            (self.item_uid_field, Field::U64(item_uid.0)),
                        ],
                    });
                }
                items.push(row_items);
                item_metas.push(row_item_metas);
            }

            self.slot_cache
                .insert(entry_id.clone(), (items, item_metas));
        }
        self.slot_cache.get(entry_id).unwrap()
    }

    fn entry_info(rng: &mut rand::rngs::ThreadRng) -> EntryInfo {
        let kinds = [
            "CPU".to_string(),
            "GPU".to_string(),
            "OMP".to_string(),
            "Py".to_string(),
            "Util".to_string(),
            "Chan".to_string(),
            "SysMem".to_string(),
        ];

        const NODES: i32 = 8192;
        const PROCS: i32 = 8;
        let mut node_slots = Vec::new();
        for node in 0..NODES {
            let mut kind_slots = Vec::new();
            let colors = &[Color32::BLUE, Color32::GREEN, Color32::RED, Color32::YELLOW];
            for (i, kind) in kinds.iter().enumerate() {
                let color = colors[i % colors.len()];
                let mut proc_slots = Vec::new();
                for proc in 0..PROCS {
                    let rows: u64 = rng.gen_range(0..64);
                    proc_slots.push(EntryInfo::Slot {
                        short_name: format!(
                            "{}{}",
                            kind.chars().next().unwrap().to_lowercase(),
                            proc
                        ),
                        long_name: format!("Node {node} {kind} {proc}"),
                        max_rows: rows,
                    });
                }
                kind_slots.push(EntryInfo::Panel {
                    short_name: kind.to_lowercase(),
                    long_name: format!("Node {node} {kind}"),
                    summary: Some(Box::new(EntryInfo::Summary { color })),
                    slots: proc_slots,
                });
            }
            node_slots.push(EntryInfo::Panel {
                short_name: format!("n{node}"),
                long_name: format!("Node {node}"),
                summary: None,
                slots: kind_slots,
            });
        }
        EntryInfo::Panel {
            short_name: "root".to_owned(),
            long_name: "root".to_owned(),
            summary: None,
            slots: node_slots,
        }
    }
}

impl DataSourceMut for RandomDataSource {
    fn fetch_description(&self) -> DataSourceDescription {
        DataSourceDescription {
            source_locator: vec!["Random Data Source".to_string()],
        }
    }
    fn fetch_info(&mut self) -> DataSourceInfo {
        self.info.clone()
    }

    fn fetch_summary_tile(
        &mut self,
        entry_id: &EntryID,
        tile_id: TileID,
        _full: bool,
    ) -> SummaryTile {
        let utilization = self.generate_summary(entry_id);

        let mut tile_utilization = Vec::new();
        let mut last_point = None;
        for point in utilization {
            let UtilPoint { time, util } = *point;
            if let Some(last_point) = last_point {
                let UtilPoint {
                    time: last_time,
                    util: last_util,
                } = last_point;

                let last_interval = Interval::new(last_time, time);
                if last_interval.contains(tile_id.0.start) {
                    let relative = last_interval.unlerp(tile_id.0.start);
                    let start_util = (last_util - util) * relative + last_util;
                    tile_utilization.push(UtilPoint {
                        time: tile_id.0.start,
                        util: start_util,
                    });
                }
                if tile_id.0.contains(time) {
                    tile_utilization.push(*point);
                }
                if last_interval.contains(tile_id.0.stop) {
                    let relative = last_interval.unlerp(tile_id.0.stop);
                    let stop_util = (last_util - util) * relative + last_util;
                    tile_utilization.push(UtilPoint {
                        time: tile_id.0.stop,
                        util: stop_util,
                    });
                }
            }

            last_point = Some(*point);
        }
        SummaryTile {
            entry_id: entry_id.clone(),
            tile_id,
            data: SummaryTileData {
                utilization: tile_utilization,
            },
        }
    }

    fn fetch_slot_tile(&mut self, entry_id: &EntryID, tile_id: TileID, _full: bool) -> SlotTile {
        let items = &self.generate_slot(entry_id).0;

        let mut slot_items = Vec::new();
        for row in items {
            let mut slot_row = Vec::new();
            for item in row {
                // When the item straddles a tile boundary, it has to be
                // sliced to fit
                if tile_id.0.overlaps(item.interval) {
                    let mut new_item = item.clone();
                    new_item.interval = new_item.interval.intersection(tile_id.0);
                    slot_row.push(new_item);
                }
            }
            slot_items.push(slot_row);
        }

        SlotTile {
            entry_id: entry_id.clone(),
            tile_id,
            data: SlotTileData { items: slot_items },
        }
    }

    fn fetch_slot_meta_tile(
        &mut self,
        entry_id: &EntryID,
        tile_id: TileID,
        _full: bool,
    ) -> SlotMetaTile {
        let (items, item_metas) = &self.generate_slot(entry_id);

        let mut slot_items = Vec::new();
        for (row, row_meta) in items.iter().zip(item_metas.iter()) {
            let mut slot_row = Vec::new();
            for (item, item_meta) in row.iter().zip(row_meta.iter()) {
                // When the item straddles a tile boundary, it has to be
                // sliced to fit
                if tile_id.0.overlaps(item.interval) {
                    slot_row.push(item_meta.clone());
                }
            }
            slot_items.push(slot_row);
        }

        SlotMetaTile {
            entry_id: entry_id.clone(),
            tile_id,
            data: SlotMetaTileData { items: slot_items },
        }
    }
}
