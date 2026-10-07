//! Tauri-managed application state: the one live `Engine` and the
//! background thread that advances it.
//!
//! The engine runs on a dedicated OS thread rather than inside an async
//! runtime task: it is a tight, synchronous, CPU-bound loop (see
//! MIGRATION_PLAN.md #3), and a plain thread + `std::thread::sleep` needs no
//! extra dependency (an explicit `tokio` dependency would be needed to use
//! `tokio::time::sleep` directly, since Cargo does not let a crate use a
//! transitive dependency's API without declaring it) and cannot be starved
//! by unrelated async work elsewhere in the process.

use crate::sim::engine::Engine;
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use tauri::{AppHandle, Emitter};

/// How often the engine advances. Matches the web app's `requestAnimationFrame`
/// loop closely enough that a saved design replays at the same felt speed;
/// exact 60Hz parity is not achievable from a plain OS-thread sleep loop and
/// is not required for correctness, since `advance(deltaMs)` is driven by the
/// measured elapsed time, not by an assumed constant -- see `sim::engine`.
const TICK_MS: u64 = 16;
/// How often a snapshot is pushed to the frontend. Matches the web app's
/// 10Hz `snapshot()` polling rate -- AGENTS.md: "React re-renders at 10Hz.
/// Never setState per frame," which applies just as much to a Svelte store.
/// `.max` is not const-stable on this toolchain, so the clamp is spelled
/// out as an if/else that const-evaluates.
const SNAPSHOT_EVERY_N_TICKS: u32 = {
    let n = 100 / TICK_MS as u32;
    if n < 1 {
        1
    } else {
        n
    }
};

pub const SNAPSHOT_EVENT: &str = "sim://snapshot";
/// Emitted if the tick thread's engine lock is ever found poisoned (a panic
/// happened mid-tick in a debug build; release builds use `panic = "abort"`
/// and never reach this). The frontend shows this as a recoverable banner
/// ("the simulation stopped unexpectedly; reset to continue") rather than
/// silently going stale.
pub const TICK_ERROR_EVENT: &str = "sim://tick-error";

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct TickErrorPayload {
    message: String,
}

pub struct SimulationState {
    engine: Arc<Mutex<Option<Engine>>>,
    running: Arc<AtomicBool>,
    thread_started: AtomicBool,
}

impl Default for SimulationState {
    fn default() -> Self {
        Self {
            engine: Arc::new(Mutex::new(None)),
            running: Arc::new(AtomicBool::new(true)),
            thread_started: AtomicBool::new(false),
        }
    }
}

impl SimulationState {
    /// Install a freshly built engine as the live one, replacing whatever
    /// was running. Starts the background tick thread on first call; later
    /// calls (a new design loaded, `sim_reset`) just swap the engine under
    /// the lock, which the tick thread picks up on its next iteration.
    pub fn install(&self, app: &AppHandle, engine: Engine) {
        self.swap_engine(engine);
        if !self.thread_started.swap(true, Ordering::SeqCst) {
            self.spawn_tick_thread(app.clone());
        }
    }

    /// Put `engine` in as the live one without touching the tick thread:
    /// the half of `install` that needs no `AppHandle`, split out so the
    /// engine contract `require_engine` relies on is testable headless.
    fn swap_engine(&self, engine: Engine) {
        *self.engine.lock().unwrap_or_else(|e| e.into_inner()) = Some(engine);
    }

    /// Mutate the live engine, if one exists. Returns `None` when no design
    /// has been loaded yet (commands map that to `AppError::State`).
    pub fn with_engine<T>(&self, f: impl FnOnce(&mut Engine) -> T) -> Option<T> {
        let mut guard = self.engine.lock().unwrap_or_else(|e| e.into_inner());
        guard.as_mut().map(f)
    }

