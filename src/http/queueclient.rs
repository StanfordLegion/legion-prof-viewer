use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use reqwest::header::Entry;
use serde::{Deserialize, Serialize};

use crate::{
    data::{
        DataSource, EntryID, EntryInfo, Initializer, SlotMetaTile, SlotTile, SummaryTile, TileID,
    },
    deferred_data::DeferredDataSource,
    logging::*,
    queue::queue::{ProcessType, Work},
    timestamp::Interval,
};
use ehttp::{self, headers, Request};

use super::schema::{FetchMultipleRequest, FetchRequest, FetchTilesRequest};

pub struct HTTPQueueDataSource {
    pub host: String,
    pub port: u16,
    pub client: reqwest::Client,
    pub queue: Arc<Mutex<Vec<Work>>>,
    pub fetch_info: EntryInfo,
    pub initializer: Initializer,
    pub interval: Interval,
    fetch_tiles_cache: BTreeMap<EntryID, Vec<TileID>>,
    fetch_summary_tiles_cache: Vec<SummaryTile>,
    fetch_slot_tiles_cache: Vec<SlotTile>,
    fetch_slot_meta_tiles_cache: Vec<SlotMetaTile>,
}

impl HTTPQueueDataSource {
    pub fn new(
        host: String,
        port: u16,
        queue: Arc<Mutex<Vec<Work>>>,
        initializer: Initializer,
    ) -> Self {
        log("INIT HTTPQueueDataSource");
        Self {
            host,
            port,
            client: reqwest::ClientBuilder::new()
                // .timeout(std::time::Duration::from_secs(5))
                // .gzip(true)
                // .brotli(true)
                .build()
                .unwrap(),
            // queue: Arc::new(Mutex::new(Vec::new())),
            queue,
            fetch_info: initializer.clone().entry_info,
            initializer: initializer.clone(),
            interval: Interval::default(),
            fetch_tiles_cache: BTreeMap::new(),
            fetch_summary_tiles_cache: Vec::new(),
            fetch_slot_meta_tiles_cache: Vec::new(),
            fetch_slot_tiles_cache: Vec::new(),
        }
    }

    // empty queue and add results to respective caches
    fn process_queue(&mut self) {
        // log("process_queue");
        let mut q = self.queue.lock().unwrap();

        for work in q.iter() {
            match work.process_type {
                ProcessType::FETCH_SLOT_META_TILE => {
                    // deserialize work.data into SlotMetaTile
                    let smt = serde_json::from_str::<SlotMetaTile>(&work.data).unwrap();
                    // add to cache or create new vector

                    self.fetch_slot_meta_tiles_cache.push(smt.clone());
                }
                ProcessType::FETCH_SLOT_TILE => {
                    // deserialize work.data into SlotTile
                    let st = serde_json::from_str::<SlotTile>(&work.data).unwrap();
                    // add to cache
                    self.fetch_slot_tiles_cache.push(st.clone());
                }

                ProcessType::FETCH_TILES => {
                    // deserialize work.data into Vec<TileID>
                    let tiles = serde_json::from_str::<Vec<TileID>>(&work.data).unwrap();
                    // add to cache
                    self.fetch_tiles_cache
                        .entry(work.entry_id.clone())
                        .or_insert(tiles.clone())
                        .extend(tiles.clone());
                }
                ProcessType::FETCH_SUMMARY_TILE => {
                    // deserialize work.data into SummaryTile
                    let st = serde_json::from_str::<SummaryTile>(&work.data).unwrap();
                    // add to cache
                    self.fetch_summary_tiles_cache.push(st.clone());
                }
                ProcessType::INTERVAL => {
                    // deserialize work.data into Interval
                    let interval = serde_json::from_str::<Interval>(&work.data).unwrap();
                    // add to cache
                    self.interval = interval;

                    // clear all the caches
                    // self.FETCH_TILES_cache.clear();
                    // self.fetch_summary_tiles_cache.clear();
                    // self.fetch_slot_meta_tiles_cache.clear();
                    // self.fetch_slot_tiles_cache.clear();
                }
            }
        }
        // empty queue
        q.clear(); // ?
    }

