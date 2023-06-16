use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;
use std::time::Duration;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

use egui::{
    Align2, Color32, NumExt, Pos2, Rect, RichText, ScrollArea, Slider, Stroke, TextStyle, Vec2,
};
use serde::{Deserialize, Serialize};

use crate::data::{
    DataSourceInfo, EntryID, EntryIndex, EntryInfo, Field, ItemMeta, ItemUID, SlotMetaTileData,
    SlotTileData, SummaryTileData, TileID, UtilPoint,
};
use crate::deferred_data::{CountingDeferredDataSource, DeferredDataSource};
use crate::timestamp::{Interval, Timestamp, TimestampParseError};

/// Overview:
///   ProfApp -> Context, Window *
///   Window -> Config, Panel
///   Panel -> Summary, { Panel | Slot } *
///   Summary
///   Slot -> Item *
///
/// Context:
///   * Global configuration state (i.e., for all profiles)
///
/// Window:
///   * One Windows per profile
///   * Owns the ScrollArea (there is only **ONE** ScrollArea)
///   * Handles pan/zoom (there is only **ONE** pan/zoom setting)
///
/// Config:
///   * Window configuration state (i.e., specific to a profile)
///
/// Panel:
///   * One Panel for each level of nesting in the profile (root, node, kind)
///   * Table widget for (nested) cells
///   * Each row contains: label, content
///
/// Summary:
///   * Utilization widget
///
/// Slot:
///   * One Slot for each processor, channel, memory
///   * Viewer widget for items

struct Summary {
    entry_id: EntryID,
    color: Color32,
    tiles: BTreeMap<TileID, Option<SummaryTileData>>,
    last_view_interval: Option<Interval>,
}

struct Slot {
    entry_id: EntryID,
    short_name: String,
    long_name: String,
    expanded: bool,
    max_rows: u64,
    tile_ids: Vec<TileID>,
    tiles: BTreeMap<TileID, Option<SlotTileData>>,
    tile_metas: BTreeMap<TileID, Option<SlotMetaTileData>>,
    last_view_interval: Option<Interval>,
}

struct Panel<S: Entry> {
    entry_id: EntryID,
    short_name: String,
    long_name: String,
    expanded: bool,

    summary: Option<Summary>,
    slots: Vec<S>,
}

#[derive(Debug)]
struct SearchCacheItem {
    // For horizontal scroll, we need the item's interval
    interval: Interval,

    // For vertical scroll, we need the item's location
    entry_id: EntryID,
    tile_id: TileID,
    row: usize,
    col: usize,

    // Cache fields for display
    title: String,
}

#[derive(Default)]
struct SearchState {
    // Search parameters
    query: String,
    last_query: String,
    include_collapsed_entries: bool,

    // Cache of matching items
    result_set: BTreeSet<ItemUID>,
    result_cache: BTreeMap<EntryID, BTreeMap<TileID, BTreeMap<ItemUID, SearchCacheItem>>>,
}

struct Config {
    // Node selection controls
    min_node: u64,
    max_node: u64,

    // This is just for the local profile
    interval: Interval,

    data_source: CountingDeferredDataSource<Box<dyn DeferredDataSource>>,

    search_state: SearchState,
}

struct Window {
    panel: Panel<Panel<Panel<Slot>>>, // nodes -> kind -> proc/chan/mem
    index: u64,
    kinds: Vec<String>,
    config: Config,
}

#[derive(Default, Deserialize, Serialize)]
struct ZoomState {
    levels: Vec<Interval>,
    index: usize,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
enum IntervalSelectError {
    InvalidValue,
    NoUnit,
    InvalidUnit,
    StartAfterStop,
    StartAfterEnd,
    StopBeforeStart,
}

impl From<TimestampParseError> for IntervalSelectError {
    fn from(val: TimestampParseError) -> Self {
        match val {
            TimestampParseError::InvalidValue => IntervalSelectError::InvalidValue,
            TimestampParseError::NoUnit => IntervalSelectError::NoUnit,
            TimestampParseError::InvalidUnit => IntervalSelectError::InvalidUnit,
        }
    }
}

impl fmt::Display for IntervalSelectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IntervalSelectError::InvalidValue => write!(f, "invalid value"),
            IntervalSelectError::NoUnit => write!(f, "no unit"),
            IntervalSelectError::InvalidUnit => write!(f, "invalid unit"),
            IntervalSelectError::StartAfterStop => write!(f, "start after stop"),
            IntervalSelectError::StartAfterEnd => write!(f, "start after end"),
            IntervalSelectError::StopBeforeStart => write!(f, "stop before start"),
        }
    }
}

#[derive(Default)]
struct IntervalSelectState {
    // User-entered strings for the interval start/stop.
    start_buffer: String,
    stop_buffer: String,

    // Parse errors for the respective strings (if any).
    start_error: Option<IntervalSelectError>,
    stop_error: Option<IntervalSelectError>,
}

#[derive(Default, Deserialize, Serialize)]
struct Context {
    row_height: f32,

    subheading_size: f32,

    // This is across all profiles
    total_interval: Interval,

    // Visible time range
    view_interval: Interval,

    drag_origin: Option<Pos2>,

    // Hack: We need to track the screenspace rect where slot/summary
    // data gets drawn. This gets used rendering the cursor, but we
    // only know it when we render slots. So stash it here.
    slot_rect: Option<Rect>,

    toggle_dark_mode: bool,

    debug: bool,

    zoom_state: ZoomState,
    #[serde(skip)]
    interval_state: IntervalSelectState,
}

#[derive(Default, Deserialize, Serialize)]
#[serde(default)] // deserialize missing fields as default value
struct ProfApp {
    // Data sources waiting to be turned into windows.
    #[serde(skip)]
    pending_data_sources: VecDeque<Box<dyn DeferredDataSource>>,

    #[serde(skip)]
    windows: Vec<Window>,

    cx: Context,

    #[cfg(not(target_arch = "wasm32"))]
    #[serde(skip)]
    last_update: Option<Instant>,
}

trait Entry {
    fn new(info: &EntryInfo, entry_id: EntryID) -> Self;

    fn entry_id(&self) -> &EntryID;
    fn label_text(&self) -> &str;
    fn hover_text(&self) -> &str;

    fn find_slot(&mut self, entry_id: &EntryID, level: u64) -> Option<&mut Slot>;
    fn find_summary(&mut self, entry_id: &EntryID, level: u64) -> Option<&mut Summary>;

    fn inflate_meta(&mut self, config: &mut Config, cx: &mut Context);

    fn search(&mut self, config: &mut Config, cx: &mut Context);