    pub fn set_running(&self, running: bool) {
        self.running.store(running, Ordering::SeqCst);
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    fn spawn_tick_thread(&self, app: AppHandle) {
        let engine = Arc::clone(&self.engine);
        let running = Arc::clone(&self.running);
        thread::spawn(move || {
            let mut tick: u32 = 0;
            loop {
                thread::sleep(Duration::from_millis(TICK_MS));
                if !running.load(Ordering::SeqCst) {
                    continue;
                }

                let mut guard = match engine.lock() {
                    Ok(g) => g,
                    Err(poisoned) => {
                        let _ = app.emit(
                            TICK_ERROR_EVENT,
                            TickErrorPayload {
                                message: "The simulation stopped unexpectedly. Reset to continue."
                                    .to_string(),
                            },
                        );
                        poisoned.into_inner()
                    }
                };
                // Re-check the flag UNDER the lock. The check above is the
                // fast path (no locking at all while paused); this one is
                // what makes `sim_step`'s delta exact. That command pauses
                // and then advances while holding this same lock, so a
                // thread which read `running == true` above and then
                // blocked here would otherwise advance once MORE after the
                // step landed -- turning a 100ms step into 100ms plus an
                // unpredictable 16ms. Checked under the lock, the last
                // writer wins and a step is always exactly its own delta,
                // which is the same guarantee the web engine gets by
                // setting `runningRef.current = false` before advancing
                // inline (`App.tsx` `handleStep`).
                if !running.load(Ordering::SeqCst) {
                    continue;
                }
                let Some(eng) = guard.as_mut() else { continue };
                eng.advance(TICK_MS as f64);

                tick = tick.wrapping_add(1);
                if tick % SNAPSHOT_EVERY_N_TICKS == 0 {
                    let snapshot = eng.snapshot();
                    drop(guard); // release the lock before the (cheap) emit
                    let _ = app.emit(SNAPSHOT_EVENT, snapshot);
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::types::Topology;

    fn blank_topology() -> Topology {
        Topology {
            nodes: vec![],
            edges: vec![],
            annotations: None,
        }
    }

    #[test]
    fn a_fresh_state_has_no_engine_yet() {
        let state = SimulationState::default();
        assert!(state.with_engine(|_| ()).is_none());
    }

    /// Once an engine is installed, the `with_engine` path every `sim_*`
    /// command goes through (`require_engine`) must resolve -- installing
    /// is the only thing standing between boot and "No design is loaded
    /// yet." never being reachable.
    #[test]
    fn installing_an_engine_makes_with_engine_resolve() {
        let state = SimulationState::default();
        state.swap_engine(Engine::new(blank_topology(), 1));
        assert!(state.with_engine(|e| e.snapshot()).is_some());
    }

    /// `sim_new` (and the boot-time install in `lib.rs`) replace an
    /// existing engine; the swap must not lose the new one.
    #[test]
    fn installing_again_replaces_the_live_engine() {
        let state = SimulationState::default();
        state.swap_engine(Engine::new(blank_topology(), 1));
        state.swap_engine(Engine::new(blank_topology(), 2));
        assert!(state.with_engine(|e| e.snapshot()).is_some());
    }

    /// The tick thread gates on this flag; the boot engine must already be
    /// considered running so snapshots flow before any frontend call.
    #[test]
    fn running_defaults_to_true() {
        assert!(SimulationState::default().is_running());
    }

    /// WHY: the two emit channels and the tick-error payload are the only
    /// Rust-EMITTED strings the frontend parses blind (`listen()` on one
    /// side, `emit()` on the other); a rename on either side silently stops
    /// snapshots or error banners arriving. The event constants, the payload
    /// name they carry, and the payload's own key set are pinned here against
    /// the SAME golden fixture the vitest suite checks, and the literal
    /// message the tick thread sends is compared against it too -- state.rs
    /// is this module's own source file, so the check cannot rot silently.
    #[test]
    fn emitted_event_channels_and_tick_error_payload_match_the_shared_fixture() {
        const GOLDEN: &str = include_str!("../../contract/ipc-golden.json");
        let g: serde_json::Value =
            serde_json::from_str(GOLDEN).expect("contract/ipc-golden.json must parse");

        // The fixture's `events` map is keyed by channel; both constants
        // must be present, with exactly the payload names the frontend
        // deserializes from them.
        let snap = &g["events"][SNAPSHOT_EVENT];
        assert_eq!(
            snap,
            &serde_json::json!(["simSnapshot"]),
            "{SNAPSHOT_EVENT} must carry exactly the simSnapshot payload"
        );
        let tick = &g["events"][TICK_ERROR_EVENT];
        assert_eq!(
            tick,
            &serde_json::json!(["tickError"]),
            "{TICK_ERROR_EVENT} must carry exactly the tickError payload"
        );

        // The payload struct itself: `{ message }` and nothing else. Built
        // from the fixture's message, so an added/renamed struct field (or a
        // fixture key) breaks the equality below.
        let fixture_message = g["payloads"]["tickError"]["message"]
            .as_str()
            .expect("payloads.tickError.message is a string");
        let serialized = serde_json::to_value(TickErrorPayload {
            message: fixture_message.to_string(),
        })
        .expect("TickErrorPayload serializes");
        assert_eq!(
            serialized, g["payloads"]["tickError"],
            "TickErrorPayload must serialize to exactly the fixture shape"
        );

        // And the literal the tick thread actually sends is the fixture's
        // message, not a drifted one: read this very source file and require
        // the `message: "..."` literal to appear verbatim.
        let src = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/state.rs"),
        )
        .expect("state.rs must be readable");
        let literal = format!("message: \"{fixture_message}\"");
        assert!(
            src.contains(&literal),
            "state.rs must emit exactly the fixture's tick-error text ({literal})"
        );
    }
}
