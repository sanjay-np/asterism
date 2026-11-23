//! The pets as the app runs them: each a small Overlay layer surface, above every window, moved
//! about the output by its margins as it goes; only its body takes the pointer, so clicks
//! everywhere else pass through. One timer moves them all on (quickly while they move, slowly
//! asleep). There can be as many as you like (`[pet] pets`), and they see each other.
//!
//! They see the pointer only where the shell does: over the wallpaper, the desktop's widgets, the
//! bar and the pets themselves (`saw_pointer`). They sleep after a minute without input (an idle
//! notification of their own, apart from the lock's). A click pats one; a drag picks it up (it
//! follows the pointer's own motion, see `PetSurface::at`) and it lands when let go.
//! Their props, a bowl of food, a ball, yarn and a bed, are small surfaces of their own on the
//! floor, on the Top layer: always under the pets, which are on Overlay.

use super::*;
use crate::pet::{Event, FEET, Friend, H, Pet, Species, W, World};
use smithay_client_toolkit::reexports::protocols::ext::idle_notify::v1::client::ext_idle_notification_v1::{self, ExtIdleNotificationV1};

/// Which pet: its place in `[pet] pets` (from 0).
pub(super) type Slot = usize;

pub(super) struct PetSurface {
    pub slot: Slot,
    pub surf: Surf,
    pub layer: LayerSurface,
    pub output: wl_output::WlOutput,
    pub pet: Pet,
    /// When it was last moved on.
    pub stepped: Instant,
    /// The margins last given to the compositor (left, top).
    pub placed: Option<(i32, i32)>,
    /// A press on it: where (on the output), and whether it's become a drag.
    pub press: Option<((f32, f32), bool)>,
    /// During a press, where the pointer is on the output: kept by the pointer's own motion
    /// (relative-pointer), since positions on the pet's surface lag behind its moves while it's
    /// carried, and reading them made it run off (to the top of the screen).
    pub at: (f32, f32),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum PropKind {
    Bowl,
    Ball,
    Yarn,
    Bed,
}

impl PropKind {
    fn size(self) -> (f32, f32) {
        match self {
            PropKind::Bed => (104.0, 40.0),
            PropKind::Yarn => (64.0, 48.0),
            _ => (48.0, 48.0),
        }
    }

    fn toy(self) -> bool {
        matches!(self, PropKind::Ball | PropKind::Yarn)
    }
}

/// A bowl, a toy or a bed on the floor: a small surface of its own.
pub(super) struct Prop {
    pub surf: Surf,
    pub layer: LayerSurface,
    pub kind: PropKind,
    pub x: f32,
    /// Rolling (a toy): speed, and a little hop when it's batted.
    pub vx: f32,
    pub lift: f32,
    pub vy: f32,
    pub placed: Option<(i32, i32)>,
    /// Gone at this time (an empty bowl, a toy nobody's played with).
    pub until: Option<Instant>,
}

/// After this long without input, the pets go to sleep.
const NAP_AFTER_MS: u32 = 60_000;

pub struct PetIdle;

impl Dispatch2<ExtIdleNotificationV1, App> for PetIdle {
    fn event(&self, app: &mut App, _: &ExtIdleNotificationV1, event: ext_idle_notification_v1::Event, _: &Connection, _: &QueueHandle<App>) {
        app.pet_idle = matches!(event, ext_idle_notification_v1::Event::Idled);
        log::debug!("pet: {}", if app.pet_idle { "you're away, to sleep" } else { "you're back" });
    }
}

impl App {
    /// Which animal and coat a slot is set to.
    fn pet_wanted(&self, slot: Slot) -> (Option<Species>, String) {
        let entries = self.config.pet.entries();
        let (animal, coat) = entries.get(slot).map(|e| crate::config::pet_entry(e)).unwrap_or_default();
        (Species::from_name(&animal), coat)
    }

    /// How many pets the config asks for (entries naming an animal there is).
    pub(super) fn pets_wanted(&self) -> usize {
        self.config.pet.entries().iter().filter(|e| Species::from_name(&crate::config::pet_entry(e).0).is_some()).count()
    }

