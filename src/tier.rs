//! The three torches, and what each may hold.

use mod_api::{BlockKey, Host};

/// Our three blocks, all sharing one block entity type.
pub(super) const BLOCKS: &[&str] = &["accel_torch", "advanced_accel_torch", "super_accel_torch"];

/// The fastest speed any torch may hold, and the furthest any of them may reach on one axis.
/// A speed is a percentage: 100% hands every block in range one extra random tick per tick.
pub(super) const MAX_SPEED: i32 = 640_000;
pub(super) const MAX_REACH: i32 = 27;

/// The three torches: they differ in how fast they may go and how far they reach.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Tier {
    Normal,
    Advanced,
    Super,
}

impl Tier {
    /// The name the screen shows.
    pub(super) fn title(self) -> &'static str {
        match self {
            Tier::Normal => "加速火把",
            Tier::Advanced => "高级加速火把",
            Tier::Super => "超级加速火把",
        }
    }

    /// Which of the three spark sprite sets this torch shows.
    pub(super) fn spark_index(self) -> usize {
        match self {
            Tier::Normal => 0,
            Tier::Advanced => 1,
            Tier::Super => 2,
        }
    }

    /// The four speeds this tier offers, as percentages.
    pub(super) fn speeds(self) -> &'static [i32] {
        match self {
            Tier::Normal => &[100, 200, 300, 400],
            Tier::Advanced => &[4_000, 8_000, 12_000, 16_000],
            Tier::Super => &[160_000, 320_000, 480_000, 640_000],
        }
    }

    /// The speed a new torch of this tier starts at: the slowest one it offers.
    pub(super) fn default_speed(self) -> i32 {
        self.speeds()[0]
    }

    /// The furthest this tier may reach on one axis, so the area it covers is 7x7x7, 19x19x19
    /// or 55x55x55 blocks.
    pub(super) fn max_reach(self) -> i32 {
        match self {
            Tier::Normal => 3,
            Tier::Advanced => 9,
            Tier::Super => 27,
        }
    }

    /// The fastest speed this tier allows that is not above `speed`, or its slowest speed when
    /// `speed` is under that.
    pub(super) fn clamp(self, speed: i32) -> i32 {
        let mut best = self.speeds()[0];
        for allowed in self.speeds() {
            if speed >= *allowed {
                best = *allowed;
            }
        }
        best
    }
}

/// The tier of a block, when it is one of ours.
pub(super) fn tier_of(host: &dyn Host, block: BlockKey) -> Option<Tier> {
    tier_of_name(&block.id(host))
}

/// The tier a block id names, from the block a hook saw or the one a block entity saved.
pub(super) fn tier_of_name(name: &str) -> Option<Tier> {
    match name.rsplit(':').next()? {
        "accel_torch" => Some(Tier::Normal),
        "advanced_accel_torch" => Some(Tier::Advanced),
        "super_accel_torch" => Some(Tier::Super),
        _ => None,
    }
}
