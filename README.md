# 加速火把 (Accel Torch)

Three accelerating torches for [Horizon](https://github.com/horizon-rust/horizon) 26.3, a native Rust mod.
Inspired by [Torcherino](https://modrinth.com/mod/torcherino); original code and textures, no assets or code taken from it.

| Tier | Speed | Reach per axis | Largest area |
| --- | --- | --- | --- |
| 加速火把 `accel_torch` | 100-400% | 1 or 3 | up to 7x7x7 |
| 高级加速火把 `advanced_accel_torch` | 4000-16000% | 1 to 9 | up to 19x19x19 |
| 超级加速火把 `super_accel_torch` | 160000-640000% | 1 to 27 | up to 55x55x55 |

Right-click a torch for its panel (X/Y/Z reach, speed, on/off). Each number is how far the area reaches out from the
torch on that axis, so 1 covers 3x3x3 with the torch in the middle and 3 covers 7x7x7. A percentage is extra random
ticks per block per tick; 100% means one extra `randomTick` per block per tick, about 1365x vanilla's 3/4096 chance.

The panel takes the plain right click, the way a crafting table does, so `sneak + right-click` is what places a block
against a torch. The server side cancels the click for a standing player and lets it through while the shift key is
down, which keeps both halves in agreement.

## Server cost and access

The random-tick budget is shared by every torch on the server: at most 3000 calls and 3 ms of work per tick,
reset every server tick, so stacking torches cannot drag the server below 20 TPS. A torch whose area needs more
than the budget sweeps its box over several ticks with a per-torch cursor, so the whole area is still covered.

The panel asks the server for the torch's real settings when it opens. The server only accepts a change from a
player in the same level and within 6 blocks of that torch; other packets are ignored and answered with a chat
note (a failed payload handler would disconnect the sender).

## Build

Requires the Horizon launcher and the SDK for build `0a875da91dbb0fdcb7ba8c42c11341aa229af51a`:

    HORIZON_SDK_BASE=/path/to/sdk cargo horizon-mod build

This writes `target/horizon-mod/accel_torch.hmod`. To play it, run
`horizon-launcher native-build . --instance <name> --trust-native-code`, or enable the built `.hmod` in the launcher.

## Layout

- `src/lib.rs` — the hooks: registration, the right-click gate, and the client tick and screen entry points.
- `src/tier.rs` — the three torches, their speed steps and reach limits.
- `src/settings.rs` — what a torch stores, the payload the server checks, and the 6-block range check.
- `src/screen.rs` — the settings panel: its widgets, its frame, its input.
- `src/torch.rs` — the block entity: its tick, and the random ticks it hands to blocks in range.
- `src/budget.rs` — the random-tick budget every torch shares, and each torch's fair share of it.
- `src/spark.rs` — the spark a running torch shows, and the small random source behind it.
- `src/util.rs` — reading a saved compound, and the odd reach numbers.
- `assets/accel_torch/` — blockstates, models, textures (256x256 blocks, 16x16 items, 8x8 particles), lang.
- `data/accel_torch/` — block, item, recipe, loot table and creative tab declarations.
- `horizon-mod.toml` — manifest; pins game 26.3, horizon `0a875da`, mixins `e59da90`.

## Credits

Written by **Wen** (concept, block and item textures, in-game testing) together with **DeepSeek**, an AI coding
assistant that wrote most of `src/` (block entity, tick scheduling, config screen, payloads) and set up the
packaging. Commits that contain AI-written code carry a `Co-authored-by: DeepSeek <noreply@deepseek.com>` trailer.
Particle sprites started as programmatically generated placeholders and can be replaced freely.

## Licence

MIT, see `LICENSE`. Torcherino is only an inspiration for the idea and the numbers; nothing is copied from it.