    /// The pets follow the config: come, go, or turn into other animals; the bed too. A pet on
    /// the screen stays itself (where it is, how it feels) as long as there's an entry for its
    /// animal, even when others before it in the list come or go.
    pub(super) fn sync_pet(&mut self) {
        let wanted: Vec<(Slot, Species)> = self.config.pet.entries().iter().enumerate().filter_map(|(slot, e)| Some((slot, Species::from_name(&crate::config::pet_entry(e).0)?))).collect();
        let shown: Vec<(Slot, Species)> = self.pets.iter().map(|p| (p.slot, p.pet.species)).collect();
        let (fates, new) = crate::pet::match_pets(&shown, &wanted);
        let mut turned = Vec::new();
        // From the back, so taking one away leaves the places before it as they were
        for (index, fate) in fates.into_iter().enumerate().rev() {
            match fate {
                None => {
                    let gone = self.pets.remove(index);
                    self.painter.forget_surface(gone.surf.id);
                }
                Some((slot, species)) => {
                    let shown = &mut self.pets[index];
                    shown.slot = slot;
                    if shown.pet.species != species {
                        let (x, y) = (shown.pet.x, shown.pet.y);
                        shown.pet = Pet::new(species, x, y, crate::transition::Chance::from_clock());
                        turned.push(shown.surf.id);
                    }
                    // A new coat or size: drawn again
                    shown.surf.last = None;
                }
            }
        }
        // Another animal is another size: its region follows (found again, since pets before it
        // may have gone)
        for id in turned {
            if let Some(index) = self.pets.iter().position(|p| p.surf.id == id) {
                self.pet_region(index);
            }
        }
        for (slot, species) in new {
            self.add_pet(slot, species);
        }
        if self.pets.is_empty() {
            if let Some(watch) = self.pet_idle_watch.take() {
                watch.destroy();
            }
            for prop in std::mem::take(&mut self.props) {
                self.painter.forget_surface(prop.surf.id);
            }
        } else {
            if self.pet_idle_watch.is_none()
                && let (Some(notifier), Some(seat)) = (&self.idle_notifier, &self.seat)
            {
                self.pet_idle_watch = Some(notifier.get_idle_notification(NAP_AFTER_MS, seat, &self.qh, PetIdle));
            }
            self.sync_bed();
        }
        self.watch_spectrum();
    }

    /// A small surface on the output, at the top-left, moved by its margins.
    fn small_layer(&mut self, output: &wl_output::WlOutput, layer: Layer, namespace: &str, w: f32, h: f32) -> (Surf, LayerSurface) {
        let wl = self.compositor.create_surface(&self.qh);
        let mut surf = self.new_surf(wl.clone());
        if let Some(bar) = self.bars.iter().find(|b| &b.output == output) {
            surf.scale120 = bar.surf.scale120;
            surf.int_scale = bar.surf.int_scale;
        }
        let surface = self.layer_shell.create_layer_surface(&self.qh, wl, layer, Some(namespace), Some(output));
        surface.set_anchor(Anchor::TOP | Anchor::LEFT);
        surface.set_size(w as u32, h as u32);
        // Anywhere on the output, the bar's room too
        surface.set_exclusive_zone(-1);
        surface.set_keyboard_interactivity(KeyboardInteractivity::None);
        surface.commit();
        (surf, surface)
    }

    fn add_pet(&mut self, slot: Slot, species: Species) {
        let output = match self.pets.first() {
            Some(other) => other.output.clone(),
            None => match self.working_output() {
                Some(output) => output,
                None => return,
            },
        };
        let (w, h) = self.output_size(&output);
        let (surf, layer) = self.small_layer(&output, Layer::Overlay, "asterism-pet", W, H);
        let floor = self.pet_floor(h);
        // Spread out along the floor, each new one somewhere else (by the golden ratio)
        let x = w * (0.1 + 0.8 * (0.7 + slot as f32 * 0.618).fract());
        let pet = Pet::new(species, x, floor, crate::transition::Chance::from_clock());
        self.pets.push(PetSurface { slot, surf, layer, output, pet, stepped: Instant::now(), placed: None, press: None, at: (0.0, 0.0) });
        if !self.pet_timer {
            self.pet_timer = true;
            let _ = self.handle.insert_source(Timer::from_duration(Duration::from_millis(33)), |_, _, app| match app.step_pets() {
                Some(next) => TimeoutAction::ToDuration(next),
                None => {
                    app.pet_timer = false;
                    TimeoutAction::Drop
                }
            });
        }
    }

    pub(super) fn output_size(&self, output: &wl_output::WlOutput) -> (f32, f32) {
        self.outputs.info(output).and_then(|i| i.logical_size).map_or((1920.0, 1080.0), |(w, h)| (w as f32, h as f32))
    }

    /// Where feet stand: the foot of the screen, or just above the bar when it's down there.
    fn pet_floor(&self, h: f32) -> f32 {
        if self.theme.on_top { h - 2.0 } else { h - self.theme.reserve() - 2.0 }
    }

