// ═══════════════════════════════════════════════════════════
// ZEUS — Voice Page — TTS/STT Configuration & Testing
// ═══════════════════════════════════════════════════════════

use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;
use crate::api;
use crate::components::design::*;

#[component]
pub fn VoicePage() -> impl IntoView {
    let tts_text = RwSignal::new("Hello, I am Zeus. Your cognitive platform is online.".to_string());
    let tts_status = RwSignal::new(String::new());
    let stt_status = RwSignal::new(String::new());
    let stt_result = RwSignal::new(String::new());
    let config = RwSignal::new(api::ConfigResponse::default());
    let voice_count = RwSignal::new(0usize);
    let provider_count = RwSignal::new(0usize);
    let loading = RwSignal::new(true);

    // Fetch config + voice providers/voices in parallel
    {
        spawn_local(async move {
            if let Ok(c) = api::fetch_config().await {
                config.set(c);
            }
            loading.set(false);
        });
    }
    {
        spawn_local(async move {
            if let Ok(v) = api::fetch_tts_voices().await {
                voice_count.set(v.voices.len());
            }
        });
    }
    {
        spawn_local(async move {
            if let Ok(p) = api::fetch_tts_providers().await {
                provider_count.set(p.providers.len());
            }
        });
    }

    let test_tts = move |_| {
        let text = tts_text.get();
        if text.trim().is_empty() { return; }
        tts_status.set("Synthesizing...".to_string());
        spawn_local(async move {
            match api::tts_synthesize(&text).await {
                Ok(audio_bytes) => {
                    tts_status.set(format!("Generated {} bytes audio", audio_bytes.len()));
                    // Play audio via Web Audio API
                    if !audio_bytes.is_empty() {
                        let array = js_sys::Uint8Array::new_with_length(audio_bytes.len() as u32);
                        array.copy_from(&audio_bytes);
                        let blob_parts = js_sys::Array::new();
                        blob_parts.push(&array.buffer());
                        let blob = web_sys::Blob::new_with_u8_array_sequence_and_options(
                            &blob_parts,
                            web_sys::BlobPropertyBag::new().type_("audio/mp3"),
                        ).ok();
                        if let Some(blob) = blob {
                            let url = web_sys::Url::create_object_url_with_blob(&blob).ok();
                            if let Some(url) = url
                                && let Ok(audio) = web_sys::HtmlAudioElement::new_with_src(&url) {
                                    let _ = audio.play();
                                    tts_status.set("Playing audio...".to_string());
                                }
                        }
                    }
                }
                Err(e) => tts_status.set(format!("Error: {}", e)),
            }
        });
    };

    view! {
        <div style="padding: 32px;">
            <div style="margin-bottom: 24px;">
                <h1 style="font-family: 'Orbitron', monospace; font-size: 14px; letter-spacing: 6px; color: rgba(255,245,240,0.9); margin: 0;">"VOICE"</h1>
                <p style="color: rgba(255,245,240,0.7); font-size: 12px;">"Speech-to-text and text-to-speech configuration"</p>
            </div>

            // Voice capabilities overview
            <SectionTitle>"CAPABILITIES"</SectionTitle>
            <div style="display: flex; gap: 12px; margin-bottom: 24px; flex-wrap: wrap;">
                <MetricCard label="TTS ENGINE" value="Piper".to_string() icon="speaker" />
                <MetricCard label="STT ENGINE" value="Whisper".to_string() icon="mic" />
                {move || {
                    let vc = voice_count.get();
                    let label = if vc == 0 { "...".to_string() } else { vc.to_string() };
                    view! { <MetricCard label="VOICES" value=label icon="users" /> }
                }}
                {move || {
                    let pc = provider_count.get();
                    let status = if loading.get() { "...".to_string() }
                        else if pc > 0 { "ONLINE".to_string() }
                        else { "OFFLINE".to_string() };
                    view! { <MetricCard label="STATUS" value=status icon="check" /> }
                }}
            </div>

            // TTS Test
            <SectionTitle>"TEXT-TO-SPEECH TEST"</SectionTitle>
            <Card style="margin-bottom: 24px;">
                <div style="display: flex; flex-direction: column; gap: 10px;">
                    <textarea
                        prop:value=move || tts_text.get()
                        on:input=move |ev| tts_text.set(event_target_value(&ev))
                        rows=3
                        placeholder="Enter text to synthesize..."
                        style="background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.1); border-radius: 6px; padding: 10px 14px; color: rgba(255,245,240,0.9); font-size: 13px; font-family: 'Rajdhani', sans-serif; resize: vertical;"
                    />
                    <div style="display: flex; align-items: center; justify-content: space-between;">
                        <Button primary=true on_click=Some(Callback::new(test_tts))>
                            <Icon name="speaker" size=14 /> " Synthesize & Play"
                        </Button>
                        {move || {
                            let status = tts_status.get();
                            (!status.is_empty()).then(|| view! {
                                <span style="font-size: 11px; color: rgba(255,245,240,0.7);">{status}</span>
                            })
                        }}
                    </div>
                </div>
            </Card>

            // STT section
            <SectionTitle>"SPEECH-TO-TEXT"</SectionTitle>
            <Card style="margin-bottom: 24px;">
                <div style="text-align: center; padding: 24px;">
                    <div style="width: 60px; height: 60px; border-radius: 50%; background: rgba(255,60,20,0.15); border: 2px solid rgba(255,60,20,0.15); display: flex; align-items: center; justify-content: center; margin: 0 auto 12px;">
                        <Icon name="mic" size=24 />
                    </div>
                    <div style="font-family: 'Orbitron', monospace; font-size: 10px; letter-spacing: 2px; color: rgba(255,245,240,0.5); margin-bottom: 8px;">"WHISPER STT"</div>
                    <div style="font-size: 12px; color: rgba(255,245,240,0.5);">
                        "Use the microphone in Agent Studio for live speech-to-text transcription"
                    </div>
                    {move || {
                        let result = stt_result.get();
                        (!result.is_empty()).then(|| view! {
                            <div style="margin-top: 12px; padding: 10px; background: rgba(255,255,255,0.03); border-radius: 6px; font-size: 12px; color: rgba(255,245,240,0.5);">
                                {result}
                            </div>
                        })
                    }}
                    {move || {
                        let status = stt_status.get();
                        (!status.is_empty()).then(|| view! {
                            <div style="margin-top: 8px; font-size: 10px; color: rgba(255,245,240,0.5);">{status}</div>
                        })
                    }}
                </div>
            </Card>

            // Voice providers info
            <SectionTitle>"VOICE PROVIDERS"</SectionTitle>
            <div style="display: grid; grid-template-columns: repeat(auto-fill, minmax(280px, 1fr)); gap: 12px;">
                <Card>
                    <div style="display: flex; align-items: center; gap: 10px; margin-bottom: 8px;">
                        <div style="width: 32px; height: 32px; border-radius: 6px; background: rgba(34,197,94,0.1); display: flex; align-items: center; justify-content: center;">
                            <span style="font-family: 'Orbitron', monospace; font-size: 10px; color: #22c55e;">"PI"</span>
                        </div>
                        <div>
                            <div style="font-size: 13px; font-weight: 500; color: rgba(255,245,240,0.9);">"Piper TTS"</div>
                            <div style="font-size: 10px; color: rgba(255,245,240,0.5);">"Local neural TTS — 8 voices"</div>
                        </div>
                        <StatusDot status="connected".to_string() />
                    </div>
                    <div style="font-size: 10px; color: rgba(255,245,240,0.5);">"Endpoint: 192.168.1.249:8104"</div>
                </Card>

                <Card>
                    <div style="display: flex; align-items: center; gap: 10px; margin-bottom: 8px;">
                        <div style="width: 32px; height: 32px; border-radius: 6px; background: rgba(59,130,246,0.1); display: flex; align-items: center; justify-content: center;">
                            <span style="font-family: 'Orbitron', monospace; font-size: 10px; color: #3b82f6;">"WH"</span>
                        </div>
                        <div>
                            <div style="font-size: 13px; font-weight: 500; color: rgba(255,245,240,0.9);">"Whisper STT"</div>
                            <div style="font-size: 10px; color: rgba(255,245,240,0.5);">"Speech recognition"</div>
                        </div>
                        <StatusDot status="connected".to_string() />
                    </div>
                    <div style="font-size: 10px; color: rgba(255,245,240,0.5);">"Endpoint: configured via ZEUS_WHISPER_URL"</div>
                </Card>

                <Card>
                    <div style="display: flex; align-items: center; gap: 10px; margin-bottom: 8px;">
                        <div style="width: 32px; height: 32px; border-radius: 6px; background: rgba(234,179,8,0.1); display: flex; align-items: center; justify-content: center;">
                            <span style="font-family: 'Orbitron', monospace; font-size: 10px; color: #eab308;">"TW"</span>
                        </div>
                        <div>
                            <div style="font-size: 13px; font-weight: 500; color: rgba(255,245,240,0.9);">"Twilio Voice"</div>
                            <div style="font-size: 10px; color: rgba(255,245,240,0.5);">"Outbound voice calls"</div>
                        </div>
                        <StatusDot status="idle".to_string() />
                    </div>
                    <div style="font-size: 10px; color: rgba(255,245,240,0.5);">"Configure in Settings → API Keys"</div>
                </Card>
            </div>
        </div>
    }
}
