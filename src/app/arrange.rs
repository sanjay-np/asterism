//! Moving the bar's items on the bar itself. A middle-button drag carries an item any time; in
//! arrange mode (Settings → Bar, `asterism msg bar arrange`) a plain left drag does, and clicks
//! don't open anything. Dropped between other items it goes there, in whichever part; pulled
//! well off the bar it's taken off. The three lists are written as Settings writes them.

use super::{App, BTN_LEFT, BTN_MIDDLE, PanelKind, Which};
use crate::bar::Carry;
use crate::settings::BarMove;
use crate::ui::Action;
use smithay_client_toolkit::reexports::protocols::wp::cursor_shape::v1::client::wp_cursor_shape_device_v1::Shape;

/// How far a middle press must move before it's a carry (less: it was a click).
const MOVE: f32 = 6.0;
/// How far off the bar an item must be let go to be taken off.
const OFF: f32 = 40.0;

/// An item being carried, and on which bar.
pub(super) struct Carrying {
    pub bar: usize,
    pub carry: Carry,
    /// Where the press was, and what a middle click there does (it waits for the release).
    press: f32,
    middle: Option<Action>,
}

impl App {
    /// Arrange mode on, off, or the other way.
    pub(super) fn arrange_bar_mode(&mut self, on: Option<bool>) {
        let on = on.unwrap_or(!self.bar_arrange.mode);
        if on {
            self.close_menu();
        }
        self.bar_arrange.mode = on;
        self.bar_carry = None;
        for bar in &mut self.bars {
            bar.surf.dirty = true;
        }
        if let Some(settings) = self.settings.as_mut() {
            settings.arranging = on;
        }
        if let Some(index) = self.panel_index(PanelKind::Settings) {
            self.panels[index].surf.dirty = true;
        }
        log::debug!("arranging the bar: {on}");
    }

    pub(super) fn bar_carrying(&self, which: Which) -> bool {
        matches!(which, Which::Bar(bar) if self.bar_carry.as_ref().is_some_and(|c| c.bar == bar))
    }

    /// A press on the bar that picks an item up: the middle button on any item, or the left on
    /// one in arrange mode. Anything else goes on as a press.
    pub(super) fn bar_pick_up(&mut self, bar: usize, x: f32, y: f32, button: u32) -> bool {
        let mode = self.bar_arrange.mode;
        let Some(surface) = self.bars.get(bar) else { return false };
        let hit = surface.surf.hit(x, y);
        let item = match button {
            BTN_LEFT if mode => match hit.and_then(|p| p.key.strip_prefix("arrange.grab.")) {
                Some(name) => surface.placed.spans.iter().find(|s| s.name == name),
                None => return false,
            },
            BTN_MIDDLE => surface.placed.at(x),
            _ => return false,
        };
        let Some(span) = item.cloned() else { return false };
        // A middle click keeps doing what it did (a tray app's second action), on the release
        let middle = if button == BTN_MIDDLE { hit.and_then(|p| p.actions.middle.clone()) } else { None };
        let moved = button == BTN_LEFT;
        self.close_menu();
        self.bar_carry = Some(Carrying { bar, carry: Carry { item: span.name.clone(), x, grab: x - span.x0, off: false, moved }, press: x, middle });
        if moved {
            self.set_cursor(Shape::Grabbing);
        }
        // Its tooltip goes while it's carried
        if let Some(surface) = self.bars.get_mut(bar) {
            surface.surf.hovered = None;
            surface.surf.dirty = true;
        }
        true
    }

    pub(super) fn bar_carry_motion(&mut self, x: f32, y: f32) {
        let Some(carrying) = self.bar_carry.as_mut() else { return };
        let Some(surface) = self.bars.get_mut(carrying.bar) else { return };
        let carry = &mut carrying.carry;
        if !carry.moved && (x - carrying.press).abs() < MOVE {
            return;
        }
        let started = !carry.moved;
        carry.moved = true;
        carry.x = x;
        carry.off = y < -OFF || y > surface.surf.h + OFF;
        surface.surf.dirty = true;
        if started {
            self.set_cursor(Shape::Grabbing);
        }
    }

    /// Let go: a middle click if it never moved, else dropped where the mark is (or taken off).
    pub(super) fn bar_carry_release(&mut self, x: f32, y: f32) {
        let Some(carrying) = self.bar_carry.take() else { return };
        let Some(surface) = self.bars.get_mut(carrying.bar) else { return };
        surface.surf.pressed = None;
        surface.surf.dirty = true;
        let off = y < -OFF || y > surface.surf.h + OFF;
        let slot = surface.placed.slot(&carrying.carry.item, x);
        self.set_cursor(if self.bar_arrange.mode { Shape::Grab } else { Shape::Default });
        if !carrying.carry.moved {
            if let Some(action) = carrying.middle {
                self.act(action, None, 0);
            }
            return;
        }
        let item = carrying.carry.item;
        let how = if off { BarMove::Off(item) } else { BarMove::Place(item, slot.part, slot.after) };
        log::debug!("bar item dropped: {how:?}");
        self.arrange_bar(&how);
    }
}