    /// The pointer was seen at (x, y) on this output.
    pub(super) fn saw_pointer(&mut self, output: &wl_output::WlOutput, x: f32, y: f32) {
        if self.pets.iter().any(|p| &p.output == output) {
            self.pet_pointer = Some(((x, y), Instant::now()));
        }
    }

    /// The screen as the pet in `index` sees it (the others are its friends).
    fn pet_world(&self, index: usize) -> World {
        let output = &self.pets[index].output;
        let (w, h) = self.output_size(output);
        let playing = self.model.media.player().is_some_and(|p| p.playing);
        let beat = (playing && !self.model.spectrum.is_empty()).then(|| {
            let low = &self.model.spectrum[..self.model.spectrum.len().min(8)];
            low.iter().sum::<f32>() / low.len() as f32
        });
        // The pointer counts for a moment after it was last seen
        let pointer = self.pet_pointer.filter(|(_, at)| at.elapsed() < Duration::from_millis(1500)).map(|(p, _)| p);
        let bowl = self.props.iter().find(|p| p.kind == PropKind::Bowl && p.until.is_none()).map(|p| p.x);
        let ball = self.props.iter().filter(|p| p.kind.toy()).min_by(|a, b| (a.x - self.pets[index].pet.x).abs().total_cmp(&(b.x - self.pets[index].pet.x).abs())).map(|p| (p.x, p.vx));
        let bed = self.props.iter().find(|p| p.kind == PropKind::Bed).map(|p| p.x);
        let friends = self
            .pets
            .iter()
            .enumerate()
            .filter(|(i, p)| *i != index && &p.output == output)
            .map(|(_, p)| Friend { x: p.pet.x, act: p.pet.act, facing: p.pet.facing, species: p.pet.species })
            .collect();
        World { size: (w, h), floor: self.pet_floor(h), perch: self.theme.on_top.then(|| self.theme.reserve()), pointer, idle: self.pet_idle, busy: self.model.machine.cpu, beat, bowl, ball, bed, friends }
    }

    /// Every pet's moment: moved on, its props too, placed, drawn. Returns when to come back,
    /// or None when there are no pets.
    /// Each pet moves on by its own clock (a sleeping one a few times a second, a running one
    /// thirty), the props by theirs: one busy pet, or a rolling ball, doesn't make every pet
    /// step and draw as often as it does. The timer comes back when the next is due.
    fn step_pets(&mut self) -> Option<Duration> {
        if self.pets.is_empty() {
            return None;
        }
        let now = Instant::now();
        let mut next = Duration::from_secs(1);
        for index in 0..self.pets.len() {
            let since = now.saturating_duration_since(self.pets[index].stepped);
            let delay = self.pets[index].pet.delay();
            // Up to a third early is on time: pets nearly due go with the one that is, so several
            // pets don't each wake the shell on their own (held, it follows the pointer every time)
            if since + delay / 3 < delay && self.pets[index].press.is_none() {
                next = next.min(delay - since);
                continue;
            }
            let world = self.pet_world(index);
            let shown = &mut self.pets[index];
            shown.stepped = now;
            let events = shown.pet.step(since.as_secs_f32(), &world);
            next = next.min(shown.pet.delay());
            for event in events {
                self.pet_event(index, event);
            }
            self.draw_pet(index, &world);
        }
        if !self.props.is_empty() {
            let since = now.saturating_duration_since(self.props_stepped);
            let delay = self.props_delay();
            if since + delay / 3 >= delay {
                self.props_stepped = now;
                let world = self.pet_world(0);
                self.step_props(since.as_secs_f32().min(0.25), &world);
                next = next.min(self.props_delay());
            } else {
                next = next.min(delay - since);
            }
        }
        Some(next)
    }

