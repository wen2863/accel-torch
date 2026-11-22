//! Accel Torches: three Torcherino-style torches for Horizon.
//!
//! Blocks, items and recipes live in data/ and assets/. This crate holds the behaviour, split so
//! the whole mod can move into another one without renaming anything:
//!
//! - tier: the three torches, and what each of them may hold.
//! - settings: what a torch saves, and the screen payload the server checks and stores.
//! - screen: the settings screen - its widgets, its frame, its input.
//! - torch: the block entity every torch shares - its tick, and the random ticks it hands out.
//! - budget: the random-tick budget all running torches share, and the fair share of each.
//! - spark: the spark a running torch shows, and the small random source behind it.
//! - util: reading a saved compound, and the odd numbers a reach is kept in.
//!
//! A torch saves its settings in its own block entity compound: active (0|1), speed (a
//! percentage) and how far its area reaches on each axis, sx/sy/sz, odd numbers up to what its
//! tier may cover - 3 for an accel torch, 9 for an advanced one, 27 for a super one, where 1
//! reaches one block out and covers 3x3x3. Right-clicking opens the screen; its buttons send the
//! changed value to the server, which checks it against the sender and the torch, then stores it.

mod budget;
mod screen;
mod settings;
mod spark;
mod tier;
mod torch;
mod util;

use mod_api::{
    ClientHost, ClientTickPreEvent, Flow, Hand, Hooks, HostSide, InteractionResult, Registrar,
    RegistrationContext, RightClickBlockEvent, ScreenCharTypedPreEvent, ScreenKeyPressedPreEvent,
    ScreenMouseButtonPressedPreEvent, ScreenMouseButtonReleasedPreEvent, ScreenRenderPostEvent,
    TriState,
};

use crate::tier::tier_of;
use crate::torch::Torch;

#[derive(Default)]
struct Mod;

#[mod_api::horizon_mod]
impl Hooks for Mod {
    fn register(&self, r: &mut dyn Registrar, _cx: &RegistrationContext) {
        let torch = r
            .block_entity_type("accel_torch")
            .blocks(tier::BLOCKS)
            .build();
        r.block_entity_part(&torch, Torch)
            .expect("native block entity type");
        spark::register(r);
        settings::register(r);
    }

    /// A plain right click on a torch opens the settings screen; sneaking lets the click through,
    /// so a block in hand can still be placed against the torch.
    fn right_click_block(&self, host: HostSide, event: &mut RightClickBlockEvent<'_>) -> Flow {
        // Vanilla offers the click to both hands; only the main hand counts.
        if event.click.hand != Hand::Main {
            return Flow::Continue;
        }
        let pos = event.click.pos;
        match host {
            HostSide::Client(host) => {
                if host.modifiers().shift() {
                    return Flow::Continue;
                }
                let dim = host.dimension().to_owned();
                let Some(level) = host.level(&dim) else {
                    return Flow::Continue;
                };
                let Some(block) = level.block(&*host, pos) else {
                    return Flow::Continue;
                };
                let Some(tier) = tier_of(&*host, block) else {
                    return Flow::Continue;
                };
                screen::open(host, dim, pos, tier);
            }
            HostSide::Server(host) => {
                // The server cannot see the mouse modifiers, but it does see the shift key
                // state. A standing player gets the screen, and must not place a block against
                // the torch; a sneaking one keeps vanilla placement, so the click passes.
                let dim = host.dimension().to_owned();
                let Some(level) = host.level(&dim) else {
                    return Flow::Continue;
                };
                let Some(block) = level.block(&*host, pos) else {
                    return Flow::Continue;
                };
                if tier_of(&*host, block).is_none() {
                    return Flow::Continue;
                }
                let Some(player) = host.players().acting_player() else {
                    return Flow::Continue;
                };
                if player.entity().is_shift_key_down(&*host) == Some(true) {
                    return Flow::Continue;
                }
            }
        }
        *event.use_block = TriState::False;
        *event.use_item = TriState::False;
        *event.cancellation_result = InteractionResult::Success;
        Flow::Cancel
    }

    /// The input hooks can only read the host, so a change waits in settings for the client tick.
    fn client_tick_pre(&self, host: &mut dyn ClientHost, _: &mut ClientTickPreEvent) {
        screen::tick(host);
        settings::send_pending(host);
    }

    fn screen_render_post(&self, host: &dyn ClientHost, event: &mut ScreenRenderPostEvent<'_>) {
        screen::render(host, event);
    }

    fn screen_mouse_button_pressed_pre(
        &self,
        host: &dyn ClientHost,
        event: &mut ScreenMouseButtonPressedPreEvent<'_>,
    ) -> Flow {
        let (at, button, modifiers) = (event.screen.mouse, event.button, event.modifiers);
        screen::handle(host, |widgets| {
            widgets.mouse_pressed(host, at, button, modifiers)
        })
    }

    fn screen_mouse_button_released_pre(
        &self,
        host: &dyn ClientHost,
        event: &mut ScreenMouseButtonReleasedPreEvent<'_>,
    ) -> Flow {
        let button = event.button;
        screen::handle(host, |widgets| widgets.mouse_released(button))
    }

    fn screen_key_pressed_pre(
        &self,
        host: &dyn ClientHost,
        event: &mut ScreenKeyPressedPreEvent<'_>,
    ) -> Flow {
        let (key, modifiers) = (event.key, event.modifiers);
        screen::handle(host, |widgets| widgets.key_pressed(host, key, modifiers))
    }

    fn screen_char_typed_pre(
        &self,
        host: &dyn ClientHost,
        event: &mut ScreenCharTypedPreEvent<'_>,
    ) -> Flow {
        let c = event.c;
        screen::handle(host, |widgets| widgets.char_typed(host, c))
    }
}
