//! Accel Torches: three Torcherino-style torches for Horizon.
//!
//! Blocks, items and recipes live in data/ and assets/; this crate holds the behaviour:
//! the random-tick acceleration (see block_entity_tick), the settings screen, and the
//! per-torch settings a block entity saves.
//!
//! A torch saves its settings in its own block entity compound: active (0|1), speed (a
//! percentage) and the size of its area sx/sy/sz in blocks, odd numbers up to what its tier
//! may cover - 3 for an accel torch, 9 for an advanced one, 27 for a super one. Right-clicking
//! opens the screen; the buttons send the changed value to the server, which checks it against
//! the torch's tier and stores it.
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use mod_api::client::widgets::{Button, Outcome, WidgetId, Widgets};
use mod_api::{
    self, BlockEntityEvent, BlockEntityPart, BlockEntityTickEvent, BlockEntityTickReply,
    BlockEntityTicksEvent, BlockEntityUpdateTagEvent, BlockPart, ClientHost, ClientTickPreEvent,
    Compound, Flow, Hand, Hooks, Host, HostSide, IPayloadContext, InteractionResult, LevelSide,
    ParticleKey, ParticleOptions, PayloadCodec, PayloadId, Registrar, RegistrationContext,
    RightClickBlockEvent, ScreenCharTypedPreEvent, ScreenKeyPressedPreEvent,
    ScreenMouseButtonPressedPreEvent, ScreenMouseButtonReleasedPreEvent, ScreenRenderPostEvent,
    ServerHost, Tag, Text, ToClient, ToServer, TriState,
};

/// Our three blocks, all sharing one block entity type.
const BLOCKS: &[&str] = &["accel_torch", "advanced_accel_torch", "super_accel_torch"];

/// The id of our own screen, and of the payload that carries its changes.
const SCREEN_ID: &str = "accel_torch:config";

/// The id of the payload the server answers with the torch's real values.
const STATE_ID: &str = "accel_torch:state";

/// The fastest speed any torch may hold, and the widest area any of them may cover in blocks
/// (odd, so the torch stands in the middle). A speed is a percentage: 100% hands every block in
/// range one extra random tick per tick (see `block_entity_tick`).
const MAX_SPEED: i32 = 640_000;
const MAX_SIDE: i32 = 27;

/// What all the running torches of a server together may spend in one tick: a whole 27x27x27
/// area at 640000% would be millions of random ticks, far more than a tick can pay for, so the
/// torches take turns at these limits and every pass starts where the previous one stopped.
const MAX_CALLS_PER_TICK: i64 = 3_000;
const CALL_TIME_BUDGET: Duration = Duration::from_millis(3);
const ROUNDS_PER_CELL: i64 = 64;

/// How far the player using a screen may stand from its torch: a setting is accepted only from
/// the torch's own level and within this reach, as for any block they interact with.
const MAX_SETTING_DISTANCE: f64 = 6.0;

/// What one torch's screen sends when a button changes something.
#[derive(Clone, Debug, PartialEq, Eq, Default, PayloadCodec)]
struct Config {
    /// 0 = store these values, 1 = ask the server for the torch's real ones.
    kind: i32,
    x: i32,
    y: i32,
    z: i32,
    active: i32,
    speed: i32,
    /// How many blocks wide the area is along X, Y and Z. Odd, because the torch is the middle
    /// block of its area.
    sx: i32,
    sy: i32,
    sz: i32,
    dim: String,
}

/// What the open screen edits: the torch it belongs to, its tier, and the values the server
/// confirmed. The values stay unknown until its answer arrives, so the screen never edits
/// made-up numbers.
struct Open {
    dim: String,
    pos: [i32; 3],
    config: Option<Config>,
    tier: Tier,
}

/// The client's token for Config, from registration.
static SEND: OnceLock<ToServer<Config>> = OnceLock::new();

/// The server's token for the values it answers with.
static REPLY: OnceLock<ToClient<Config>> = OnceLock::new();

/// The open screen's torch and its confirmed values.
static OPEN: Mutex<Option<Open>> = Mutex::new(None);

/// Values the server sent, picked up by the next client tick.
static INCOMING: Mutex<Option<Config>> = Mutex::new(None);

/// One sprite set per tier, so a running torch shows which one it is.
static SPARKS: OnceLock<[ParticleKey; 3]> = OnceLock::new();