    fn draw_pet(&mut self, index: usize, world: &World) {
        // Nothing to do until the compositor wants a frame (hidden, it may not for a while)
        if !self.pets[index].surf.configured || self.pets[index].surf.frame_pending {
            return;
        }
        let (species, coat) = {
            let (_, coat) = self.pet_wanted(self.pets[index].slot);
            (self.pets[index].pet.species, coat)
        };
        let coat = crate::pet::draw::coat(species, &coat, &self.theme);
        let size = self.config.pet.size.clamp(70, 140) as f32 / 100.0;
        let (w, h) = world.size;
        let shown = &mut self.pets[index];
        // Its square on the output (it may hang off an edge a little rather than jump)
        let left = ((shown.pet.x - FEET.0).round() as i32).clamp(-(W as i32) / 3, (w - W * 2.0 / 3.0) as i32);
        let top = ((shown.pet.y - FEET.1).round() as i32).clamp(-(H as i32) / 2, (h - H) as i32 + 30);
        let moved = shown.placed != Some((left, top));
        if moved {
            shown.layer.set_margin(top, 0, 0, left);
            shown.placed = Some((left, top));
        }
        let mut piece = shown.pet.draw(&self.theme, &coat, size, world.floor);
        crate::pet::tighten(&mut piece);
        shown.surf.dirty = false;
        shown.surf.anim.begin();
        match shell::render_panel(&mut shown.surf, vec![piece], &mut self.painter, &self.theme, None, 0.0) {
            Some(damage) => present(&mut shown.surf, damage, &mut self.pool, &self.qh),
            None => {
                // The same picture: nothing to send, unless it moved
                if moved && let Some(wl) = &shown.surf.wl {
                    wl.commit();
                }
            }
        }
    }

    /// What a pet's moment brought: words to say aloud, a toy batted, a bowl emptied.
    fn pet_event(&mut self, index: usize, event: Event) {
        log::debug!("pet {index}: {event:?}");
        match event {
            Event::Say(voice, take) => {
                let pet = &self.config.pet;
                // Quiet when you've asked for quiet, and while the screen's locked
                if pet.sounds && pet.volume > 0 && !self.model.notifications.quiet && !crate::lock::running() {
                    self.send(Command::PetSound(voice, take, pet.volume.min(100) as f32 / 100.0));
                }
            }
            Event::Kick(speed) => {
                let x = self.pets[index].pet.x;
                if let Some(toy) = self.props.iter_mut().filter(|p| p.kind.toy()).min_by(|a, b| (a.x - x).abs().total_cmp(&(b.x - x).abs())) {
                    // Yarn is soft: it goes less far, and hardly hops
                    let (push, hop) = if toy.kind == PropKind::Yarn { (0.7, -80.0) } else { (1.0, -180.0) };
                    toy.vx = speed * push;
                    toy.vy = hop;
                    toy.until = Some(Instant::now() + Duration::from_secs(120));
                }
            }
            Event::Ate => {
                // The bowl, empty, goes a moment later
                if let Some(bowl) = self.props.iter_mut().find(|p| p.kind == PropKind::Bowl) {
                    bowl.until = Some(Instant::now() + Duration::from_secs(3));
                }
            }
            Event::Bored => {
                let x = self.pets[index].pet.x;
                if let Some(toy) = self.props.iter_mut().filter(|p| p.kind.toy()).min_by(|a, b| (a.x - x).abs().total_cmp(&(b.x - x).abs())) {
                    toy.until = Some(Instant::now() + Duration::from_secs(20));
                }
            }
        }
    }

    // ---------------------------------------------------------------- props

    /// A bowl of food (or seeds) down on the floor, a little way off.
    pub(super) fn pet_feed(&mut self) {
        self.add_prop(PropKind::Bowl, None);
    }

    /// A ball to play with, rolled out toward them.
    pub(super) fn pet_play(&mut self) {
        self.add_prop(PropKind::Ball, None);
    }

    /// A ball of yarn, rolled out toward them.
    pub(super) fn pet_yarn(&mut self) {
        self.add_prop(PropKind::Yarn, None);
    }

    /// A bed, put down beside the first pet (and kept in the config, so it stays), or taken away.
    pub(super) fn pet_bed(&mut self, put: bool) {
        let value = if put {
            let Some(shown) = self.pets.first() else { return };
            let (w, _) = self.output_size(&shown.output);
            let x = (shown.pet.x - shown.pet.facing * 90.0).clamp(80.0, w - 80.0);
            (x / w * 1000.0).round() as i64
        } else {
            -1
        };
        self.set_setting(&["pet".into(), "bed".into()], &crate::settings::Value::Int(value));
    }

    /// The bed where the config says it is (or none).
    fn sync_bed(&mut self) {
        let wanted = self.config.pet.bed;
        let index = self.props.iter().position(|p| p.kind == PropKind::Bed);
        let Some(output) = self.pets.first().map(|p| p.output.clone()) else { return };
        let (w, _) = self.output_size(&output);
        match (wanted >= 0, index) {
            (false, Some(index)) => {
                let gone = self.props.remove(index);
                self.painter.forget_surface(gone.surf.id);
            }
            (true, Some(index)) => self.props[index].x = wanted as f32 / 1000.0 * w,
            (true, None) => self.add_prop(PropKind::Bed, Some(wanted as f32 / 1000.0 * w)),
            (false, None) => {}
        }
    }

