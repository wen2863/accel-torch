# 加速火把 (Accel Torch)

Three accelerating torches for [Horizon](https://github.com/horizon-rust/horizon) 26.3, a native Rust mod.
Inspired by [Torcherino](https://modrinth.com/mod/torcherino); original code and textures, no assets or code taken from it.

| Tier | Speed | Largest area |
| --- | --- | --- |
| 加速火把 `accel_torch` | 100-400% | 3x3x3 |
| 高级加速火把 `advanced_accel_torch` | 4000-16000% | 9x9x9 |
| 超级加速火把 `super_accel_torch` | 160000-640000% | 27x27x27 |

Right-click a torch for its panel (X/Y/Z size, speed, on/off). A percentage is extra random ticks per block per tick;
100% means one extra `randomTick` per block per tick, about 1365x vanilla's 3/4096 chance.

## Build

Requires the Horizon launcher and the SDK for build `0a875da91dbb0fdcb7ba8c42c11341aa229af51a`:

    HORIZON_SDK_BASE=/path/to/sdk cargo horizon-mod build

This writes `target/horizon-mod/accel_torch.hmod`. To play it, run
`horizon-launcher native-build . --instance <name> --trust-native-code`, or enable the built `.hmod` in the launcher.

## Layout

- `src/lib.rs` — the whole mod: block entity, config panel, payloads, particles.
- `assets/accel_torch/` — blockstates, models, textures (256x256 blocks, 16x16 items, 8x8 particles), lang.
- `data/accel_torch/` — block, item, recipe, loot table and creative tab declarations.
- `horizon-mod.toml` — manifest; pins game 26.3, horizon `0a875da`, mixins `e59da90`.

## Licence

MIT, see `LICENSE`. Torcherino is only an inspiration for the idea and the numbers; nothing is copied from it.