/// The open screen's widgets, made on its first frame.
static SCREEN: Mutex<Option<Screen>> = Mutex::new(None);

/// A change the next client tick sends (the input hooks only read the host).
static PENDING: Mutex<Option<Config>> = Mutex::new(None);

/// The server tick the shared budget belongs to, the random ticks it has spent, the instant it
/// runs out, and the clock those are measured on.
static BUDGET_TICK: AtomicI64 = AtomicI64::new(i64::MIN);
static BUDGET_CALLS: AtomicI64 = AtomicI64::new(0);
static BUDGET_DEADLINE: AtomicU64 = AtomicU64::new(0);
static CLOCK: OnceLock<Instant> = OnceLock::new();

/// The torches that ticked in the last server tick, so each one takes an equal share of the
/// budget however the server orders them, and where each one's next pass starts.
static TORCHES: Mutex<Vec<TorchState>> = Mutex::new(Vec::new());

struct TorchState {
    pos: [i32; 3],
    tick: i64,
    cursor: i64,
}

/// Done was pressed: the next client tick closes the screen.
static CLOSE: AtomicBool = AtomicBool::new(false);

#[derive(Default)]
struct Mod;

#[mod_api::horizon_mod]
impl Hooks for Mod {
    fn register(&self, r: &mut dyn Registrar, _cx: &RegistrationContext) {
        let torch = r.block_entity_type("accel_torch").blocks(BLOCKS).build();
        r.block_entity_part(&torch, Torch)
            .expect("native block entity type");
        let sparks = [
            r.particle_key("accel_torch:accel"),
            r.particle_key("accel_torch:advanced"),
            r.particle_key("accel_torch:super"),
        ];
        let _ = SPARKS.set(sparks);

        let mut payloads = r.payload_handlers();
        let mut view = payloads.registrar("1");
        let send = view
            .play_to_server_typed::<Config>(PayloadId(SCREEN_ID.to_owned()), |config, context| {
                apply(config, context)
            });
        let reply = view.play_to_client_typed::<Config>(
            PayloadId(STATE_ID.to_owned()),
            |config, _context| {
                *INCOMING.lock().unwrap() = Some(config);
                Ok(())
            },
        );
        let _ = SEND.set(send);
        let _ = REPLY.set(reply);
    }

