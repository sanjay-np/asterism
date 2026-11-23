//! The day panel as the app runs it: opening it under the clock, typing into its task field and
//! notes, ticking tasks off, and writing the files back a moment after the last change.

use std::time::{Duration, Instant};

use smithay_client_toolkit::seat::keyboard::{KeyEvent, Keysym};
use smithay_client_toolkit::shell::wlr_layer::{Anchor, KeyboardInteractivity, Layer};

use super::App;
use super::shell::PanelKind;
use crate::model::Command;
use crate::popups;

/// How long after the last change the files are written (typing a note shouldn't write a file
/// for every letter).
const SAVE_AFTER: Duration = Duration::from_secs(1);

/// Which of the panel's fields has the keyboard.
pub(super) const TASK_FIELD: u8 = 1;
pub(super) const NOTES_FIELD: u8 = 2;

impl App {
    pub(super) fn day_folder(&self) -> std::path::PathBuf {
        crate::day::folder(&self.config.day.folder)
    }

    pub(super) fn toggle_day(&mut self, open: Option<bool>) {
        let showing = self.panel_index(PanelKind::Day).is_some();
        match (open, showing) {
            (Some(true), true) | (Some(false), false) => {}
            (_, true) => self.close_day(),
            (_, false) => {
                self.close_menu();
                self.close_panel(PanelKind::Control);
                self.close_panel(PanelKind::Launcher);
                self.day_state = popups::State::default();
                // Read again each time, so edits made to the files elsewhere show; unless a change
                // of ours is still waiting to be written, which is newer
                if self.day_dirty.is_none() {
                    self.send(Command::DayLoad(self.day_folder()));
                }
                // The whole output below the bar, like the control centre: a click beside the card
                // closes it
                self.open_panel(PanelKind::Day, Layer::Top, Anchor::all(), (0, 0), (0, 0, 0, 0), KeyboardInteractivity::Exclusive, 0);
            }
        }
    }

    /// Closes the panel, writing what changed straight away.
    pub(super) fn close_day(&mut self) {
        self.close_panel(PanelKind::Day);
        self.save_day(true);
    }

    pub(super) fn day_changed(&mut self) {
        self.day_dirty = Some(Instant::now());
        self.redraw_day();
        // The bar's day button counts what's left to do
        for bar in &mut self.bars {
            bar.surf.dirty = true;
        }
    }

    pub(super) fn redraw_day(&mut self) {
        if let Some(index) = self.panel_index(PanelKind::Day) {
            self.panels[index].surf.dirty = true;
        }
    }

    /// Writes the files once a change has settled (or now, with `now`). Nothing is written
    /// before they've been read, so an early keypress can't empty them.
    pub(super) fn save_day(&mut self, now: bool) {
        let Some(changed) = self.day_dirty else { return };
        if !now && changed.elapsed() < SAVE_AFTER {
            return;
        }
        self.day_dirty = None;
        if self.model.day.loaded {
            self.send(Command::DaySave(self.day_folder(), self.model.day.clone()));
        }
    }

    pub(super) fn day_focus(&mut self, field: u8) {
        self.day_state.day_field = if self.day_state.day_field == field && field == TASK_FIELD { 0 } else { field };
        self.redraw_day();
    }

    pub(super) fn day_toggle(&mut self, index: usize) {
        if let Some(task) = self.model.day.tasks.get_mut(index) {
            task.done = !task.done;
            self.day_changed();
        }
    }

    pub(super) fn day_remove(&mut self, index: usize) {
        if index < self.model.day.tasks.len() {
            self.model.day.tasks.remove(index);
            self.day_changed();
        }
    }

    pub(super) fn day_clear_done(&mut self) {
        let before = self.model.day.tasks.len();
        self.model.day.tasks.retain(|task| !task.done);
        if self.model.day.tasks.len() != before {
            self.day_changed();
        }
    }

    pub(super) fn day_key(&mut self, event: &KeyEvent) {
        let field = self.day_state.day_field;
        let ctrl = self.modifiers.ctrl;
        let plain = |text: &Option<String>| text.as_ref().filter(|t| !ctrl && !t.is_empty() && t.chars().all(|c| !c.is_control())).cloned();
        match event.keysym {
            // Esc lets go of a field first, then closes
            Keysym::Escape if field != 0 => self.day_state.day_field = 0,
            Keysym::Escape => return self.close_day(),
            Keysym::Tab => self.day_state.day_field = if field == TASK_FIELD { NOTES_FIELD } else { TASK_FIELD },
            _ if field == TASK_FIELD => match event.keysym {
                Keysym::Return | Keysym::KP_Enter => {
                    let text = self.day_state.day_typed.trim().to_string();
                    if !text.is_empty() {
                        self.model.day.tasks.push(crate::day::Task { text, done: false });
                        self.day_state.day_typed.clear();
                        self.day_changed();
                    }
                }
                Keysym::BackSpace if ctrl => self.day_state.day_typed.clear(),
                Keysym::BackSpace => {
                    self.day_state.day_typed.pop();
                }
                _ => match plain(&event.utf8) {
                    Some(text) => self.day_state.day_typed.push_str(&text),
                    None => return,
                },
            },
            _ if field == NOTES_FIELD => match event.keysym {
                Keysym::Return | Keysym::KP_Enter => {
                    self.model.day.notes.push('\n');
                    self.day_changed();
                }
                Keysym::BackSpace if ctrl => {
                    // A word at a time
                    let notes = &mut self.model.day.notes;
                    let trimmed = notes.trim_end_matches(' ').len();
                    let cut = notes[..trimmed].rfind([' ', '\n']).map_or(0, |i| i + 1);
                    notes.truncate(cut);
                    self.day_changed();
                }
                Keysym::BackSpace => {
                    if self.model.day.notes.pop().is_some() {
                        self.day_changed();
                    }
                }
                _ => match plain(&event.utf8) {
                    Some(text) => {
                        self.model.day.notes.push_str(&text);
                        self.day_changed();
                    }
                    None => return,
                },
            },
            // Nothing being typed: a letter starts a task
            _ => match plain(&event.utf8) {
                Some(text) => {
                    self.day_state.day_field = TASK_FIELD;
                    self.day_state.day_typed.push_str(&text);
                }
                None => return,
            },
        }
        self.redraw_day();
    }
}
