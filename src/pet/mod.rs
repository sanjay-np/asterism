//! A pet living on the screen: a cat or a fox along its foot, or a bird flying about and
//! perching on the bar. It behaves like one: it has energy (it naps when tired), hunger (fed
//! from a bowl, it asks when it's empty) and wants attention (ignored a while, it comes to find
//! you); it stretches and yawns when it wakes, loafs, grooms, stalks the pointer and pounces,
//! gets the zoomies when the machine is busy, bats a ball about, purrs when petted, dangles when
//! picked up and lands on its feet, and speaks now and then (`Event::Say`; the app plays the
//! sound, made in `sound.rs`).
//!
//! Everything here is plain: `Pet::step` moves it on by a moment and returns what happened;
//! `Pet::draw` makes its picture (`draw.rs`). `app/pet.rs` gives it a surface and the props.

pub mod draw;
pub mod sound;

use crate::theme::Theme;
use crate::transition::Chance;
use crate::ui::{Piece, Prim};
use draw::{Extras, Frame, Rig};
pub use sound::Voice;

/// The pet's surface; its feet stand at `FEET` in it (room above for a speech bubble).
pub const W: f32 = 240.0;
pub const H: f32 = 180.0;
pub const FEET: (f32, f32) = (120.0, 146.0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Species {
    Cat,
    Fox,
    Bird,
}

impl Species {
    pub fn from_name(name: &str) -> Option<Species> {
        match name.trim().to_lowercase().as_str() {
            "cat" => Some(Species::Cat),
            "fox" => Some(Species::Fox),
            "bird" => Some(Species::Bird),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Species::Cat => "cat",
            Species::Fox => "fox",
            Species::Bird => "bird",
        }
    }

    /// The next one, for "Another animal".
    pub fn next(self) -> Species {
        match self {
            Species::Cat => Species::Fox,
            Species::Fox => Species::Bird,
            Species::Bird => Species::Cat,
        }
    }

    fn grounded(self) -> bool {
        self != Species::Bird
    }
}

/// What it says: a call (meow), hello or thanks (a trill), contentment (a purr), or a complaint.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Mood {
    Call,
    Hello,
    Content,
    Complain,
}

/// What it's doing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Act {
    Stand,
    Walk,
    Run,
    Sit,
    Loaf,
    Lie,
    Sleep,
    Groom,
    Stretch,
    Yawn,
    /// Low and still, the hind end wiggling: about to pounce.
    Crouch,
    Pounce,
    /// A paw at the ball.
    Bat,
    Eat,
    /// Picked up, dangling.
    Held,
    Fall,
    // The bird's own
    Fly,
    Hop,
    Peck,
}

/// What happened in a moment, for the app.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// It spoke: which sound (and which take of it).
    Say(Voice, u32),
    /// It batted the ball this hard (pixels a second, signed).
    Kick(f32),
    /// It finished the bowl.
    Ate,
    /// It's had enough of the ball.
    Bored,
}

/// What the pet knows of the screen and of you, each moment.
pub struct World {
    /// The output's size, logical pixels.
    pub size: (f32, f32),
    /// Where feet stand: the screen's foot, or just above a bar down there.
    pub floor: f32,
    /// Where the bird can perch besides the floor: the bar's lower edge, when it's at the top.
    pub perch: Option<f32>,
    /// The pointer, when the shell can see it.
    pub pointer: Option<(f32, f32)>,
    /// Nobody's touched anything for a while.
    pub idle: bool,
    /// How busy the processor is, 0 to 1.
    pub busy: f32,
    /// The music's low notes, 0 to 1, while something plays.
    pub beat: Option<f32>,
    /// A bowl of food on the floor, at this x.
    pub bowl: Option<f32>,
    /// A toy on the floor (a ball, yarn): where, and how fast it's rolling.
    pub ball: Option<(f32, f32)>,
    /// Its bed, at this x on the floor: where it goes to nap.
    pub bed: Option<f32>,
    /// The other pets on the screen.
    pub friends: Vec<Friend>,
}

/// Another pet, as this one sees it.
#[derive(Clone, Copy, Debug)]
pub struct Friend {
    pub x: f32,
    pub act: Act,
    pub facing: f32,
    pub species: Species,
}

pub struct Pet {
    pub species: Species,
    /// Where its feet are, on the output.
    pub x: f32,
    pub y: f32,
    vx: f32,
    vy: f32,
    /// 1 faces right, −1 left.
    pub facing: f32,
    pub act: Act,
    /// Seconds left of what it's doing.
    left: f32,
    target: (f32, f32),
    /// How far its legs (or wings) have gone round.
    stride: f32,
    clock: f32,
    /// Needs, 0 to 1: energy (it sleeps when it's low), hunger, wanting attention.
    pub energy: f32,
    pub hunger: f32,
    pub lonely: f32,
    heart: f32,
    /// Purring, contentedly shut-eyed, for a while.
    purr: f32,
    alert: f32,
    bubble: Option<(String, f32)>,
    /// Seconds till the next blink, and how far through one it is.
    blink: f32,
    blinking: f32,
    /// Seconds before it'll leap again, speak again, call out by itself.
    calm: f32,
    hush: f32,
    call_in: f32,
    /// Bats at the ball so far, and how long it'll leave the ball alone.
    bats: u32,
    ball_cool: f32,
    /// Where it was held, and how high it was dropped from.
    grip: (f32, f32),
    dropped_from: f32,
    /// Just woken: a stretch and a yawn first.
    waking: bool,
    /// You're away (nobody's watching it closely).
    away: bool,
    /// What it does when it gets where it's walking (lie down by a friend, nap on its bed).
    after: Option<(Act, f32)>,
    /// The clock its picture moves by: stepped while it's calm, so the same few pictures come
    /// round again and an unchanged one isn't painted at all.
    frame_clock: f32,
    pub rig: Rig,
    look: (f32, f32),
    events: Vec<Event>,
    chance: Chance,
}