    fn queue_work(&mut self, work: Work) {
        // log("queue_work");
        let _work = work.clone();
        let url = match work.process_type {
            ProcessType::FETCH_SLOT_META_TILE => {
                format!("http://{}:{}/slot_meta_tile", self.host, self.port)
            }
            ProcessType::FETCH_SLOT_TILE => format!("http://{}:{}/slot_tile", self.host, self.port),
            ProcessType::FETCH_TILES => format!("http://{}:{}/tiles", self.host, self.port),
            ProcessType::FETCH_SUMMARY_TILE => {
                format!("http://{}:{}/summary_tile", self.host, self.port)
            }
            ProcessType::INTERVAL => format!("http://{}:{}/interval", self.host, self.port),
        };

        let body = match work.process_type {
            ProcessType::FETCH_SLOT_META_TILE => serde_json::to_string(&FetchRequest {
                entry_id: work.entry_id.clone(),
                tile_id: work.tile_id.unwrap(),
            })
            .unwrap(),
            ProcessType::FETCH_SLOT_TILE => serde_json::to_string(&FetchRequest {
                entry_id: work.entry_id.clone(),
                tile_id: work.tile_id.unwrap(),
            })
            .unwrap(),
            ProcessType::FETCH_TILES => serde_json::to_string(&FetchTilesRequest {
                entry_id: work.entry_id.clone(),
                interval: work.interval.unwrap(),
            })
            .unwrap(),
            ProcessType::FETCH_SUMMARY_TILE => serde_json::to_string(&FetchRequest {
                entry_id: work.entry_id.clone(),
                tile_id: work.tile_id.unwrap(),
            })
            .unwrap(),
            ProcessType::INTERVAL => "".to_string(),
        };

        let request = Request {
            method: "POST".to_owned(),
            url: url.to_string(),
            body: body.into(),
            headers: headers(&[("Accept", "*/*"), ("Content-Type", "javascript/json;")]),
        };
        // request.body = body.into();

        // log(&url.clone());
        let queue = self.queue.clone();
        ehttp::fetch(request, move |result: ehttp::Result<ehttp::Response>| {
            // deserialize response into a vector of TileIDs

            let work = Work {
                entry_id: work.entry_id.clone(),
                tile_id: work.tile_id,
                tile_ids: _work.tile_ids.clone(),
                interval: work.interval,
                data: result.unwrap().text().unwrap().to_string(),
                process_type: work.process_type,
            };

            // console_log!("ASYNC: pushing new work to queue: {:?}", work);
            queue.lock().unwrap().push(work);
        });
    }
}

impl DeferredDataSource for HTTPQueueDataSource {
    fn interval(&mut self) -> Interval {
        self.process_queue();
        let work = Work {
            entry_id: EntryID::root(),
            tile_id: None,
            tile_ids: None,
            interval: None,
            data: "".to_string(),
            process_type: ProcessType::INTERVAL,
        };
        self.queue_work(work);
        self.interval
    }
    fn fetch_info(&mut self) -> EntryInfo {
        self.process_queue();

        self.fetch_info.clone()
    }

    fn init(&mut self) -> crate::data::Initializer {
        self.initializer.clone()
    }
    fn fetch_tiles(&mut self, entry_id: EntryID, request_interval: Interval) {
        self.process_queue();
        // queue work
        let work = Work {
            entry_id: entry_id.clone(),
            tile_id: None,
            tile_ids: None,
            interval: Some(request_interval),
            data: "".to_string(),
            process_type: ProcessType::FETCH_TILES,
        };
        self.queue_work(work);
    }

    fn get_tiles(&mut self, entry_id: EntryID) -> Vec<TileID> {
        self.process_queue();
        if let Some(tiles) = self.fetch_tiles_cache.get(&(entry_id.clone())) {
            return tiles.to_vec();
        } else {
            return vec![];
        }
    }

    fn fetch_summary_tile(&mut self, entry_id: EntryID, tile_id: TileID) {
        // queue work
        self.process_queue();
        let work = Work {
            entry_id: entry_id.clone(),
            tile_id: Some(tile_id),
            interval: None,
            tile_ids: None,
            data: "".to_string(),
            process_type: ProcessType::FETCH_SUMMARY_TILE,
        };
        self.queue_work(work);
    }

    fn get_summary_tiles(&mut self) -> Vec<SummaryTile> {
        self.process_queue();

        let tiles = self.fetch_summary_tiles_cache.clone();
        self.fetch_summary_tiles_cache.clear();
        tiles
    }
    fn fetch_slot_tile(&mut self, entry_id: EntryID, tile_id: TileID) {
        self.process_queue();
        // queue work
        let work = Work {
            entry_id: entry_id.clone(),
            tile_id: Some(tile_id),
            interval: None,
            tile_ids: None,
            data: "".to_string(),
            process_type: ProcessType::FETCH_SLOT_TILE,
        };
        self.queue_work(work);
    }

    fn get_slot_tile(&mut self) -> Vec<SlotTile> {
        self.process_queue();

        let tiles = self.fetch_slot_tiles_cache.clone();
        self.fetch_slot_tiles_cache.clear();
        tiles
    }

    fn fetch_slot_meta_tile(&mut self, entry_id: EntryID, tile_id: TileID) {
        self.process_queue();
        // check cache

        // queue work
        let work = Work {
            entry_id: entry_id.clone(),
            tile_id: Some(tile_id),
            tile_ids: None,
            interval: None,
            data: "".to_string(),
            process_type: ProcessType::FETCH_SLOT_META_TILE,
        };
        self.queue_work(work);
    }

    fn get_slot_meta_tile(&mut self) -> Vec<SlotMetaTile> {
        self.process_queue();

        let tiles = self.fetch_slot_meta_tiles_cache.clone();
        self.fetch_slot_meta_tiles_cache.clear();
        tiles
    }
}