    fn add_prop(&mut self, kind: PropKind, at: Option<f32>) {
        let Some(shown) = self.pets.first() else { return };
        let output = shown.output.clone();
        let (w, _) = self.output_size(&output);
        // Toward the middle of the screen from where the first pet is
        let away = if shown.pet.x < w / 2.0 { 1.0 } else { -1.0 };
        let x = at.unwrap_or((shown.pet.x + away * 170.0).clamp(60.0, w - 60.0));
        if let Some(index) = self.props.iter().position(|p| p.kind == kind) {
            let old = self.props.remove(index);
            self.painter.forget_surface(old.surf.id);
        }
        let namespace = match kind {
            PropKind::Bowl => "asterism-pet-bowl",
            PropKind::Ball => "asterism-pet-ball",
            PropKind::Yarn => "asterism-pet-yarn",
            PropKind::Bed => "asterism-pet-bed",
        };
        let (pw, ph) = kind.size();
        // Under the pets (Top, below their Overlay): a cat on its bed is in front of it
        let (surf, layer) = self.small_layer(&output, Layer::Top, namespace, pw, ph);
        let vx = if kind.toy() { -away * 260.0 } else { 0.0 };
        let until = kind.toy().then(|| Instant::now() + Duration::from_secs(120));
        self.props.push(Prop { surf, layer, kind, x, vx, lift: 0.0, vy: 0.0, placed: None, until });
        if kind == PropKind::Bowl {
            // Food: they notice
            for shown in &mut self.pets {
                shown.pet.notice();
            }
        }
    }

    fn props_delay(&self) -> Duration {
        if self.props.iter().any(|p| p.vx.abs() > 5.0 || p.lift > 0.0) { Duration::from_millis(16) } else { Duration::from_secs(1) }
    }

    /// Toys roll and slow down, bounce off the sides; empty bowls and forgotten toys go.
    fn step_props(&mut self, dt: f32, world: &World) {
        let now = Instant::now();
        let (gone, kept): (Vec<Prop>, Vec<Prop>) = std::mem::take(&mut self.props).into_iter().partition(|p| p.until.is_some_and(|u| u <= now));
        for prop in gone {
            self.painter.forget_surface(prop.surf.id);
        }
        self.props = kept;
        let dt = dt.clamp(0.0, 0.1);
        let floor = world.floor;
        let (w, h) = world.size;
        let species = self.pets.first().map(|p| p.pet.species);
        for i in 0..self.props.len() {
            let prop = &mut self.props[i];
            let (pw, ph) = prop.kind.size();
            if prop.kind.toy() {
                prop.x += prop.vx * dt;
                // Yarn drags on the floor more than a ball
                prop.vx *= (-(if prop.kind == PropKind::Yarn { 2.2 } else { 1.1 }) * dt).exp();
                if prop.vx.abs() < 4.0 {
                    prop.vx = 0.0;
                }
                if prop.x < 30.0 || prop.x > w - 30.0 {
                    prop.x = prop.x.clamp(30.0, w - 30.0);
                    prop.vx = -prop.vx * 0.6;
                }
                if prop.lift > 0.0 || prop.vy < 0.0 {
                    prop.vy += 900.0 * dt;
                    prop.lift = (prop.lift - prop.vy * dt).max(0.0);
                    if prop.lift == 0.0 {
                        prop.vy = if prop.vy > 120.0 { -prop.vy * 0.4 } else { 0.0 };
                    }
                }
            }
            if !prop.surf.configured || prop.surf.frame_pending {
                continue;
            }
            let left = (prop.x - pw / 2.0).round() as i32;
            let top = (floor - ph + 4.0 - prop.lift).round().min(h - ph) as i32;
            let moved = prop.placed != Some((left, top));
            if moved {
                prop.layer.set_margin(top, 0, 0, left);
                prop.placed = Some((left, top));
            }
            let piece = prop_piece(prop.kind, prop.x, prop.vx, species, &self.theme);
            prop.surf.dirty = false;
            prop.surf.anim.begin();
            match shell::render_panel(&mut prop.surf, vec![piece], &mut self.painter, &self.theme, None, 0.0) {
                Some(damage) => present(&mut prop.surf, damage, &mut self.pool, &self.qh),
                None => {
                    if moved && let Some(wl) = &prop.surf.wl {
                        wl.commit();
                    }
                }
            }
        }
    }