    /// A plain right click on a torch opens the settings screen; sneaking lets the click through
    /// so a block in hand can still be placed against the torch.
    fn right_click_block(&self, host: HostSide, event: &mut RightClickBlockEvent<'_>) -> Flow {
        // Vanilla offers the click to both hands; only the main hand counts.
        if event.click.hand != Hand::Main {
            return Flow::Continue;
        }
        let pos = event.click.pos;
        match host {
            HostSide::Client(host) => {
                // Sneaking keeps vanilla's placement: the screen would otherwise eat every click.
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
                *OPEN.lock().unwrap() = Some(Open {
                    dim: dim.clone(),
                    pos,
                    config: None,
                    tier,
                });
                *SCREEN.lock().unwrap() = None;
                host.mod_screen_open(SCREEN_ID);
                // The client's copy of the torch may be out of date; ask for the server's.
                let query = Config {
                    kind: 1,
                    x: pos[0],
                    y: pos[1],
                    z: pos[2],
                    dim,
                    ..Config::default()
                };
                if let Some(send) = SEND.get() {
                    let _ = send.send(host, &query);
                }
            }
            HostSide::Server(host) => {
                // The server cannot see the mouse modifiers, but it does see the shift key's
                // state. A standing player gets the screen (and must not place a block against
                // the torch); a sneaking one keeps vanilla placement, so the click passes.
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

    /// The input hooks can only read the host, so a change waits here for the client tick.
    fn client_tick_pre(&self, host: &mut dyn ClientHost, _: &mut ClientTickPreEvent) {
        if CLOSE.swap(false, Ordering::Relaxed) {
            host.mod_screen_close(SCREEN_ID);
            *OPEN.lock().unwrap() = None;
            *SCREEN.lock().unwrap() = None;
        }
        if let Some(config) = INCOMING.lock().unwrap().take() {
            let mut slot = SCREEN.lock().unwrap();
            let mut open = OPEN.lock().unwrap();
            if let Some(open) = open.as_mut() {
                if open.dim == config.dim && open.pos == [config.x, config.y, config.z] {
                    open.config = Some(config);
                    // The screen is built again from the confirmed values.
                    *slot = None;
                }
            }
        }
        let Some(config) = PENDING.lock().unwrap().take() else {
            return;
        };
        if let Some(send) = SEND.get() {
            let _ = send.send(host, &config);
        }
    }

    fn screen_render_post(&self, host: &dyn ClientHost, event: &mut ScreenRenderPostEvent<'_>) {
        if host.mod_screen() != Some(SCREEN_ID) {
            return;
        }
        let width = event.draw.size()[0];
        let mut slot = SCREEN.lock().unwrap();
        if slot.is_none() {
            let open = OPEN.lock().unwrap();
            let Some(open) = open.as_ref() else {
                return;
            };
            let Some(config) = open.config.clone() else {
                // The title shows while the server's answer is on its way.
                let x = (width / 2.0 - 104.0).floor();
                event.draw.fill([x, 20.0, 208.0, 40.0], 0xC8_10_10_10);
                event
                    .draw
                    .text(open.tier.title(), [x + 10.0, 26.0], 0xFFFF_FFFF, false);
                event
                    .draw
                    .text("正在读取设置", [x + 10.0, 40.0], 0xFFC0_C0C0, false);
                return;
            };
            let tier = open.tier;
            *slot = Some(Screen::new(width, config, tier));
        }
        let Some(screen) = slot.as_mut() else {
            return;
        };
        let (x, y) = (screen.x, screen.y);
        let values = [
            screen.config.sx.to_string(),
            screen.config.sy.to_string(),
            screen.config.sz.to_string(),
            format!("{}%", screen.config.speed),
        ];
        event
            .draw
            .fill([x - 8.0, y - 4.0, 208.0, 172.0], 0xC8_10_10_10);
        event
            .draw
            .text(screen.tier.title(), [x + 10.0, y + 4.0], 0xFFFF_FFFF, false);
        for (row, label) in ["范围 X", "范围 Y", "范围 Z", "速度"].iter().enumerate() {
            let row_y = y + 30.0 + row as f32 * 22.0;
            event
                .draw
                .text(label, [x + 10.0, row_y], 0xFFC0_C0C0, false);
            // Right-aligned against the minus button, so long values keep clear of it.
            let value = &values[row];
            let value_width = event.draw.text_width(value);
            event
                .draw
                .text(value, [x + 122.0 - value_width, row_y], 0xFFFF_FF80, false);
        }
        // The size of the whole area, so nobody has to work it out from the three rows.
        let area = format!(
            "{}x{}x{}",
            screen.config.sx, screen.config.sy, screen.config.sz
        );
        let area_width = event.draw.text_width(&area);
        event
            .draw
            .text("区域", [x + 10.0, y + 110.0], 0xFFC0_C0C0, false);
        event.draw.text(
            &area,
            [x + 122.0 - area_width, y + 110.0],
            0xFFFF_FF80,
            false,
        );
        // A refused label is not drawn; screen input remains usable.
        let _ = screen
            .widgets
            .draw(&mut *event.draw, host, event.screen.mouse);
    }

    fn screen_mouse_button_pressed_pre(
        &self,
        host: &dyn ClientHost,
        event: &mut ScreenMouseButtonPressedPreEvent<'_>,
    ) -> Flow {
        let (at, button, modifiers) = (event.screen.mouse, event.button, event.modifiers);
        self.input(host, |w| w.mouse_pressed(host, at, button, modifiers))
    }

    fn screen_mouse_button_released_pre(
        &self,
        host: &dyn ClientHost,
        event: &mut ScreenMouseButtonReleasedPreEvent<'_>,
    ) -> Flow {
        let button = event.button;
        self.input(host, |w| w.mouse_released(button))
    }

    fn screen_key_pressed_pre(
        &self,
        host: &dyn ClientHost,
        event: &mut ScreenKeyPressedPreEvent<'_>,
    ) -> Flow {
        let (key, modifiers) = (event.key, event.modifiers);
        self.input(host, |w| w.key_pressed(host, key, modifiers))
    }

    fn screen_char_typed_pre(
        &self,
        host: &dyn ClientHost,
        event: &mut ScreenCharTypedPreEvent<'_>,
    ) -> Flow {
        let c = event.c;
        self.input(host, |w| w.char_typed(host, c))
    }
}

impl Mod {
    /// Runs the input on the open screen's widgets and acts on their answer.
    fn input(&self, host: &dyn ClientHost, input: impl FnOnce(&mut Widgets) -> Outcome) -> Flow {
        if host.mod_screen() != Some(SCREEN_ID) {
            return Flow::Continue;
        }
        let mut slot = SCREEN.lock().unwrap();
        let Some(screen) = slot.as_mut() else {
            return Flow::Continue;
        };
        let outcome = input(&mut screen.widgets);
        if let Outcome::Pressed(id) = outcome {
            if id == screen.done {
                CLOSE.store(true, Ordering::Relaxed);
                return Flow::Cancel;
            }
            let changed = if id == screen.toggle {
                screen.config.active = if screen.config.active == 0 { 1 } else { 0 };
                true
            } else if let Some(row) = screen.minus.iter().position(|hovered| *hovered == id) {
                screen.step(row, -1)
            } else if let Some(row) = screen.plus.iter().position(|hovered| *hovered == id) {
                screen.step(row, 1)
            } else {
                false
            };
            if changed {
                screen.refresh();
                *PENDING.lock().unwrap() = Some(screen.config.clone());
            }
            return Flow::Cancel;
        }
        if outcome.handled() {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }
}

/// The settings screen: four rows of minus/plus buttons and a switch, on the torch they belong to.
struct Screen {
    widgets: Widgets,
    config: Config,
    tier: Tier,
    x: f32,
    y: f32,
    minus: Vec<WidgetId>,
    plus: Vec<WidgetId>,
    toggle: WidgetId,
    done: WidgetId,
}

impl Screen {
    fn new(width: f32, config: Config, tier: Tier) -> Self {
        let x = (width / 2.0 - 104.0).floor();
        let y = 24.0;
        let mut widgets = Widgets::new();
        let mut minus = Vec::with_capacity(4);
        let mut plus = Vec::with_capacity(4);
        for row in 0..4 {
            let row_y = y + 26.0 + row as f32 * 22.0;
            minus.push(widgets.add_button(Button::new(
                [x + 128.0, row_y, 18.0, 18.0],
                Text::literal("-"),
            )));
            plus.push(widgets.add_button(Button::new(
                [x + 168.0, row_y, 18.0, 18.0],
                Text::literal("+"),
            )));
        }
        let toggle = widgets.add_button(Button::new(
            [x + 8.0, y + 128.0, 100.0, 18.0],
            Text::literal(toggle_label(config.active)),
        ));
        let done = widgets.add_button(Button::new(
            [x + 112.0, y + 128.0, 88.0, 18.0],
            Text::literal("完成"),
        ));
        Self {
            widgets,
            config,
            tier,
            x,
            y,
            minus,
            plus,
            toggle,
            done,
        }
    }

    /// Moves row 0 width, 1 height, 2 depth, 3 speed one step; false when it cannot move.
    fn step(&mut self, row: usize, delta: i32) -> bool {
        match row {
            0 => bump_side(&mut self.config.sx, delta, self.tier.max_side()),
            1 => bump_side(&mut self.config.sy, delta, self.tier.max_side()),
            2 => bump_side(&mut self.config.sz, delta, self.tier.max_side()),
            _ => {
                let speeds = self.tier.speeds();
                let index = speeds
                    .iter()
                    .position(|speed| *speed >= self.config.speed)
                    .unwrap_or(speeds.len() - 1);
                let next = (index as i32 + delta).clamp(0, speeds.len() as i32 - 1) as usize;
                if next == index {
                    false
                } else {
                    self.config.speed = speeds[next];
                    true
                }
            }
        }
    }

    fn refresh(&mut self) {
        if let Some(button) = self.widgets.button_mut(self.toggle) {
            button.label = Text::literal(toggle_label(self.config.active));
        }
    }
}

/// Widens or narrows one side of the area by two blocks, staying odd and inside 1..=max; false
/// when it did not move.
fn bump_side(value: &mut i32, delta: i32, max: i32) -> bool {
    let next = odd_side(*value + 2 * delta, max);
    if next == *value {
        false
    } else {
        *value = next;
        true
    }
}

fn toggle_label(active: i32) -> &'static str {
    if active == 0 {
        "火把：关闭"
    } else {
        "火把：开启"
    }
}

/// Reads a torch's settings off its block entity, with the defaults a new torch has.
fn load(host: &dyn Host, level: mod_api::LevelRef, pos: [i32; 3], dim: &str, tier: Tier) -> Config {
    let default_speed = tier.default_speed();
    let mut config = Config {
        kind: 0,
        x: pos[0],
        y: pos[1],
        z: pos[2],
        active: 0,
        speed: default_speed,
        sx: 3,
        sy: 1,
        sz: 3,
        dim: dim.to_owned(),
    };
    if let Some(entity) = host.block_entity(level, pos) {
        if let Some(data) = entity.data(host) {
            config.active = if flag(&data, "active") { 1 } else { 0 };
            config.speed = tier.clamp(int(&data, "speed", default_speed));
            let max_side = tier.max_side();
            config.sx = side(&data, "sx", "rx", 3, max_side);
            config.sy = side(&data, "sy", "ry", 1, max_side);
            config.sz = side(&data, "sz", "rz", 3, max_side);
        }
    }
    config
}

/// The server end of a screen change: check it against the sender and the torch, then store it.
fn apply(config: Config, context: &mut dyn IPayloadContext) -> Result<(), String> {
    let Ok(player) = context.player() else {
        return Ok(());
    };
    let Ok(host) = context.server_host() else {
        return Ok(());
    };
    // The packet names the level and the position: only the player standing in that level, close
    // to that torch, may read or change it, whatever the packet says.
    let sender_dimension = player
        .dimension(&*host)
        .map_or_else(String::new, |dimension| dimension.to_string());
    let pos = [config.x, config.y, config.z];
    let near = player.entity().position(&*host).is_some_and(|at| {
        let dx = at[0] - (f64::from(pos[0]) + 0.5);
        let dy = at[1] - (f64::from(pos[1]) + 0.5);
        let dz = at[2] - (f64::from(pos[2]) + 0.5);
        dx * dx + dy * dy + dz * dz <= MAX_SETTING_DISTANCE * MAX_SETTING_DISTANCE
    });
    if sender_dimension != config.dim || !near {
        // Never fail the payload: a failed handler disconnects the sender. Just tell the player.
        let _ = player.tell(host, Text::literal("加速火把：距离太远或不在同一维度"));
        return Ok(());
    }
    let Some(level) = host.level(&config.dim) else {
        return Ok(());
    };
    let Some(block) = level.block(&*host, pos) else {
        return Ok(());
    };
    let Some(tier) = tier_of(&*host, block) else {
        return Ok(());
    };
    if config.kind == 1 {
        // A query: answer with what the torch really holds.
        let truth = load(&*host, level, pos, &config.dim, tier);
        if let Some(reply) = REPLY.get() {
            let _ = reply.reply(context, &truth);
        }
        return Ok(());
    }
    let Some(entity) = host.block_entity(level, pos) else {
        return Ok(());
    };
    let mut patch = Compound::new();
    patch.insert(
        "active".into(),
        Tag::Byte(u8::from(config.active != 0) as i8),
    );
    patch.insert("speed".into(), Tag::Int(tier.clamp(config.speed)));
    let max_side = tier.max_side();
    patch.insert("sx".into(), Tag::Int(odd_side(config.sx, max_side)));
    patch.insert("sy".into(), Tag::Int(odd_side(config.sy, max_side)));
    patch.insert("sz".into(), Tag::Int(odd_side(config.sz, max_side)));
    let _ = entity.merge_data(host, patch);
    host.block_entity_sync(pos);
    Ok(())
}

/// The three torches: they differ in how fast they may go.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Tier {
    Normal,
    Advanced,
    Super,
}

impl Tier {
    fn title(self) -> &'static str {
        match self {
            Tier::Normal => "加速火把",
            Tier::Advanced => "高级加速火把",
            Tier::Super => "超级加速火把",
        }
    }

    /// Which of the three spark sprite sets this torch shows.
    fn spark_index(self) -> usize {
        match self {
            Tier::Normal => 0,
            Tier::Advanced => 1,
            Tier::Super => 2,
        }
    }

    /// The four speeds this tier offers, as percentages.
    fn speeds(self) -> &'static [i32] {
        match self {
            Tier::Normal => &[100, 200, 300, 400],
            Tier::Advanced => &[4_000, 8_000, 12_000, 16_000],
            Tier::Super => &[160_000, 320_000, 480_000, 640_000],
        }
    }

