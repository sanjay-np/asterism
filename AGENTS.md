# AGENTS.md

Guide for anyone (AI agent or human) working on **Asterism** (`asterism`): what it is, how it's built, the
rules it follows, where it stands, and what comes next. Read this before changing code.

## What this project is

Asterism began as a rewrite of the top bar of [ETEA Shell](https://github.com/SDcold/etea-shell)
(a Quickshell/QML desktop shell) as a standalone **pure-Rust** program for **niri** and
**Hyprland**, and is now a **complete desktop shell**, and the user's only one (it replaced
Noctalia on 2026-11-21): bar and menus, launcher, window overview, control centre,
notifications, lock screen (with fingerprint), polkit/Wi-Fi/Bluetooth prompts, wallpaper and
its changer, desktop widgets, settings app, dashboard, clipboard history, screenshots and
recording, timers, weather, night light, keyboard-shortcut sheet. README.md is the user-facing
tour, grouped by what you're doing; this file is how it works and where it stands.

**The look is Asterism's own ("hairline"), chosen 2026-11-20** after the user dropped ETEA's
starry look as too busy and asked for something "neat and minimal", pointing at
cykler01/cykler-caelestia as a *reference only* ("you don't need to copy everything, design it
yourself"). Don't drift back to either ETEA's stars or a straight Caelestia copy. The rules:

- **Colour comes from the wallpaper**: a dark Material 3 tonal-spot scheme (`palette.rs`) grown
  from the image's most telling colour, or `[colors] accent`. Paint with the roles on `Theme`
  (`surface`, `card_deep`, `primary`, `outline_variant`…), never hard-coded colours.
- **Flat surfaces, edged with a hairline** (`Theme::hairline()`, 1 px `outline_variant`): the bar
  is a solid strip with a hairline on its inner edge; menus, panels and toasts are `Prim::Card`s
  (container colour, hairline, a soft 3-layer shadow, radius `t.radius` = 16) floating 8 px off
  the bar. No glow, no stars, no ink lines.
- **One flourish, the asterism**: workspaces are dots; busy ones solid and joined by a thin
  line, empty ones hollow rings, the current one a larger accent dot with a ring.
- **Accent marks state, sparingly**: hover on the bar is a short accent underline that widens
  in (`underline()` in `app/mod.rs`), not a tinted pill; sliders are a 4 px track with a round
  thumb; switches are compact (36×20); tiles are outlined when off, filled `primary_container`
  with an accent icon disc when on; small letter-spaced capital labels (`Col::label`) head groups.
- Type: Google Sans Flex; icons: thin Material outline. Motion stays quick (160 / 260 ms).
- The other styles stay supported: `frame` (screen frame + attached drawers with fillets),
  `floating`, `islands`, `transparent`. Check `line` and `frame` after any visual change.

Licence: GPL-3.0-or-later (same as ETEA). Bundled assets keep their licences: Google Sans Flex
and Poppins (OFL), Material outline icons and Material Symbols (Apache 2.0), Phosphor (MIT).

## Hard constraints (decided with the user, don't undo them)

- **No GTK, no Qt, no gtk-layer-shell.** The user chose pure Rust over installing
  `gtk4-layer-shell`. Wayland is spoken directly through smithay-client-toolkit.
- **The only system library linked is libxkbcommon** (via sctk's `xkbcommon` feature, for typing
  in the launcher with real layouts). It's present on every Wayland desktop. Don't add other C
  libraries (no libpulse, no libpam bindings, no image libraries with C code) without asking.
- **Spawning standard CLI tools is fine** where a protocol is impractical: `pactl` for sound,
  `curl` for web album art, `nmcli` to join wifi, `systemd-run` to start apps, `wl-copy`.
- **zbus is pinned to 5.x** on purpose (6.x reorganised its API). Don't bump it casually.
- Must work on **both niri and Hyprland**. Hyprland 0.55+ uses a Lua config where `dispatch`
  takes Lua (`hl.dsp.focus({ workspace = 3 })`); older versions take `workspace 3`. The Hyprland
  backend tries one form and falls back to the other, remembering which worked. Keep that.
- **Never put a second visible bar on the user's desktop to "check" a visual change** when a
  snapshot will do (see Verifying).

### The user's machine (since 2026-11-21)

Asterism **is** the user's shell now: niri starts `/home/uncore/.local/bin/asterism`
(`spawn-at-startup`; Noctalia's line is commented out; the original config is saved as
`~/.config/niri/config.kdl.before-asterism`). Binds: Mod+Space launcher, Mod+A control centre,
Mod+N notifications, Mod+W wallpaper changer, Mod+Shift+W random wallpaper, Mod+Shift+Escape
session menu; volume/brightness/media keys stay on wpctl/brightnessctl/playerctl. Lid close only
suspends (no locker installed; the user chose that) — the session menu's Lock does nothing
until a locker exists. After a rebuild, `install -Dm755 target/release/asterism
~/.local/bin/asterism`, then restart it (`pkill -x asterism; setsid -f ~/.local/bin/asterism`)
for the change to show. A restart briefly takes the bar away, so say so first.

## Build, run, test

```sh
cargo build --release
./target/release/asterism                       # run the shell (refuses if one already runs)
./target/release/asterism msg help              # drive the running shell
cargo test --release                            # 52 tests, see below
cargo test --release frame_cost -- --ignored --nocapture   # a wallpaper animation frame's cost
cargo clippy --release                          # remaining warnings are style-only
cargo run --example fixtures -- 60              # fake MPRIS player + fake tray app, 60 s
RUST_LOG=asterism::app=trace asterism           # per-frame timings
```

Rust edition 2024 (let-chains are used everywhere). Config lives at
`~/.config/asterism/config.toml` (see `config.example.toml`), polled every 2 s and hot-reloaded.

## Verifying changes

The snapshot mode renders with **live data and no compositor** into a PNG. Use it for every
visual change and look at the result:

```sh
asterism --snapshot out.png [--width 1920] [--scale 2] [--hover KEY] [--wait 3]
asterism --snapshot out.png --menu NAME [--focus KEY] [--room PX --scroll PX]
asterism --snapshot out.png --panel launcher|toasts|osd|control|auth|wallpaper [--query TEXT]
asterism --snapshot out.png --panel wallpaper --menu NAME|control   # whole desktop, menu in place
asterism --snapshot out.png --panel widgets [--hover widget.KIND]  # the desktop while arranging widgets
asterism --snapshot out.png --panel deskmenu [--query KIND]        # the desktop's right-click menu
asterism --snapshot out.png --panel song                           # the new-song card (needs a player)
asterism --snapshot out.png --panel transition [--query KIND@0.3]  # a frame of the wallpaper changing
asterism --snapshot out.png --demo      # made-up workspaces, windows, networks (use for anything shared)
```

`--panel wallpaper --menu …` is the truest picture: the bar, the frame (if any) and the open
menu or control centre where they'd really sit. Plain `--panel control` shows the card alone.

Menu names: calendar, media, sound, network, bluetooth, power, session, machine, traffic,
keyboard, notifications, tray (first tray item). Piece keys for `--hover`/`--focus` look like
`clock`, `page.1`, `status.volume`, `media.pause`, `tray.entry.3`. Snapshots inject sample
notifications when none exist. Run `examples/fixtures` first to get media and tray content.

For behaviour that needs a compositor, prefer short scripted runs (`timeout 8 asterism`), the
control socket (`asterism msg …`), and `niri msg layers` (and `niri msg windows`, for the settings window) to see which surfaces mapped. To test
the notification daemon without fighting the user's daemon, use a private bus:
`dbus-run-session -- sh script.sh` with `notify-send` inside.

**Nested niri, for anything that would disturb the real session** (the lock screen, idle,
clipboard): run `niri -c min.kdl` (a config with nothing but `hotkey-overlay { skip-at-startup }`)
from the real session: it opens as a window with its own Wayland display (`wayland-2`) and its
own clipboard. Run a throwaway shell in it with `WAYLAND_DISPLAY=wayland-2 asterism -c
test.toml` (polkit off, notification daemon off, `[lock] before_sleep = false`) and drive it with
`WAYLAND_DISPLAY=wayland-2 asterism msg …` (the control socket is per display). Stop it by its
config path (`pkill -f -- "-c …/test.toml"`), **never** `pkill -x asterism` while testing. To see
it, prefer asking the nested niri itself (`NIRI_SOCKET=<nested socket> niri msg action
screenshot-screen --write-to-disk true`): it copies to the nested clipboard, not the user's. A
screenshot from the real session (`niri msg action screenshot-window --id N`) also lands on the
user's clipboard, and must come **only after checking the id is the nested `niri` window**: an
empty id screenshots whatever is focused (it happened once and caught one of the user's own
windows; the file was deleted). Move or delete such screenshots from ~/Pictures/Screenshots
afterwards (they're named by the time; check the newest is yours).

Things a script can't do (real clicks, key presses, grabs) must be listed for the user to try.

## Architecture

```
main.rs ── CLI: run | msg … | --snapshot …
   │
   ├── app/ (Wayland thread, calloop event loop)
   │     mod.rs    App: outputs → one bar layer surface each; one xdg popup (the open menu);
   │               pointer/keyboard routing; frame pacing; damage tracking; present()
   │     shell.rs  layer-shell panels: launcher, notification toasts, OSD, control centre;
   │               keyboard routing for panels; `msg` requests
   │     backdrop.rs  wallpaper surfaces (background layer per output), painted on a thread
   │     wallpaper.rs the changer as the app runs it: picker panel, set/step, palette off-thread, rotation
   │     clipboard.rs ext-data-control: read selections on a worker thread, offer one back
   │     night.rs   wlr-gamma-control per output, schedule, the control-centre switch
   │     idle.rs    ext-idle-notify → lock; start_lock() spawns `asterism lock` detached
   │     timer.rs   timer actions and the once-a-second tick
   │     desk.rs    desktop widgets: a Layer::Bottom surface per output, input region = widgets,
   │               drag to move (Grab cursor), drawn with shell::render_panel; the editor
   │               (`edit_widgets`): same surfaces set_layer(Overlay), whole input, Esc; starts
   │               and stops the visualizer's capture (`watch_spectrum`)
   │
   ├── services/ (own thread, tokio current_thread runtime; polkit agent, notification daemon…)
   │     send Update → calloop channel → App::update ; receive Command ← App
   │
   ├── bar.rs, popups.rs, cards.rs, launcher.rs   build Pieces from the Model
   ├── ui.rs        Piece, Prim, Action, Animator/Ease
   ├── render/      Painter (tiny-skia): prims, cards, drawers/fillets, icons, per-piece cache; text.rs
   ├── model.rs     Model, Update, Command, Request
   ├── palette.rs   Material 3 scheme from the wallpaper (material-colors: Celebi quantizer + Score)
   ├── wallpapers.rs  the changer: folder listing, remembered choice, cached thumbnails, picker grid
   ├── lock.rs      the lock screen: its OWN process (`asterism lock`), ext-session-lock, unix_chkpwd
   ├── clipboard.rs history (memory only) + picker; app/clipboard.rs does ext-data-control
   ├── shortcuts.rs niri/Hyprland bind parsing + the sheet; timers.rs, weather.rs, night.rs: pure logic
   ├── dashboard.rs graphs (Prim::Area), disks, process list; the machine service samples processes
   │               only while it's open (Command::WatchProcesses)
   ├── widgets.rs   desktop widgets + saved places (~/.local/state/asterism/widgets.json) + lyrics parsing;
   │               `arrange` (placing, shared with snapshots), `placeholder`, `editor` (frames, ×, add strip)
   ├── spectrum.rs  the visualizer's analysis: radix-2 FFT, log bands, auto-gain, fall-off
   ├── transition.rs wallpaper animations: kinds, `pick` (random, never twice), per-pixel reveal
   │               maps, `compose` (a lookup table per frame), `ease`
   ├── appcolors.rs the scheme for kitty/foot/alacritty/GTK: 16 ANSI colours harmonized to the
   │               source, files beside the apps' configs, written only when changed
   ├── settings.rs  pages of rows → Action::SetSetting(keys, Value) → toml_edit write keeping comments;
   │               drawn as a window (sidebar, page title, rows on a card, stacked when narrow, scrolls)
   ├── world.rs     the wallpaper: an image, a quiet gradient in the scheme, or (opt-in) a planet's hills
   ├── theme.rs, config.rs, images.rs, ipc.rs, snapshot.rs
```

### The UI model (read this before touching drawing)

- Everything is rebuilt immediate-mode from the `Model` whenever something changes, as a list
  of **`Piece`s**: small units (a page, the clock, one status icon, a menu row) made of drawing
  **`Prim`s** (Text, Marquee, Icon, GlowIcon, Image, Ring, Line, Rect, Card, Fillet, Drawer).
- A piece carries its **actions** (left/right/middle/scroll up/down/drag), an optional hover
  **label**, `lift` (bar: accent underline on hover), `highlight` (rows: tonal state layer),
  `blocks` (catches clicks with no action), `pad` (room drawn outside the piece: shadows,
  rings), `glow` (legacy, 0 by default), `moving` (needs frames).
- `Painter::piece` renders each piece once into its own pixmap (with glow) and caches it by a
  content hash; frames are composites of those pixmaps.
- **Damage tracking** (`render_bar`, `render_panel`): each frame compares pieces' hash, bounds,
  lift and opacity with the last frame and repaints only the changed rectangle when it is small.
  `present()` keeps up to 3 shm buffers, each with its accumulated stale region, so only damage
  is converted to BGRA and sent. Menus (`render_popup`) repaint fully (they're small).
- Anything drawn that isn't captured by a piece's content hash (theme colours, glow) needs
  `surf.last = None` to force a full repaint (see `App::reload`).
- **Animation**: `Animator::value(key, target, ms, ease)` is immediate-mode: ask every frame,
  it glides. `surf.anim.begin()` must run before a frame's first `value` call and `end()` after
  (snapshot code calls `Surf::begin_frame`). While anything moves, frame callbacks keep coming;
  the media marquee instead uses a 33 ms timer and a pre-rendered glowing strip (cheap).
- Layout units are logical pixels; drawing multiplies by the surface scale (fractional scale via
  wp_fractional_scale + wp_viewporter, else integer buffer scale).
- Menus are xdg popups under the bar. Clicked open: pointer grab, bar layer keyboard
  `OnDemand` while open. Opened by `msg` (no serial): no grab, bar layer keyboard `Exclusive`
  (Esc or the same keybind closes). Back to `None` on close.
- Panels (`app/shell.rs`, `PanelKind`) are layer surfaces (`Role::Layer`), except the settings
  app, an xdg toplevel (`Role::Window`, `open_window`, app id `asterism-settings`, min 720×480,
  decorations asked of the server: niri and Hyprland draw none, so it has its own title area
  that calls `move_` and a ×). Keys reach it only while it has keyboard focus
  (`App::keyboard_on`, from `KeyboardHandler::enter`). Its configure may leave the size to us
  (then `Panel::want`). Launcher = full-output Overlay layer, Exclusive
  keyboard, dimmed backdrop drawn as the patch background, click on backdrop closes; toasts =
  Top layer anchored to the corner, resized via `set_size` + wait for configure; OSD = Overlay,
  empty input region, closed by a generation-checked timer; control centre = full-output Top
  layer (exclusive zone 0, so it sits below the bar), Exclusive keyboard, card at the right
  edge, a click beside the card closes it. Cards that must not close their panel on click set
  `blocks`.
- Slider drags live on `App::drag` (`DragOn::Popup` or `DragOn::Control`); the value under the
  pointer is shown through `popups::State::dragging` until the system reports back. Second-click
  arming (`Action::Arm`) goes to the open menu's state, else the control centre's
  (`App::control_state`).
- Wallpapers (`app/backdrop.rs`): one Background layer per output, painted by `world::render`
  on a spawned thread, sent back through a calloop channel, presented once, then the canvas is
  dropped. `world::key` (size, scale, planet, settings, image mtime) decides repaints.

### Services (`src/services/`)

| Service | Source | Notes |
|---|---|---|
| `niri.rs` | `$NIRI_SOCKET` event stream + one connection per action | tests cover event parsing |
| `hyprland.rs` | `.socket.sock` (open/close per request) + `.socket2.sock` events | Lua/plain dispatch fallback, tests with fake sockets |
| `machine.rs` | /proc/stat, meminfo, net/dev, thermal/hwmon | every 2 s |
| `audio.rs` | `pactl subscribe` + `pactl -f json …` | volume, sinks, mic, per-app streams |
| `mpris.rs` | D-Bus MPRIS | player choice: chosen > playing > remembered > paused > first; art via curl cache |
| `tray.rs` | StatusNotifierWatcher (own it, or join another as host) + DBusMenu | |
| `power.rs` | UPower DisplayDevice + power-profiles-daemon | |
| `network.rs` | NetworkManager (root path signals + 10 s poll) | AP list only while the menu wants it |
| `bluetooth.rs` | BlueZ object manager | |
| `brightness.rs` | /sys/class/backlight; set via logind `SetBrightness` | brightnessctl fallback |
| `polkit.rs` | org.freedesktop.PolicyKit1 agent; password checked by `polkit-agent-helper-1` via `/run/polkit/agent-helper.socket` (polkit ≥ 126) or the setuid helper | registers for the session (XDG_SESSION_ID, else logind's display session); queue of requests; retries on failure |
| `notifications.rs` | org.freedesktop.Notifications daemon, else swaync-client | yields if another daemon owns the name |
| `xkb.rs` | evdev.lst | layout names ↔ codes |
| (mod.rs) | `ListPhotos(dir)` → `Update::Photos`; `AppColors(palette, apps)` → `appcolors::write` on a blocking thread, then `pkill -USR1 -x kitty` if kitty's file changed | |
| `spectrum.rs` | `parec --device=@DEFAULT_MONITOR@` (else `pw-record` capturing the sink), s16le mono 44.1 kHz | only on `WatchSpectrum(true)`: visualizer shown + a player playing; 30 `Update::Spectrum`/s, none while silent; `App::update` redraws only desks for it |
| `dbus.rs` | helpers: get_all/get/set/call, value conversions, `watch()` match rules | |

Long-running children are started with `services::child()`, which sets `PR_SET_PDEATHSIG` so
they die with the bar. Apps from the launcher go through `launcher::spawn_app` (own systemd
scope via `systemd-run --user --scope`, own process group) so restarting the shell never kills
them. Keep both rules.

`services::Roles` decides side effects: the real shell runs the notification daemon and the
control socket; snapshots run neither.

### Control socket (`ipc.rs`)

`$XDG_RUNTIME_DIR/asterism-$WAYLAND_DISPLAY.sock`, one line in, one line out. `ping` → `pong`
(used for the single-instance guard). Requests parse to `model::Request` and reach
`App::request` as `Update::Request`. Add new commands in `ipc::parse` + `USAGE` + the handler,
with a test in `ipc::tests`.

## Code conventions

- Comments and doc comments say *why*, in plain sentences, in the voice already used
  ("The title slides on with time, once for every bar showing it"). Keep that tone.
- Prefer small free functions and immediate-mode builders over stateful widgets.
- Colours and sizes come from `Theme`'s Material roles; no hard-coded colours. Several old
  names (`color_text`, `color_soft`, `card_deep`, `glow`, `lit`, `warn`) are kept as aliases of
  roles; prefer the role names in new code.
- New config options: field in `config.rs` with a default, documented in
  `config.example.toml`, mentioned in README if user-facing.
- Icons are embedded at build time from `assets/icons/{outline,material,phosphor-fill}` by
  `build.rs`; a missing name in the chosen pack falls back to `material`.
- Don't block the Wayland thread: file walks, icon theme lookups, image decoding and D-Bus all
  happen on the services thread (e.g. `Command::AppIcon` → `Update::AppIcon`).
- Edit with exact, asserted replacements; re-read a file before editing if it may have changed.
- **clippy is clean** (`cargo clippy --release`, no warnings, as of 2026-11-21); keep it so.
  Don't run `cargo fmt`: the code is hand-formatted with wide lines (no rustfmt.toml), and a
  run would rewrite over a thousand places. Watch `clippy --fix`: it once moved a
  state-changing `take()` into a match guard, which works but hides the change.
- **Night light holds the gamma controls only while warming** (`app/night.rs`): holding them
  overrides any colour calibration and blocks other tools (wlsunset, gammastep). Back to
  neutral → `destroy()` them; a `Failed` control sets `gamma_refused` until night light is next
  turned off.
- **The lock's outside unlock** (`asterism unlock` → a file in `$XDG_RUNTIME_DIR`) means any
  process running as the user can unlock the screen, the same trade-off as swaylock's
  `SIGUSR1`. It's the recovery path from a console; don't remove it without another one.
- **Memory with big pictures**: `images::tune_allocator()` (first thing in `main`) pins glibc's
  mmap threshold at 1 MB, so decoded wallpapers (tens of MB) go back to the system when freed;
  `images::give_back_memory()` (`malloc_trim`) runs after wallpaper paints, palette and
  thumbnail work. Without them RSS climbed 75 → 250 MB after a few wallpaper changes. Closing a
  panel must call `Painter::forget_surface` (each panel opening is a new surface id). Thumbnail
  decoding is limited to 3 at a time (semaphore in `services::run`).

### Tests (52)

`theme` (icons and widgets follow the wallpaper: blue vs orange, plain, accent with a distinct
"on", colourful groups), `transition` (every kind starts old and ends new, a wipe goes its way, random never repeats,
maps fit; `frame_cost` is an ignored timing), `world` (the repaint key ignores interval and
animation), `appcolors` (terminal colours keep their meaning and read on the background; every file format,
alacritty's parses as TOML), `widgets` (places old `[x, y]` and new `[x, y, scale]`, scaling a
piece), `spectrum` (FFT of a wave, a tone lands in its band, silence sinks to dots, band edges), `overview` (window maps fit their card), `services::bluetooth` (the agent over a real session
bus: yes, Rejected, passkey; names vs addresses), `services::niri` window rects,
`settings` (writes keep comments, incl. into config.example.toml), `widgets` (synced lyrics),
`dashboard` (ranking, sizes, never ending session processes), `timers` (countdown/pause, focus rounds, stopwatch), `weather` (Open-Meteo parsing, codes),
`night` (curves, schedule), `shortcuts` (niri and Hyprland binds), `clipboard` (dedupe, caps),
`launcher` emoji/ssh, `notifications` rules, `wallpapers` (folder listing, a remembered choice yielding to a config edit, next/random),
`palette` (a dark scheme with light text and rising containers), `bar::kanji_tests` (kanji numerals), `services::polkit` (helper conversation with a pretend helper, picking the user, secrets), `world` (whole opaque picture, repaint key), `services::hyprland` (state parsing, Lua dispatch, legacy fallback, stepping), `services::niri`
(stream parsing, following changes), `ipc` (request parsing), `launcher` (maths, Exec cleaning,
ranking, desktop file parsing). Add tests for any new parsing or protocol logic.

## Current progress (as of 2026-11-21: the first and second rounds from ETEA / caelestia, then
fingerprint, Wi-Fi passwords, Bluetooth pairing and the window overview; newest first)

### Icons and widgets in the wallpaper's colours (2026-11-21, the user asked)

The scheme already followed the wallpaper, but bar icons and widget text were drawn in
`color_text` (a near-white), so it hardly showed. Now `Theme` has `icon_tints: [Rgba; 3]`
(`icon_tint(Icons::{Status, Machine, Tools})`), `widget_strong`, `widget_soft`, `widget_card`,
all derived in `Theme::with_palette` from `[colors] bar_icons` = tinted (default; text mixed
72% toward primary) | accent (primary; then `lit` becomes tertiary so a tool that's on still
stands out) | colourful (primary / text→secondary / text→tertiary per group) | plain, and
`[colors] widgets` = tinted (default) | plain. The key is `bar_icons`, not `icons`: top-level
`icons` is the icon set (a sed in testing once rewrote both). `Palette` gained `secondary`.
`bar.rs` uses `icon_tint` for every icon (status, tools, resources' rings' icons and arrows,
weather, music note, launcher, busy workspaces, tinted tray icons); values and the clock stay
text. Widgets: big clock, headings, titles, row names, placeholder icons → strong; dates and
weekday names → soft; cards → `widget_card` (surface toward primary_container). Fixed on the
way: `repaint_everything` (a new palette) didn't repaint the desks, so widgets kept old colours
until the next minute. Settings → Look has both. Verified: test, snapshots of the bar in all
four modes and two wallpapers' desktops (amber → golden, city lights → peach). Not yet seen by
the user.

### Wallpaper animations (2026-11-21, the user asked for automatic changes "with different
animation each time")

Automatic changes already existed (`[wallpaper] interval`, `rotate_wallpaper` in
`app/wallpaper.rs`; the user's own config has it at 0, i.e. off). New: every change of picture
animates. `[wallpaper] transition` = random (default; never the same kind twice in a row,
`App::last_transition`) | none | fade | wipe | circle | blinds | ink | pixels | slide, and
`transition_ms` (1200; Settings → Wallpaper has both, the choices wrapping onto lines).

How (`app/backdrop.rs`, `transition.rs`): `paint_backdrops` gives the painting thread the new
job and, when a picture is already up at the same size, the old job (`Backdrop::job`) plus one
kind picked for every output; the thread paints both at once (`thread::scope`) and makes the
kind's map (u8 per pixel: when it turns over; wipe = ramp, circle = distance, blinds = strip
delay + within, ink = two octaves of value noise stretched to 0..255, pixels = hashed squares;
fade and slide need none). `painted` starts a `Change` (old, new, map, its own `SlotPool` so the
memory goes after); `step_changes` (first thing in `flush`) composes the eased frame into
`surf.last` and presents it whenever the frame callback has come, and when time is up shows the
new picture the old one-shot way (`show_backdrop`) even with no callback (hidden desktop). The
old picture isn't kept between changes: memory measured 47 MB idle, 88 MB during, 49 MB after.
`world::key` now ignores interval, directory and the animation settings, so changing them no
longer repaints (or animates).

Verified: 51 tests; snapshots of six kinds at 45% (`--panel transition`); nested niri: four
`msg wallpaper next` gave ink, pixels, wipe, fade, each ended on time and freed its memory;
painting both pictures takes 250–450 ms at 1712×1148. A frame at 1920×1200 costs 2–6 ms
(`frame_cost`). **Not verified**: the frame rate on screen. The nested niri window was off
screen in the user's scrolling layout, so it got almost no frame callbacks (3–4 frames per
change); on a visible desktop the callbacks come at refresh rate. The user should watch one.

### Desktop, round two (2026-11-21): the user picked these from the ETEA/caelestia comparison

How each works, where it lives, and how it was checked:

- **The desktop's right-click menu** (`PanelKind::DeskMenu`, `desk::open_desk_menu`,
  `popups::desktop`): a right click on the wallpaper (the backdrop surface now takes the whole
  pointer: `set_input_region(None)` in `backdrop.rs`; before, it took none) or on a widget (the
  desk handles it: `desk_pointer`) opens a full-output Overlay panel, the card at the click,
  kept on screen; a click beside it or Esc closes it (`CloseMenu` closes it too). Items: for a
  widget, Take it off / Write the note (`Action::EditNote` → Settings, Desktop page, note being
  typed) / Next picture (`Action::NextPhoto` → `App::photo_step`); then Arrange widgets, Change
  wallpaper, Another wallpaper, Settings. `open_panel_on` opens a panel on a given output.
- **Widget sizes**: places are now `widgets::Place { x, y, scale }`, saved as `[x, y, scale]`
  (old `[x, y]` files read as scale 1). `Piece::scale(k)` (ui.rs) multiplies every prim, text
  included. In the editor the wheel on a widget changes it by 10% (`STEP`), 60–200%
  (`SMALLEST`/`BIGGEST`), with a percentage badge on its frame while pointed at.
- **Photo frame** (`"photos"`): `[widgets] photos` (empty → ~/Pictures), `photo_minutes`;
  folder listed on the services thread (`Command::ListPhotos`), pictures through the existing
  thumbnail cache (384×240); which one = (minutes / photo_minutes + photo_step) × a large prime
  mod count.
- **Sticky note** (`"note"`): `[widgets] note`, wrapped, paragraphs kept, primary_container
  colours; empty says "Right-click to write a note". Edited in Settings (a Text row) or via the
  menu; `settings::text_value` now gives any typed value's current text.
- **Edge visualizer** (`"edge"`): built in `widgets::edge` (needs the desk's size), 80 bars over
  the whole width, bass in the middle, rising from the foot (or from the bar when it's at the
  bottom); not draggable, not resizable, not in the input region (no `blocks`).
- **New-song card** (`PanelKind::Song`, `cards::song`, `show_song`/`notice_song`): Overlay,
  anchored to the bar's edge with exclusive zone 0 (so it sits just off the bar), empty input
  region, 4.5 s by a generation-checked timer. Shown when (bus, artist, title) of a playing
  player changes, not within 5 s of start, not while the media menu or control centre is open;
  `song_popup = false` turns it off. While it shows, the capture runs too (`watch_spectrum`).
- **Colours for apps** (`appcolors.rs`, `[colors] apps`, four switches in Settings → Look): sent
  at start, on reload and on every wallpaper palette (`send_app_colors`).

Verified: 47 tests; snapshots of the desktop with note, photo frame and edge bars, the editor
(frames, ×, size badge path), the menu (for a widget), the song card; nested niri: colour files
written for foot/alacritty/gtk into a scratch XDG_CONFIG_HOME, the song card's layer maps when a
player starts playing and is gone after its time, not again for the same song; the capture
starts for it. **Not verified (needs the user)**: right-clicking the wallpaper and widgets,
the menu's items by pointer, the wheel resizing, kitty's live reload (not exercised, to leave
the user's kitty alone), GTK apps picking the file up. The user had already moved widgets by
pointer with the first editor (widgets.json held their drags), so dragging works.

How to test in nested niri without touching the real session: see "Verifying changes"; the
fake player (`examples/fixtures`) is on the shared session bus, so the user's real shell sees
it too (a second `parec` from the real shell is expected); stop it with `pkill -x fixtures`,
never `pkill -f` with a pattern that's also in your own command line (it killed the command's
own shell once).

### Settings window, widget editor, visualizer (2026-11-21, after the user asked)

- **Settings is an ordinary window** now (see Panels above), tiled by niri like any app; the
  bar's gear and the control-centre tile only open it (`Action::ToggleSettings` →
  `toggle_settings(Some(true))`), `msg settings` still toggles. Ctrl+W/Ctrl+Q close, Esc only
  lets a typed value be, the wheel and Page Up/Down scroll the page.
- **Widget editor**: right-click a widget, Settings → Desktop → Arrange, or `msg widgets
  [edit|done]`. Add/remove write `[widgets] show` through `set_setting` (comments kept). Drags
  snap to 8 px while editing. Widget pieces now `blocks` (before, `Surf::hit` skipped them, so
  a press on a widget found nothing, and the old drag-to-move couldn't start).
- **Visualizer** widget (`"visualizer"`): mirrored bars from `model.spectrum`, hidden unless a
  player plays; a placeholder in the editor.
- build.rs reads `CARGO_MANIFEST_DIR` at run time (the folder was moved from
  ~/Projects/quickshell/asterism; the cached build script still looked there).

Verified: snapshots of the window at 1200 and 760 wide, the desktop and the editor; in nested
niri, the window maps (`niri msg windows`: "Settings", asterism-settings, tiled, focused),
closes by `msg`, the desk moves Bottom ↔ Overlay with keyboard none ↔ exclusive, placeholders
show, `parec` starts with a (fake) player and stops when it leaves (<1% CPU itself). Not
verified (needs the user): clicking in the editor (add, ×, drag, Done), moving the window by its
sidebar, the bars moving with real music.

### Window overview (2026-11-21)

`overview.rs` + `app/overview.rs`, `PanelKind::Overview`, `msg overview`, right-click on the
pages, the user's Mod+O. niri has no per-window capture (only output screencopy), so it's a
map, not thumbnails: `Window::rect` comes from niri's layout (`niri::rects`: columns side by
side, as wide as their widest tile, 16 px gaps; tiles stacked; floating at
`tile_pos_in_workspace_view`) or Hyprland's `at`/`size`, and `overview::layout` scales a
workspace's rects to fit its card. Commands `FocusWindow`, `CloseWindow`, `MoveWindow(id, ws)`:
niri JSON checked against the live socket with a nonexistent id (all "Handled"); Hyprland's
plain forms are standard (`focuswindow`, `closewindow`, `movetoworkspacesilent ws,address:`),
its Lua forms are guesses, unconfirmed. Drag: a press on `ov.win.*` picks it up (`Carry`), the
ghost follows motion, release < 8 px = click, else dropped on the `ov.ws.*` card under it.
Verified by snapshot (demo and the user's real windows) and tests. Not verified: clicking and
dragging for real.

### Bluetooth pairing (2026-11-21)

`services/bluetooth.rs`: the shell registers `org.bluez.Agent1` at
`/dev/asterism/bluetooth_agent` (capability KeyboardDisplay, RequestDefaultAgent; again
whenever BlueZ's name gets a new owner). Each question → `Update::Pairing(Some(Pairing))` → the
`PanelKind::Pair` card (`cards::pair_card`) → `Action::PairAnswer` → `Command::PairAnswer` →
the oneshot the agent method awaits (60 s, then no). No = `org.bluez.Error.Rejected`.
AuthorizeService lets trusted devices through without asking. The menu scans 30 s on open
(`ScanBluetooth`, "btscan" in `requested`), lists named strangers with RSSI under Nearby;
Pair → trust → connect → a notification; right-click twice forgets (RemoveDevice). Tested:
the agent over a real (session) bus in `answers_pairing_questions`; cards by snapshot. **BlueZ
is inactive on the user's machine**, so pairing a real device is untested.

### Fingerprint unlock and the wifi password prompt (2026-11-21)

- **Fingerprint** (`lock.rs` `fingerprint()`): on `locked`, a thread with its own tokio runtime
  asks fprintd (system bus): GetDefaultDevice → ListEnrolledFingers(user) → Claim → VerifyStart
  "any", listening for VerifyStatus; match → unlock, no-match/retry → hint, restart a round when
  `done`. No reader or no fingers: silently password-only. `[lock] fingerprint` (default on).
  **The user's laptop has an Elan reader, USB 04f3:0c90, that libfprint doesn't support yet**
  (`fprintd-list` says "No devices available"), so this is verified only for the no-reader path
  (nested niri). Worth retrying when libfprint gains the device.
- **Wifi password** (`app/wifi.rs`, `PanelKind::Wifi`, `cards::password_card` shared with polkit
  via `cards::Ask`): secured + not saved (`AccessPoint::known`, from Settings.ListConnections)
  → the card; `Command::ConnectWifiWith(ssid, Secret)` → `network::join`:
  AddAndActivateConnection (key-mgmt sae for WPA3-only, else wpa-psk), watch Active.State for
  45 s, delete the saved connection on failure, `Update::WifiJoined`. 802.1X networks are
  marked and refused with a message. The password never reaches a command line; the String
  copy is zeroed after the call. Verified: snapshot of the card, saved-network detection on the
  real machine (counts only). Not verified: joining with a real password (needs the user).

### Second round (2026-11-21): dashboard, desktop widgets, settings

- **Dashboard** (`PanelKind::Dashboard`, opens from the CPU/memory rings instead of the old
  per-core menu, `msg dashboard`): history (60 samples × 2 s) of CPU, memory, GPU
  (`gpu_busy_percent`, amdgpu), network; disks (statvfs of real filesystems); processes (CPU from
  tick deltas, full names from cmdline when comm is cut at 15); End = SIGTERM, armed by a first
  click, never offered for ESSENTIAL session processes. Verified by snapshot and live open/close.
- **Desktop widgets** (`widgets.rs`, `app/desk.rs`): clock, weather, media (+ LRCLIB synced
  lyrics, `services/lyrics.rs`, fetched once per song, "none" remembered), calendar, system.
  Verified by snapshot (`--panel wallpaper` now draws them) and live (`asterism-desktop` layer).
  Not verified: dragging (needs the pointer), lyrics with a real player (format checked against
  LRCLIB's API).
- **Settings** (`settings.rs`, `PanelKind::Settings`, control-centre tile, the bar's gear when
  `commands.settings` is empty, `msg settings`): writes with toml_edit then `App::reload`; inline
  comments survive (decor copied). Tested against config.example.toml. Not verified: clicking
  through it (needs the pointer).

### The feature batch (2026-11-21)

The user picked these from a list of ETEA / cykler-caelestia features. All build, 32 tests pass;
how each was checked:

- **Lock screen** (`lock.rs`, `app/idle.rs`, `services/login.rs`). Separate process named
  `asterism-lock` (PR_SET_NAME, so `pkill -x asterism` can't kill it); pid file
  `$XDG_RUNTIME_DIR/asterism-lock.pid` written on `locked`; password via `unix_chkpwd USER
  nonull` (password + NUL on stdin, exit 0 = right; a wrong one gave exit 7 here); recovery
  `asterism unlock` (writes `$XDG_RUNTIME_DIR/asterism-unlock`, polled twice a second). Triggers:
  `msg lock`, logind Session.Lock, PrepareForSleep with a "delay" inhibitor (waits ≤3 s for the
  pid file), ext-idle-notify after `[lock] idle` minutes. Verified in nested niri: lock drawn,
  `msg lock`, idle (1 min), outside unlock; on the real session: the inhibitor is held
  (`systemd-inhibit --list`). **Never tested: typing the real password** (needs the user).
  Never lock the user's real session to test.
- **Clipboard history** (ext-data-control). Nested-niri verified: text, multi-line, PNG with
  thumbnail. Not verified: choosing one to paste back (needs keys/clicks).
- **Screenshots/recording**: menu from the tools' capture button; niri's own screenshot actions
  + a thumbnail notification; wf-recorder for recording (not installed here: untested).
- **Timers** (countdown, focus, stopwatch): unit tests + bar ring seen in nested niri.
- **Weather** (Open-Meteo, place from `[weather] city` or the time zone's city; nothing else
  sent): live fetch for Kathmandu seen in the menu and bar.
- **Night light** (wlr-gamma-control): on/off verified on the real screen; schedule by tests.
- **Shortcuts sheet**: parsed the user's real niri config correctly (snapshot).
- **Launcher** `:emoji` (bundled assets/emoji.tsv, colour via `Font::emoji()` → Noto Color
  Emoji first) and `>ssh`; **battery alerts**; **per-app notification rules**.

User's niri binds added: Mod+Alt+L lock, Mod+Ctrl+V clipboard, Mod+Shift+Slash shortcuts.

### Earlier: the "hairline" redesign (2026-11-20)

### The redesign (latest round)

Done and checked by snapshot (`--panel wallpaper --menu …` for every menu, the control centre,
launcher, toasts, OSD and polkit dialog, with the user's wallpaper and with none): palette from
the wallpaper, `line` style (new default), asterism workspaces, underline hover, outlined cards
with shadow, thin sliders, compact switches, outlined tiles, capital group labels, `frame` style
with drawers. Live on niri: bar plus the three `asterism-frame` edges map (frame style). Dead star
code removed (`Prim::Star` stays out; `draw_star` remains only for the opt-in `world` wallpaper).
Not yet seen by the user live: everything in the new look — ask for their impressions first.

### Done and verified on niri 26.04 (by script, snapshot, or both)

- **Bar**: pages (constellations or numbers; niri workspaces per output, Hyprland groups),
  clock, media with sliding title, resources (CPU/memory rings, temperature, network rates), pages as dots, numbers or kanji (`bar::kanji`, drawn through a system CJK font by fallback; the current one in the accent with a bōten emphasis dot above, no circle — the user asked for that),
  tray, tools (screenshot, colour picker, clipboard, microphone, stay awake), status
  (notifications, keyboard layout, network, bluetooth, volume, battery, settings, session),
  optional `launcher` button. Styles line / frame / floating / islands / transparent;
  top or bottom; per-output bars; middle region shifts and shrinks the title on narrow outputs.
- **Menus**: calendar with moon phase, media (cover, seek, controls, player choice), sound
  (volume, brightness, outputs, mic, per-app volume), network (wifi toggle, nearby networks),
  bluetooth, battery + power profiles, session (second click for log out/restart/shut down),
  machine (a bar per core), traffic, keyboard layouts, notification centre, tray app menus with
  submenus. Long menus scroll with a thin scroll bar; full keyboard navigation.
- **Notifications daemon**: pop-ups with pictures, actions, urgency, hover-to-hold, replace,
  close-by-app, expiry, do not disturb, centre with clear all. Verified with `notify-send` on a
  private bus. Yields to another daemon. On the user's machine it is the daemon now (Noctalia
  stopped); a live `notify-send` showed as an Asterism pop-up.
- **Launcher**: full-screen app grid spanning 80% of the width (`SPAN`; columns from `MIN_CELL_W` 144, cells stretch to fill), search field, 64 px icons loaded at 128 px, pages with
  asterism dots, arrows move in 2D via `Launcher::columns`/`per_page` remembered at build),
  alphabetical when empty, ranked search when typing (name, initials, keywords, exec,
  descriptions by word start), maths, `>` commands, `?` web search, apps in their own systemd
  scope. Snapshot `--panel launcher` fetches real icons.
- **Wallpaper changer** (`wallpapers.rs`, `app/wallpaper.rs`, `PanelKind::Wallpapers`):
  full-screen picker of the folder's pictures (`[wallpaper] directory`, else the picture's
  folder, else ~/Pictures/Wallpapers) as 16:10 thumbnails cached in
  ~/.cache/asterism/thumbnails (made 3 at a time, only for the page shown), the current one
  ringed with a check; click/Enter sets it, Shuffle/R random, arrows/pages like the launcher.
  Setting a picture: `wallpapers::remember` writes ~/.local/state/asterism/wallpaper (chosen
  path + the config's `image` at the time; `Config::load` applies it unless the config's line
  changed since), backdrops repaint, colours regrow via `Command::WallpaperPalette` →
  `Update::WallpaperPalette` → `App::repaint_everything`. `interval` minutes rotates at random
  (checked in `tick`, paused while the picker is open). `msg wallpaper [open|close|next|
  previous|random|set PATH]`; a control-centre tile opens it. Verified live on niri (open,
  close, next, random, set; ~38 MB at start, ~55 MB after changes) and by snapshot with 232
  pictures. Not yet verified by the user: picking by pointer and keys.
- **OSD** for volume/brightness/layout changes from any source; brightness via logind.
- **Stay awake** via the idle-inhibit protocol (systemd-inhibit fallback).
- **Control socket** `asterism msg …` and single-instance guard.
- **Polkit agent** (`services/polkit.rs`, dialog `cards::auth`, `PanelKind::Auth`): registers as
  the session's authentication agent, picks the user (you when allowed, else the first allowed
  account), talks the helper's line protocol (`PAM_PROMPT_ECHO_OFF`, `SUCCESS`/`FAILURE`), lets
  you retry, honours `CancelAuthentication`. Passwords live in `model::Secret` (zeroed on drop,
  redacted in Debug, never logged). Verified live on niri with polkit 128: registration, a real
  request showing the dialog, and polkit cancelling it. Not yet verified: a real password
  (needs the user).
- **Control centre** (`PanelKind::Control`, builder `popups::control`): time and battery, quick
  tiles, volume/brightness sliders (dragging now lives on `App::drag`, for menus and panels),
  now playing, notifications grouped by app (older ones give way when it would run off the
  screen), session row with second-click arming. Opens from the bar's hot corner
  (`hot_corner`), `msg control`; Esc or a click beside the card closes it. Verified by snapshot
  and live open/close on niri.
- **Wallpaper** on every output (background layer): an image (cover/contain/stretch) whose
  colour seeds the scheme, else a quiet gradient in the scheme's colours, or opt-in
  (`world = true`) a planet's sky and four curved hill layers. Painted on a worker thread
  (~110 ms for 1920×1200), repainted only when size, scale, planet or settings change. Verified
  live on niri and by snapshot for all four planets.
- **Performance**: ~29 MB RSS with the wallpaper (~19 MB without), ~0.1 % of a core idle, ~2 %
  while a title scrolls at 30 fps.

### Not yet verified by a person (needs real input)

- Clicking through menus, outside-click dismissal (pointer grab), sliders by drag.
- Keyboard navigation in menus opened by click (depends on the compositor giving the popup
  focus) and by `msg menu`.
- Typing in the launcher with non-US layouts.
- Stay awake actually holding off idle.
- Hyprland in general (only tested against fake sockets); Hyprland's Lua bind syntax is not
  confirmed, so README shows hyprlang binds.
- Bluetooth live, pairing above all (bluetooth.service is off on the user's machine).
- The widget editor by pointer (add chips, ×, dragging, Done) and Esc; the settings window's
  move-by-sidebar; the visualizer with real sound.
- Things by pointer and keys in the newer panels: the overview (click, drag, middle-click),
  settings (every control), the dashboard's End, dragging desktop widgets, the clipboard
  picker's Enter, the Wi-Fi card with a real password, the lock screen with the real password.
- Fingerprint unlock (the laptop's Elan reader isn't supported by libfprint yet).
- Recording (wf-recorder isn't installed).
- The polkit dialog with a real password (success, a wrong password then a retry, Cancel).
- Control centre by pointer: the hot corner, tiles, dragging its sliders, click beside the card
  to close, the session row's second click. Live checks so far were `msg control` open/close.
- Day-to-day use as the only shell (since 2026-11-21): the user hasn't reported back yet.

### Known limits

- No 802.1X (enterprise) wifi.
- Notification bodies are plain text (markup stripped); no inline reply.
- `msg menu` opens without a pointer grab: clicking elsewhere doesn't close it (Esc / keybind).
- Toasts show on the focused output only and stay there.
- The control centre and the dashboard answer only Esc on the keyboard (no arrow navigation).
- The overview has no live thumbnails (niri offers no per-window capture).
- Clipboard history is in memory only: empty after a restart (on purpose).
- The changer lists one folder (no subfolders) and PNG/JPEG only (no WebP); one picture for
  all outputs (no per-output wallpapers yet).

## Next progress (immediate)

1. **User test pass**, above all the lock screen with the real password (Mod+Alt+L), then the
   clipboard picker, timers, night light schedule; fix what breaks.
2. Try Bluetooth pairing for real once the user starts bluetooth.service.
3. The user to watch a wallpaper change animate (Settings → Wallpaper: Change by itself, or
   Mod+Shift+W) and say how it feels; then the desktop's round two (right-click menus, resizing, photo frame, note,
   edge bars, song card) and the app colours (include the file in kitty.conf, gtk.css).
4. Next from the ETEA/caelestia list (offered 2026-11-21, not chosen yet): a phone link (KDE
   Connect), workspace window previews on hover (could reuse `overview::layout` in a small
   popup), a left day panel (calendar, to-do, notes as files), OCR of a screen region
   (tesseract), Bluetooth device battery, a VPN tile, settings search. Declined as off-theme
   or out of reach: AI chat, Google Lens, stocks/GitHub widgets, a music library, SDDM theme,
   ETEA's planet/orbit and lock scenes.

Control centre follow-ups: keyboard navigation (only Esc today), a night light tile once night
light exists, a hot corner on the left for a calendar/day panel like ETEA's.

Wallpaper follow-ups (small, any time): per-output pictures, WebP.

## Future plan (later rounds, roughly in order)

4. **i18n** (French and English, like ETEA); **packaging** (AUR PKGBUILD, desktop/session
    files, a .desktop entry for the settings window).

When starting any of these: write the plan in a short message to the user first, keep the
pure-Rust constraint, add snapshot support for anything visual (`--panel`/`--menu`), add tests
for parsing/protocol logic, and update this file's progress sections when done.