    fn label(&mut self, ui: &mut egui::Ui, rect: Rect) {
        let response = ui.allocate_rect(
            rect,
            if self.is_expandable() {
                egui::Sense::click()
            } else {
                egui::Sense::hover()
            },
        );

        let style = ui.style();
        let font_id = TextStyle::Body.resolve(style);
        let visuals = if self.is_expandable() {
            style.interact_selectable(&response, false)
        } else {
            *style.noninteractive()
        };

        ui.painter()
            .rect(rect, 0.0, visuals.bg_fill, visuals.bg_stroke);
        ui.painter().text(
            rect.min + style.spacing.item_spacing,
            Align2::LEFT_TOP,
            self.label_text(),
            font_id,
            visuals.text_color(),
        );

        if response.clicked() {
            // This will take effect next frame because we can't redraw this widget now
            self.toggle_expanded();
        } else if response.hovered() {
            response.on_hover_text(self.hover_text());
        }
    }

    fn content(
        &mut self,
        ui: &mut egui::Ui,
        rect: Rect,
        viewport: Rect,
        config: &mut Config,
        cx: &mut Context,
    );

    fn height(&self, config: &Config, cx: &Context) -> f32;

    fn is_expandable(&self) -> bool;

    fn toggle_expanded(&mut self);
}

impl Summary {
    fn clear(&mut self) {
        self.tiles.clear();
    }

    fn inflate(&mut self, config: &mut Config, cx: &mut Context) {
        for tile_id in config.request_tiles(cx.view_interval) {
            config
                .data_source
                .fetch_summary_tile(&self.entry_id, tile_id);
            self.tiles.insert(tile_id, None);
        }
    }
}

impl Entry for Summary {
    fn new(info: &EntryInfo, entry_id: EntryID) -> Self {
        if let EntryInfo::Summary { color } = info {
            Self {
                entry_id,
                color: *color,
                tiles: BTreeMap::new(),
                last_view_interval: None,
            }
        } else {
            unreachable!()
        }
    }

    fn entry_id(&self) -> &EntryID {
        &self.entry_id
    }
    fn label_text(&self) -> &str {
        "avg"
    }
    fn hover_text(&self) -> &str {
        "Utilization Plot of Average Usage Over Time"
    }

    fn find_slot(&mut self, _entry_id: &EntryID, _level: u64) -> Option<&mut Slot> {
        unreachable!()
    }

    fn find_summary(&mut self, entry_id: &EntryID, level: u64) -> Option<&mut Summary> {
        assert_eq!(entry_id.level(), level);
        assert_eq!(entry_id.index(level - 1)?, EntryIndex::Summary);
        Some(self)
    }

    fn inflate_meta(&mut self, _config: &mut Config, _cx: &mut Context) {}

    fn search(&mut self, _config: &mut Config, _cx: &mut Context) {}

    fn content(
        &mut self,
        ui: &mut egui::Ui,
        rect: Rect,
        _viewport: Rect,
        config: &mut Config,
        cx: &mut Context,
    ) {
        cx.slot_rect = Some(rect); // Save slot rect for use later

        const TOOLTIP_RADIUS: f32 = 4.0;
        let response = ui.allocate_rect(rect, egui::Sense::hover());
        let hover_pos = response.hover_pos(); // where is the mouse hovering?

        if self
            .last_view_interval
            .map_or(true, |i| i != cx.view_interval)
        {
            self.clear();
        }
        self.last_view_interval = Some(cx.view_interval);
        if self.tiles.is_empty() {
            self.inflate(config, cx);
        }

        let style = ui.style();
        let visuals = style.interact_selectable(&response, false);
        ui.painter()
            .rect(rect, 0.0, visuals.bg_fill, visuals.bg_stroke);

        let stroke = Stroke::new(visuals.bg_stroke.width, self.color);

        // Conversions to and from screen space coordinates
        let util_to_screen = |util: &UtilPoint| {
            let time = cx.view_interval.unlerp(util.time);
            rect.lerp_inside(Vec2::new(time, 1.0 - util.util))
        };
        let screen_to_util = |screen: Pos2| UtilPoint {
            time: cx
                .view_interval
                .lerp((screen.x - rect.left()) / rect.width()),
            util: 1.0 - (screen.y - rect.top()) / rect.height(),
        };

        // Linear interpolation along the line from p1 to p2
        let interpolate = |p1: Pos2, p2: Pos2, x: f32| {
            let ratio = (x - p1.x) / (p2.x - p1.x);
            Rect::from_min_max(p1, p2).lerp_inside(Vec2::new(ratio, ratio))
        };

        let mut last_util: Option<&UtilPoint> = None;
        let mut last_point: Option<Pos2> = None;
        let mut hover_util = None;
        for tile in self.tiles.values().flatten() {
            for util in &tile.utilization {
                let mut point = util_to_screen(util);
                if let Some(mut last) = last_point {
                    let last_util = last_util.unwrap();
                    if cx
                        .view_interval
                        .overlaps(Interval::new(last_util.time, util.time))
                    {
                        // Interpolate when out of view
                        if last.x < rect.min.x {
                            last = interpolate(last, point, rect.min.x);
                        }
                        if point.x > rect.max.x {
                            point = interpolate(last, point, rect.max.x);
                        }

                        ui.painter().line_segment([last, point], stroke);

                        if let Some(hover) = hover_pos {
                            if last.x <= hover.x && hover.x < point.x {
                                let interp = interpolate(last, point, hover.x);
                                ui.painter().circle_stroke(
                                    interp,
                                    TOOLTIP_RADIUS,
                                    visuals.fg_stroke,
                                );
                                hover_util = Some(screen_to_util(interp));
                            }
                        }
                    }
                }

                last_point = Some(point);
                last_util = Some(util);
            }
        }

        if let Some(util) = hover_util {
            let time = cx.view_interval.unlerp(util.time);
            let util_rect = Rect::from_min_max(
                rect.lerp_inside(Vec2::new(time - 0.05, 0.0)),
                rect.lerp_inside(Vec2::new(time + 0.05, 1.0)),
            );
            ui.show_tooltip(
                "utilization_tooltip",
                &util_rect,
                format!("{:.0}% Utilization", util.util * 100.0),
            );
        }
    }

    fn height(&self, _config: &Config, cx: &Context) -> f32 {
        const ROWS: u64 = 4;
        ROWS as f32 * cx.row_height
    }

    fn is_expandable(&self) -> bool {
        false
    }

    fn toggle_expanded(&mut self) {
        unreachable!();
    }
}

impl Slot {
    fn rows(&self) -> u64 {
        const UNEXPANDED_ROWS: u64 = 2;
        if self.expanded {
            self.max_rows.at_least(UNEXPANDED_ROWS)
        } else {
            UNEXPANDED_ROWS
        }
    }

    fn clear(&mut self) {
        self.tile_ids.clear();
        self.tiles.clear();
        self.tile_metas.clear();
    }

