//! The day panel's own things: a to-do list and notes, kept as plain files you can open anywhere
//! (`todo.md`, a Markdown checklist, and `notes.md`) in `[day] folder` (~/Documents/Asterism by
//! default). Reading and writing happen on the services thread; this is the format.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Task {
    pub text: String,
    pub done: bool,
}

/// What the day panel shows: the tasks and the notes, as last read or written.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Day {
    pub tasks: Vec<Task>,
    pub notes: String,
    /// Read from the folder at least once (until then, nothing is written back over it).
    pub loaded: bool,
}

/// A made-up day for pictures (snapshots with `--demo`, or when the folder is empty).
pub fn sample() -> Day {
    let tasks = [("Call the bank", false), ("Water the plants", true), ("Send the slides to Ana", false), ("Book the train for Friday", false)];
    Day { tasks: tasks.iter().map(|(text, done)| Task { text: (*text).into(), done: *done }).collect(), notes: "Ideas for the weekend: the hills if it's clear, else the museum.\nBring the good camera.".into(), loaded: true }
}

/// The folder the files live in.
pub fn folder(configured: &str) -> PathBuf {
    let configured = configured.trim();
    if configured.is_empty() { crate::config::home().join("Documents").join("Asterism") } else { crate::config::expand_home(configured) }
}

/// The tasks in a Markdown checklist: `- [ ] to do`, `- [x] done` (`*` too, any case of x); a
/// plain `- item` is a task not done. Headings, blank lines and anything else are passed over.
pub fn parse_tasks(text: &str) -> Vec<Task> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            let rest = line.strip_prefix("- ").or_else(|| line.strip_prefix("* "))?.trim_start();
            let (done, text) = if let Some(rest) = rest.strip_prefix("[ ]") {
                (false, rest)
            } else if let Some(rest) = rest.strip_prefix("[x]").or_else(|| rest.strip_prefix("[X]")) {
                (true, rest)
            } else {
                (false, rest)
            };
            let text = text.trim();
            (!text.is_empty()).then(|| Task { text: text.to_string(), done })
        })
        .collect()
}

/// The tasks as the checklist file says them.
pub fn write_tasks(tasks: &[Task]) -> String {
    let mut out = String::from("# To do\n\n");
    for task in tasks {
        out.push_str(if task.done { "- [x] " } else { "- [ ] " });
        out.push_str(task.text.trim());
        out.push('\n');
    }
    out
}

/// Both files, read from the folder (missing ones are empty).
pub fn load(dir: &Path) -> Day {
    let tasks = std::fs::read_to_string(dir.join("todo.md")).map(|t| parse_tasks(&t)).unwrap_or_default();
    let notes = std::fs::read_to_string(dir.join("notes.md")).unwrap_or_default();
    Day { tasks, notes: notes.trim_end().to_string(), loaded: true }
}

/// Both files, written to the folder (made if need be).
pub fn save(dir: &Path, day: &Day) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join("todo.md"), write_tasks(&day.tasks))?;
    let mut notes = day.notes.trim_end().to_string();
    if !notes.is_empty() {
        notes.push('\n');
    }
    std::fs::write(dir.join("notes.md"), notes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_checklist() {
        let tasks = parse_tasks("# To do\n\n- [ ] Call the bank\n- [x] Buy milk\n* [X] Water plants\n- plain item\nsome prose\n-   [ ]   spaced  \n- [ ]\n");
        let texts: Vec<(&str, bool)> = tasks.iter().map(|t| (t.text.as_str(), t.done)).collect();
        assert_eq!(texts, [("Call the bank", false), ("Buy milk", true), ("Water plants", true), ("plain item", false), ("spaced", false)]);
    }

    #[test]
    fn writes_what_it_reads() {
        let tasks = vec![Task { text: "One".into(), done: false }, Task { text: "Two".into(), done: true }];
        assert_eq!(parse_tasks(&write_tasks(&tasks)), tasks);
        assert_eq!(write_tasks(&tasks), "# To do\n\n- [ ] One\n- [x] Two\n");
    }

    #[test]
    fn keeps_its_files() {
        let dir = std::env::temp_dir().join(format!("asterism-day-{}", std::process::id()));
        let day = Day { tasks: vec![Task { text: "Plan the week".into(), done: false }], notes: "Line one\nLine two\n\n".into(), loaded: true };
        save(&dir, &day).unwrap();
        let back = load(&dir);
        assert_eq!(back.tasks, day.tasks);
        assert_eq!(back.notes, "Line one\nLine two");
        assert_eq!(std::fs::read_to_string(dir.join("notes.md")).unwrap(), "Line one\nLine two\n");
        // Nothing there yet: empty, not an error
        assert_eq!(load(&dir.join("nowhere")), Day { loaded: true, ..Day::default() });
        let _ = std::fs::remove_dir_all(&dir);
    }
}
