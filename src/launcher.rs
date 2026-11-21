//! The launcher: your apps, found as you type, the ones you open most first. It also does
//! maths (`2*pi`, `=sqrt(2)`), runs commands (`>htop`) and searches the web (`?rust layer shell`).

use crate::model::{Command, Model};
use crate::render::text::{Font, Text};
use crate::theme::Theme;
use crate::ui::{Action, Piece, Prim};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Debug, Clone)]
pub struct Entry {
    /// The desktop file id, like "org.gnome.Nautilus".
    pub id: String,
    pub name: String,
    pub generic: String,
    pub comment: String,
    pub keywords: String,
    pub exec: String,
    pub terminal: bool,
}

#[derive(Default)]
pub struct Apps {
    pub entries: Vec<Entry>,
    stamps: Vec<(PathBuf, Option<SystemTime>)>,
}

fn application_dirs() -> Vec<PathBuf> {
    let home = crate::config::home();
    let data_home = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".local/share"));
    let mut dirs = vec![data_home.join("applications")];
    let shared = std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
    dirs.extend(shared.split(':').filter(|d| !d.is_empty()).map(|d| Path::new(d).join("applications")));
    dirs.dedup();
    dirs
}

fn stamp(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

fn in_path(program: &str) -> bool {
    if program.contains('/') {
        return Path::new(program).exists();
    }
    std::env::var_os("PATH").is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(program).is_file()))
}

/// One desktop file, if it's an app meant to be shown here.
fn parse(id: String, text: &str, desktops: &[String]) -> Option<Entry> {
    let mut fields: HashMap<&str, &str> = HashMap::new();
    let mut main = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            main = line == "[Desktop Entry]";
            continue;
        }
        if main && let Some((key, value)) = line.split_once('=') {
            fields.entry(key.trim()).or_insert(value.trim());
        }
    }
    let yes = |key: &str| fields.get(key).is_some_and(|v| *v == "true");
    if fields.get("Type").is_some_and(|t| *t != "Application") || yes("NoDisplay") || yes("Hidden") {
        return None;
    }
    let listed = |key: &str| fields.get(key).map(|v| v.split(';').filter(|s| !s.is_empty()).map(|s| s.to_lowercase()).collect::<Vec<_>>());
    if let Some(only) = listed("OnlyShowIn")
        && !only.iter().any(|d| desktops.contains(d))
    {
        return None;
    }
    if let Some(not) = listed("NotShowIn")
        && not.iter().any(|d| desktops.contains(d))
    {
        return None;
    }
    if let Some(try_exec) = fields.get("TryExec")
        && !in_path(try_exec)
    {
        return None;
    }
    let exec = fields.get("Exec")?.to_string();
    Some(Entry {
        id,
        name: fields.get("Name")?.to_string(),
        generic: fields.get("GenericName").unwrap_or(&"").to_string(),
        comment: fields.get("Comment").unwrap_or(&"").to_string(),
        keywords: fields.get("Keywords").unwrap_or(&"").replace(';', " "),
        exec,
        terminal: yes("Terminal"),
    })
}

fn walk(dir: &Path, prefix: &str, found: &mut Vec<(String, PathBuf)>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if path.is_dir() {
            // applications/kde/foo.desktop has the id "kde-foo"
            walk(&path, &format!("{prefix}{name}-"), found);
        } else if let Some(stem) = name.strip_suffix(".desktop") {
            found.push((format!("{prefix}{stem}"), path));
        }
    }
}

impl Apps {
    pub fn load() -> Apps {
        let desktops: Vec<String> = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default().split(':').map(|s| s.to_lowercase()).collect();
        let dirs = application_dirs();
        let mut seen = HashSet::new();
        let mut entries = Vec::new();
        // Earlier folders win: your own entries hide the system's
        for dir in &dirs {
            let mut found = Vec::new();
            walk(dir, "", &mut found);
            for (id, path) in found {
                if !seen.insert(id.clone()) {
                    continue;
                }
                if let Ok(text) = std::fs::read_to_string(&path)
                    && let Some(entry) = parse(id, &text, &desktops)
                {
                    entries.push(entry);
                }
            }
        }
        entries.sort_by_key(|e| e.name.to_lowercase());
        Apps { entries, stamps: dirs.iter().map(|d| (d.clone(), stamp(d))).collect() }
    }

    /// Reads them again if an applications folder changed since.
    pub fn refresh(&mut self) {
        if self.stamps.is_empty() || self.stamps.iter().any(|(dir, when)| stamp(dir) != *when) {
            *self = Apps::load();
        }
    }
}

