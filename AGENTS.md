# AGENTS.md

Guide for anyone (AI agent or human) working on **Asterism** (`asterism`): what it is, how it's built, the
rules it follows, where it stands, and what comes next. Read this before changing code.

## What this project is

Asterism began as a rewrite of the top bar of [ETEA Shell](https://github.com/SDcold/etea-shell)
(a Quickshell/QML desktop shell) as a standalone **pure-Rust** program for **niri** and
**Hyprland**, and is now a **complete desktop shell**, and the user's only one (it replaced
Noctalia on 2026-11-21): bar and menus, launcher, dock, window overview, control centre,
notifications, lock screen (with fingerprint), polkit/Wi-Fi/Bluetooth prompts, wallpaper and
its changer (pictures, and live video/GIF wallpapers), desktop widgets (solid to see-through),
optional frosted-glass blur by the compositor, settings app, dashboard, a day panel (to-do,
notes), clipboard history, screenshots and recording, timers, weather, night light,
keyboard-shortcut sheet, pets. README.md is the user-facing
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
- **Frosted glass is opt-in and the compositor's** (`app/blur.rs`, ext-background-effect): the
  hairline look stays the default. A surface asking for blur draws its cards with
  `Theme::frosted()` (card at 0.7, no shadow) and full-screen dims at `FROSTED` (0.6); widgets
  have their own opacity. Never fake blur in the shell (no blurring of the wallpaper's pixels).
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
  `gtk4-layer-shell`. Wayland is spoken directly through smithay-client-toolkit (and
  `wayland-protocols` with `staging`, for protocols sctk doesn't wrap).
- **The only system library linked is libxkbcommon** (via sctk's `xkbcommon` feature, for typing
  in the launcher with real layouts). It's present on every Wayland desktop. Don't add other C
  libraries (no libpulse, no libpam bindings, no image libraries with C code) without asking.
- **Spawning standard CLI tools is fine** where a protocol is impractical: `pactl` for sound,
  `curl` for web album art, `nmcli` to join wifi, `systemd-run` to start apps, `wl-copy`,
  `gio` to open and empty the trash (with a plain-file fallback), `ffmpeg` to decode video
  wallpapers (GIFs are decoded in Rust, by the `gif` crate).
- **zbus is pinned to 5.x** on purpose (6.x reorganised its API). Don't bump it casually.
- Must work on **both niri and Hyprland**. Hyprland 0.55+ uses a Lua config where `dispatch`
  takes Lua (`hl.dsp.focus({ workspace = 3 })`); older versions take `workspace 3`. The Hyprland
  backend tries one form and falls back to the other, remembering which worked. Keep that.
- **Never put a second visible bar on the user's desktop to "check" a visual change** when a
  snapshot will do (see Verifying).

### The user's machine (since 2026-11-21)

Asterism **is** the user's shell now: niri starts `/home/uncore/.local/bin/asterism`
(`spawn-at-startup`; Noctalia's line is commented out; the original config is saved as
`~/.config/niri/config.kdl.before-asterism`). Binds (all `asterism msg …`): Mod+Space launcher,
Mod+A control centre, Mod+Shift+D day panel, Mod+D dock, Mod+N notifications, Mod+W wallpaper changer, Mod+Shift+W random
wallpaper, Mod+Shift+Escape session menu, Mod+Alt+L lock, Mod+Ctrl+V clipboard,
Mod+Shift+Slash shortcuts sheet, Mod+O overview; volume/brightness/media keys stay on
wpctl/brightnessctl/playerctl. The lock is Asterism's own (`[lock] idle = 10`, `before_sleep =
true`). After a rebuild, `install -Dm755 target/release/asterism ~/.local/bin/asterism`, then
restart it (`pkill -x asterism; setsid -f ~/.local/bin/asterism`) for the change to show.

The machine: laptop, eDP-1 1920×1200 at scale 1, and (seen since 2026-11-22) a second
monitor, an MSI MP251W on HDMI-A-1, 1920×1080 at scale 1: so two bars, two wallpapers and two
desks, and a video wallpaper runs two players (two `ffmpeg`). niri 26.04 with `default-column-width 0.9` and
`prefer-no-csd` (so windows open wide and nobody draws title bars). PipeWire with
pipewire-pulse (`parec`, `pactl`, `pw-record` present). kitty (its kitty.conf still includes
`themes/noctalia.conf`; pointing it at `asterism-colors.conf` was offered, not done) and GTK
through adw-gtk3-dark (gtk.css is a symlink to the theme). The user's config has
`[wallpaper] interval = 5` (asked for 2026-11-21) with the default random animation, `fit =
"stretch"`, `live = true` and `live_battery = true` (set by the user in Settings on 2026-11-22;
their folder has one GIF, street2.gif, so the rotation can land on it); widgets: only the clock
now; pets: `[pet] animal = "cat"` with `friend = "fox"`, `coat = "red"`, `size = 70`, no bed
(`bed = -1`), sounds on at volume 40. No `[blur]` section and no widget opacity/blur set yet
(as of the last restart): all the blur switches are off. One VPN saved in NetworkManager
("client"); BlueZ off; no slurp/grim/tesseract. The dock (since 2026-11-22), set by the user in
Settings: `pinned = ["kitty", "com.microsoft.VSCode"]`, `magnify = false`, `corners =
"small"`, `background = "glass"`, `attached = true` (on the bottom edge, hiding). Measured on their screen after the optimization round: ~34 MB RSS (21 MB of it the
binary and fonts, file-backed), 1.9 MB shm, ~2.3 % CPU with the fox out (it was 79 MB, ~3 %).

How the user works: short requests, picks from offered options; after each round they want
the build installed and the shell restarted. **Ask before restarting** (they've said yes every
time): it briefly takes the bar away and empties the clipboard history, which lives in memory.
They try things by hand afterwards, so end with what needs their pointer or keys. They commit
themselves (the latest: 26336fc, the dock); commit only when asked. The project's rename
history (etea-bar) was removed on request; keep the ETEA Shell credit in the README.

## Build, run, test

```sh
cargo build --release
./target/release/asterism                       # run the shell (refuses if one already runs)
./target/release/asterism msg help              # drive the running shell
cargo test --release                            # 88 tests (+3 opt-in, --ignored), see below
ASTERISM_SOUNDS=dir cargo test --release write_samples -- --ignored   # the pet's sounds as WAVs
cargo test --release pet::tests::frame_cost -- --ignored --nocapture  # what a pet frame costs
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
asterism --snapshot out.png --panel day [--query typing|notes]            # the day panel (reads [day] folder)
asterism --snapshot out.png --panel wallpaper --menu NAME|control|day   # whole desktop, menu in place
asterism --snapshot out.png --panel widgets [--hover widget.KIND]  # the desktop while arranging widgets
asterism --snapshot out.png --panel deskmenu [--query KIND]        # the desktop's right-click menu
asterism --snapshot out.png --panel song                           # the new-song card (needs a player)
asterism --snapshot out.png --panel dock [--query sample,APP|apps|trash,drag,left|top|right] --demo  # the dock, out
asterism --snapshot out.png --panel deskmenu --query dock:APP|dock:@trash|dock:@trash!  # its menus
asterism --snapshot out.png --panel transition [--query KIND@0.3]  # a frame of the wallpaper changing
asterism --snapshot out.png --panel pet --query cat[:coat]|fox|bird --scale 2  # every pose
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

**README screenshots** (`screenshots/readme/`, remade 2026-11-21; the README is public, so
nothing of the user's may show). How they were made, to redo after visual changes:

- Wallpapers: original abstract gradients, never the user's (their folder is mostly third-party
  art). Each one `magick -size 960x600 xc: -sparse-color Shepards "<5 dark points>" -blur 0x40`,
  screened with a blurred bezier stroke of a lighter colour at 55 % (`-strokewidth 70 … -blur
  0x45`), resized to 1920×1200: dusk, lagoon, bloom, amber, night, moss.
- A scratch config: those pictures as `image`/`directory`, `grain = 0.0`, `[weather] city =
  "Lisbon"` (the default would show the user's city; the old widgets.png did), `lyrics = false`,
  `[day] folder = ""`. `XDG_STATE_HOME` scratch, with a hand-written `widgets.json` per layout
  (places are screen fractions `[x, y]`).
- Everything inside `dbus-run-session` with `examples/fixtures` started there, so the fake
  player and tray never reach the user's session. `--demo` on every shot that shows the
  compositor, networks, processes or the to-do list (it fakes workspaces, windows, Wi-Fi and
  VPN names, the network list capped at six, processes, `day::sample()`). The polkit card shows
  `$USER`: run it with `USER=alex`. The wallpaper picker shows its folder: run it with
  `HOME=<scratch>` and the pictures under `~/Pictures/Wallpapers`.
- Width 1440 (16:10). Composites with `magick montage … -depth 8` (montage writes 16-bit PNGs
  otherwise, 2+ MB each); everything `-strip -define png:compression-level=9`: 3.5 MB in all.
- `--panel wallpaper --menu day` draws the day panel in place (added for this), like `control`.
- A downscaled contact sheet can show false banding on these gradients; check a full-size crop.
- The older top-level `screenshots/*.png` are not referenced by the README.

**Pitfalls met while testing on the user's machine** (each cost a wrong result or a side effect
once):

- A nested test shell inherits `NIRI_SOCKET` from the real session, so it reads (and could act
  on) the user's real windows: its overview and dock showed them. Set `NIRI_SOCKET` to the
  nested niri's socket (`/run/user/1000/niri.wayland-2.*.sock`) for the test shell.
- Snapshots can't show blur: the compositor does it. Check it in nested niri with a
  `screenshot-screen` there, over a busy wallpaper (the fractal test video showed it best; a
  dark one hides it), and compare with it off.
- Killed with SIGTERM, a test shell leaves `$XDG_RUNTIME_DIR/asterism-wayland-2.sock`
  behind; remove it after.

- The nested niri window is often off screen in the user's scrolling layout, and niri sends no
  frame callbacks to what it doesn't show: animations there get 3–4 frames. Measure the cost of
  a frame instead (`frame_cost`), and don't move the user's focus to make it visible.
- `examples/fixtures` is on the shared session bus, so the user's real shell reacts to it too
  (a second `parec`, a song card). Stop it with `pkill -x fixtures`.
- Never `pkill -f PATTERN` when the pattern is also in your own command line: it killed the
  command's own shell once. Stop a test shell by its config path, as above.
- Snapshots read the user's real `~/.local/state/asterism` (widget places, the chosen
  wallpaper, which wins over `image`). Run them with `XDG_STATE_HOME=<scratch dir>` to start
  clean.
- `cargo test` and `cargo clippy` don't rebuild `target/release/asterism`: `cargo build
  --release` before a snapshot of a change.
- Test configs written by script: multi-line TOML strings need `"""…"""`; check the file
  parses (an invalid config silently falls back to the defaults).
- Top-level `icons` is the icon set; `[colors] bar_icons` is their colour. A blanket
  `sed 's/^icons = …/'` changes both.
- A screenshot (`screenshot-screen`) of an off-screen nested niri is drawn apart from the
  output, so it sends no frame callbacks: it shows whatever was last presented when the window
  was really on screen, not the state now. Trust logs (`RUST_LOG=asterism=debug`) over such
  pictures for anything that changes with time (the pet's pose, an animation).

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
   │     blur.rs    compositor blur (ext-background-effect) behind widgets and panels
   │     live.rs    live wallpapers as the app runs them: a `crate::live::Player` per backdrop,
   │               asked for a frame when the frame callback has come and nothing holds it
   │               (battery, lock, away: own idle watch); held 60 s → put away; frames copied
   │               into the surface's own pool and scaled by the viewporter
   │     wallpaper.rs the changer as the app runs it: picker panel, set/step, palette off-thread, rotation
   │     clipboard.rs ext-data-control: read selections on a worker thread, offer one back
   │     night.rs   wlr-gamma-control per output, schedule, the control-centre switch
   │     idle.rs    ext-idle-notify → lock; start_lock() spawns `asterism lock` detached
   │     timer.rs   timer actions and the once-a-second tick
   │     day.rs     the day panel: open/close (a card centred under the bar), its keys
   │               (task field, notes), tick/remove/clear, a save 1 s after the last change
   │     pet.rs     the pets (`Vec<PetSurface>`, slot 0/1): 240×180 Overlay surfaces moved by
   │               margins, one timer, idle watch, drag, sounds; props (bowl, ball, yarn, bed) on Top
   │     peek.rs    the workspace card under a hovered page dot
   │     dock.rs    the dock's surface: Top layer on its edge (PanelKind::Dock), out/in
│               by an edge strip input region, press → click or carry, its menu via the desk menu
│     desk.rs    desktop widgets: per output a `Desk` of small `Part`s, one Layer::Bottom
   │               surface per widget placed by margins (laid out on the whole output, then each
   │               group of pieces shifted into its part); arranging: a 1-pixel dim stretched by
   │               the viewporter, parts remade on Overlay above it; drag to move; starts and
   │               stops the visualizer's capture (`watch_spectrum`)
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
   ├── dock.rs      the dock, pure: items (pinned + open apps matched to desktop files), slots,
│               layout, `hole` for carrying, `build` (slide, magnify, dots, names), the trash
├── day.rs       the day panel's files: todo.md (Markdown checklist) + notes.md, parse/write/load/save
   ├── shortcuts.rs niri/Hyprland bind parsing + the sheet; timers.rs, weather.rs, night.rs: pure logic
   ├── dashboard.rs graphs (Prim::Area), disks, process list; the machine service samples processes
   │               only while it's open (Command::WatchProcesses)
   ├── widgets.rs   desktop widgets + saved places (~/.local/state/asterism/widgets.json) + lyrics parsing;
   │               `arrange` (placing, shared with snapshots), `placeholder`, `editor` (frames, ×, add strip)
   ├── spectrum.rs  the visualizer's analysis: radix-2 FFT, log bands, auto-gain, fall-off
   ├── pet/         the pets: mod.rs behaviour (friends, bed, `after`, stepped frame clock) (needs, acts, `Pet::step` → events, poses),
   │               draw.rs (rig, IK legs, coats), sound.rs (voices synthesized → WAV)
   ├── live.rs      live wallpapers, pure + threads: `Spec` (what plays), `Player` (a thread per
   │               wallpaper: a frame per ask, paced; `ffmpeg` child for videos, the `gif` crate
   │               for GIFs, composed here), stills (`video_still`, `gif_still`; `images::still`)
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
| (mod.rs) | `LoadApps` → `Update::Apps` (desktop files, for the dock); `CountTrash`/`EmptyTrash` → `Update::Trash`; `ListPhotos(dir)` → `Update::Photos`; `AppColors(palette, apps)` → `appcolors::write` on a blocking thread, then `pkill -USR1 -x kitty` if kitty's file changed | |
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
- Colours and sizes come from `Theme`'s Material roles; no hard-coded colours. **One deliberate
  exception: the pet's coats** (`pet::draw::coat`) are the animals' own colours (tabby, ginger,
  red fox, sparrow…), because the user asked for it to be lifelike; its "wallpaper" coat and
  everything round it (bubble, bowl, ball) still use the roles. Several old
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
- **A `SlotPool` buffer's canvas can be longer than w×h×4** (its length is rounded up to 64
  bytes): copy into `canvas[..len]`, never `copy_from_slice` the whole slice (a 1799×1080 GIF
  frame panicked the shell; `present_fill` once had the same trap).
- **Live wallpapers are shm, so frame size is the cost**: converting, the pipe, our copy and the
  compositor's upload all go with pixels, and reading frames back from the GPU is slow, so even
  VA-API scaling to BGRA measured ~19 ms/frame at full size. Measured on the user's laptop
  (1712×1028 nested, a 5 Mb/s 1080p h264): `-threads 1 -filter_threads 1` is both cheaper and
  smaller than ffmpeg's default threads; `-hwaccel auto` at 50 % ≈ 8 ms/frame (~24 % of a core
  at 30 fps), RSS ~150 MB (≈ 80 anon + 75 driver libs); at 100 % ≈ 15–20 ms, ~240 MB. ffmpeg's
  memory is its decoder and filter frames, not the output queue (`-thread_queue_size` changed
  nothing). Hence `live_scale` 50 by default. GPU surfaces (dmabuf) would need EGL/GBM: C
  libraries, ruled out.
- **Buffers for big surfaces come from the surface's own pool** (`BIG_BUFFER` = 1 MB in
  `present`; at most 2 buffers), dropped with the surface. The shared `SlotPool` never shrinks,
  so before this one full-screen panel opened once kept ~27 MB resident for good. Don't make
  whole-output surfaces to show something small (the desktop widgets were one, 9 MB a buffer
  for a clock); fill a whole screen with one colour by a 1-pixel buffer + viewporter
  (`desk::present_fill`). The wallpaper drops its buffer and pool once its frame has come
  (`release_backdrops`).
- **Dragging something whose surface moves with it** (the pet, a desktop widget): never take
  the pointer's position from the moving surface. Event positions are relative to wherever the
  compositor has the surface at that instant, which lags the margins just asked for; adding
  the new margin to an old position overshoots, the overshoot moves it further, and it runs off
  (the user saw the pet fly to the top). Take the press position (exact: nothing has moved
  yet), then add the pointer's own motion (`relative-pointer`, `App::relative_motion`), clamped
  to the output like the cursor. Without the protocol it falls back to surface positions.
- **Don't redraw for news that doesn't concern you**: `App::update` dirties the desks only for
  what widgets show; a part waiting on a frame callback is marked and drawn when it comes
  (`desk_due`), never retried in a loop (hidden under windows, its frame may never come).
- **Memory with big pictures**: `images::tune_allocator()` (first thing in `main`) pins glibc's
  mmap threshold at 1 MB, so decoded wallpapers (tens of MB) go back to the system when freed;
  `images::give_back_memory()` (`malloc_trim`) runs after wallpaper paints, palette and
  thumbnail work. Without them RSS climbed 75 → 250 MB after a few wallpaper changes. Closing a
  panel must call `Painter::forget_surface` (each panel opening is a new surface id). Thumbnail
  decoding is limited to 3 at a time (semaphore in `services::run`).

### Tests (88, +3 opt-in)

`live` (which files move, ffmpeg's arguments and frame size, a GIF cut/widened/shrunk to the
screen, a GIF played round with transparency and delays, a one-frame GIF doesn't play).

`dock` (a frosted card is see-through, app ids match desktop ids loosely, pinned then open apps with their windows, a click
starts / focuses / cycles, repinning, slots in groups and while carrying, where a carried app
lands, builds and stays empty while hidden, nothing grows at rest, the look's measures, frames on every edge, upright sizes).

`day` (reads a Markdown checklist, writes what it reads, keeps its files in a folder).

`pet` also: runs off when a friend chases it, sleeps on its bed (away and tired), naps beside a
sleeping friend, a calm pet repeats its picture; `ipc`: pet friend commands, screen-text.


`pet` (sleeps while away and wakes with a stretch and a hello, stalks and pounces, follows a
high pointer, ignores a far one, eats from the bowl and bats the ball, purrs when petted and
complains when dropped from high, stays on screen and speaks a few times in ten minutes, draws
every pose in every coat), `pet::sound` (every voice and take: length, peak, not silence; a
meow's pitch falls after its middle; WAV header; files made once), `theme` (icons and widgets follow the wallpaper: blue vs orange, plain, accent with a distinct
"on", colourful groups; widget cards from solid to see-through; frosted cards), `transition` (every kind starts old and ends new, a wipe goes its way, random never repeats,
maps fit; `frame_cost` is an ignored timing), `world` (the repaint key ignores interval and
animation), `appcolors` (terminal colours keep their meaning and read on the background; every file format,
alacritty's parses as TOML), `widgets` (places old `[x, y]` and new `[x, y, scale]`, scaling a
piece, a blur region follows the cards' corners), `spectrum` (FFT of a wave, a tone lands in its band, silence sinks to dots, band edges), `overview` (window maps fit their card), `services::bluetooth` (the agent over a real session
bus: yes, Rejected, passkey; names vs addresses), `services::niri` window rects,
`settings` (writes keep comments, incl. into config.example.toml), `widgets` (synced lyrics),
`dashboard` (ranking, sizes, never ending session processes), `timers` (countdown/pause, focus rounds, stopwatch), `weather` (Open-Meteo parsing, codes),
`night` (curves, schedule), `shortcuts` (niri and Hyprland binds), `clipboard` (dedupe, caps),
`launcher` emoji/ssh, `notifications` rules, `wallpapers` (folder listing, a remembered choice yielding to a config edit, next/random),
`palette` (a dark scheme with light text and rising containers), `bar::kanji_tests` (kanji numerals), `services::polkit` (helper conversation with a pretend helper, picking the user, secrets), `world` (whole opaque picture, repaint key), `services::hyprland` (state parsing, Lua dispatch, legacy fallback, stepping), `services::niri`
(stream parsing, following changes), `ipc` (request parsing), `launcher` (maths, Exec cleaning,
ranking, desktop file parsing). Add tests for any new parsing or protocol logic.

## Current progress (as of 2026-11-22: the first and second rounds from ETEA / caelestia, then
fingerprint, Wi-Fi passwords, Bluetooth pairing, the window overview, the desktop rounds, the
pets, an optimization round, the drag fix, the small everyday round, the day panel, the
temperature gauge, the dock, live wallpapers, see-through widgets and blur; newest first)

**State of the tree**: the user's commits up to 26336fc (the dock) hold everything up to it.
After it, uncommitted: live wallpapers, see-through widgets, widget blur and panel blur below.

### Blur behind the control centre and notifications (2026-11-22, the user asked "too")

`[blur] control`, `notifications` (false), Settings → Look. `Theme::frosted()` (field
`frosted`): `card_deep` at 0.7 and `draw_card` without its shadows (they'd show through as a
dark rim). The painter's piece hash now includes `card_deep.key()`, so a card redraws when it
turns frosted (it also keeps cards from going stale on a palette change). `blur::card_rows`
turns every `Prim::Card` in a set of pieces into rounded rows: the Control and Toasts panels
(`panel_blur`), and the menu popup when it's the notification centre (`PopupSurface.blur`,
first field, so it drops before the popup; rows offset by `POPUP_MARGIN`, a commit when only
the blur changed). draw_panel/draw_popup render those with the frosted theme. The framed
control centre is a `Drawer` joined to the frame: no cards, so no blur. Toasts' region is their
resting place (their short rise-in isn't followed). Verified: test
`frosted_cards_let_the_blur_through`; nested niri under `dbus-run-session` (with NIRI_SOCKET set
to the nested niri) over the fractal video: a notify-send pop-up, the control centre and the
notification centre all frosted, no shadow rim. Not seen by the user yet.

### Blur behind the dock, launcher, overview and picker (2026-11-22, the user: "add blur option
for the dock background, app menu over view and wall paper selection too"; read as the dock,
the launcher, the window overview and the wallpaper picker)

The blur code moved to `app/blur.rs`: `BlurCaps`, `bind_blur`, `Blur` (now also on every
`Panel`, declared before its role so it drops first) with `Blur::set(wl, rects, manager,
compositor, qh) -> asked` (sends only changes; the caller commits when nothing was presented),
`App::blurs(wanted)` (asked, capable, bound), `App::panel_blur(kind, pieces, w, h)`: the whole
surface for Launcher/Overview/Wallpapers, the `dock.card` piece's rounded rows (radius from
`Geometry::radius`) while its opacity > 0.5 (it fades as it slides) unless background "none".
Config `[blur] dock, launcher, overview, wallpapers` (all false), Settings → Look (four
switches). Frosted, the full-screen dims drop to `FROSTED` (0.6 of the surface colour, from
0.9/0.93/0.92), and `dock::View::frosted` draws card/flat as glass (0.62 fill) so the blur shows.
Verified: test `a_frosted_card_lets_the_blur_show`; nested niri with all on: launcher,
overview, picker over a frosted screen, the dock's glass card blurred vs not (over the fractal
video). Not seen by the user yet. Note: a nested test shell inherits `NIRI_SOCKET`, so its
overview and dock showed the user's real windows (nothing was clicked); set `NIRI_SOCKET` to the
nested socket when that matters.

### Blur behind widgets (2026-11-22, the user: "add a blur option for the widget background")

The compositor's blur, not ours: ext-background-effect-v1 (niri 26.04 has it, and
org_kde_kwin_blur). `wayland-protocols` is now a direct dependency with `staging` (same 0.32.13
already in the tree, pure Rust). `desk::bind_blur` binds the manager; `BlurCaps` records the
`capabilities` event (`App::blur_capable`, desks repainted when it changes). Each desk `Part`
has a `Blur` (the effect object, made once per surface when first wanted, destroyed in `Drop`
before the layer: it's declared before `layer`, fields drop in order) and the rectangles last
set. In `draw_desk`, with `[widgets] blur` and the capability: for every `widget.KIND` piece
with a card (`widgets::card_radius`: none for clock and edge, 18 note, 20 others, × the place's
scale), `widgets::rounded_rows` makes the rounded rect out of rows (regions are rectangles
only; like rows merged); set only when changed, with a commit even if nothing was redrawn.
Off → `set_blur_region(None)`. Hyprland: no such global as far as known, README says to use a
layerrule on `asterism-desktop`. Settings → Desktop "Blur behind widgets". Verified: test
`a_blur_region_follows_the_cards_corners`; nested niri: capability true, screenshots with blur
on (frosted cards, the corner follows the curve inside the hairline, nothing behind the clock)
and off after a live config edit. Not seen by the user yet; not tried on Hyprland.

### See-through widgets (2026-11-22, the user: "make desktop widget's background transparent")

`[widgets] opacity` (0–100, default 60; was a fixed 62 %) and `outline` (true), Settings →
Desktop "Widget background" (stepper, 10 %) and "Widget outline". `Theme::widget_opacity`,
`widget_outline`, `widget_shadow` (opacity < 0.4) beside `widget_card` (its alpha is the
opacity). `widgets::card` draws the fill only above 0 and the hairline as a stroke
(`outline()`), no longer as a rect under the fill (with a clear fill that would have filled the
card). `widgets::build` wraps `build_widget` and, when `widget_shadow`, `shadow_words` puts the
clock's soft shadow under every Text/Marquee not already shadowed. The note's paper is
0.92 × opacity/0.6 (capped at 0.92), with a hairline once it's under 0.3. Verified: test
`widget_cards_can_be_see_through`; snapshots at 60, 20 and 0 % on street2.gif (cards gone at 0,
words shadowed, hairlines kept). Not seen by the user yet.

### Live wallpapers (2026-11-22, the user: "add support for live wallpaper"; picked videos and
animated GIFs, paused when covered, on battery, and when locked or idle)

- **What**: `[wallpaper] image` (or a pick in the changer) ending in mp4/webm/mkv/mov/m4v/avi/ogv
  or gif plays (`live::is_live`). The changer lists them (`wallpapers::KINDS` + videos) with a
  small play badge; thumbnails, the palette and the still painted by `world::render` all come
  from the first frame (`images::still`: ffmpeg for a video, never read whole; GIFs by magic in
  `images::decode`).
- **Playing** (`live.rs`): `Player::start` spawns the thread (and for a video `ffmpeg -hwaccel
  auto -threads 1 -stream_loop -1 … -vf fps,scale+crop|pad,format=bgra -f rawvideo -`, with
  PDEATHSIG). The thread waits for an ask (which hands back the last frame's Vec: one buffer
  circulates), reads/composes the next frame, waits for its time (on the ask channel, so a
  dropped player doesn't sleep on), and sends it through a calloop channel. Not asked → it waits,
  ffmpeg blocks on its pipe: no CPU. After a pause it goes on from now. Drop kills ffmpeg. GIFs:
  the canvas only (not every frame), disposal Keep/Background/Previous, delay ≤ 1 → 100 ms, a
  one-frame GIF stops after its frame; sent at their own size cut (cover) or widened (contain) to
  the screen's shape, shrunk to `live_scale` when bigger, nearest pixel (`layout`).
- **App** (`app/live.rs`): `sync_live` (from `show_backdrop`, `sync_backdrops`, the tick, the
  idle watch) starts a player for a shown backdrop whose job image is playable, restarts on a
  new `Spec` (size, fit, fps, scale, background), stops otherwise. `step_live` (in `flush`) asks
  when the frame callback came and nothing holds (`live_held`: lock running, `live_away` from
  its own ext-idle notification of `live_idle` minutes, on battery without `live_battery`); held
  60 s → `live_spec` None → put away (ffmpeg and buffers go), restarted from the start when
  released. `live_frame` presents (`present_frame`: whole frame into the surface's own pool,
  two buffers, viewporter destination = the screen). `release_backdrops` leaves a playing
  backdrop's buffers alone. A change of picture starts from the frame on screen
  (`Player::last_picture` replaces the repainted first frame).
- **Covered**: nothing of its own; niri throttles frame callbacks for hidden surfaces to about
  one a second (seen: 16 frames in ~25 s with the nested window off screen), so the player is
  asked that often. With the user's `default-column-width 0.9` the wallpaper is rarely fully
  covered, so expect it to play beside windows.
- Config `[wallpaper] live (true), live_fps (30), live_scale (50 %), live_battery (false),
  live_idle (5 min)`, Settings → Wallpaper has all five; excluded from `world::key`.

Verified: 84 tests (5 new in `live`, the listing test with gif/MP4); snapshots of a video's and
a GIF's still and of the picker (thumbnails + badge); nested niri: on battery → held, put away
after 60 s, ffmpeg gone; `live_battery = true` → a test video plays at 856×514, `msg wallpaper
set` to a GIF → video put away (ffmpeg gone), fade, the GIF plays (screenshot: advanced frames,
colours right); the user's `street2.gif` (1920×1080, 28 frames) first panicked the shell (the
64-byte rounding above), fixed, then played at 856×514, test shell 56 MB (74 before shrinking
big GIFs). ffmpeg costs measured outside the shell (see Code conventions). **Not verified**:
the frame rate and CPU on a visible screen (the nested window was off screen), how a video
looks at 50 % on their screen, the lock and idle holds live (logic shared with the battery
hold, which was seen).

### The dock (2026-11-22, the user: "add new dock feature"; picked auto-hide, pinned + running,
apps button, drag to reorder, magnify, and "Trash/recent", read as a trash can, recent left out)

- **Surface** (`app/dock.rs`): `PanelKind::Dock`, namespace `asterism-dock`, Layer::Top,
  anchored to its one edge only (so the compositor centres it; see Position below), exclusive
  zone 0 (inside a bar on the same edge) or the card's depth when always out, keyboard none,
  sized to the card + room for names (`dock::surface_size`; `set_size` again from
  `dock_pieces` when apps come or go). One dock, on the working output (`sync_dock` from
  tick/reload). Input region (`dock_region`, also after each configure): the card and the room
  icons grow into while out; a 2 px strip on the screen's edge, card-long, while in.
- **Up/down**: Enter → up (`CountTrash`, desktop files re-read at most every 2 min via
  `Command::LoadApps` → `Update::Apps` → `model.apps`); Leave → down after 450 ms
  (generation-checked) unless busy (inside, a press, a carry, its menu open: `desk_menu.widget`
  starts with "dock:"); `msg dock [show|hide]` (up for 3 s). Hidden, `dock::build` returns no
  pieces, so nothing is sent.
- **Pointer** (`dock_pointer`, before the generic routing; the rest — hover, middle and right
  clicks — goes through the usual panel path): a left press on `dock.app.*` waits; release =
  `DockClick` (launch, focus, or the app's next window: `dock::click`), moving > 6 px = carry
  (`DockState.carry` by key; `dock::hole` against the still layout); drop rewrites `[dock]
  pinned` through `set_setting` (`dock::repin`). An open app keeps its place (a `Slot::Hole`)
  until over the pinned group.
- **Menu**: `Action::DockMenu` opens the desktop menu (`open_desk_menu`) with widget
  `dock:KEY` over the icon; `popups::dock` lists windows (`FocusWindow`), New window/Open,
  pin/unpin, Close (all); for `@trash`: Open, Empty (armed: `dock:@trash!`, `DockTrashArm`).
- **Trash** (`dock.rs`): counts `~/.local/share/Trash/files`; opens with `gio open trash:///`
  (else xdg-open the folder); empties with `gio trash --empty` (else removes files/info by
  hand, never following links) on the services thread. New icons `trash`, `trash-full`.
- Launching: the desktop entry's Exec (`clean_exec`, terminal wrap) through `spawn_app`; a pin
  without an entry runs as a plain command name if it's on PATH, else a notification.
- Closing the launcher keeps the dock's icons (they were being dropped with the grid's).
- Config `[dock] enabled (true), pinned ([]), autohide (true), hide_delay (450 ms), running
  (true), size (44), magnify, launcher, trash (true)`.
