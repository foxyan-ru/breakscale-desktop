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
        *self.engine.lock().unwrap_or_else(|e| e.into_inner()) = Some(engine);
        if !self.thread_started.swap(true, Ordering::SeqCst) {
            self.spawn_tick_thread(app.clone());
        }
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
