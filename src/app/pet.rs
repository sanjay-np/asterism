//! The pet as the app runs it: a small Overlay layer surface, above every window, moved about
//! the output by its margins as the pet goes; only the pet's body takes the pointer, so clicks
//! everywhere else pass through. A timer moves it on (quickly while it moves, slowly asleep).
//!
//! It sees the pointer only where the shell does: over the wallpaper, the desktop's widgets, the
//! bar and the pet itself (`saw_pointer`). It sleeps after a minute without input (an idle
//! notification of its own, apart from the lock's). A click pats it; a drag picks it up (the
//! press holds the pointer, so the drag follows even past its surface) and it lands when let go.
//! Its props, a bowl of food and a ball, are small surfaces of their own on the floor.

use super::*;
use crate::pet::{Event, FEET, H, Pet, Species, W, World};
use smithay_client_toolkit::reexports::protocols::ext::idle_notify::v1::client::ext_idle_notification_v1::{self, ExtIdleNotificationV1};

pub(super) struct PetSurface {
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
}

/// A bowl or a ball on the floor: a small surface of its own.
pub(super) struct Prop {
    pub surf: Surf,
    pub layer: LayerSurface,
    pub kind: PropKind,
    pub x: f32,
    /// Rolling (the ball): speed, and a little hop when it's batted.
    pub vx: f32,
    pub lift: f32,
    pub vy: f32,
    pub placed: Option<(i32, i32)>,
    /// Gone at this time (an empty bowl, a ball nobody's played with).
    pub until: Option<Instant>,
}

/// After this long without input, the pet goes to sleep.
const NAP_AFTER_MS: u32 = 60_000;
/// A prop's square.
const PROP: f32 = 48.0;

pub struct PetIdle;

impl Dispatch2<ExtIdleNotificationV1, App> for PetIdle {
    fn event(&self, app: &mut App, _: &ExtIdleNotificationV1, event: ext_idle_notification_v1::Event, _: &Connection, _: &QueueHandle<App>) {
        app.pet_idle = matches!(event, ext_idle_notification_v1::Event::Idled);
        log::debug!("pet: {}", if app.pet_idle { "you're away, to sleep" } else { "you're back" });
    }
}

impl App {
    /// The pet follows the config: comes, goes, or turns into another animal.
    pub(super) fn sync_pet(&mut self) {
        let species = Species::from_name(&self.config.pet.animal);
        match (species, self.pet.as_mut()) {
            (None, _) => {
                if let Some(pet) = self.pet.take() {
                    self.painter.forget_surface(pet.surf.id);
                }
                if let Some(watch) = self.pet_idle_watch.take() {
                    watch.destroy();
                }
                self.props.clear();
            }
            (Some(species), Some(shown)) => {
                if shown.pet.species != species {
                    let (x, y) = (shown.pet.x, shown.pet.y);
                    shown.pet = Pet::new(species, x, y, crate::transition::Chance::from_clock());
                }
                // A new coat or size: drawn again
                shown.surf.last = None;
            }
            (Some(species), None) => self.add_pet(species),
        }
        if self.pet.is_some()
            && self.pet_idle_watch.is_none()
            && let (Some(notifier), Some(seat)) = (&self.idle_notifier, &self.seat)
        {
            self.pet_idle_watch = Some(notifier.get_idle_notification(NAP_AFTER_MS, seat, &self.qh, PetIdle));
        }
        self.watch_spectrum();
    }

    /// A small Overlay surface on the output, at the top-left, moved by its margins.
    fn small_layer(&mut self, output: &wl_output::WlOutput, namespace: &str, w: f32, h: f32) -> (Surf, LayerSurface) {
        let wl = self.compositor.create_surface(&self.qh);
        let mut surf = self.new_surf(wl.clone());
        if let Some(bar) = self.bars.iter().find(|b| &b.output == output) {
            surf.scale120 = bar.surf.scale120;
            surf.int_scale = bar.surf.int_scale;
        }
        let layer = self.layer_shell.create_layer_surface(&self.qh, wl, Layer::Overlay, Some(namespace), Some(output));
        layer.set_anchor(Anchor::TOP | Anchor::LEFT);
        layer.set_size(w as u32, h as u32);
        // Anywhere on the output, the bar's room too
        layer.set_exclusive_zone(-1);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        layer.commit();
        (surf, layer)
    }

