//! Timers: a countdown, a focus timer (work, a break, work again), and a stopwatch. Plain
//! state and arithmetic on `Instant`s; the app ticks it once a second and says what happened.

use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FocusSettings {
    pub work: Duration,
    pub short_break: Duration,
    pub long_break: Duration,
    /// A long break after this many rounds of work.
    pub rounds: u32,
}

impl Default for FocusSettings {
    fn default() -> Self {
        Self { work: mins(25), short_break: mins(5), long_break: mins(15), rounds: 4 }
    }
}

pub fn mins(m: u64) -> Duration {
    Duration::from_secs(m * 60)
}

/// Time running down: either towards an end, or held with what's left.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Clock {
    Running { ends: Instant },
    Paused { left: Duration },
}

impl Clock {
    pub fn start(length: Duration, now: Instant) -> Self {
        Clock::Running { ends: now + length }
    }
    pub fn left(&self, now: Instant) -> Duration {
        match self {
            Clock::Running { ends } => ends.saturating_duration_since(now),
            Clock::Paused { left } => *left,
        }
    }
    pub fn running(&self) -> bool {
        matches!(self, Clock::Running { .. })
    }
    pub fn toggle(&mut self, now: Instant) {
        *self = match *self {
            Clock::Running { .. } => Clock::Paused { left: self.left(now) },
            Clock::Paused { left } => Clock::Running { ends: now + left },
        };
    }
    pub fn add(&mut self, more: Duration) {
        match self {
            Clock::Running { ends } => *ends += more,
            Clock::Paused { left } => *left += more,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Countdown {
    pub total: Duration,
    pub clock: Clock,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Work,
    ShortBreak,
    LongBreak,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Focus {
    pub phase: Phase,
    /// Which round of work, from 1.
    pub round: u32,
    pub total: Duration,
    pub clock: Clock,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Stopwatch {
    /// Since when it's been running, if it is.
    pub since: Option<Instant>,
    /// Time counted before the last start.
    pub banked: Duration,
    /// Each lap's time on the stopwatch when it was taken, newest last.
    pub laps: Vec<Duration>,
}

impl Stopwatch {
    pub fn elapsed(&self, now: Instant) -> Duration {
        self.banked + self.since.map_or(Duration::ZERO, |s| now.saturating_duration_since(s))
    }
    pub fn toggle(&mut self, now: Instant) {
        match self.since.take() {
            Some(since) => self.banked += now.saturating_duration_since(since),
            None => self.since = Some(now),
        }
    }
    pub fn lap(&mut self, now: Instant) {
        if self.since.is_some() {
            self.laps.push(self.elapsed(now));
        }
    }
    pub fn reset(&mut self) {
        *self = Stopwatch::default();
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Timers {
    pub countdown: Option<Countdown>,
    pub focus: Option<Focus>,
    pub stopwatch: Stopwatch,
    pub settings: FocusSettings,
}

/// Something worth telling someone about.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    CountdownDone(Duration),
    /// The focus timer moved on to this phase.
    FocusNext(Phase, u32),
}

impl Timers {
    /// Anything that changes from one second to the next (so the bar wants painting).
    pub fn ticking(&self) -> bool {
        self.countdown.is_some_and(|c| c.clock.running()) || self.focus.is_some_and(|f| f.clock.running()) || self.stopwatch.since.is_some()
    }

    pub fn start_countdown(&mut self, length: Duration, now: Instant) {
        self.countdown = Some(Countdown { total: length, clock: Clock::start(length, now) });
    }

    pub fn start_focus(&mut self, now: Instant) {
        let work = self.settings.work;
        self.focus = Some(Focus { phase: Phase::Work, round: 1, total: work, clock: Clock::start(work, now) });
    }

    /// On to the next phase: a break after work (a long one every few rounds), work after a break.
    pub fn next_phase(&mut self, now: Instant) -> Option<Event> {
        let s = self.settings;
        let focus = self.focus.as_mut()?;
        let (phase, round, total) = match focus.phase {
            Phase::Work if focus.round % s.rounds.max(1) == 0 => (Phase::LongBreak, focus.round, s.long_break),
            Phase::Work => (Phase::ShortBreak, focus.round, s.short_break),
            Phase::ShortBreak | Phase::LongBreak => (Phase::Work, focus.round + 1, s.work),
        };
        *focus = Focus { phase, round, total, clock: Clock::start(total, now) };
        Some(Event::FocusNext(phase, round))
    }

    /// Once a second: what ran out.
    pub fn tick(&mut self, now: Instant) -> Vec<Event> {
        let mut events = Vec::new();
        if let Some(countdown) = self.countdown
            && countdown.clock.running()
            && countdown.clock.left(now).is_zero()
        {
            self.countdown = None;
            events.push(Event::CountdownDone(countdown.total));
        }
        if self.focus.is_some_and(|f| f.clock.running() && f.clock.left(now).is_zero()) {
            events.extend(self.next_phase(now));
        }
        events
    }
}

/// 1:05:09, 25:00, 0:42
pub fn clock_text(d: Duration) -> String {
    let s = d.as_secs();
    if s >= 3600 { format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60) } else { format!("{}:{:02}", s / 60, s % 60) }
}

/// 25 min, 1 h 30 min, 45 s
pub fn length_text(d: Duration) -> String {
    let s = d.as_secs();
    match (s / 3600, s / 60 % 60) {
        (0, 0) => format!("{s} s"),
        (0, m) => format!("{m} min"),
        (h, 0) => format!("{h} h"),
        (h, m) => format!("{h} h {m} min"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_down_pauses_and_rings() {
        let t0 = Instant::now();
        let mut timers = Timers::default();
        timers.start_countdown(Duration::from_secs(60), t0);
        assert_eq!(timers.tick(t0 + Duration::from_secs(30)), []);
        timers.countdown.as_mut().unwrap().clock.toggle(t0 + Duration::from_secs(30));
        // Paused: an hour later, still 30 s left
        assert_eq!(timers.tick(t0 + mins(60)), []);
        assert_eq!(timers.countdown.unwrap().clock.left(t0 + mins(60)), Duration::from_secs(30));
        let back = t0 + mins(60);
        timers.countdown.as_mut().unwrap().clock.toggle(back);
        assert_eq!(timers.tick(back + Duration::from_secs(31)), [Event::CountdownDone(Duration::from_secs(60))]);
        assert!(timers.countdown.is_none());
    }

    #[test]
    fn focus_goes_round() {
        let t0 = Instant::now();
        let mut timers = Timers { settings: FocusSettings { rounds: 2, ..Default::default() }, ..Default::default() };
        timers.start_focus(t0);
        let mut now = t0;
        let mut seen = Vec::new();
        for _ in 0..4 {
            now += timers.focus.unwrap().clock.left(now) + Duration::from_secs(1);
            seen.extend(timers.tick(now));
        }
        assert_eq!(seen, [Event::FocusNext(Phase::ShortBreak, 1), Event::FocusNext(Phase::Work, 2), Event::FocusNext(Phase::LongBreak, 2), Event::FocusNext(Phase::Work, 3)]);
    }

    #[test]
    fn stopwatch_keeps_time_and_laps() {
        let t0 = Instant::now();
        let mut watch = Stopwatch::default();
        watch.toggle(t0);
        watch.lap(t0 + Duration::from_secs(10));
        watch.toggle(t0 + Duration::from_secs(15));
        assert_eq!(watch.elapsed(t0 + mins(5)), Duration::from_secs(15));
        watch.toggle(t0 + mins(5));
        assert_eq!(watch.elapsed(t0 + mins(5) + Duration::from_secs(5)), Duration::from_secs(20));
        assert_eq!(watch.laps, [Duration::from_secs(10)]);
        assert_eq!(clock_text(Duration::from_secs(3909)), "1:05:09");
        assert_eq!(length_text(mins(90)), "1 h 30 min");
    }
}