/// What it says, by animal and mood.
fn words(species: Species, mood: Mood, pick: usize) -> (Voice, &'static str) {
    let pick_of = |list: &[&'static str]| list[pick % list.len()];
    match (species, mood) {
        (Species::Cat, Mood::Call) => (Voice::Meow, pick_of(&["Meow", "Meow!", "Mew?", "Mrrrow"])),
        (Species::Cat, Mood::Hello) => (Voice::Trill, pick_of(&["Mrrp?", "Prrt!", "Mrrp"])),
        (Species::Cat, Mood::Content) => (Voice::Purr, pick_of(&["Purr…", "Prrrrr…"])),
        (Species::Cat, Mood::Complain) => (Voice::Complain, pick_of(&["Mrrow!", "Mrrrowl!"])),
        (Species::Fox, Mood::Content) => (Voice::Yip, pick_of(&["Heh-heh…", "Mrr…"])),
        (Species::Fox, Mood::Complain) => (Voice::Yip, pick_of(&["Yiiip!", "Ack!"])),
        (Species::Fox, _) => (Voice::Yip, pick_of(&["Yip!", "Yip yip!", "Yip?"])),
        (Species::Bird, _) => (Voice::Chirp, pick_of(&["Tweet!", "Chirp chirp!", "Pip?", "Tsip!"])),
    }
}

impl Pet {
    pub fn new(species: Species, x: f32, y: f32, mut chance: Chance) -> Pet {
        let act = if species == Species::Bird { Act::Fly } else { Act::Sit };
        let call_in = 60.0 + chance.unit() * 120.0;
        let rig = pose(species, act, 0.0, 0.0, None);
        Pet {
            species,
            x,
            y,
            vx: 0.0,
            vy: 0.0,
            facing: -1.0,
            act,
            left: 2.0,
            target: (x, y),
            stride: 0.0,
            clock: 0.0,
            energy: 0.9,
            hunger: 0.2,
            lonely: 0.2,
            heart: 0.0,
            purr: 0.0,
            alert: 0.0,
            bubble: None,
            blink: 3.0,
            blinking: 0.0,
            calm: 0.0,
            hush: 0.0,
            call_in,
            bats: 0,
            ball_cool: 0.0,
            grip: (0.0, 0.0),
            dropped_from: 0.0,
            waking: false,
            away: false,
            after: None,
            frame_clock: 0.0,
            rig,
            look: (0.0, 0.0),
            events: Vec::new(),
            chance,
        }
    }

    /// Every pose it has, still, for a snapshot; the last with a heart and words.
    pub fn poses(species: Species) -> Vec<Pet> {
        let acts: &[Act] = if species == Species::Bird {
            &[Act::Stand, Act::Fly, Act::Peck, Act::Groom, Act::Sleep, Act::Stand]
        } else {
            &[Act::Stand, Act::Walk, Act::Run, Act::Sit, Act::Loaf, Act::Lie, Act::Groom, Act::Stretch, Act::Yawn, Act::Crouch, Act::Pounce, Act::Bat, Act::Eat, Act::Held, Act::Sleep, Act::Sit]
        };
        acts.iter()
            .enumerate()
            .map(|(i, act)| {
                let mut pet = Pet::new(species, 0.0, 0.0, Chance::from_clock());
                pet.facing = 1.0;
                pet.act = *act;
                pet.clock = 0.4 * i as f32;
                pet.rig = pose(species, *act, 0.15 + 0.1 * i as f32, pet.clock, None);
                pet.look = (0.6, 0.2);
                if i == acts.len() - 1 {
                    pet.heart = 1.0;
                    pet.purr = 2.0;
                    pet.rig.eyes = 0.15;
                    pet.bubble = Some((words(species, Mood::Content, 0).1.to_string(), 2.0));
                }
                pet
            })
            .collect()
    }

    /// Says something (a sound, and the words in a bubble), unless it just did.
    fn say(&mut self, mood: Mood) {
        if self.hush > 0.0 {
            return;
        }
        let pick = self.chance.below(8) as usize;
        let (voice, text) = words(self.species, mood, pick);
        self.events.push(Event::Say(voice, (pick % sound::VARIANTS as usize) as u32));
        self.bubble = Some((text.to_string(), if mood == Mood::Content { 2.6 } else { 2.0 }));
        self.hush = 3.0;
    }

    /// Stroked (a click): it purrs, shut-eyed, a heart over it; woken, it trills first.
    pub fn petted(&mut self) {
        self.heart = 1.8;
        self.lonely = (self.lonely - 0.35).max(0.0);
        if self.act == Act::Sleep {
            self.act = Act::Sit;
            self.left = 3.0;
            self.say(Mood::Hello);
            return;
        }
        if matches!(self.act, Act::Walk | Act::Run | Act::Stand) {
            self.act = if self.species.grounded() { Act::Sit } else { Act::Stand };
            self.left = 4.0;
        }
        if self.purr <= 0.0 {
            self.say(Mood::Content);
        }
        self.purr = 3.5;
    }

    /// A notification came: ears up, a look toward the corner they pop up in.
    pub fn notice(&mut self) {
        if !matches!(self.act, Act::Sleep | Act::Held) {
            self.alert = 2.2;
            self.facing = 1.0;
            if self.chance.unit() < 0.3 {
                self.say(Mood::Hello);
            }
        }
    }

    /// Picked up at (x, y) on the output.
    pub fn grab(&mut self, x: f32, y: f32) {
        self.grip = (self.x - x, self.y - y);
        self.act = Act::Held;
        self.purr = 0.0;
        if self.chance.unit() < 0.5 {
            self.say(Mood::Hello);
        }
    }

    /// Carried to (x, y).
    pub fn carry(&mut self, x: f32, y: f32, world: &World) {
        if self.act == Act::Held {
            self.x = (x + self.grip.0).clamp(30.0, world.size.0 - 30.0);
            self.y = (y + self.grip.1).clamp(40.0, world.floor);
        }
    }

    /// Let go: it falls, and lands on its feet (the bird flies off).
    pub fn drop_down(&mut self, world: &World) {
        if self.act != Act::Held {
            return;
        }
        self.dropped_from = world.floor - self.y;
        if self.species.grounded() {
            self.act = Act::Fall;
            self.vy = 0.0;
        } else {
            self.act = Act::Fly;
            self.left = 0.0;
        }
    }

    /// How long until the next moment is worth drawing: smooth when it moves fast, a few frames
    /// a second while it sits (breathing, a tail tip), fewer asleep, fewer still when you're
    /// away. A frame costs most of a millisecond, so this is most of what the pet costs.
    pub fn delay(&self) -> std::time::Duration {
        let calm = self.heart <= 0.0 && self.purr <= 0.0;
        let ms = match self.act {
            Act::Sleep if self.away => 500,
            Act::Sleep => 200,
            Act::Sit | Act::Loaf | Act::Lie | Act::Groom | Act::Stand | Act::Eat | Act::Peck if calm => 83,
            Act::Walk | Act::Hop => 50,
            _ => 33,
        };
        std::time::Duration::from_millis(ms)
    }

    /// A moment later, `dt` seconds; returns what happened.
    pub fn step(&mut self, dt: f32, world: &World) -> Vec<Event> {
        let dt = dt.clamp(0.0, 0.6);
        self.away = world.idle;
        self.clock += dt;
        for timer in [&mut self.heart, &mut self.purr, &mut self.alert, &mut self.calm, &mut self.hush, &mut self.ball_cool] {
            *timer = (*timer - dt).max(0.0);
        }
        if let Some((_, left)) = &mut self.bubble {
            *left -= dt;
            if *left <= 0.0 {
                self.bubble = None;
            }
        }
        // Needs: tiring while awake (a nap puts it right), hungry over hours, lonely over minutes
        let asleep = self.act == Act::Sleep;
        self.energy = if asleep { (self.energy + dt / 90.0).min(1.0) } else { (self.energy - dt / 1500.0).max(0.0) };
        self.hunger = (self.hunger + dt / 7200.0).min(1.0);
        if !asleep && !world.idle {
            self.lonely = (self.lonely + dt / 900.0).min(1.0);
        }
        // Blinking
        self.blink -= dt;
        if self.blink <= 0.0 {
            self.blinking = 0.16;
            self.blink = 2.5 + 4.0 * self.chance.unit();
        }
        self.blinking = (self.blinking - dt).max(0.0);

        self.live(dt, world);
        self.move_on(dt, world);
        // (While you're away, `live` settles it down instead)
        if self.left <= 0.0 && !world.idle && !matches!(self.act, Act::Pounce | Act::Held | Act::Fall) {
            self.choose(world);
        }

        // It speaks by itself now and then: more often hungry or lonely, never asleep
        self.call_in -= dt;
        if self.call_in <= 0.0 {
            let needy = self.hunger > 0.7 || self.lonely > 0.75;
            self.call_in = if needy { 40.0 + self.chance.unit() * 50.0 } else { 150.0 + self.chance.unit() * 240.0 };
            if !matches!(self.act, Act::Sleep | Act::Held | Act::Eat) && !world.idle {
                let mood = if needy || self.chance.unit() < 0.6 { Mood::Call } else { Mood::Hello };
                self.say(mood);
            }
        }

        let (w, _) = world.size;
        if self.act != Act::Held {
            self.x = self.x.clamp(40.0, (w - 40.0).max(40.0));
            self.y = self.y.min(world.floor).max(40.0);
        }

        // Its eyes on what it's watching: the pointer, the ball, the bowl
        let watch = world.pointer.or(world.ball.map(|(x, _)| (x, world.floor))).or(world.bowl.map(|x| (x, world.floor)));
        let look = watch.map_or((0.0, 0.0), |(px, py)| {
            let (dx, dy) = ((px - self.x) * self.facing, py - (self.y - 35.0));
            let d = dx.hypot(dy).max(1.0);
            (dx / d, dy / d)
        });
        self.look = (self.look.0 + (look.0 - self.look.0) * (dt * 8.0).min(1.0), self.look.1 + (look.1 - self.look.1) * (dt * 8.0).min(1.0));
        // In tenths: the eyes settle rather than creep
        self.look = ((self.look.0 * 10.0).round() / 10.0, (self.look.1 * 10.0).round() / 10.0);

        // The body follows the pose smoothly; walking feet follow the stride directly
        // Calm, its picture moves in steps (six a second, four asleep): breathing and a tail tip
        // read just as well, and between steps the picture is the same, so it isn't painted
        let calm = matches!(self.act, Act::Sit | Act::Loaf | Act::Lie | Act::Sleep | Act::Stand) && world.beat.is_none();
        self.frame_clock = if calm {
            let steps = if self.act == Act::Sleep { 4.0 } else { 6.0 };
            (self.clock * steps).floor() / steps
        } else {
            self.clock
        };
        let mut target = pose(self.species, self.act, self.stride, self.frame_clock, world.beat);
        if self.alert > 0.0 {
            target.ears = 1.0;
        }
        if self.purr > 0.0 {
            target.eyes = 0.15;
        }
        if self.blinking > 0.0 {
            target.eyes = 0.0;
        }
        let k = 1.0 - (-dt * 10.0).exp();
        let moving = matches!(self.act, Act::Walk | Act::Run | Act::Pounce | Act::Fly | Act::Hop);
        self.rig = self.rig.lerp(&target, k, if moving { 1.0 } else { 1.0 - (-dt * 14.0).exp() });
        // Close enough is there: a pose that has arrived stops changing
        if self.rig.distance(&target) < 0.08 {
            self.rig = target;
        }
        std::mem::take(&mut self.events)
    }

    /// What it decides to do this moment, besides finishing what it's at.
    fn live(&mut self, dt: f32, world: &World) {
        let busy_with = matches!(self.act, Act::Held | Act::Fall | Act::Pounce | Act::Eat | Act::Bat | Act::Stretch | Act::Yawn);

        // You're away: it settles down to sleep (the bird lands first). Back: it wakes slowly
        if world.idle {
            // To its bed first, if it has one
            if let Some(bed) = world.bed
                && self.species.grounded()
                && (self.x - bed).abs() > 6.0
                && !matches!(self.act, Act::Held | Act::Fall | Act::Walk)
            {
                self.go(Act::Walk, bed, world.floor);
                self.after = Some((Act::Lie, 2.0));
                return;
            }
            if self.act == Act::Walk {
                return;
            }
            if !matches!(self.act, Act::Sleep | Act::Held | Act::Fall) {
                if self.species == Species::Bird && self.y < world.floor - 1.0 && world.perch.is_none_or(|p| (self.y - p).abs() > 1.0) {
                    self.act = Act::Fly;
                    self.target = (self.x, world.floor);
                } else if self.species.grounded() && self.act != Act::Lie {
                    self.act = Act::Lie;
                    self.left = 3.0;
                } else if self.act == Act::Lie && self.left <= 0.0 || !self.species.grounded() {
                    self.act = Act::Sleep;
                }
            }
            if self.act == Act::Sleep {
                self.left = 10_000.0;
            }
            return;
        }
        if self.act == Act::Sleep && self.left > 1000.0 {
            // Woken by your return: a stretch, a yawn, a hello
            self.waking = true;
            self.act = if self.species.grounded() { Act::Stretch } else { Act::Stand };
            self.left = 1.6;
            return;
        }
        if busy_with || self.act == Act::Sleep {
            return;
        }

        // A friend running at it: a game of chase, it runs off
        if self.species.grounded()
            && self.energy > 0.3
            && let Some(friend) = world.friends.iter().find(|f| f.act == Act::Run && f.species != Species::Bird && (self.x - f.x).abs() < 170.0 && (self.x - f.x).signum() == f.facing)
            && self.act != Act::Run
        {
            let away = (self.x - friend.x).signum();
            let (w, _) = world.size;
            let spot = (self.x + away * 260.0).clamp(60.0, w - 60.0);
            // Cornered: back past it instead
            let spot = if (spot - self.x).abs() < 60.0 { (friend.x - away * 200.0).clamp(60.0, w - 60.0) } else { spot };
            self.go(Act::Run, spot, world.floor);
            self.after = Some((Act::Sit, 2.0));
            return;
        }

        // Food out and hungry at all: to the bowl
        if let Some(bowl) = world.bowl
            && self.hunger > 0.08
        {
            let side = if self.x < bowl { -1.0 } else { 1.0 };
            let spot = bowl + side * if self.species.grounded() { 24.0 } else { 12.0 };
            if (self.x - spot).abs() < 6.0 && (self.y - world.floor).abs() < 2.0 {
                self.facing = -side;
                self.act = if self.species.grounded() { Act::Eat } else { Act::Peck };
                self.left = 7.0;
            } else if self.species.grounded() {
                self.go(Act::Walk, spot, world.floor);
            } else {
                self.go(Act::Fly, spot, world.floor);
            }
            return;
        }

        // A ball rolling about: after it, a paw at it
        if let Some((ball, speed)) = world.ball
            && self.species.grounded()
            && self.energy > 0.15
            && self.ball_cool <= 0.0
        {
            let dx = ball - self.x;
            if dx.abs() < 26.0 && speed.abs() < 140.0 {
                self.facing = dx.signum();
                self.act = Act::Bat;
                self.left = 0.45;
                self.bats += 1;
                let strength = 220.0 + 260.0 * self.chance.unit();
                // Now and then the other way, the way a cat bats a toy back and forth
                let way = if self.chance.unit() < 0.25 { -1.0 } else { 1.0 };
                self.events.push(Event::Kick(self.facing * way * strength));
                if self.bats >= 6 + self.chance.below(5) as u32 {
                    self.bats = 0;
                    self.ball_cool = 60.0;
                    self.events.push(Event::Bored);
                }
                self.lonely = (self.lonely - 0.05).max(0.0);
                self.energy = (self.energy - 0.01).max(0.0);
            } else if dx.abs() >= 26.0 {
                self.go(Act::Run, ball - dx.signum() * 18.0, world.floor);
            }
            return;
        }

        // The pointer: stalked and pounced on when it's low and near, followed when it's far,
        // watched when it's up high
        if let Some((px, py)) = world.pointer
            && (px - self.x).abs() < 520.0
            && self.energy > 0.2
        {
            let dx = px - self.x;
            if self.species.grounded() {
                // Low and near: stalked; otherwise run over and sit to watch it
                let low = py > world.floor - 150.0;
                if low && dx.abs() < 160.0 && self.calm <= 0.0 {
                    if self.act != Act::Crouch {
                        self.facing = dx.signum();
                        self.act = Act::Crouch;
                        self.left = 0.8 + 0.9 * self.chance.unit();
                    }
                } else if dx.abs() > 90.0 {
                    self.go(Act::Run, px - dx.signum() * 60.0, world.floor);
                } else if matches!(self.act, Act::Walk | Act::Run | Act::Stand) {
                    self.facing = dx.signum();
                    self.act = Act::Sit;
                    self.left = 2.0;
                }
            } else if self.act != Act::Fly || (self.target.0 - px).abs() > 60.0 {
                // The bird comes down near it
                let spot = (px - dx.signum() * 50.0, if py < world.floor - 200.0 { world.perch.unwrap_or(world.floor) } else { world.floor });
                self.go(Act::Fly, spot.0, spot.1);
            }
            return;
        }

        // Lonely, and you're nowhere it can see: it calls (in `step`); hungry, the same
        let _ = dt;
    }

    /// Off somewhere, walking, running or flying.
    fn go(&mut self, act: Act, x: f32, y: f32) {
        self.act = act;
        self.target = (x, y);
        self.left = 20.0;
    }

    /// Movement for what it's doing.
    fn move_on(&mut self, dt: f32, world: &World) {
        let fast = world.busy > 0.75;
        match self.act {
            Act::Held => {
                self.stride += dt * 3.0;
            }
            Act::Fall => {
                self.vy += 1100.0 * dt;
                self.y += self.vy * dt;
                if self.y >= world.floor {
                    self.y = world.floor;
                    self.vy = 0.0;
                    self.act = Act::Sit;
                    self.left = 2.5;
                    if self.dropped_from > 260.0 {
                        self.say(Mood::Complain);
                    }
                }
            }
            Act::Walk | Act::Run => {
                let speed = match (self.act, fast) {
                    (Act::Run, _) => 200.0,
                    (_, true) => 120.0,
                    _ => 42.0,
                };
                let dx = self.target.0 - self.x;
                if dx.abs() < 3.0 {
                    (self.act, self.left) = self.after.take().unwrap_or((Act::Stand, 1.0 + 2.0 * self.chance.unit()));
                } else {
                    self.facing = dx.signum();
                    let step = (speed * dt).min(dx.abs());
                    self.x += step * self.facing;
                    // Legs go round once every ~34 pixels walking, ~60 running (a longer reach)
                    self.stride += step / if self.act == Act::Run { 60.0 } else { 34.0 };
                }
                self.y = world.floor;
                self.left -= dt;
            }
            Act::Crouch => {
                self.left -= dt;
                if self.left <= 0.0 {
                    // The leap
                    self.act = Act::Pounce;
                    self.vy = -320.0;
                    let reach = world.pointer.map_or(90.0, |(px, _)| (px - self.x).abs().clamp(40.0, 150.0));
                    self.vx = self.facing * reach / 0.6;
                }
            }
            Act::Pounce => {
                self.vy += 1100.0 * dt;
                self.y += self.vy * dt;
                self.x += self.vx * dt;
                if self.y >= world.floor {
                    self.y = world.floor;
                    self.vy = 0.0;
                    self.act = Act::Sit;
                    self.left = 1.5;
                    self.calm = 4.0;
                    self.energy = (self.energy - 0.02).max(0.0);
                    self.lonely = (self.lonely - 0.08).max(0.0);
                }
            }
            Act::Fly => {
                let (dx, dy) = (self.target.0 - self.x, self.target.1 - self.y);
                let distance = dx.hypot(dy);
                let speed = if fast { 260.0 } else { 150.0 };
                self.stride += dt;
                if distance < 3.0 {
                    let landed = world.perch.into_iter().chain([world.floor]).any(|p| (self.y - p).abs() < 2.0);
                    if landed {
                        self.act = if world.idle { Act::Sleep } else { Act::Stand };
                        self.left = 2.0 + 4.0 * self.chance.unit();
                    } else {
                        self.left = 0.0;
                    }
                } else {
                    if dx.abs() > 1.0 {
                        self.facing = dx.signum();
                    }
                    let step = (speed * dt).min(distance);
                    self.x += dx / distance * step;
                    self.y += dy / distance * step;
                }
                self.left -= dt;
            }
            Act::Hop => {
                // Small hops along where it's perched
                self.vy += 900.0 * dt;
                self.y += self.vy * dt;
                self.x += self.facing * 70.0 * dt;
                let ground = world.perch.filter(|p| (self.target.1 - p).abs() < 2.0).unwrap_or(world.floor);
                if self.y >= ground {
                    self.y = ground;
                    self.left -= 1.0;
                    if self.left > 0.0 {
                        self.vy = -170.0;
                    } else {
                        self.act = Act::Stand;
                        self.left = 1.0 + self.chance.unit() * 2.0;
                    }
                }
            }
            Act::Eat | Act::Peck => {
                self.left -= dt;
                if self.left <= 0.0 {
                    self.hunger = 0.0;
                    self.events.push(Event::Ate);
                    self.act = if self.species.grounded() { Act::Sit } else { Act::Stand };
                    self.left = 2.0;
                    self.say(Mood::Hello);
                }
            }
            Act::Stretch | Act::Yawn => {
                self.left -= dt;
                if self.left <= 0.0 && self.waking {
                    if self.act == Act::Stretch {
                        self.act = Act::Yawn;
                        self.left = 1.3;
                    } else {
                        self.waking = false;
                        self.act = Act::Sit;
                        self.left = 2.0;
                        self.say(Mood::Hello);
                    }
                }
            }
            _ => {
                self.left -= dt;
                if self.species.grounded() {
                    self.y = world.floor;
                }
            }
        }
    }

    fn choose(&mut self, world: &World) {
        let (w, h) = world.size;
        let roll = self.chance.unit();
        let anywhere = 60.0 + self.chance.unit() * (w - 120.0).max(1.0);
        if !self.species.grounded() {
            self.left = 2.0 + 3.0 * self.chance.unit();
            let on = |p: f32| (self.y - p).abs() < 2.0;
            let perched = on(world.floor) || world.perch.is_some_and(on);
            self.act = if !perched || roll < 0.35 {
                let air = 0.15 * h + self.chance.unit() * (world.floor - 80.0 - 0.15 * h).max(1.0);
                let pick = self.chance.unit();
                self.target = if pick < 0.35 { (anywhere, air) } else if pick < 0.7 { (anywhere, world.perch.unwrap_or(world.floor)) } else { (anywhere, world.floor) };
                self.left = 15.0;
                Act::Fly
            } else if roll < 0.55 {
                self.facing = if self.chance.unit() < 0.5 { -1.0 } else { 1.0 };
                self.target = (self.x, self.y);
                self.vy = -170.0;
                self.left = 2.0 + self.chance.below(3) as f32;
                Act::Hop
            } else if roll < 0.7 {
                Act::Peck
            } else if roll < 0.82 {
                Act::Groom
            } else if self.energy < 0.4 && roll < 0.9 {
                self.left = 20.0;
                Act::Sleep
            } else {
                Act::Stand
            };
            return;
        }
        // Tired: to its bed if it has one, lie down, then a nap
        if self.energy < 0.25 {
            if let Some(bed) = world.bed
                && (self.x - bed).abs() > 6.0
            {
                self.go(Act::Walk, bed, world.floor);
                self.after = Some((Act::Lie, 4.0));
                return;
            }
            (self.act, self.left) = if self.act == Act::Lie { (Act::Sleep, 30.0 + 40.0 * self.chance.unit()) } else { (Act::Lie, 4.0) };
            return;
        }
        // A friend about: go and say hello, start a chase, or lie down by it while it sleeps
        if let Some(friend) = world.friends.iter().filter(|f| f.species != Species::Bird).min_by(|a, b| (a.x - self.x).abs().total_cmp(&(b.x - self.x).abs())).copied()
            && roll < 0.3
        {
            let side = if self.x < friend.x { -1.0 } else { 1.0 };
            let beside = (friend.x + side * 46.0).clamp(60.0, w - 60.0);
            if friend.act == Act::Sleep {
                self.go(Act::Walk, beside, world.floor);
                self.after = Some((Act::Sleep, 30.0 + 30.0 * self.chance.unit()));
            } else if roll < 0.12 && self.energy > 0.5 && (friend.x - self.x).abs() > 80.0 {
                self.go(Act::Run, friend.x, world.floor);
                self.after = Some((Act::Stand, 1.0));
            } else if (friend.x - self.x).abs() > 70.0 {
                self.go(Act::Walk, beside, world.floor);
                self.after = Some((Act::Sit, 4.0));
                if self.chance.unit() < 0.4 {
                    self.say(Mood::Hello);
                }
            } else {
                self.facing = -side;
                self.act = Act::Sit;
                self.left = 4.0;
            }
            return;
        }
        // Lonely or hungry: it comes over to the middle of the screen to ask
        if (self.hunger > 0.7 || self.lonely > 0.8) && roll < 0.3 {
            let spot = w / 2.0 + (self.chance.unit() - 0.5) * 200.0;
            self.go(Act::Walk, spot, world.floor);
            return;
        }
        // The machine racing: the zoomies, back and forth
        if world.busy > 0.75 && roll < 0.35 {
            self.go(Act::Run, anywhere, world.floor);
            return;
        }
        // Music: it sits and sways with it
        if world.beat.is_some() && roll < 0.4 {
            self.act = Act::Sit;
            self.left = 6.0;
            return;
        }
        let r = self.chance.unit();
        (self.act, self.left) = if r < 0.22 {
            (Act::Sit, 4.0 + 6.0 * self.chance.unit())
        } else if r < 0.46 {
            self.target = (anywhere, world.floor);
            (Act::Walk, 30.0)
        } else if r < 0.58 {
            (Act::Loaf, 8.0 + 10.0 * self.chance.unit())
        } else if r < 0.7 {
            (Act::Groom, 3.5 + 3.0 * self.chance.unit())
        } else if r < 0.78 {
            (Act::Lie, 6.0 + 8.0 * self.chance.unit())
        } else if r < 0.84 {
            (Act::Stretch, 1.6)
        } else if r < 0.9 {
            (Act::Yawn, 1.3)
        } else if r < 0.96 {
            (Act::Stand, 2.0 + 2.0 * self.chance.unit())
        } else {
            (Act::Sleep, 15.0 + 25.0 * self.chance.unit())
        };
        if self.act == Act::Walk {
            self.left = 30.0;
        }
        // A nap is taken on its bed, if it has one
        if self.act == Act::Sleep
            && let Some(bed) = world.bed
            && (self.x - bed).abs() > 6.0
        {
            let nap = self.left;
            self.go(Act::Walk, bed, world.floor);
            self.after = Some((Act::Sleep, nap));
        }
    }

    /// Its picture on its surface, with what's round it.
    pub fn draw(&self, t: &Theme, coat: &draw::Coat, size: f32, floor: f32) -> Piece {
        let mut piece = Piece::new("pet", W, H);
        // A songbird drawn true to size next to a cat would be a speck
        let fr = Frame { feet: FEET, f: self.facing, s: if self.species == Species::Bird { size * 1.35 } else { size } };
        let extras = Extras {
            heart: self.heart,
            alert: self.alert > 0.0,
            asleep: self.act == Act::Sleep,
            happy: self.purr > 0.0 && self.act != Act::Sleep,
            bubble: self.bubble.as_ref().map(|(text, left)| (text.as_str(), (*left / 0.4).min(1.0))),
            ground: ((floor - self.y) / size).max(0.0),
            look: self.look,
        };
        let mut prims: Vec<Prim> = Vec::new();
        match self.species {
            Species::Cat | Species::Fox => draw::quadruped(&mut prims, &fr, &self.rig, coat, self.species == Species::Fox, self.frame_clock, &extras),
            Species::Bird => draw::bird(&mut prims, &fr, &self.rig, coat, self.frame_clock, &extras),
        }
        let top = (FEET.0 + self.rig.head.0 * self.facing * size, FEET.1 + (self.rig.head.1 - 10.0) * size);
        draw::extras(&mut prims, t, top, (W, H), &extras, self.frame_clock);
        piece.prims = prims;
        piece
    }
}

/// The pose for what it's doing (in the animal's coordinates; see `draw::Rig`), moving with its
/// stride and the clock.
fn pose(species: Species, act: Act, stride: f32, clock: f32, beat: Option<f32>) -> Rig {
    let tau = std::f32::consts::TAU;
    let pi = std::f32::consts::PI;
    if species == Species::Bird {
        let mut r = Rig {
            hip: (0.0, -9.0),
            shoulder: (0.0, -9.0),
            head: (6.5, -16.0),
            tilt: 0.0,
            feet: [(-1.5, 0.0), (1.5, 0.0), (0.0, 0.0), (0.0, 0.0)],
            tail: pi + 0.35,
            curl: 0.0,
            sway: 0.0,
            ears: 1.0,
            eyes: 1.0,
            mouth: 0.0,
            tuck: 0.0,
            ball: 0.0,
            wing: 0.0,
        };
        match act {
            Act::Fly | Act::Held => {
                r.hip = (0.0, -16.0);
                r.head = (8.0, -21.0);
                r.wing = 1.0;
                r.tail = pi + 0.1;
            }
            Act::Peck | Act::Eat => {
                let bob = (clock * 9.0).sin().max(0.0);
                r.head = (8.0, -11.0 + 7.0 * bob);
                r.tilt = 0.5 + 0.4 * bob;
                r.tail = pi + 0.6;
            }
            Act::Groom => {
                r.head = (3.0, -14.0);
                r.tilt = 0.9 + 0.2 * (clock * 7.0).sin();
            }
            Act::Sleep => {
                r.hip = (0.0, -8.0);
                r.head = (4.5, -13.0);
                r.eyes = 0.0;
                r.ball = 1.0;
                r.tail = pi + 0.45;
            }
            Act::Hop => {
                r.feet = [(-1.5, -2.0), (1.5, -2.0), (0.0, 0.0), (0.0, 0.0)];
            }
            _ => {
                // Perched: the head turning now and then, bobbing to music
                r.head.1 += beat.map_or(0.0, |b| -b * 2.5 * (clock * 8.0).sin().abs());
                r.tilt = 0.15 * (clock * 0.7).sin();
            }
        }
        return r;
    }

    // The cat's (and the fox's) standing pose, and the rest from it
    let mut r = Rig {
        hip: (-15.0, -19.0),
        shoulder: (14.0, -20.0),
        head: (26.0, -31.0),
        tilt: 0.0,
        feet: [(16.0, 0.0), (11.0, 0.0), (-12.0, 0.0), (-17.0, 0.0)],
        tail: -2.35,
        curl: 0.16,
        sway: 0.25,
        ears: 0.85,
        eyes: 1.0,
        mouth: 0.0,
        tuck: 0.0,
        ball: 0.0,
        wing: 0.0,
    };
    let gait = |r: &mut Rig, reach: f32, lift: f32| {
        // Each foot round its own part of the stride: hind and front of a side half a stride
        // apart, the two sides a quarter
        let bases = [r.feet[0].0, r.feet[1].0, r.feet[2].0, r.feet[3].0];
        for (i, offset) in [0.25f32, 0.75, 0.0, 0.5].iter().enumerate() {
            let u = (stride + offset).fract();
            let (x, y) = if u < 0.38 {
                let k = u / 0.38;
                (bases[i] - reach + 2.0 * reach * k, -lift * (pi * k).sin())
            } else {
                let k = (u - 0.38) / 0.62;
                (bases[i] + reach - 2.0 * reach * k, 0.0)
            };
            r.feet[i] = (x, y);
        }
    };
    match act {
        Act::Stand => {
            r.head.1 += (clock * 0.8).sin() * 0.6;
            r.tilt = 0.08 * (clock * 0.5).sin();
        }
        Act::Walk => {
            gait(&mut r, 6.5, 3.5);
            let bob = (stride * tau * 2.0).sin();
            r.hip.1 += bob * 0.7;
            r.shoulder.1 += bob * 0.7;
            r.head.1 += bob * 0.9;
            r.tail = -2.15;
            r.curl = 0.2;
        }
        Act::Run => {
            gait(&mut r, 11.0, 6.0);
            let flex = (stride * tau).sin();
            r.hip = (-16.0 - flex * 3.0, -18.0 + flex.abs() * 1.5);
            r.shoulder = (16.0 + flex * 3.0, -18.0 + flex.abs() * 1.5);
            r.head = (29.0 + flex * 2.0, -27.0);
            r.tail = pi + 0.25;
            r.curl = 0.0;
            r.sway = 0.1;
            r.ears = 0.6;
        }
        Act::Sit => {
            r.hip = (-7.0, -10.0);
            r.shoulder = (5.0, -29.0);
            r.head = (9.0, -42.0 + beat.map_or(0.0, |b| -b * 3.0 * (clock * 8.0).sin().abs()));
            r.tilt = beat.map_or(0.0, |b| 0.12 * b * (clock * 4.0).sin());
            r.feet = [(9.0, 0.0), (6.0, 0.0), (-1.0, 0.0), (-4.0, 0.0)];
            r.tuck = 0.5;
            r.tail = pi - 0.05;
            r.curl = -0.2;
            r.sway = 0.35;
        }
        Act::Loaf => {
            r.hip = (-13.0, -10.0);
            r.shoulder = (10.0, -11.0);
            r.head = (21.0, -22.0);
            r.tuck = 1.0;
            r.tail = pi - 0.1;
            r.curl = -0.28;
            r.sway = 0.1;
            r.eyes = 0.75;
        }
        Act::Lie => {
            r.hip = (-14.0, -9.0);
            r.shoulder = (9.0, -9.5);
            r.head = (21.0, -20.0);
            r.feet = [(25.0, 0.0), (21.0, 0.0), (-8.0, 0.0), (-12.0, 0.0)];
            r.tuck = 0.5;
            r.tail = pi - 0.05;
            r.curl = -0.08;
            r.sway = 0.15;
            r.eyes = 0.55;
        }
        Act::Sleep => {
            r.ball = 1.0;
            r.eyes = 0.0;
            r.ears = 0.45;
            r.hip = (-11.0, -10.0);
            r.shoulder = (10.0, -10.0);
            r.head = (15.0, -10.0);
        }
        Act::Groom => {
            // Sitting, licking a raised paw, then washing the face with it
            r.hip = (-7.0, -10.0);
            r.shoulder = (5.0, -29.0);
            r.tuck = 0.5;
            let wash = (clock * 0.5).sin() > 0.0;
            let lick = (clock * 7.0).sin();
            r.feet = [if wash { (14.0, -32.0 + lick * 2.0) } else { (14.0, -24.0) }, (6.0, 0.0), (-1.0, 0.0), (-4.0, 0.0)];
            r.head = if wash { (11.0, -40.0) } else { (12.0, -34.0 + lick) };
            r.tilt = if wash { 0.15 } else { 0.5 };
            r.eyes = 0.3;
            r.tail = pi - 0.05;
            r.curl = -0.2;
        }
        Act::Stretch => {
            r.hip = (-15.0, -22.0);
            r.shoulder = (16.0, -11.0);
            r.head = (29.0, -14.0);
            r.tilt = -0.25;
            r.feet = [(33.0, 0.0), (29.0, 0.0), (-13.0, 0.0), (-18.0, 0.0)];
            r.tail = -1.9;
            r.curl = 0.08;
            r.eyes = 0.3;
            r.ears = 0.6;
        }
        Act::Yawn => {
            r.hip = (-7.0, -10.0);
            r.shoulder = (5.0, -29.0);
            r.head = (8.0, -43.0);
            r.tilt = -0.45;
            r.mouth = 1.0;
            r.eyes = 0.1;
            r.ears = 0.55;
            r.feet = [(9.0, 0.0), (6.0, 0.0), (-1.0, 0.0), (-4.0, 0.0)];
            r.tuck = 0.5;
            r.tail = pi - 0.05;
            r.curl = -0.2;
        }
        Act::Crouch => {
            // Low, eyes wide, the hind end wiggling, the tail tip twitching
            let wiggle = (clock * 20.0).sin();
            r.hip = (-15.0 + wiggle * 1.4, -13.0);
            r.shoulder = (13.0, -9.0);
            r.head = (24.0, -13.0);
            r.feet = [(18.0, 0.0), (14.0, 0.0), (-10.0, 0.0), (-15.0, 0.0)];
            r.tail = pi + 0.05;
            r.curl = 0.0;
            r.sway = 0.6;
            r.ears = 1.0;
        }
        Act::Pounce => {
            r.hip = (-18.0, -22.0);
            r.shoulder = (17.0, -26.0);
            r.head = (28.0, -33.0);
            r.tilt = -0.1;
            r.feet = [(31.0, -22.0), (27.0, -20.0), (-30.0, -14.0), (-27.0, -12.0)];
            r.tail = pi + 0.3;
            r.curl = 0.0;
            r.ears = 1.0;
        }
        Act::Bat => {
            let swipe = (clock * 14.0).sin();
            r.hip = (-10.0, -11.0);
            r.shoulder = (8.0, -22.0);
            r.head = (17.0, -30.0);
            r.tilt = 0.35;
            r.feet = [(22.0 + 6.0 * swipe, -5.0 - 4.0 * swipe.abs()), (9.0, 0.0), (-3.0, 0.0), (-7.0, 0.0)];
            r.tuck = 0.5;
            r.ears = 1.0;
            r.tail = pi;
            r.curl = -0.05;
            r.sway = 0.5;
        }
        Act::Eat => {
            let chew = (clock * 6.0).sin();
            r.head = (27.0, -10.0 + chew * 0.6);
            r.tilt = 0.75;
            r.tail = -2.5;
            r.curl = 0.2;
            r.eyes = 0.6;
        }
        Act::Held | Act::Fall => {
            // Dangling from the scruff, legs hanging, the tail swinging
            let swing = (clock * 2.0).sin();
            r.shoulder = (2.0, -36.0);
            r.hip = (-1.0 + swing * 2.0, -11.0);
            r.head = (6.0, -48.0);
            r.tilt = 0.1;
            let reach = if act == Act::Fall { 3.0 } else { 0.0 };
            r.feet = [(7.0, -16.0 + reach * 4.0), (3.0, -15.0 + reach * 4.0), (1.0 + swing, 7.0 - reach), (-3.0 + swing, 6.0 - reach)];
            r.tail = pi / 2.0 - 0.2 + swing * 0.3;
            r.curl = 0.05;
            r.sway = 0.2;
            r.ears = if act == Act::Fall { 1.0 } else { 0.35 };
        }
        _ => {}
    }
    // A fox carries its brush low and straight back, not up in a curl like a cat
    if species == Species::Fox && matches!(act, Act::Stand | Act::Walk | Act::Eat | Act::Stretch | Act::Groom) {
        r.tail = pi - 0.3;
        r.curl = 0.04;
        r.sway = 0.15;
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    fn world() -> World {
        World { size: (1920.0, 1200.0), floor: 1200.0, perch: Some(36.0), pointer: None, idle: false, busy: 0.1, beat: None, bowl: None, ball: None, bed: None, friends: Vec::new() }
    }

    #[test]
    fn runs_off_when_a_friend_chases_it() {
        let mut pet = Pet::new(Species::Cat, 1000.0, 1200.0, Chance::new(11));
        let mut w = world();
        // A fox running at it from the left
        w.friends = vec![Friend { x: 880.0, act: Act::Run, facing: 1.0, species: Species::Fox }];
        run(&mut pet, &w, 0.5);
        assert_eq!(pet.act, Act::Run, "fled");
        assert!(pet.x > 1000.0, "away from the fox, to {}", pet.x);
    }

    #[test]
    fn sleeps_on_its_bed() {
        // Away: it walks to its bed first, then sleeps there
        let mut pet = Pet::new(Species::Cat, 400.0, 1200.0, Chance::new(12));
        let mut w = world();
        w.bed = Some(900.0);
        w.idle = true;
        run(&mut pet, &w, 25.0);
        assert_eq!(pet.act, Act::Sleep);
        assert!((pet.x - 900.0).abs() < 8.0, "on the bed, at {}", pet.x);
        // Tired, the same
        let mut pet = Pet::new(Species::Fox, 1500.0, 1200.0, Chance::new(13));
        pet.energy = 0.1;
        let w = World { idle: false, ..w };
        run(&mut pet, &w, 30.0);
        assert!((pet.x - 900.0).abs() < 8.0 && matches!(pet.act, Act::Lie | Act::Sleep), "{:?} at {}", pet.act, pet.x);
    }

    #[test]
    fn naps_beside_a_sleeping_friend() {
        let mut w = world();
        w.friends = vec![Friend { x: 1300.0, act: Act::Sleep, facing: 1.0, species: Species::Cat }];
        // Over enough tries one of its choices is the nap together
        let napped = (0..40).any(|seed| {
            let mut pet = Pet::new(Species::Cat, 700.0, 1200.0, Chance::new(seed));
            run(&mut pet, &w, 40.0);
            pet.act == Act::Sleep && (pet.x - 1300.0).abs() < 60.0
        });
        assert!(napped);
    }

    #[test]
    fn a_calm_pet_draws_the_same_picture_between_steps() {
        let theme = Theme::new(&crate::config::Config::default());
        let coat = draw::coat(Species::Cat, "", &theme);
        let mut pet = Pet::new(Species::Cat, 700.0, 1200.0, Chance::new(14));
        pet.act = Act::Loaf;
        pet.left = 100.0;
        let w = world();
        // Settled into the pose, then a fiftieth of a second apart (within one of six steps); no
        // blink meanwhile (a blink is a real change, and is drawn)
        run(&mut pet, &w, 3.0);
        pet.blink = 100.0;
        run(&mut pet, &w, 1.0);
        let hash = |pet: &Pet| pet.draw(&theme, &coat, 1.0, 1200.0).content_hash(1.0);
        let mut same = 0;
        for _ in 0..30 {
            let before = hash(&pet);
            pet.step(0.02, &w);
            if hash(&pet) == before {
                same += 1;
            }
        }
        assert!(same >= 20, "only {same} of 30 frames repeated");
    }

    fn run(pet: &mut Pet, world: &World, seconds: f32) -> Vec<Event> {
        let mut events = Vec::new();
        for _ in 0..(seconds * 30.0) as usize {
            events.extend(pet.step(1.0 / 30.0, world));
        }
        events
    }

    #[test]
    fn sleeps_while_you_are_away_and_wakes_with_a_stretch() {
        for species in [Species::Cat, Species::Fox, Species::Bird] {
            let mut pet = Pet::new(species, 900.0, 1200.0, Chance::new(1));
            let mut w = world();
            w.idle = true;
            run(&mut pet, &w, 25.0);
            assert_eq!(pet.act, Act::Sleep, "{species:?}");
            assert!(pet.y == w.floor || Some(pet.y) == w.perch, "{species:?} sleeps on the floor or a perch, not at {}", pet.y);
            w.idle = false;
            run(&mut pet, &w, 0.1);
            if species.grounded() {
                assert_eq!(pet.act, Act::Stretch, "{species:?} wakes with a stretch");
                let said = run(&mut pet, &w, 4.0);
                assert!(said.iter().any(|e| matches!(e, Event::Say(..))), "and says hello");
            } else {
                assert_ne!(pet.act, Act::Sleep);
            }
        }
    }

    #[test]
    fn stalks_and_pounces_on_the_pointer() {
        let mut pet = Pet::new(Species::Cat, 1400.0, 1200.0, Chance::new(3));
        let mut w = world();
        w.pointer = Some((1480.0, 1160.0));
        let mut crouched = false;
        let mut leapt = false;
        for _ in 0..150 {
            pet.step(1.0 / 30.0, &w);
            crouched |= pet.act == Act::Crouch;
            leapt |= pet.y < w.floor - 5.0;
        }
        assert!(crouched && leapt, "crouched {crouched}, leapt {leapt}");
        // Far off it follows; much further, it doesn't notice
        let mut pet = Pet::new(Species::Cat, 1000.0, 1200.0, Chance::new(3));
        w.pointer = Some((1400.0, 600.0));
        run(&mut pet, &w, 4.0);
        assert!((pet.x - 1400.0).abs() < 120.0, "followed to {}", pet.x);
        let mut far = Pet::new(Species::Cat, 100.0, 1200.0, Chance::new(3));
        far.step(0.03, &w);
        assert_ne!(far.act, Act::Run);
    }

    #[test]
    fn eats_from_the_bowl_and_bats_the_ball() {
        let mut pet = Pet::new(Species::Cat, 600.0, 1200.0, Chance::new(5));
        pet.hunger = 0.8;
        let mut w = world();
        w.bowl = Some(900.0);
        let events = run(&mut pet, &w, 20.0);
        assert!(events.contains(&Event::Ate), "ate");
        assert!(pet.hunger < 0.01, "full: {}", pet.hunger);
        w.bowl = None;
        w.ball = Some((pet.x + 120.0, 0.0));
        let events = run(&mut pet, &w, 6.0);
        assert!(events.iter().any(|e| matches!(e, Event::Kick(_))), "batted the ball");
    }

    #[test]
    fn purrs_when_petted_and_complains_when_dropped_from_high() {
        let mut pet = Pet::new(Species::Cat, 600.0, 1200.0, Chance::new(6));
        let w = world();
        run(&mut pet, &w, 4.0);
        pet.petted();
        let said = run(&mut pet, &w, 0.1);
        assert!(pet.purr > 0.0);
        let _ = said;
        pet.hush = 0.0;
        pet.grab(600.0, 1150.0);
        pet.carry(700.0, 500.0, &w);
        assert_eq!(pet.act, Act::Held);
        pet.hush = 0.0;
        pet.drop_down(&w);
        let events = run(&mut pet, &w, 2.0);
        assert_eq!(pet.y, w.floor, "landed");
        assert!(events.iter().any(|e| matches!(e, Event::Say(Voice::Complain, _))), "complained: {events:?}");
    }

    #[test]
    fn stays_on_the_screen_and_speaks_now_and_then() {
        for species in [Species::Cat, Species::Fox, Species::Bird] {
            let mut pet = Pet::new(species, 960.0, 1200.0, Chance::new(9));
            let mut w = world();
            let mut said = 0;
            for i in 0..30 * 600 {
                w.busy = if i % 3000 < 900 { 0.9 } else { 0.1 };
                w.beat = (i % 2000 < 600).then_some(0.8);
                said += pet.step(1.0 / 30.0, &w).iter().filter(|e| matches!(e, Event::Say(..))).count();
                assert!(pet.x >= 40.0 && pet.x <= 1880.0 && pet.y >= 40.0 && pet.y <= w.floor, "{species:?} at {}, {}", pet.x, pet.y);
            }
            // Ten minutes: a few times, not a chatterbox
            assert!((1..=12).contains(&said), "{species:?} spoke {said} times");
        }
    }

    /// What a frame costs: a step, its picture, painted (run with --ignored --nocapture).
    #[test]
    #[ignore]
    fn frame_cost() {
        let theme = Theme::new(&crate::config::Config::default());
        let mut painter = crate::render::Painter::new("Google Sans Flex", "outline");
        for species in [Species::Cat, Species::Fox, Species::Bird] {
            let coat = draw::coat(species, "", &theme);
            let mut pet = Pet::new(species, 900.0, 1200.0, Chance::new(2));
            let mut w = world();
            w.pointer = Some((1300.0, 600.0));
            let frames = 300;
            let (mut stepping, mut building, mut painting) = (std::time::Duration::ZERO, std::time::Duration::ZERO, std::time::Duration::ZERO);
            let mut shapes = 0;
            for _ in 0..frames {
                let t = std::time::Instant::now();
                pet.step(1.0 / 30.0, &w);
                stepping += t.elapsed();
                let t = std::time::Instant::now();
                let piece = pet.draw(&theme, &coat, 1.0, 1200.0);
                building += t.elapsed();
                shapes += piece.prims.len();
                let mut canvas = tiny_skia::Pixmap::new(W as u32, H as u32).unwrap();
                let t = std::time::Instant::now();
                painter.draw(&mut canvas, &theme, &piece.prims, 0.0, 0.0, 1.0);
                painting += t.elapsed();
            }
            println!("{species:?}: step {:?}, build {:?}, paint {:?} a frame; {} shapes", stepping / frames, building / frames, painting / frames, shapes / frames as usize);
        }
    }

    #[test]
    fn draws_every_pose_in_every_coat() {
        let theme = Theme::new(&crate::config::Config::default());
        for species in [Species::Cat, Species::Fox, Species::Bird] {
            for name in draw::coats(species) {
                let coat = draw::coat(species, name, &theme);
                let mut pet = Pet::new(species, 500.0, 1200.0, Chance::new(4));
                for act in [Act::Stand, Act::Walk, Act::Run, Act::Sit, Act::Loaf, Act::Lie, Act::Sleep, Act::Groom, Act::Stretch, Act::Yawn, Act::Crouch, Act::Pounce, Act::Bat, Act::Eat, Act::Held, Act::Fly, Act::Hop, Act::Peck] {
                    pet.act = act;
                    pet.rig = pose(species, act, 0.3, 1.0, None);
                    let piece = pet.draw(&theme, &coat, 1.0, 1200.0);
                    assert!(piece.prims.len() > 8, "{species:?} {name} {act:?}");
                }
            }
        }
        assert_eq!(Species::from_name("Fox"), Some(Species::Fox));
        assert_eq!(Species::from_name("none"), None);
        assert_eq!(Species::Bird.next(), Species::Cat);
    }
}