    fn add_pet(&mut self, species: Species) {
        let Some(output) = self.working_output() else { return };
        let (w, h) = self.output_size(&output);
        let (surf, layer) = self.small_layer(&output, "asterism-pet", W, H);
        let floor = self.pet_floor(h);
        let pet = Pet::new(species, w * 0.7, floor, crate::transition::Chance::from_clock());
        self.pet = Some(PetSurface { surf, layer, output, pet, stepped: Instant::now(), placed: None, press: None, at: (0.0, 0.0) });
        if !self.pet_timer {
            self.pet_timer = true;
            let _ = self.handle.insert_source(Timer::from_duration(Duration::from_millis(33)), |_, _, app| match app.step_pet() {
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

    /// Where its feet stand: the foot of the screen, or just above the bar when it's down there.
    fn pet_floor(&self, h: f32) -> f32 {
        if self.theme.on_top { h - 2.0 } else { h - self.theme.reserve() - 2.0 }
    }

    /// The pointer was seen at (x, y) on this output.
    pub(super) fn saw_pointer(&mut self, output: &wl_output::WlOutput, x: f32, y: f32) {
        if self.pet.as_ref().is_some_and(|p| &p.output == output) {
            self.pet_pointer = Some(((x, y), Instant::now()));
        }
    }

    fn pet_world(&self, output: &wl_output::WlOutput) -> World {
        let (w, h) = self.output_size(output);
        let playing = self.model.media.player().is_some_and(|p| p.playing);
        let beat = (playing && !self.model.spectrum.is_empty()).then(|| {
            let low = &self.model.spectrum[..self.model.spectrum.len().min(8)];
            low.iter().sum::<f32>() / low.len() as f32
        });
        // The pointer counts for a moment after it was last seen
        let pointer = self.pet_pointer.filter(|(_, at)| at.elapsed() < Duration::from_millis(1500)).map(|(p, _)| p);
        let bowl = self.props.iter().find(|p| p.kind == PropKind::Bowl && p.until.is_none()).map(|p| p.x);
        let ball = self.props.iter().find(|p| p.kind == PropKind::Ball).map(|p| (p.x, p.vx));
        World { size: (w, h), floor: self.pet_floor(h), perch: self.theme.on_top.then(|| self.theme.reserve()), pointer, idle: self.pet_idle, busy: self.model.machine.cpu, beat, bowl, ball }
    }

    /// The pet's moment: moved on, its props too, placed, drawn. Returns when to come back, or
    /// None when the pet's gone.
    fn step_pet(&mut self) -> Option<Duration> {
        let output = self.pet.as_ref()?.output.clone();
        let world = self.pet_world(&output);
        let shown = self.pet.as_mut()?;
        let dt = shown.stepped.elapsed().as_secs_f32();
        shown.stepped = Instant::now();
        let events = shown.pet.step(dt, &world);
        let next = shown.pet.delay();
        for event in events {
            self.pet_event(event);
        }
        self.step_props(dt, &world);

        let coat = crate::pet::draw::coat(self.pet.as_ref()?.pet.species, &self.config.pet.coat, &self.theme);
        let size = self.config.pet.size.clamp(70, 140) as f32 / 100.0;
        let (w, h) = world.size;
        let shown = self.pet.as_mut()?;
        if !shown.surf.configured || shown.surf.frame_pending {
            return Some(next.min(self.props_delay()));
        }
        // Its square on the output (it may hang off an edge a little rather than jump)
        let left = ((shown.pet.x - FEET.0).round() as i32).clamp(-(W as i32) / 3, (w - W * 2.0 / 3.0) as i32);
        let top = ((shown.pet.y - FEET.1).round() as i32).clamp(-(H as i32) / 2, (h - H) as i32 + 30);
        if shown.placed != Some((left, top)) {
            shown.layer.set_margin(top, 0, 0, left);
            shown.placed = Some((left, top));
        }
        let piece = shown.pet.draw(&self.theme, &coat, size, world.floor);
        shown.surf.dirty = false;
        shown.surf.anim.begin();
        match shell::render_panel(&mut shown.surf, vec![piece], &mut self.painter, &self.theme, None, 0.0) {
            Some(damage) => present(&mut shown.surf, damage, &mut self.pool, &self.qh),
            None => {
                // Nothing new to draw, but a move still needs committing
                if let Some(wl) = &shown.surf.wl {
                    wl.commit();
                }
            }
        }
        Some(next.min(self.props_delay()))
    }

    /// What the pet's moment brought: words to say aloud, a ball batted, a bowl emptied.
    fn pet_event(&mut self, event: Event) {
        log::debug!("pet: {event:?}");
        match event {
            Event::Say(voice, take) => {
                let pet = &self.config.pet;
                // Quiet when you've asked for quiet, and while the screen's locked
                if pet.sounds && pet.volume > 0 && !self.model.notifications.quiet && !crate::lock::running() {
                    self.send(Command::PetSound(voice, take, pet.volume.min(100) as f32 / 100.0));
                }
            }
            Event::Kick(speed) => {
                if let Some(ball) = self.props.iter_mut().find(|p| p.kind == PropKind::Ball) {
                    ball.vx = speed;
                    ball.vy = -180.0;
                    ball.until = Some(Instant::now() + Duration::from_secs(120));
                }
            }
            Event::Ate => {
                // The bowl, empty, goes a moment later
                if let Some(bowl) = self.props.iter_mut().find(|p| p.kind == PropKind::Bowl) {
                    bowl.until = Some(Instant::now() + Duration::from_secs(3));
                }
            }
            Event::Bored => {
                if let Some(ball) = self.props.iter_mut().find(|p| p.kind == PropKind::Ball) {
                    ball.until = Some(Instant::now() + Duration::from_secs(20));
                }
            }
        }
    }

    // ---------------------------------------------------------------- props

    /// A bowl of food (or seeds) down on the floor, a little way off.
    pub(super) fn pet_feed(&mut self) {
        self.add_prop(PropKind::Bowl);
    }

    /// A ball to play with, rolled out toward it.
    pub(super) fn pet_play(&mut self) {
        self.add_prop(PropKind::Ball);
    }

    fn add_prop(&mut self, kind: PropKind) {
        let Some(shown) = self.pet.as_ref() else { return };
        let output = shown.output.clone();
        let (w, _) = self.output_size(&output);
        // Toward the middle of the screen from where it is
        let away = if shown.pet.x < w / 2.0 { 1.0 } else { -1.0 };
        let x = (shown.pet.x + away * 170.0).clamp(60.0, w - 60.0);
        if let Some(index) = self.props.iter().position(|p| p.kind == kind) {
            let old = self.props.remove(index);
            self.painter.forget_surface(old.surf.id);
        }
        let (surf, layer) = self.small_layer(&output, if kind == PropKind::Bowl { "asterism-pet-bowl" } else { "asterism-pet-ball" }, PROP, PROP);
        let vx = if kind == PropKind::Ball { -away * 260.0 } else { 0.0 };
        let until = (kind == PropKind::Ball).then(|| Instant::now() + Duration::from_secs(120));
        self.props.push(Prop { surf, layer, kind, x, vx, lift: 0.0, vy: 0.0, placed: None, until });
        if let Some(shown) = self.pet.as_mut()
            && kind == PropKind::Bowl
        {
            // Food: it notices
            shown.pet.notice();
        }
    }

    fn props_delay(&self) -> Duration {
        if self.props.iter().any(|p| p.vx.abs() > 5.0 || p.lift > 0.0) { Duration::from_millis(16) } else { Duration::from_secs(1) }
    }

    /// Balls roll and slow down, bounce off the sides; empty bowls and forgotten balls go.
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
        for i in 0..self.props.len() {
            let prop = &mut self.props[i];
            if prop.kind == PropKind::Ball {
                prop.x += prop.vx * dt;
                prop.vx *= (-1.1 * dt).exp();
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
            let left = (prop.x - PROP / 2.0).round() as i32;
            let top = (floor - PROP + 4.0 - prop.lift).round().min(h - PROP) as i32;
            if prop.placed != Some((left, top)) {
                prop.layer.set_margin(top, 0, 0, left);
                prop.placed = Some((left, top));
            }
            let piece = prop_piece(prop.kind, prop.x, self.pet.as_ref().map(|p| p.pet.species), &self.theme);
            prop.surf.dirty = false;
            prop.surf.anim.begin();
            match shell::render_panel(&mut prop.surf, vec![piece], &mut self.painter, &self.theme, None, 0.0) {
                Some(damage) => present(&mut prop.surf, damage, &mut self.pool, &self.qh),
                None => {
                    if let Some(wl) = &prop.surf.wl {
                        wl.commit();
                    }
                }
            }
        }
    }

    /// A prop's surface was configured: its size, and what takes the pointer (the ball does,
    /// to be kicked; the bowl doesn't).
    pub(super) fn prop_configured(&mut self, index: usize) {
        let prop = &mut self.props[index];
        prop.surf.w = PROP;
        prop.surf.h = PROP;
        prop.surf.configured = true;
        prop.surf.dirty = true;
        prop.surf.frame_pending = false;
        if let Some(wl) = &prop.surf.wl
            && let Ok(region) = Region::new(&self.compositor)
        {
            if prop.kind == PropKind::Ball {
                region.add(10, 18, 28, 28);
            }
            wl.set_input_region(Some(region.wl_region()));
        }
    }

    /// The ball, clicked: kicked away from the pointer.
    pub(super) fn prop_pointer_event(&mut self, index: usize, event: &PointerEvent) {
        if let PointerEventKind::Enter { serial } = event.kind {
            self.enter_serial = serial;
            self.cursor_shape = None;
            self.set_cursor(Shape::Pointer);
        }
        if let PointerEventKind::Press { button: BTN_LEFT, .. } = event.kind {
            let prop = &mut self.props[index];
            let side = if event.position.0 as f32 > PROP / 2.0 { -1.0 } else { 1.0 };
            prop.vx = side * 480.0;
            prop.vy = -220.0;
            prop.until = Some(Instant::now() + Duration::from_secs(120));
        }
    }

    // ---------------------------------------------------------------- the pet's surface

    /// Only its body takes the pointer: clicks around it go to what's under it.
    pub(super) fn pet_configured(&mut self) {
        let Some(shown) = self.pet.as_mut() else { return };
        if let Some(wl) = &shown.surf.wl
            && let Ok(region) = Region::new(&self.compositor)
        {
            region.add((FEET.0 - 44.0) as i32, (FEET.1 - 58.0) as i32, 88, 64);
            wl.set_input_region(Some(region.wl_region()));
        }
    }

    /// The pointer on the pet: seen; a click is a pat, a drag picks it up; a right click opens
    /// its menu.
    pub(super) fn pet_pointer_event(&mut self, event: &PointerEvent) {
        let Some(shown) = self.pet.as_ref() else { return };
        let (left, top) = shown.placed.unwrap_or((0, 0));
        let (x, y) = (event.position.0 as f32 + left as f32, event.position.1 as f32 + top as f32);
        let output = shown.output.clone();
        match event.kind {
            PointerEventKind::Enter { serial } => {
                self.enter_serial = serial;
                self.cursor_shape = None;
                self.set_cursor(Shape::Grab);
                self.saw_pointer(&output, x, y);
            }
            PointerEventKind::Motion { .. } => {
                let pressed = self.pet.as_ref().is_some_and(|p| p.press.is_some());
                if !pressed {
                    self.saw_pointer(&output, x, y);
                } else if self.relative.is_none() {
                    // No relative motion here: the surface's own positions will have to do
                    self.pet_pointer_to(x, y);
                }
            }
            PointerEventKind::Press { button: BTN_LEFT, .. } => {
                // Where it's pressed is exact: nothing has moved yet
                if let Some(shown) = self.pet.as_mut() {
                    shown.press = Some(((x, y), false));
                    shown.at = (x, y);
                }
            }
            PointerEventKind::Release { button: BTN_LEFT, .. } => {
                let world = self.pet_world(&output);
                let Some(shown) = self.pet.as_mut() else { return };
                match shown.press.take() {
                    Some((_, true)) => {
                        shown.pet.drop_down(&world);
                        self.set_cursor(Shape::Grab);
                    }
                    Some((_, false)) => shown.pet.petted(),
                    None => {}
                }
            }
            PointerEventKind::Press { button: BTN_RIGHT, .. } => self.open_desk_menu(output, (x, y), Some("pet".into())),
            _ => {}
        }
    }

    /// While the pet is pressed, the pointer is at (x, y) on the output: a few pixels from the
    /// press picks it up, and from then it's carried.
    fn pet_pointer_to(&mut self, x: f32, y: f32) {
        let Some(output) = self.pet.as_ref().map(|p| p.output.clone()) else { return };
        let world = self.pet_world(&output);
        let Some(shown) = self.pet.as_mut() else { return };
        let Some(((px, py), dragging)) = shown.press else { return };
        shown.at = (x, y);
        if !dragging && (x - px).hypot(y - py) > 6.0 {
            shown.pet.grab(px, py);
            shown.press = Some(((px, py), true));
            self.set_cursor(Shape::Grabbing);
        }
        if let Some(shown) = self.pet.as_mut()
            && shown.press.is_some_and(|(_, d)| d)
        {
            shown.pet.carry(x, y, &world);
        }
    }

    /// The pointer moved by (dx, dy): while the pet is pressed, that's where the pointer is.
    pub(super) fn pet_relative_motion(&mut self, dx: f32, dy: f32) {
        let Some(shown) = self.pet.as_ref().filter(|p| p.press.is_some()) else { return };
        let (w, h) = self.output_size(&shown.output);
        let at = ((shown.at.0 + dx).clamp(0.0, w), (shown.at.1 + dy).clamp(0.0, h));
        self.pet_pointer_to(at.0, at.1);
    }

    /// "Another animal" and "Put it away", from its menu.
    pub(super) fn pet_action(&mut self, next: bool) {
        let animal = if next { Species::from_name(&self.config.pet.animal).map_or("cat", |s| s.next().name()) } else { "none" };
        self.set_setting(&["pet".into(), "animal".into()], &crate::settings::Value::Str(animal.into()));
    }
}

/// A bowl (kibble for a cat or a fox, seeds for a bird) or a ball that turns as it rolls.
fn prop_piece(kind: PropKind, x: f32, species: Option<Species>, t: &Theme) -> Piece {
    let mut piece = Piece::new("prop", PROP, PROP);
    let p = &mut piece.prims;
    let shadow = |w: f32| Prim::Rect { x: (PROP - w) / 2.0, y: PROP - 6.0, w, h: 4.0, radius: 2.0, color: crate::theme::Rgba::BLACK.alpha(0.2) };
    match kind {
        PropKind::Bowl => {
            p.push(shadow(38.0));
            let food = if species == Some(Species::Bird) { crate::theme::Rgba::rgb(0xc8, 0xa8, 0x6a) } else { crate::theme::Rgba::rgb(0x8a, 0x58, 0x36) };
            // The food heaped up, then the bowl round it
            for (i, dx) in [-8.0f32, -3.0, 2.0, 7.0, -5.0, 0.0, 5.0].iter().enumerate() {
                let dy = if i < 4 { 0.0 } else { -3.0 };
                p.push(Prim::Rect { x: PROP / 2.0 + dx - 3.0, y: PROP - 22.0 + dy, w: 6.0, h: 5.0, radius: 2.5, color: food.mix(crate::theme::Rgba::BLACK, 0.1 * (i % 3) as f32) });
            }
            let (l, r, top, bottom) = (PROP / 2.0 - 18.0, PROP / 2.0 + 18.0, PROP - 19.0, PROP - 6.0);
            p.push(Prim::Shape { points: vec![(l, top), (r, top), (r - 4.0, bottom), (l + 4.0, bottom)], color: t.primary });
            p.push(Prim::Rect { x: l, y: top - 2.0, w: r - l, h: 4.0, radius: 2.0, color: t.primary.mix(crate::theme::Rgba::WHITE, 0.3) });
        }
        PropKind::Ball => {
            p.push(shadow(22.0));
            let (cx, cy, r) = (PROP / 2.0, PROP - 16.0, 10.0);
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
    }
    piece
}