    /// A prop's surface was configured: its size, and what takes the pointer (a toy does, to
    /// be kicked; the bowl and the bed don't).
    pub(super) fn prop_configured(&mut self, index: usize) {
        let prop = &mut self.props[index];
        let (pw, ph) = prop.kind.size();
        prop.surf.w = pw;
        prop.surf.h = ph;
        prop.surf.configured = true;
        prop.surf.dirty = true;
        prop.surf.frame_pending = false;
        if let Some(wl) = &prop.surf.wl
            && let Ok(region) = Region::new(&self.compositor)
        {
            if prop.kind.toy() {
                region.add((pw / 2.0 - 14.0) as i32, (ph - 30.0) as i32, 28, 28);
            }
            wl.set_input_region(Some(region.wl_region()));
        }
    }

    /// A toy, clicked: kicked away from the pointer.
    pub(super) fn prop_pointer_event(&mut self, index: usize, event: &PointerEvent) {
        if let PointerEventKind::Enter { serial } = event.kind {
            self.enter_serial = serial;
            self.cursor_shape = None;
            self.set_cursor(Shape::Pointer);
        }
        if let PointerEventKind::Press { button: BTN_LEFT, .. } = event.kind {
            let prop = &mut self.props[index];
            let side = if event.position.0 as f32 > prop.kind.size().0 / 2.0 { -1.0 } else { 1.0 };
            prop.vx = side * 480.0;
            prop.vy = -220.0;
            prop.until = Some(Instant::now() + Duration::from_secs(120));
        }
    }

    // ---------------------------------------------------------------- the pets' surfaces

    /// Only its body takes the pointer: clicks around it go to what's under it.
    pub(super) fn pet_configured(&mut self, index: usize) {
        let shown = &mut self.pets[index];
        shown.surf.w = W;
        shown.surf.h = H;
        shown.surf.configured = true;
        shown.surf.dirty = true;
        shown.surf.frame_pending = false;
        self.pet_region(index);
    }

    /// Only the animal's body takes the pointer (as big as the animal is drawn); clicks round it
    /// pass through.
    fn pet_region(&mut self, index: usize) {
        let k = (self.pets[index].pet.species.scale() * self.config.pet.size.clamp(70, 140) as f32 / 100.0).min(1.9);
        let (half, tall) = ((44.0 * k).min(FEET.0), (58.0 * k).min(FEET.1));
        if let Some(wl) = &self.pets[index].surf.wl
            && let Ok(region) = Region::new(&self.compositor)
        {
            region.add((FEET.0 - half) as i32, (FEET.1 - tall) as i32, (half * 2.0) as i32, (tall + 6.0) as i32);
            wl.set_input_region(Some(region.wl_region()));
        }
    }

    /// The pointer on a pet: seen; a click is a pat, a drag picks it up; a right click opens
    /// its menu.
    pub(super) fn pet_pointer_event(&mut self, index: usize, event: &PointerEvent) {
        let shown = &self.pets[index];
        let (left, top) = shown.placed.unwrap_or((0, 0));
        let (x, y) = (event.position.0 as f32 + left as f32, event.position.1 as f32 + top as f32);
        let output = shown.output.clone();
        let slot = shown.slot;
        match event.kind {
            PointerEventKind::Enter { serial } => {
                self.enter_serial = serial;
                self.cursor_shape = None;
                self.set_cursor(Shape::Grab);
                self.saw_pointer(&output, x, y);
            }
            PointerEventKind::Motion { .. } => {
                if self.pets[index].press.is_none() {
                    self.saw_pointer(&output, x, y);
                } else if self.relative.is_none() {
                    // No relative motion here: the surface's own positions will have to do
                    self.pet_pointer_to(index, x, y);
                }
            }
            PointerEventKind::Press { button: BTN_LEFT, .. } => {
                // Where it's pressed is exact: nothing has moved yet
                let shown = &mut self.pets[index];
                shown.press = Some(((x, y), false));
                shown.at = (x, y);
            }
            PointerEventKind::Release { button: BTN_LEFT, .. } => {
                let world = self.pet_world(index);
                let shown = &mut self.pets[index];
                match shown.press.take() {
                    Some((_, true)) => {
                        shown.pet.drop_down(&world);
                        self.set_cursor(Shape::Grab);
                    }
                    Some((_, false)) => shown.pet.petted(),
                    None => {}
                }
            }
            PointerEventKind::Press { button: BTN_RIGHT, .. } => self.open_desk_menu(output, (x, y), Some(format!("pet.{slot}"))),
            _ => {}
        }
    }

