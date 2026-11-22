//! The settings screen: its widgets, its frame, and the input that edits a torch.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use mod_api::client::widgets::{Button, Outcome, WidgetId, Widgets};
use mod_api::{ClientHost, Flow, ScreenRenderPostEvent, Text};

use crate::settings::{self, Config};
use crate::tier::Tier;
use crate::util::odd_side;

/// The open screen widgets, made on its first frame.
static SCREEN: Mutex<Option<Screen>> = Mutex::new(None);

/// Done was pressed: the next client tick closes the screen.
static CLOSE: AtomicBool = AtomicBool::new(false);

/// Opens the screen on the torch at `pos`, whose values come from the server.
pub(super) fn open(host: &mut dyn ClientHost, dim: String, pos: [i32; 3], tier: Tier) {
    *settings::OPEN.lock().unwrap() = Some(settings::Open {
        dim: dim.clone(),
        pos,
        config: None,
        tier,
    });
    *SCREEN.lock().unwrap() = None;
    host.mod_screen_open(settings::SCREEN_ID);
    settings::send_query(host, &dim, pos);
}

/// Runs on the client tick: closes the screen once Done was pressed, and takes the values the
/// server answered with.
pub(super) fn tick(host: &mut dyn ClientHost) {
    if CLOSE.swap(false, Ordering::Relaxed) {
        host.mod_screen_close(settings::SCREEN_ID);
        settings::close();
        *SCREEN.lock().unwrap() = None;
    }
    let Some(config) = settings::take_incoming() else {
        return;
    };
    let mut slot = SCREEN.lock().unwrap();
    let mut open = settings::OPEN.lock().unwrap();
    if let Some(open) = open.as_mut()
        && open.dim == config.dim
        && open.pos == [config.x, config.y, config.z]
    {
        open.config = Some(config);
        // The screen is built again from the confirmed values.
        *slot = None;
    }
}

/// Draws the open screen, showing the title alone until the values arrive.
pub(super) fn render(host: &dyn ClientHost, event: &mut ScreenRenderPostEvent<'_>) {
    if host.mod_screen() != Some(settings::SCREEN_ID) {
        return;
    }
    let width = event.draw.size()[0];
    let mut slot = SCREEN.lock().unwrap();
    if slot.is_none() {
        let open = settings::OPEN.lock().unwrap();
        let Some(open) = open.as_ref() else {
            return;
        };
        let Some(config) = open.config.clone() else {
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
        .fill([x - 8.0, y - 4.0, 208.0, 154.0], 0xC8_10_10_10);
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

    // A refused label is not drawn; screen input remains usable.
    let _ = screen
        .widgets
        .draw(&mut *event.draw, host, event.screen.mouse);
}
/// Runs the input on the open screen widgets and acts on their answer.
pub(super) fn handle(host: &dyn ClientHost, input: impl FnOnce(&mut Widgets) -> Outcome) -> Flow {
    if host.mod_screen() != Some(settings::SCREEN_ID) {
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
            settings::push(screen.config.clone());
        }
        return Flow::Cancel;
    }
    if outcome.handled() {
        Flow::Cancel
    } else {
        Flow::Continue
    }
}
/// The settings screen: four rows of minus/plus buttons and a switch, for the torch they belong
/// to.
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

    /// Moves row 0 reach X, 1 reach Y, 2 reach Z, 3 speed one step; false when it cannot move.
    fn step(&mut self, row: usize, delta: i32) -> bool {
        match row {
            0 => bump(&mut self.config.sx, delta, self.tier.max_reach()),
            1 => bump(&mut self.config.sy, delta, self.tier.max_reach()),
            2 => bump(&mut self.config.sz, delta, self.tier.max_reach()),
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

/// Widens or narrows the reach by two blocks, staying odd and inside 1..=max; false when it did
/// not move.
fn bump(value: &mut i32, delta: i32, max: i32) -> bool {
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
