//! The settings a torch saves, the payloads of its screen, and the server end that checks a
//! change against the sender and the torch before storing it.

use std::sync::{Mutex, OnceLock};

use mod_api::{
    ClientHost, Compound, Host, IPayloadContext, LevelRef, PayloadCodec, PayloadId, Registrar, Tag,
    Text, ToClient, ToServer,
};

use crate::tier::{Tier, tier_of};
use crate::util::{flag, int, odd_side};

/// The id of the payload that carries a change, and of the one the server answers with.
pub(super) const SCREEN_ID: &str = "accel_torch:config";
const STATE_ID: &str = "accel_torch:state";

/// How far the player using a screen may stand from its torch: a setting is accepted only from
/// the torch own level and within this reach, as for any block they interact with.
const MAX_SETTING_DISTANCE: f64 = 6.0;

/// What one torch screen sends when a button changes something, and what the server answers
/// with when the screen opens.
#[derive(Clone, Debug, PartialEq, Eq, Default, PayloadCodec)]
pub(super) struct Config {
    /// 0 = store these values, 1 = ask the server for the values the torch really holds.
    pub(super) kind: i32,
    pub(super) x: i32,
    pub(super) y: i32,
    pub(super) z: i32,
    /// 0 = off, 1 = on.
    pub(super) active: i32,
    /// A percentage: 100% is one extra random tick per block per tick.
    pub(super) speed: i32,
    /// How far the area reaches out from the torch on each axis: odd, up to the tier limit.
    pub(super) sx: i32,
    pub(super) sy: i32,
    pub(super) sz: i32,
    pub(super) dim: String,
}

/// What the open screen edits: the torch it belongs to, its tier, and the values the server
/// confirmed. The values stay unknown until that answer arrives, so the screen never shows or
/// sends numbers nobody confirmed.
pub(super) struct Open {
    pub(super) dim: String,
    pub(super) pos: [i32; 3],
    pub(super) config: Option<Config>,
    pub(super) tier: Tier,
}

/// The client token for Config, from registration.
static SEND: OnceLock<ToServer<Config>> = OnceLock::new();

/// The server token for the values it answers with.
static REPLY: OnceLock<ToClient<Config>> = OnceLock::new();

/// The open screen of this client.
pub(super) static OPEN: Mutex<Option<Open>> = Mutex::new(None);

/// Values the server sent, picked up by the next client tick.
static INCOMING: Mutex<Option<Config>> = Mutex::new(None);

/// A change the next client tick sends (the input hooks can only read the host).
static PENDING: Mutex<Option<Config>> = Mutex::new(None);

/// Registers the two payloads of the screen.
pub(super) fn register(r: &mut dyn Registrar) {
    let mut payloads = r.payload_handlers();
    let mut view = payloads.registrar("1");
    let send = view
        .play_to_server_typed::<Config>(PayloadId(SCREEN_ID.to_owned()), |config, context| {
            apply(config, context)
        });
    let reply =
        view.play_to_client_typed::<Config>(PayloadId(STATE_ID.to_owned()), |config, _context| {
            *INCOMING.lock().unwrap() = Some(config);
            Ok(())
        });
    let _ = SEND.set(send);
    let _ = REPLY.set(reply);
}

/// Asks the server for the values the torch at `pos` really holds: the client copy may be out of
/// date, so the screen never edits guessed numbers.
pub(super) fn send_query(host: &mut dyn ClientHost, dim: &str, pos: [i32; 3]) {
    let query = Config {
        kind: 1,
        x: pos[0],
        y: pos[1],
        z: pos[2],
        dim: dim.to_owned(),
        ..Config::default()
    };
    if let Some(send) = SEND.get() {
        let _ = send.send(host, &query);
    }
}

/// Sends the change a button made, from the client tick (the input hooks only read the host).
pub(super) fn send_pending(host: &mut dyn ClientHost) {
    let Some(config) = PENDING.lock().unwrap().take() else {
        return;
    };
    if let Some(send) = SEND.get() {
        let _ = send.send(host, &config);
    }
}

/// Remembers a change for the next client tick to send.
pub(super) fn push(config: Config) {
    *PENDING.lock().unwrap() = Some(config);
}

/// The values the server sent, if any arrived since the last tick.
pub(super) fn take_incoming() -> Option<Config> {
    INCOMING.lock().unwrap().take()
}

/// Forgets the torch the screen belonged to.
pub(super) fn close() {
    *OPEN.lock().unwrap() = None;
}

/// Reads the settings a torch holds, with the defaults a new torch has.
fn load(host: &dyn Host, level: LevelRef, pos: [i32; 3], dim: &str, tier: Tier) -> Config {
    let default_speed = tier.default_speed();
    let mut config = Config {
        kind: 0,
        x: pos[0],
        y: pos[1],
        z: pos[2],
        active: 0,
        speed: default_speed,
        sx: 1,
        sy: 1,
        sz: 1,
        dim: dim.to_owned(),
    };
    if let Some(entity) = host.block_entity(level, pos)
        && let Some(data) = entity.data(host)
    {
        config.active = i32::from(flag(&data, "active"));
        config.speed = tier.clamp(int(&data, "speed", default_speed));
        let max_reach = tier.max_reach();
        config.sx = odd_side(int(&data, "sx", 1), max_reach);
        config.sy = odd_side(int(&data, "sy", 1), max_reach);
        config.sz = odd_side(int(&data, "sz", 1), max_reach);
    }
    config
}

/// The server end of a screen change: check it against the sender and the torch, then either
/// answer with what the torch holds or store what the buttons changed.
///
/// Never fails: a failed payload handler disconnects the sender, so every refusal only tells the
/// player and returns.
fn apply(config: Config, context: &mut dyn IPayloadContext) -> Result<(), String> {
    let Ok(player) = context.player() else {
        return Ok(());
    };
    let Ok(host) = context.server_host() else {
        return Ok(());
    };
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
        let truth = load(&*host, level, pos, &config.dim, tier);
        if let Some(reply) = REPLY.get() {
            let _ = reply.reply(context, &truth);
        }
        return Ok(());
    }
    let Some(entity) = host.block_entity(level, pos) else {
        return Ok(());
    };
    let max_reach = tier.max_reach();
    let mut patch = Compound::new();
    patch.insert(
        "active".into(),
        Tag::Byte(u8::from(config.active != 0) as i8),
    );
    patch.insert("speed".into(), Tag::Int(tier.clamp(config.speed)));
    patch.insert("sx".into(), Tag::Int(odd_side(config.sx, max_reach)));
    patch.insert("sy".into(), Tag::Int(odd_side(config.sy, max_reach)));
    patch.insert("sz".into(), Tag::Int(odd_side(config.sz, max_reach)));
    let _ = entity.merge_data(host, patch);
    host.block_entity_sync(pos);
    Ok(())
}
