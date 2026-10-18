//! Reads ai.ini: the computer player's AI triggers, team types, task forces and
//! scripts, with the rules.ini numbers that go with them.
//!
//! The sections, keys and line formats.

use ra2_formats::ini::Ini;
use ra2_sim::teams::{Data, TeamType, Thing, Trigger};

fn clean(value: &str) -> &str {
    value.split(';').next().unwrap_or("").trim()
}

/// Builds the data from ai.ini and rules.ini.
pub fn load(ai: &Ini, rules: &Ini, unit: &dyn Fn(&str) -> Option<u16>, building: &dyn Fn(&str) -> Option<u16>, country: &dyn Fn(&str) -> Option<u8>) -> Data {
    let ids = |section: &str| -> Vec<String> { ai.section(section).iter().map(|(_, id)| clean(id).to_owned()).filter(|id| !id.is_empty()).collect() };
    let yes = |section: &str, key: &str| ai.get(section, key).is_some_and(|v| matches!(clean(v).to_lowercase().as_str(), "yes" | "true" | "1"));
    let number = |section: &str, key: &str, default: i32| ai.get(section, key).and_then(|v| clean(v).parse().ok()).unwrap_or(default);

    // Keys `0` to `5`: `count,Type`. An entry whose type is unknown is dropped.
    let force_ids = ids("TaskForces");
    let task_forces: Vec<Vec<(u32, u16)>> = force_ids
        .iter()
        .map(|id| {
            (0..6)
                .filter_map(|key| {
                    let (count, kind) = ai.get(id, &key.to_string())?.split_once(',')?;
                    Some((clean(count).parse().ok()?, unit(clean(kind))?))
                })
                .collect()
        })
        .collect();
    // Keys `0` to `49`: `action,argument`; missing keys are passed over.
    let script_ids = ids("ScriptTypes");
    let scripts: Vec<Vec<(i32, i32)>> = script_ids
        .iter()
        .map(|id| {
            (0..50)
                .filter_map(|key| {
                    let (action, argument) = ai.get(id, &key.to_string())?.split_once(',')?;
                    Some((clean(action).parse().ok()?, clean(argument).parse().ok()?))
                })
                .collect()
        })
        .collect();
    let team_ids = ids("TeamTypes");
    let find = |list: &[String], id: &str| list.iter().position(|entry| entry.eq_ignore_ascii_case(id.trim()));
    let team_types: Vec<TeamType> = team_ids
        .iter()
        .map(|id| TeamType {
            name: ai.get(id, "Name").map(|v| clean(v).to_owned()).unwrap_or_default(),
            max: number(id, "Max", 0),
            priority: number(id, "Priority", 0),
            base_defense: yes(id, "IsBaseDefense"),
            reinforce: yes(id, "Reinforce"),
            annoyance: yes(id, "Annoyance"),
            recruiter: yes(id, "Recruiter"),
            aggressive: yes(id, "Aggressive"),
            suicide: yes(id, "Suicide"),
            only_target_house_enemy: yes(id, "OnlyTargetHouseEnemy"),
            members_recruitable: yes(id, "AreTeamMembersRecruitable"),
            transports_return: yes(id, "TransportsReturnOnUnload"),
            script: ai.get(id, "Script").and_then(|v| find(&script_ids, clean(v))).unwrap_or(usize::MAX),
            task_force: ai.get(id, "TaskForce").and_then(|v| find(&force_ids, clean(v))).unwrap_or(usize::MAX),
            autocreate: yes(id, "Autocreate"),
        })
        .collect();
    let triggers: Vec<Trigger> = ai
        .section("AITriggerTypes")
        .iter()
        .filter_map(|(_, line)| {
            let fields: Vec<&str> = line.split(',').map(str::trim).collect();
            if fields.len() < 7 {
                return None;
            }
            let team = |at: usize| fields.get(at).and_then(|id| find(&team_ids, id));
            let flag = |at: usize, default: bool| fields.get(at).map_or(default, |v| *v != "0");
            let weight = |at: usize| fields.get(at).and_then(|v| v.parse::<f64>().ok()).map_or(1.0, f64::trunc);
            // The comparator: the operand then the operator, each four bytes,
            // low byte first, in hexadecimal.
            let word = |at: usize| -> i32 {
                let hex = fields[6].get(at * 8..at * 8 + 8).unwrap_or("00000000");
                let bytes: Vec<u8> = (0..4).map(|i| u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).unwrap_or(0)).collect();
                i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
            };
            Some(Trigger {
                team1: team(1),
                team2: team(14),
                owner: if fields[2].eq_ignore_ascii_case("<all>") { Some(None) } else { country(fields[2]).map(Some) },
                condition: fields[4].parse().unwrap_or(-1),
                // Tried as a soldier, vehicle or aircraft type, then a building.
                object: unit(fields[5]).map(Thing::Unit).or_else(|| building(fields[5]).map(Thing::Building)),
                operand: word(0),
                operator: word(1),
                weight: weight(7),
                min_weight: weight(8),
                max_weight: weight(9),
                skirmish: flag(10, false),
                side: fields.get(12).and_then(|v| v.parse().ok()).unwrap_or(0),
                enabled: [flag(15, true), flag(16, true), flag(17, true)],
                successes: 0,
                completed: 0,
            })
        })
        .collect();

    let general = |key: &str| rules.get("General", key).map(clean);
    let triple = |key: &str, default: [i32; 3]| -> [i32; 3] {
        let list: Vec<i32> = general(key).map(|v| v.split(',').filter_map(|n| n.trim().parse().ok()).collect()).unwrap_or_default();
        [0, 1, 2].map(|i| list.get(i).copied().unwrap_or(default[i]))
    };
    let float = |section: &str, key: &str, default: f64| rules.get(section, key).map(clean).and_then(|v| v.parse::<f64>().ok()).unwrap_or(default);
    let defaults = Data::default();
    Data {
        triggers,
        team_types,
        task_forces,
        scripts,
        team_delays: triple("TeamDelays", [2000, 2500, 3500]).map(|v| v.max(1) as u32),
        min_defensive: triple("MinimumAIDefensiveTeams", [1; 3]),
        max_defensive: triple("MaximumAIDefensiveTeams", [2; 3]),
        team_cap: triple("TotalAITeamCap", [30; 3]),
        use_min_defense: general("UseMinDefenseRule").is_none_or(|v| matches!(v.to_lowercase().as_str(), "yes" | "true" | "1")),
        dissolve_delay: float("General", "DissolveUnfilledTeamDelay", 5000.0) as u32,
        success_delta: float("General", "AITriggerSuccessWeightDelta", defaults.success_delta),
        failure_delta: float("General", "AITriggerFailureWeightDelta", defaults.failure_delta),
        track_record: float("General", "AITriggerTrackRecordCoefficient", defaults.track_record),
        safe_distance: float("General", "AISafeDistance", 20.0) as i32,
        stray: (float("General", "Stray", 2.0) * 256.0) as i32,
        relaxed_stray: (float("General", "RelaxedStray", 3.0) * 256.0) as i32,
        close_enough: (float("General", "CloseEnough", 2.25) * 256.0) as i32,
        enemy_bonus: float("General", "EnemyHouseThreatBonus", defaults.enemy_bonus),
        defense_response: float("General", "ComputerBaseDefenseResponse", 3.0) as i32,
        defense_delay: (float("General", "BaseDefenseDelay", 0.25) * 900.0) as u32,
    }
}
