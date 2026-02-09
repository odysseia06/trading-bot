//! Depth stream synchronization manager.
//!
//! Implements the Binance depth stream initialization protocol:
//! 1. Subscribe to `{symbol}@depth@100ms` WebSocket stream
//! 2. Buffer incoming depth events
//! 3. Fetch REST snapshot: `GET /api/v3/depth?symbol=SYMBOL&limit=1000`
//! 4. Drop buffered events where `final_update_id <= snapshot.lastUpdateId`
//! 5. First valid event must have `first_update_id <= lastUpdateId+1 AND final_update_id >= lastUpdateId+1`
//! 6. Apply subsequent deltas; on sequence gap, re-fetch snapshot

use model::DepthUpdate;
use rust_decimal::Decimal;
use std::collections::{HashMap, VecDeque};
use std::str::FromStr;

/// Maximum number of events to buffer while waiting for snapshot.
const MAX_BUFFER_SIZE: usize = 1000;

/// State of depth synchronization for a symbol.
#[derive(Debug)]
enum SymbolState {
    /// Buffering events, waiting for snapshot.
    Buffering { events: VecDeque<DepthUpdate> },
    /// Synchronized and applying deltas.
    Synchronized { last_update_id: u64 },
}

/// Result of processing a depth update.
#[derive(Debug)]
pub enum ProcessResult {
    /// Event should be applied to order book.
    Apply(DepthUpdate),
    /// Event was buffered, waiting for snapshot.
    Buffered,
    /// Event was stale and dropped.
    Dropped,
    /// Need to fetch snapshot for this symbol.
    NeedSnapshot(String),
}

/// Result of applying a snapshot.
#[derive(Debug)]
pub struct SnapshotResult {
    /// Symbol the snapshot was for.
    pub symbol: String,
    /// Buffered events that should now be applied.
    pub events_to_apply: Vec<DepthUpdate>,
    /// Number of stale events that were dropped.
    pub dropped_count: usize,
}

/// Manages depth stream synchronization for multiple symbols.
pub struct DepthManager {
    states: HashMap<String, SymbolState>,
}

impl DepthManager {
    /// Create a new depth manager.
    pub fn new() -> Self {
        Self {
            states: HashMap::new(),
        }
    }

    /// Process an incoming depth update.
    ///
    /// Returns the action to take based on the current state.
    pub fn process_update(&mut self, update: DepthUpdate) -> ProcessResult {
        let symbol = update.symbol.clone();

        match self.states.get_mut(&symbol) {
            None => {
                // First event for this symbol - start buffering
                let mut events = VecDeque::with_capacity(MAX_BUFFER_SIZE);
                events.push_back(update);
                self.states
                    .insert(symbol.clone(), SymbolState::Buffering { events });
                ProcessResult::NeedSnapshot(symbol)
            }
            Some(SymbolState::Buffering { events }) => {
                // Still waiting for snapshot - buffer the event
                if events.len() < MAX_BUFFER_SIZE {
                    events.push_back(update);
                    ProcessResult::Buffered
                } else {
                    // Buffer overflow - drop oldest event and add new one
                    events.pop_front();
                    events.push_back(update);
                    ProcessResult::Buffered
                }
            }
            Some(SymbolState::Synchronized { last_update_id }) => {
                // Check if this is a valid next update
                if update.final_update_id <= *last_update_id {
                    // Stale update, drop it
                    ProcessResult::Dropped
                } else if update.first_update_id <= *last_update_id + 1
                    && update.final_update_id >= *last_update_id + 1
                {
                    // Valid sequential update
                    *last_update_id = update.final_update_id;
                    ProcessResult::Apply(update)
                } else {
                    // Sequence gap detected - need to re-sync
                    tracing::warn!(
                        symbol = %symbol,
                        expected = *last_update_id + 1,
                        got_first = update.first_update_id,
                        got_final = update.final_update_id,
                        "Depth sequence gap detected, re-syncing"
                    );
                    // Reset to buffering state
                    let mut events = VecDeque::with_capacity(MAX_BUFFER_SIZE);
                    events.push_back(update);
                    self.states
                        .insert(symbol.clone(), SymbolState::Buffering { events });
                    ProcessResult::NeedSnapshot(symbol)
                }
            }
        }
    }