    /// What a new torch of this tier starts at: the slowest speed it offers.
    fn default_speed(self) -> i32 {
        self.speeds()[0]
    }

    /// The widest area this torch may cover, in blocks: 3x3x3, 9x9x9 or 27x27x27.
    fn max_side(self) -> i32 {
        match self {
            Tier::Normal => 3,
            Tier::Advanced => 9,
            Tier::Super => 27,
        }
    }

    /// The fastest allowed speed not above speed, or the slowest one when speed is under it.
    fn clamp(self, speed: i32) -> i32 {
        let mut best = self.speeds()[0];
        for allowed in self.speeds() {
            if speed >= *allowed {
                best = *allowed;
            }
        }
        best
    }
}

fn tier_of(host: &dyn Host, block: mod_api::BlockKey) -> Option<Tier> {
    tier_of_name(&block.id(host))
}

fn tier_of_name(name: &str) -> Option<Tier> {
    match name.rsplit(':').next()? {
        "accel_torch" => Some(Tier::Normal),
        "advanced_accel_torch" => Some(Tier::Advanced),
        "super_accel_torch" => Some(Tier::Super),
        _ => None,
    }
}

/// The behaviour every accel torch shares: it speeds up the blocks around it.
struct Torch;

#[mod_api::horizon_part]
impl BlockPart for Torch {}

