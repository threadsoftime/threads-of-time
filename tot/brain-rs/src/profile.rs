//! Declarative grind profiles (M2 slice 2.1). serde/TOML-loaded, fail-fast validated.
//! Externalizes the goal_emitter grind constants with explicit per-camp anchors.

use std::collections::HashMap;
use std::path::Path;

use serde::Deserialize;

/// Known rotation ids (2.1 shipped auto_attack; 2.2 adds the per-spec Bracket-1 set).
const KNOWN_ROTATIONS: &[&str] =
    &["auto_attack", "warrior_b1", "rogue_b1", "mage_frost_b1", "priest_smite_b1"];

#[derive(Debug, thiserror::Error)]
pub enum ProfileError {
    #[error("profile toml parse error: {0}")]
    Parse(String),
    #[error("profile '{profile_id}' invalid: {reason}")]
    Validation { profile_id: String, reason: String },
    #[error("profile file io error: {0}")]
    Io(String),
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LevelBand {
    pub below: u32,
    pub above: u32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileAnchor {
    pub map_id: u32,
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

/// Vendor spawn position in world-space (same map as the camp anchor by design).
/// Used by the economy trip planner to navigate to the vendor.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VendorPos {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GrindProfile {
    pub anchor: ProfileAnchor,
    pub level_band: LevelBand,
    pub creature_type: String,
    pub wander_radius: f32,
    pub max_search_radius: f32,
    pub rest_threshold: f32,
    pub rotation_id: String,
    #[serde(default)]
    pub custom_behavior: Option<String>,
    /// Economy vendor group (M2 slice 2.4) — all three present or all absent
    /// (validated). Mined per camp from the world-DB dumps (spec §6); the vendor
    /// is on the SAME map as the anchor.
    #[serde(default)]
    pub vendor_spawn_id: Option<u64>,
    #[serde(default)]
    pub vendor_pos: Option<VendorPos>,
    #[serde(default)]
    pub vendor_can_repair: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct ProfileRegistry {
    profiles: HashMap<String, GrindProfile>,
}

impl ProfileRegistry {
    /// Parse + validate a profiles TOML document (top-level = map of profile_id → GrindProfile).
    pub fn from_toml_str(s: &str) -> Result<Self, ProfileError> {
        let profiles: HashMap<String, GrindProfile> =
            toml::from_str(s).map_err(|e| ProfileError::Parse(e.to_string()))?;
        for (id, p) in &profiles {
            Self::validate(id, p)?;
        }
        Ok(Self { profiles })
    }

    /// Load + validate from a file path (used at startup).
    pub fn load(path: &Path) -> Result<Self, ProfileError> {
        let s = std::fs::read_to_string(path).map_err(|e| ProfileError::Io(e.to_string()))?;
        Self::from_toml_str(&s)
    }

    #[allow(clippy::neg_cmp_op_on_partial_ord)] // !(x > 0.0) also rejects NaN; x <= 0.0 would accept NaN
    fn validate(id: &str, p: &GrindProfile) -> Result<(), ProfileError> {
        let bad = |reason: &str| ProfileError::Validation {
            profile_id: id.to_string(),
            reason: reason.to_string(),
        };
        if !KNOWN_ROTATIONS.contains(&p.rotation_id.as_str()) {
            return Err(bad(&format!(
                "unknown rotation_id '{}' (known: {:?})",
                p.rotation_id, KNOWN_ROTATIONS
            )));
        }
        if p.creature_type.trim().is_empty() {
            return Err(bad("creature_type must be non-empty"));
        }
        if !(p.wander_radius > 0.0) {
            return Err(bad("wander_radius must be > 0"));
        }
        if !(p.max_search_radius > 0.0) {
            return Err(bad("max_search_radius must be > 0"));
        }
        if p.max_search_radius > p.wander_radius {
            return Err(bad("max_search_radius must be <= wander_radius"));
        }
        if !(p.rest_threshold > 0.0 && p.rest_threshold <= 1.0) {
            return Err(bad("rest_threshold must be in (0, 1]"));
        }
        let vendor_present = [p.vendor_spawn_id.is_some(), p.vendor_pos.is_some(),
                              p.vendor_can_repair.is_some()];
        let n = vendor_present.iter().filter(|b| **b).count();
        if n != 0 && n != 3 {
            return Err(bad(
                "vendor group is all-or-nothing: vendor_spawn_id, vendor_pos, vendor_can_repair",
            ));
        }
        Ok(())
    }

    pub fn get(&self, id: &str) -> Option<&GrindProfile> {
        self.profiles.get(id)
    }

    pub fn len(&self) -> usize {
        self.profiles.len()
    }

    pub fn is_empty(&self) -> bool {
        self.profiles.is_empty()
    }

    /// True if `id` is a known profile (used for the startup roster cross-check).
    pub fn contains(&self, id: &str) -> bool {
        self.profiles.contains_key(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID: &str = r#"
[elwynn_fargodeep]
anchor = { map_id = 0, x = -9690.0, y = 100.0, z = 55.0 }
level_band = { below = 3, above = 2 }
creature_type = "humanoid"
wander_radius = 90.0
max_search_radius = 35.0
rest_threshold = 0.35
rotation_id = "auto_attack"

[dun_morogh_camp]
anchor = { map_id = 0, x = -6240.0, y = 330.0, z = 380.0 }
level_band = { below = 2, above = 3 }
creature_type = "humanoid"
wander_radius = 80.0
max_search_radius = 30.0
rest_threshold = 0.4
rotation_id = "auto_attack"
"#;

    #[test]
    fn loads_valid_multi_profile() {
        let reg = ProfileRegistry::from_toml_str(VALID).expect("valid toml");
        assert_eq!(reg.len(), 2);
        let p = reg.get("elwynn_fargodeep").expect("present");
        assert_eq!(p.anchor.x, -9690.0);
        assert_eq!(p.level_band.below, 3);
        assert_eq!(p.creature_type, "humanoid");
        assert_eq!(p.rotation_id, "auto_attack");
        assert!(p.custom_behavior.is_none());
        assert!(reg.get("nope").is_none());
        assert!(reg.contains("dun_morogh_camp"));
    }

    #[test]
    fn new_rotation_ids_validate() {
        for rid in ["warrior_b1", "rogue_b1", "mage_frost_b1", "priest_smite_b1"] {
            let toml = format!(
                r#"
[camp]
anchor = {{ map_id = 0, x = 1.0, y = 2.0, z = 3.0 }}
level_band = {{ below = 3, above = 2 }}
creature_type = "humanoid"
wander_radius = 90.0
max_search_radius = 35.0
rest_threshold = 0.35
rotation_id = "{rid}"
"#
            );
            assert!(ProfileRegistry::from_toml_str(&toml).is_ok(), "{rid} must be known");
        }
    }

    #[test]
    fn rejects_unknown_rotation_id() {
        let s = VALID.replace(r#"rotation_id = "auto_attack""#, r#"rotation_id = "frostbolt""#);
        let err = ProfileRegistry::from_toml_str(&s).unwrap_err();
        assert!(matches!(err, ProfileError::Validation { .. }), "got {err:?}");
    }

    #[test]
    fn rejects_empty_creature_type() {
        let s = VALID.replace(r#"creature_type = "humanoid""#, r#"creature_type = """#);
        assert!(matches!(ProfileRegistry::from_toml_str(&s).unwrap_err(), ProfileError::Validation { .. }));
    }

    #[test]
    fn rejects_non_positive_radius() {
        let s = VALID.replace("wander_radius = 90.0", "wander_radius = 0.0");
        assert!(matches!(ProfileRegistry::from_toml_str(&s).unwrap_err(), ProfileError::Validation { .. }));
    }

    #[test]
    fn rejects_rest_threshold_out_of_range() {
        let s = VALID.replace("rest_threshold = 0.35", "rest_threshold = 1.5");
        assert!(matches!(ProfileRegistry::from_toml_str(&s).unwrap_err(), ProfileError::Validation { .. }));
    }

    #[test]
    fn rejects_search_radius_exceeding_wander() {
        let s = VALID.replace("max_search_radius = 35.0", "max_search_radius = 200.0");
        assert!(matches!(ProfileRegistry::from_toml_str(&s).unwrap_err(), ProfileError::Validation { .. }));
    }

    #[test]
    fn rejects_malformed_toml() {
        assert!(matches!(ProfileRegistry::from_toml_str("not = [valid").unwrap_err(), ProfileError::Parse(_)));
    }

    #[test]
    fn accepts_optional_custom_behavior() {
        let with_cb = VALID.replace(
            "rotation_id = \"auto_attack\"\n\n[dun_morogh_camp]",
            "rotation_id = \"auto_attack\"\ncustom_behavior = \"escort_v1\"\n\n[dun_morogh_camp]",
        );
        let reg = ProfileRegistry::from_toml_str(&with_cb).expect("valid with custom_behavior");
        assert_eq!(reg.get("elwynn_fargodeep").unwrap().custom_behavior.as_deref(), Some("escort_v1"));
    }

    #[test]
    fn rejects_unknown_field_in_profile() {
        // A typo'd key (restthreshold) must now fail at parse, not silently drop.
        let s = VALID.replace(
            "rotation_id = \"auto_attack\"\n\n[dun_morogh_camp]",
            "rotation_id = \"auto_attack\"\nrestthreshold = 0.99\n\n[dun_morogh_camp]",
        );
        assert!(matches!(ProfileRegistry::from_toml_str(&s).unwrap_err(), ProfileError::Parse(_)));
    }

    #[test]
    fn rejects_rest_threshold_zero() {
        // rest_threshold range is (0, 1] — zero is invalid.
        let s = VALID.replace("rest_threshold = 0.35", "rest_threshold = 0.0");
        assert!(matches!(ProfileRegistry::from_toml_str(&s).unwrap_err(), ProfileError::Validation { .. }));
    }

    #[test]
    fn rejects_non_positive_max_search_radius() {
        let s = VALID.replace("max_search_radius = 35.0", "max_search_radius = 0.0");
        assert!(matches!(ProfileRegistry::from_toml_str(&s).unwrap_err(), ProfileError::Validation { .. }));
    }

    #[test]
    fn ships_valid_profiles_toml() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/profiles/profiles.toml");
        let reg = ProfileRegistry::load(std::path::Path::new(path))
            .expect("shipped profiles.toml must load + validate");
        assert!(reg.len() >= 2, "expect 2-3 camp profiles");
        assert!(reg.contains("elwynn_fargodeep"));
    }

    const VENDOR_GROUP: &str = r#"
vendor_spawn_id = 40001
vendor_pos = { x = 2200.0, y = -300.0, z = 95.0 }
vendor_can_repair = true
"#;

    #[test]
    fn accepts_full_vendor_group() {
        let s = VALID.replace(
            "rotation_id = \"auto_attack\"\n\n[dun_morogh_camp]",
            &format!("rotation_id = \"auto_attack\"\n{VENDOR_GROUP}\n[dun_morogh_camp]"),
        );
        let reg = ProfileRegistry::from_toml_str(&s).expect("full vendor group valid");
        let p = reg.get("elwynn_fargodeep").unwrap();
        assert_eq!(p.vendor_spawn_id, Some(40001));
        assert_eq!(p.vendor_pos.as_ref().map(|v| v.x), Some(2200.0));
        assert_eq!(p.vendor_can_repair, Some(true));
        // dun_morogh_camp has no group — must stay None and still validate.
        assert!(reg.get("dun_morogh_camp").unwrap().vendor_spawn_id.is_none());
    }

    #[test]
    fn rejects_partial_vendor_group() {
        // Each single field alone must fail validation (all-or-nothing, spec §7).
        for field in ["vendor_spawn_id = 40001",
                      r#"vendor_pos = { x = 1.0, y = 2.0, z = 3.0 }"#,
                      "vendor_can_repair = true"] {
            let s = VALID.replace(
                "rotation_id = \"auto_attack\"\n\n[dun_morogh_camp]",
                &format!("rotation_id = \"auto_attack\"\n{field}\n\n[dun_morogh_camp]"),
            );
            let err = ProfileRegistry::from_toml_str(&s).unwrap_err();
            assert!(matches!(err, ProfileError::Validation { .. }),
                    "partial group '{field}' must be a Validation error, got {err:?}");
        }
    }
}
