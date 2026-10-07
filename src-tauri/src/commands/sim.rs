//! `sim_*` commands. Names/argument shapes match `desktop/src/lib/api/sim.ts`.
//!
//! Every command here mutates the one live `Engine` held in
//! `SimulationState` (see `state.rs`); the background tick thread `state.rs`
//! spawns is what actually advances it and pushes `sim://snapshot` events,
//! so most of these commands are a lock, a call, and a return -- no timing
//! logic of their own.

use crate::error::{AppError, AppResult};
use crate::sim::engine::Engine;
use crate::sim::types::{FailureKind, FailureOpts, SimSnapshot, Topology};
use crate::state::SimulationState;
use tauri::{AppHandle, State};

fn require_engine<T>(
    state: &State<'_, SimulationState>,
    f: impl FnOnce(&mut Engine) -> T,
) -> AppResult<T> {
    state
        .with_engine(f)
        .ok_or_else(|| AppError::State("No design is loaded yet.".into()))
}

#[tauri::command]
pub fn sim_new(
    app: AppHandle,
    state: State<'_, SimulationState>,
    topology: Topology,
    seed: Option<u32>,
) -> AppResult<()> {
    let engine = Engine::new(topology, seed.unwrap_or(1));
    state.install(&app, engine);
    Ok(())
}

#[tauri::command]
pub fn sim_set_topology(state: State<'_, SimulationState>, topology: Topology) -> AppResult<()> {
    require_engine(&state, |e| e.set_topology(topology))
}

#[tauri::command]
pub fn sim_update_node_config(
    state: State<'_, SimulationState>,
    node_id: String,
    patch: serde_json::Value,
) -> AppResult<()> {
    require_engine(&state, move |e| e.update_node_config(&node_id, patch))?
}

#[tauri::command]
pub fn sim_inject_failure(
    state: State<'_, SimulationState>,
    node_id: String,
    kind: FailureKind,
    opts: FailureOpts,
) -> AppResult<()> {
    require_engine(&state, move |e| e.inject_failure(&node_id, kind, opts))
}

#[tauri::command]
pub fn sim_clear_failure(state: State<'_, SimulationState>, node_id: String) -> AppResult<()> {
    require_engine(&state, move |e| e.clear_failure(&node_id))
}

#[tauri::command]
pub fn sim_reset(state: State<'_, SimulationState>) -> AppResult<()> {
    require_engine(&state, |e| e.reset())
}

#[tauri::command]
pub fn sim_set_running(state: State<'_, SimulationState>, running: bool) -> AppResult<()> {
    state.set_running(running);
    Ok(())
}

#[tauri::command]
pub fn sim_get_snapshot(state: State<'_, SimulationState>) -> AppResult<SimSnapshot> {
    require_engine(&state, |e| e.snapshot())
}