    /// While a pet is pressed, the pointer is at (x, y) on the output: a few pixels from the
    /// press picks it up, and from then it's carried.
    fn pet_pointer_to(&mut self, index: usize, x: f32, y: f32) {
        let world = self.pet_world(index);
        let shown = &mut self.pets[index];
        let Some(((px, py), dragging)) = shown.press else { return };
        shown.at = (x, y);
        if !dragging && (x - px).hypot(y - py) > 6.0 {
            shown.pet.grab(px, py);
            shown.press = Some(((px, py), true));
            self.set_cursor(Shape::Grabbing);
        }
        let shown = &mut self.pets[index];
        if shown.press.is_some_and(|(_, d)| d) {
            shown.pet.carry(x, y, &world);
        }
    }

    /// The pointer moved by (dx, dy): while a pet is pressed, that's where the pointer is.
    pub(super) fn pet_relative_motion(&mut self, dx: f32, dy: f32) {
        let Some(index) = self.pets.iter().position(|p| p.press.is_some()) else { return };
        let (w, h) = self.output_size(&self.pets[index].output);
        let at = self.pets[index].at;
        let at = ((at.0 + dx).clamp(0.0, w), (at.1 + dy).clamp(0.0, h));
        self.pet_pointer_to(index, at.0, at.1);
    }

    /// "Another animal" and "Put it away", from a pet's menu.
    pub(super) fn pet_action(&mut self, slot: Slot, next: bool) {
        let mut entries = self.config.pet.entries();
        if slot >= entries.len() {
            return;
        }
        if next {
            // The next animal, in its own first coat (the coats differ by animal)
            let (species, _) = self.pet_wanted(slot);
            entries[slot] = species.map_or(Species::Cat, Species::next).name().to_string();
        } else {
            entries.remove(slot);
        }
        self.write_pets(entries);
    }

    /// Another pet: this animal (and coat, "panda:brown"), or when none is named, one not out
    /// yet (else the next after the last).
    pub(super) fn pet_add(&mut self, entry: &str) {
        let mut entries = self.config.pet.entries();
        let entry = if Species::from_name(&crate::config::pet_entry(entry).0).is_some() {
            entry.trim().to_lowercase()
        } else {
            let out: Vec<Species> = entries.iter().filter_map(|e| Species::from_name(&crate::config::pet_entry(e).0)).collect();
            let pick = Species::ALL.into_iter().find(|s| !out.contains(s)).unwrap_or_else(|| out.last().map_or(Species::Cat, |s| s.next()));
            pick.name().to_string()
        };
        entries.push(entry);
        self.write_pets(entries);
    }

    /// Which pet the Settings' Pets page shows (when Settings is open).
    pub(super) fn settings_pet(&mut self, pick: crate::settings::PetPick) {
        if let Some(settings) = self.settings.as_mut() {
            settings.pet = pick;
        }
        if let Some(index) = self.panel_index(PanelKind::Settings) {
            self.panels[index].surf.dirty = true;
        }
    }