#[mod_api::horizon_part]
impl BlockEntityPart for Torch {
    /// New torches start switched off with the smallest range, like Torcherino's.
    fn block_entity(
        &self,
        _level: LevelSide,
        event: &mut BlockEntityEvent<'_>,
    ) -> Option<Compound> {
        let tier = tier_of_name(event.block).unwrap_or(Tier::Normal);
        let mut data = Compound::new();
        data.insert("active".into(), Tag::Byte(0));
        data.insert("speed".into(), Tag::Int(tier.default_speed()));
        data.insert("sx".into(), Tag::Int(3));
        data.insert("sy".into(), Tag::Int(1));
        data.insert("sz".into(), Tag::Int(3));
        Some(data)
    }

    fn block_entity_ticks(&self, _host: &dyn Host, event: &mut BlockEntityTicksEvent<'_>) {
        event.ticks = true;
    }

    /// Each tick, offer every block in range a share of random ticks - 100% is one extra
    /// random tick per block per tick - and hand them to the level. The top settings of a super
    /// torch would be millions of calls, so all the torches of a server share one call and time
    /// budget per tick, and each pass starts where the previous one stopped, so a capped torch
    /// still covers its whole area over a few ticks.
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
        // The area is stored as its width in blocks, so a super torch at its widest reads 27.
        let sx = odd_side(int(data, "sx", 3), MAX_SIDE);
        let sy = odd_side(int(data, "sy", 1), MAX_SIDE);
        let sz = odd_side(int(data, "sz", 3), MAX_SIDE);
        let (rx, ry, rz) = ((sx - 1) / 2, (sy - 1) / 2, (sz - 1) / 2);
        let Some(level) = host.level(host.dimension()) else {
            return;
        };
        let [x, y, z] = pos;
        let (nx, ny, nz) = (i64::from(sx), i64::from(sy), i64::from(sz));
        // Every cell of the area but the one the torch itself stands in.
        let cells = nx * ny * nz - 1;
        if cells <= 0 {
            return;
        }