    fn inflate(&mut self, config: &mut Config, cx: &mut Context) {
        for tile_id in config.request_tiles(cx.view_interval) {
            config.data_source.fetch_slot_tile(&self.entry_id, tile_id);
            self.tile_ids.push(tile_id);
            self.tiles.insert(tile_id, None);
        }
    }

    fn fetch_meta_tile(
        &mut self,
        tile_id: TileID,
        config: &mut Config,
    ) -> Option<&SlotMetaTileData> {
        self.tile_metas
            .entry(tile_id)
            .or_insert_with(|| {
                config
                    .data_source
                    .fetch_slot_meta_tile(&self.entry_id, tile_id);
                None
            })
            .as_ref()
    }

    #[allow(clippy::too_many_arguments)]
    fn render_tile(
        &mut self,
        tile_index: usize,
        rows: u64,
        mut hover_pos: Option<Pos2>,
        ui: &mut egui::Ui,
        rect: Rect,
        viewport: Rect,
        config: &mut Config,
        cx: &mut Context,
    ) -> Option<Pos2> {
        // Hack: can't pass this as an argument because it aliases self.
        let tile_id = self.tile_ids[tile_index];
        let tile = self.tiles.get(&tile_id).unwrap();

        if !tile.is_some() {
            // Tile hasn't finished loading.
            return hover_pos;
        }
        let tile = tile.as_ref().unwrap();

        if !cx.view_interval.overlaps(tile_id.0) {
            return hover_pos;
        }

        // Track which item, if any, we're interacting with
        let mut interact_item = None;

        for (row, row_items) in tile.items.iter().enumerate() {
            // Need to reverse the rows because we're working in screen space
            let irow = rows - (row as u64) - 1;

            // We want to do this first on rows, so that we can cut the
            // entire row if we don't need it

            // Compute bounds for the whole row
            let row_min = rect.lerp_inside(Vec2::new(0.0, (irow as f32 + 0.05) / rows as f32));
            let row_max = rect.lerp_inside(Vec2::new(1.0, (irow as f32 + 0.95) / rows as f32));

            // Cull if out of bounds
            // Note: need to shift by rect.min to get to viewport space
            if row_max.y - rect.min.y < viewport.min.y {
                break;
            } else if row_min.y - rect.min.y > viewport.max.y {
                continue;
            }

            // Check if mouse is hovering over this row
            let row_rect = Rect::from_min_max(row_min, row_max);
            let row_hover = hover_pos.map_or(false, |h| row_rect.contains(h));

            // Now handle the items
            for (item_idx, item) in row_items.iter().enumerate() {
                if !cx.view_interval.overlaps(item.interval) {
                    continue;
                }

                // Note: the interval is EXCLUSIVE. This turns out to be what
                // we want here, because in screen coordinates interval.stop
                // is the BEGINNING of the interval.stop nanosecond.
                let start = cx.view_interval.unlerp(item.interval.start).at_least(0.0);
                let stop = cx.view_interval.unlerp(item.interval.stop).at_most(1.0);
                let min = rect.lerp_inside(Vec2::new(start, (irow as f32 + 0.05) / rows as f32));
                let max = rect.lerp_inside(Vec2::new(stop, (irow as f32 + 0.95) / rows as f32));

                let item_rect = Rect::from_min_max(min, max);
                if row_hover && hover_pos.map_or(false, |h| item_rect.contains(h)) {
                    hover_pos = None;
                    interact_item = Some((row, item_idx, item_rect, tile_id));
                }
                ui.painter().rect(item_rect, 0.0, item.color, Stroke::NONE);
            }
        }

        if let Some((row, item_idx, item_rect, tile_id)) = interact_item {
            if let Some(tile_meta) = self.fetch_meta_tile(tile_id, config) {
                let item_meta = &tile_meta.items[row][item_idx];
                ui.show_tooltip_ui("task_tooltip", &item_rect, |ui| {
                    ui.label(&item_meta.title);
                    if cx.debug {
                        ui.label(format!("Item UID: {}", item_meta.item_uid.0));
                    }
                    for (name, field) in &item_meta.fields {
                        match field {
                            Field::I64(value) => {
                                ui.label(format!("{name}: {value}"));
                            }
                            Field::U64(value) => {
                                ui.label(format!("{name}: {value}"));
                            }
                            Field::String(value) => {
                                ui.label(format!("{name}: {value}"));
                            }
                            Field::Interval(value) => {
                                ui.label(format!("{name}: {value}"));
                            }
                            Field::Empty => {
                                ui.label(name);
                            }
                        }
                    }
                });
            }
        }

        hover_pos
    }
}

impl Entry for Slot {
    fn new(info: &EntryInfo, entry_id: EntryID) -> Self {
        if let EntryInfo::Slot {
            short_name,
            long_name,
            max_rows,
        } = info
        {
            Self {
                entry_id,
                short_name: short_name.to_owned(),
                long_name: long_name.to_owned(),
                expanded: true,
                max_rows: *max_rows,
                tile_ids: Vec::new(),
                tiles: BTreeMap::new(),
                tile_metas: BTreeMap::new(),
                last_view_interval: None,
            }
        } else {
            unreachable!()
        }
    }

    fn entry_id(&self) -> &EntryID {
        &self.entry_id
    }
    fn label_text(&self) -> &str {
        &self.short_name
    }
    fn hover_text(&self) -> &str {
        &self.long_name
    }

    fn find_slot(&mut self, entry_id: &EntryID, level: u64) -> Option<&mut Slot> {
        assert_eq!(entry_id.level(), level);
        assert!(entry_id.slot_index(level - 1).is_some());
        Some(self)
    }

    fn find_summary(&mut self, _entry_id: &EntryID, _level: u64) -> Option<&mut Summary> {
        unreachable!()
    }

    fn inflate_meta(&mut self, config: &mut Config, cx: &mut Context) {
        for tile_id in config.request_tiles(cx.view_interval) {
            self.fetch_meta_tile(tile_id, config);
        }
    }

    fn search(&mut self, config: &mut Config, cx: &mut Context) {
        config.search_state.start_entry(self);
        for (tile_id, tile) in &self.tile_metas {
            if config.search_state.start_tile(self, *tile_id) {
                return;
            }

            if let Some(tile) = tile {
                for (row, row_items) in tile.items.iter().enumerate() {
                    for (col, item) in row_items.iter().enumerate() {
                        if config.search_state.is_match(item) {
                            config.search_state.insert(self, *tile_id, row, col, item);
                        }
                    }
                }
            }
        }
    }