- **Its look** (2026-11-22, the user: "add some design options, like not rounded border"):
  `corners` rounded (the theme's 16) | small (6) | square | pill (half the card; the pad grows
  so icons clear the round ends), `background` card (`Prim::Card`) | flat (hairline + fill, no
  shadow) | glass (fill at 62 %) | none, `attached` (foot 0; the card runs on past the screen's
  edge by its radius so the lower corners go out of sight), `indicator` dots | line | none,
  `separators`. All in `Geometry::of(&DockSettings)` (was `new(size)`), `Geometry::radius`,
  `slots` and `build`; Settings → Dock groups behaviour, then look, then the apps. Checked by
  snapshots of five combinations on the user's config; test `the_look_changes_the_measures`.
- **Position** (2026-11-22, the user: "add position like top, bottom, left, right"): `[dock]
  position`. Everything in `dock.rs` is laid out as if on the bottom edge — `a` along, `c`
  across from the inner side — and a `Frame { edge, thick }` maps rectangles (and points back,
  for carrying) onto the real edge, for the surface and inside each piece (`piece_frame`);
  squares stay squares, text is only placed, never turned. So icons grow away from every edge,
  the windows' marks sit between icon and edge, names go on the inner side (beside the icon on
  the left/right, cut to `TIP_MAX` 200 px, which is the room kept there), separators turn.
  `Geometry::along/thick/frame`; `surface_size` swaps for upright docks. App side: anchored to
  that one edge (the compositor centres it in the room the bar leaves), the input region and
  strip through the frame, carry positions turned to (a, c), the menu on the card's inner side;
  a new edge closes and reopens the surface (`DockState.edge`). Snapshot: `--panel dock --query
  sample,APP,left`. Tests `frames_put_the_dock_on_its_edge`, `an_upright_dock_is_tall`. Nested
  niri: bottom → left (always out) live, one surface, a half-width tile 832 → 798 px; right and
  top with `msg dock` up and down; screenshots of left, right and top looked right.
