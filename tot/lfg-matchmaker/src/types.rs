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
            "tank" => Some(Self::Tank),
            "healer" | "heal" => Some(Self::Healer),
            "dps" | "damage" => Some(Self::Dps),
            _ => None,
        }
    }

    /// Decode an LFG roles bitmask (TANK=2, HEALER=4, DAMAGE=8) to a primary role.
    /// Priority: tank > healer > damage. Returns None only if no recognised bit is set.
    pub fn from_lfg_bitmask(bits: u32) -> Option<Role> {
        if bits & 2 != 0 {
            Some(Self::Tank)
        } else if bits & 4 != 0 {
            Some(Self::Healer)
        } else if bits & 8 != 0 {
            Some(Self::Dps)
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Faction {
    Alliance,
    Horde,
}

impl Faction {
    pub fn parse(s: &str) -> Option<Faction> {
        match s.to_ascii_lowercase().as_str() {
            "alliance" | "a" => Some(Faction::Alliance),
            "horde" | "h" => Some(Faction::Horde),
            _ => None,
        }
    }

    /// Map a race string from `obs.get_state` result.self.race to a faction.
    /// Alliance: human, dwarf, nightelf, night_elf, gnome, draenei.
    /// Everything else (orc, undead, tauren, troll, bloodelf, blood_elf) → Horde.
    pub fn from_race(race: &str) -> Faction {
        match race.to_ascii_lowercase().replace('-', "_").as_str() {
            "human" | "dwarf" | "nightelf" | "night_elf" | "gnome" | "draenei" => Faction::Alliance,
            _ => Faction::Horde,
        }
    }

    /// Decode a WoW team_id (0 = Alliance, 1 = Horde) to a Faction.
    pub fn from_team_id(id: u32) -> Faction {
        if id == 0 { Faction::Alliance } else { Faction::Horde }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueueEntry {
    pub guid: u64,
    pub role: Role,
    pub dungeon_id: u32,
    pub faction: Faction,
    /// True when this entry was injected by (or on behalf of) a real human player,
    /// as opposed to a bot. Affects placement: real players are placed by
    /// `lfg.form_group`'s TeleportPlayer; bots must be placed via `bot.enter_instance`.
    #[serde(default)]
    pub is_real_player: bool,
}

/// A pending real-player or bot LFG intent, decoded from `obs.lfg_pending`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Intent {
    pub guid: u64,
    pub faction: Faction,
    pub role: Role,
    pub dungeon_ids: Vec<u32>,
    pub is_real_player: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MatchProposal {
    pub dungeon_id: u32,
    pub tank: u64,
    pub healer: u64,
    pub dps: Vec<u64>, // exactly 3
    /// True when the proposal contains a real human player (placed by lfg.form_group
    /// TeleportPlayer + bot.enter_instance for the bot members). False means all-bot
    /// (invite/accept path via the existing `fulfill` orchestration).
    pub has_real_player: bool,
    /// Guid of the real player in this proposal, if any.
    pub real_player_guid: Option<u64>,
}

impl MatchProposal {
    /// Group members, leader (tank) first.
    pub fn members(&self) -> Vec<u64> {
        let mut v = vec![self.tank, self.healer];
        v.extend(&self.dps);
        v
    }
    pub const fn leader(&self) -> u64 {
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
    fn faction_parse_accepts_aliases() {
        assert_eq!(Faction::parse("alliance"), Some(Faction::Alliance));
        assert_eq!(Faction::parse("A"), Some(Faction::Alliance));
        assert_eq!(Faction::parse("HORDE"), Some(Faction::Horde));
        assert_eq!(Faction::parse("h"), Some(Faction::Horde));
        assert_eq!(Faction::parse("neutral"), None);
    }

    #[test]
    fn proposal_members_leader_first() {
        let p = MatchProposal {
            dungeon_id: 1,
            tank: 10,
            healer: 20,
            dps: vec![30, 31, 32],
            has_real_player: false,
            real_player_guid: None,
        };
        assert_eq!(p.leader(), 10);
        assert_eq!(p.members(), vec![10, 20, 30, 31, 32]);
    }

    #[test]
    fn role_from_lfg_bitmask_priority() {
        // TANK bit wins over others.
        assert_eq!(Role::from_lfg_bitmask(2), Some(Role::Tank));
        assert_eq!(Role::from_lfg_bitmask(4), Some(Role::Healer));
        assert_eq!(Role::from_lfg_bitmask(8), Some(Role::Dps));
        // Priority: tank > healer > damage when multiple bits set.
        assert_eq!(Role::from_lfg_bitmask(2 | 4 | 8), Some(Role::Tank));
        assert_eq!(Role::from_lfg_bitmask(4 | 8), Some(Role::Healer));
        // Zero → None.
        assert_eq!(Role::from_lfg_bitmask(0), None);
        // Unrecognised bit → None.
        assert_eq!(Role::from_lfg_bitmask(1), None);
    }

    #[test]
    fn faction_from_race_alliance_strings() {
        for race in &["human", "Human", "dwarf", "nightelf", "night_elf", "gnome", "draenei"] {
            assert_eq!(
                Faction::from_race(race),
                Faction::Alliance,
                "race {race} should be Alliance"
            );
        }
    }

    #[test]
    fn faction_from_race_horde_strings() {
        for race in &["orc", "undead", "tauren", "troll", "bloodelf", "blood_elf", "goblin"] {
            assert_eq!(
                Faction::from_race(race),
                Faction::Horde,
                "race {race} should be Horde"
            );
        }
    }

    #[test]
    fn faction_from_team_id() {
        assert_eq!(Faction::from_team_id(0), Faction::Alliance);
        assert_eq!(Faction::from_team_id(1), Faction::Horde);
        assert_eq!(Faction::from_team_id(99), Faction::Horde);
    }

    #[test]
    fn queue_entry_is_real_player_defaults_false_on_deserialize() {
        let json = r#"{"guid":1,"role":"tank","dungeon_id":4,"faction":"horde"}"#;
        let e: QueueEntry = serde_json::from_str(json).unwrap();
        assert!(!e.is_real_player);
    }
}
