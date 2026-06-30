#![allow(dead_code)]
// ═══════════════════════════════════════════════════════════
// ZEUS — Audio Engine for Onboarding
// Plays SFX + TTS via web_sys HtmlAudioElement
// ═══════════════════════════════════════════════════════════

use wasm_bindgen::JsCast;

/// Play an audio file from the /audio/ directory.
/// Volume: 0.0 to 1.0. Returns the audio element for later control (pause/stop).
pub fn play(file: &str, volume: f64) -> Option<web_sys::HtmlAudioElement> {
    let path = format!("/audio/{}", file);
    if let Ok(audio) = web_sys::HtmlAudioElement::new_with_src(&path) {
        audio.set_volume(volume);
        let _ = audio.play();
        Some(audio)
    } else {
        None
    }
}

/// Play a looping ambient sound. Returns the element so caller can stop it.
pub fn play_loop(file: &str, volume: f64) -> Option<web_sys::HtmlAudioElement> {
    let path = format!("/audio/{}", file);
    if let Ok(audio) = web_sys::HtmlAudioElement::new_with_src(&path) {
        audio.set_volume(volume);
        audio.set_loop(true);
        let _ = audio.play();
        Some(audio)
    } else {
        None
    }
}

/// Fade out and stop an audio element over duration_ms.
pub fn fade_out(audio: &web_sys::HtmlAudioElement, duration_ms: u32) {
    let audio = audio.clone();
    let steps = 20u32;
    let interval = duration_ms / steps;
    let initial_vol = audio.volume();
    let step_vol = initial_vol / steps as f64;

    if let Some(win) = web_sys::window() {
        for i in 1..=steps {
            let audio_c = audio.clone();
            let vol = (initial_vol - step_vol * i as f64).max(0.0);
            let is_last = i == steps;
            let cb = wasm_bindgen::closure::Closure::once(move || {
                audio_c.set_volume(vol);
                if is_last {
                    audio_c.pause().ok();
                }
            });
            let _ = win.set_timeout_with_callback_and_timeout_and_arguments_0(
                cb.as_ref().unchecked_ref(),
                (interval * i) as i32,
            );
            cb.forget();
        }
    }
}

/// Get the TTS filename for a given onboarding step transition.
pub fn tts_for_step(leaving_step: usize) -> &'static str {
    match leaving_step {
        0 => "tts_step0_init.mp3",
        1 => "tts_step1_identity.mp3",
        2 => "tts_step2_intelligence.mp3",
        3 => "tts_step3_model.mp3",
        4 => "tts_step4_channels.mp3",
        5 => "tts_step5_security.mp3",
        6 => "tts_step6_features.mp3",
        7 => "tts_step7_alive.mp3",
        _ => "sfx_click.mp3",
    }
}
