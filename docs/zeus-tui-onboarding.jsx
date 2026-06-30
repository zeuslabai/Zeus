import { useState, useEffect, useRef } from "react";

/* ═══ PALETTE ═══ */
const C = {
  bg: "#0a0a0f", bg2: "#12100e", bg3: "#1a1610",
  fg: "#d4cfc8", dim: "#5a5650", muted: "#3a3632", dark: "#2a2420",
  accent: "#ff3c14", accentDim: "#a0301a", accentBright: "#ff6842",
  accentFaint: "#401008", accentSoft: "rgba(255,60,20,0.05)",
  green: "#22c55e", greenDim: "#1a4a2e",
  yellow: "#eab308", yellowDim: "#6b5a10",
  blue: "#3b82f6", blueDim: "#1a2a4a",
  cyan: "#06b6d4", cyanDim: "#164e63",
  red: "#ef4444", redDim: "#4a1a1a",
  amber: "#ffa050", amberDim: "#5a3010",
  purple: "#a855f7", purpleDim: "#3a1a4a",
  white: "#f0ece6",
};

/* ═══ ZEUS FACE ═══
   Animated ASCII face — ported from the production TUI for brand parity.
   States: ready, thinking, working, success, error, listening, sleeping, alert, queued, tool
*/
const FACE_FRAMES = {
  ready:     ["(◉‿◉)", "(◉‿◉)", "(◉‿◉)", "(◉‿◉)", "(-‿-)", "(◉‿◉)", "(◉‿◉)", "(◉‿◉)"],
  listening: ["(◉_◉)", "(◉_◉)", "(-_◉)", "(◉_◉)"],
  thinking:  ["(◉.◉)", "(◔.◉)", "(◔.◔)", "(◉.◔)", "(◉.◉)", "(◔ ◔)", "(- -)", "(◉.◉)"],
  working:   ["(◣_◢)", "(◢_◣)", "(◣_◢)", "(◢_◣)", "(▰_▰)", "(◣_◢)", "(◢_◣)", "(▰_▰)"],
  tool:      ["[◉_◉]", "[◉.◉]", "[◉_◉]", "[◉.◉]", "[●_●]", "[◉_◉]", "[◉.◉]", "[◉_◉]"],
  success:   ["(◉‿◉)✓", "(^‿^)✓", "(◉‿◉)✓", "(^‿^)✓"],
  error:     ["(✕_✕)", "(✕.✕)", "(✕_✕)", "(>_<)", "(✕_✕)", "(>_<)", "(✕_✕)", "(✕.✕)"],
  alert:     ["(◉ω◉)!", "(◉ω◉)!", "(◉_◉)!", "(◉ω◉)!"],
  queued:    ["(◔‿◉)", "(◉‿◔)", "(◔‿◉)", "(◉‿◔)"],
  sleeping:  ["(-_-) z", "(-_-) zZ", "(-_-) zZz", "(-_-) zZ"],
};
const FACE_COLORS = {
  ready: "#ff3c14", listening: "#06b6d4", thinking: "#ffa050", working: "#ffa050",
  tool: "#06b6d4", success: "#22c55e", error: "#ef4444", alert: "#eab308",
  queued: "#ffa050", sleeping: "#5a5650",
};
const ZeusFace = ({ state = "ready", small = false, label = null, speed = 250 }) => {
  const [frame, setFrame] = useState(0);
  const frames = FACE_FRAMES[state] || FACE_FRAMES.ready;
  const color = FACE_COLORS[state] || C.accent;
  useEffect(() => {
    setFrame(0);
    const t = setInterval(() => setFrame(f => (f + 1) % frames.length), speed);
    return () => clearInterval(t);
  }, [state, frames.length, speed]);
  return (
    <span style={{
      display: "inline-flex", alignItems: "center", gap: 6,
      fontFamily: "inherit", fontSize: small ? 11 : 14,
      color, fontWeight: 700, whiteSpace: "pre",
      textShadow: `0 0 8px ${color}55`,
    }}>
      <span style={{ minWidth: small ? 52 : 66, display: "inline-block" }}>{frames[frame]}</span>
      {label && <span style={{ fontSize: small ? 9 : 10, color, fontWeight: 600, fontStyle: "italic", opacity: 0.85 }}>{label}</span>}
    </span>
  );
};

/* ═══ ASCII LOGO ═══ */
const LOGO = [
  "██████╗ ███████╗██╗   ██╗███████╗",
  "╚════██╗██╔════╝██║   ██║██╔════╝",
  "  ███╔═╝█████╗  ██║   ██║███████╗",
  " ██╔══╝ ██╔══╝  ██║   ██║╚════██║",
  "███████╗███████╗╚██████╔╝███████║",
  "╚══════╝╚══════╝ ╚═════╝ ╚══════╝",
];
const LOGO_COLORS = [C.accent, C.accent, C.accentBright, C.accentBright, C.accentDim, C.muted];

/* ═══ STEP DEFINITIONS ═══ */
const STEPS = [
  { id: "welcome", code: "WLCM", name: "Welcome", required: true, optional: false },
  { id: "mode", code: "MODE", name: "Setup Mode", required: true, optional: false },
  { id: "provider", code: "PROV", name: "LLM Provider", required: true, optional: false },
  { id: "auth", code: "AUTH", name: "Auth", required: true, optional: false },
  { id: "model", code: "MODL", name: "Model", required: true, optional: false },
  { id: "fallback", code: "FLBK", name: "Backup LLMs", required: false, optional: true },
  { id: "channels", code: "CHAN", name: "Channels", required: false, optional: true },
  { id: "chanconfig", code: "CCFG", name: "Channel Config", required: false, optional: true, conditional: true },
  { id: "gateway", code: "GTWY", name: "Gateway", required: true, optional: false },
  { id: "agent", code: "AGNT", name: "Agent", required: true, optional: false },
  { id: "workspace", code: "WKSP", name: "Workspace", required: true, optional: false },
  { id: "security", code: "SECR", name: "Security", required: true, optional: false },
  { id: "features", code: "FEAT", name: "Features", required: false, optional: true },
  { id: "voice", code: "VOIC", name: "Voice", required: false, optional: true },
  { id: "images", code: "IMGS", name: "Images", required: false, optional: true },
  { id: "orchestration", code: "ORCH", name: "Orchestration", required: false, optional: true },
  { id: "memory", code: "MNEM", name: "Memory", required: false, optional: true },
  { id: "skills", code: "SKIL", name: "Skills", required: false, optional: true },
  { id: "complete", code: "DONE", name: "Complete", required: true, optional: false },
];

/* ═══ DATA ═══ */
const LLM_PROVIDERS = [
  { id: "anthropic", name: "Anthropic", glyph: "ANT", color: C.accent, sub: "Deep reasoning, code, long context", flagship: "claude-opus-4-7", price: "$15/$75 per Mtok", available: true, keyFmt: "sk-ant-..." },
  { id: "openai", name: "OpenAI", glyph: "OAI", color: C.green, sub: "Broad multimodal capability", flagship: "gpt-4o", price: "$2.50/$10 per Mtok", available: true, keyFmt: "sk-..." },
  { id: "minimax", name: "MiniMax", glyph: "MNX", color: C.amber, sub: "High-throughput, multilingual, agentic", flagship: "abab-7-chat", price: "$0.20/$0.80 per Mtok", available: true, keyFmt: "mnx-...", featured: true },
  { id: "google", name: "Google", glyph: "GCP", color: C.blue, sub: "Speed, multimodal workflows", flagship: "gemini-2.5-pro", price: "$1.25/$5 per Mtok", available: true, keyFmt: "AIza..." },
  { id: "ollama", name: "Ollama", glyph: "OLM", color: C.cyan, sub: "Zero-cost, air-gapped, private", flagship: "llama-3.3-70b", price: "Free (local)", available: true, keyFmt: "(none)", detected: true },
  { id: "openrouter", name: "OpenRouter", glyph: "OR", color: C.purple, sub: "100+ models, cost optimization", flagship: "auto", price: "Varies", available: true, keyFmt: "sk-or-..." },
  { id: "groq", name: "Groq", glyph: "GRQ", color: C.yellow, sub: "Ultra-low latency inference", flagship: "llama-3.3-70b-versatile", price: "$0.59/$0.79 per Mtok", available: true, keyFmt: "gsk_..." },
  { id: "mistral", name: "Mistral", glyph: "MST", color: C.accent, sub: "European AI, multilingual", flagship: "mistral-large-2", price: "$2/$6 per Mtok", available: true, keyFmt: "..." },
  { id: "together", name: "Together", glyph: "TGT", color: C.green, sub: "Open-source at scale", flagship: "Llama-3.3-70B", price: "$0.88 per Mtok", available: true, keyFmt: "..." },
  { id: "fireworks", name: "Fireworks", glyph: "FRW", color: C.yellow, sub: "Fast open-source inference", flagship: "llama-v3p3", price: "$0.90 per Mtok", available: true, keyFmt: "..." },
  { id: "azure", name: "Azure / Bedrock", glyph: "ENT", color: C.purple, sub: "Enterprise compliance, SLAs", flagship: "varies", price: "Enterprise", available: true, keyFmt: "..." },
  { id: "custom", name: "Custom OpenAI-compat", glyph: "CST", color: C.cyan, sub: "vLLM, LM Studio, internal proxies", flagship: "user-supplied", price: "Self-hosted", available: true, keyFmt: "(optional)" },
];

const CHANNELS = [
  { id: "telegram", name: "Telegram", glyph: "TG", color: C.blue, group: "Cloud APIs", desc: "Full chat, groups, bots, media", sdk: "grammers MTProto" },
  { id: "discord", name: "Discord", glyph: "DC", color: C.purple, group: "Cloud APIs", desc: "Channels, threads, reactions, embeds", sdk: "Serenity gateway" },
  { id: "slack", name: "Slack", glyph: "SL", color: C.green, group: "Cloud APIs", desc: "Channels, threads, DMs, files", sdk: "Socket Mode + Web API" },
  { id: "email", name: "Email", glyph: "EM", color: C.amber, group: "Cloud APIs", desc: "Send, read, search, flag, forward", sdk: "lettre SMTP + IMAP" },
  { id: "imessage", name: "iMessage", glyph: "iM", color: C.cyan, group: "Phone-paired", desc: "Send, read, conversations (macOS)", sdk: "AppleScript bridge" },
  { id: "whatsapp", name: "WhatsApp", glyph: "WA", color: C.green, group: "Phone-paired", desc: "Requires QR scan from your phone", sdk: "Cloud API" },
  { id: "signal", name: "Signal", glyph: "SG", color: C.blue, group: "Phone-paired", desc: "Requires QR scan from your phone", sdk: "signal-cli JSON-RPC" },
  { id: "matrix", name: "Matrix", glyph: "MX", color: C.accent, group: "Phone-paired", desc: "Rooms, E2E encryption, federation", sdk: "matrix-sdk v0.16" },
];

const PERSONAS = [
  { id: "coordinator", name: "Coordinator", glyph: "COO", color: C.accent, sub: "Orchestrates the fleet", tone: "professional, direct, decisive" },
  { id: "engineer", name: "Engineer", glyph: "ENG", color: C.cyan, sub: "Writes and reviews code", tone: "precise, technical, terse" },
  { id: "creative", name: "Creative", glyph: "CRT", color: C.purple, sub: "Marketing and content", tone: "warm, expressive, narrative" },
  { id: "sysadmin", name: "Sysadmin", glyph: "OPS", color: C.green, sub: "Monitors and maintains", tone: "calm, observational, methodical" },
  { id: "analyst", name: "Analyst", glyph: "ANL", color: C.amber, sub: "Research and synthesis", tone: "curious, rigorous, thorough" },
  { id: "custom", name: "Custom", glyph: "CST", color: C.dim, sub: "Define your own", tone: "" },
];

const SECURITY_LEVELS = [
  { id: "strict", name: "Strict", glyph: "STR", color: C.red, sub: "Shared-machine fleet bots", blocked: ["shell", "web_fetch", "apply_patch", "fs_write outside workspace"], allowed: ["fs_read in workspace", "memory ops", "channel send"] },
  { id: "standard", name: "Standard", glyph: "STD", color: C.amber, sub: "Personal coding assistant", blocked: ["shell with sudo", "fs_write outside workspace + home"], allowed: ["all read", "shell (filtered)", "web_fetch (allowlisted)", "apply_patch"], recommended: true },
  { id: "permissive", name: "Permissive", glyph: "PRM", color: C.yellow, sub: "Sandbox / research", blocked: [], allowed: ["everything", "approval pipeline still active"] },
  { id: "custom", name: "Custom", glyph: "CST", color: C.dim, sub: "Per-tool allowlist", blocked: ["..."], allowed: ["..."] },
];

const FEATURES = [
  { id: "talos", name: "Talos (macOS automation)", color: C.accent, desc: "193 tools across Calendar, Notes, Mail, Safari, etc.", platforms: ["macOS"], required_on: "macOS", warning: "macOS gate — without this, image-gen, AppleScript, system-info ALL silently fail" },
  { id: "nous", name: "Nous (cognitive learning)", color: C.cyan, desc: "Captures intent + improves over time. Optional but recommended." },
  { id: "mnemosyne", name: "Mnemosyne (memory)", color: C.amber, desc: "Three-layer persistent memory system." },
  { id: "hermes", name: "Hermes (channels)", color: C.green, desc: "Cross-channel messaging coordination." },
  { id: "athena", name: "Athena (research)", color: C.purple, desc: "Vault-based knowledge synthesis (Obsidian)." },
  { id: "browser", name: "Browser (Chrome CDP)", color: C.blue, desc: "11 browser automation tools." },
  { id: "voice", name: "Voice (TTS/STT)", color: C.cyan, desc: "Twilio calls + Whisper STT + TTS." },
  { id: "skills", name: "Skill marketplace", color: C.yellow, desc: "Plugin system for adding tools." },
];

const VOICE_PROVIDERS = [
  { id: "elevenlabs", name: "ElevenLabs", glyph: "11L", color: C.accent, sub: "Premium quality voices" },
  { id: "openai-tts", name: "OpenAI TTS", glyph: "OAI", color: C.green, sub: "Native multimodal" },
  { id: "cartesia", name: "Cartesia", glyph: "CTS", color: C.cyan, sub: "Real-time streaming" },
  { id: "custom", name: "Custom Endpoint", glyph: "API", color: C.amber, sub: "Self-hosted Piper / Kokoro" },
  { id: "none", name: "Skip", glyph: "—", color: C.dim, sub: "No voice configured" },
];

const IMAGE_PROVIDERS = [
  { id: "openai", name: "OpenAI GPT Image", glyph: "OAI", color: C.green, sub: "gpt-image-1" },
  { id: "google", name: "Google NanoBanana", glyph: "GCP", color: C.blue, sub: "gemini-2.5-flash-image" },
  { id: "bfl", name: "BFL Flux", glyph: "BFL", color: C.amber, sub: "flux-pro / dev / schnell" },
  { id: "openai-custom", name: "OpenAI compat URL", glyph: "API", color: C.cyan, sub: "vLLM, fal.ai, proxies" },
  { id: "a1111", name: "Automatic1111 URL", glyph: "A11", color: C.accentBright, sub: "Z-Image Turbo path" },
  { id: "none", name: "Skip", glyph: "—", color: C.dim, sub: "No image gen" },
];

const SKILLS = {
  Productivity: [
    { id: "calendar-pro", name: "Calendar Pro", desc: "Auto-schedule + conflict detection", recommended: true },
    { id: "email-triage", name: "Email Triage", desc: "Inbox prioritization", recommended: true },
    { id: "todo-sync", name: "Todo Sync", desc: "Cross-platform task sync" },
  ],
  Dev: [
    { id: "git-flow", name: "Git Flow", desc: "Branch + PR automation", recommended: true },
    { id: "ci-watch", name: "CI Watch", desc: "Pipeline monitoring + fixes", recommended: true },
    { id: "openclaw-compat", name: "OpenClaw Compat", desc: "Adds claw_* tools" },
    { id: "test-gen", name: "Test Gen", desc: "Auto-generate test cases" },
  ],
  Marketing: [
    { id: "content-cycle", name: "Content Cycle", desc: "Multi-channel campaigns", recommended: true },
    { id: "brand-voice", name: "Brand Voice", desc: "Style consistency check" },
  ],
  Security: [
    { id: "secret-scan", name: "Secret Scan", desc: "Pre-commit credential check", recommended: true },
    { id: "audit-trail", name: "Audit Trail", desc: "Compliance log generation" },
  ],
  Research: [
    { id: "deep-synth", name: "Deep Synthesis", desc: "Multi-source research", recommended: true },
    { id: "paper-tracker", name: "Paper Tracker", desc: "ArXiv monitoring" },
  ],
};