- **Settings → Dock** (page 4, between Desktop and Night light; `settings::dock_rows`; the later
  pages moved up one): every option, then each app on the dock with a Pin/Unpin button
  (`Action::DockPin`, keyless rows).
- **Always up** (`autohide = false`): shown from the start, exclusive zone card + foot (68 px at
  44) so windows tile above it, set again on reload (`sync_dock` updates the zone and shows or
  hides); `msg dock` does nothing then. `running = false` keeps only the pinned
  (`App::dock_items`).
- **Design, cleaned up (the user: "cleanup the dock design")**: the apps button and the trash
  are glyphs in the icons' tint (no filled disc, no tile); apps without a picture get a quiet
  hairline-edged tile with a soft initial; a state layer (text colour at 7 %, 11 % held) behind
  the pointed-at icon, growing with it; a pressed icon dips to 92 %; separators are half-height
  hairlines; dots 3 px, the focused app's a 10 px accent pill; the name is the bar's tooltip
  pill (`container_highest`, text − 1) just over the grown icon; card = icon + 16 (6 above,
  dots, 3 under), cells icon + 10. Fixed on the way: with nothing hovered, a separator counted
  as hovered (its key and the hover were both None), so its neighbours were grown at rest
  (tested now).

Installed and restarted on the user's screen (2026-11-22): the `asterism-dock` layer mapped;
39 MB RSS a minute after the restart with the fox out (15.7 MB anon, 21.6 MB file, 2.1 MB shm;
about 34 MB before the dock, so up to ~5 MB is the app list and dock icons, not yet re-measured
after settling).