    /// Apply a snapshot and process buffered events.
    ///
    /// Returns the events that should be applied to the order book after the snapshot.
    pub fn apply_snapshot(
        &mut self,
        symbol: &str,
        last_update_id: u64,
        bids: &[(String, String)],
        asks: &[(String, String)],
    ) -> SnapshotResult {
        let mut events_to_apply = Vec::new();
        let mut dropped_count = 0;

        // Convert string prices to decimal for the initial snapshot application
        let bid_levels: Vec<(Decimal, Decimal)> = bids
            .iter()
            .filter_map(|(price, qty)| {
                Some((Decimal::from_str(price).ok()?, Decimal::from_str(qty).ok()?))
            })
            .collect();

        let ask_levels: Vec<(Decimal, Decimal)> = asks
            .iter()
            .filter_map(|(price, qty)| {
                Some((Decimal::from_str(price).ok()?, Decimal::from_str(qty).ok()?))
            })
            .collect();

        // Create a synthetic depth update for the snapshot
        let snapshot_update = DepthUpdate {
            exchange: model::Exchange::Binance,
            symbol: symbol.to_string(),
            first_update_id: 0,
            final_update_id: last_update_id,
            bids: bid_levels,
            asks: ask_levels,
            timestamp_ms: chrono::Utc::now().timestamp_millis(),
            is_snapshot: true,
        };
        events_to_apply.push(snapshot_update);

        // Process buffered events
        if let Some(SymbolState::Buffering { events }) = self.states.remove(symbol) {
            let mut found_first_valid = false;

            for event in events {
                if event.final_update_id <= last_update_id {
                    // Stale event, drop it
                    dropped_count += 1;
                    continue;
                }

                if !found_first_valid {
                    // First valid event must satisfy the initialization condition
                    if event.first_update_id <= last_update_id + 1
                        && event.final_update_id >= last_update_id + 1
                    {
                        found_first_valid = true;
                        events_to_apply.push(event);
                    } else {
                        // Gap between snapshot and first event - this shouldn't happen
                        // if we fetched the snapshot quickly enough
                        tracing::warn!(
                            symbol = %symbol,
                            snapshot_id = last_update_id,
                            event_first = event.first_update_id,
                            event_final = event.final_update_id,
                            "Gap between snapshot and first buffered event"
                        );
                        dropped_count += 1;
                    }
                } else {
                    // After finding first valid, add all subsequent events
                    events_to_apply.push(event);
                }
            }
        }

        // Update state to synchronized
        let final_update_id = events_to_apply
            .last()
            .map(|e| e.final_update_id)
            .unwrap_or(last_update_id);

        self.states.insert(
            symbol.to_string(),
            SymbolState::Synchronized {
                last_update_id: final_update_id,
            },
        );

        tracing::info!(
            symbol = %symbol,
            snapshot_id = last_update_id,
            events_to_apply = events_to_apply.len(),
            dropped = dropped_count,
            "Depth stream synchronized"
        );

        SnapshotResult {
            symbol: symbol.to_string(),
            events_to_apply,
            dropped_count,
        }
    }

    /// Check if a symbol is synchronized.
    pub fn is_synchronized(&self, symbol: &str) -> bool {
        matches!(
            self.states.get(symbol),
            Some(SymbolState::Synchronized { .. })
        )
    }

    /// Get the last update ID for a synchronized symbol.
    pub fn last_update_id(&self, symbol: &str) -> Option<u64> {
        match self.states.get(symbol) {
            Some(SymbolState::Synchronized { last_update_id }) => Some(*last_update_id),
            _ => None,
        }
    }

    /// Clear state for a symbol, forcing re-synchronization.
    pub fn clear(&mut self, symbol: &str) {
        self.states.remove(symbol);
    }

    /// Clear all state.
    pub fn clear_all(&mut self) {
        self.states.clear();
    }
}

