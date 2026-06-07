//! Declarative grind profiles (M2 slice 2.1). serde/TOML-loaded, fail-fast validated.
//! Externalizes the goal_emitter grind constants with explicit per-camp anchors.

use std::collections::HashMap;
use std::path::Path;

use serde::Deserialize;

/// Known rotation ids (2.1 ships only AutoAttack; 2.2 grows this).
const KNOWN_ROTATIONS: &[&str] = &["auto_attack"];

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
}