Verified (design and settings round): 76 tests (the build test now checks nothing grows at
rest); snapshots of the new look with the user's wallpaper and of Settings → Dock; nested niri:
`autohide = false` written live → the dock up and a window's tile 1080 → 1012 px tall, back to
`true` → down after the delay and the tile 1080 again.

Verified (first round): 76 tests (7 new in `dock`, `ipc` dock parsing); snapshots of the dock (plain,
magnified with its name, carrying, the trash) and its menus; nested niri: `asterism-dock` maps
on Top with keyboard none, `msg dock show` / toggle / auto-down after 3 s (debug log "dock
up/down"), a new window's app joins with the focused pill and the surface widens. **Not
verified (needs the user)**: revealing by touching the bottom edge, hover magnify and names,
clicks (focus, cycle, launch, middle click), the right-click menus, dragging to reorder / pin /
unpin, the trash's open and empty.

### Temperature gauge (2026-11-22, the user: "add the temperature gauge in the bar")

The `temperature` resource was a sun icon and a number, off by default. Now it's a ring like
CPU and memory (`bar.rs` `resources`, one branch for all three): filled from 30 °C to 100 °C,
the warning colour past 85 °C, a new `thermometer` icon in all three packs (hand-drawn, matched
to the outline weight), tooltip "Processor temperature · N °C", click opens the dashboard.
On by default (`cpu, memory, temperature, network`); Settings → Bar → "Temperature gauge"
(`settings::resource`, keeps the resources in their usual order). Hidden when no sensor reads.
The user's sensor is k10temp. Verified: snapshot (hover), 69 tests.

### The day panel (2026-11-21, the user: "proceed with the day panel")

A card **centred under the bar** (under the clock, where it usually sits; first built on the
left, moved to the middle at the user's request the same day) (`PanelKind::Day`, namespace
`asterism-day`, full-output Top layer, exclusive zone 0, Exclusive keyboard, a click beside the
card or Esc closes it; framed: hung from the bar like a menu, `card(…, 1)` (4 with the bar at
the bottom), not stretched; with the bar at the bottom it sits just above it).
Opens from a click on the clock (its right click keeps the calendar menu), a new top-left hot
corner (`corner.day`, `[day] corner`, default on), `msg day [open|close]`. Opening it closes
menus, the control centre and the launcher; opening the control centre closes it.

- Built by `popups::day` (`DAY_WIDTH` 380): weekday big + date and ISO week; `calendar()` (its
  `Action::Month` goes to `App::day_state` when no menu is open); timers (running focus /
  countdown with `big_time` + Pause/Stop, else Focus / 5 / 10 / 25 min buttons through
  `Action::Timer`); to-do (rows `day.task.N` → `DayToggle`, × `DayRemove`, "Clear done"
  `DayClearDone`, at most 8 shown then "N more in todo.md", an "Add a task" field
  `DayFocus(1)`); notes (`DayFocus(2)`, wrapped, the last lines when long, a caret); where the
  files are.
- Typing (`app/day.rs` `day_key`): `State.day_field` (0 none, 1 task, 2 notes) and
  `day_typed`; Enter adds a task / a newline in notes; Backspace, Ctrl+Backspace (clear / a
  word); Tab switches; Esc lets go of a field, then closes; a letter with no field starts a
  task.
- Files (`day.rs`): `[day] folder` (empty → ~/Documents/Asterism), `todo.md` as a Markdown
  checklist (reads `- [ ]`, `- [x]`/`[X]`, `*`, plain `- item`; writes `# To do` + boxes) and
  `notes.md`. `Command::DayLoad` on every opening (skipped while one of our changes is still
  unsaved: ours is newer) → `Update::Day`; `Command::DaySave` 1 s after the last change
  (`App::day_dirty`, checked in `tick`) and on close; nothing is written before the files were
  read once (`Day::loaded`), so an early key can't empty them. Both on the services thread
  (`spawn_blocking`).
- The panel redraws on every model update and timer tick (`redraw_day`).
- Settings → Bar: "Day panel corner" and "To-do and notes folder".

Verified: 69 tests (3 new in `day`); snapshots (`--panel day`, `--query typing|notes`); nested
niri with `[day] folder` in scratch: `msg day open` maps `asterism-day` (Top, exclusive
keyboard), `msg day` toggles, `close` closes; the panel showed the scratch todo.md (one open,
one done) and wrote nothing without a change. **Not verified (needs the user)**: clicking the
clock and the top-left corner, ticking/removing/adding by pointer and keys, writing notes, the
save landing in ~/Documents/Asterism.

### Pet round three and small everyday things (2026-11-21, the user: "proceed with the pet first
and small and useful everyday option")

- **Cheaper pet frames**: calm acts (Sit, Loaf, Lie, Sleep, Stand, no music) draw from
  `Pet::frame_clock`, stepped 6/s (4/s asleep); the rig snaps to its target once within 0.08
  (`Rig::distance`); `look` in tenths. Unchanged frames hash the same, so `render_panel` sends
  nothing. Test: a loafing cat repeats ≥ 20 of 30 frames at 50 fps (blinks excluded: they're
  real changes).
- **A second pet** (`[pet] friend`, `friend_coat`): `App::pets: Vec<PetSurface>` with a `slot`
  (0 animal, 1 friend); `pet_world(index)` gives each the others as `World::friends`
  (`Friend { x, act, facing, species }`). Behaviour: in `choose`, 30% when a friend's about: walk
  over and sit (40% a hello), or a chase (run at it), or lie down by it when it sleeps
  (`Pet::after`: what to do on arrival). In `live`: a friend running at it within 170 px → it
  runs off (cornered: back past). Menus per pet: "pet" / "pet.friend" → `PetNext(slot)`,
  `PetAway(slot)`.
- **Bed and yarn**: props now on Layer::Top (pets on Overlay, so always in front). Bed:
  `[pet] bed` in thousandths of the width (-1 none; Int so `settings::Value` can write it),
  `Action::PetBed(bool)`, the menu toggles it; pets walk to it to nap and when you're away.
  Yarn: a softer toy (more friction, 0.7 of the kick, a small hop), a trailing thread.
  `msg pet feed|play|yarn|bed|nobed|next|away`, `pet friend next|away`.
- **Workspace peek** (`app/peek.rs`, `cards::peek`): hovering a `page.{id}` piece 350 ms opens a
  small Overlay surface under it (exclusive zone 0, margin 22, empty input region) with
  `overview::layout` of that workspace's windows and their icons; any other hover closes it;
  redrawn on every update while open. Only for workspaces with windows. `--panel peek [--query
  N]`.
- **Bluetooth battery**: was already read (Battery1) and shown (menu, bar value); added each
  device's charge to the bar tooltip and a one-time notification at ≤ 15% (`App::bt_warned`,
  reset above 25%). BlueZ is off on the user's machine: untested live.
