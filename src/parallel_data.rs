use std::sync::{Arc, Mutex};

use crate::data::{
    DataSource, DataSourceInfo, EntryID, SlotMetaTile, SlotTile, SummaryTile, TileID, TileSet,
};
use crate::deferred_data::DeferredDataSource;

pub struct ParallelDeferredDataSource<T: DataSource + Send + Sync + 'static> {
    data_source: Arc<T>,
    infos: Arc<Mutex<Vec<DataSourceInfo>>>,
    tile_sets: Arc<Mutex<Vec<TileSet>>>,
    summary_tiles: Arc<Mutex<Vec<SummaryTile>>>,
    slot_tiles: Arc<Mutex<Vec<SlotTile>>>,
    slot_meta_tiles: Arc<Mutex<Vec<SlotMetaTile>>>,
}

impl<T: DataSource + Send + Sync + 'static> ParallelDeferredDataSource<T> {
    pub fn new(data_source: T) -> Self {
        Self {
            data_source: Arc::new(data_source),
            infos: Arc::new(Mutex::new(Vec::new())),
            tile_sets: Arc::new(Mutex::new(Vec::new())),
            summary_tiles: Arc::new(Mutex::new(Vec::new())),
            slot_tiles: Arc::new(Mutex::new(Vec::new())),
            slot_meta_tiles: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl<T: DataSource + Send + Sync + 'static> DeferredDataSource for ParallelDeferredDataSource<T> {
    fn fetch_info(&mut self) {
        let data_source = self.data_source.clone();
        let infos = self.infos.clone();
        rayon::spawn(move || {
            let result = data_source.fetch_info();
            infos.lock().unwrap().push(result);
        });
    }

    fn get_infos(&mut self) -> Vec<DataSourceInfo> {
        std::mem::take(&mut self.infos.lock().unwrap())
    }

    fn fetch_tile_set(&mut self) {
        let data_source = self.data_source.clone();
        let tile_sets = self.tile_sets.clone();
        rayon::spawn(move || {
            let result = data_source.fetch_tile_set();
            tile_sets.lock().unwrap().push(result);
        });
    }

    fn get_tile_sets(&mut self) -> Vec<TileSet> {
        std::mem::take(&mut self.tile_sets.lock().unwrap())
    }

    fn fetch_summary_tile(&mut self, entry_id: &EntryID, tile_id: TileID) {
        let entry_id = entry_id.clone();
        let data_source = self.data_source.clone();
        let summary_tiles = self.summary_tiles.clone();
        rayon::spawn(move || {
            let result = data_source.fetch_summary_tile(&entry_id, tile_id);
            summary_tiles.lock().unwrap().push(result);
        });
    }

    fn get_summary_tiles(&mut self) -> Vec<SummaryTile> {
        std::mem::take(&mut self.summary_tiles.lock().unwrap())
    }

    fn fetch_slot_tile(&mut self, entry_id: &EntryID, tile_id: TileID) {
        let entry_id = entry_id.clone();
        let data_source = self.data_source.clone();
        let slot_tiles = self.slot_tiles.clone();
        rayon::spawn(move || {
            let result = data_source.fetch_slot_tile(&entry_id, tile_id);
            slot_tiles.lock().unwrap().push(result);
        });
    }

    fn get_slot_tiles(&mut self) -> Vec<SlotTile> {
        std::mem::take(&mut self.slot_tiles.lock().unwrap())
    }

    fn fetch_slot_meta_tile(&mut self, entry_id: &EntryID, tile_id: TileID) {
        let entry_id = entry_id.clone();
        let data_source = self.data_source.clone();
        let slot_meta_tiles = self.slot_meta_tiles.clone();
        rayon::spawn(move || {
            let result = data_source.fetch_slot_meta_tile(&entry_id, tile_id);
            slot_meta_tiles.lock().unwrap().push(result);
        });
    }

    fn get_slot_meta_tiles(&mut self) -> Vec<SlotMetaTile> {
        std::mem::take(&mut self.slot_meta_tiles.lock().unwrap())
    }
}
