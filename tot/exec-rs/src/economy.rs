//! Economy interrupt (M2 slice 2.4): bags-full / grey-threshold triggers and the
//! vendor trip (nav → sell greys → repair → nav back). Spec:
//! docs/superpowers/specs/2026-06-10-m2-slice-2.4-economy-exec-flows-design.md
//! (azerothcore-heimdal master). Failures are never terminal (spec §5).

use serde::Deserialize;

use crate::grind::GrindError;
use tot_harness_client::HarnessClient;

/// Trigger thresholds + pacing (exec constants, not goal fields — spec §3).
#[allow(dead_code)] // consumed by Task 6-8
pub(crate) const FREE_SLOT_TRIGGER: u32 = 3;
#[allow(dead_code)] // consumed by Task 6-8
pub(crate) const GREY_COUNT_TRIGGER: u32 = 8;
#[allow(dead_code)] // consumed by Task 6-8
pub(crate) const ECONOMY_CHECK_KILL_STRIDE: u32 = 5;
#[allow(dead_code)] // consumed by Task 6-8
pub(crate) const VENDOR_TRIP_COOLDOWN_S: u64 = 600;

/// Backpack proper (INVENTORY_SLOT_ITEM_START..END) is 16 slots.
const BACKPACK_SLOTS: u32 = 16;

#[derive(Debug, Deserialize)]
struct InvItem {
    #[serde(default)]
    quality: u32,
}
#[derive(Debug, Deserialize)]
struct NestedBag {
    capacity: u32,
    #[serde(default)]
    contents: Vec<InvItem>,
}
#[derive(Debug, Deserialize)]
struct Inventory {
    #[serde(default)]
    bags: Vec<InvItem>,
    #[serde(default)]
    nested_bags: Vec<NestedBag>,
}

/// What the trigger check needs from `obs.get_inventory` (spec §3).
#[derive(Debug, Clone, Copy, PartialEq)]
#[allow(dead_code)] // consumed by Task 6-8
pub(crate) struct BagSummary {
    pub free_slots: u32,
    pub grey_count: u32,
}

impl BagSummary {
    #[allow(dead_code)] // consumed by Task 6-8
    pub(crate) fn triggered(&self) -> bool {
        self.free_slots <= FREE_SLOT_TRIGGER || self.grey_count >= GREY_COUNT_TRIGGER
    }
}

/// Pure derivation per spec §3: free = (16 − backpack items) + Σ(bag capacity − bag
/// items); greys = quality==0 across backpack + nested bags. Equipped NEVER counts.
pub(crate) fn summarize_inventory(raw: serde_json::Value) -> Result<BagSummary, GrindError> {
    let inv: Inventory = serde_json::from_value(raw)
        .map_err(|e| GrindError::Shape(format!("obs.get_inventory: {e}")))?;
    let mut free = BACKPACK_SLOTS.saturating_sub(inv.bags.len() as u32);
    let mut greys = inv.bags.iter().filter(|i| i.quality == 0).count() as u32;
    for b in &inv.nested_bags {
        free = free.saturating_add(b.capacity.saturating_sub(b.contents.len() as u32));
        greys = greys.saturating_add(b.contents.iter().filter(|i| i.quality == 0).count() as u32);
    }
    Ok(BagSummary { free_slots: free, grey_count: greys })
}

/// One `obs.get_inventory` round-trip → `BagSummary`.
#[allow(dead_code)] // consumed by Task 6-8
pub(crate) async fn read_bags(client: &HarnessClient, bot_guid: u64) -> Result<BagSummary, GrindError> {
    let raw = client
        .call("obs.get_inventory", serde_json::json!({ "target_guid": bot_guid as i64 }))
        .await?;
    summarize_inventory(raw)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Backpack with 14 items (12 grey) + one 6-slot bag with 4 items (2 grey):
    /// free = (16-14) + (6-4) = 4; greys = 14. Equipped greys must NOT count.
    fn busy_inventory() -> serde_json::Value {
        let grey = |slot: u64| json!({"slot": slot, "item_entry": 750, "name": "Tough Wolf Meat",
                                      "itemset": 0, "quality": 0, "count": 1, "guid": 9000 + slot});
        let white = |slot: u64| json!({"slot": slot, "item_entry": 2589, "name": "Linen Cloth",
                                       "itemset": 0, "quality": 1, "count": 5, "guid": 9100 + slot});
        let mut bags: Vec<serde_json::Value> = (23..35).map(grey).collect(); // 12 greys
        bags.push(white(35)); bags.push(white(36)); // 14 occupied
        json!({
            "equipped": [ {"slot": 0, "item_entry": 1, "name": "Grey Hat (equipped)",
                           "itemset": 0, "quality": 0, "count": 1, "guid": 8000} ],
            "bags": bags,
            "nested_bags": [ {"bag_slot": 19, "capacity": 6,
                              "contents": [grey(0), grey(1), white(2), white(3)]} ]
        })
    }

    #[test]
    fn summarize_counts_free_slots_and_greys() {
        let s = summarize_inventory(busy_inventory()).unwrap();
        assert_eq!(s.free_slots, 4, "(16-14) backpack + (6-4) nested");
        assert_eq!(s.grey_count, 14, "12 backpack + 2 nested; equipped excluded");
    }

    #[test]
    fn empty_inventory_is_untriggered() {
        let s = summarize_inventory(json!({"equipped": [], "bags": [], "nested_bags": []})).unwrap();
        assert_eq!(s.free_slots, 16);
        assert_eq!(s.grey_count, 0);
        assert!(!s.triggered());
    }

    #[test]
    fn missing_nested_bags_defaults_empty() {
        // Older mocks / minimal responses omit nested_bags — must not be a Shape error.
        let s = summarize_inventory(json!({"bags": []})).unwrap();
        assert_eq!(s.free_slots, 16);
    }

    #[test]
    fn triggered_on_free_slots() {
        assert!(BagSummary { free_slots: 3, grey_count: 0 }.triggered());
        assert!(!BagSummary { free_slots: 4, grey_count: 0 }.triggered());
    }

    #[test]
    fn triggered_on_grey_count() {
        assert!(BagSummary { free_slots: 16, grey_count: 8 }.triggered());
        assert!(!BagSummary { free_slots: 16, grey_count: 7 }.triggered());
    }
}