- **VPN** (`network.rs` `saved_vpns`, cached, re-read every 60 s; `Network::vpns` with
  active/connecting from the active connections' Uuid and State; `Command::SetVpn(path, on)` →
  `ActivateConnection` / `DeactivateConnection`): a switch per VPN in the network menu, a tile in
  the control centre. Verified reading the user's saved "client" VPN (off); **never switched it**.
- **Settings search**: `Settings::query`, a field in the sidebar; typing (not into a value) goes
  there, Backspace/Esc; `settings::search` matches label, description and page name (all words),
  rows keep working and say their page. `--panel settings --query search:WORDS`.
- **Text off the screen** (`capture::screen_text`, `Command::ScreenText`, `msg screen-text`, a row
  in the capture menu): slurp → grim → tesseract through a pipe → wl-copy, a notification with
  what was found. None of slurp/grim/tesseract is installed on the user's machine: the menu says
  so instead; untested.

Verified: 66 tests; snapshots (settings search, the control centre with the VPN tile, the peek
card with demo windows); nested niri with the fox and a cat: both pet surfaces, bed and yarn on
the Top layer, both pets batting the yarn (7 kicks each) then bored, 32.5 MB. **Not verified**:
the peek by real hover, the pets' friendship by eye (chase, napping together), the bed nap.