    fn content(
        &mut self,
        ui: &mut egui::Ui,
        rect: Rect,
        viewport: Rect,
        config: &mut Config,
        cx: &mut Context,
    ) {
        cx.slot_rect = Some(rect); // Save slot rect for use later

        let response = ui.allocate_rect(rect, egui::Sense::hover());
        let mut hover_pos = response.hover_pos(); // where is the mouse hovering?

        if self.expanded {
            if self
                .last_view_interval
                .map_or(true, |i| i != cx.view_interval)
            {
                self.clear();
            }
            self.last_view_interval = Some(cx.view_interval);
            if self.tiles.is_empty() {
                self.inflate(config, cx);
            }

            let style = ui.style();
            let visuals = style.interact_selectable(&response, false);
            ui.painter()
                .rect(rect, 0.0, visuals.bg_fill, visuals.bg_stroke);

            let rows = self.rows();
            for tile_index in 0..self.tile_ids.len() {
                hover_pos =
                    self.render_tile(tile_index, rows, hover_pos, ui, rect, viewport, config, cx);
            }
        }
    }

    fn height(&self, _config: &Config, cx: &Context) -> f32 {
        self.rows() as f32 * cx.row_height
    }

    fn is_expandable(&self) -> bool {
        true
    }

    fn toggle_expanded(&mut self) {
        self.expanded = !self.expanded;
    }
}

impl<S: Entry> Panel<S> {
    fn render<T: Entry>(
        ui: &mut egui::Ui,
        rect: Rect,
        viewport: Rect,
        slot: &mut T,
        y: &mut f32,
        config: &mut Config,
        cx: &mut Context,
    ) -> bool {
        const LABEL_WIDTH: f32 = 60.0;
        const COL_PADDING: f32 = 4.0;
        const ROW_PADDING: f32 = 4.0;

        // Compute the size of this slot
        // This is in screen (i.e., rect) space
        let min_y = *y;
        let max_y = min_y + slot.height(config, cx);
        *y = max_y + ROW_PADDING;

        // Cull if out of bounds
        // Note: need to shift by rect.min to get to viewport space
        if max_y - rect.min.y < viewport.min.y {
            return false;
        } else if min_y - rect.min.y > viewport.max.y {
            return true;
        }

        // Draw label and content
        let label_min = rect.min.x;
        let label_max = (rect.min.x + LABEL_WIDTH).at_most(rect.max.x);
        let content_min = (label_max + COL_PADDING).at_most(rect.max.x);
        let content_max = rect.max.x;

        let label_subrect =
            Rect::from_min_max(Pos2::new(label_min, min_y), Pos2::new(label_max, max_y));
        let content_subrect =
            Rect::from_min_max(Pos2::new(content_min, min_y), Pos2::new(content_max, max_y));

        // Shift viewport up by the amount consumed
        // Invariant: (0, 0) in viewport is rect.min
        //   (i.e., subtracting rect.min gets us from screen space to viewport space)
        // Note: viewport.min is NOT necessarily (0, 0)
        let content_viewport = viewport.translate(Vec2::new(0.0, rect.min.y - min_y));

        slot.content(ui, content_subrect, content_viewport, config, cx);
        slot.label(ui, label_subrect);

        false
    }

    fn is_slot_visible(entry_id: &EntryID, config: &Config) -> bool {
        let index = entry_id.last_slot_index().unwrap();
        entry_id.level() != 1 || (index >= config.min_node && index <= config.max_node)
    }
}

impl<S: Entry> Entry for Panel<S> {
    fn new(info: &EntryInfo, entry_id: EntryID) -> Self {
        if let EntryInfo::Panel {
            short_name,
            long_name,
            summary,
            slots,
        } = info
        {
            let expanded = entry_id.level() != 2;
            let summary = summary
                .as_ref()
                .map(|s| Summary::new(s, entry_id.summary()));
            let slots = slots
                .iter()
                .enumerate()
                .map(|(i, s)| S::new(s, entry_id.child(i as u64)))
                .collect();
            Self {
                entry_id,
                short_name: short_name.to_owned(),
                long_name: long_name.to_owned(),
                expanded,
                summary,
                slots,
            }
        } else {
            unreachable!()
        }
    }

    fn entry_id(&self) -> &EntryID {
        &self.entry_id
    }
    fn label_text(&self) -> &str {
        &self.short_name
    }
    fn hover_text(&self) -> &str {
        &self.long_name
    }

    fn find_slot(&mut self, entry_id: &EntryID, level: u64) -> Option<&mut Slot> {
        self.slots
            .get_mut(entry_id.slot_index(level)? as usize)?
            .find_slot(entry_id, level + 1)
    }

    fn find_summary(&mut self, entry_id: &EntryID, level: u64) -> Option<&mut Summary> {
        if level < entry_id.level() - 1 {
            self.slots
                .get_mut(entry_id.slot_index(level)? as usize)?
                .find_summary(entry_id, level + 1)
        } else {
            self.summary.as_mut()?.find_summary(entry_id, level + 1)
        }
    }

    fn inflate_meta(&mut self, config: &mut Config, cx: &mut Context) {
        let force = config.search_state.include_collapsed_entries;
        if self.expanded || force {
            for slot in &mut self.slots {
                // Apply visibility settings
                if !force && !Self::is_slot_visible(slot.entry_id(), config) {
                    continue;
                }

                slot.inflate_meta(config, cx);
            }
        }
    }

    fn search(&mut self, config: &mut Config, cx: &mut Context) {
        let force = config.search_state.include_collapsed_entries;
        if self.expanded || force {
            for slot in &mut self.slots {
                // Apply visibility settings
                if !force && !Self::is_slot_visible(slot.entry_id(), config) {
                    continue;
                }

                slot.search(config, cx);
            }
        }
    }

    fn content(
        &mut self,
        ui: &mut egui::Ui,
        rect: Rect,
        viewport: Rect,
        config: &mut Config,
        cx: &mut Context,
    ) {
        let mut y = rect.min.y;
        if let Some(summary) = &mut self.summary {
            Self::render(ui, rect, viewport, summary, &mut y, config, cx);
        }

        if self.expanded {
            for slot in &mut self.slots {
                // Apply visibility settings
                if !Self::is_slot_visible(slot.entry_id(), config) {
                    continue;
                }

                if Self::render(ui, rect, viewport, slot, &mut y, config, cx) {
                    break;
                }
            }
        }
    }

    fn height(&self, config: &Config, cx: &Context) -> f32 {
        const UNEXPANDED_ROWS: u64 = 2;
        const ROW_PADDING: f32 = 4.0;

        let mut total = 0.0;
        let mut rows: i64 = 0;
        if let Some(summary) = &self.summary {
            total += summary.height(config, cx);
            rows += 1;
        } else if !self.expanded {
            // Need some minimum space if this panel has no summary and is collapsed
            total += UNEXPANDED_ROWS as f32 * cx.row_height;
            rows += 1;
        }

        if self.expanded {
            for slot in &self.slots {
                // Apply visibility settings
                if !Self::is_slot_visible(slot.entry_id(), config) {
                    continue;
                }

                total += slot.height(config, cx);
                rows += 1;
            }
        }

        total += (rows - 1).at_least(0) as f32 * ROW_PADDING;

        total
    }

