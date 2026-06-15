//! Wall tracker: detect large resting orders appearing and disappearing in the book.
//!
//! Tracks "walls" (large orders at a price level) through their lifecycle:
//! appearance -> growth -> shrinkage -> removal (cancelled) or execution (absorbed).
//! This is the key differentiator from the old studies that only used top-5 imbalance.

use forge_core::Side;

/// A tracked wall (large resting order) in the book.
#[derive(Debug, Clone)]
pub struct Wall {
    /// Price level of the wall.
    pub price: f64,
    /// Side of the wall.
    pub side: Side,
    /// Current size of the wall.
    pub size: f64,
    /// Size when first detected (appearance size).
    pub appearance_size: f64,
    /// Maximum size observed during the wall's lifetime.
    pub peak_size: f64,
    /// Timestamp when the wall first appeared (ns).
    pub appeared_at: u64,
    /// Timestamp of the last update (ns).
    pub last_updated_at: u64,
    /// Number of times the wall grew after appearance.
    pub growth_count: u32,
    /// Number of times the wall shrank (partial pull).
    pub shrink_count: u32,
    /// Whether the wall has been removed (cancelled or fully executed).
    pub removed: bool,
    /// How the wall was removed: "cancelled" (size went to 0 without trade through)
    /// or "executed" (price traded through the wall level).
    pub removal_type: Option<String>,
}

/// Wall tracker: monitors large orders in the book over time.
#[derive(Debug, Clone)]
pub struct WallTracker {
    /// Minimum size (in base units) to qualify as a "wall".
    pub wall_threshold: f64,
    /// Active walls currently in the book, keyed by (side, price).
    pub active_walls: Vec<Wall>,
    /// Completed walls that have been removed.
    pub completed_walls: Vec<Wall>,
}

impl WallTracker {
    /// Create a new wall tracker with the given minimum size threshold.
    pub fn new(wall_threshold: f64) -> Self {
        Self {
            wall_threshold,
            active_walls: Vec::new(),
            completed_walls: Vec::new(),
        }
    }

    /// Update the tracker with a new book state. Call this on each book delta.
    /// `bid_levels` and `ask_levels` are (price, size) pairs sorted best-first.
    /// `ts` is the current timestamp in nanoseconds.
    pub fn update(
        &mut self,
        bid_levels: &[(f64, f64)],
        ask_levels: &[(f64, f64)],
        ts: u64,
    ) {
        // Check for walls on bid side
        for &(price, size) in bid_levels {
            self.update_side(price, size, Side::Bid, ts);
        }
        // Check for walls on ask side
        for &(price, size) in ask_levels {
            self.update_side(price, size, Side::Ask, ts);
        }
    }

    fn update_side(&mut self, price: f64, size: f64, side: Side, ts: u64) {
        // Find existing wall at this price+side
        let existing_idx = self.active_walls.iter().position(|w| {
            (w.price - price).abs() < f64::EPSILON && w.side == side
        });

        if size >= self.wall_threshold {
            // Wall exists or should be created
            if let Some(idx) = existing_idx {
                let wall = &mut self.active_walls[idx];
                let old_size = wall.size;
                wall.size = size;
                wall.last_updated_at = ts;
                if size > wall.peak_size {
                    wall.peak_size = size;
                }
                if size > old_size {
                    wall.growth_count += 1;
                } else if size < old_size {
                    wall.shrink_count += 1;
                }
            } else {
                // New wall appeared
                self.active_walls.push(Wall {
                    price,
                    side,
                    size,
                    appearance_size: size,
                    peak_size: size,
                    appeared_at: ts,
                    last_updated_at: ts,
                    growth_count: 0,
                    shrink_count: 0,
                    removed: false,
                    removal_type: None,
                });
            }
        } else if let Some(idx) = existing_idx {
            // Wall was at this level but size dropped below threshold
            let mut wall = self.active_walls.swap_remove(idx);
            wall.size = size;
            wall.last_updated_at = ts;
            wall.shrink_count += 1;
            wall.removed = true;
            // Heuristic: if size went to 0, it was cancelled; if size just shrank,
            // it might have been partially executed. We can't tell for sure without
            // trade data, so we mark it as "cancelled" if size == 0, "partial" otherwise.
            wall.removal_type = Some(if size < 1e-10 { "cancelled".to_string() } else { "partial".to_string() });
            self.completed_walls.push(wall);
        }
    }

    /// Get walls that were cancelled (size went to 0) vs executed (price traded through).
    /// This is a heuristic: cancelled walls shrank to 0 without price trading through.
    pub fn cancelled_walls(&self) -> Vec<&Wall> {
        self.completed_walls.iter()
            .filter(|w| w.removal_type.as_deref() == Some("cancelled"))
            .collect()
    }

    /// Get the ratio of cancelled walls to total completed walls.
    /// High ratio = lots of spoofing (walls pulled, not executed).
    pub fn cancel_ratio(&self) -> f64 {
        if self.completed_walls.is_empty() { return 0.0; }
        let cancelled = self.cancelled_walls().len() as f64;
        cancelled / self.completed_walls.len() as f64
    }

    /// Average wall lifetime in nanoseconds.
    pub fn avg_wall_lifetime_ns(&self) -> f64 {
        if self.completed_walls.is_empty() { return 0.0; }
        let total: f64 = self.completed_walls.iter()
            .map(|w| (w.last_updated_at - w.appeared_at) as f64)
            .sum();
        total / self.completed_walls.len() as f64
    }
}