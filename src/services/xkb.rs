//! Compositors name keyboard layouts the way xkb describes them ("English (US)"); the bar
//! shows codes ("US"). xkb's own list turns one back into the other.

use std::collections::HashMap;
use std::sync::OnceLock;

#[derive(Default)]
struct Tables {
    /// Description → layout code, variants included (they map to their layout).
    codes: HashMap<String, String>,
    /// Layout code → its description, layouts only.
    names: HashMap<String, String>,
}

fn tables() -> &'static Tables {
    static TABLES: OnceLock<Tables> = OnceLock::new();
    TABLES.get_or_init(|| {
        let mut tables = Tables::default();
        for path in ["/usr/share/X11/xkb/rules/evdev.lst", "/usr/share/xkeyboard-config-2/rules/evdev.lst"] {
            let Ok(text) = std::fs::read_to_string(path) else { continue };
            let mut section = "";
            for line in text.lines() {
                if let Some(name) = line.strip_prefix('!') {
                    section = match name.trim() {
                        "layout" => "layout",
                        "variant" => "variant",
                        _ => "",
                    };
                    continue;
                }
                let Some((code, description)) = line.trim().split_once(char::is_whitespace) else { continue };
                let description = description.trim();
                match section {
                    "layout" => {
                        tables.codes.insert(description.to_string(), code.to_string());
                        tables.names.insert(code.to_string(), description.to_string());
                    }
                    // "intl            us: English (US, intl., with dead keys)": the variant belongs to "us"
                    "variant" => {
                        if let Some((parent, name)) = description.split_once(": ") {
                            tables.codes.entry(name.trim().to_string()).or_insert_with(|| parent.trim().to_string());
                        }
                    }
                    _ => {}
                }
            }
            break;
        }
        tables
    })
}

/// "English (US)" → "us". Unknown names keep their first two letters.
pub fn code_of(name: &str) -> String {
    if let Some(code) = tables().codes.get(name) {
        return code.clone();
    }
    name.chars().filter(|c| c.is_alphabetic()).take(2).collect::<String>().to_lowercase()
}

/// "us" → "English (US)".
pub fn name_of(code: &str) -> String {
    tables().names.get(code).cloned().unwrap_or_else(|| code.to_uppercase())
}