    fn is_expandable(&self) -> bool {
        !self.slots.is_empty()
    }

    fn toggle_expanded(&mut self) {
        self.expanded = !self.expanded;
    }
}

impl SearchState {
    fn clear(&mut self) {
        self.result_set.clear();
        self.result_cache.clear();
    }

    fn ensure_valid_cache(&mut self) {
        // FIXME (Elliott): other invalidation cases (like on view interval change)

        // Invalidate cache when the search query changes.
        if self.query != self.last_query {
            self.clear();
            self.last_query = self.query.clone();
        }
    }

    fn is_match(&self, item: &ItemMeta) -> bool {
        item.title.find(&self.query).is_some()
    }

    fn start_entry<E: Entry>(&mut self, entry: &E) {
        // Double lookup is better than cloning unconditionally.
        if !self.result_cache.contains_key(entry.entry_id()) {
            self.result_cache
                .entry(entry.entry_id().clone())
                .or_insert_with(BTreeMap::new);
        }

        // No result here because we need to recurse into the tiles
        // regardless. Tiles can arrive asynchronously, and we don't want to
        // manage a cache-invalidation scheme for tile inflation.
    }

    fn start_tile<E: Entry>(&mut self, entry: &E, tile_id: TileID) -> bool {
        let mut result = true;
        // Always called second, so we know the entry exists.
        self.result_cache
            .get_mut(entry.entry_id())
            .unwrap()
            .entry(tile_id)
            .and_modify(|_| {
                result = false;
            })
            .or_insert_with(BTreeMap::new);
        result
    }

    fn insert<E: Entry>(
        &mut self,
        entry: &E,
        tile_id: TileID,
        row: usize,
        col: usize,
        item: &ItemMeta,
    ) {
        const MAX_SEARCH_RESULTS: usize = 1000;
        if self.result_set.len() >= MAX_SEARCH_RESULTS {
            return;
        }

        // We want each item to appear once, so check the result set first
        // before inserting.
        if self.result_set.insert(item.item_uid) {
            self.result_cache
                .get_mut(entry.entry_id())
                .unwrap()
                .get_mut(&tile_id)
                .unwrap()
                .entry(item.item_uid)
                .or_insert_with(|| SearchCacheItem {
                    interval: item.original_interval,
                    entry_id: entry.entry_id().clone(),
                    tile_id: tile_id,
                    row,
                    col,
                    title: item.title.clone(),
                });
        }
    }
}

impl Config {
    fn new(data_source: Box<dyn DeferredDataSource>, info: &DataSourceInfo) -> Self {
        let max_node = info.entry_info.nodes();
        let interval = info.interval;

        Self {
            min_node: 0,
            max_node,
            interval,
            data_source: CountingDeferredDataSource::new(data_source),
            search_state: SearchState::default(),
        }
    }

    fn request_tiles(&mut self, request_interval: Interval) -> Vec<TileID> {
        // For now, always return a single tile
        vec![TileID(request_interval)]
    }
}

impl Window {
    fn new(data_source: Box<dyn DeferredDataSource>, info: &DataSourceInfo, index: u64) -> Self {
        Self {
            panel: Panel::new(&info.entry_info, EntryID::root()),
            index,
            kinds: info.entry_info.kinds(),
            config: Config::new(data_source, info),
        }
    }

    fn find_slot(&mut self, entry_id: &EntryID) -> Option<&mut Slot> {
        self.panel.find_slot(entry_id, 0)
    }

    fn find_summary(&mut self, entry_id: &EntryID) -> Option<&mut Summary> {
        self.panel.find_summary(entry_id, 0)
    }

    fn content(&mut self, ui: &mut egui::Ui, cx: &mut Context) {
        ui.horizontal(|ui| {
            ui.heading(format!("Profile {}", self.index));
            ui.label(cx.view_interval.to_string())
        });

        ScrollArea::vertical()
            .auto_shrink([false; 2])
            .show_viewport(ui, |ui, viewport| {
                let height = self.panel.height(&self.config, cx);
                ui.set_height(height);
                ui.set_width(ui.available_width());

                let rect = Rect::from_min_size(ui.min_rect().min, viewport.size());

                // Root panel has no label
                self.panel.content(ui, rect, viewport, &mut self.config, cx);
            });
    }

    fn node_selection(&mut self, ui: &mut egui::Ui, cx: &Context) {
        ui.subheading("Node Selection", cx);
        let total = self.panel.slots.len().saturating_sub(1) as u64;
        let min_node = &mut self.config.min_node;
        let max_node = &mut self.config.max_node;
        ui.add(Slider::new(min_node, 0..=total).text("First"));
        if *min_node > *max_node {
            *max_node = *min_node;
        }
        ui.add(Slider::new(max_node, 0..=total).text("Last"));
        if *min_node > *max_node {
            *min_node = *max_node;
        }
    }

    fn expand_collapse(&mut self, ui: &mut egui::Ui, cx: &Context) {
        let mut toggle_all = |label, toggle| {
            for node in &mut self.panel.slots {
                for kind in &mut node.slots {
                    if kind.expanded == toggle && kind.label_text() == label {
                        kind.toggle_expanded();
                    }
                }
            }
        };

        ui.subheading("Expand/Collapse", cx);
        ui.label("Expand by kind:");
        ui.horizontal_wrapped(|ui| {
            for kind in &self.kinds {
                if ui.button(kind).clicked() {
                    toggle_all(kind.to_lowercase(), false);
                }
            }
        });
        ui.label("Collapse by kind:");
        ui.horizontal_wrapped(|ui| {
            for kind in &self.kinds {
                if ui.button(kind).clicked() {
                    toggle_all(kind.to_lowercase(), true);
                }
            }
        });
    }

