//! Who this copy of Ovrtone is licensed to: the name and licence ID the installer wrote, shown in
//! Settings and About. A label only: nothing is locked or checked by it.

use std::sync::OnceLock;

/// The Name and Id of `license.ini` in the settings folder, as the installer wrote them.
fn fields() -> &'static (Option<String>, Option<String>) {
    static FIELDS: OnceLock<(Option<String>, Option<String>)> = OnceLock::new();
    FIELDS.get_or_init(|| {
        let Some(text) = crate::recent_files::config_dir()
            .and_then(|dir| std::fs::read_to_string(dir.join("license.ini")).ok())
        else {
            return (None, None);
        };
        let field = |key: &str| {
            text.lines()
                .find_map(|line| line.trim().strip_prefix(key))
                .map(|v| v.trim().chars().take(80).collect::<String>())
                .filter(|v| !v.is_empty())
        };
        (field("Name="), field("Id="))
    })
}

/// The name the installer was given, if any.
pub fn licensed_to() -> Option<&'static str> {
    fields().0.as_deref()
}

/// The licence ID the installer made, such as OVR-7K2M-QX9D-4HTB.
pub fn id() -> Option<&'static str> {
    fields().1.as_deref()
}

/// "Licensed to Name by Ovrtone (ID ...)", or that it was installed without a name.
pub fn line() -> String {
    let id = id().map(|id| format!(" (ID {id})")).unwrap_or_default();
    match licensed_to() {
        Some(name) => format!("Licensed to {name} by Ovrtone{id}"),
        None if !id.is_empty() => format!("Not registered to a name{id}"),
        None => "Not registered to a name".to_string(),
    }
}