### Dragging the pet sent it to the top (2026-11-21, fixed)

The user picked up the pet and it flew to the top of the screen. Cause: during a drag the
pointer was read from the pet's own surface (local position + the margins last asked for), but
that surface moves with the drag and the compositor reports positions against where it has the
surface *now*, which lags the margins just requested; each overshoot moved it further (positive
feedback). The per-widget desk parts from the optimization round had the same bug waiting.
Fixed with `zwp_relative_pointer_v1` (sctk `RelativePointerState`, bound in `run`, one per
pointer in `new_capability`, `RelativePointerHandler` → `App::relative_motion`): the press
position is exact, the drag adds the pointer's own motion to it, clamped to the output
(`PetSurface::at`, `WidgetDrag::at`; `pet_pointer_to`, `move_widget`). Surface positions are the
fallback only when the protocol is missing. See the rule in Code conventions. **Not verified by
a script** (needs a real drag); the user restarted on it to try.

### Less memory and CPU (2026-11-21, the user asked to "use less ram and less cpu")

Measured first on the user's running shell: 79 MB RSS (37 MB shm, 20 MB anon, 21 MB file),
~3 % CPU, nearly all on the Wayland thread (the pet). The shm pool's mapping had grown to 74 MB.

- Desktop widgets: a surface per widget (see Architecture), not a full-output one (whose
  buffers were 9.2 MB each ×3, plus a full-screen `LastFrame` canvas in anon memory).
