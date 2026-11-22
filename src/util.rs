//! Reading what a torch saved in its block entity, and the odd numbers its reach is kept in.

use mod_api::{Compound, Tag};

/// Whether the byte tag at `key` is set.
pub(super) fn flag(data: &Compound, key: &str) -> bool {
    matches!(data.get(&key.into()), Some(Tag::Byte(value)) if *value != 0)
}

/// The integer at `key`, or `default` when it is missing or not a number.
pub(super) fn int(data: &Compound, key: &str, default: i32) -> i32 {
    match data.get(&key.into()) {
        Some(Tag::Int(value)) => *value,
        Some(Tag::Short(value)) => i32::from(*value),
        Some(Tag::Byte(value)) => i32::from(*value),
        _ => default,
    }
}

/// Clamps the reach on one axis to 1..=max and rounds it up to an odd number, so the torch always
/// stands in the middle of a symmetric area.
pub(super) fn odd_side(value: i32, max: i32) -> i32 {
    let value = value.clamp(1, max);
    if value % 2 == 0 {
        (value + 1).min(max)
    } else {
        value
    }
}

/// How many blocks one axis spans when the area reaches `reach` blocks out on both sides: the
/// block the torch stands in plus twice the reach, so a reach of 1 covers 3 and of 27 covers 55.
pub(super) fn span(reach: i32) -> i64 {
    i64::from(reach) * 2 + 1
}
