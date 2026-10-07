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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    /// The exact argument JSON `simNew()` in `src/lib/api/sim.ts` sends for
    /// the frontend's startup fallback (blank topology, default seed) --
    /// `invoke('sim_new', { topology, seed })`. If this stops parsing, the
    /// very first engine command of every session rejects before a design
    /// exists, which is the failure mode behind "No design is loaded yet."
    #[test]
    fn sim_new_args_parse_blank_startup_payload() {
        let args: Value =
            serde_json::from_str(r#"{ "topology": { "nodes": [], "edges": [] }, "seed": 1 }"#)
                .unwrap();
        let topology: Topology = serde_json::from_value(args["topology"].clone()).unwrap();
        let seed: Option<u32> = serde_json::from_value(args["seed"].clone()).unwrap();
        assert_eq!(seed, Some(1));
        assert!(topology.nodes.is_empty() && topology.annotations.is_none());
        Engine::new(topology, seed.unwrap_or(1));
    }

    /// A payload shaped exactly like what the frontend assembles for
    /// `sim_new` after loading an example: `makeNode`-shaped nodes (all 15
    /// non-optional `NodeConfig` fields -- see `defaultNodeConfig` in
    /// `geometry.ts`), an `addEdge`-shaped edge, and the store's
    /// annotations. Pins the camelCase wire contract both ways.
    #[test]
    fn sim_new_args_parse_a_full_frontend_topology() {
        let args: Value = serde_json::from_str(
            r#"{
                "topology": {
                    "nodes": [
                        { "id": "node-1", "kind": "client", "label": "Client",
                          "x": 120, "y": 240,
                          "config": { "capacity": 10, "serviceMs": 20, "serviceCv": 0.5,
                                      "queueLimit": 100, "hitRate": 0, "errorRate": 0.01,
                                      "timeoutMs": 2000, "retries": 0, "rps": 50,
                                      "replicaCount": 2, "replicationLagMs": 50,
                                      "readFraction": 0.8, "shardCount": 4,
                                      "shardCapacity": 10, "hotKeyFraction": 0 } },
                        { "id": "node-2", "kind": "service", "label": "Service",
                          "x": 420, "y": 240,
                          "config": { "capacity": 10, "serviceMs": 20, "serviceCv": 0.5,
                                      "queueLimit": 100, "hitRate": 0, "errorRate": 0.01,
                                      "timeoutMs": 2000, "retries": 0, "rps": 0,
                                      "replicaCount": 2, "replicationLagMs": 50,
                                      "readFraction": 0.8, "shardCount": 4,
                                      "shardCapacity": 10, "hotKeyFraction": 0 } }
                    ],
                    "edges": [
                        { "id": "edge-1", "from": "node-1", "to": "node-2", "weight": 1 }
                    ],
                    "annotations": [
                        { "id": "ann-1", "kind": "note", "text": "entry point",
                          "x": 16, "y": 16, "width": 200, "size": 16 }
                    ]
                },
                "seed": 1
            }"#,
        )
        .unwrap();
        let topology: Topology = serde_json::from_value(args["topology"].clone()).unwrap();
        assert_eq!(topology.nodes.len(), 2);
        assert_eq!(topology.edges.len(), 1);
        assert_eq!(topology.annotations.as_ref().map(Vec::len), Some(1));
        Engine::new(topology, 1);
    }

    /// `NodeConfig`'s fleet-sizing fields are non-optional by design (see
    /// README "Design compatibility edge case"): a topology missing one is
    /// rejected by `serde` rather than silently simulated with a zero.
    #[test]
    fn node_config_rejects_a_missing_required_field() {
        let mut args: Value = serde_json::from_str(
            r#"{ "topology": { "nodes": [
                { "id": "n", "kind": "service", "label": "Service", "x": 0, "y": 0,
                  "config": { "capacity": 1, "serviceMs": 1, "serviceCv": 0,
                              "queueLimit": 1, "hitRate": 0, "errorRate": 0,
                              "timeoutMs": 0, "retries": 0, "rps": 1 } }
            ], "edges": [] } }"#,
        )
        .unwrap();
        let err = serde_json::from_value::<Topology>(args["topology"].clone())
            .expect_err("a config missing replicaCount must be rejected");
        assert!(err.to_string().contains("replicaCount"), "got: {err}");

        // ...and the same payload with all six fleet fields restored parses,
        // so the rejection above is about those fields, not the shape
        // around them.
        let config = &mut args["topology"]["nodes"][0]["config"];
        config["replicaCount"] = Value::from(1);
        config["replicationLagMs"] = Value::from(50);
        config["readFraction"] = Value::from(0.8);
        config["shardCount"] = Value::from(4);
        config["shardCapacity"] = Value::from(4);
        config["hotKeyFraction"] = Value::from(0);
        serde_json::from_value::<Topology>(args["topology"].clone()).unwrap();
    }
}
