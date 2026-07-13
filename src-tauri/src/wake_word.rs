//! Hands-free wake word listener.
//!
//! When `wake_word_enabled` is on, a background thread keeps its own
//! microphone stream open, segments speech with Silero VAD, and transcribes
//! each short utterance. If the utterance contains the configured wake word,
//! it toggles a normal transcription (same path as the transcribe hotkey).
//! While a wake-word-started recording is running, the listener watches for
//! trailing silence and toggles the recording off — no key press needed.

use std::collections::VecDeque;
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

use log::{debug, info, warn};
use tauri::{AppHandle, Manager};

use crate::audio_toolkit::{audio::AudioRecorder, SileroVad, VadPolicy, VoiceActivityDetector};
use crate::managers::audio::AudioRecordingManager;
use crate::managers::transcription::TranscriptionManager;
use crate::settings::get_settings;
use crate::TranscriptionCoordinator;

const SAMPLE_RATE: usize = 16_000;
const FRAME_SAMPLES: usize = 480; // 30 ms @ 16 kHz, Silero's frame size
const VAD_THRESHOLD: f32 = 0.5;
const PRE_ROLL_FRAMES: usize = 10; // 300 ms kept before speech onset
const ONSET_FRAMES: usize = 3; // 90 ms of speech = utterance start
const END_SILENCE_FRAMES: usize = 25; // 750 ms of silence = utterance end
const MAX_UTTERANCE_SECS: usize = 6; // wake phrase should be short
const STOP_SILENCE_FRAMES: usize = 67; // ~2 s of silence auto-stops dictation

pub fn spawn(app: AppHandle) {
    std::thread::Builder::new()
        .name("wake-word".into())
        .spawn(move || run(app))
        .ok();
}