    fn select_interval(&mut self, ui: &mut egui::Ui, cx: &mut Context) {
        ui.subheading("Interval", cx);
        let start_res = ui
            .horizontal(|ui| {
                ui.label("Start:");
                ui.text_edit_singleline(&mut cx.interval_state.start_buffer)
            })
            .inner;

        if let Some(error) = cx.interval_state.start_error {
            ui.label(RichText::new(error.to_string()).color(Color32::RED));
        }

        let stop_res = ui
            .horizontal(|ui| {
                ui.label("Stop:");
                ui.text_edit_singleline(&mut cx.interval_state.stop_buffer)
            })
            .inner;

        if let Some(error) = cx.interval_state.stop_error {
            ui.label(RichText::new(error.to_string()).color(Color32::RED));
        }

        if start_res.lost_focus()
            && cx.interval_state.start_buffer != cx.view_interval.start.to_string()
        {
            match Timestamp::parse(&cx.interval_state.start_buffer) {
                Ok(start) => {
                    // validate timestamp
                    if start > cx.view_interval.stop {
                        cx.interval_state.start_error = Some(IntervalSelectError::StartAfterStop);
                        return;
                    }
                    if start > cx.total_interval.stop {
                        cx.interval_state.start_error = Some(IntervalSelectError::StartAfterEnd);
                        return;
                    }
                    cx.view_interval.start = start;
                    ProfApp::zoom(cx, cx.view_interval);
                    cx.interval_state.start_error = None;
                    cx.interval_state.stop_error = None;
                }
                Err(e) => {
                    cx.interval_state.start_error = Some(e.into());
                }
            }
        }
        if stop_res.lost_focus()
            && cx.interval_state.stop_buffer != cx.view_interval.stop.to_string()
        {
            match Timestamp::parse(&cx.interval_state.stop_buffer) {
                Ok(stop) => {
                    // validate timestamp
                    if stop < cx.view_interval.start {
                        cx.interval_state.stop_error = Some(IntervalSelectError::StopBeforeStart);
                        return;
                    }
                    cx.view_interval.stop = stop;
                    ProfApp::zoom(cx, cx.view_interval);
                    cx.interval_state.start_error = None;
                    cx.interval_state.stop_error = None;
                }
                Err(e) => {
                    cx.interval_state.stop_error = Some(e.into());
                }
            }
        }
    }

    fn controls(&mut self, ui: &mut egui::Ui, cx: &mut Context) {
        const WIDGET_PADDING: f32 = 8.0;
        ui.heading(format!("Profile {}: Controls", self.index));
        ui.add_space(WIDGET_PADDING);
        self.node_selection(ui, cx);
        ui.add_space(WIDGET_PADDING);
        self.expand_collapse(ui, cx);
        ui.add_space(WIDGET_PADDING);
        self.select_interval(ui, cx);
        if ui.button("Reset Zoom Level").clicked() {
            ProfApp::zoom(cx, cx.total_interval);
        }
    }

    fn search(&mut self, cx: &mut Context) {
        // 0. Invalidate cache if the search query changed.
        self.config.search_state.ensure_valid_cache();

        // 1. Expand meta tiles. (Including collapsed entries, if requested).
        self.panel.inflate_meta(&mut self.config, cx);

        // 2. Search whatever data we have. Results are cached by entry/tile.
        self.panel.search(&mut self.config, cx);

        // 3. Done? Modify draw code to highlight cached items.
        // 4. Code to render search results (hint: use cache).
    }

    fn search_box(&mut self, ui: &mut egui::Ui, cx: &mut Context) {
        ui.subheading("Search", cx);
        ui.text_edit_singleline(&mut self.config.search_state.query);
        if !self.config.search_state.query.is_empty() {
            self.search(cx);
        }
    }

    fn search_controls(&mut self, ui: &mut egui::Ui, cx: &mut Context) {
        const WIDGET_PADDING: f32 = 8.0;
        ui.heading(format!("Profile {}: Task Details", self.index));
        ui.add_space(WIDGET_PADDING);
        self.search_box(ui, cx);
        ui.add_space(WIDGET_PADDING);
        ui.subheading("Task Details", cx);
        ui.label("Click on a task to see it displayed here.");
    }
}