const ORCH_MODES = [
  { id: "all-on", name: "All-on", glyph: "ALL", color: C.accent, sub: "Heartbeat + cron + watchdog", desc: "Full autonomous operation. Recommended for fleet agents.", recommended: true },
  { id: "heartbeat-only", name: "Heartbeat-only", glyph: "HB", color: C.amber, sub: "Wake events only, no scheduled tasks", desc: "Reactive only. Agent wakes on inputs, doesn't run scheduled work." },
  { id: "disabled", name: "Disabled", glyph: "OFF", color: C.dim, sub: "Manual invocation only", desc: "No background activity. Agent runs only when explicitly invoked." },
];

const MEMORY_PROVIDERS = [
  { id: "ollama", name: "Ollama", glyph: "OLM", color: C.cyan, sub: "Local, free, private", model: "nomic-embed-text", recommended: true, detected: true },
  { id: "openai", name: "OpenAI", glyph: "OAI", color: C.green, sub: "Cloud, paid, fast", model: "text-embedding-3-small" },
  { id: "none", name: "FTS-only", glyph: "FTS", color: C.amber, sub: "No embeddings, full-text search only", model: "—" },
];

/* ═══ TOP BAR ═══ */
const TopBar = ({ stepIdx, hostname, faceState }) => (
  <div style={{ height: 24, background: C.bg2, borderBottom: `1px solid ${C.muted}`, display: "flex", alignItems: "center", padding: "0 10px", gap: 6, flexShrink: 0 }}>
    <span style={{ color: C.accent, fontWeight: 700, fontSize: 9, letterSpacing: 3 }}>ZEUS</span>
    <span style={{ color: C.muted }}>│</span>
    <span style={{ color: C.accentDim, fontSize: 9, fontWeight: 700, letterSpacing: 2 }}>ONBOARDING</span>
    <span style={{ color: C.muted }}>│</span>
    <span style={{ color: C.dim, fontSize: 9 }}>Step {stepIdx + 1} of {STEPS.length}</span>
    <span style={{ color: C.muted }}>│</span>
    <span style={{ color: C.dim, fontSize: 9 }}>{STEPS[stepIdx].code}</span>
    <span style={{ flex: 1 }} />
    <ZeusFace state={faceState || "ready"} small speed={faceState === "working" ? 200 : faceState === "ready" ? 600 : 320} />
    <span style={{ color: C.muted }}>│</span>
    <span style={{ color: C.green, fontSize: 8 }}>●</span>
    <span style={{ color: C.dim, fontSize: 9 }}>config draft</span>
    <span style={{ color: C.muted }}>│</span>
    <span style={{ color: C.dim, fontSize: 9 }}>{hostname || "~/.zeus/config.toml"}</span>
  </div>
);

/* ═══ STEP INDICATOR ═══ */
const StepIndicator = ({ current, completed, skipped }) => {
  const visible = STEPS.map((s, i) => ({ ...s, idx: i }))
    .filter((_, i) => Math.abs(i - current) <= 4 || i === 0 || i === STEPS.length - 1);

  return (
    <div style={{ padding: "6px 12px", borderBottom: `1px solid ${C.muted}`, background: C.bg2, display: "flex", alignItems: "center", gap: 0, fontSize: 8, overflow: "hidden", flexShrink: 0 }}>
      {visible.map((s, vi) => {
        const isCurrent = s.idx === current;
        const isCompleted = completed.has(s.idx);
        const isSkipped = skipped.has(s.idx);
        const prevIdx = vi > 0 ? visible[vi - 1].idx : -1;
        const showEllipsis = prevIdx >= 0 && s.idx - prevIdx > 1;

        return (
          <div key={s.idx} style={{ display: "flex", alignItems: "center" }}>
            {showEllipsis && <span style={{ color: C.muted, padding: "0 6px", fontSize: 9 }}>···</span>}
            <div style={{ display: "flex", alignItems: "center", padding: "0 4px", gap: 4 }}>
              <span style={{
                width: 16, height: 14, display: "inline-flex", alignItems: "center", justifyContent: "center",
                fontSize: 8, fontWeight: 700,
                background: isCurrent ? C.accent : isCompleted ? C.accentFaint : isSkipped ? C.bg : C.bg,
                color: isCurrent ? C.bg : isCompleted ? C.accent : isSkipped ? C.muted : C.muted,
                border: `1px solid ${isCurrent ? C.accent : isCompleted ? C.accentDim : isSkipped ? C.muted : C.muted}`,
              }}>
                {isCompleted ? "✓" : isSkipped ? "⏭" : String(s.idx + 1).padStart(2, "0")}
              </span>
              <span style={{
                color: isCurrent ? C.fg : isCompleted ? C.dim : isSkipped ? C.muted : C.muted,
                fontWeight: isCurrent ? 700 : 400,
                letterSpacing: 1,
                whiteSpace: "nowrap",
              }}>
                {s.name}
              </span>
            </div>
            {vi < visible.length - 1 && <span style={{ color: C.muted, fontSize: 9 }}>›</span>}
          </div>
        );
      })}
    </div>
  );
};

/* ═══ STEP HEADER (left rail, used in card layouts) ═══ */
const StepHeader = ({ idx, title, subtitle }) => (
  <div style={{ padding: "12px 14px 10px", borderBottom: `1px solid ${C.muted}` }}>
    <div style={{ display: "flex", alignItems: "center", gap: 8, marginBottom: 4 }}>
      <span style={{ color: C.accentDim, fontSize: 9, fontWeight: 700, letterSpacing: 3 }}>STEP {String(idx + 1).padStart(2, "0")}/{STEPS.length}</span>
      <span style={{ color: C.muted }}>│</span>
      <span style={{ color: C.dim, fontSize: 9, fontWeight: 700, letterSpacing: 2 }}>{STEPS[idx].code}</span>
    </div>
    <div style={{ color: C.fg, fontSize: 14, fontWeight: 700, letterSpacing: 0.5 }}>{title}</div>
    {subtitle && <div style={{ color: C.dim, fontSize: 10, marginTop: 2 }}>{subtitle}</div>}
  </div>
);

/* ═══ FIELD INPUT ═══ */
const Field = ({ label, value, onChange, focused, onFocus, placeholder, secret, required, hint, error, options, valid }) => {
  const [revealed, setRevealed] = useState(false);
  const isSecret = secret && !revealed;

  return (
    <div style={{ marginBottom: hint || error ? 0 : 4 }}>
      <div style={{ display: "flex", alignItems: "stretch" }}>
        <div style={{
          width: 140, padding: "6px 10px", fontSize: 10,
          color: focused ? C.accent : C.dim, fontWeight: 700, letterSpacing: 1,
          display: "flex", alignItems: "center", flexShrink: 0,
        }}>
          {required && <span style={{ color: C.accent, marginRight: 4 }}>*</span>}
          {label.toUpperCase()}
        </div>
        <div style={{ width: 1, background: C.muted, flexShrink: 0 }} />
        <div style={{ flex: 1, position: "relative", display: "flex", alignItems: "center" }}>
          <input
            type={isSecret ? "password" : "text"}
            value={value || ""}
            placeholder={placeholder}
            onChange={(e) => onChange(e.target.value)}
            onFocus={onFocus}
            style={{
              flex: 1,
              background: focused ? C.bg2 : "transparent",
              border: `1px solid ${focused ? C.accent : error ? C.red : "transparent"}`,
              color: C.fg, fontFamily: "inherit", fontSize: 11,
              padding: "6px 10px", outline: "none",
            }}
          />
          {valid === true && value && (
            <span style={{ color: C.green, fontSize: 11, marginRight: 8 }}>✓</span>
          )}
          {valid === false && value && (
            <span style={{ color: C.red, fontSize: 11, marginRight: 8 }}>✕</span>
          )}
          {secret && value && (
            <button
              onClick={(e) => { e.stopPropagation(); setRevealed(!revealed); }}
              style={{
                background: "transparent", border: `1px solid ${C.muted}`, color: C.dim,
                fontSize: 8, padding: "2px 6px", cursor: "pointer", fontFamily: "inherit",
                letterSpacing: 1, marginRight: 6,
              }}
            >{revealed ? "HIDE" : "SHOW"}</button>
          )}
          {options && <span style={{ color: C.muted, fontSize: 9, marginRight: 8 }}>↓</span>}
        </div>
      </div>
      {hint && (
        <div style={{ paddingLeft: 152, marginTop: 2, marginBottom: 6, fontSize: 9, color: C.muted, fontStyle: "italic" }}>
          ℹ {hint}
        </div>
      )}
      {error && (
        <div style={{ paddingLeft: 152, marginTop: 2, marginBottom: 6, fontSize: 9, color: C.red }}>
          ✕ {error}
        </div>
      )}
    </div>
  );
};

/* ═══ CARD COMPONENT (used across many steps) ═══ */
const Card = ({ id, glyph, name, sub, color, selected, focused, multiselect, toggled, badge, onClick, dim, recommended, featured, detected, large, extra }) => (
  <div
    onClick={onClick}
    style={{
      background: selected ? C.accentFaint : focused ? C.bg2 : C.bg,
      border: `1px solid ${selected ? C.accent : featured ? C.amber : focused ? C.accentDim : C.muted}`,
      borderLeft: `2px solid ${selected ? C.accent : color}`,
      padding: large ? "12px 14px" : "10px 12px",
      cursor: "pointer",
      display: "flex", flexDirection: "column", gap: 4,
      position: "relative",
      opacity: dim ? 0.5 : 1,
    }}
  >
    {selected && (
      <span style={{ position: "absolute", left: -14, top: 11, color: C.accent, fontWeight: 700, fontSize: 11 }}>▸</span>
    )}
    <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
      <span style={{
        width: 32, height: 18, display: "inline-flex", alignItems: "center", justifyContent: "center",
        background: selected ? color : C.bg2, color: selected ? C.bg : color,
        fontWeight: 700, fontSize: 9, letterSpacing: 1,
        border: `1px solid ${color}`, flexShrink: 0,
      }}>{glyph}</span>
      <span style={{ color: selected ? C.white : C.fg, fontWeight: 600, fontSize: 12, flex: 1 }}>{name}</span>
      {multiselect && (
        <span style={{
          width: 14, height: 14, display: "inline-flex", alignItems: "center", justifyContent: "center",
          fontSize: 9, fontWeight: 700,
          background: toggled ? C.accent : "transparent",
          color: toggled ? C.bg : C.muted,
          border: `1px solid ${toggled ? C.accent : C.muted}`,
        }}>{toggled ? "✓" : ""}</span>
      )}
      {selected && !multiselect && (
        <span style={{ color: C.accent, fontSize: 8, fontWeight: 700, letterSpacing: 2 }}>SELECTED</span>
      )}
      {featured && <span style={{ color: C.amber, fontSize: 7, fontWeight: 700, letterSpacing: 2, padding: "1px 4px", background: C.bg2, border: `1px solid ${C.amber}` }}>FEATURED</span>}
      {recommended && !selected && <span style={{ color: C.green, fontSize: 7, fontWeight: 700, letterSpacing: 2 }}>★ REC</span>}
      {detected && <span style={{ color: C.green, fontSize: 7, fontWeight: 700, letterSpacing: 1 }}>● DETECTED</span>}
      {badge && <span style={{ color: badge.color || C.dim, fontSize: 8, fontWeight: 700, letterSpacing: 1 }}>{badge.text}</span>}
    </div>
    {sub && <div style={{ color: C.dim, fontSize: 10, paddingLeft: 40, marginTop: -2 }}>{sub}</div>}
    {extra && <div style={{ paddingLeft: 40, marginTop: 2 }}>{extra}</div>}
  </div>
);

/* ═══ STATUS BAR ═══ */
const StatusBar = ({ canBack, canSkip, canContinue, currentStep, totalSteps, onBack, onSkip, onContinue, validationState, extraKeys }) => (
  <div style={{
    height: 22, background: C.bg2, borderTop: `1px solid ${C.muted}`,
    display: "flex", alignItems: "center", padding: "0 10px", gap: 10,
    flexShrink: 0, fontSize: 9,
  }}>
    <span style={{ color: C.green }}>●</span>
    <span style={{ color: C.dim }}>onboard</span>
    <span style={{ color: C.muted }}>│</span>
    <span><span style={{ color: C.accentDim, fontWeight: 700 }}>↑↓</span> <span style={{ color: C.dim }}>Navigate</span></span>
    <span><span style={{ color: C.accentDim, fontWeight: 700 }}>↵</span> <span style={{ color: C.dim }}>Select</span></span>
    <span><span style={{ color: C.accentDim, fontWeight: 700 }}>Tab</span> <span style={{ color: C.dim }}>Field</span></span>
    {extraKeys && extraKeys.map((k, i) => (
      <span key={i}><span style={{ color: C.accentDim, fontWeight: 700 }}>{k.k}</span> <span style={{ color: C.dim }}>{k.v}</span></span>
    ))}
    <span><span style={{ color: C.accentDim, fontWeight: 700 }}>?</span> <span style={{ color: C.dim }}>Help</span></span>
    {canBack && <span onClick={onBack} style={{ cursor: "pointer" }}><span style={{ color: C.accentDim, fontWeight: 700 }}>Esc</span> <span style={{ color: C.dim }}>Back</span></span>}

    <span style={{ flex: 1 }} />

    <span style={{ color: validationState === "valid" ? C.green : validationState === "incomplete" ? C.yellow : C.dim, fontSize: 8 }}>●</span>
    <span style={{ color: validationState === "valid" ? C.green : validationState === "incomplete" ? C.yellow : C.dim }}>
      {validationState === "valid" ? "VALID" : validationState === "incomplete" ? "INCOMPLETE" : "READY"}
    </span>
    <span style={{ color: C.muted }}>│</span>

    {canSkip && (
      <span onClick={onSkip} style={{ color: C.dim, cursor: "pointer", fontWeight: 700, letterSpacing: 1 }}>
        SKIP →
      </span>
    )}
    {canSkip && <span style={{ color: C.muted }}>│</span>}

    <span
      onClick={canContinue ? onContinue : undefined}
      style={{
        color: canContinue ? C.accent : C.muted,
        fontWeight: 700, letterSpacing: 2,
        cursor: canContinue ? "pointer" : "not-allowed",
      }}
    >
      CONTINUE → {String(currentStep + 2).padStart(2, "0")}/{String(totalSteps).padStart(2, "0")}
    </span>
  </div>
);

/* ═══════════════════════════════════════════════════ */
/* INDIVIDUAL STEP RENDERERS                            */
/* ═══════════════════════════════════════════════════ */

const WelcomeStep = ({ existing, onContinue }) => (
  <div style={{ flex: 1, display: "flex", flexDirection: "column", alignItems: "center", justifyContent: "center", padding: "20px", overflow: "auto" }}>
    <div style={{ marginBottom: 20 }}>
      {LOGO.map((line, i) => (
        <div key={i} style={{ fontSize: 13, lineHeight: "18px", whiteSpace: "pre", color: LOGO_COLORS[i], fontWeight: i < 2 ? 700 : 400 }}>{line}</div>
      ))}
    </div>
    <div style={{ fontSize: 11, letterSpacing: 6, color: C.accentDim, fontWeight: 700, marginBottom: 6 }}>O P E R A T I N G   S Y S T E M</div>
    <div style={{ fontSize: 10, color: C.dim, marginBottom: 14 }}>Autonomous AI agents on your hardware</div>

    {/* ZeusFace greeting */}
    <div style={{ display: "flex", alignItems: "center", gap: 10, marginBottom: 22, padding: "6px 16px", border: `1px solid ${C.muted}`, background: C.bg2 }}>
      <ZeusFace state="ready" speed={500} />
      <span style={{ color: C.dim, fontSize: 11, fontStyle: "italic" }}>"Let's wake the fleet. This won't take long."</span>
    </div>

    {existing && (
      <div style={{
        padding: "10px 16px", border: `1px solid ${C.amber}`, borderLeft: `2px solid ${C.amber}`,
        background: C.bg2, marginBottom: 24, maxWidth: 500,
      }}>
        <div style={{ color: C.amber, fontSize: 9, fontWeight: 700, letterSpacing: 3, marginBottom: 4 }}>↻ EXISTING CONFIG DETECTED</div>
        <div style={{ color: C.fg, fontSize: 11 }}>Welcome back, <span style={{ color: C.accent, fontWeight: 700 }}>Zeus100</span>. Re-running will pre-populate fields from your current config.</div>
      </div>
    )}

    <div style={{ width: 480, border: `1px solid ${C.muted}` }}>
      <div style={{ padding: "8px 14px", background: C.bg2, borderBottom: `1px solid ${C.muted}`, display: "flex", alignItems: "center", gap: 8 }}>
        <span style={{ color: C.accent, fontWeight: 700, fontSize: 9, letterSpacing: 3 }}>▸ INITIATE</span>
        <span style={{ flex: 1 }} />
        <span style={{ fontSize: 9, color: C.muted }}>v0.4.7 · 391,269 LOC · 365 tools</span>
      </div>
      <div style={{ padding: "20px 24px" }}>
        <div style={{ color: C.fg, fontSize: 12, lineHeight: 1.6, marginBottom: 14 }}>
          This wizard configures every system on your Zeus deployment. You can skip optional sections — your fleet will land at a working baseline regardless.
        </div>
        <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
          {[
            ["19 STEPS", "10 required, 9 optional"],
            ["~5 MIN", "QuickStart path"],
            ["~25 MIN", "Full configuration"],
          ].map(([l, r]) => (
            <div key={l} style={{ display: "flex", justifyContent: "space-between", fontSize: 10 }}>
              <span style={{ color: C.accentDim, fontWeight: 700, letterSpacing: 2 }}>{l}</span>
              <span style={{ color: C.dim }}>{r}</span>
            </div>
          ))}
        </div>
      </div>
      <div style={{ padding: "8px 14px", background: C.bg2, borderTop: `1px solid ${C.muted}`, display: "flex", gap: 12 }}>
        <span style={{ fontSize: 9, color: C.dim }}><span style={{ color: C.accentDim, fontWeight: 700 }}>↵</span> Continue</span>
        <span style={{ fontSize: 9, color: C.dim }}><span style={{ color: C.accentDim, fontWeight: 700 }}>N</span> Exit</span>
        <span style={{ flex: 1 }} />
        <span style={{ fontSize: 9, color: C.muted }}>build a1c4f29 · main</span>
      </div>
    </div>

    <div style={{ marginTop: 18, fontSize: 9, color: C.muted }}>
      Press <span style={{ color: C.accent, fontWeight: 700 }}>↵ Enter</span> to begin
    </div>
  </div>
);

