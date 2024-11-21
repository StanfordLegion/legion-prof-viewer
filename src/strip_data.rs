use crate::data::{
    DataSource, DataSourceDescription, DataSourceInfo, EntryID, Field, ItemLink, SlotMetaTile,
    SlotTile, SummaryTile, TileID,
};

pub struct StripDataSource<T: DataSource> {
    data_source: T,
}

impl<T: DataSource> StripDataSource<T> {
    pub fn new(data_source: T) -> Self {
        Self { data_source }
    }
}

impl<T: DataSource> DataSource for StripDataSource<T> {
    fn fetch_description(&self) -> DataSourceDescription {
        self.data_source.fetch_description()
    }

    fn fetch_info(&self) -> DataSourceInfo {
        self.data_source.fetch_info()
    }

    fn fetch_summary_tile(&self, entry_id: &EntryID, tile_id: TileID, full: bool) -> SummaryTile {
        self.data_source.fetch_summary_tile(entry_id, tile_id, full)
    }

    fn fetch_slot_tile(&self, entry_id: &EntryID, tile_id: TileID, full: bool) -> SlotTile {
        self.data_source.fetch_slot_tile(entry_id, tile_id, full)
    }

    fn fetch_slot_meta_tile(
        &self,
        entry_id: &EntryID,
        tile_id: TileID,
        full: bool,
    ) -> SlotMetaTile {
        let mut tile = self
            .data_source
            .fetch_slot_meta_tile(entry_id, tile_id, full);
        for row in &mut tile.data.items {
            for item in row {
                item.title = "Redacted".to_string();
                for field in &mut item.fields {
                    match field.1 {
                        Field::I64(_) => {}
                        Field::U64(_) => {}
                        Field::String(ref mut x) => {
                            *x = "Redacted".to_string();
                        }
                        Field::Interval(_) => {}
                        Field::ItemLink(ItemLink { ref mut title, .. }) => {
                            *title = "Redacted".to_string();
                        }
                        Field::Vec(_) => {
                            todo!()
                        }
                        Field::Empty => {}
                    }
                }
            }
        }
        tile
    }
}