// ---------------------------------------------------------------- maths

/// A small calculator: + - * / % ^, brackets, functions and constants.
pub fn evaluate(text: &str) -> Option<f64> {
    struct Parser<'a> {
        chars: std::iter::Peekable<std::str::Chars<'a>>,
    }
    impl Parser<'_> {
        fn space(&mut self) {
            while self.chars.peek().is_some_and(|c| c.is_whitespace()) {
                self.chars.next();
            }
        }
        fn expression(&mut self) -> Option<f64> {
            let mut value = self.term()?;
            loop {
                self.space();
                match self.chars.peek() {
                    Some('+') => {
                        self.chars.next();
                        value += self.term()?;
                    }
                    Some('-') => {
                        self.chars.next();
                        value -= self.term()?;
                    }
                    _ => return Some(value),
                }
            }
        }
        fn term(&mut self) -> Option<f64> {
            let mut value = self.unary()?;
            loop {
                self.space();
                match self.chars.peek() {
                    Some('*' | '×') => {
                        self.chars.next();
                        value *= self.unary()?;
                    }
                    Some('/' | '÷') => {
                        self.chars.next();
                        value /= self.unary()?;
                    }
                    Some('%') => {
                        self.chars.next();
                        value %= self.unary()?;
                    }
                    _ => return Some(value),
                }
            }
        }
        fn power(&mut self) -> Option<f64> {
            let base = self.atom()?;
            self.space();
            if self.chars.peek() == Some(&'^') {
                self.chars.next();
                return Some(base.powf(self.unary()?));
            }
            Some(base)
        }
        fn unary(&mut self) -> Option<f64> {
            self.space();
            match self.chars.peek() {
                Some('-') => {
                    self.chars.next();
                    Some(-self.unary()?)
                }
                Some('+') => {
                    self.chars.next();
                    self.unary()
                }
                _ => self.power(),
            }
        }
        fn atom(&mut self) -> Option<f64> {
            self.space();
            let c = *self.chars.peek()?;
            if c == '(' {
                self.chars.next();
                let value = self.expression()?;
                self.space();
                return (self.chars.next()? == ')').then_some(value);
            }
            if c.is_ascii_digit() || c == '.' {
                let mut number = String::new();
                while let Some(&c) = self.chars.peek() {
                    if c.is_ascii_digit() || c == '.' {
                        number.push(c);
                        self.chars.next();
                    } else if c == '_' || c == ',' {
                        self.chars.next();
                    } else {
                        break;
                    }
                }
                return number.parse().ok();
            }
            if c.is_alphabetic() {
                let mut name = String::new();
                while let Some(&c) = self.chars.peek() {
                    if c.is_alphanumeric() {
                        name.push(c);
                        self.chars.next();
                    } else {
                        break;
                    }
                }
                let constant = match name.as_str() {
                    "pi" | "π" => Some(std::f64::consts::PI),
                    "e" => Some(std::f64::consts::E),
                    "tau" => Some(std::f64::consts::TAU),
                    _ => None,
                };
                if let Some(value) = constant {
                    return Some(value);
                }
                let argument = self.atom()?;
                return Some(match name.as_str() {
                    "sqrt" => argument.sqrt(),
                    "cbrt" => argument.cbrt(),
                    "sin" => argument.sin(),
                    "cos" => argument.cos(),
                    "tan" => argument.tan(),
                    "asin" => argument.asin(),
                    "acos" => argument.acos(),
                    "atan" => argument.atan(),
                    "ln" => argument.ln(),
                    "log" => argument.log10(),
                    "log2" => argument.log2(),
                    "exp" => argument.exp(),
                    "abs" => argument.abs(),
                    "round" => argument.round(),
                    "floor" => argument.floor(),
                    "ceil" => argument.ceil(),
                    _ => return None,
                });
            }
            None
        }
    }
    let mut parser = Parser { chars: text.chars().peekable() };
    let value = parser.expression()?;
    parser.space();
    (parser.chars.next().is_none() && value.is_finite()).then_some(value)
}

/// 0.1+0.2 shows as 0.3, big and small numbers stay readable.
pub fn format_number(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() < 1e15 {
        return format!("{}", value as i64);
    }
    let text = format!("{:.10}", value);
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text.len() > 18 { format!("{value:e}") } else { text.to_string() }
}

/// Whether a query reads like maths rather than a name.
fn looks_like_maths(query: &str) -> bool {
    query.chars().any(|c| c.is_ascii_digit())
        && (query.chars().any(|c| "+-*/^%×÷(π".contains(c)) || query.contains("pi"))
        && query.chars().all(|c| c.is_ascii_digit() || c.is_alphabetic() || " .,_+-*/^%×÷()π".contains(c))
}

