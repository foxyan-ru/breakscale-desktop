//! `challenges_list`/`challenge_start` commands, invented by the frontend's
//! Challenge/Challenges port -- see
//! `desktop/src/lib/components/challenges/Challenges.svelte`'s integration
//! notes. Pass/fail evaluation is deliberately NOT a command: the frontend
//! already receives a `SimSnapshot` at 10Hz and evaluates locally
//! (`evaluateChallenge` in `api/challenges.ts`, a port of
//! `sim::challenge::evaluate`) rather than adding an IPC round trip for
//! arithmetic it already has the inputs for.

use crate::error::{AppError, AppResult};
use crate::sim::challenge::{self, Challenge};
use crate::sim::challenges;
use crate::sim::presets;
use crate::sim::types::Topology;
use serde::Serialize;

/// Enough to render the picker grid without the full brief/goals/hints.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChallengeSummary {
    pub id: String,
    pub name: String,
    pub brief: String,
}

#[tauri::command]
pub fn challenges_list() -> AppResult<Vec<ChallengeSummary>> {
    Ok(challenges::challenges()
        .into_iter()
        .map(|c| ChallengeSummary {
            id: c.id,
            name: c.name,
            brief: c.brief,
        })
        .collect())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChallengeStart {
    pub challenge: Challenge,
    pub topology: Topology,
}

/// Resolve a challenge's preset and apply its judged load. Deliberately
/// does NOT touch the live engine itself -- the frontend still calls
/// `sim_new`/`sim_set_topology` with the returned topology, the same
/// already-finished commands any other "load a design" path uses, so this
/// command has exactly one job.
#[tauri::command]
pub fn challenge_start(id: String) -> AppResult<ChallengeStart> {
    let challenge = challenges::challenge_by_id(&id)
        .ok_or_else(|| AppError::NotFound(format!("\"{id}\" is not a known challenge.")))?;
    let preset = presets::preset_by_id(&challenge.preset_id).ok_or_else(|| {
        AppError::State(format!(
            "Challenge \"{id}\" points at a preset that no longer exists."
        ))
    })?;
    let topology = challenge::apply_load(preset.topology, challenge.load_rps);
    Ok(ChallengeStart { challenge, topology })
}