        // The budget is the server's, not this torch's: the first torch to tick in a tick renews
        // it and every torch spends from the same share, so more torches never cost more than
        // this in one tick.
        let tick = level
            .game_time(&*host)
            .unwrap_or_else(|| (now_nanos() / 50_000_000) as i64);
        begin_tick(tick);
        let left = (MAX_CALLS_PER_TICK - BUDGET_CALLS.load(Ordering::Relaxed)).max(0);
        // This torch's own slice of the tick, so the next torch always has something left.
        let (share, start) = claim(pos, tick, cells);
        let remaining = share.min(left);
        if remaining > 0 {
            // How many random ticks one block gets on a pass: its full share, trimmed to what is
            // left of the tick. What a pass does not reach is picked up by the next one.
            let rounds = (speed / 100).max(1) as i64;
            let per_cell = rounds
                .min(ROUNDS_PER_CELL)
                .min((remaining / cells).max(1))
                .min(remaining);
            let visits = (remaining / per_cell).min(cells);
            let center = (i64::from(rz) * ny + i64::from(ry)) * nx + i64::from(rx);
            let mut visited = 0i64;
            while visited < visits {
                if visited & 31 == 0 && now_nanos() >= BUDGET_DEADLINE.load(Ordering::Relaxed) {
                    break;
                }
                let index = (start + visited) % cells;
                // Step over the torch's own cell.
                let index = if index >= center { index + 1 } else { index };
                let at = [
                    x + (index % nx) as i32 - rx,
                    y + (index / nx % ny) as i32 - ry,
                    z + (index / (nx * ny)) as i32 - rz,
                ];
                for _ in 0..per_cell {
                    level.block_random_tick(host, at);
                }
                BUDGET_CALLS.fetch_add(per_cell, Ordering::Relaxed);
                visited += 1;
            }
            advance(pos, start + visited);
        }
        // Now and then a spark over the flame, so a running torch shows its tier.
        if roll(0.35)
            && let (Some(sparks), Some(block)) = (SPARKS.get(), level.block(&*host, pos))
            && let Some(tier) = tier_of(&*host, block)
        {
            let options = ParticleOptions::simple(sparks[tier.spark_index()]);
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
    }