// ---------------------------------------------------------------- searching

#[derive(Debug, Clone, PartialEq)]
pub enum Hit {
    App(usize),
    /// The expression and its result.
    Maths(String, String),
    Run(String),
    Web(String),
    /// The emoji, and its name.
    Emoji(&'static str, &'static str),
    /// A host from ~/.ssh/config.
    Ssh(String),
}

/// Every emoji with its name and group, from Unicode's list (see assets/emoji-LICENSE.txt).
fn emoji() -> &'static [(&'static str, &'static str, &'static str)] {
    static LIST: std::sync::OnceLock<Vec<(&'static str, &'static str, &'static str)>> = std::sync::OnceLock::new();
    LIST.get_or_init(|| {
        include_str!("../assets/emoji.tsv")
            .lines()
            .filter(|l| !l.starts_with('#'))
            .filter_map(|l| {
                let mut parts = l.split('\t');
                Some((parts.next()?, parts.next()?, parts.next().unwrap_or("")))
            })
            .collect()
    })
}

/// The hosts you can reach by name: `Host` lines in ~/.ssh/config, without patterns.
pub fn ssh_hosts(config: &str) -> Vec<String> {
    let mut hosts = Vec::new();
    for line in config.lines() {
        let line = line.trim();
        let Some((key, rest)) = line.split_once(char::is_whitespace) else { continue };
        if !key.eq_ignore_ascii_case("host") {
            continue;
        }
        for host in rest.split_whitespace() {
            if !host.contains(['*', '?', '!']) && !hosts.iter().any(|h| h == host) {
                hosts.push(host.to_string());
            }
        }
    }
    hosts
}

pub struct Launcher {
    pub apps: Apps,
    pub query: String,
    pub selected: usize,
    pub hits: Vec<Hit>,
    counts: HashMap<String, u32>,
    /// How the grid last fell: columns across, and apps on a page.
    pub columns: usize,
    pub per_page: usize,
}

fn counts_path() -> PathBuf {
    let state = std::env::var_os("XDG_STATE_HOME").map(PathBuf::from).unwrap_or_else(|| crate::config::home().join(".local/state"));
    state.join("asterism").join("launches.json")
}

/// How well a query matches a name: whole, start, start of a word, inside, or letters in order.
fn closeness(text: &str, query: &str) -> i32 {
    let text = text.to_lowercase();
    if text.is_empty() {
        return 0;
    }
    if text == query {
        return 1000;
    }
    if text.starts_with(query) {
        return 800;
    }
    if text.split(|c: char| !c.is_alphanumeric()).any(|word| word.starts_with(query)) {
        return 600;
    }
    if text.contains(query) {
        return 400;
    }
    // The words' first letters: "vsc" for Visual Studio Code
    let initials: String = text.split(|c: char| !c.is_alphanumeric()).filter_map(|word| word.chars().next()).collect();
    if query.chars().count() > 1 && initials.starts_with(query) {
        return 500;
    }
    // Letters in order, closer together is better
    let mut chars = text.chars().enumerate();
    let mut gaps = 0;
    let mut last = 0;
    for (n, q) in query.chars().enumerate() {
        match chars.by_ref().find(|(_, c)| *c == q) {
            Some((at, _)) => {
                if n > 0 {
                    gaps += at - last - 1;
                }
                last = at;
            }
            None => return 0,
        }
    }
    // Letters strewn far apart are chance, not a match
    if gaps > query.chars().count() * 2 {
        return 0;
    }
    200 - gaps as i32 * 10
}

impl Launcher {
    pub fn new() -> Self {
        // Counts kept under the old name carry over
        let old = counts_path().parent().and_then(|p| p.parent()).map(|state| state.join("etea-bar").join("launches.json"));
        let text = std::fs::read_to_string(counts_path()).ok().or_else(|| old.and_then(|o| std::fs::read_to_string(o).ok()));
        let counts = text.and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        Self { apps: Apps::default(), query: String::new(), selected: 0, hits: Vec::new(), counts, columns: 6, per_page: 24 }
    }

    pub fn open(&mut self) {
        self.apps.refresh();
        self.query.clear();
        self.search();
    }

    pub fn search(&mut self) {
        self.selected = 0;
        let query = self.query.trim().to_lowercase();
        let mut hits = Vec::new();
        // >ssh: the hosts you know, narrowed by what follows
        if let Some(rest) = self.query.trim_start().strip_prefix(">ssh") {
            let wanted = rest.trim().to_lowercase();
            let config = std::fs::read_to_string(crate::config::home().join(".ssh/config")).unwrap_or_default();
            let mut found: Vec<(i32, String)> =
                ssh_hosts(&config).into_iter().filter_map(|h| if wanted.is_empty() { Some((0, h)) } else { Some((closeness(&h, &wanted), h)).filter(|(s, _)| *s > 0) }).collect();
            found.sort_by_key(|f| std::cmp::Reverse(f.0));
            hits.extend(found.into_iter().map(|(_, h)| Hit::Ssh(h)));
            if hits.is_empty() && !wanted.is_empty() {
                hits.push(Hit::Ssh(rest.trim().to_string()));
            }
            self.hits = hits;
            return;
        }
        if let Some(command) = self.query.trim().strip_prefix('>') {
            hits.push(Hit::Run(command.trim().to_string()));
            self.hits = hits;
            return;
        }
        // :name: emoji, copied when chosen
        if let Some(rest) = self.query.trim_start().strip_prefix(':') {
            let wanted = rest.trim().to_lowercase();
            let mut found: Vec<(i32, usize)> = emoji()
                .iter()
                .enumerate()
                .filter_map(|(i, (_, name, group))| {
                    if wanted.is_empty() {
                        return Some((0, i));
                    }
                    let score = closeness(name, &wanted).max(closeness(group, &wanted) / 3);
                    (score > 0).then_some((score, i))
                })
                .collect();
            found.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
            hits.extend(found.into_iter().map(|(_, i)| Hit::Emoji(emoji()[i].0, emoji()[i].1)));
            self.hits = hits;
            return;
        }
        if let Some(question) = self.query.trim().strip_prefix('?') {
            hits.push(Hit::Web(question.trim().to_string()));
            self.hits = hits;
            return;
        }
        let maths = query.strip_prefix('=').map(str::trim).unwrap_or(&query).to_string();
        if (query.starts_with('=') || looks_like_maths(&query))
            && let Some(value) = evaluate(&maths)
        {
            hits.push(Hit::Maths(maths, format_number(value)));
        }
        let used = |id: &str| self.counts.get(id).copied().unwrap_or(0);
        let mut scored: Vec<(i32, usize)> = self
            .apps
            .entries
            .iter()
            .enumerate()
            .filter_map(|(i, entry)| {
                // Nothing asked: every app in its alphabetical place, so each keeps its spot
                if query.is_empty() {
                    return Some((0, i));
                }
                let bonus = ((used(&entry.id) as f32).ln_1p() * 60.0) as i32;
                let exec = entry.exec.split_whitespace().next().unwrap_or("").rsplit('/').next().unwrap_or("");
                let score = closeness(&entry.name, &query)
                    .max(closeness(&entry.generic, &query) / 2)
                    .max(closeness(&entry.keywords, &query) / 2)
                    .max(closeness(exec, &query) / 2)
                    // A description counts only where one of its words starts with the query
                    .max(Some(closeness(&entry.comment, &query)).filter(|c| *c >= 600).unwrap_or(0) / 3);
                (score > 0).then_some((score + bonus, i))
            })
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        hits.extend(scored.into_iter().map(|(_, i)| Hit::App(i)));
        if hits.is_empty() && !query.is_empty() {
            hits.push(Hit::Web(self.query.trim().to_string()));
        }
        self.hits = hits;
    }

    pub fn type_text(&mut self, text: &str) {
        self.query.extend(text.chars().filter(|c| !c.is_control()));
        self.search();
    }

    pub fn erase(&mut self, word: bool) {
        if word {
            let trimmed = self.query.trim_end().len();
            self.query.truncate(trimmed);
            while self.query.chars().last().is_some_and(|c| !c.is_whitespace()) {
                self.query.pop();
            }
        } else {
            self.query.pop();
        }
        self.search();
    }

    pub fn clear(&mut self) {
        self.query.clear();
        self.search();
    }

    /// Moves the choice: one step wraps round the ends, bigger ones (a row, a page) stop there.
    pub fn step(&mut self, by: i32) {
        if self.hits.is_empty() {
            return;
        }
        let count = self.hits.len() as i32;
        self.selected = if by.abs() == 1 { (self.selected as i32 + by).rem_euclid(count) } else { (self.selected as i32 + by).clamp(0, count - 1) } as usize;
    }

    /// Opens what was chosen. Returns whether the launcher's done.
    pub fn activate(&mut self, index: usize, terminal: &str, search: &str) -> bool {
        let Some(hit) = self.hits.get(index).cloned() else { return false };
        match hit {
            Hit::App(i) => {
                let entry = self.apps.entries[i].clone();
                let command = clean_exec(&entry.exec);
                let command = if entry.terminal { format!("{} -e {command}", terminal_command(terminal)) } else { command };
                spawn_app(&command);
                *self.counts.entry(entry.id).or_insert(0) += 1;
                let path = counts_path();
                if let Some(dir) = path.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                let _ = std::fs::write(path, serde_json::to_string(&self.counts).unwrap_or_default());
            }
            Hit::Maths(_, result) => {
                if crate::config::command_available("wl-copy") {
                    let _ = std::process::Command::new("wl-copy").arg("--").arg(&result).spawn().map(|mut c| std::thread::spawn(move || c.wait()));
                }
            }
            Hit::Run(command) => {
                if !command.is_empty() {
                    spawn_app(&command);
                }
            }
            Hit::Emoji(emoji, _) => {
                if crate::config::command_available("wl-copy") {
                    let _ = std::process::Command::new("wl-copy").arg("--").arg(emoji).spawn().map(|mut c| std::thread::spawn(move || c.wait()));
                }
            }
            Hit::Ssh(host) => {
                // The host name goes to ssh as one word, never through a shell's hands
                let safe: String = host.chars().filter(|c| c.is_alphanumeric() || "._-@:".contains(*c)).collect();
                if !safe.is_empty() {
                    spawn_app(&format!("{} -e ssh {safe}", terminal_command(terminal)));
                }
            }
            Hit::Web(question) => {
                let encoded: String = question
                    .bytes()
                    .map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") })
                    .collect();
                spawn_app(&format!("xdg-open '{}'", search.replace("{}", &encoded).replace('\'', "%27")));
            }
        }
        true
    }
}

/// Exec lines carry field codes (%f, %U…) for files: none are given here.
pub fn clean_exec(exec: &str) -> String {
    let mut out = String::with_capacity(exec.len());
    let mut chars = exec.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '%' {
            if let Some('%') = chars.next() { out.push('%') }
        } else {
            out.push(c);
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn terminal_command(configured: &str) -> String {
    if !configured.is_empty() {
        return configured.to_string();
    }
    if let Ok(term) = std::env::var("TERMINAL")
        && in_path(term.split_whitespace().next().unwrap_or(""))
    {
        return term;
    }
    ["kitty", "foot", "alacritty", "wezterm", "ghostty", "konsole", "gnome-terminal", "xterm"]
        .iter()
        .find(|t| in_path(t))
        .map(|t| t.to_string())
        .unwrap_or_else(|| "xterm".into())
}

/// Starts an app on its own: in a systemd scope of its own when there's systemd (so restarting
/// the shell doesn't take the app with it), in a process group of its own either way.
pub fn spawn_app(command: &str) {
    use std::os::unix::process::CommandExt;
    let mut process = if in_path("systemd-run") {
        let mut p = std::process::Command::new("systemd-run");
        p.args(["--user", "--scope", "--quiet", "--collect", "--slice=app.slice", "--", "sh", "-c", command]);
        p
    } else {
        let mut p = std::process::Command::new("sh");
        p.args(["-c", command]);
        p
    };
    process.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).process_group(0);
    match process.spawn() {
        // Someone has to wait for it, so it doesn't linger as a zombie
        Ok(mut child) => {
            std::thread::spawn(move || child.wait());
        }
        Err(error) => log::warn!("couldn't start {command}: {error}"),
    }
}

// ---------------------------------------------------------------- drawing

/// How much of the screen's width the grid spans, and the narrowest a place in it may be
/// (gaps included); the places stretch to fill the span exactly.
const SPAN: f32 = 0.8;
const MIN_CELL_W: f32 = 144.0;
const CELL_H: f32 = 128.0;
const ICON: f32 = 64.0;
const FIELD_H: f32 = 48.0;

pub struct View<'a> {
    pub theme: &'a Theme,
    pub text: &'a mut Text,
    pub model: &'a Model,
    pub requested: &'a HashSet<String>,
    pub requests: Vec<Command>,
    /// The surface, logical pixels.
    pub w: f32,
    pub h: f32,
}

impl Launcher {
    /// The whole screen: a search field at the top, then a page of apps in a grid, then a dot
    /// per page, joined like the bar's workspaces. Remembers how the grid fell, for the keys.
    pub fn build(&mut self, view: &mut View<'_>) -> Vec<Piece> {
        let t = view.theme;
        let mut pieces = Vec::new();

        // The field: an outlined pill, its words in the middle of it
        let fw = 560f32.min(view.w - 48.0);
        let fx = ((view.w - fw) / 2.0).round();
        let fy = (view.h * 0.07).round().max(28.0);
        let big = Font::new(t.text + 3.0);
        let (_, qh) = view.text.measure("Ag", big);
        let mut field = Piece::new("launcher.query", fw, FIELD_H);
        field.prims.push(Prim::Rect { x: 0.0, y: 0.0, w: fw, h: FIELD_H, radius: FIELD_H / 2.0, color: t.hairline() });
        field.prims.push(Prim::Rect { x: 1.0, y: 1.0, w: fw - 2.0, h: FIELD_H - 2.0, radius: FIELD_H / 2.0 - 1.0, color: t.card_deep });
        field.prims.push(Prim::Icon { x: 18.0, y: (FIELD_H - 20.0) / 2.0, size: 20.0, name: "search".into(), color: t.color_soft });
        let room = fw - 50.0 - 24.0;
        let shown = crate::bar::Ctx::ellipsize_with(view.text, &self.query, big, room);
        let ty = (FIELD_H - qh) / 2.0;
        if self.query.is_empty() {
            field.prims.push(Prim::Text { x: 50.0, y: ty, text: "Search apps".into(), font: big, color: t.color_soft.alpha(0.55) });
        } else {
            field.prims.push(Prim::Text { x: 50.0, y: ty, text: shown.clone(), font: big, color: t.color_text });
        }
        let caret = 50.0 + if self.query.is_empty() { 0.0 } else { view.text.measure(&shown, big).0 + 2.0 };
        field.prims.push(Prim::Rect { x: caret, y: (FIELD_H - 22.0) / 2.0, w: 2.0, h: 22.0, radius: 1.0, color: t.primary });
        field.x = fx;
        field.y = fy;
        field.blocks = true;
        pieces.push(field);

        // How the grid falls on this screen
        let top = fy + FIELD_H + 36.0;
        let bottom = view.h - 72.0;
        let span = (view.w * SPAN).round();
        let columns = ((span / MIN_CELL_W).floor() as usize).max(2);
        let cell_w = span / columns as f32;
        let rows = (((bottom - top) / CELL_H).floor() as usize).max(1);
        self.columns = columns;
        self.per_page = columns * rows;
        let per_page = self.per_page;
        let pages = self.hits.len().div_ceil(per_page).max(1);
        let page = (self.selected / per_page).min(pages - 1);
        let left = ((view.w - span) / 2.0).round();

        let name_font = Font::new(t.text).weight(500);
        let small = Font::new(t.text - 2.0);
        let (_, nh) = view.text.measure("Ag", name_font);
        // A cell is drawn 6px in from its place each side, so the hover layer (which reaches 6px
        // out) and the chosen one's tint fill the place exactly
        let (pw, ph) = (cell_w - 12.0, CELL_H - 8.0);
        for (slot, (index, hit)) in self.hits.iter().enumerate().skip(page * per_page).take(per_page).enumerate() {
            let mut cell = Piece::new(format!("launcher.cell.{slot}"), pw, ph);
            if index == self.selected {
                cell.prims.push(Prim::Rect { x: -6.0, y: 0.0, w: pw + 12.0, h: ph, radius: 12.0, color: t.secondary_container.alpha(0.75) });
            }
            let (title, detail, icon) = match hit {
                Hit::App(i) => {
                    let entry = &self.apps.entries[*i];
                    (entry.name.clone(), String::new(), Some(entry.id.clone()))
                }
                Hit::Maths(expression, result) => (format!("= {result}"), format!("{expression} · Enter copies"), None),
                Hit::Run(command) => (if command.is_empty() { "Run a command".into() } else { command.clone() }, "Run it".into(), None),
                Hit::Web(question) => (if question.is_empty() { "Search the web".into() } else { question.clone() }, "Search the web".into(), None),
                Hit::Emoji(_, name) => (name.to_string(), String::new(), None),
                Hit::Ssh(host) => (host.clone(), "Connect over SSH".into(), None),
            };
            let (ix, iy) = ((pw - ICON) / 2.0, 14.0);
            let picture = icon.as_ref().and_then(|id| view.model.app_icons.get(id).cloned().flatten());
            match (&icon, picture) {
                (_, Some(image)) => cell.prims.push(Prim::Image { x: ix, y: iy, w: ICON, h: ICON, image, tint: None, radius: 0.0 }),
                (Some(id), None) => {
                    if !view.model.app_icons.contains_key(id) && !view.requested.contains(&format!("icon:{id}")) {
                        view.requests.push(Command::AppIcon(id.clone()));
                    }
                    // Until (or unless) it has a picture: a quiet tile with its first letter
                    cell.prims.push(Prim::Rect { x: ix + 4.0, y: iy + 4.0, w: ICON - 8.0, h: ICON - 8.0, radius: 16.0, color: t.container_high });
                    let initial: String = title.chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_default();
                    let font = Font::new(26.0).weight(500);
                    let (iw, ih) = view.text.measure(&initial, font);
                    cell.prims.push(Prim::Text { x: ix + (ICON - iw) / 2.0, y: iy + (ICON - ih) / 2.0, text: initial, font, color: t.color_soft });
                }
                (None, None) if matches!(hit, Hit::Emoji(..)) => {
                    // The emoji itself, big
                    let Hit::Emoji(emoji, _) = hit else { unreachable!() };
                    let font = Font::new(40.0).emoji();
                    let (ew, eh) = view.text.measure(emoji, font);
                    cell.prims.push(Prim::Text { x: ix + (ICON - ew) / 2.0, y: iy + (ICON - eh) / 2.0, text: emoji.to_string(), font, color: t.color_text });
                }
                (None, None) => {
                    let name = match hit {
                        Hit::Maths(..) => "add",
                        Hit::Web(_) => "travel_explore",
                        Hit::Ssh(_) => "plugs-connected",
                        _ => "search",
                    };
                    cell.prims.push(Prim::Rect { x: ix + 4.0, y: iy + 4.0, w: ICON - 8.0, h: ICON - 8.0, radius: 16.0, color: t.primary_container });
                    cell.prims.push(Prim::Icon { x: ix + 18.0, y: iy + 18.0, size: 28.0, name: name.into(), color: t.on_primary_container });
                }
            }
            let title = crate::bar::Ctx::ellipsize_with(view.text, &title, name_font, pw - 8.0);
            let (lw, _) = view.text.measure(&title, name_font);
            let ly = iy + ICON + 10.0;
            cell.prims.push(Prim::Text { x: ((pw - lw) / 2.0).max(4.0), y: ly, text: title, font: name_font, color: t.color_text });
            if !detail.is_empty() {
                let detail = crate::bar::Ctx::ellipsize_with(view.text, &detail, small, pw - 8.0);
                let (dw, _) = view.text.measure(&detail, small);
                cell.prims.push(Prim::Text { x: ((pw - dw) / 2.0).max(4.0), y: ly + nh, text: detail, font: small, color: t.color_soft });
            }
            cell.pad = 8.0;
            cell.highlight = true;
            cell.actions.left = Some(Action::Launch(index));
            // The wheel turns the pages
            cell.actions.up = Some(Action::Move(-(per_page as i32)));
            cell.actions.down = Some(Action::Move(per_page as i32));
            cell.x = (left + (slot % columns) as f32 * cell_w + 6.0).round();
            cell.y = top + (slot / columns) as f32 * CELL_H + 4.0;
            pieces.push(cell);
        }
        if self.hits.is_empty() {
            let text = "No apps match";
            let (w, h) = view.text.measure(text, name_font);
            let mut none = Piece::new("launcher.none", w, h);
            none.prims.push(Prim::Text { x: 0.0, y: 0.0, text: text.into(), font: name_font, color: t.color_soft });
            none.x = ((view.w - w) / 2.0).round();
            none.y = top + 40.0;
            pieces.push(none);
        }

        // A dot per page, joined; the one shown bigger in the accent and ringed
        if pages > 1 {
            let slot = 22.0;
            let x0 = ((view.w - pages as f32 * slot) / 2.0).round();
            for p in 0..pages {
                let mut dot = Piece::new(format!("launcher.page.{p}"), slot, slot);
                let (cx, cy) = (slot / 2.0, slot / 2.0);
                let here = p == page;
                let r = if here { 4.5 } else { 3.0 };
                let thread = t.outline.alpha(0.55);
                if p > 0 {
                    dot.prims.push(Prim::Line { points: vec![(-0.5, cy), (cx - r - 3.0, cy)], width: 1.2, color: thread });
                }
                if p + 1 < pages {
                    dot.prims.push(Prim::Line { points: vec![(cx + r + 3.0, cy), (slot + 0.5, cy)], width: 1.2, color: thread });
                }
                dot.prims.push(Prim::Rect { x: cx - r, y: cy - r, w: r * 2.0, h: r * 2.0, radius: r, color: if here { t.primary } else { t.color_soft } });
                if here {
                    dot.prims.push(Prim::Ring { cx, cy, r: r + 3.2, width: 1.2, value: 1.0, color: t.primary.alpha(0.5) });
                }
                let jump = (p * per_page) as i32 - self.selected as i32;
                dot.actions.left = (!here).then_some(Action::Move(jump));
                dot.actions.up = Some(Action::Move(-(per_page as i32)));
                dot.actions.down = Some(Action::Move(per_page as i32));
                dot.pad = 6.0;
                dot.x = x0 + p as f32 * slot;
                dot.y = (bottom + 8.0).round();
                pieces.push(dot);
            }
        }

        // A word on what else it does
        let hint = if matches!(self.hits.first(), Some(Hit::Emoji(..))) {
            "Enter copies the emoji · Esc closes"
        } else {
            "Type to search · = maths · > a command · >ssh a host · :emoji · ? the web · Esc closes"
        };
        let font = Font::new(t.text - 2.0);
        let (hw, hh) = view.text.measure(hint, font);
        let mut footer = Piece::new("launcher.footer", hw, hh);
        footer.prims.push(Prim::Text { x: 0.0, y: 0.0, text: hint.into(), font, color: t.color_soft.alpha(0.6) });
        footer.x = ((view.w - hw) / 2.0).round();
        footer.y = (view.h - 22.0 - hh).round();
        pieces.push(footer);
        pieces
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn does_maths() {
        assert_eq!(evaluate("1+2*3"), Some(7.0));
        assert_eq!(evaluate("(1+2)*3"), Some(9.0));
        assert_eq!(evaluate("2^3^2"), Some(512.0));
        assert_eq!(evaluate("-2^2"), Some(-4.0));
        assert_eq!(evaluate("sqrt(16)+pi-pi"), Some(4.0));
        assert_eq!(evaluate("10 % 4"), Some(2.0));
        assert_eq!(evaluate("2^-1"), Some(0.5));
        assert_eq!(evaluate("1/0"), None);
        assert_eq!(evaluate("2+"), None);
        assert_eq!(format_number(0.1 + 0.2), "0.3");
        assert_eq!(format_number(42.0), "42");
        assert!(looks_like_maths("2*pi"));
        assert!(!looks_like_maths("firefox"));
        assert!(!looks_like_maths("gimp 2"));
    }

    #[test]
    fn cleans_exec_lines() {
        assert_eq!(clean_exec("firefox %u"), "firefox");
        assert_eq!(clean_exec("app --name=%c %F --percent=100%%"), "app --name= --percent=100%");
    }

    #[test]
    fn finds_emoji_and_hosts() {
        let mut launcher = Launcher::new();
        launcher.query = ":red heart".into();
        launcher.search();
        assert!(matches!(launcher.hits.first(), Some(Hit::Emoji("❤️", _))), "{:?}", launcher.hits.first());
        launcher.query = ":".into();
        launcher.search();
        assert!(launcher.hits.len() > 1500);
        let config = "Host work\n  HostName 10.0.0.2\nHost *.lan !bad pi nas\nhost work\nMatch all\n";
        assert_eq!(ssh_hosts(config), ["work", "pi", "nas"]);
    }

    #[test]
    fn ranks_names() {
        assert!(closeness("Firefox", "fire") > closeness("Thunderbird", "fire"));
        assert!(closeness("Visual Studio Code", "code") > closeness("Visual Studio Code", "vsc"));
        assert!(closeness("Visual Studio Code", "vsc") > 0);
        assert_eq!(closeness("Files", "xyz"), 0);
    }

    #[test]
    fn reads_desktop_files() {
        let text = "[Desktop Entry]\nType=Application\nName=Files\nGenericName=File Manager\nExec=nautilus --new-window %U\nIcon=org.gnome.Nautilus\nKeywords=folder;manager;\n[Desktop Action new]\nName=New Window\n";
        let entry = parse("org.gnome.Nautilus".into(), text, &[]).unwrap();
        assert_eq!(entry.name, "Files");
        assert_eq!(entry.generic, "File Manager");
        assert_eq!(clean_exec(&entry.exec), "nautilus --new-window");
        assert!(parse("x".into(), "[Desktop Entry]\nType=Application\nName=X\nExec=x\nNoDisplay=true\n", &[]).is_none());
        assert!(parse("x".into(), "[Desktop Entry]\nType=Application\nName=X\nExec=x\nOnlyShowIn=KDE;\n", &["niri".into()]).is_none());
    }
}