/// Lowercase and strip everything but letters/digits/spaces so Whisper
/// punctuation and casing never break the match.
fn normalize(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn toggle_transcription(app: &AppHandle) {
    if let Some(coordinator) = app.try_state::<TranscriptionCoordinator>() {
        coordinator.send_input("transcribe", "wake_word", true, false);
    }
}

fn run(app: AppHandle) {
    let (tx, rx) = mpsc::channel::<Vec<f32>>();
    let mut recorder: Option<AudioRecorder> = None;
    let mut vad: Option<SileroVad> = None;

    // Segmentation state
    let mut pre_roll: VecDeque<Vec<f32>> = VecDeque::with_capacity(PRE_ROLL_FRAMES);
    let mut pending: Vec<f32> = Vec::new(); // partial frame accumulator
    let mut utterance: Vec<f32> = Vec::new();
    let mut speech_run = 0usize;
    let mut silence_run = 0usize;
    let mut collecting = false;
    let mut heard_speech_while_recording = false;
    let mut we_started = false;

    loop {
        let settings = get_settings(&app);

        if !settings.wake_word_enabled {
            if let Some(mut r) = recorder.take() {
                let _ = r.stop();
                let _ = r.close();
                info!("Wake word listener stopped");
            }
            vad = None;
            while rx.try_recv().is_ok() {}
            std::thread::sleep(Duration::from_secs(1));
            continue;
        }

        if recorder.is_none() {
            match open_recorder(&app, tx.clone()) {
                Ok(r) => {
                    recorder = Some(r);
                    info!(
                        "Wake word listener started (wake word: '{}')",
                        settings.wake_word
                    );
                }
                Err(e) => {
                    warn!("Wake word listener failed to open microphone: {e}");
                    std::thread::sleep(Duration::from_secs(5));
                    continue;
                }
            }
        }
        if vad.is_none() {
            match load_vad(&app) {
                Ok(v) => vad = Some(v),
                Err(e) => {
                    warn!("Wake word listener failed to load VAD: {e}");
                    std::thread::sleep(Duration::from_secs(5));
                    continue;
                }
            }
        }

        let Ok(chunk) = rx.recv_timeout(Duration::from_secs(1)) else {
            continue;
        };
        pending.extend_from_slice(&chunk);

        let rm = app.state::<Arc<AudioRecordingManager>>();
        let recording = rm.is_recording();
        if !recording && !we_started {
            heard_speech_while_recording = false;
        }
        if we_started && !recording {
            // Recording ended by other means (hotkey, cancel).
            we_started = false;
            heard_speech_while_recording = false;
        }

        while pending.len() >= FRAME_SAMPLES {
            let frame: Vec<f32> = pending.drain(..FRAME_SAMPLES).collect();
            let is_speech = vad
                .as_mut()
                .and_then(|v| v.is_voice(&frame).ok())
                .unwrap_or(false);

            if recording {
                // Not our listener's job to transcribe while the main pipeline
                // records — only watch for trailing silence to auto-stop a
                // wake-word-started dictation.
                collecting = false;
                utterance.clear();
                if !we_started {
                    continue;
                }
                if is_speech {
                    heard_speech_while_recording = true;
                    silence_run = 0;
                } else {
                    silence_run += 1;
                    if heard_speech_while_recording && silence_run >= STOP_SILENCE_FRAMES {
                        debug!("Wake word: trailing silence, stopping dictation");
                        toggle_transcription(&app);
                        we_started = false;
                        heard_speech_while_recording = false;
                        silence_run = 0;
                    }
                }
                continue;
            }

            if is_speech {
                speech_run += 1;
                silence_run = 0;
            } else {
                silence_run += 1;
                speech_run = 0;
            }

            if !collecting {
                pre_roll.push_back(frame.clone());
                if pre_roll.len() > PRE_ROLL_FRAMES {
                    pre_roll.pop_front();
                }
                if speech_run >= ONSET_FRAMES {
                    collecting = true;
                    utterance.clear();
                    for f in &pre_roll {
                        utterance.extend_from_slice(f);
                    }
                    pre_roll.clear();
                }
                continue;
            }

            utterance.extend_from_slice(&frame);
            let too_long = utterance.len() > MAX_UTTERANCE_SECS * SAMPLE_RATE;
            if silence_run >= END_SILENCE_FRAMES || too_long {
                collecting = false;
                let samples = std::mem::take(&mut utterance);
                // Skip blips shorter than ~400 ms of audio.
                if !too_long && samples.len() >= SAMPLE_RATE * 2 / 5 {
                    check_utterance(&app, samples, &settings.wake_word, &mut we_started);
                }
            }
        }
    }
}

/// Transcribe a finished utterance and start dictation if it contains the
/// wake word.
// ponytail: full Whisper pass per utterance — heavy but reuses the loaded
// model; swap in a dedicated keyword-spotting model if idle GPU churn hurts.
fn check_utterance(app: &AppHandle, samples: Vec<f32>, wake_word: &str, we_started: &mut bool) {
    let keyword = normalize(wake_word);
    if keyword.is_empty() {
        return;
    }
    let tm = app.state::<Arc<TranscriptionManager>>();
    if !tm.is_model_loaded() {
        tm.initiate_model_load();
    }
    match tm.transcribe(samples) {
        Ok(text) => {
            let heard = normalize(&text);
            debug!("Wake word listener heard: '{heard}'");
            if heard.contains(&keyword) {
                info!("Wake word '{wake_word}' detected, starting dictation");
                toggle_transcription(app);
                *we_started = true;
            }
        }
        Err(e) => debug!("Wake word transcription failed: {e}"),
    }
}

fn load_vad(app: &AppHandle) -> anyhow::Result<SileroVad> {
    let path = app
        .path()
        .resolve(
            "resources/models/silero_vad_v4.onnx",
            tauri::path::BaseDirectory::Resource,
        )
        .map_err(|e| anyhow::anyhow!("resolve VAD model: {e}"))?;
    SileroVad::new(path, VAD_THRESHOLD)
}

fn open_recorder(
    _app: &AppHandle,
    tx: mpsc::Sender<Vec<f32>>,
) -> Result<AudioRecorder, Box<dyn std::error::Error>> {
    // ponytail: system default microphone; wire to selected_microphone if
    // anyone runs a non-default mic with wake word.
    let mut recorder = AudioRecorder::new()?.with_audio_callback(move |frame: &[f32]| {
        let _ = tx.send(frame.to_vec());
    });
    recorder.open(None)?;
    recorder.start(VadPolicy::Disabled)?;
    Ok(recorder)
}

#[cfg(test)]
mod tests {
    use super::normalize;

    #[test]
    fn normalize_strips_punctuation_and_case() {
        assert_eq!(normalize(" Hey, Undertone! "), "hey undertone");
        assert_eq!(normalize("Under-tone."), "under tone");
    }
}