    /// The pets, written as the config's list (from then on the older two keys are left alone).
    fn write_pets(&mut self, entries: Vec<String>) {
        self.set_setting(&["pet".into(), "pets".into()], &crate::settings::Value::List(entries));
    }
}

/// A bowl (kibble for a cat or a fox, seeds for a bird), a ball or yarn that turns as it rolls,
/// or a bed: a round cushion in a low basket.
fn prop_piece(kind: PropKind, x: f32, vx: f32, species: Option<Species>, t: &Theme) -> Piece {
    let (pw, ph) = kind.size();
    let mut piece = Piece::new("prop", pw, ph);
    let p = &mut piece.prims;
    let shadow = |w: f32| Prim::Rect { x: (pw - w) / 2.0, y: ph - 6.0, w, h: 4.0, radius: 2.0, color: crate::theme::Rgba::BLACK.alpha(0.2) };
    match kind {
        PropKind::Bowl => {
            p.push(shadow(38.0));
            let food = if species == Some(Species::Bird) { crate::theme::Rgba::rgb(0xc8, 0xa8, 0x6a) } else { crate::theme::Rgba::rgb(0x8a, 0x58, 0x36) };
            // The food heaped up, then the bowl round it
            for (i, dx) in [-8.0f32, -3.0, 2.0, 7.0, -5.0, 0.0, 5.0].iter().enumerate() {
                let dy = if i < 4 { 0.0 } else { -3.0 };
                p.push(Prim::Rect { x: pw / 2.0 + dx - 3.0, y: ph - 22.0 + dy, w: 6.0, h: 5.0, radius: 2.5, color: food.mix(crate::theme::Rgba::BLACK, 0.1 * (i % 3) as f32) });
            }
            let (l, r, top, bottom) = (pw / 2.0 - 18.0, pw / 2.0 + 18.0, ph - 19.0, ph - 6.0);
            p.push(Prim::Shape { points: vec![(l, top), (r, top), (r - 4.0, bottom), (l + 4.0, bottom)], color: t.primary });
            p.push(Prim::Rect { x: l, y: top - 2.0, w: r - l, h: 4.0, radius: 2.0, color: t.primary.mix(crate::theme::Rgba::WHITE, 0.3) });
        }
        PropKind::Ball => {
            p.push(shadow(22.0));
            let (cx, cy, r) = (pw / 2.0, ph - 16.0, 10.0);
            p.push(Prim::Rect { x: cx - r, y: cy - r, w: r * 2.0, h: r * 2.0, radius: r, color: t.primary });
            // Two seams that turn with the rolling
            let turn = x / r;
            for offset in [0.0f32, std::f32::consts::PI] {
                let a = turn + offset;
                let points: Vec<(f32, f32)> = (0..7)
                    .map(|i| {
                        let k = i as f32 / 6.0 * std::f32::consts::PI - std::f32::consts::FRAC_PI_2;
                        let sx = a.cos() * r * 0.6 * k.cos();
                        (cx + sx + a.sin() * 2.0, cy + r * 0.92 * k.sin())
                    })
                    .collect();
                p.push(Prim::Line { points, width: 1.6, color: t.on_primary.alpha(0.7) });
            }
            p.push(Prim::Rect { x: cx - r * 0.55, y: cy - r * 0.7, w: r * 0.5, h: r * 0.4, radius: r * 0.2, color: crate::theme::Rgba::WHITE.alpha(0.35) });
        }
        PropKind::Yarn => {
            p.push(shadow(26.0));
            let (cx, cy, r) = (pw / 2.0, ph - 17.0, 11.0);
            let wool = t.palette.tertiary;
            // A loose thread trailing behind, the way it last rolled from
            let behind = if vx > 0.0 { -1.0 } else { 1.0 };
            let wave = (x * 0.15).sin() * 2.0;
            let thread = vec![(cx + behind * r * 0.6, cy + r * 0.6), (cx + behind * (r + 7.0), cy + r - 1.0 + wave), (cx + behind * (r + 14.0), cy + r + 2.0 - wave), (cx + behind * (r + 20.0), cy + r + 3.0)];
            p.push(Prim::Line { points: thread, width: 1.4, color: wool.mix(crate::theme::Rgba::BLACK, 0.15) });
            p.push(Prim::Rect { x: cx - r, y: cy - r, w: r * 2.0, h: r * 2.0, radius: r, color: wool });
            // Wound strands, turning as it rolls
            let turn = x / r;
            for (i, tilt) in [0.0f32, 1.0, 2.1].iter().enumerate() {
                let a = turn + tilt;
                let points: Vec<(f32, f32)> = (0..9)
                    .map(|k| {
                        let u = k as f32 / 8.0 * std::f32::consts::PI;
                        let (lx, ly) = (r * 0.85 * u.cos(), r * 0.45 * u.sin() * if i % 2 == 0 { 1.0 } else { -1.0 });
                        (cx + lx * a.cos() - ly * a.sin(), cy + lx * a.sin() + ly * a.cos())
                    })
                    .collect();
                p.push(Prim::Line { points, width: 1.2, color: wool.mix(crate::theme::Rgba::BLACK, 0.25) });
            }
        }
        PropKind::Bed => {
            p.push(shadow(pw - 10.0));
            // A low basket round a cushion
            let (l, r, top, bottom) = (6.0, pw - 6.0, ph - 22.0, ph - 5.0);
            p.push(Prim::Rect { x: l, y: top, w: r - l, h: bottom - top, radius: 9.0, color: t.primary.mix(t.surface, 0.35) });
            p.push(Prim::Rect { x: l + 7.0, y: top - 4.0, w: r - l - 14.0, h: 12.0, radius: 6.0, color: t.primary_container.mix(crate::theme::Rgba::WHITE, 0.15) });
            p.push(Prim::Rect { x: l, y: top, w: r - l, h: 4.0, radius: 2.0, color: t.primary.mix(crate::theme::Rgba::WHITE, 0.25) });
        }
    }
    piece
}