const ModeStep = ({ selected, onSelect }) => {
  const modes = [
    { id: "quickstart", name: "QuickStart", glyph: "QS", color: C.green, sub: "1 LLM, 1 channel, sane defaults", time: "~3 min", steps: "1 step left" },
    { id: "full", name: "Full Setup", glyph: "FU", color: C.accent, sub: "Walk every section in detail", time: "~25 min", steps: "17 steps left" },
    { id: "custom", name: "Custom", glyph: "CU", color: C.cyan, sub: "Pick which steps you want", time: "varies", steps: "you choose" },
  ];

  return (
    <div style={{ flex: 1, display: "flex", flexDirection: "column", padding: "16px 18px", overflow: "auto" }}>
      <div style={{ marginBottom: 16 }}>
        <div style={{ color: C.fg, fontSize: 16, fontWeight: 700, marginBottom: 4 }}>Choose your setup mode</div>
        <div style={{ color: C.dim, fontSize: 11 }}>Select the path that matches how much you want to configure now. You can re-run <span style={{ color: C.accentBright }}>zeus onboard</span> anytime to fill in skipped sections.</div>
      </div>

      <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr 1fr", gap: 12, marginBottom: 16 }}>
        {modes.map((m) => (
          <div
            key={m.id}
            onClick={() => onSelect(m.id)}
            style={{
              background: selected === m.id ? C.accentFaint : C.bg2,
              border: `1px solid ${selected === m.id ? C.accent : C.muted}`,
              borderLeft: `2px solid ${m.color}`,
              padding: "16px 18px",
              cursor: "pointer",
              display: "flex", flexDirection: "column", gap: 10,
              position: "relative",
              minHeight: 200,
            }}
          >
            {selected === m.id && (
              <span style={{ position: "absolute", top: 10, right: 12, color: C.accent, fontSize: 10, fontWeight: 700, letterSpacing: 2 }}>▸ SELECTED</span>
            )}

            <div style={{
              width: 48, height: 48, display: "flex", alignItems: "center", justifyContent: "center",
              background: selected === m.id ? m.color : C.bg, color: selected === m.id ? C.bg : m.color,
              fontWeight: 700, fontSize: 14, letterSpacing: 2,
              border: `1px solid ${m.color}`,
            }}>{m.glyph}</div>

            <div>
              <div style={{ color: selected === m.id ? C.white : C.fg, fontSize: 16, fontWeight: 700, marginBottom: 4 }}>{m.name}</div>
              <div style={{ color: C.dim, fontSize: 11 }}>{m.sub}</div>
            </div>

            <div style={{ marginTop: "auto", display: "flex", flexDirection: "column", gap: 4 }}>
              <div style={{ display: "flex", justifyContent: "space-between", fontSize: 10 }}>
                <span style={{ color: C.muted, fontWeight: 700, letterSpacing: 1 }}>TIME</span>
                <span style={{ color: m.color }}>{m.time}</span>
              </div>
              <div style={{ display: "flex", justifyContent: "space-between", fontSize: 10 }}>
                <span style={{ color: C.muted, fontWeight: 700, letterSpacing: 1 }}>STEPS</span>
                <span style={{ color: C.dim }}>{m.steps}</span>
              </div>
            </div>
          </div>
        ))}
      </div>

      <div style={{ padding: "10px 14px", border: `1px solid ${C.muted}`, background: C.bg2, fontSize: 10 }}>
        <span style={{ color: C.accentDim, fontWeight: 700, letterSpacing: 3 }}>NOTE</span>
        <span style={{ color: C.dim, marginLeft: 8 }}>Skipped sections can be configured later via <span style={{ color: C.accentBright }}>zeus onboard --resume</span> or by editing <span style={{ color: C.accentBright }}>~/.zeus/config.toml</span> directly.</span>
      </div>
    </div>
  );
};

const ProviderStep = ({ selected, focused, onSelect, onFocus }) => (
  <div style={{ flex: 1, display: "flex", overflow: "hidden" }}>
    <div style={{ width: 360, borderRight: `1px solid ${C.muted}`, display: "flex", flexDirection: "column" }}>
      <StepHeader idx={2} title="Pick your LLM provider" subtitle="Primary model that powers agent reasoning" />
      <div style={{ flex: 1, overflowY: "auto", padding: "8px 14px 8px 22px", display: "flex", flexDirection: "column", gap: 6 }}>
        {LLM_PROVIDERS.map(p => (
          <Card key={p.id} {...p} selected={selected === p.id} focused={focused === p.id}
            onClick={() => onSelect(p.id)} />
        ))}
      </div>
      <div style={{ padding: "6px 14px", borderTop: `1px solid ${C.muted}`, background: C.bg2, fontSize: 9, color: C.dim, display: "flex", justifyContent: "space-between" }}>
        <span><span style={{ color: C.accent, fontWeight: 700 }}>{LLM_PROVIDERS.length}</span> providers</span>
        <span>Sorted by usage frequency</span>
      </div>
    </div>

    <div style={{ flex: 1, padding: "16px 18px", overflowY: "auto" }}>
      {selected && (() => {
        const p = LLM_PROVIDERS.find(x => x.id === selected);
        return (
          <>
            <div style={{ display: "flex", alignItems: "flex-start", gap: 14, marginBottom: 18 }}>
              <div style={{ width: 56, height: 56, background: p.color, color: C.bg, display: "flex", alignItems: "center", justifyContent: "center", fontWeight: 700, fontSize: 16, letterSpacing: 2, border: `1px solid ${p.color}`, flexShrink: 0 }}>{p.glyph}</div>
              <div style={{ flex: 1 }}>
                <div style={{ display: "flex", alignItems: "center", gap: 8, marginBottom: 4 }}>
                  <span style={{ color: C.white, fontSize: 18, fontWeight: 700 }}>{p.name}</span>
                  {p.featured && <span style={{ padding: "2px 6px", fontSize: 8, fontWeight: 700, letterSpacing: 2, background: C.bg2, color: C.amber, border: `1px solid ${C.amber}` }}>FEATURED</span>}
                  {p.detected && <span style={{ padding: "2px 6px", fontSize: 8, fontWeight: 700, letterSpacing: 2, background: C.greenDim, color: C.green, border: `1px solid ${C.green}` }}>● DETECTED</span>}
                </div>
                <div style={{ color: C.dim, fontSize: 11, marginBottom: 8 }}>{p.sub}</div>
                <div style={{ display: "flex", gap: 16, fontSize: 9 }}>
                  <span><span style={{ color: C.accentDim, fontWeight: 700, letterSpacing: 2 }}>FLAGSHIP</span> <span style={{ color: C.fg }}>{p.flagship}</span></span>
                  <span><span style={{ color: C.accentDim, fontWeight: 700, letterSpacing: 2 }}>PRICING</span> <span style={{ color: C.fg }}>{p.price}</span></span>
                  <span><span style={{ color: C.accentDim, fontWeight: 700, letterSpacing: 2 }}>KEY FORMAT</span> <span style={{ color: C.accentBright }}>{p.keyFmt}</span></span>
                </div>
              </div>
            </div>

            <div style={{ padding: "10px 14px", background: C.bg2, border: `1px solid ${C.muted}`, fontSize: 11 }}>
              <div style={{ color: C.dim, fontSize: 9, fontWeight: 700, letterSpacing: 3, marginBottom: 6 }}>WILL WRITE TO ~/.zeus/config.toml</div>
              <div style={{ color: C.dim }}>model</div>
              <div><span style={{ color: C.fg }}>model</span><span style={{ color: C.muted }}> = </span><span style={{ color: C.accentBright }}>"{p.id}/{p.flagship}"</span></div>
            </div>

            <div style={{ marginTop: 16, fontSize: 10, color: C.dim }}>
              <span style={{ color: C.accentDim, fontWeight: 700, letterSpacing: 2 }}>NEXT</span>
              <span style={{ marginLeft: 8 }}>Step 04 (AUTH) will collect the API key for <span style={{ color: C.accentBright }}>{p.name}</span>.</span>
            </div>
          </>
        );
      })()}
    </div>

    <div style={{ width: 200, borderLeft: `1px solid ${C.muted}`, background: C.bg2, padding: "10px 12px" }}>
      <div style={{ color: C.accentDim, fontSize: 9, fontWeight: 700, letterSpacing: 3, marginBottom: 8 }}>HINTS</div>
      <div style={{ fontSize: 10, color: C.fg, lineHeight: 1.5, marginBottom: 10 }}>
        Pick the provider you'll use most. You can configure backup providers in step 06.
      </div>
      <div style={{ marginTop: 14 }}>
        <div style={{ color: C.accentDim, fontSize: 9, fontWeight: 700, letterSpacing: 3, marginBottom: 6 }}>RECOMMENDATIONS</div>
        <div style={{ fontSize: 9, color: C.dim, lineHeight: 1.7 }}>
          <div>● Reasoning → <span style={{ color: C.accent }}>Anthropic</span></div>
          <div>● Multimodal → <span style={{ color: C.green }}>OpenAI</span></div>
          <div>● Throughput → <span style={{ color: C.amber }}>MiniMax</span></div>
          <div>● Local → <span style={{ color: C.cyan }}>Ollama</span></div>
          <div>● Speed → <span style={{ color: C.yellow }}>Groq</span></div>
        </div>
      </div>
    </div>
  </div>
);

const AuthStep = ({ provider, mode, setMode, values, setValue, focusedField, setFocusedField, testStatus, onTest }) => {
  const p = LLM_PROVIDERS.find(x => x.id === provider) || LLM_PROVIDERS[0];
  const modes = [
    { id: "key", label: "API Key", desc: "Paste a provider-issued API key" },
    { id: "token", label: "Setup Token", desc: "Paste an existing setup token" },
    { id: "browser", label: "Browser OAuth", desc: "Authenticate via browser callback" },
  ];

  const keyValid = values.api_key && values.api_key.startsWith(p.keyFmt.replace("...", ""));

  return (
    <div style={{ flex: 1, display: "flex", flexDirection: "column", padding: "16px 18px", overflow: "auto" }}>
      <div style={{ marginBottom: 14 }}>
        <div style={{ color: C.fg, fontSize: 16, fontWeight: 700, marginBottom: 4 }}>
          Authenticate with <span style={{ color: p.color }}>{p.name}</span>
        </div>
        <div style={{ color: C.dim, fontSize: 11 }}>Credentials persist to <span style={{ color: C.accentBright }}>~/.zeus/config.toml [credentials]</span> with 0600 permissions.</div>
      </div>

      {/* Mode tabs */}
      <div style={{ display: "flex", gap: 0, marginBottom: 16, borderBottom: `1px solid ${C.muted}` }}>
        {modes.map(m => (
          <div key={m.id} onClick={() => setMode(m.id)} style={{
            padding: "10px 18px", cursor: "pointer",
            borderBottom: `2px solid ${mode === m.id ? C.accent : "transparent"}`,
            color: mode === m.id ? C.fg : C.dim, fontWeight: mode === m.id ? 700 : 400,
            fontSize: 11,
          }}>
            <div>{m.label}</div>
            <div style={{ fontSize: 9, color: C.muted, marginTop: 2 }}>{m.desc}</div>
          </div>
        ))}
      </div>

      {mode === "key" && (
        <div style={{ marginBottom: 16 }}>
          <div style={{ color: C.accentDim, fontSize: 9, fontWeight: 700, letterSpacing: 3, marginBottom: 8 }}>API KEY</div>
          <Field
            label="API Key"
            value={values.api_key}
            onChange={(v) => setValue("api_key", v)}
            placeholder={p.keyFmt}
            secret required
            focused={focusedField === "api_key"}
            onFocus={() => setFocusedField("api_key")}
            valid={values.api_key ? keyValid : null}
            hint={keyValid === false && values.api_key ? `Expected format: ${p.keyFmt}` : null}
          />
          <div style={{ marginTop: 12, marginBottom: 12, paddingLeft: 152 }}>
            <button
              onClick={onTest}
              disabled={testStatus === "testing" || !values.api_key}
              style={{
                background: testStatus === "success" ? C.greenDim : testStatus === "error" ? "rgba(239,68,68,0.15)" : C.accentFaint,
                border: `1px solid ${testStatus === "success" ? C.green : testStatus === "error" ? C.red : C.accent}`,
                color: testStatus === "success" ? C.green : testStatus === "error" ? C.red : C.accent,
                padding: "5px 14px", fontFamily: "inherit", fontSize: 10,
                fontWeight: 700, letterSpacing: 2,
                cursor: values.api_key && testStatus !== "testing" ? "pointer" : "not-allowed",
                opacity: values.api_key ? 1 : 0.5,
              }}
            >
              {testStatus === "testing" ? "▸ TESTING..." :
               testStatus === "success" ? "✓ AUTH OK" :
               testStatus === "error" ? "✕ AUTH FAILED" :
               "▸ TEST CONNECTION"}
            </button>
            {testStatus === "testing" && (
              <span style={{ marginLeft: 12, display: "inline-flex", alignItems: "center" }}><ZeusFace state="thinking" small speed={220} label="probing endpoint" /></span>
            )}
            {testStatus === "success" && (
              <span style={{ marginLeft: 12, display: "inline-flex", alignItems: "center", gap: 10 }}>
                <ZeusFace state="success" small speed={400} />
                <span style={{ color: C.green, fontSize: 10 }}>● /v1/models returned 200 · 184ms · 47 models available</span>
              </span>
            )}
            {testStatus === "error" && (
              <span style={{ marginLeft: 12, display: "inline-flex", alignItems: "center", gap: 10 }}>
                <ZeusFace state="error" small speed={220} />
                <span style={{ color: C.red, fontSize: 10 }}>✕ 401 Unauthorized — check the API key</span>
              </span>
            )}
          </div>
        </div>
      )}

      {mode === "token" && (
        <div>
          <div style={{ color: C.accentDim, fontSize: 9, fontWeight: 700, letterSpacing: 3, marginBottom: 8 }}>SETUP TOKEN</div>
          <div style={{
            padding: "8px 12px", background: C.bg2, border: `1px solid ${C.muted}`,
            borderLeft: `2px solid ${C.amber}`, marginBottom: 10, fontSize: 10, color: C.fg,
          }}>
            <span style={{ color: C.amber, fontWeight: 700 }}>↻ Detected</span>
            <span style={{ color: C.dim }}> setup token at </span>
            <span style={{ color: C.accentBright }}>~/.zeus/setup-token</span>
            <span style={{ color: C.dim }}> · pre-populating</span>
          </div>
          <Field label="Token" value="zeus-setup-tk_a1b2c3d4..." onChange={() => {}} placeholder="paste token" secret required focused={focusedField === "token"} onFocus={() => setFocusedField("token")} />
        </div>
      )}

      {mode === "browser" && (
        <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
          <div style={{ color: C.accentDim, fontSize: 9, fontWeight: 700, letterSpacing: 3, marginBottom: 4 }}>OAUTH FLOW</div>
          {[
            { state: "done", text: "Opening browser to authentication URL..." },
            { state: "done", text: "Waiting for callback on http://127.0.0.1:8765/callback..." },
            { state: "active", text: "Received callback. Validating token..." },
            { state: "pending", text: "Storing token to credentials.json..." },
          ].map((s, i) => (
            <div key={i} style={{ display: "flex", alignItems: "center", gap: 10, fontSize: 11 }}>
              <span style={{
                width: 14, height: 14, display: "inline-flex", alignItems: "center", justifyContent: "center",
                background: s.state === "done" ? C.greenDim : s.state === "active" ? C.accentFaint : C.bg,
                border: `1px solid ${s.state === "done" ? C.green : s.state === "active" ? C.accent : C.muted}`,
                color: s.state === "done" ? C.green : s.state === "active" ? C.accent : C.muted,
                fontSize: 8, fontWeight: 700,
              }}>
                {s.state === "done" ? "✓" : s.state === "active" ? "▸" : "○"}
              </span>
              <span style={{ color: s.state === "done" ? C.dim : s.state === "active" ? C.fg : C.muted }}>{s.text}</span>
              {s.state === "active" && <span style={{ color: C.accent, fontSize: 9 }}>...</span>}
            </div>
          ))}
        </div>
      )}

      {/* Config preview */}
      <div style={{ marginTop: 18, padding: "10px 14px", background: C.bg2, border: `1px solid ${C.muted}`, fontSize: 11 }}>
        <div style={{ color: C.dim, fontSize: 9, fontWeight: 700, letterSpacing: 3, marginBottom: 6 }}>WILL WRITE TO ~/.zeus/config.toml</div>
        <div style={{ color: C.dim }}>[credentials]</div>
        <div>
          <span style={{ color: C.fg }}>{p.id}_api_key</span>
          <span style={{ color: C.muted }}> = </span>
          <span style={{ color: C.accentBright }}>"{values.api_key ? "***" + values.api_key.slice(-4) : "..."}"</span>
        </div>
      </div>
    </div>
  );
};