impl ProfApp {
    /// Called once before the first frame.
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        mut data_sources: Vec<Box<dyn DeferredDataSource>>,
    ) -> Self {
        // This is also where you can customized the look at feel of egui using
        // `cc.egui_ctx.set_visuals` and `cc.egui_ctx.set_fonts`.

        // Load previous app state (if any).
        // Note that you must enable the `persistence` feature for this to work.
        let mut result: Self = if let Some(storage) = cc.storage {
            eframe::get_value(storage, eframe::APP_KEY).unwrap_or_default()
        } else {
            Default::default()
        };

        for data_source in &mut data_sources {
            data_source.fetch_info();
        }
        result.pending_data_sources.clear();
        result.pending_data_sources.extend(data_sources);

        result.windows.clear();

        #[cfg(not(target_arch = "wasm32"))]
        {
            result.last_update = Some(Instant::now());
        }

        let theme = if result.cx.toggle_dark_mode {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        };
        cc.egui_ctx.set_visuals(theme);

        result
    }

    fn zoom(cx: &mut Context, interval: Interval) {
        if cx.view_interval == interval {
            return;
        }

        cx.view_interval = interval;
        cx.zoom_state.levels.truncate(cx.zoom_state.index + 1);
        cx.zoom_state.levels.push(cx.view_interval);
        cx.zoom_state.index = cx.zoom_state.levels.len() - 1;
        cx.interval_state.start_buffer = cx.view_interval.start.to_string();
        cx.interval_state.stop_buffer = cx.view_interval.stop.to_string();
    }

    fn undo_zoom(cx: &mut Context) {
        if cx.zoom_state.index == 0 {
            return;
        }
        cx.zoom_state.index -= 1;
        cx.view_interval = cx.zoom_state.levels[cx.zoom_state.index];
        cx.interval_state.start_buffer = cx.view_interval.start.to_string();
        cx.interval_state.stop_buffer = cx.view_interval.stop.to_string();
    }

    fn redo_zoom(cx: &mut Context) {
        if cx.zoom_state.index == cx.zoom_state.levels.len() - 1 {
            return;
        }
        cx.zoom_state.index += 1;
        cx.view_interval = cx.zoom_state.levels[cx.zoom_state.index];
        cx.interval_state.start_buffer = cx.view_interval.start.to_string();
        cx.interval_state.stop_buffer = cx.view_interval.stop.to_string();
    }

    fn keyboard(ctx: &egui::Context, cx: &mut Context) {
        // Focus is elsewhere, don't check any keys
        if ctx.memory(|m| m.focus().is_some()) {
            return;
        }

        enum Actions {
            UndoZoom,
            RedoZoom,
            ResetZoom,
            NoAction,
        }
        let action = ctx.input(|i| {
            if i.modifiers.ctrl {
                if i.key_pressed(egui::Key::ArrowLeft) {
                    Actions::UndoZoom
                } else if i.key_pressed(egui::Key::ArrowRight) {
                    Actions::RedoZoom
                } else if i.key_pressed(egui::Key::Num0) {
                    Actions::ResetZoom
                } else {
                    Actions::NoAction
                }
            } else {
                Actions::NoAction
            }
        });
        match action {
            Actions::UndoZoom => ProfApp::undo_zoom(cx),
            Actions::RedoZoom => ProfApp::redo_zoom(cx),
            Actions::ResetZoom => ProfApp::zoom(cx, cx.total_interval),
            Actions::NoAction => {}
        }
    }

    fn cursor(ui: &mut egui::Ui, cx: &mut Context) {
        // Hack: the UI rect we have at this point is not where the
        // timeline is being drawn. So fish out the coordinates we
        // need to draw the correct rect.

        // Sometimes slot_rect is None when initializing the UI
        if cx.slot_rect.is_none() {
            return;
        }

        let ui_rect = ui.min_rect();
        let slot_rect = cx.slot_rect.unwrap();
        let rect = Rect::from_min_max(
            Pos2::new(slot_rect.min.x, ui_rect.min.y),
            Pos2::new(slot_rect.max.x, ui_rect.max.y),
        );

        let response = ui.allocate_rect(rect, egui::Sense::drag());

        // Handle drag detection
        let mut drag_interval = None;

        let is_active_drag = response.dragged_by(egui::PointerButton::Primary);
        if is_active_drag && response.drag_started() {
            // On the beginning of a drag, save our position so we can
            // calculate the delta
            cx.drag_origin = response.interact_pointer_pos();
        }

        if let Some(origin) = cx.drag_origin {
            // We're in a drag, calculate the drag inetrval
            let current = response.interact_pointer_pos().unwrap();
            let min = origin.x.min(current.x);
            let max = origin.x.max(current.x);

            let start = (min - rect.left()) / rect.width();
            let start = cx.view_interval.lerp(start);
            let stop = (max - rect.left()) / rect.width();
            let stop = cx.view_interval.lerp(stop);

            let interval = Interval::new(start, stop);

            if is_active_drag {
                // Still in drag, draw a rectangle to show the dragged region
                let drag_rect =
                    Rect::from_min_max(Pos2::new(min, rect.min.y), Pos2::new(max, rect.max.y));
                let color = Color32::DARK_GRAY.linear_multiply(0.5);
                ui.painter().rect(drag_rect, 0.0, color, Stroke::NONE);

                drag_interval = Some(interval);
            } else if response.drag_released() {
                // Only set view interval if the drag was a certain amount
                const MIN_DRAG_DISTANCE: f32 = 4.0;
                if max - min > MIN_DRAG_DISTANCE {
                    ProfApp::zoom(cx, interval);
                }

                cx.drag_origin = None;
            }
        }

        // Handle hover detection
        if let Some(hover) = response.hover_pos() {
            let visuals = ui.style().interact_selectable(&response, false);

            // Draw vertical line through cursor
            const RADIUS: f32 = 12.0;
            let top = Pos2::new(hover.x, ui.min_rect().min.y);
            let mid_top = Pos2::new(hover.x, (hover.y - RADIUS).at_least(ui.min_rect().min.y));
            let mid_bottom = Pos2::new(hover.x, (hover.y + RADIUS).at_most(ui.min_rect().max.y));
            let bottom = Pos2::new(hover.x, ui.min_rect().max.y);
            ui.painter().line_segment([top, mid_top], visuals.fg_stroke);
            ui.painter()
                .line_segment([mid_bottom, bottom], visuals.fg_stroke);

            // Show timestamp popup

            const HOVER_PADDING: f32 = 8.0;
            let time = (hover.x - rect.left()) / rect.width();
            let time = cx.view_interval.lerp(time);

            // Hack: This avoids an issue where popups displayed normally are
            // forced to stack, even when an explicit position is
            // requested. Instead we display the popup manually via black magic
            let popup_size = if drag_interval.is_some() { 300.0 } else { 90.0 };
            let mut popup_rect = Rect::from_min_size(
                Pos2::new(top.x + HOVER_PADDING, top.y),
                Vec2::new(popup_size, 100.0),
            );
            // This is a hack to keep the time viewer on the screen when we
            // approach the right edge.
            if popup_rect.right() > ui.min_rect().right() {
                popup_rect = popup_rect
                    .translate(Vec2::new(ui.min_rect().right() - popup_rect.right(), 0.0));
            }
            let mut popup_ui = egui::Ui::new(
                ui.ctx().clone(),
                ui.layer_id(),
                ui.id(),
                popup_rect,
                popup_rect.expand(16.0),
            );
            egui::Frame::popup(ui.style()).show(&mut popup_ui, |ui| {
                if let Some(drag) = drag_interval {
                    ui.label(format!("{drag}"));
                } else {
                    ui.label(format!("t={time}"));
                }
            });

            // ui.show_tooltip_at("timestamp_tooltip", Some(top), format!("t={time}"));
        }
    }
}

