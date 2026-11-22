//! The block entity every accel torch shares: its settings, its tick, and what clients keep of
//! it.

use mod_api::{
    BlockEntityEvent, BlockEntityPart, BlockEntityTickEvent, BlockEntityTickReply,
    BlockEntityTicksEvent, BlockEntityUpdateTagEvent, BlockPart, Compound, Host, LevelRef,
    LevelSide, ServerHost, Tag,
};

use crate::tier::{MAX_REACH, MAX_SPEED, Tier, tier_of, tier_of_name};
use crate::util::{flag, int, odd_side, span};
use crate::{budget, spark};

/// The behaviour every accel torch shares: it speeds up the blocks around it.
pub(super) struct Torch;

#[mod_api::horizon_part]
impl BlockPart for Torch {}

#[mod_api::horizon_part]
impl BlockEntityPart for Torch {
    /// New torches start switched off, at the smallest reach their tier has, like Torcherino.
    fn block_entity(
        &self,
        _level: LevelSide,
        event: &mut BlockEntityEvent<'_>,
    ) -> Option<Compound> {
        let tier = tier_of_name(event.block).unwrap_or(Tier::Normal);
        let mut data = Compound::new();
        data.insert("active".into(), Tag::Byte(0));
        data.insert("speed".into(), Tag::Int(tier.default_speed()));
        data.insert("sx".into(), Tag::Int(1));
        data.insert("sy".into(), Tag::Int(1));
        data.insert("sz".into(), Tag::Int(1));
        Some(data)
    }

    fn block_entity_ticks(&self, _host: &dyn Host, event: &mut BlockEntityTicksEvent<'_>) {
        event.ticks = true;
    }

    /// Each tick, offers every block in range its share of random ticks - 100% is one extra
    /// random tick per block per tick - and hands them to the level, within the budget all the
    /// running torches share. Then, now and then, shows a spark in the tier own colour.
    fn block_entity_tick(&self, host: &mut dyn ServerHost, event: &mut BlockEntityTickEvent<'_>) {
        *event.reply = BlockEntityTickReply::Unchanged;
        let pos = event.pos;
        let data = &event.entity.nbt;
        if !flag(data, "active") {
            return;
        }
        let speed = int(data, "speed", 100).clamp(0, MAX_SPEED);
        if speed <= 0 {
            return;
        }
        // Each number is how far the area reaches out from the torch on that axis, so 1 covers
        // 3x3x3 with the torch in the middle and 27 covers 55x55x55.
        let reach = [
            odd_side(int(data, "sx", 1), MAX_REACH),
            odd_side(int(data, "sy", 1), MAX_REACH),
            odd_side(int(data, "sz", 1), MAX_REACH),
        ];
        let Some(level) = host.level(host.dimension()) else {
            return;
        };
        accelerate(host, level, pos, reach, speed);
        if let Some(tier) = level
            .block(&*host, pos)
            .and_then(|block| tier_of(&*host, block))
        {
            spark::emit(host, level, pos, tier);
        }
    }

    /// What clients keep of the torch: its settings, so its screen opens on them.
    fn block_entity_update_tag(
        &self,
        event: &mut BlockEntityUpdateTagEvent<'_>,
    ) -> Option<Compound> {
        let mut tag = Compound::new();
        for key in ["active", "speed", "sx", "sy", "sz"] {
            if let Some(value) = event.entity.nbt.get(&key.into()) {
                tag.insert(key.into(), value.clone());
            }
        }
        Some(tag)
    }
}

/// Hands random ticks to the blocks in range of one torch, spending only this torch share of the
/// tick budget and carrying on where the last pass stopped, so a capped torch still covers its
/// whole area over a few ticks.
fn accelerate(
    host: &mut dyn ServerHost,
    level: LevelRef,
    pos: [i32; 3],
    reach: [i32; 3],
    speed: i32,
) {
    let [rx, ry, rz] = reach;
    let (nx, ny, nz) = (span(rx), span(ry), span(rz));
    // Every cell of the area but the one the torch itself stands in.
    let cells = nx * ny * nz - 1;
    if cells <= 0 {
        return;
    }
    let tick = level
        .game_time(&*host)
        .unwrap_or_else(|| (budget::now_nanos() / 50_000_000) as i64);
    budget::begin(tick);
    // This torch own slice of the tick, so the next torch always has something left.
    let (share, start) = budget::claim(pos, tick, cells);
    let remaining = share.min(budget::left());
    if remaining <= 0 {
        return;
    }
    // How many random ticks one block gets on a pass: its full share, trimmed to what is left of
    // the tick. What a pass does not reach is picked up by the next one.
    let rounds = (speed / 100).max(1) as i64;
    let per_cell = rounds
        .min(budget::ROUNDS_PER_CELL)
        .min((remaining / cells).max(1))
        .min(remaining);
    let visits = (remaining / per_cell).min(cells);
    let [x, y, z] = pos;
    let center = (i64::from(rz) * ny + i64::from(ry)) * nx + i64::from(rx);
    let mut visited = 0i64;
    while visited < visits {
        // Checking the clock costs more than a call to a block that does nothing, so only every
        // 32nd cell looks at it.
        if visited & 31 == 0 && budget::out_of_time() {
            break;
        }
        let index = (start + visited) % cells;
        // Step over the cell the torch itself stands in.
        let index = if index >= center { index + 1 } else { index };
        let at = [
            x + (index % nx) as i32 - rx,
            y + (index / nx % ny) as i32 - ry,
            z + (index / (nx * ny)) as i32 - rz,
        ];
        for _ in 0..per_cell {
            level.block_random_tick(host, at);
        }
        budget::spend(per_cell);
        visited += 1;
    }
    budget::advance(pos, start + visited);
}