const ModelStep = ({ provider, selectedModel, onSelect }) => {
  const models = {
    anthropic: [
      { id: "claude-opus-4-7", name: "Claude Opus 4.7", ctx: "1M", price: "$15/$75 per Mtok", recommended: true, sub: "Most capable for reasoning + code" },
      { id: "claude-sonnet-4-6", name: "Claude Sonnet 4.6", ctx: "200K", price: "$3/$15 per Mtok", sub: "Balanced cost / quality" },
      { id: "claude-haiku-4-5", name: "Claude Haiku 4.5", ctx: "200K", price: "$0.80/$4 per Mtok", sub: "Fast + cheap" },
    ],
    openai: [
      { id: "gpt-4o", name: "GPT-4o", ctx: "128K", price: "$2.50/$10 per Mtok", recommended: true, sub: "Multimodal flagship" },
      { id: "gpt-4o-mini", name: "GPT-4o-mini", ctx: "128K", price: "$0.15/$0.60 per Mtok", sub: "Cost-efficient" },
      { id: "o1-pro", name: "o1-pro", ctx: "200K", price: "$15/$60 per Mtok", sub: "Deep reasoning" },
    ],
    minimax: [
      { id: "abab-7-chat", name: "abab-7-chat", ctx: "245K", price: "$0.20/$0.80 per Mtok", recommended: true, sub: "Featured throughput model" },
      { id: "abab-6.5s-chat", name: "abab-6.5s-chat", ctx: "245K", price: "$0.10/$0.40 per Mtok", sub: "Lighter, faster" },
    ],
  };

  const list = models[provider] || models.anthropic;

  return (
    <div style={{ flex: 1, display: "flex", flexDirection: "column", padding: "16px 18px", overflow: "auto" }}>
      <div style={{ marginBottom: 14 }}>
        <div style={{ color: C.fg, fontSize: 16, fontWeight: 700, marginBottom: 4 }}>Pick a model</div>
        <div style={{ color: C.dim, fontSize: 11 }}>From <span style={{ color: C.accentBright }}>{provider}</span>'s catalog. You can change anytime via <span style={{ color: C.accentBright }}>zeus config set model ...</span></div>
      </div>

      <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
        {list.map(m => (
          <div
            key={m.id}
            onClick={() => onSelect(m.id)}
            style={{
              border: `1px solid ${selectedModel === m.id ? C.accent : C.muted}`,
              background: selectedModel === m.id ? C.accentFaint : C.bg2,
              borderLeft: `2px solid ${selectedModel === m.id ? C.accent : C.dim}`,
              padding: "10px 14px", cursor: "pointer",
              display: "flex", alignItems: "center", gap: 12,
            }}
          >
            <span style={{
              width: 14, height: 14, display: "inline-flex", alignItems: "center", justifyContent: "center",
              background: selectedModel === m.id ? C.accent : "transparent",
              color: selectedModel === m.id ? C.bg : C.muted,
              border: `1px solid ${selectedModel === m.id ? C.accent : C.muted}`, borderRadius: 7,
              fontSize: 8, fontWeight: 700,
            }}>{selectedModel === m.id ? "●" : ""}</span>

            <div style={{ flex: 1 }}>
              <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
                <span style={{ color: selectedModel === m.id ? C.white : C.fg, fontWeight: 600, fontSize: 12 }}>{m.name}</span>
                <span style={{ color: C.muted, fontSize: 10 }}>{m.id}</span>
                {m.recommended && <span style={{ color: C.green, fontSize: 8, fontWeight: 700, letterSpacing: 2 }}>★ RECOMMENDED</span>}
              </div>
              <div style={{ color: C.dim, fontSize: 10, marginTop: 1 }}>{m.sub}</div>
            </div>

            <div style={{ display: "flex", flexDirection: "column", gap: 2, alignItems: "flex-end", minWidth: 140 }}>
              <span style={{ fontSize: 9, color: C.muted, letterSpacing: 2, fontWeight: 700 }}>CONTEXT</span>
              <span style={{ fontSize: 12, color: C.accent, fontWeight: 700 }}>{m.ctx}</span>
            </div>
            <div style={{ display: "flex", flexDirection: "column", gap: 2, alignItems: "flex-end", minWidth: 180 }}>
              <span style={{ fontSize: 9, color: C.muted, letterSpacing: 2, fontWeight: 700 }}>PRICING</span>
              <span style={{ fontSize: 10, color: C.fg }}>{m.price}</span>
            </div>
          </div>
        ))}
      </div>

      {provider === "ollama" && (
        <div style={{ marginTop: 14, padding: "8px 12px", background: C.bg2, border: `1px solid ${C.cyan}`, borderLeft: `2px solid ${C.cyan}`, fontSize: 10 }}>
          <span style={{ color: C.cyan, fontWeight: 700, letterSpacing: 2 }}>● LIVE FETCH</span>
          <span style={{ color: C.dim, marginLeft: 8 }}>Models pulled from <span style={{ color: C.accentBright }}>localhost:11434/api/tags</span> · 12 models locally available</span>
        </div>
      )}
    </div>
  );
};

const FallbackStep = ({ chain, setChain, primary }) => {
  const candidates = LLM_PROVIDERS.filter(p => p.id !== primary);

  return (
    <div style={{ flex: 1, display: "flex", padding: "16px 18px", gap: 14, overflow: "auto" }}>
      <div style={{ flex: 1, display: "flex", flexDirection: "column" }}>
        <div style={{ marginBottom: 14 }}>
          <div style={{ color: C.fg, fontSize: 16, fontWeight: 700, marginBottom: 4 }}>Backup LLM chain</div>
          <div style={{ color: C.dim, fontSize: 11 }}>If your primary provider fails, the agent loop tries each fallback in order. Pick 0-3 backups.</div>
        </div>

        <div style={{ color: C.accentDim, fontSize: 9, fontWeight: 700, letterSpacing: 3, marginBottom: 6 }}>AVAILABLE</div>
        <div style={{ display: "flex", flexDirection: "column", gap: 4, marginBottom: 16 }}>
          {candidates.slice(0, 6).map(p => {
            const inChain = chain.includes(p.id);
            return (
              <div key={p.id} onClick={() => {
                if (inChain) setChain(chain.filter(x => x !== p.id));
                else setChain([...chain, p.id]);
              }} style={{
                display: "flex", alignItems: "center", gap: 10, padding: "6px 10px",
                border: `1px solid ${C.muted}`, borderLeft: `2px solid ${p.color}`,
                background: inChain ? C.accentFaint : C.bg2, cursor: "pointer", opacity: inChain ? 1 : 0.7,
              }}>
                <span style={{
                  width: 14, height: 14, display: "inline-flex", alignItems: "center", justifyContent: "center",
                  background: inChain ? C.accent : "transparent",
                  color: inChain ? C.bg : C.muted,
                  border: `1px solid ${inChain ? C.accent : C.muted}`,
                  fontSize: 9, fontWeight: 700,
                }}>{inChain ? "✓" : ""}</span>
                <span style={{ width: 26, fontSize: 8, color: p.color, fontWeight: 700, textAlign: "center", letterSpacing: 1 }}>{p.glyph}</span>
                <span style={{ color: C.fg, fontSize: 11, flex: 1 }}>{p.name}</span>
                <span style={{ color: C.dim, fontSize: 9 }}>{p.flagship}</span>
              </div>
            );
          })}
        </div>
      </div>

      <div style={{ width: 360, padding: "0 0 0 14px", borderLeft: `1px solid ${C.muted}` }}>
        <div style={{ color: C.accentDim, fontSize: 9, fontWeight: 700, letterSpacing: 3, marginBottom: 6 }}>FALLBACK CHAIN ({chain.length})</div>
        <div style={{ fontSize: 10, color: C.dim, marginBottom: 10 }}>Reorder with <span style={{ color: C.accent }}>[</span> / <span style={{ color: C.accent }}>]</span></div>

        {chain.length === 0 && (
          <div style={{ padding: "16px 14px", border: `1px dashed ${C.muted}`, color: C.muted, fontSize: 10, textAlign: "center" }}>
            No fallbacks selected.<br />Primary failures will fail the agent loop.
          </div>
        )}

        {chain.map((id, i) => {
          const p = LLM_PROVIDERS.find(x => x.id === id);
          return (
            <div key={id} style={{
              display: "flex", alignItems: "center", gap: 10, padding: "8px 12px",
              border: `1px solid ${C.accent}`, borderLeft: `3px solid ${C.accent}`,
              background: C.accentFaint, marginBottom: 4,
            }}>
              <span style={{ width: 18, height: 18, display: "inline-flex", alignItems: "center", justifyContent: "center", background: C.accent, color: C.bg, fontSize: 10, fontWeight: 700 }}>{i + 1}</span>
              <span style={{ flex: 1 }}>
                <div style={{ color: C.white, fontSize: 12, fontWeight: 600 }}>{p.name}</div>
                <div style={{ color: C.dim, fontSize: 9 }}>{p.flagship}</div>
              </span>
              <span style={{ color: C.dim, fontSize: 12, cursor: "pointer" }}>↑</span>
              <span style={{ color: C.dim, fontSize: 12, cursor: "pointer" }}>↓</span>
              <span style={{ color: C.dim, fontSize: 11, cursor: "pointer" }} onClick={() => setChain(chain.filter(x => x !== id))}>✕</span>
            </div>
          );
        })}

        {chain.length > 0 && (
          <div style={{ marginTop: 10, padding: "8px 10px", background: C.bg2, border: `1px solid ${C.muted}`, fontSize: 9, color: C.dim }}>
            <div style={{ color: C.accentDim, fontWeight: 700, letterSpacing: 2, marginBottom: 4 }}>SUGGESTED</div>
            Based on Anthropic primary, consider adding <span style={{ color: C.accent, fontWeight: 700 }}>OpenAI + Groq</span> for cheap-fast fallback.
          </div>
        )}
      </div>
    </div>
  );
};

const ChannelsStep = ({ toggled, onToggle, focused, setFocused }) => {
  const groups = ["Cloud APIs", "Phone-paired"];
  return (
    <div style={{ flex: 1, display: "flex", padding: "16px 18px", gap: 16, overflow: "auto" }}>
      <div style={{ flex: 1 }}>
        <div style={{ marginBottom: 14 }}>
          <div style={{ color: C.fg, fontSize: 16, fontWeight: 700, marginBottom: 4 }}>Pick messaging channels</div>
          <div style={{ color: C.dim, fontSize: 11 }}>Select which channels Zeus should bridge. Per-channel credentials collected next.</div>
        </div>

        {groups.map(group => (
          <div key={group} style={{ marginBottom: 18 }}>
            <div style={{ display: "flex", alignItems: "center", gap: 8, marginBottom: 8 }}>
              <span style={{ color: C.accentDim, fontSize: 9, fontWeight: 700, letterSpacing: 3 }}>{group.toUpperCase()}</span>
              <span style={{ flex: 1, height: 1, background: C.muted }} />
              <span style={{ color: C.muted, fontSize: 9 }}>
                {group === "Cloud APIs" ? "API key auth" : "QR pairing required"}
              </span>
            </div>
            <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 6 }}>
              {CHANNELS.filter(c => c.group === group).map(c => (
                <Card key={c.id} {...c} multiselect toggled={toggled.has(c.id)} focused={focused === c.id}
                  onClick={() => onToggle(c.id)} />
              ))}
            </div>
          </div>
        ))}
      </div>

      <div style={{ width: 280, padding: "0 0 0 14px", borderLeft: `1px solid ${C.muted}` }}>
        <div style={{ color: C.accentDim, fontSize: 9, fontWeight: 700, letterSpacing: 3, marginBottom: 8 }}>SELECTED ({toggled.size})</div>
        <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
          {[...toggled].map(id => {
            const c = CHANNELS.find(x => x.id === id);
            return (
              <div key={id} style={{
                display: "flex", alignItems: "center", gap: 8, padding: "6px 10px",
                background: C.bg2, border: `1px solid ${C.muted}`, borderLeft: `2px solid ${c.color}`,
              }}>
                <span style={{ color: c.color, fontWeight: 700, fontSize: 10, width: 24 }}>{c.glyph}</span>
                <span style={{ color: C.fg, fontSize: 11, flex: 1 }}>{c.name}</span>
                <span style={{ color: C.dim, fontSize: 9 }}>next: config</span>
              </div>
            );
          })}
        </div>
        {toggled.size === 0 && (
          <div style={{ padding: "16px 12px", border: `1px dashed ${C.muted}`, fontSize: 10, color: C.muted, textAlign: "center" }}>
            No channels selected.<br />Zeus will run console-only.
          </div>
        )}
      </div>
    </div>
  );
};