impl eframe::App for ProfApp {
    /// Called to save state before shutdown.
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, eframe::APP_KEY, self);
    }

    /// Called each time the UI needs repainting.
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let Self {
            pending_data_sources,
            windows,
            cx,
            #[cfg(not(target_arch = "wasm32"))]
            last_update,
            ..
        } = self;

        if let Some(mut source) = pending_data_sources.pop_front() {
            // We made one request, so we know there is always zero or one
            // elements in this list.
            if let Some(info) = source.get_infos().pop() {
                let window = Window::new(source, &info, windows.len() as u64);
                if windows.is_empty() {
                    cx.total_interval = window.config.interval;
                } else {
                    cx.total_interval = cx.total_interval.union(window.config.interval);
                }
                ProfApp::zoom(cx, cx.total_interval);
                windows.push(window);
            } else {
                pending_data_sources.push_front(source);
            }
        }

        for window in windows.iter_mut() {
            for tile in window.config.data_source.get_summary_tiles() {
                if let Some(entry) = window.find_summary(&tile.entry_id) {
                    // If the entry doesn't exist, we already zoomed away and
                    // are no longer interested in this tile.
                    entry
                        .tiles
                        .entry(tile.tile_id)
                        .and_modify(|t| *t = Some(tile.data));
                }
            }

            for tile in window.config.data_source.get_slot_tiles() {
                if let Some(entry) = window.find_slot(&tile.entry_id) {
                    // If the entry doesn't exist, we already zoomed away and
                    // are no longer interested in this tile.
                    entry
                        .tiles
                        .entry(tile.tile_id)
                        .and_modify(|t| *t = Some(tile.data));
                }
            }

            for tile in window.config.data_source.get_slot_meta_tiles() {
                if let Some(entry) = window.find_slot(&tile.entry_id) {
                    // If the entry doesn't exist, we already zoomed away and
                    // are no longer interested in this tile.
                    entry
                        .tile_metas
                        .entry(tile.tile_id)
                        .and_modify(|t| *t = Some(tile.data));
                }
            }
        }

        let mut _fps = 0.0;
        #[cfg(not(target_arch = "wasm32"))]
        {
            let now = Instant::now();
            if let Some(last) = last_update {
                _fps = 1.0 / now.duration_since(*last).as_secs_f64();
            }
            *last_update = Some(now);
        }

        #[cfg(not(target_arch = "wasm32"))]
        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("Quit").clicked() {
                        _frame.close();
                    }
                });
            });
        });

        egui::SidePanel::left("side_panel").show(ctx, |ui| {
            let body = TextStyle::Body.resolve(ui.style()).size;
            let heading = TextStyle::Heading.resolve(ui.style()).size;
            // Just set this on every frame for now
            cx.subheading_size = (heading + body) * 0.5;

            ui.heading("Legion Prof Tech Demo");

            const WIDGET_PADDING: f32 = 8.0;
            ui.add_space(WIDGET_PADDING);

            for window in windows.iter_mut() {
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    window.controls(ui, cx);
                });
            }

            for window in windows.iter_mut() {
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    window.search_controls(ui, cx);
                });
            }

            ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    ui.label("powered by ");
                    ui.hyperlink_to("egui", "https://github.com/emilk/egui");
                    ui.label(" and ");
                    ui.hyperlink_to(
                        "eframe",
                        "https://github.com/emilk/egui/tree/master/crates/eframe",
                    );
                    ui.label(".");
                });

                ui.horizontal(|ui| {
                    // swap to dark mode
                    let mut current_theme = if cx.toggle_dark_mode {
                        egui::Visuals::dark()
                    } else {
                        egui::Visuals::light()
                    };

                    current_theme.light_dark_radio_buttons(ui);
                    if current_theme.dark_mode != cx.toggle_dark_mode {
                        cx.toggle_dark_mode = current_theme.dark_mode;
                        ctx.set_visuals(current_theme);
                    }

                    let debug_color = if cx.debug {
                        ui.visuals().hyperlink_color
                    } else {
                        ui.visuals().text_color()
                    };

                    let button = egui::Button::new(
                        egui::RichText::new("🛠 Debug").color(debug_color).size(12.0),
                    )
                    .frame(true);
                    if ui
                        .add(button)
                        .on_hover_text(format!(
                            "Toggle debug mode {}",
                            if cx.debug { "off" } else { "on" }
                        ))
                        .clicked()
                    {
                        cx.debug = !cx.debug;
                    }
                });

                egui::warn_if_debug_build(ui);

                #[cfg(not(target_arch = "wasm32"))]
                {
                    ui.separator();
                    ui.label(format!("FPS: {_fps:.0}"));
                }
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            // Use body font to figure out how tall to draw rectangles.
            let font_id = TextStyle::Body.resolve(ui.style());
            let row_height = ui.fonts(|f| f.row_height(&font_id));
            // Just set this on every frame for now
            cx.row_height = row_height;

            let mut remaining = windows.len();
            // Only wrap in a frame if more than one profile
            if remaining > 1 {
                for window in windows.iter_mut() {
                    egui::Frame::group(ui.style()).show(ui, |ui| {
                        ui.push_id(window.index, |ui| {
                            ui.set_height(ui.available_height() / (remaining as f32));
                            ui.set_width(ui.available_width());
                            window.content(ui, cx);
                            remaining -= 1;
                        });
                    });
                }
            } else {
                for window in windows.iter_mut() {
                    window.content(ui, cx);
                }
            }

            Self::cursor(ui, cx);
        });

        Self::keyboard(ctx, cx);

        // Keep repainting as long as we have outstanding requests.
        if !pending_data_sources.is_empty()
            || windows
                .iter()
                .any(|w| w.config.data_source.outstanding_requests() > 0)
        {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
    }
}

trait UiExtra {
    fn subheading(&mut self, text: impl Into<egui::RichText>, cx: &Context) -> egui::Response;
    fn show_tooltip(
        &mut self,
        id_source: impl core::hash::Hash,
        rect: &Rect,
        text: impl Into<egui::WidgetText>,
    );
    fn show_tooltip_ui(
        &mut self,
        id_source: impl core::hash::Hash,
        rect: &Rect,
        add_contents: impl FnOnce(&mut egui::Ui),
    );
    fn show_tooltip_at(
        &mut self,
        id_source: impl core::hash::Hash,
        suggested_position: Option<Pos2>,
        text: impl Into<egui::WidgetText>,
    );
}

impl UiExtra for egui::Ui {
    fn subheading(&mut self, text: impl Into<egui::RichText>, cx: &Context) -> egui::Response {
        self.add(egui::Label::new(
            text.into().heading().size(cx.subheading_size),
        ))
    }

    /// This is a method for showing a fast, very responsive
    /// tooltip. The standard hover methods force a delay (presumably
    /// to confirm the mouse has stopped), this bypasses that. Best
    /// used in situations where the user might quickly skim over the
    /// content (e.g., utilization plots).
    fn show_tooltip(
        &mut self,
        id_source: impl core::hash::Hash,
        rect: &Rect,
        text: impl Into<egui::WidgetText>,
    ) {
        self.show_tooltip_ui(id_source, rect, |ui| {
            ui.add(egui::Label::new(text));
        });
    }
    fn show_tooltip_ui(
        &mut self,
        id_source: impl core::hash::Hash,
        rect: &Rect,
        add_contents: impl FnOnce(&mut egui::Ui),
    ) {
        egui::containers::show_tooltip_for(
            self.ctx(),
            self.auto_id_with(id_source),
            rect,
            add_contents,
        );
    }
    fn show_tooltip_at(
        &mut self,
        id_source: impl core::hash::Hash,
        suggested_position: Option<Pos2>,
        text: impl Into<egui::WidgetText>,
    ) {
        egui::containers::show_tooltip_at(
            self.ctx(),
            self.auto_id_with(id_source),
            suggested_position,
            |ui| {
                ui.add(egui::Label::new(text));
            },
        );
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn start(data_sources: Vec<Box<dyn DeferredDataSource>>) {
    env_logger::init(); // Log to stderr (if you run with `RUST_LOG=debug`).

    let native_options = eframe::NativeOptions::default();
    eframe::run_native(
        "Legion Prof",
        native_options,
        Box::new(|cc| Box::new(ProfApp::new(cc, data_sources))),
    )
    .expect("failed to start eframe");
}

#[cfg(target_arch = "wasm32")]
pub fn start(data_sources: Vec<Box<dyn DeferredDataSource>>) {
    // Redirect `log` message to `console.log` and friends:
    eframe::WebLogger::init(log::LevelFilter::Debug).ok();

    let web_options = eframe::WebOptions::default();

    wasm_bindgen_futures::spawn_local(async {
        eframe::WebRunner::new()
            .start(
                "the_canvas_id", // hardcode it
                web_options,
                Box::new(|cc| Box::new(ProfApp::new(cc, data_sources))),
            )
            .await
            .expect("failed to start eframe");
    });
}
