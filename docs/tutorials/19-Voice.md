# Voice — Speech-to-Text and Text-to-Speech

Zeus supports voice interaction through multiple STT and TTS providers. Convert speech to text for input, generate spoken responses, and even run voice calls.

## TTS Providers

Zeus supports 5 text-to-speech backends:

| Provider | Config Key | Notes |
|----------|-----------|-------|
| **Piper HTTP** | `piper_url` | Self-hosted, fast, no API key needed |
| **OpenAI** | `OPENAI_API_KEY` | tts-1 / tts-1-hd, 6 voices |
| **ElevenLabs** | `ELEVENLABS_API_KEY` | High-quality, many voices |
| **Edge TTS** | (none) | Free Microsoft Edge TTS, requires `edge-tts` CLI |
| **Local macOS** | (none) | Built-in `say` command |

### Configuration

In `~/.zeus/config.toml`:

```toml
[tts]
provider = "piper"           # piper | openai | elevenlabs | edge | local
piper_url = "https://piper.novaxai.ai"
voice = "default"            # Provider-specific voice name
```

Or via environment:
```bash
export OPENAI_API_KEY="sk-..."
export ELEVENLABS_API_KEY="..."
```

### Generating Speech (CLI)

```bash
# Using macOS say command (quick and local)
say "Hello from Zeus" -o /tmp/hello.aiff

# The gateway can generate TTS via the voice subsystem
```

### Generating Speech (API)

```bash
curl -X POST http://localhost:3001/v1/tts \
  -H "Content-Type: application/json" \
  -d '{"text":"Hello from Zeus","provider":"piper"}' \
  --output /tmp/response.wav
```

## STT Providers

| Provider | Endpoint | Notes |
|----------|----------|-------|
| **Whisper (self-hosted)** | `wsp.novaxai.ai` | whisper.cpp server, no API key |
| **OpenAI Whisper** | OpenAI API | Requires `OPENAI_API_KEY` |

### Transcribing Audio

Audio files must be WAV format (16kHz mono) for best results:

```bash
# Convert OGG to WAV first
ffmpeg -y -i input.ogg -ar 16000 -ac 1 output.wav

# Transcribe with self-hosted Whisper
curl -X POST https://wsp.novaxai.ai/inference \
  -F "file=@output.wav" \
  -F "response_format=json"
```

Response:
```json
{"text": "Hello, this is a test message"}
```

### Transcribing via Zeus API

```bash
curl -X POST http://localhost:3001/v1/stt \
  -F "file=@audio.wav" \
  -F "response_format=json"
```

## Voice Messages on Discord

Zeus agents can send and receive Discord voice messages:

1. **Receiving**: OGG voice messages from Discord are automatically converted to WAV and transcribed via Whisper
2. **Sending**: Generate audio with `say` or TTS, then send as a voice message attachment — OGG conversion happens automatically

```bash
# Generate voice message
say -o /tmp/reply.aiff "Your build passed all tests"

# Send as Discord voice message (auto-converts to OGG/Opus)
# Use the send_file tool or message tool with attachment
```

## Voice Call System

Zeus includes a full voice call system with telephony integration:

### Supported Providers

| Provider | Use Case |
|----------|----------|
| **Twilio** | Inbound/outbound phone calls |
| **Telnyx** | Alternative telephony provider |
| **Plivo** | Third telephony option |

### Voice Call Flow

1. Incoming call arrives via webhook
2. Zeus answers and streams audio
3. Voice Activity Detection (VAD) detects speech boundaries
4. Audio sent to STT for transcription
5. Transcribed text processed by Zeus agent
6. Response generated, converted to speech via TTS
7. Audio streamed back to caller

### Configuration

```toml
[voice]
provider = "twilio"
twilio_account_sid = "AC..."
twilio_auth_token = "..."
twilio_phone_number = "+1..."

[voice.vad]
threshold = 0.5          # Voice activity detection sensitivity
silence_duration_ms = 800 # Silence before end-of-speech
```

### Wake Word Detection

Zeus supports wake word detection for hands-free activation:

```toml
[voice]
wake_word = "hey zeus"
wake_word_sensitivity = 0.7
```

## Talk Mode

Interactive voice conversation mode where Zeus listens continuously:

```bash
zeus talk
```

In talk mode:
- Zeus listens through your microphone
- Detects when you start/stop speaking (VAD)
- Transcribes your speech
- Generates a response
- Speaks the response back
- Loops until you say "goodbye" or press Ctrl+C

## Audio Format Notes

- **Input**: WAV (16kHz, mono) is the universal format for STT
- **Discord**: OGG/Opus for voice messages (auto-converted)
- **TTS output**: Varies by provider (WAV, MP3, OGG)
- **Convert anything to WAV**: `ffmpeg -y -i input.ext -ar 16000 -ac 1 output.wav`

## What's Next

→ [[17-macOS-Automation]] — System automation tools
→ [[09-Channels]] — Messaging integrations
→ [[04-Chat-and-Conversations]] — Text-based interaction
