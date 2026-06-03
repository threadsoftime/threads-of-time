use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionKind {
    Action,
    #[serde(rename = "no_op")]
    NoOp,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Decision {
    pub kind: DecisionKind,
    pub tool: Option<String>,
    pub args: Option<serde_json::Value>,
    pub confidence: f64,
    pub reasoning: String,
    pub wakeup_in_ms: Option<i64>,
}

impl Decision {
    pub fn validate(&self) -> Result<(), String> {
        if self.kind == DecisionKind::Action
            && (self.tool.is_none() || self.args.is_none())
        {
            return Err("ACTION decision requires tool and args".to_string());
        }
        Ok(())
    }
}

/// Per-bot personality seed (mirrors models.py PersonalityCard).
/// "class" is a Rust keyword; the field is named class_ internally but
/// serializes/deserializes as "class" via #[serde(rename)].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersonalityCard {
    pub name: String,
    pub race: String,
    #[serde(rename = "class")]
    pub class_: String,
    pub backstory: String,
    pub talkativeness: f64,
    pub courage: f64,
    pub greed: f64,
    pub attitude_to_master: f64,
    #[serde(default = "default_party_invite_policy")]
    pub party_invite_policy: String,
    // v2 fields — None signals "needs morph"
    #[serde(default)]
    pub pvp_appetite: Option<f64>,
    #[serde(default)]
    pub raid_appetite: Option<f64>,
    #[serde(default)]
    pub completionist_streak: Option<f64>,
    #[serde(default)]
    pub gold_motivation: Option<f64>,
    #[serde(default)]
    pub profession_appetite: Option<f64>,
}

fn default_party_invite_policy() -> String {
    "accept_from_known".to_string()
}

#[derive(Debug, Clone)]
pub struct TriageResult {
    pub should_decide: bool,
    pub reason: String,
    pub hot_inputs: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone)]
pub struct TickState {
    pub bot_guid: i64,
    pub last_tick_ms: i64,
    pub last_decision_id: Option<String>,
}

impl TickState {
    pub fn new(bot_guid: i64) -> Self {
        Self { bot_guid, last_tick_ms: 0, last_decision_id: None }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decision_kind_serialization() {
        assert_eq!(
            serde_json::to_string(&DecisionKind::Action).unwrap(),
            "\"action\""
        );
        assert_eq!(
            serde_json::to_string(&DecisionKind::NoOp).unwrap(),
            "\"no_op\""
        );
    }

    #[test]
    fn test_decision_action_requires_tool_and_args() {
        // Valid action
        let d = Decision {
            kind: DecisionKind::Action,
            tool: Some("bot.send_chat".to_string()),
            args: Some(serde_json::json!({"bot_guid": 1})),
            confidence: 0.9,
            reasoning: "test".to_string(),
            wakeup_in_ms: None,
        };
        assert!(d.validate().is_ok());
        // Invalid: action without tool
        let bad = Decision {
            kind: DecisionKind::Action,
            tool: None,
            args: Some(serde_json::json!({})),
            confidence: 0.9,
            reasoning: "test".to_string(),
            wakeup_in_ms: None,
        };
        assert!(bad.validate().is_err());
    }

    #[test]
    fn test_no_op_decision_round_trips_json() {
        let d = Decision {
            kind: DecisionKind::NoOp,
            tool: None,
            args: None,
            confidence: 0.0,
            reasoning: "llm_unparseable: foo".to_string(),
            wakeup_in_ms: None,
        };
        let json = serde_json::to_string(&d).unwrap();
        let back: Decision = serde_json::from_str(&json).unwrap();
        assert_eq!(back.kind, DecisionKind::NoOp);
        assert_eq!(back.reasoning, "llm_unparseable: foo");
    }

    #[test]
    fn test_personality_card_class_alias() {
        // "class" is a Rust keyword; must serialize/deserialize as "class" not "class_"
        let json = r#"{
            "name": "Kael", "race": "Blood Elf", "class": "Paladin",
            "backstory": "A noble warrior.", "talkativeness": 0.7, "courage": 0.8,
            "greed": 0.3, "attitude_to_master": 0.0,
            "party_invite_policy": "accept_from_known"
        }"#;
        let card: PersonalityCard = serde_json::from_str(json).unwrap();
        assert_eq!(card.class_, "Paladin");
        let back = serde_json::to_value(&card).unwrap();
        assert_eq!(back["class"], "Paladin");
        assert!(back.get("class_").is_none());
    }

    #[test]
    fn test_personality_card_v2_fields_default_to_none() {
        let json = r#"{"name":"K","race":"Human","class":"Warrior","backstory":"x",
            "talkativeness":0.5,"courage":0.5,"greed":0.3,"attitude_to_master":0.0}"#;
        let card: PersonalityCard = serde_json::from_str(json).unwrap();
        assert!(card.pvp_appetite.is_none());
        assert!(card.raid_appetite.is_none());
    }

    #[test]
    fn test_tick_state_new() {
        let ts = TickState::new(12345);
        assert_eq!(ts.bot_guid, 12345);
        assert_eq!(ts.last_tick_ms, 0);
        assert!(ts.last_decision_id.is_none());
    }

}