    /// What clients keep of the torch: its settings, so the screen opens on them.
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

fn flag(data: &Compound, key: &str) -> bool {
    matches!(data.get(&key.into()), Some(Tag::Byte(value)) if *value != 0)
}

fn int(data: &Compound, key: &str, default: i32) -> i32 {
    match data.get(&key.into()) {
        Some(Tag::Int(value)) => *value,
        Some(Tag::Short(value)) => i32::from(*value),
        Some(Tag::Byte(value)) => i32::from(*value),
        _ => default,
    }
}

/// Reads one side of the area: the stored width, else a radius from a torch saved before widths
/// were stored, else the default. Always odd and never above the tier's maximum.
fn side(data: &Compound, key: &str, radius_key: &str, default: i32, max: i32) -> i32 {
    let stored = int(data, key, 0);
    let value = if stored > 0 {
        stored
    } else {
        let radius = int(data, radius_key, -1);
        if radius >= 0 { 2 * radius + 1 } else { default }
    };
    odd_side(value, max)
}

/// Clamps a side of the area to 1..=max and rounds it up to an odd number of blocks.
fn odd_side(value: i32, max: i32) -> i32 {
    let value = value.clamp(1, max);
    if value % 2 == 0 {
        (value + 1).min(max)
    } else {
        value
    }
}

/// Nanoseconds since the mod first measured time, for the shared per-tick budget.
fn now_nanos() -> u64 {
    let start = CLOCK.get_or_init(Instant::now);
    Instant::now().duration_since(*start).as_nanos() as u64
}

/// Renews the shared budget when a new server tick reaches its first torch: the calls it may
/// spend and the instant its time runs out.
fn begin_tick(tick: i64) {
    if BUDGET_TICK.swap(tick, Ordering::Relaxed) != tick {
        BUDGET_CALLS.store(0, Ordering::Relaxed);
        let deadline = now_nanos() + CALL_TIME_BUDGET.as_nanos() as u64;
        BUDGET_DEADLINE.store(deadline, Ordering::Relaxed);
    }
}

/// The share of the tick's budget this torch may spend, and where its pass starts.
///
/// Every torch that ticked in the previous tick takes an equal part of the budget, so a torch
/// can never spend what a later one would have used: with five running torches each spends a
/// fifth, in whatever order the server calls them. A pass carries on from the cell the last
/// one stopped at, so a torch whose share is smaller than its area still covers it over a few
/// ticks instead of starting over at the same cell.
fn claim(pos: [i32; 3], tick: i64, cells: i64) -> (i64, i64) {
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

/// Remembers where this torch's next pass starts.
fn advance(pos: [i32; 3], cursor: i64) {
    if let Ok(mut torches) = TORCHES.lock()
        && let Some(torch) = torches.iter_mut().find(|torch| torch.pos == pos)
    {
        torch.cursor = cursor;
    }
}

/// A small SplitMix64, so the mod needs no crate of its own for the rolls.
static SEED: AtomicU64 = AtomicU64::new(0);

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

fn roll(chance: f64) -> bool {
    let unit = (next_u64() >> 11) as f64 / 9_007_199_254_740_992.0;
    unit < chance
}
