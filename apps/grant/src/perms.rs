//! Permission bits per scope, transcribed from SAGE forge's permission tables (not public); the
//! IDLs do not define the bit values. Each scope's program gives its own bits meaning; bit 63
//! (`DRAIN_SOL_VAULT`) means the same in every scope.

use crate::profile::{PLAYER_PROFILE_PROGRAM, PROFILE_FACTION_PROGRAM, SAGE_PROGRAM};

pub struct Bit {
    pub index: u32,
    pub name: &'static str,
    /// A player action a session key may be granted; the rest are admin bits.
    pub player: bool,
}

const fn b(bit: u32, name: &'static str, player: bool) -> Bit {
    Bit {
        index: bit,
        name,
        player,
    }
}

// ANCHOR: bits
pub const DRAIN_SOL_VAULT: u32 = 63;

pub const SAGE: &[Bit] = &[
    b(0, "MANAGE_GAME", false),
    b(1, "MANAGE_SYSTEM", false),
    b(2, "MANAGE_STAR", false),
    b(3, "MANAGE_PLANET", false),
    b(4, "MANAGE_SHIP", false),
    b(5, "MANAGE_CHARACTER", false),
    b(6, "MANAGE_STAR_BASE", false),
    b(7, "MANAGE_MINE_ITEM", false),
    b(8, "MANAGE_RESOURCE", false),
    b(9, "REMOVE_SHIP_ESCROW", true),
    b(10, "MOVE_FLEET", true),
    b(11, "UNDOCK", true),
    b(12, "DOCK", true),
    b(13, "RENT_FLEET", true),
    b(14, "DO_CRAFTING", true),
    b(15, "MANAGE_CARGO_POD", true),
    b(16, "ADD_REMOVE_CARGO", true),
    b(17, "DO_STAR_BASE_UPGRADES", true),
    b(18, "MANAGE_FLEET", true),
    b(19, "MANAGE_FLEET_CARGO", true),
    b(20, "DO_MINING", true),
    b(21, "RESPAWN", true),
    b(22, "MANAGE_SURVEY_DATA_UNIT", true),
    b(23, "SCAN_SURVEY_DATA_UNIT", true),
    b(24, "DO_STAR_BASE_UPKEEP", true),
    b(25, "MANAGE_PROGRESSION", false),
    b(26, "MANAGE_CREW_CONFIG", false),
    b(27, "MANAGE_CREW", true),
    b(28, "WITHDRAW_CREW", true),
    b(29, "MANAGE_REGION", false),
    b(30, "MANAGE_CARGO", false),
    b(31, "MANAGE_CLAIM_STAKE_CONFIG", false),
    b(32, "MANAGE_COMBAT", false),
    b(33, "MANAGE_CLAIM_STAKE", true),
    b(34, "MANAGE_CRAFTING", false),
    b(35, "ENGAGE_IN_COMBAT", true),
    b(36, "REPAIR", true),
    b(37, "MANAGE_SCANNING", false),
    b(38, "SCAN", true),
    b(39, "MANAGE_LOCAL_MARKET_ORDERS", true),
    b(40, "DO_RESEARCH", true),
    b(41, "DAILY_CHECK_IN", true),
    b(42, "MANAGE_CURRENCY", true),
];

pub const PROFILE: &[Bit] = &[
    b(0, "AUTH", false),
    b(1, "ADD_KEYS", false),
    b(2, "REMOVE_KEYS", false),
    b(3, "CHANGE_NAME", false),
    b(4, "CREATE_ROLE", false),
    b(5, "REMOVE_ROLE", false),
    b(6, "SET_AUTHORIZER", false),
    b(7, "JOIN_ROLE", false),
    b(8, "LEAVE_ROLE", false),
    b(9, "TOGGLE_ACCEPTING_NEW_MEMBERS", false),
    b(10, "ADD_MEMBER", false),
    b(11, "REMOVE_MEMBER", false),
];

pub const FACTION: &[Bit] = &[b(0, "SET_FACTION", false)];
// ANCHOR_END: bits

pub struct Preset {
    pub name: &'static str,
    pub bits: &'static [u32],
}

// ANCHOR: presets
/// SAGE presets for a session key. None includes `DRAIN_SOL_VAULT`; it is a separate choice.
pub const PRESETS: &[Preset] = &[
    Preset {
        name: "Fleet and movement",
        bits: &[10, 11, 12, 18, 19],
    },
    Preset {
        name: "Mining and cargo",
        bits: &[16, 19, 20, 38],
    },
    Preset {
        name: "Market trading",
        bits: &[16, 19, 39],
    },
    Preset {
        name: "All player actions",
        bits: &[
            9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 27, 28, 33, 35, 36, 38,
            39, 40, 41, 42,
        ],
    },
];
// ANCHOR_END: presets

pub fn mask(bits: &[u32]) -> u64 {
    bits.iter()
        .fold(0, |m, bit| m | 1u64.checked_shl(*bit).unwrap_or(0))
}

pub const fn has(mask: u64, bit: u32) -> bool {
    match 1u64.checked_shl(bit) {
        Some(b) => mask & b != 0,
        None => false,
    }
}

pub fn scope_label(scope: &str) -> &str {
    match scope {
        SAGE_PROGRAM => "SAGE",
        PLAYER_PROFILE_PROGRAM => "Profile",
        PROFILE_FACTION_PROGRAM => "Faction",
        other => other,
    }
}

/// The names of the bits set in `mask` under `scope`; unknown bits as `bit N`.
pub fn names(scope: &str, mask: u64) -> Vec<String> {
    if mask == u64::MAX {
        return vec!["ALL".into()];
    }
    let table = match scope {
        SAGE_PROGRAM => SAGE,
        PLAYER_PROFILE_PROGRAM => PROFILE,
        PROFILE_FACTION_PROGRAM => FACTION,
        _ => &[],
    };
    (0..64)
        .filter(|bit| has(mask, *bit))
        .map(|bit| match table.iter().find(|b| b.index == bit) {
            Some(b) => b.name.to_string(),
            None if bit == DRAIN_SOL_VAULT => "DRAIN_SOL_VAULT".into(),
            None => format!("bit {bit}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_hold_only_player_bits() {
        for p in PRESETS {
            for bit in p.bits {
                let entry = SAGE.iter().find(|b| b.index == *bit).unwrap();
                assert!(entry.player, "{} in {}", entry.name, p.name);
            }
        }
    }

    #[test]
    fn names_bits_per_scope() {
        let m = mask(&[10, 20, 63, 50]);
        assert_eq!(
            names(SAGE_PROGRAM, m),
            ["MOVE_FLEET", "DO_MINING", "bit 50", "DRAIN_SOL_VAULT"]
        );
        assert_eq!(
            names(PLAYER_PROFILE_PROGRAM, 0b110),
            ["ADD_KEYS", "REMOVE_KEYS"]
        );
        assert_eq!(names(SAGE_PROGRAM, u64::MAX), ["ALL"]);
        assert!(has(m, 63) && !has(m, 64));
    }
}
