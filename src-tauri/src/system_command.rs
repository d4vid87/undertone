//! Voice system commands: lock, sleep, shutdown, restart, launch app.
//!
//! Deliberately a deterministic keyword parser, not an LLM call — a
//! mis-transcription must never reboot a machine. Commands only fire on
//! exact leading verbs; anything else falls back to normal dictation.

use std::process::Command;

use log::{error, info};

#[derive(Debug, PartialEq)]
pub enum SystemCommand {
    Lock,
    Sleep,
    Shutdown,
    Restart,
    LaunchApp(String),
}

/// Words that carry no meaning for command matching.
const FILLER: &[&str] = &[
    "the", "my", "this", "please", "computer", "pc", "machine", "system",
];

fn normalize(text: &str) -> Vec<String> {
    text.to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .map(str::to_string)
        .collect()
}

pub fn parse(text: &str) -> Option<SystemCommand> {
    let words = normalize(text);
    if words.is_empty() {
        return None;
    }

    // Launch verbs keep their target verbatim (app names can contain filler
    // words); check before filler-stripping.
    if matches!(words[0].as_str(), "open" | "launch" | "start") && words.len() > 1 {
        let mut target = &words[1..];
        if target[0] == "the" && target.len() > 1 {
            target = &target[1..];
        }
        // "open/start the computer" is not an app launch.
        if !(target.len() == 1 && FILLER.contains(&target[0].as_str())) {
            return Some(SystemCommand::LaunchApp(target.join(" ")));
        }
    }

    let key: Vec<&str> = words
        .iter()
        .map(String::as_str)
        .filter(|w| !FILLER.contains(w))
        .collect();

    match key.as_slice() {
        ["lock"] | ["lock", "screen"] => Some(SystemCommand::Lock),
        ["sleep"] | ["go", "to", "sleep"] | ["suspend"] => Some(SystemCommand::Sleep),
        ["shutdown"] | ["shut", "down"] | ["power", "off"] | ["turn", "off"] => {
            Some(SystemCommand::Shutdown)
        }
        ["restart"] | ["reboot"] => Some(SystemCommand::Restart),
        _ => None,
    }
}

fn run(cmd: &str, args: &[&str]) {
    info!("System command: {cmd} {args:?}");
    if let Err(e) = Command::new(cmd).args(args).spawn() {
        error!("System command '{cmd}' failed: {e}");
    }
}

#[cfg(target_os = "linux")]
pub fn execute(command: SystemCommand) {
    match command {
        SystemCommand::Lock => run("loginctl", &["lock-session"]),
        SystemCommand::Sleep => run("systemctl", &["suspend"]),
        SystemCommand::Shutdown => run("systemctl", &["poweroff"]),
        SystemCommand::Restart => run("systemctl", &["reboot"]),
        SystemCommand::LaunchApp(name) => launch_app_linux(&name),
    }
}

#[cfg(target_os = "windows")]
pub fn execute(command: SystemCommand) {
    match command {
        SystemCommand::Lock => run("rundll32", &["user32.dll,LockWorkStation"]),
        SystemCommand::Sleep => run("rundll32", &["powrprof.dll,SetSuspendState", "0,1,0"]),
        SystemCommand::Shutdown => run("shutdown", &["/s", "/t", "5"]),
        SystemCommand::Restart => run("shutdown", &["/r", "/t", "5"]),
        SystemCommand::LaunchApp(name) => run("cmd", &["/c", "start", "", &name]),
    }
}

#[cfg(target_os = "macos")]
pub fn execute(command: SystemCommand) {
    match command {
        SystemCommand::Lock => run("pmset", &["displaysleepnow"]),
        SystemCommand::Sleep => run("pmset", &["sleepnow"]),
        SystemCommand::Shutdown => run(
            "osascript",
            &["-e", "tell application \"System Events\" to shut down"],
        ),
        SystemCommand::Restart => run(
            "osascript",
            &["-e", "tell application \"System Events\" to restart"],
        ),
        SystemCommand::LaunchApp(name) => run("open", &["-a", &name]),
    }
}

/// Fuzzy-match spoken name against installed .desktop entries, launch via
/// gtk-launch.
// ponytail: substring match on Name=/file stem; swap in real fuzzy scoring
// if collisions annoy anyone.
#[cfg(target_os = "linux")]
fn launch_app_linux(name: &str) {
    let wanted = name.to_lowercase();
    let mut dirs = vec![std::path::PathBuf::from("/usr/share/applications")];
    if let Some(home) = std::env::var_os("HOME") {
        dirs.push(std::path::PathBuf::from(home).join(".local/share/applications"));
    }
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|e| e != "desktop") {
                continue;
            }
            let stem = path
                .file_stem()
                .map(|s| s.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            let display_name = std::fs::read_to_string(&path)
                .ok()
                .and_then(|c| {
                    c.lines()
                        .find(|l| l.starts_with("Name="))
                        .map(|l| l[5..].to_lowercase())
                })
                .unwrap_or_default();
            if stem.contains(&wanted) || display_name.contains(&wanted) {
                run("gtk-launch", &[&stem]);
                return;
            }
        }
    }
    error!("No installed application matched '{name}'");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_power_commands_with_filler() {
        assert_eq!(
            parse("Shut down the computer."),
            Some(SystemCommand::Shutdown)
        );
        assert_eq!(parse("power off"), Some(SystemCommand::Shutdown));
        assert_eq!(
            parse("Turn off my PC, please."),
            Some(SystemCommand::Shutdown)
        );
        assert_eq!(parse("Restart the computer"), Some(SystemCommand::Restart));
        assert_eq!(parse("reboot"), Some(SystemCommand::Restart));
        assert_eq!(parse("Lock the screen"), Some(SystemCommand::Lock));
        assert_eq!(parse("go to sleep"), Some(SystemCommand::Sleep));
    }

    #[test]
    fn parses_app_launch() {
        assert_eq!(
            parse("Open Firefox."),
            Some(SystemCommand::LaunchApp("firefox".into()))
        );
        assert_eq!(
            parse("launch the GNU Image Manipulation Program"),
            Some(SystemCommand::LaunchApp(
                "gnu image manipulation program".into()
            ))
        );
        // "start the computer" is not an app launch or power command
        assert_eq!(parse("start the computer"), None);
    }

    #[test]
    fn ignores_ordinary_speech() {
        assert_eq!(parse("hello there"), None);
        assert_eq!(parse("I want to shut down this conversation now"), None);
        assert_eq!(parse(""), None);
    }
}