- Per-surface pools for buffers ≥ 1 MB, two buffers at most; the wallpaper lets its pool go
  after its frame; wallpaper transitions use the surface's pool (their own one is gone).
- `Painter::configure` on reload instead of a new `Painter` (which rescanned every system font).
- Widgets redraw only for updates they show; parts waiting on frames don't spin (a bug in the
  first version of the per-widget desk, found by measuring: a hidden part kept the desk dirty).
- Pet frame rates by activity (`Pet::delay`: 33/50/83/200/500 ms), 20-point ellipses.
- Caches bounded: covers ≤ 6, the photo frame keeps one thumbnail while the picker is closed,
  closing the launcher drops icons of apps with no window open.

Results, A/B in nested niri with the user's config (clock, media, visualizer, the fox): RSS 48.6
→ 31.7 MB, shm 10.8 → 1.8 MB, anon 17.5 → 10.4 MB, CPU ~3 % → 1.6 % (no pet: 0.27 % both;
arranging widgets 37 MB). A frame of the cat costs ~0.35 ms to paint (`pet::tests::frame_cost`,
opt-in). Found and fixed on the way: `present_fill` wrote 4 bytes into a 64-byte slot (a panic on
arranging, caught in the nested run). **Not yet measured**: the user's real screen after a restart.

### The pet, made lifelike (2026-11-21: "life like… like real pet… interactions, like speak like
meow for cat from time to time")

Replaces the first pet's drawing and behaviour (below; its surface, idle watch, pointer
sightings and menu stay).

- **Drawing** (`pet/draw.rs`): a `Rig` (hip, shoulder, head + tilt, four feet, tail angle/curl/
  sway, ears, eyes, mouth, tuck, ball, wing) that the behaviour lerps toward each act's pose
  (`pose()` in mod.rs; walking feet come from a 4-beat gait directly). Drawn with the new
  `Prim::Shape` (filled closed outline: ellipses, tapered capsules, triangles): torso = capsule +
  haunch + chest, legs by 2-joint IK bending back (elbow/hock), paws, a 9-segment tapering tail
  (the fox's a bushy brush) clamped to the floor unless airborne, head with ears (+inner), muzzle
  or snout, almond eyes with slit pupils that follow `look` and blink, nose, mouth/yawn,
  whiskers; tabby stripes, Siamese points, fox socks; the bird with breast, streaks, flapping
  wings (drawn 1.35× so it isn't a speck). A floor shadow. Bubble, heart, "!", z's round it.
- **Behaviour** (`pet/mod.rs`): needs energy (−1/1500 s awake, +1/90 s asleep), hunger
  (+1/7200 s), lonely (+1/900 s; petting −0.35). Acts Stand, Walk, Run, Sit, Loaf, Lie, Sleep,
  Groom, Stretch, Yawn, Crouch, Pounce, Bat, Eat, Held, Fall, Fly, Hop, Peck. Priorities in
  `live`: idle → lie → sleep; waking → Stretch → Yawn → hello; a bowl and any hunger → eat; a
  ball → run and bat (`Event::Kick`, bored after 6–10, 60 s cool); the pointer within 520 px →
  low and near: crouch-wiggle-pounce (4 s calm after), else run over and sit watching. `choose`
  by needs: tired → lie/nap, needy → walk to the middle and call, busy → zoomies, music → sit and
  sway, else weighted idle acts. Speaks (`Event::Say(voice, take)` + bubble) on its own every
  150–390 s (40–90 s when hungry or lonely), when petted (purr), woken, fed, notified (30%),
  dropped from > 260 px (complain); never twice within 3 s.
- **Sound** (`pet/sound.rs`): additive synthesis (harmonics × moving formants, breath noise,
  envelopes) at 22 050 Hz: meow (m→ee→a→ow, pitch up then down), trill (rolled 30 Hz tremble),
  purr (25 Hz pulses in two breaths), complain, yip, chirp; 3 takes each; WAVs cached in
  ~/.cache/asterism/pet; `Command::PetSound` → `pw-play --volume` (else `paplay`). Gated in
  `App::pet_event`: `[pet] sounds`, volume > 0, not do-not-disturb, not locked.
- **Interactions** (`app/pet.rs`): click = petted; press + 6 px = grab (implicit grab keeps the
  motion coming past the surface), carried, dropped (falls with gravity); right-click menu:
  Feed it, Play with a ball, Another animal, Put it away (`Action::PetFeed/PetPlay/...`, also
  `msg pet feed|play|next|away`). Props are 48 px Overlay surfaces: the bowl (kibble or seeds)
  gone 3 s after `Ate`; the ball rolls with friction, bounces off the sides, hops when batted,
  can be clicked to kick, goes 20 s after `Bored` or 2 min untouched.
- Config `[pet] animal, coat, sounds (true), volume (40), size (100)`; Settings → Desktop shows
  coat/sounds/volume/size once an animal is chosen.
- Verified: 62 tests; every pose in every coat by snapshot; the sounds written and played (at
  volume 0, so nothing was heard); nested niri with sounds off: `msg pet feed` → bowl mapped,
  the cat ate (`Ate`, a trill), the bowl went; `msg pet play` → six kicks (one backward) then
  `Bored`; ~3% CPU while active (the first pet ~1%), 49 MB. **Not verified (needs the user)**:
  how the sounds actually sound, stroking, dragging, the right-click menu, the pounce on the
  real pointer, the frame rate and CPU on their visible screen.

### The pet (2026-11-21, the user asked for "pets": over everything, a few animals, reacting
to the pointer, idle/busy, music and notifications)

`[pet] animal` = none (default) | cat | fox | bird; Settings → Desktop → Pet; right-click menu
(the desktop menu with widget "pet": Another animal → `Action::PetNext`, Put it away →
`PetAway`, both write the config). Not a recreation of any existing pet program: drawn from the
shell's prims (ellipses as round Rects, ears/beak/tail feathers as `Prim::Area` triangles, legs
and tails as round-capped lines), coloured by roles (fox = primary, cat = soft text toward the
icon tint, bird = icon tint with primary wings), so they follow the wallpaper.

- **Surface** (`app/pet.rs`): one 96×96 Overlay layer, anchor top-left, exclusive −1, moved by
  `set_margin` each frame; input region only the body (16,30 64×58), so clicks pass through
  around it. On the working output; if that output goes, it comes back on another at a tick.
  A calloop timer steps it (`Pet::delay`: 33 ms moving, 90 ms still, 140 ms asleep) and draws
  when the frame callback has come; `render_panel` on one piece.
- **Behaviour** (`pet.rs`, pure): acts Sit, Walk, Groom, Sleep, Chase, Pounce, Fly; wanders by
  chance (`transition::Chance`), floor = screen foot (above a bottom bar), the bird's perch =
  the bar's lower edge. Idle: an idle notification of its own (60 s, `PetIdle`, apart from the
  lock's) → sleeps (the bird lands first) and wakes on resume. Busy (cpu > 0.6): runs. Music:
  bobs by the mean of the 8 lowest spectrum bands (so `watch_spectrum` also runs the capture
  while a pet is out and music plays). Notifications (`toast`) → "!" and a look right for 2 s.
  Pointer: `saw_pointer` from motion over backdrops, desks, bars (bar y converted to the output)
  and the pet; counts for 1.5 s; chased within 700 px; a pounce when it's near and low, then 3 s
  calm. Click → heart (solid, grows and shrinks: overlapping see-through shapes showed seams).