impl Default for DepthManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use model::Exchange;
    use rust_decimal_macros::dec;

    fn make_depth_update(symbol: &str, first: u64, final_id: u64) -> DepthUpdate {
        DepthUpdate {
            exchange: Exchange::Binance,
            symbol: symbol.to_string(),
            first_update_id: first,
            final_update_id: final_id,
            bids: vec![(dec!(100), dec!(1))],
            asks: vec![(dec!(101), dec!(1))],
            timestamp_ms: 0,
            is_snapshot: false,
        }
    }

    #[test]
    fn test_first_event_triggers_snapshot_request() {
        let mut manager = DepthManager::new();
        let update = make_depth_update("BTCUSDT", 100, 105);

        match manager.process_update(update) {
            ProcessResult::NeedSnapshot(symbol) => {
                assert_eq!(symbol, "BTCUSDT");
            }
            _ => panic!("Expected NeedSnapshot"),
        }
    }

    #[test]
    fn test_events_buffered_before_snapshot() {
        let mut manager = DepthManager::new();

        // First event triggers snapshot request
        let update1 = make_depth_update("BTCUSDT", 100, 105);
        assert!(matches!(
            manager.process_update(update1),
            ProcessResult::NeedSnapshot(_)
        ));

        // Subsequent events are buffered
        let update2 = make_depth_update("BTCUSDT", 106, 110);
        assert!(matches!(
            manager.process_update(update2),
            ProcessResult::Buffered
        ));

        let update3 = make_depth_update("BTCUSDT", 111, 115);
        assert!(matches!(
            manager.process_update(update3),
            ProcessResult::Buffered
        ));
    }

    #[test]
    fn test_snapshot_filters_stale_events() {
        let mut manager = DepthManager::new();

        // Buffer some events
        let _ = manager.process_update(make_depth_update("BTCUSDT", 100, 105));
        let _ = manager.process_update(make_depth_update("BTCUSDT", 106, 110));
        let _ = manager.process_update(make_depth_update("BTCUSDT", 111, 115));

        // Apply snapshot with ID 108
        let result = manager.apply_snapshot(
            "BTCUSDT",
            108,
            &[("100".to_string(), "1".to_string())],
            &[("101".to_string(), "1".to_string())],
        );

        // First event (100-105) should be dropped (stale)
        // Second event (106-110) should be included (spans 108+1=109)
        // Third event (111-115) should be included
        assert_eq!(result.dropped_count, 1);
        // snapshot + 2 valid events
        assert_eq!(result.events_to_apply.len(), 3);
    }

    #[test]
    fn test_synchronized_applies_sequential_updates() {
        let mut manager = DepthManager::new();

        // Trigger snapshot request and apply snapshot
        let _ = manager.process_update(make_depth_update("BTCUSDT", 100, 105));
        let _ = manager.apply_snapshot(
            "BTCUSDT",
            100,
            &[("100".to_string(), "1".to_string())],
            &[("101".to_string(), "1".to_string())],
        );

        // Now synchronized, should apply sequential updates
        // The snapshot result should have advanced last_update_id to 105
        // Actually, looking at apply_snapshot, it uses final_update_id from last event
        // which is the buffered event 100-105, so last_update_id should be 105

        let update = make_depth_update("BTCUSDT", 106, 110);
        assert!(matches!(
            manager.process_update(update),
            ProcessResult::Apply(_)
        ));
    }

    #[test]
    fn test_stale_update_dropped() {
        let mut manager = DepthManager::new();

        // Set up synchronized state
        let _ = manager.process_update(make_depth_update("BTCUSDT", 100, 105));
        let _ = manager.apply_snapshot(
            "BTCUSDT",
            100,
            &[("100".to_string(), "1".to_string())],
            &[("101".to_string(), "1".to_string())],
        );

        // Apply a valid update to advance state
        let _ = manager.process_update(make_depth_update("BTCUSDT", 106, 110));

        // Now try a stale update
        let stale = make_depth_update("BTCUSDT", 106, 108);
        assert!(matches!(
            manager.process_update(stale),
            ProcessResult::Dropped
        ));
    }

    #[test]
    fn test_sequence_gap_triggers_resync() {
        let mut manager = DepthManager::new();

        // Set up synchronized state
        let _ = manager.process_update(make_depth_update("BTCUSDT", 100, 105));
        let _ = manager.apply_snapshot(
            "BTCUSDT",
            100,
            &[("100".to_string(), "1".to_string())],
            &[("101".to_string(), "1".to_string())],
        );

        // Gap: expected 106, got 200
        let gap_update = make_depth_update("BTCUSDT", 200, 205);
        match manager.process_update(gap_update) {
            ProcessResult::NeedSnapshot(symbol) => {
                assert_eq!(symbol, "BTCUSDT");
            }
            _ => panic!("Expected NeedSnapshot due to gap"),
        }
    }
}
