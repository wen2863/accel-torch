//! The spark a running torch shows over its flame, and the small random source that decides when.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use mod_api::{LevelRef, ParticleKey, ParticleOptions, Registrar, ServerHost};

use crate::tier::Tier;

/// How often a running torch shows a spark, per tick.
const SPARK_CHANCE: f64 = 0.35;

/// One sprite set per tier, so a running torch shows which one it is.
static SPARKS: OnceLock<[ParticleKey; 3]> = OnceLock::new();

/// A small SplitMix64, so the mod needs no crate of its own for the rolls.
static SEED: AtomicU64 = AtomicU64::new(0);

/// Takes the three spark sprite sets, once the registries are open.
pub(super) fn register(r: &mut dyn Registrar) {
    let sparks = [
        r.particle_key("accel_torch:accel"),
        r.particle_key("accel_torch:advanced"),
        r.particle_key("accel_torch:super"),
    ];
    let _ = SPARKS.set(sparks);
}

/// Now and then a spark over the flame, in the tier own colour.
pub(super) fn emit(host: &mut dyn ServerHost, level: LevelRef, pos: [i32; 3], tier: Tier) {
    if !roll(SPARK_CHANCE) {
        return;
    }
    let Some(sparks) = SPARKS.get() else {
        return;
    };
    let options = ParticleOptions::simple(sparks[tier.spark_index()]);
    let [x, y, z] = pos;
    level.send_particles(
        host,
        &options,
        None,
        [f64::from(x) + 0.5, f64::from(y) + 0.66, f64::from(z) + 0.5],
        [0.12, 0.03, 0.12],
        0.0,
        1,
    );
}

/// Whether a roll of the shared generator lands under `chance`.
pub(super) fn roll(chance: f64) -> bool {
    let unit = (next_u64() >> 11) as f64 / 9_007_199_254_740_992.0;
    unit < chance
}

fn next_u64() -> u64 {
    let mut current = SEED.load(Ordering::Relaxed);
    if current == 0 {
        current = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0x9e37_79b9_7f4a_7c15, |elapsed| elapsed.as_nanos() as u64)
            | 1;
    }
    let next = current.wrapping_add(0x9e37_79b9_7f4a_7c15);
    SEED.store(next, Ordering::Relaxed);
    let mut z = next;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}
