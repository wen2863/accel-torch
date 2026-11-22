//! The random-tick budget every running torch spends from, and the fair share each one takes.
//!
//! The budget belongs to the server tick, not to a torch: the first torch to tick in a tick
//! renews it, and every torch then spends from the same cap, so more torches never cost more
//! than the cap allows in one tick.

use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// What all the running torches of a server together may spend in one tick: a whole 55x55x55
/// area at 640000% would be millions of random ticks, far more than a tick can pay for.
pub(super) const MAX_CALLS_PER_TICK: i64 = 3_000;

/// How long the shared budget may take in one tick, so a slow tick drops calls instead of
/// stalling the server.
pub(super) const CALL_TIME_BUDGET: Duration = Duration::from_millis(3);

/// The most random ticks one block gets on a single pass, however fast its torch runs.
pub(super) const ROUNDS_PER_CELL: i64 = 64;

/// The server tick the budget belongs to, the random ticks it has spent, the instant it runs
/// out, and the clock those are measured on.
static BUDGET_TICK: AtomicI64 = AtomicI64::new(i64::MIN);
static BUDGET_CALLS: AtomicI64 = AtomicI64::new(0);
static BUDGET_DEADLINE: AtomicU64 = AtomicU64::new(0);
static CLOCK: OnceLock<Instant> = OnceLock::new();

/// The torches that ticked in the last server tick, so each takes an equal share however the
/// server orders them, and where each one next pass starts.
static TORCHES: Mutex<Vec<TorchState>> = Mutex::new(Vec::new());

struct TorchState {
    pos: [i32; 3],
    tick: i64,
    cursor: i64,
}

/// Renews the shared budget when a new server tick reaches its first torch.
pub(super) fn begin(tick: i64) {
    if BUDGET_TICK.swap(tick, Ordering::Relaxed) != tick {
        BUDGET_CALLS.store(0, Ordering::Relaxed);
        let deadline = now_nanos() + CALL_TIME_BUDGET.as_nanos() as u64;
        BUDGET_DEADLINE.store(deadline, Ordering::Relaxed);
    }
}

/// What no torch has spent yet of this tick.
pub(super) fn left() -> i64 {
    (MAX_CALLS_PER_TICK - BUDGET_CALLS.load(Ordering::Relaxed)).max(0)
}

/// Records random ticks handed to the level.
pub(super) fn spend(calls: i64) {
    BUDGET_CALLS.fetch_add(calls, Ordering::Relaxed);
}

/// Whether this tick has spent its time, so the rest of the pass waits for the next tick.
pub(super) fn out_of_time() -> bool {
    now_nanos() >= BUDGET_DEADLINE.load(Ordering::Relaxed)
}

/// The share of the tick budget this torch may spend, and where its pass starts.
///
/// Every torch that ticked in the previous tick takes an equal part of the budget, so a torch
/// can never spend what a later one would have used: with five running torches each spends a
/// fifth, in whatever order the server calls them. A pass carries on from the cell the last one
/// stopped at, so a torch whose share is smaller than its area still covers the whole area over
/// a few ticks instead of starting over at the same cell.
pub(super) fn claim(pos: [i32; 3], tick: i64, cells: i64) -> (i64, i64) {
    let Ok(mut torches) = TORCHES.lock() else {
        return (MAX_CALLS_PER_TICK, 0);
    };
    // A torch that has not ticked for two ticks is gone (broken, or its chunk unloaded).
    torches.retain(|torch| tick - torch.tick <= 1);
    let index = match torches.iter().position(|torch| torch.pos == pos) {
        Some(index) => index,
        None => {
            torches.push(TorchState {
                pos,
                tick,
                cursor: 0,
            });
            torches.len() - 1
        }
    };
    torches[index].tick = tick;
    let share = (MAX_CALLS_PER_TICK / torches.len().max(1) as i64).max(1);
    (share, torches[index].cursor % cells)
}

/// Remembers where this torch next pass starts.
pub(super) fn advance(pos: [i32; 3], cursor: i64) {
    if let Ok(mut torches) = TORCHES.lock()
        && let Some(torch) = torches.iter_mut().find(|torch| torch.pos == pos)
    {
        torch.cursor = cursor;
    }
}

/// Nanoseconds since the mod first measured time, for the shared per-tick budget.
pub(super) fn now_nanos() -> u64 {
    let start = CLOCK.get_or_init(Instant::now);
    Instant::now().duration_since(*start).as_nanos() as u64
}
