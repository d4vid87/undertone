//! Command mode: speak an instruction, apply it to the currently selected text.
//!
//! Flow: on shortcut press we snapshot the active selection (simulated copy +
//! clipboard read), record the spoken instruction as usual, then send both to
//! the configured post-process LLM and paste the rewritten text over the
//! selection.

use crate::input::EnigoState;
use crate::settings::AppSettings;
use enigo::{Direction, Enigo, Key, Keyboard};
use log::{debug, error, info};
use std::process::Command;
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Manager};
use tauri_plugin_clipboard_manager::ClipboardExt;

#[cfg(target_os = "linux")]
use crate::utils::is_wayland;

/// Selection captured when the command-mode shortcut was pressed, consumed
/// when the transcription finishes.
static CAPTURED_SELECTION: Mutex<Option<String>> = Mutex::new(None);

pub fn take_captured_selection() -> Option<String> {
    CAPTURED_SELECTION.lock().ok().and_then(|mut s| s.take())
}

/// Snapshot the current selection by simulating a copy keystroke and reading
/// the clipboard. The previous clipboard content is restored afterwards.
pub fn capture_selection(app: &AppHandle) {
    let clipboard = app.clipboard();
    let previous = clipboard.read_text().unwrap_or_default();

    // Clear so we can tell whether the copy produced anything.
    let _ = clipboard.write_text(String::new());

    if let Err(e) = send_copy_combo(app) {
        error!("Command mode: failed to send copy keystroke: {}", e);
    }
    std::thread::sleep(Duration::from_millis(150));

    let selection = clipboard.read_text().unwrap_or_default();
    let _ = clipboard.write_text(previous);

    if selection.trim().is_empty() {
        info!("Command mode: no text selected");
        if let Ok(mut slot) = CAPTURED_SELECTION.lock() {
            *slot = None;
        }
    } else {
        debug!(
            "Command mode: captured {} chars of selection",
            selection.len()
        );
        if let Ok(mut slot) = CAPTURED_SELECTION.lock() {
            *slot = Some(selection);
        }
    }
}

fn send_copy_combo(app: &AppHandle) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    {
        if try_send_copy_linux()? {
            return Ok(());
        }
    }

    let enigo_state = app
        .try_state::<EnigoState>()
        .ok_or("Enigo state unavailable")?;
    let mut enigo = enigo_state
        .0
        .lock()
        .map_err(|e| format!("Enigo lock poisoned: {}", e))?;
    send_copy_enigo(&mut enigo)
}

fn send_copy_enigo(enigo: &mut Enigo) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let (modifier, c_key) = (Key::Meta, Key::Other(8));
    #[cfg(target_os = "windows")]
    let (modifier, c_key) = (Key::Control, Key::Other(0x43)); // VK_C
    #[cfg(target_os = "linux")]
    let (modifier, c_key) = (Key::Control, Key::Unicode('c'));

    enigo
        .key(modifier, Direction::Press)
        .map_err(|e| format!("Failed to press modifier: {}", e))?;
    enigo
        .key(c_key, Direction::Click)
        .map_err(|e| format!("Failed to click C: {}", e))?;
    std::thread::sleep(Duration::from_millis(100));
    enigo
        .key(modifier, Direction::Release)
        .map_err(|e| format!("Failed to release modifier: {}", e))?;
    Ok(())
}

/// Mirrors clipboard.rs's Linux native-tool fallback chain for the copy combo.
/// Returns Ok(true) if a native tool handled it.
#[cfg(target_os = "linux")]
fn try_send_copy_linux() -> Result<bool, String> {
    fn available(cmd: &str) -> bool {
        Command::new("which")
            .arg(cmd)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
    fn run(cmd: &str, args: &[&str]) -> Result<(), String> {
        let status = Command::new(cmd)
            .args(args)
            .status()
            .map_err(|e| format!("{} failed: {}", cmd, e))?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("{} exited with {}", cmd, status))
        }
    }

    if is_wayland() {
        if available("dotool") {
            use std::io::Write;
            let mut child = Command::new("dotool")
                .stdin(std::process::Stdio::piped())
                .spawn()
                .map_err(|e| format!("dotool failed: {}", e))?;
            if let Some(stdin) = child.stdin.as_mut() {
                let _ = stdin.write_all(b"key ctrl+c\n");
            }
            let _ = child.wait();
            return Ok(true);
        }
        if available("ydotool") {
            // 29 = KEY_LEFTCTRL, 46 = KEY_C
            run("ydotool", &["key", "29:1", "46:1", "46:0", "29:0"])?;
            return Ok(true);
        }
    } else if available("xdotool") {
        run("xdotool", &["key", "--clearmodifiers", "ctrl+c"])?;
        return Ok(true);
    }
    Ok(false)
}

const COMMAND_PROMPT: &str = r#"You are a text editing assistant. Apply the instruction to the text and return ONLY the edited text.

Rules:
- Return only the resulting text: no explanations, no preamble, no quotes, no markdown fences.
- Preserve the original formatting style (line breaks, indentation, markup) unless the instruction says otherwise.
- If the instruction is unclear or inapplicable, return the original text unchanged.

<instruction>
${instruction}
</instruction>

<text>
${text}
</text>"#;

/// Apply the spoken `instruction` to `selection` via the configured
/// post-process LLM provider. Returns None (caller falls back) on any failure.
pub async fn apply_command(
    settings: &AppSettings,
    selection: &str,
    instruction: &str,
) -> Option<String> {
    let provider = settings.active_post_process_provider().cloned()?;
    let model = settings.post_process_models.get(&provider.id).cloned()?;
    if model.trim().is_empty() {
        debug!(
            "Command mode: provider '{}' has no model configured",
            provider.id
        );
        return None;
    }
    let api_key = settings
        .post_process_api_keys
        .get(&provider.id)
        .cloned()
        .unwrap_or_default();

    let prompt = COMMAND_PROMPT
        .replace("${instruction}", instruction)
        .replace("${text}", selection);

    let reasoning_effort = if provider.id == "custom" {
        Some("none".to_string())
    } else {
        None
    };

    match crate::llm_client::send_chat_completion(
        &provider,
        api_key,
        &model,
        prompt,
        reasoning_effort,
        None,
    )
    .await
    {
        Ok(Some(content)) => {
            let content = content.trim();
            if content.is_empty() {
                None
            } else {
                Some(content.to_string())
            }
        }
        Ok(None) => {
            error!("Command mode: LLM returned no content");
            None
        }
        Err(e) => {
            error!("Command mode: LLM request failed: {}", e);
            None
        }
    }
}
