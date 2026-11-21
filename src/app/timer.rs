//! The timers as the app runs them: what their buttons and `msg timer …` do, and the
//! once-a-second tick that rings when time's up.

use super::*;
use crate::model::{Note, TimerAction};
use crate::timers::{Event, Phase, length_text};

impl App {
    pub(super) fn timer_action(&mut self, action: TimerAction) {
        let now = Instant::now();
        let timers = &mut self.model.timers;
        match action {
            TimerAction::Countdown(seconds) => timers.start_countdown(Duration::from_secs(seconds.max(1)), now),
            TimerAction::CountdownAdd(seconds) => {
                if let Some(countdown) = timers.countdown.as_mut()
                    && seconds >= 0
                {
                    countdown.clock.add(Duration::from_secs(seconds as u64));
                    countdown.total += Duration::from_secs(seconds as u64);
                }
            }
            TimerAction::CountdownPause => {
                if let Some(countdown) = timers.countdown.as_mut() {
                    countdown.clock.toggle(now);
                }
            }
            TimerAction::CountdownCancel => timers.countdown = None,
            TimerAction::Focus => {
                if timers.focus.is_some() {
                    timers.focus = None;
                } else {
                    timers.start_focus(now);
                }
            }
            TimerAction::FocusPause => {
                if let Some(focus) = timers.focus.as_mut() {
                    focus.clock.toggle(now);
                }
            }
            TimerAction::FocusSkip => {
                timers.next_phase(now);
            }
            TimerAction::Stopwatch => timers.stopwatch.toggle(now),
            TimerAction::StopwatchLap => timers.stopwatch.lap(now),
            TimerAction::StopwatchReset => timers.stopwatch.reset(),
        }
        self.timers_changed();
    }

    /// The bar and an open timer menu show the new state.
    fn timers_changed(&mut self) {
        for bar in &mut self.bars {
            bar.surf.dirty = true;
        }
        if let Some(popup) = self.popup.as_mut() {
            popup.surf.dirty = true;
        }
    }

    /// Once a second: what ran out rings, and what's running moves on.
    pub(super) fn tick_timers(&mut self) {
        self.model.timers.settings = self.config.timers.focus_settings();
        if !self.model.timers.ticking() {
            return;
        }
        for event in self.model.timers.tick(Instant::now()) {
            let note = match event {
                Event::CountdownDone(total) => Note { summary: "Time's up".into(), body: format!("Your {} countdown is done.", length_text(total)), icon: "alarm-symbolic".into(), urgency: 2 },
                Event::FocusNext(Phase::Work, round) => Note { summary: "Back to it".into(), body: format!("Round {round}: {} of focus.", length_text(self.model.timers.settings.work)), icon: "alarm-symbolic".into(), urgency: 1 },
                Event::FocusNext(phase, _) => {
                    let (name, length) = if phase == Phase::LongBreak { ("a long break", self.model.timers.settings.long_break) } else { ("a break", self.model.timers.settings.short_break) };
                    Note { summary: "Take a break".into(), body: format!("Time for {name}: {}.", length_text(length)), icon: "alarm-symbolic".into(), urgency: 1 }
                }
            };
            self.send(Command::Notify(note));
        }
        self.timers_changed();
    }
}
