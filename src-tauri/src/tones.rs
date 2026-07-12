//! Tones: detect the active application and inject a matching style hint into
//! the post-process prompt (casual for chat apps, formal for email, etc.).

use crate::settings::AppSettings;
use log::debug;
use std::process::Command;

/// Best-effort active window application name (lowercased). None when
/// detection is unavailable — callers treat that as "no tone hint".
pub fn active_app_name() -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        active_app_name_linux()
    }
    #[cfg(target_os = "windows")]
    {
        // ponytail: Windows detection not wired yet — tones silently off there.
        None
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    {
        None
    }
}

#[cfg(target_os = "linux")]
fn active_app_name_linux() -> Option<String> {
    fn run(cmd: &str, args: &[&str]) -> Option<String> {
        let out = Command::new(cmd).args(args).output().ok()?;
        if !out.status.success() {
            return None;
        }
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if s.is_empty() {
            None
        } else {
            Some(s)
        }
    }

    if crate::utils::is_wayland() {
        // KDE Wayland: kdotool speaks the KWin scripting API.
        if let Some(win) = run("kdotool", &["getactivewindow"]) {
            if let Some(class) = run("kdotool", &["getwindowclassname", &win]) {
                return Some(class.to_lowercase());
            }
        }
        None
    } else {
        // X11: window class of the active window.
        let id = run("xdotool", &["getactivewindow"])?;
        let out = run("xprop", &["-id", &id, "WM_CLASS"])?;
        // WM_CLASS(STRING) = "instance", "Class"
        out.rsplit('"').nth(1).map(|c| c.to_lowercase())
    }
}

/// Returns a style hint sentence for the active app, from the user's
/// app-pattern → tone table. Empty table or unknown app = None.
pub fn tone_hint(settings: &AppSettings) -> Option<String> {
    if !settings.tones_enabled || settings.tones.is_empty() {
        return None;
    }
    let app = active_app_name()?;
    debug!("Tones: active app '{}'", app);
    for rule in &settings.tones {
        if !rule.app_pattern.is_empty() && app.contains(&rule.app_pattern.to_lowercase()) {
            return Some(format!(
                "The user is dictating into {}. Adjust the tone/style accordingly: {}",
                app, rule.style
            ));
        }
    }
    None
}
