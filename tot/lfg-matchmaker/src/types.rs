use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Tank,
    Healer,
    Dps,
}

impl Role {
    pub fn parse(s: &str) -> Option<Role> {
        match s.to_ascii_lowercase().as_str() {
            "tank" => Some(Role::Tank),
            "healer" | "heal" => Some(Role::Healer),
            "dps" | "damage" => Some(Role::Dps),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueueEntry {
    pub guid: u64,
    pub role: Role,
    pub dungeon_id: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MatchProposal {
    pub dungeon_id: u32,
    pub tank: u64,
    pub healer: u64,
    pub dps: Vec<u64>, // exactly 3
}

impl MatchProposal {
    /// Group members, leader (tank) first.
    pub fn members(&self) -> Vec<u64> {
        let mut v = vec![self.tank, self.healer];
        v.extend(&self.dps);
        v
    }
    pub fn leader(&self) -> u64 {
        self.tank
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn role_parse_accepts_aliases() {
        assert_eq!(Role::parse("tank"), Some(Role::Tank));
        assert_eq!(Role::parse("HEAL"), Some(Role::Healer));
        assert_eq!(Role::parse("healer"), Some(Role::Healer));
        assert_eq!(Role::parse("dps"), Some(Role::Dps));
        assert_eq!(Role::parse("damage"), Some(Role::Dps));
        assert_eq!(Role::parse("bogus"), None);
    }

    #[test]
    fn proposal_members_leader_first() {
        let p = MatchProposal { dungeon_id: 1, tank: 10, healer: 20, dps: vec![30, 31, 32] };
        assert_eq!(p.leader(), 10);
        assert_eq!(p.members(), vec![10, 20, 30, 31, 32]);
    }
}