- Verified: 4 tests; poses by snapshot (`--panel pet`); nested niri: the layer maps on Overlay
  with no keyboard, the cat walks the foot of the screen in the wallpaper's colours, the idle
  notice comes after 60 s (debug log), ~1% CPU. **Not verified (needs the user)**: clicking and
  right-clicking it, chasing the real pointer, the bird on a real bar, bobbing to real music,
  how big it feels on their screen (96 px; a size setting could follow).

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
- **Performance** (re-measured 2026-11-21 after the optimization round below): ~31 MB RSS with
  the wallpaper and three widgets (was ~46–79 MB), ~0.3 % of a core idle, the pet ~1.5 %.

### Not yet verified by a person (needs real input)

- Live wallpapers on the visible screen: the frame rate, ffmpeg's real CPU and memory, how a
  video looks at 50 %, the lock and idle holds; two screens each playing.
- See-through widgets (opacity, outline) and every blur switch (widgets, dock, launcher,
  overview, picker, control centre, notifications) on their real screen: seen only in nested
  niri.

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
- The pet by pointer: stroking, picking up and dropping (the relative-pointer drag), the
  right-click menu, the ball by click; its sounds heard; the desktop right-click menu.
- Things by pointer and keys in the newer panels: the overview (click, drag, middle-click),
  settings (every control), the dashboard's End, dragging desktop widgets, the clipboard
  picker's Enter, the Wi-Fi card with a real password, the lock screen with the real password.
- Fingerprint unlock (the laptop's Elan reader isn't supported by libfprint yet).
- Recording (wf-recorder isn't installed).
- The polkit dialog with a real password (success, a wrong password then a retry, Cancel).
- Control centre by pointer: the hot corner, tiles, dragging its sliders, click beside the card
  to close, the session row's second click. Live checks so far were `msg control` open/close.
- The day panel by pointer and keys: the clock and top-left corner, adding/ticking/removing
  tasks, writing notes, Mod+Shift+D; the files landing in ~/Documents/Asterism.
- The dock by pointer: the edge reveal, magnify, clicks, menus, drag to reorder/pin/unpin, trash.
- Day-to-day use as the only shell (since 2026-11-21): the user hasn't reported back yet.

### Known limits

- Blur needs the compositor's ext-background-effect (niri 26+); without it the switches do
  nothing (Hyprland: layer rules, written in the README, untested). The framed control centre
  (a drawer) isn't frosted. A toast's blur region is its resting place, not its short rise-in.
  Blur on the dock follows the card only while it's mostly out (opacity > 0.5).

- No 802.1X (enterprise) wifi.
- Notification bodies are plain text (markup stripped); no inline reply.
- `msg menu` opens without a pointer grab: clicking elsewhere doesn't close it (Esc / keybind).
- Toasts show on the focused output only and stay there.
- The control centre and the dashboard answer only Esc on the keyboard (no arrow navigation).
- The overview has no live thumbnails (niri offers no per-window capture).
- Clipboard history is in memory only: empty after a restart (on purpose).
- The changer lists one folder (no subfolders): PNG, JPEG, GIF, videos (no WebP); one picture
  for all outputs (no per-output wallpapers yet).
- Live wallpapers go through shm, drawn by the processor (no EGL): a video costs ~¼ core and
  ~150 MB in ffmpeg at half size. Its sound is dropped. One player per output (each its own
  ffmpeg) on several screens.
- The pets see the pointer only over the shell's own surfaces (wallpaper, widgets, bar, the
  pets): Wayland doesn't tell an app where the pointer is over other windows. They live on one
  output (the working one when they first appear).
- Dragging the pets and the widgets needs relative-pointer (niri and Hyprland have it); without
  it they fall back to surface positions, which can drift while the surface moves.
- The workspace card is a map of the windows, not pictures of them (the same as the overview).
- The day panel's notes are typed at the end only (no cursor to move, no selection); for real
  editing open notes.md. Tasks can't be reordered or edited in place (remove and add, or edit
  todo.md).
- The dock is on one output, revealed from a strip as long as its card (not the whole edge); on
  the bar's edge it sits inside the bar and comes out from the bar's inner edge; its
  trash count is the home trash only; "recent files" was not done.
- Copying text off the screen needs slurp, grim and tesseract (not installed on the user's
  machine); it reads with tesseract's default language.

## Next progress (immediate)

0. The user to try frosted glass and see-through widgets: Settings → Look (the six blur
   switches), Settings → Desktop (Widget background, outline, blur); open the launcher, overview,
   picker, control centre and notification centre with them on, and say which they keep.

0. The user to try live wallpapers: pick `street2.gif` (or a video) in the changer (Mod+W), see
   it play beside windows, plug/unplug (on battery it holds still by default), and say how a
   video looks at 50 % and what `top` shows for ffmpeg; Settings → Wallpaper has the knobs.
0. The user to try the dock: touch the edge by it, pin apps (right-click → Keep in the dock,
   or Settings → Dock), drag to reorder, the trash, the other edges and looks; Mod+D brings it
   out (added to their niri config, previous copy at config.kdl.before-dock-bind).
0. The user to try the day panel: click the clock (or the top-left corner), add a task, tick
   it, write a note, and look at ~/Documents/Asterism/todo.md; Mod+Shift+D opens it (added to
   their niri config, previous copy at config.kdl.before-day-bind).
0. The user to try the previous round: hover a workspace dot, a second pet and the bed/yarn (menu),
   settings search, the VPN switch (their "client"); install slurp/grim/tesseract to try
   copying text off the screen.
1. **User test pass**, above all the lock screen with the real password (Mod+Alt+L), then the
   clipboard picker, timers, night light schedule; fix what breaks.
2. Try Bluetooth pairing for real once the user starts bluetooth.service.
3. The user to try the pet by hand: pick it up (the fixed drag), stroke it, feed it, play with
   the ball, and say how its synthesized sounds sound; the widgets' drag (same fix). If its CPU
   (~2.3 % with the fox) matters: cache the rendered frames of still poses, or a lower frame
   rate while walking (both offered, not chosen).
4. The user to watch a wallpaper change animate (Settings → Wallpaper: Change by itself, or
   Mod+Shift+W) and say how it feels; then the desktop's round two (right-click menus, resizing, photo frame, note,
   edge bars, song card) and the app colours (include the file in kitty.conf, gtk.css).
5. Next from the ETEA/caelestia list (offered 2026-11-21, not chosen yet): a phone link (KDE
   Connect). Done since: workspace previews on hover, the day panel, OCR, Bluetooth battery,
   the VPN tile, settings search. Day panel follow-ups, if asked: due dates or several lists,
   calendar events (an .ics file). Declined as off-theme
   or out of reach: AI chat, Google Lens, stocks/GitHub widgets, a music library, SDDM theme,
   ETEA's planet/orbit and lock scenes.

Control centre follow-ups: keyboard navigation (only Esc today). (The night light tile and the
left hot corner, now the day panel's, exist.)

Wallpaper follow-ups (small, any time): per-output pictures, WebP.

## Future plan (later rounds, roughly in order)

4. **i18n** (French and English, like ETEA); **packaging** (AUR PKGBUILD, desktop/session
    files, a .desktop entry for the settings window).

When starting any of these: write the plan in a short message to the user first, keep the
pure-Rust constraint, add snapshot support for anything visual (`--panel`/`--menu`), add tests
for parsing/protocol logic, and update this file's progress sections when done.