const ChanConfigStep = ({ toggled, configValues, setConfigValue, focusedField, setFocusedField, testStatuses, onTest }) => {
  const channels = CHANNELS.filter(c => toggled.has(c.id));
  const fieldsByChannel = {
    telegram: [
      { key: "api_id", label: "API ID", placeholder: "12345678", required: true },
      { key: "api_hash", label: "API Hash", placeholder: "...", secret: true, required: true },
      { key: "phone", label: "Phone", placeholder: "+1234567890", required: true },
    ],
    discord: [
      { key: "token", label: "Bot Token", placeholder: "MTAxxxx...", secret: true, required: true },
      { key: "channel_id", label: "Default Channel ID", placeholder: "1234567890", required: false },
    ],
    slack: [
      { key: "bot_token", label: "Bot Token", placeholder: "xoxb-...", secret: true, required: true },
      { key: "app_token", label: "App Token", placeholder: "xapp-...", secret: true, required: true },
    ],
    email: [
      { key: "smtp_host", label: "SMTP Host", placeholder: "smtp.gmail.com", required: true },
      { key: "smtp_port", label: "SMTP Port", placeholder: "587", default: "587", required: true },
      { key: "username", label: "Username", placeholder: "[email protected]", required: true },
      { key: "password", label: "App Password", placeholder: "...", secret: true, required: true },
    ],
    imessage: [],
    whatsapp: [
      { key: "phone_id", label: "Phone Number ID", placeholder: "...", required: true },
      { key: "access_token", label: "Access Token", placeholder: "...", secret: true, required: true },
    ],
    signal: [],
    matrix: [
      { key: "homeserver", label: "Homeserver URL", placeholder: "https://matrix.org", required: true },
      { key: "username", label: "Username", placeholder: "@user:matrix.org", required: true },
      { key: "password", label: "Password", placeholder: "...", secret: true, required: true },
    ],
  };

  return (
    <div style={{ flex: 1, padding: "16px 18px", overflow: "auto" }}>
      <div style={{ marginBottom: 14 }}>
        <div style={{ color: C.fg, fontSize: 16, fontWeight: 700, marginBottom: 4 }}>Configure {channels.length} channel{channels.length !== 1 ? "s" : ""}</div>
        <div style={{ color: C.dim, fontSize: 11 }}>All channels visible — fill in any order. Test buttons send a "Zeus connected ✅" message to verify.</div>
      </div>

      <div style={{ display: "flex", flexDirection: "column", gap: 14 }}>
        {channels.map(c => {
          const fields = fieldsByChannel[c.id] || [];
          const isQR = c.id === "signal" || c.id === "whatsapp";
          const isAppleScript = c.id === "imessage";
          const testState = testStatuses[c.id];

          return (
            <div key={c.id} style={{
              border: `1px solid ${C.muted}`, borderLeft: `2px solid ${c.color}`,
              background: C.bg2,
            }}>
              <div style={{ padding: "10px 14px", borderBottom: `1px solid ${C.muted}`, display: "flex", alignItems: "center", gap: 10 }}>
                <span style={{
                  width: 32, height: 18, display: "inline-flex", alignItems: "center", justifyContent: "center",
                  background: c.color, color: C.bg, fontWeight: 700, fontSize: 9, letterSpacing: 1,
                  border: `1px solid ${c.color}`,
                }}>{c.glyph}</span>
                <span style={{ color: C.white, fontWeight: 700, fontSize: 13 }}>{c.name}</span>
                <span style={{ color: C.dim, fontSize: 10, fontStyle: "italic" }}>{c.sdk}</span>
                <span style={{ flex: 1 }} />
                {isQR && (
                  <span style={{ color: C.amber, fontSize: 9, fontWeight: 700, letterSpacing: 2, padding: "2px 6px", border: `1px solid ${C.amber}` }}>QR PAIRING</span>
                )}
                {isAppleScript && (
                  <span style={{ color: C.cyan, fontSize: 9, fontWeight: 700, letterSpacing: 2, padding: "2px 6px", border: `1px solid ${C.cyan}` }}>APPLESCRIPT</span>
                )}
                {testState === "success" && (
                  <span style={{ color: C.green, fontSize: 9, fontWeight: 700 }}>✓ TESTED</span>
                )}
              </div>

              <div style={{ padding: "10px 14px" }}>
                {isAppleScript && (
                  <div style={{ fontSize: 11, color: C.fg, padding: "8px 0" }}>
                    <span style={{ color: C.cyan }}>●</span> Uses native macOS bridge. No credentials needed. Will request Messages permission on first use.
                  </div>
                )}
                {isQR && (
                  <div style={{ fontSize: 11, color: C.fg, padding: "8px 0" }}>
                    <span style={{ color: C.amber }}>⚠</span> Requires phone-side QR scan. Pairing screen will display after this step.
                  </div>
                )}
                {fields.map(f => (
                  <Field
                    key={f.key}
                    {...f}
                    value={configValues[`${c.id}.${f.key}`]}
                    onChange={(v) => setConfigValue(`${c.id}.${f.key}`, v)}
                    focused={focusedField === `${c.id}.${f.key}`}
                    onFocus={() => setFocusedField(`${c.id}.${f.key}`)}
                  />
                ))}
                {fields.length > 0 && (
                  <div style={{ marginTop: 8, paddingLeft: 152 }}>
                    <button onClick={() => onTest(c.id)} disabled={testState === "testing"} style={{
                      background: testState === "success" ? C.greenDim : C.accentFaint,
                      border: `1px solid ${testState === "success" ? C.green : C.accent}`,
                      color: testState === "success" ? C.green : C.accent,
                      padding: "4px 12px", fontFamily: "inherit", fontSize: 9,
                      fontWeight: 700, letterSpacing: 2, cursor: "pointer",
                    }}>
                      {testState === "testing" ? "▸ SENDING..." : testState === "success" ? "✓ DELIVERED" : "▸ SEND TEST"}
                    </button>
                    {testState === "success" && (
                      <span style={{ marginLeft: 12, color: C.green, fontSize: 10 }}>● Test message delivered to {c.name}</span>
                    )}
                  </div>
                )}
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
};

const SignalPairStep = () => (
  <div style={{ flex: 1, display: "flex", flexDirection: "column", alignItems: "center", justifyContent: "center", padding: "20px", overflow: "auto" }}>
    <div style={{ marginBottom: 14, textAlign: "center" }}>
      <div style={{ color: C.fg, fontSize: 18, fontWeight: 700, marginBottom: 4 }}>Pair Signal device</div>
      <div style={{ color: C.dim, fontSize: 11 }}>Open Signal on your phone → Settings → Linked Devices → Link New Device → scan</div>
    </div>

    <div style={{ display: "flex", gap: 24, alignItems: "stretch" }}>
      {/* QR */}
      <div style={{
        width: 240, height: 240, background: C.fg, padding: 14,
        display: "flex", alignItems: "center", justifyContent: "center",
        border: `1px solid ${C.muted}`,
      }}>
        <div style={{ width: "100%", height: "100%", background: "white", padding: 0, position: "relative" }}>
          {/* Simulated QR pattern */}
          <div style={{ display: "grid", gridTemplateColumns: "repeat(25, 1fr)", gridTemplateRows: "repeat(25, 1fr)", width: "100%", height: "100%" }}>
            {Array.from({ length: 625 }).map((_, i) => {
              // Pseudo-random QR pattern
              const x = i % 25, y = Math.floor(i / 25);
              const isCorner = (x < 7 && y < 7) || (x > 17 && y < 7) || (x < 7 && y > 17);
              const cornerInner = (x >= 1 && x <= 5 && y >= 1 && y <= 5) || (x >= 18 && x <= 22 && y >= 1 && y <= 5) || (x >= 1 && x <= 5 && y >= 18 && y <= 22);
              const cornerCenter = (x >= 2 && x <= 4 && y >= 2 && y <= 4) || (x >= 19 && x <= 21 && y >= 2 && y <= 4) || (x >= 2 && x <= 4 && y >= 19 && y <= 21);
              let fill = "white";
              if (isCorner && !cornerInner) fill = "black";
              else if (isCorner && cornerCenter) fill = "black";
              else if (!isCorner) {
                const seed = (x * 7 + y * 13 + x * y) % 100;
                fill = seed < 48 ? "black" : "white";
              }
              return <div key={i} style={{ background: fill }} />;
            })}
          </div>
        </div>
      </div>

      <div style={{ display: "flex", flexDirection: "column", justifyContent: "space-between", minWidth: 240 }}>
        <div>
          <div style={{ color: C.accentDim, fontSize: 9, fontWeight: 700, letterSpacing: 3, marginBottom: 6 }}>STATUS</div>
          <div style={{ display: "flex", alignItems: "center", gap: 8, marginBottom: 4 }}>
            <span style={{ width: 8, height: 8, borderRadius: 4, background: C.amber, boxShadow: `0 0 6px ${C.amber}` }} />
            <span style={{ color: C.fg, fontSize: 12 }}>Waiting for scan...</span>
          </div>
          <div style={{ color: C.muted, fontSize: 10, marginTop: 4 }}>QR expires in <span style={{ color: C.amber }}>4:23</span></div>
        </div>

        <div>
          <div style={{ color: C.accentDim, fontSize: 9, fontWeight: 700, letterSpacing: 3, marginBottom: 6 }}>HAVING TROUBLE?</div>
          <div style={{ fontSize: 10, color: C.dim, lineHeight: 1.6 }}>
            QR rendering can vary by terminal:<br />
            <span style={{ color: C.cyan }}>iTerm2</span> — sharpest, recommended<br />
            <span style={{ color: C.cyan }}>Apple Terminal</span> — works, blockier<br />
            <span style={{ color: C.cyan }}>tmux</span> — may need <span style={{ color: C.accentBright }}>set-window-option utf8 on</span>
          </div>
        </div>
      </div>
    </div>

    <div style={{ marginTop: 18, fontSize: 10, color: C.muted }}>
      Press <span style={{ color: C.accentDim, fontWeight: 700 }}>r</span> to refresh QR · <span style={{ color: C.accentDim, fontWeight: 700 }}>Esc</span> to skip
    </div>
  </div>
);

const GatewayStep = ({ values, setValue, focusedField, setFocusedField, serviceMode, setServiceMode }) => {
  const services = [
    { id: "launchd", name: "launchd", glyph: "MAC", color: C.accent, sub: "macOS native (recommended)", path: "~/Library/LaunchAgents/ai.zeuslab.gateway.plist" },
    { id: "systemd", name: "systemd", glyph: "LIN", color: C.green, sub: "Linux native", path: "/etc/systemd/system/zeus-gateway.service" },
    { id: "rcd", name: "rc.d", glyph: "BSD", color: C.cyan, sub: "FreeBSD native", path: "/usr/local/etc/rc.d/zeus_gateway" },
    { id: "manual", name: "Manual start", glyph: "—", color: C.dim, sub: "I'll start zeus manually", path: null },
  ];
  const portInUse = values.port === "8080";

  return (
    <div style={{ flex: 1, padding: "16px 18px", overflow: "auto" }}>
      <div style={{ marginBottom: 14 }}>
        <div style={{ color: C.fg, fontSize: 16, fontWeight: 700, marginBottom: 4 }}>Configure gateway</div>
        <div style={{ color: C.dim, fontSize: 11 }}>The gateway hosts the API, WebUI, and agent processing loop.</div>
      </div>

      <div style={{ marginBottom: 16 }}>
        <div style={{ color: C.accentDim, fontSize: 9, fontWeight: 700, letterSpacing: 3, marginBottom: 8 }}>BIND</div>
        <Field label="Host" value={values.host || "127.0.0.1"} onChange={v => setValue("host", v)} placeholder="127.0.0.1" required focused={focusedField === "host"} onFocus={() => setFocusedField("host")} hint="Use 0.0.0.0 to expose on LAN" />
        <Field label="Port" value={values.port || "8080"} onChange={v => setValue("port", v)} placeholder="8080" required focused={focusedField === "port"} onFocus={() => setFocusedField("port")}
          error={portInUse && values.port === "8080" ? "Port 8080 in use by PID 47291 (zeus). Pick a different port or stop the existing instance." : null} />
      </div>

      <div style={{ marginBottom: 16 }}>
        <div style={{ color: C.accentDim, fontSize: 9, fontWeight: 700, letterSpacing: 3, marginBottom: 8 }}>FEATURES</div>
        {[
          { key: "agent_processing", label: "Agent Processing Loop", desc: "Background heartbeat + cron + watchdog", default: true },
          { key: "webui", label: "WebUI Co-host", desc: "Serves Leptos frontend on the same port (or 8081 if 8080 is taken)", default: true },
          { key: "mcp", label: "MCP Server", desc: "Model Context Protocol endpoint for Claude Desktop / cursor", default: false },
        ].map(t => (
          <div key={t.key} style={{
            display: "flex", alignItems: "center", gap: 12, padding: "8px 12px",
            border: `1px solid ${C.muted}`, marginBottom: 4, background: C.bg2,
          }}>
            <div onClick={() => setValue(t.key, !values[t.key])} style={{
              width: 30, height: 16, borderRadius: 8, position: "relative", cursor: "pointer",
              background: (values[t.key] ?? t.default) ? C.accent : C.dark,
              border: `1px solid ${(values[t.key] ?? t.default) ? C.accent : C.muted}`,
            }}>
              <div style={{
                width: 12, height: 12, borderRadius: 6, position: "absolute",
                top: 1, left: (values[t.key] ?? t.default) ? 16 : 1,
                background: (values[t.key] ?? t.default) ? C.bg : C.dim,
                transition: "left 0.1s",
              }} />
            </div>
            <div style={{ flex: 1 }}>
              <div style={{ color: C.fg, fontSize: 11, fontWeight: 600 }}>{t.label}</div>
              <div style={{ color: C.dim, fontSize: 10 }}>{t.desc}</div>
            </div>
          </div>
        ))}
      </div>

      <div>
        <div style={{ color: C.accentDim, fontSize: 9, fontWeight: 700, letterSpacing: 3, marginBottom: 8 }}>INSTALL AS SERVICE</div>
        <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr 1fr 1fr", gap: 6 }}>
          {services.map(s => (
            <Card key={s.id} {...s} selected={serviceMode === s.id} onClick={() => setServiceMode(s.id)} dim={s.id !== "launchd" && s.id !== "manual"} />
          ))}
        </div>
        {serviceMode && services.find(s => s.id === serviceMode)?.path && (
          <div style={{ marginTop: 8, padding: "6px 10px", background: C.bg2, border: `1px solid ${C.muted}`, fontSize: 10, color: C.dim }}>
            <span style={{ color: C.accentDim, fontWeight: 700, letterSpacing: 2 }}>WILL INSTALL</span>
            <span style={{ marginLeft: 8, color: C.accentBright }}>{services.find(s => s.id === serviceMode).path}</span>
          </div>
        )}
      </div>
    </div>
  );
};

const AgentStep = ({ persona, setPersona, values, setValue, focusedField, setFocusedField, hostname }) => {
  const p = PERSONAS.find(x => x.id === persona);
  const suggestedName = hostname ? `zeus${hostname.split(".").pop()}` : "Zeus100";

  return (
    <div style={{ flex: 1, display: "flex", padding: "16px 18px", gap: 14, overflow: "auto" }}>
      <div style={{ flex: 1 }}>
        <div style={{ marginBottom: 14 }}>
          <div style={{ color: C.fg, fontSize: 16, fontWeight: 700, marginBottom: 4 }}>Agent persona</div>
          <div style={{ color: C.dim, fontSize: 11 }}>Pick an archetype to seed your agent's <span style={{ color: C.accentBright }}>SOUL.md</span>. Customize freely after onboarding.</div>
        </div>

        <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 6, marginBottom: 16 }}>
          {PERSONAS.map(p => (
            <Card key={p.id} {...p} selected={persona === p.id} onClick={() => setPersona(p.id)} />
          ))}
        </div>

        <div style={{ color: C.accentDim, fontSize: 9, fontWeight: 700, letterSpacing: 3, marginBottom: 8 }}>IDENTITY</div>
        <Field label="Agent Name" value={values.name || suggestedName} onChange={v => setValue("name", v)} placeholder={suggestedName} required focused={focusedField === "name"} onFocus={() => setFocusedField("name")}
          hint={`Auto-suggested from hostname: ${hostname || "(unknown)"}`} />
        <Field label="Role" value={values.role || (p && p.name)} onChange={v => setValue("role", v)} placeholder="Coordinator" focused={focusedField === "role"} onFocus={() => setFocusedField("role")} />
        <Field label="Tone" value={values.tone || (p && p.tone)} onChange={v => setValue("tone", v)} placeholder="professional, direct" focused={focusedField === "tone"} onFocus={() => setFocusedField("tone")} hint="Used in SOUL.md prompt seed" />
      </div>

      <div style={{ width: 360, padding: "0 0 0 14px", borderLeft: `1px solid ${C.muted}`, display: "flex", flexDirection: "column" }}>
        <div style={{ color: C.accentDim, fontSize: 9, fontWeight: 700, letterSpacing: 3, marginBottom: 6 }}>SOUL.MD PREVIEW</div>
        <div style={{ flex: 1, padding: "10px 12px", background: C.bg2, border: `1px solid ${C.muted}`, fontSize: 10, color: C.fg, fontFamily: "inherit", lineHeight: 1.6 }}>
          <div style={{ color: C.accent }}># {values.name || suggestedName}</div>
          <div style={{ color: C.dim, marginTop: 6 }}>## Role</div>
          <div>{values.role || (p && p.name)}</div>
          <div style={{ color: C.dim, marginTop: 6 }}>## Tone</div>
          <div>{values.tone || (p && p.tone)}</div>
          <div style={{ color: C.dim, marginTop: 6 }}>## Guiding Principles</div>
          {persona === "coordinator" && <div>- Make decisions quickly when blocked.<br />- Delegate clearly. Track outcomes.<br />- Escalate to humans only when truly ambiguous.</div>}
          {persona === "engineer" && <div>- Read existing code before writing new.<br />- Tests pass before commit.<br />- One thing at a time.</div>}
          {persona === "creative" && <div>- Voice over voicelessness.<br />- Specific over generic.<br />- Iterate until it sings.</div>}
        </div>
        <div style={{ marginTop: 6, fontSize: 9, color: C.muted, textAlign: "right" }}>
          Live preview · writes to <span style={{ color: C.accentBright }}>~/.zeus/workspace/SOUL.md</span>
        </div>
      </div>
    </div>
  );
};

const WorkspaceStep = ({ values, setValue, focusedField, setFocusedField, existingDetected }) => (
  <div style={{ flex: 1, padding: "16px 18px", overflow: "auto" }}>
    <div style={{ marginBottom: 14 }}>
      <div style={{ color: C.fg, fontSize: 16, fontWeight: 700, marginBottom: 4 }}>Workspace paths</div>
      <div style={{ color: C.dim, fontSize: 11 }}>Where Zeus stores your agent's working memory, sessions, and journal.</div>
    </div>

    {existingDetected && (
      <div style={{ marginBottom: 14, padding: "10px 14px", border: `1px solid ${C.amber}`, borderLeft: `2px solid ${C.amber}`, background: C.bg2 }}>
        <div style={{ color: C.amber, fontSize: 9, fontWeight: 700, letterSpacing: 3, marginBottom: 6 }}>↻ EXISTING WORKSPACE FOUND</div>
        <div style={{ color: C.fg, fontSize: 11, marginBottom: 8 }}>
          <span style={{ color: C.accentBright }}>~/.zeus/workspace</span> contains <span style={{ color: C.fg, fontWeight: 700 }}>2,847</span> memory facts, <span style={{ color: C.fg, fontWeight: 700 }}>147</span> sessions, last modified <span style={{ color: C.fg, fontWeight: 700 }}>2 minutes ago</span>.
        </div>
        <div style={{ display: "flex", gap: 8 }}>
          <button style={{ background: C.accent, color: C.bg, border: "none", padding: "5px 12px", fontSize: 9, fontWeight: 700, letterSpacing: 2, cursor: "pointer", fontFamily: "inherit" }}>USE EXISTING</button>
          <button style={{ background: "transparent", color: C.dim, border: `1px solid ${C.muted}`, padding: "5px 12px", fontSize: 9, fontWeight: 700, letterSpacing: 2, cursor: "pointer", fontFamily: "inherit" }}>START FRESH (BACKUP OLD)</button>
        </div>
      </div>
    )}

    <div style={{ color: C.accentDim, fontSize: 9, fontWeight: 700, letterSpacing: 3, marginBottom: 8 }}>PATHS</div>
    <Field label="Workspace" value={values.workspace || "~/.zeus/workspace"} onChange={v => setValue("workspace", v)} required focused={focusedField === "workspace"} onFocus={() => setFocusedField("workspace")} hint="AGENTS.md, SOUL.md, journals, daily notes" />
    <Field label="Sessions" value={values.sessions || "~/.zeus/sessions"} onChange={v => setValue("sessions", v)} required focused={focusedField === "sessions"} onFocus={() => setFocusedField("sessions")} hint="Per-conversation JSONL logs (grows ~5MB/day per active agent)" />
    <Field label="Mnemosyne DB" value={values.mnem || "~/.zeus/mnemosyne.db"} onChange={v => setValue("mnem", v)} focused={focusedField === "mnem"} onFocus={() => setFocusedField("mnem")} hint="SQLite + vector embeddings (can grow to GBs)" />

    <div style={{ marginTop: 16, padding: "10px 14px", background: C.bg2, border: `1px solid ${C.muted}`, fontSize: 10 }}>
      <div style={{ color: C.accentDim, fontSize: 9, fontWeight: 700, letterSpacing: 3, marginBottom: 6 }}>DISK USAGE PROJECTION</div>
      <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr 1fr", gap: 12 }}>
        {[
          ["Workspace", "~50 MB", "after 30 days"],
          ["Sessions", "~150 MB", "@ 5 MB/day for 30d"],
          ["Mnemosyne", "~800 MB", "after 1000 sessions"],
        ].map(([k, v, sub]) => (
          <div key={k}>
            <div style={{ color: C.dim, fontSize: 9, fontWeight: 700, letterSpacing: 1 }}>{k}</div>
            <div style={{ color: C.accent, fontSize: 14, fontWeight: 700, marginTop: 2 }}>{v}</div>
            <div style={{ color: C.muted, fontSize: 9 }}>{sub}</div>
          </div>
        ))}
      </div>
    </div>
  </div>
);

const SecurityStep = ({ selected, onSelect }) => (
  <div style={{ flex: 1, padding: "16px 18px", overflow: "auto" }}>
    <div style={{ marginBottom: 14 }}>
      <div style={{ color: C.fg, fontSize: 16, fontWeight: 700, marginBottom: 4 }}>Aegis security level</div>
      <div style={{ color: C.dim, fontSize: 11 }}>Sandbox aggressiveness for tool execution. Approval pipeline is always active regardless of level.</div>
    </div>

    <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr 1fr 1fr", gap: 8, marginBottom: 16 }}>
      {SECURITY_LEVELS.map(s => (
        <div key={s.id} onClick={() => onSelect(s.id)} style={{
          background: selected === s.id ? C.accentFaint : C.bg2,
          border: `1px solid ${selected === s.id ? C.accent : s.recommended ? C.green : C.muted}`,
          borderLeft: `2px solid ${s.color}`,
          padding: "12px 14px", cursor: "pointer", position: "relative",
          minHeight: 200,
        }}>
          {selected === s.id && (
            <span style={{ position: "absolute", top: 8, right: 10, color: C.accent, fontSize: 9, fontWeight: 700, letterSpacing: 2 }}>▸ SELECTED</span>
          )}
          {s.recommended && !selected && (
            <span style={{ position: "absolute", top: 8, right: 10, color: C.green, fontSize: 8, fontWeight: 700, letterSpacing: 2 }}>★ REC</span>
          )}
          <div style={{
            width: 36, height: 36, background: s.color, color: C.bg,
            display: "flex", alignItems: "center", justifyContent: "center",
            fontWeight: 700, fontSize: 11, letterSpacing: 2, marginBottom: 8,
          }}>{s.glyph}</div>
          <div style={{ color: C.white, fontSize: 14, fontWeight: 700, marginBottom: 2 }}>{s.name}</div>
          <div style={{ color: C.dim, fontSize: 9, marginBottom: 8, fontStyle: "italic" }}>{s.sub}</div>
          {s.blocked.length > 0 && (
            <div>
              <div style={{ color: C.red, fontSize: 8, fontWeight: 700, letterSpacing: 2, marginBottom: 2 }}>BLOCKED</div>
              <ul style={{ listStyle: "none", padding: 0, fontSize: 9, color: C.dim }}>
                {s.blocked.slice(0, 3).map((b, i) => <li key={i} style={{ marginBottom: 1 }}>✕ {b}</li>)}
              </ul>
            </div>
          )}
        </div>
      ))}
    </div>

    {selected && (() => {
      const sec = SECURITY_LEVELS.find(s => s.id === selected);
      return (
        <div style={{ padding: "12px 14px", background: C.bg2, border: `1px solid ${C.muted}` }}>
          <div style={{ display: "flex", alignItems: "center", gap: 8, marginBottom: 6 }}>
            <span style={{ color: C.accentDim, fontSize: 9, fontWeight: 700, letterSpacing: 3 }}>SELECTED: {sec.name.toUpperCase()}</span>
          </div>
          <div style={{ color: C.fg, fontSize: 11 }}>
            Will write <span style={{ color: C.accentBright }}>[aegis] level = "{selected}"</span> to ~/.zeus/config.toml
          </div>
        </div>
      );
    })()}
  </div>
);

const FeaturesStep = ({ toggled, onToggle, platform = "macOS" }) => (
  <div style={{ flex: 1, padding: "16px 18px", overflow: "auto" }}>
    <div style={{ marginBottom: 14 }}>
      <div style={{ color: C.fg, fontSize: 16, fontWeight: 700, marginBottom: 4 }}>Enable subsystems</div>
      <div style={{ color: C.dim, fontSize: 11 }}>Toggle which Zeus crates are active in this deployment. Disabled crates compile but don't load.</div>
    </div>

    {/* Talos warning banner */}
    <div style={{ marginBottom: 14, padding: "10px 14px", background: C.bg2, border: `1px solid ${C.accent}`, borderLeft: `2px solid ${C.accent}` }}>
      <div style={{ color: C.accent, fontSize: 9, fontWeight: 700, letterSpacing: 3, marginBottom: 4 }}>⚠ MACOS GATE — TALOS IS MANDATORY</div>
      <div style={{ color: C.fg, fontSize: 11, lineHeight: 1.5 }}>
        On macOS, the <span style={{ color: C.accentBright }}>[talos]</span> block must be present (even if empty) or 193 tools — including image-gen, AppleScript, system-info — silently fail to register. Talos is force-enabled here regardless of toggle.
      </div>
    </div>

    <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
      {FEATURES.map(f => {
        const isMandatory = f.required_on === platform;
        const isEnabled = isMandatory || toggled.has(f.id);
        return (
          <div key={f.id} onClick={() => !isMandatory && onToggle(f.id)} style={{
            display: "flex", alignItems: "center", gap: 12, padding: "10px 14px",
            border: `1px solid ${isMandatory ? C.accent : C.muted}`, borderLeft: `2px solid ${f.color}`,
            background: isEnabled ? C.bg2 : C.bg, cursor: isMandatory ? "default" : "pointer",
          }}>
            <div style={{
              width: 30, height: 16, borderRadius: 8, position: "relative",
              background: isEnabled ? f.color : C.dark,
              border: `1px solid ${isEnabled ? f.color : C.muted}`,
              opacity: isMandatory ? 0.7 : 1,
            }}>
              <div style={{
                width: 12, height: 12, borderRadius: 6, position: "absolute",
                top: 1, left: isEnabled ? 16 : 1,
                background: isEnabled ? C.bg : C.dim,
              }} />
            </div>
            <div style={{ flex: 1 }}>
              <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
                <span style={{ color: C.fg, fontSize: 12, fontWeight: 600 }}>{f.name}</span>
                {isMandatory && <span style={{ color: C.accent, fontSize: 8, fontWeight: 700, letterSpacing: 2, padding: "1px 4px", background: C.accentFaint, border: `1px solid ${C.accent}` }}>FORCE-ON ON {platform.toUpperCase()}</span>}
              </div>
              <div style={{ color: C.dim, fontSize: 10, marginTop: 1 }}>{f.desc}</div>
              {f.warning && isMandatory && (
                <div style={{ color: C.amber, fontSize: 9, marginTop: 4, fontStyle: "italic" }}>⚠ {f.warning}</div>
              )}
            </div>
            <span style={{ color: isEnabled ? C.green : C.muted, fontSize: 9, fontWeight: 700, letterSpacing: 2 }}>
              {isEnabled ? "● ON" : "○ OFF"}
            </span>
          </div>
        );
      })}
    </div>
  </div>
);

const VoiceStep = ({ selected, onSelect, values, setValue, focusedField, setFocusedField }) => (
  <div style={{ flex: 1, display: "flex", padding: "16px 18px", gap: 14, overflow: "auto" }}>
    <div style={{ flex: 1 }}>
      <div style={{ marginBottom: 14 }}>
        <div style={{ color: C.fg, fontSize: 16, fontWeight: 700, marginBottom: 4 }}>Voice / TTS provider</div>
        <div style={{ color: C.dim, fontSize: 11 }}>Powers <span style={{ color: C.accentBright }}>voice_say</span>, <span style={{ color: C.accentBright }}>voice_call</span>, and Twilio outbound calls.</div>
      </div>

      <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
        {VOICE_PROVIDERS.map(v => (
          <Card key={v.id} {...v} selected={selected === v.id} onClick={() => onSelect(v.id)} />
        ))}
      </div>
    </div>

    <div style={{ width: 380, padding: "0 0 0 14px", borderLeft: `1px solid ${C.muted}` }}>
      {selected && selected !== "none" && (() => {
        const v = VOICE_PROVIDERS.find(x => x.id === selected);
        return (
          <>
            <div style={{ display: "flex", alignItems: "flex-start", gap: 10, marginBottom: 14 }}>
              <div style={{ width: 42, height: 42, background: v.color, color: C.bg, display: "flex", alignItems: "center", justifyContent: "center", fontWeight: 700, fontSize: 12, letterSpacing: 1 }}>{v.glyph}</div>
              <div>
                <div style={{ color: C.white, fontSize: 14, fontWeight: 700 }}>{v.name}</div>
                <div style={{ color: C.dim, fontSize: 11 }}>{v.sub}</div>
              </div>
            </div>

            <div style={{ color: C.accentDim, fontSize: 9, fontWeight: 700, letterSpacing: 3, marginBottom: 8 }}>CREDENTIALS</div>
            <Field label="API Key" value={values.api_key} onChange={v => setValue("api_key", v)} placeholder="..." secret required focused={focusedField === "api_key"} onFocus={() => setFocusedField("api_key")} />
            <Field label="Voice ID" value={values.voice_id} onChange={v => setValue("voice_id", v)} placeholder="default" focused={focusedField === "voice_id"} onFocus={() => setFocusedField("voice_id")} />
            {selected === "custom" && (
              <Field label="Base URL" value={values.base_url} onChange={v => setValue("base_url", v)} placeholder="http://localhost:5000" required focused={focusedField === "base_url"} onFocus={() => setFocusedField("base_url")} />
            )}

            <div style={{ marginTop: 12, paddingLeft: 152 }}>
              <button style={{
                background: C.accentFaint, border: `1px solid ${C.accent}`, color: C.accent,
                padding: "5px 14px", fontFamily: "inherit", fontSize: 9, fontWeight: 700, letterSpacing: 2, cursor: "pointer",
              }}>▸ TEST VOICE</button>
            </div>
          </>
        );
      })()}
      {selected === "none" && (
        <div style={{ padding: "16px 14px", background: C.bg2, border: `1px solid ${C.yellow}`, borderLeft: `2px solid ${C.yellow}` }}>
          <div style={{ color: C.yellow, fontSize: 9, fontWeight: 700, letterSpacing: 3, marginBottom: 6 }}>⚠ NO VOICE CONFIGURED</div>
          <div style={{ color: C.fg, fontSize: 11 }}>Voice tools will be unavailable. Re-run <span style={{ color: C.accentBright }}>zeus onboard --resume voice</span> later.</div>
        </div>
      )}
    </div>
  </div>
);

const ImagesStep = ({ selected, onSelect, values, setValue, focusedField, setFocusedField }) => (
  <div style={{ flex: 1, display: "flex", padding: "16px 18px", gap: 14, overflow: "auto" }}>
    <div style={{ flex: 1 }}>
      <div style={{ marginBottom: 14 }}>
        <div style={{ color: C.fg, fontSize: 16, fontWeight: 700, marginBottom: 4 }}>Image generator</div>
        <div style={{ color: C.dim, fontSize: 11 }}>Powers <span style={{ color: C.accentBright }}>image_generate</span>, <span style={{ color: C.accentBright }}>image_edit</span>. Writes to <span style={{ color: C.accentBright }}>[talos.image]</span>.</div>
      </div>

      <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
        {IMAGE_PROVIDERS.map(p => (
          <Card key={p.id} {...p} selected={selected === p.id} onClick={() => onSelect(p.id)} />
        ))}
      </div>
    </div>

    <div style={{ width: 380, padding: "0 0 0 14px", borderLeft: `1px solid ${C.muted}` }}>
      {selected && selected !== "none" && (() => {
        const p = IMAGE_PROVIDERS.find(x => x.id === selected);
        return (
          <>
            <div style={{ display: "flex", alignItems: "flex-start", gap: 10, marginBottom: 14 }}>
              <div style={{ width: 42, height: 42, background: p.color, color: C.bg, display: "flex", alignItems: "center", justifyContent: "center", fontWeight: 700, fontSize: 12, letterSpacing: 1 }}>{p.glyph}</div>
              <div>
                <div style={{ color: C.white, fontSize: 14, fontWeight: 700 }}>{p.name}</div>
                <div style={{ color: C.dim, fontSize: 11 }}>{p.sub}</div>
              </div>
            </div>

            <div style={{ color: C.accentDim, fontSize: 9, fontWeight: 700, letterSpacing: 3, marginBottom: 8 }}>CONFIG</div>
            {(selected === "openai-custom" || selected === "a1111") && (
              <Field label="Base URL" value={values.base_url} onChange={v => setValue("base_url", v)} placeholder={selected === "a1111" ? "http://dgx-spark:7860" : "https://..."} required focused={focusedField === "base_url"} onFocus={() => setFocusedField("base_url")} />
            )}
            <Field label="API Key" value={values.api_key} onChange={v => setValue("api_key", v)} placeholder="..." secret required={selected !== "a1111"} focused={focusedField === "api_key"} onFocus={() => setFocusedField("api_key")} />
            <Field label="Model" value={values.model_id} onChange={v => setValue("model_id", v)} placeholder={p.sub} required focused={focusedField === "model_id"} onFocus={() => setFocusedField("model_id")} />
            {selected === "a1111" && (
              <Field label="Steps" value={values.steps} onChange={v => setValue("steps", v)} placeholder="1" focused={focusedField === "steps"} onFocus={() => setFocusedField("steps")} hint="⚠ Z-Image Turbo: must be 1 (multi-step returns black PNG)" />
            )}
          </>
        );
      })()}
    </div>
  </div>
);

const OrchestrationStep = ({ selected, onSelect, values, setValue, focusedField, setFocusedField }) => (
  <div style={{ flex: 1, display: "flex", flexDirection: "column", padding: "16px 18px", overflow: "auto" }}>
    <div style={{ marginBottom: 14 }}>
      <div style={{ color: C.fg, fontSize: 16, fontWeight: 700, marginBottom: 4 }}>Orchestration mode</div>
      <div style={{ color: C.dim, fontSize: 11 }}>How Zeus runs background work — heartbeat, cron, watchdog.</div>
    </div>

    <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr 1fr", gap: 8, marginBottom: 14 }}>
      {ORCH_MODES.map(m => (
        <div key={m.id} onClick={() => onSelect(m.id)} style={{
          background: selected === m.id ? C.accentFaint : C.bg2,
          border: `1px solid ${selected === m.id ? C.accent : m.recommended ? C.green : C.muted}`,
          borderLeft: `2px solid ${m.color}`,
          padding: "12px 14px", cursor: "pointer", minHeight: 130, position: "relative",
        }}>
          {selected === m.id && (
            <span style={{ position: "absolute", top: 8, right: 10, color: C.accent, fontSize: 9, fontWeight: 700, letterSpacing: 2 }}>▸ SELECTED</span>
          )}
          {m.recommended && !selected && (
            <span style={{ position: "absolute", top: 8, right: 10, color: C.green, fontSize: 8, fontWeight: 700, letterSpacing: 2 }}>★ REC</span>
          )}
          <div style={{
            width: 36, height: 22, background: m.color, color: C.bg,
            display: "flex", alignItems: "center", justifyContent: "center",
            fontWeight: 700, fontSize: 10, letterSpacing: 1, marginBottom: 8,
          }}>{m.glyph}</div>
          <div style={{ color: C.white, fontSize: 14, fontWeight: 700 }}>{m.name}</div>
          <div style={{ color: C.dim, fontSize: 10, marginTop: 2, marginBottom: 6, fontStyle: "italic" }}>{m.sub}</div>
          <div style={{ color: C.dim, fontSize: 10 }}>{m.desc}</div>
        </div>
      ))}
    </div>

    {selected !== "disabled" && (
      <>
        <div style={{ color: C.accentDim, fontSize: 9, fontWeight: 700, letterSpacing: 3, marginBottom: 8 }}>HEARTBEAT TIMING</div>
        <Field label="Interval" value={values.interval || "300"} onChange={v => setValue("interval", v)} placeholder="300" focused={focusedField === "interval"} onFocus={() => setFocusedField("interval")} hint="Seconds between heartbeat ticks (default 300 = 5 min)" />
        <Field label="Quiet Start" value={values.quiet_start || "23"} onChange={v => setValue("quiet_start", v)} placeholder="23" focused={focusedField === "quiet_start"} onFocus={() => setFocusedField("quiet_start")} hint="Hour (24h) when heartbeat goes quiet" />
        <Field label="Quiet End" value={values.quiet_end || "8"} onChange={v => setValue("quiet_end", v)} placeholder="8" focused={focusedField === "quiet_end"} onFocus={() => setFocusedField("quiet_end")} hint="Hour (24h) when heartbeat resumes" />
      </>
    )}
  </div>
);

const MemoryStep = ({ selected, onSelect, values, setValue, focusedField, setFocusedField }) => (
  <div style={{ flex: 1, display: "flex", padding: "16px 18px", gap: 14, overflow: "auto" }}>
    <div style={{ flex: 1 }}>
      <div style={{ marginBottom: 14 }}>
        <div style={{ color: C.fg, fontSize: 16, fontWeight: 700, marginBottom: 4 }}>Memory backend</div>
        <div style={{ color: C.dim, fontSize: 11 }}>Mnemosyne — semantic search over agent history. Pick embedding provider.</div>
      </div>

      <div style={{ display: "flex", flexDirection: "column", gap: 6, marginBottom: 14 }}>
        {MEMORY_PROVIDERS.map(p => (
          <Card key={p.id} {...p} selected={selected === p.id} onClick={() => onSelect(p.id)} large />
        ))}
      </div>

      <div style={{ color: C.accentDim, fontSize: 9, fontWeight: 700, letterSpacing: 3, marginBottom: 8 }}>STORAGE</div>
      <Field label="DB Path" value={values.db_path || "~/.zeus/mnemosyne.db"} onChange={v => setValue("db_path", v)} focused={focusedField === "db_path"} onFocus={() => setFocusedField("db_path")} />
      {selected !== "none" && (
        <Field label="Embedding Model" value={values.model || (selected === "ollama" ? "nomic-embed-text" : "text-embedding-3-small")} onChange={v => setValue("model", v)} focused={focusedField === "model"} onFocus={() => setFocusedField("model")} />
      )}
    </div>

    <div style={{ width: 280, padding: "0 0 0 14px", borderLeft: `1px solid ${C.muted}` }}>
      <div style={{ color: C.accentDim, fontSize: 9, fontWeight: 700, letterSpacing: 3, marginBottom: 8 }}>DISK PROJECTION</div>
      <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
        {[
          ["1K facts", "~12 MB"],
          ["10K facts", "~120 MB"],
          ["100K facts", "~1.2 GB"],
          ["1M facts", "~12 GB"],
        ].map(([k, v]) => (
          <div key={k} style={{ display: "flex", justifyContent: "space-between", padding: "4px 0", borderBottom: `1px solid ${C.muted}`, fontSize: 10 }}>
            <span style={{ color: C.dim }}>{k}</span>
            <span style={{ color: C.accent, fontWeight: 700 }}>{v}</span>
          </div>
        ))}
      </div>

      <div style={{ marginTop: 14, padding: "10px 12px", background: C.bg2, border: `1px solid ${C.cyan}`, borderLeft: `2px solid ${C.cyan}` }}>
        <div style={{ color: C.cyan, fontSize: 9, fontWeight: 700, letterSpacing: 3, marginBottom: 4 }}>● OLLAMA DETECTED</div>
        <div style={{ color: C.fg, fontSize: 10, lineHeight: 1.5 }}>Found local Ollama at <span style={{ color: C.accentBright }}>localhost:11434</span> with <span style={{ color: C.fg, fontWeight: 700 }}>nomic-embed-text</span> available. Recommended for free local embeddings.</div>
      </div>
    </div>
  </div>
);

const SkillsStep = ({ installed, onToggle }) => {
  const [filter, setFilter] = useState("");
  const [activeCategory, setActiveCategory] = useState("All");
  const categories = ["All", ...Object.keys(SKILLS)];
  const flatSkills = Object.entries(SKILLS).flatMap(([cat, skills]) => skills.map(s => ({ ...s, category: cat })));
  const visible = flatSkills.filter(s =>
    (activeCategory === "All" || s.category === activeCategory) &&
    (!filter || s.name.toLowerCase().includes(filter.toLowerCase()))
  );

  return (
    <div style={{ flex: 1, display: "flex", flexDirection: "column", padding: "16px 18px", overflow: "hidden" }}>
      <div style={{ marginBottom: 14, display: "flex", alignItems: "center", gap: 12 }}>
        <div style={{ flex: 1 }}>
          <div style={{ color: C.fg, fontSize: 16, fontWeight: 700, marginBottom: 4 }}>Install starter skills</div>
          <div style={{ color: C.dim, fontSize: 11 }}>SKILL.md plugins from the registry. Each grants a set of tools.</div>
        </div>
        <div style={{ display: "flex", alignItems: "center", border: `1px solid ${C.muted}`, padding: "0 10px" }}>
          <span style={{ color: C.dim, fontSize: 11 }}>/</span>
          <input
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
            placeholder="filter..."
            style={{
              background: "transparent", border: "none", color: C.fg,
              fontFamily: "inherit", fontSize: 11, padding: "6px 8px",
              outline: "none", width: 140,
            }}
          />
        </div>
      </div>

      {/* Category tabs */}
      <div style={{ display: "flex", gap: 4, marginBottom: 12, borderBottom: `1px solid ${C.muted}`, paddingBottom: 6 }}>
        {categories.map(cat => (
          <div key={cat} onClick={() => setActiveCategory(cat)} style={{
            padding: "4px 10px", cursor: "pointer", fontSize: 10,
            background: activeCategory === cat ? C.accentFaint : "transparent",
            border: `1px solid ${activeCategory === cat ? C.accent : "transparent"}`,
            color: activeCategory === cat ? C.accent : C.dim,
            fontWeight: activeCategory === cat ? 700 : 400,
            letterSpacing: 1,
          }}>
            {cat.toUpperCase()}
            {cat !== "All" && <span style={{ marginLeft: 6, color: C.muted, fontSize: 9 }}>({SKILLS[cat].length})</span>}
          </div>
        ))}
        <span style={{ flex: 1 }} />
        <span style={{ color: C.dim, fontSize: 9 }}>
          <span style={{ color: C.accent, fontWeight: 700 }}>{installed.size}</span> selected · <span style={{ color: C.fg }}>{flatSkills.length}</span> available
        </span>
      </div>

      <div style={{ flex: 1, overflowY: "auto", display: "grid", gridTemplateColumns: "1fr 1fr", gap: 6 }}>
        {visible.map(s => (
          <div key={s.id} onClick={() => onToggle(s.id)} style={{
            display: "flex", alignItems: "center", gap: 10, padding: "8px 12px",
            border: `1px solid ${installed.has(s.id) ? C.accent : C.muted}`,
            borderLeft: `2px solid ${C.accentDim}`,
            background: installed.has(s.id) ? C.accentFaint : C.bg2,
            cursor: "pointer",
          }}>
            <span style={{
              width: 14, height: 14, display: "inline-flex", alignItems: "center", justifyContent: "center",
              background: installed.has(s.id) ? C.accent : "transparent",
              color: installed.has(s.id) ? C.bg : C.muted,
              border: `1px solid ${installed.has(s.id) ? C.accent : C.muted}`,
              fontSize: 9, fontWeight: 700,
            }}>{installed.has(s.id) ? "✓" : ""}</span>
            <div style={{ flex: 1 }}>
              <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
                <span style={{ color: C.fg, fontSize: 11, fontWeight: 600 }}>{s.name}</span>
                {s.recommended && <span style={{ color: C.green, fontSize: 7, fontWeight: 700, letterSpacing: 2 }}>★ REC</span>}
              </div>
              <div style={{ color: C.dim, fontSize: 10, marginTop: 1 }}>{s.desc}</div>
            </div>
            <span style={{ color: C.muted, fontSize: 8, fontWeight: 700, letterSpacing: 1 }}>{s.category.toUpperCase()}</span>
          </div>
        ))}
      </div>
    </div>
  );
};

const CompleteStep = ({ summary }) => {
  const [testing, setTesting] = useState(false);
  const [tested, setTested] = useState(false);

  const handleTestAll = () => {
    setTesting(true);
    setTimeout(() => { setTesting(false); setTested(true); }, 2200);
  };

  return (
    <div style={{ flex: 1, padding: "16px 18px", overflow: "auto", display: "flex", gap: 14 }}>
      <div style={{ flex: 1 }}>
        <div style={{ marginBottom: 14, display: "flex", alignItems: "center", gap: 14 }}>
          <ZeusFace state={testing ? "working" : tested ? "success" : "ready"} speed={testing ? 200 : 450} label={testing ? "running checks" : tested ? "all systems go" : "ready to wake"} />
          <div style={{ width: 1, alignSelf: "stretch", background: C.muted }} />
          <div>
            <div style={{ color: C.fg, fontSize: 18, fontWeight: 700, marginBottom: 4 }}>✓ Configuration complete</div>
            <div style={{ color: C.dim, fontSize: 11 }}>Review your setup before launch. All settings persist to <span style={{ color: C.accentBright }}>~/.zeus/config.toml</span>.</div>
          </div>
        </div>

        <div style={{ display: "flex", flexDirection: "column", gap: 4, marginBottom: 14 }}>
          {summary.map((s, i) => (
            <div key={i} style={{
              display: "flex", alignItems: "center", gap: 10, padding: "8px 12px",
              border: `1px solid ${s.status === "configured" ? C.muted : s.status === "error" ? C.red : C.muted}`,
              borderLeft: `2px solid ${s.status === "configured" ? C.green : s.status === "skipped" ? C.dim : C.red}`,
              background: C.bg2,
            }}>
              <span style={{
                width: 12, height: 12, borderRadius: 6,
                background: s.status === "configured" ? C.green : s.status === "skipped" ? C.muted : C.red,
                boxShadow: s.status === "configured" ? `0 0 6px ${C.green}` : "none",
              }} />
              <span style={{ color: C.fg, fontSize: 11, fontWeight: 600, flex: 1 }}>{s.name}</span>
              <span style={{ color: C.dim, fontSize: 10 }}>{s.value}</span>
              <span style={{ color: s.status === "configured" ? C.green : s.status === "skipped" ? C.muted : C.red, fontSize: 9, fontWeight: 700, letterSpacing: 2, minWidth: 80, textAlign: "right" }}>
                {s.status === "configured" ? "✓ READY" : s.status === "skipped" ? "⏭ SKIPPED" : "✕ ERROR"}
              </span>
            </div>
          ))}
        </div>

        <div style={{ display: "flex", gap: 8 }}>
          <button onClick={handleTestAll} disabled={testing} style={{
            background: tested ? C.greenDim : C.accentFaint,
            border: `1px solid ${tested ? C.green : C.accent}`,
            color: tested ? C.green : C.accent,
            padding: "6px 14px", fontSize: 10, fontWeight: 700, letterSpacing: 2,
            fontFamily: "inherit", cursor: "pointer",
          }}>
            {testing ? "▸ TESTING ALL BACKENDS..." : tested ? "✓ ALL BACKENDS PASSED" : "▸ TEST ALL BACKENDS"}
          </button>
          <button style={{
            background: C.accent, color: C.bg,
            border: "none", padding: "6px 18px",
            fontSize: 10, fontWeight: 700, letterSpacing: 3,
            fontFamily: "inherit", cursor: "pointer",
          }}>▸ AWAKEN ZEUS</button>
        </div>
      </div>

      <div style={{ width: 320, padding: "0 0 0 14px", borderLeft: `1px solid ${C.muted}` }}>
        <div style={{ color: C.accentDim, fontSize: 9, fontWeight: 700, letterSpacing: 3, marginBottom: 8 }}>NEXT STEPS</div>
        <div style={{ display: "flex", flexDirection: "column", gap: 8, fontSize: 10, color: C.fg, lineHeight: 1.6 }}>
          <div>
            <span style={{ color: C.accentBright }}>$ zeus start</span>
            <div style={{ color: C.dim, marginLeft: 12 }}>Launches gateway + agent loop</div>
          </div>
          <div>
            <span style={{ color: C.accentBright }}>$ zeus chat</span>
            <div style={{ color: C.dim, marginLeft: 12 }}>Interactive chat with your agent</div>
          </div>
          <div>
            <span style={{ color: C.accentBright }}>$ zeus pantheon</span>
            <div style={{ color: C.dim, marginLeft: 12 }}>Multi-agent coordination chat</div>
          </div>
          <div>
            <span style={{ color: C.accentBright }}>$ zeus onboard --resume</span>
            <div style={{ color: C.dim, marginLeft: 12 }}>Re-run wizard for skipped sections</div>
          </div>
        </div>

        <div style={{ marginTop: 18, padding: "8px 12px", background: C.bg2, border: `1px solid ${C.muted}`, fontSize: 9, color: C.dim }}>
          <div style={{ color: C.accentDim, fontWeight: 700, letterSpacing: 2, marginBottom: 4 }}>SUMMARY SAVED</div>
          <span style={{ color: C.accentBright }}>~/.zeus/onboarding-summary.md</span>
          <div style={{ marginTop: 2 }}>Diff against future runs.</div>
        </div>
      </div>
    </div>
  );
};

/* ═══════════════════════════════════════════════════ */
/* MAIN COMPONENT                                       */
/* ═══════════════════════════════════════════════════ */

export default function ZeusOnboarding() {
  const [stepIdx, setStepIdx] = useState(0);
  const [completed, setCompleted] = useState(new Set());
  const [skipped, setSkipped] = useState(new Set());
  const [showHelp, setShowHelp] = useState(false);

  // Per-step state
  const [setupMode, setSetupMode] = useState("full");
  const [provider, setProvider] = useState("anthropic");
  const [providerFocused, setProviderFocused] = useState(null);
  const [authMode, setAuthMode] = useState("key");
  const [authValues, setAuthValues] = useState({});
  const [authFocusedField, setAuthFocusedField] = useState(null);
  const [authTestStatus, setAuthTestStatus] = useState(null);
  const [model, setModel] = useState(null);
  const [fallbackChain, setFallbackChain] = useState([]);
  const [channelsToggled, setChannelsToggled] = useState(new Set(["discord", "telegram"]));
  const [chanFocused, setChanFocused] = useState(null);
  const [chanConfigValues, setChanConfigValues] = useState({});
  const [chanFocusedField, setChanFocusedField] = useState(null);
  const [chanTestStatuses, setChanTestStatuses] = useState({});
  const [gatewayValues, setGatewayValues] = useState({});
  const [gatewayFocused, setGatewayFocused] = useState(null);
  const [gatewayServiceMode, setGatewayServiceMode] = useState("launchd");
  const [persona, setPersona] = useState("coordinator");
  const [agentValues, setAgentValues] = useState({});
  const [agentFocused, setAgentFocused] = useState(null);
  const [workspaceValues, setWorkspaceValues] = useState({});
  const [workspaceFocused, setWorkspaceFocused] = useState(null);
  const [security, setSecurity] = useState("standard");
  const [featuresToggled, setFeaturesToggled] = useState(new Set(["nous", "mnemosyne", "hermes"]));
  const [voiceProvider, setVoiceProvider] = useState("elevenlabs");
  const [voiceValues, setVoiceValues] = useState({});
  const [voiceFocused, setVoiceFocused] = useState(null);
  const [imageProvider, setImageProvider] = useState("openai");
  const [imageValues, setImageValues] = useState({});
  const [imageFocused, setImageFocused] = useState(null);
  const [orchMode, setOrchMode] = useState("all-on");
  const [orchValues, setOrchValues] = useState({});
  const [orchFocused, setOrchFocused] = useState(null);
  const [memProvider, setMemProvider] = useState("ollama");
  const [memValues, setMemValues] = useState({});
  const [memFocused, setMemFocused] = useState(null);
  const [skillsInstalled, setSkillsInstalled] = useState(new Set(["calendar-pro", "git-flow", "ci-watch", "secret-scan", "deep-synth"]));

  const advance = () => {
    setCompleted(prev => new Set([...prev, stepIdx]));
    setStepIdx(Math.min(stepIdx + 1, STEPS.length - 1));
  };
  const skip = () => {
    setSkipped(prev => new Set([...prev, stepIdx]));
    setStepIdx(Math.min(stepIdx + 1, STEPS.length - 1));
  };
  const back = () => setStepIdx(Math.max(0, stepIdx - 1));

  const stepIdEarly = STEPS[stepIdx].id;
  const isOptionalEarly = STEPS[stepIdx].optional;
  const canContinue = stepIdx < STEPS.length - 1;

  // Compute validation early — reads state directly
  const computedValidation = (() => {
    if (stepIdEarly === "auth") return authValues.api_key ? "valid" : "incomplete";
    if (stepIdEarly === "model") return model ? "valid" : "incomplete";
    return "valid";
  })();

  // ═══ KEYBOARD HANDLER ═══
  useEffect(() => {
    const handler = (e) => {
      const tag = e.target?.tagName;
      const isInput = tag === "INPUT" || tag === "TEXTAREA";
      const universalKeys = ["Escape", "Tab"];

      if (isInput && !universalKeys.includes(e.key)) {
        if (e.key === "Enter" && stepIdEarly !== "skills") {
          if (canContinue && computedValidation !== "incomplete") {
            e.preventDefault();
            advance();
          }
        }
        return;
      }

      if (e.key === "Enter") {
        e.preventDefault();
        if (computedValidation !== "incomplete") advance();
      } else if (e.key === "Escape") {
        e.preventDefault();
        if (stepIdx > 0) back();
      } else if (e.key === "ArrowDown" || e.key === "ArrowRight") {
        e.preventDefault();
        handleNavigate(1);
      } else if (e.key === "ArrowUp" || e.key === "ArrowLeft") {
        e.preventDefault();
        handleNavigate(-1);
      } else if (e.key === " " || e.key === "Spacebar") {
        if (stepIdEarly === "channels" || stepIdEarly === "features" || stepIdEarly === "skills" || stepIdEarly === "fallback") {
          e.preventDefault();
          handleToggleFocused();
        }
      } else if (e.key === "s" || e.key === "S") {
        if (isOptionalEarly) { e.preventDefault(); skip(); }
      } else if (e.key === "t" || e.key === "T") {
        if (stepIdEarly === "auth") { e.preventDefault(); handleAuthTest(); }
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  });

  // Per-step navigation — moves focus or selection
  const handleNavigate = (dir) => {
    switch (stepIdEarly) {
      case "mode": {
        const ids = ["quickstart", "full", "custom"];
        const i = ids.indexOf(setupMode);
        setSetupMode(ids[(i + dir + ids.length) % ids.length]);
        break;
      }
      case "provider": {
        const i = LLM_PROVIDERS.findIndex(p => p.id === provider);
        setProvider(LLM_PROVIDERS[(i + dir + LLM_PROVIDERS.length) % LLM_PROVIDERS.length].id);
        break;
      }
      case "model": {
        const models = {
          anthropic: ["claude-opus-4-7", "claude-sonnet-4-6", "claude-haiku-4-5"],
          openai: ["gpt-4o", "gpt-4o-mini", "o1-pro"],
          minimax: ["abab-7-chat", "abab-6.5s-chat"],
        }[provider] || ["claude-opus-4-7", "claude-sonnet-4-6", "claude-haiku-4-5"];
        const i = models.indexOf(model);
        const next = i === -1 ? 0 : (i + dir + models.length) % models.length;
        setModel(models[next]);
        break;
      }
      case "security": {
        const ids = SECURITY_LEVELS.map(s => s.id);
        const i = ids.indexOf(security);
        setSecurity(ids[(i + dir + ids.length) % ids.length]);
        break;
      }
      case "voice": {
        const ids = VOICE_PROVIDERS.map(p => p.id);
        const i = ids.indexOf(voiceProvider);
        setVoiceProvider(ids[(i + dir + ids.length) % ids.length]);
        break;
      }
      case "images": {
        const ids = IMAGE_PROVIDERS.map(p => p.id);
        const i = ids.indexOf(imageProvider);
        setImageProvider(ids[(i + dir + ids.length) % ids.length]);
        break;
      }
      case "orchestration": {
        const ids = ORCH_MODES.map(m => m.id);
        const i = ids.indexOf(orchMode);
        setOrchMode(ids[(i + dir + ids.length) % ids.length]);
        break;
      }
      case "memory": {
        const ids = MEMORY_PROVIDERS.map(m => m.id);
        const i = ids.indexOf(memProvider);
        setMemProvider(ids[(i + dir + ids.length) % ids.length]);
        break;
      }
      case "agent": {
        const ids = PERSONAS.map(p => p.id);
        const i = ids.indexOf(persona);
        setPersona(ids[(i + dir + ids.length) % ids.length]);
        break;
      }
      case "channels": {
        const ids = CHANNELS.map(c => c.id);
        const cur = chanFocused || ids[0];
        const i = ids.indexOf(cur);
        setChanFocused(ids[(i + dir + ids.length) % ids.length]);
        break;
      }
    }
  };

  const handleToggleFocused = () => {
    switch (stepIdEarly) {
      case "channels": {
        const id = chanFocused || CHANNELS[0].id;
        const n = new Set(channelsToggled);
        if (n.has(id)) n.delete(id); else n.add(id);
        setChannelsToggled(n);
        break;
      }
    }
  };

  const handleAuthTest = () => {
    setAuthTestStatus("testing");
    setTimeout(() => setAuthTestStatus(authValues.api_key && authValues.api_key.length > 10 ? "success" : "error"), 1500);
  };

  const handleChanTest = (id) => {
    setChanTestStatuses(prev => ({ ...prev, [id]: "testing" }));
    setTimeout(() => setChanTestStatuses(prev => ({ ...prev, [id]: "success" })), 1400);
  };

  const stepId = stepIdEarly;
  const isOptional = isOptionalEarly;

  // Map step id to renderer
  let content;
  let extraKeys = [];

  switch (stepId) {
    case "welcome":
      content = <WelcomeStep existing={true} onContinue={advance} />;
      break;
    case "mode":
      content = <ModeStep selected={setupMode} onSelect={setSetupMode} />;
      break;
    case "provider":
      content = <ProviderStep selected={provider} focused={providerFocused} onSelect={setProvider} onFocus={setProviderFocused} />;
      break;
    case "auth":
      content = <AuthStep provider={provider} mode={authMode} setMode={setAuthMode}
        values={authValues}
        setValue={(k, v) => setAuthValues(prev => ({ ...prev, [k]: v }))}
        focusedField={authFocusedField} setFocusedField={setAuthFocusedField}
        testStatus={authTestStatus} onTest={handleAuthTest} />;
      extraKeys = [{ k: "t", v: "Test" }];
      break;
    case "model":
      content = <ModelStep provider={provider} selectedModel={model} onSelect={setModel} />;
      break;
    case "fallback":
      content = <FallbackStep chain={fallbackChain} setChain={setFallbackChain} primary={provider} />;
      extraKeys = [{ k: "[", v: "" }, { k: "]", v: "Reorder" }];
      break;
    case "channels":
      content = <ChannelsStep toggled={channelsToggled} focused={chanFocused}
        onToggle={(id) => {
          const n = new Set(channelsToggled);
          if (n.has(id)) n.delete(id); else n.add(id);
          setChannelsToggled(n);
        }} setFocused={setChanFocused} />;
      extraKeys = [{ k: "Sp", v: "Toggle" }];
      break;
    case "chanconfig":
      content = <ChanConfigStep toggled={channelsToggled}
        configValues={chanConfigValues}
        setConfigValue={(k, v) => setChanConfigValues(prev => ({ ...prev, [k]: v }))}
        focusedField={chanFocusedField} setFocusedField={setChanFocusedField}
        testStatuses={chanTestStatuses} onTest={handleChanTest} />;
      break;
    case "gateway":
      content = <GatewayStep values={gatewayValues}
        setValue={(k, v) => setGatewayValues(prev => ({ ...prev, [k]: v }))}
        focusedField={gatewayFocused} setFocusedField={setGatewayFocused}
        serviceMode={gatewayServiceMode} setServiceMode={setGatewayServiceMode} />;
      break;
    case "agent":
      content = <AgentStep persona={persona} setPersona={setPersona}
        values={agentValues}
        setValue={(k, v) => setAgentValues(prev => ({ ...prev, [k]: v }))}
        focusedField={agentFocused} setFocusedField={setAgentFocused}
        hostname="zeus.local" />;
      break;
    case "workspace":
      content = <WorkspaceStep values={workspaceValues}
        setValue={(k, v) => setWorkspaceValues(prev => ({ ...prev, [k]: v }))}
        focusedField={workspaceFocused} setFocusedField={setWorkspaceFocused}
        existingDetected={true} />;
      break;
    case "security":
      content = <SecurityStep selected={security} onSelect={setSecurity} />;
      break;
    case "features":
      content = <FeaturesStep toggled={featuresToggled}
        onToggle={(id) => {
          const n = new Set(featuresToggled);
          if (n.has(id)) n.delete(id); else n.add(id);
          setFeaturesToggled(n);
        }} platform="macOS" />;
      break;
    case "voice":
      content = <VoiceStep selected={voiceProvider} onSelect={setVoiceProvider}
        values={voiceValues}
        setValue={(k, v) => setVoiceValues(prev => ({ ...prev, [k]: v }))}
        focusedField={voiceFocused} setFocusedField={setVoiceFocused} />;
      break;
    case "images":
      content = <ImagesStep selected={imageProvider} onSelect={setImageProvider}
        values={imageValues}
        setValue={(k, v) => setImageValues(prev => ({ ...prev, [k]: v }))}
        focusedField={imageFocused} setFocusedField={setImageFocused} />;
      break;
    case "orchestration":
      content = <OrchestrationStep selected={orchMode} onSelect={setOrchMode}
        values={orchValues}
        setValue={(k, v) => setOrchValues(prev => ({ ...prev, [k]: v }))}
        focusedField={orchFocused} setFocusedField={setOrchFocused} />;
      break;
    case "memory":
      content = <MemoryStep selected={memProvider} onSelect={setMemProvider}
        values={memValues}
        setValue={(k, v) => setMemValues(prev => ({ ...prev, [k]: v }))}
        focusedField={memFocused} setFocusedField={setMemFocused} />;
      break;
    case "skills":
      content = <SkillsStep installed={skillsInstalled}
        onToggle={(id) => {
          const n = new Set(skillsInstalled);
          if (n.has(id)) n.delete(id); else n.add(id);
          setSkillsInstalled(n);
        }} />;
      extraKeys = [{ k: "/", v: "Filter" }];
      break;
    case "complete":
      content = <CompleteStep summary={[
        { name: "LLM Provider", value: `${provider}/${model || "claude-opus-4-7"}`, status: "configured" },
        { name: "Authentication", value: authMode + " · ✓ tested", status: "configured" },
        { name: "Backup LLMs", value: `${fallbackChain.length} configured`, status: fallbackChain.length > 0 ? "configured" : "skipped" },
        { name: "Channels", value: `${channelsToggled.size} bridged`, status: channelsToggled.size > 0 ? "configured" : "skipped" },
        { name: "Gateway", value: `${gatewayValues.host || "127.0.0.1"}:${gatewayValues.port || "8080"}`, status: "configured" },
        { name: "Agent Persona", value: `${persona} (${agentValues.name || "Zeus100"})`, status: "configured" },
        { name: "Workspace", value: workspaceValues.workspace || "~/.zeus/workspace", status: "configured" },
        { name: "Security", value: `aegis level: ${security}`, status: "configured" },
        { name: "Features", value: `${featuresToggled.size} subsystems on`, status: "configured" },
        { name: "Voice (TTS)", value: voiceProvider, status: voiceProvider === "none" ? "skipped" : "configured" },
        { name: "Image Generator", value: imageProvider, status: imageProvider === "none" ? "skipped" : "configured" },
        { name: "Orchestration", value: orchMode, status: "configured" },
        { name: "Memory", value: `embeddings: ${memProvider}`, status: "configured" },
        { name: "Skills", value: `${skillsInstalled.size} installed`, status: skillsInstalled.size > 0 ? "configured" : "skipped" },
      ]} />;
      break;
    default:
      content = <div style={{ padding: 40, color: C.dim }}>Step: {stepId}</div>;
  }

  // Validation state
  let validationState = "ready";
  if (stepId === "auth") validationState = authValues.api_key ? "valid" : "incomplete";
  else if (stepId === "model") validationState = model ? "valid" : "incomplete";
  else validationState = "valid";

  // ── Reactive ZeusFace state (mirrors production compute_face_state priority) ──
  let topFaceState = "ready";
  if (stepId === "auth" && authTestStatus === "testing") topFaceState = "thinking";
  else if (stepId === "auth" && authTestStatus === "error") topFaceState = "error";
  else if (stepId === "auth" && authTestStatus === "success") topFaceState = "success";
  else if (stepId === "complete") topFaceState = "success";
  else if (validationState === "incomplete") topFaceState = "listening";
  else if (stepIdx === 0) topFaceState = "ready";
  else topFaceState = "working";

  return (
    <>
      <style>{`
        @import url('https://fonts.googleapis.com/css2?family=JetBrains+Mono:wght@300;400;500;700&display=swap');
        *, *::before, *::after { margin:0; padding:0; box-sizing:border-box; }
        body { background: ${C.bg}; overflow: hidden; }
        ::selection { background: ${C.accentDim}; color: ${C.white}; }
        input::placeholder { color: ${C.muted}; }
        ::-webkit-scrollbar { width: 4px; height: 4px; }
        ::-webkit-scrollbar-track { background: transparent; }
        ::-webkit-scrollbar-thumb { background: ${C.muted}; border-radius: 2px; }
      `}</style>

      <div style={{
        fontFamily: "'JetBrains Mono', monospace", fontSize: 12, lineHeight: 1.5,
        color: C.fg, background: C.bg, height: "100vh",
        display: "flex", flexDirection: "column", overflow: "hidden",
      }}>
        <TopBar stepIdx={stepIdx} faceState={topFaceState} />
        <StepIndicator current={stepIdx} completed={completed} skipped={skipped} />

        <div style={{ flex: 1, display: "flex", overflow: "hidden" }}>
          <div style={{ flex: 1, display: "flex", flexDirection: "column", overflow: "hidden" }}>
            {content}
          </div>
        </div>

        <StatusBar
          canBack={stepIdx > 0}
          canSkip={isOptional}
          canContinue={stepIdx < STEPS.length - 1}
          currentStep={stepIdx}
          totalSteps={STEPS.length}
          onBack={back}
          onSkip={skip}
          onContinue={advance}
          validationState={validationState}
          extraKeys={extraKeys}
        />
      </div>
    </>
  );
}
