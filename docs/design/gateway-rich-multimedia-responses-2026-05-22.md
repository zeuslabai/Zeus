# Gateway-Mediated Rich Multimedia Responses

**Author:** Zeus112
**Date:** 2026-05-22
**Sprint:** #85
**Status:** Design proposal (first draft, working set)

---

## 0. TL;DR

Today, Zeus channel adapters render plain text uniformly. Rich content (images, embeds, blocks, tables, voice notes, file attachments) is either impossible end-to-end or implemented ad-hoc per channel. This proposal defines a **gateway-centric universal rich-response pipeline**: the gateway produces a single channel-agnostic intermediate format (`RichResponse`), and each channel adapter becomes a thin protocol translator that renders or degrades to its native capability set.

**Load-bearing architectural directive (merakizzz, verbatim):**
> "Use the gateway for this. Most of our features should just go around the gateway. Because it's so much more complicated if we do separate features for each one of the different channels and terminals."

This doc:
1. Walks current substrate seams.
2. Proposes the `RichResponse` intermediate type + capability negotiation.
3. Specifies a per-channel renderer matrix with graceful-degradation fallback.
4. Defines an image-source decision tree (web-search vs Z-Image vs local-MCP).
5. Defines a "when-to-attach-image" trigger model.
6. Lists migration cuts (estimated SLOC + risk).

---

## 1. Current Substrate (Verified)

### 1.1 Channel adapter trait
- `crates/zeus-channels/src/lib.rs:538` — `pub trait ChannelAdapter`
- `lib.rs:561` — `async fn send(&self, to: &ChannelSource, content: &str) -> Result<()>` — **text-only**, the universal seam every channel implements.
- `lib.rs:611` — `async fn send_file(...)` — exists on the trait but defaults to a "not supported" error. Implemented by Discord, Slack, Telegram inconsistently.

### 1.2 Canonical message types
- `lib.rs:270` — `pub struct ChannelAttachment` (url-or-data, mime, filename).
- `lib.rs:282` — constructors: `from_url`, `from_data`, `with_filename`.
- `lib.rs:329` — `pub struct ChannelMessage` carries `attachments: Vec<ChannelAttachment>` (line 339).

**Key finding:** Zeus already has a canonical attachment type **for inbound traffic** (parsing user uploads). The gap is on the **outbound** path — `ChannelAdapter::send` only takes `&str`. There is no symmetric outbound `RichResponse` type.

### 1.3 Per-channel rich-content surface
| Channel | Rich primitive | Substrate citation |
|---|---|---|
| Discord | Embeds (title/desc/fields/color/footer/thumb/image/author), file attachments via `CreateAttachment` | `crates/zeus-channels/src/discord.rs:55` (DiscordEmbed), `discord.rs:137-169` (builder), `discord.rs:710` (`send_embed`), `discord.rs:471` (inbound attachments) |
| Slack | Block Kit (Section/Header/Divider/Context/Actions), file upload via `files.uploadV2` | `crates/zeus-channels/src/slack.rs:27` (Block enum), `slack.rs:125-177` (builder), `slack.rs:827` (`send_rich_message`), `slack.rs:864` (`upload_file`) |
| Telegram | sendPhoto / sendDocument / sendMediaGroup (TBD — not surfaced this walk) | TBD |
| WebUI | HTML/Markdown native, `<img>` inline | TBD (apps/ZeusWeb) |
| CLI | ANSI escapes only, no inline images | TBD |
| Voice | Audio synthesis only | TBD |

### 1.4 Gateway dispatch
- `src/gateway.rs` — 3277 LOC. Dispatch sites send text via `ChannelAdapter::send(channel_source, content)` exclusively.
- No current code path constructs a structured outbound response and asks the adapter to render it.

### 1.5 Image sources already in tree
- `src/image_provider.rs` — abstractions for `openai`, `openai_compatible`, `automatic1111`, `comfyui`, `fooocus`.
- MCP tools registered: `image_generate`, `fooocus_generate`, `analyze_image`.
- Z-Image runs on DGX Spark `:7860` per merakizzz (external endpoint, not yet wrapped as MCP tool — TBD verify).
- **Gap:** no web-image-search tool. Proposal §5 adds one.

---

## 2. Proposed Architecture

### 2.1 Intermediate format: `RichResponse`

```rust
// New type, lives in crates/zeus-channels/src/rich.rs
pub struct RichResponse {
    /// Plain-text fallback. ALWAYS populated. Channels that
    /// can't render anything richer get this verbatim.
    pub text: String,

    /// Ordered list of content blocks. Renderers walk this list
    /// and emit channel-native equivalents.
    pub blocks: Vec<ContentBlock>,

    /// Out-of-band attachments (already-resolved files/URLs).
    pub attachments: Vec<ChannelAttachment>,

    /// Hints for the renderer (e.g. "prefer-embed", "no-preview").
    pub hints: ResponseHints,
}

pub enum ContentBlock {
    Text(String),              // markdown allowed
    Heading { level: u8, text: String },
    Image { source: ImageSource, alt: String, caption: Option<String> },
    Code { lang: Option<String>, body: String },
    Table { headers: Vec<String>, rows: Vec<Vec<String>> },
    Quote(String),
    Divider,
    Actions(Vec<ActionButton>), // Discord/Slack interactive
    Embed(EmbedCard),           // title+desc+fields+thumb, channel-agnostic
}

pub enum ImageSource {
    Url(String),
    Data { bytes: Vec<u8>, mime: String },
    Generated { prompt: String, provider_hint: Option<String> }, // lazy resolution
    Search { query: String },  // lazy: gateway resolves before dispatch
}
```

### 2.2 Renderer trait extension

Extend `ChannelAdapter` (additively — preserve `send` for text-only callers):

```rust
pub trait ChannelAdapter: Send + Sync {
    // existing
    async fn send(&self, to: &ChannelSource, content: &str) -> Result<()>;
    async fn send_file(&self, ...) -> Result<()>;

    // new — default impl falls back to text rendering
    async fn send_rich(&self, to: &ChannelSource, resp: &RichResponse) -> Result<()> {
        // Default: degrade to text via RichResponse::to_text()
        self.send(to, &resp.to_text()).await
    }

    // new — capability advertisement
    fn capabilities(&self) -> ChannelCapabilities {
        ChannelCapabilities::TEXT_ONLY
    }
}

bitflags! {
    pub struct ChannelCapabilities: u32 {
        const TEXT_ONLY     = 0;
        const MARKDOWN      = 1 << 0;
        const INLINE_IMAGE  = 1 << 1;
        const FILE_UPLOAD   = 1 << 2;
        const RICH_EMBED    = 1 << 3;
        const INTERACTIVE   = 1 << 4;
        const THREADS       = 1 << 5;
        const VOICE         = 1 << 6;
    }
}
```

### 2.3 Gateway-side flow

```
LLM → tool-call stream → ResponseBuilder
                              ↓
                      RichResponse (channel-agnostic)
                              ↓
                ┌─────────────┴─────────────┐
                ↓                           ↓
      Image-source resolver         Adapter dispatch
      (lazy → eager URLs/bytes)     adapter.send_rich(target, &resp)
                                              ↓
                                  per-channel renderer
                                              ↓
                              Discord embed / Slack blocks /
                              Telegram media / WebUI HTML / CLI text
```

**Key invariant:** the gateway never knows what channel it's talking to. The adapter is sole authority on degradation strategy.

---

## 3. Per-Channel Renderer Matrix

| Block type | Discord | Slack | Telegram | WebUI | CLI | Voice |
|---|---|---|---|---|---|---|
| Text(md) | md → embed desc | Section block | parseMode=MarkdownV2 | native | strip md | TTS |
| Heading | embed title or **bold** | Header block | *bold* | `<h2>` | ANSI bold | pause+emphasis |
| Image(url) | embed.image | image block | sendPhoto | `<img>` | "[image: alt]" | "shows: caption" |
| Image(data) | CreateAttachment | upload_file | sendPhoto(InputFile) | base64 inline | save+path | (skip) |
| Code | ```` ```lang ```` | rich_text_preformatted | ```` ``` ```` | `<pre>` | ANSI | "code block omitted" |
| Table | render as code-block ASCII | Section w/ mrkdwn | escaped md | `<table>` | column-aligned ASCII | "table with N rows" |
| Quote | embed.description quote | quote block | blockquote | `<blockquote>` | "> " prefix | "quote: ..." |
| Divider | `\n---\n` in embed | Divider block | `\n────\n` | `<hr>` | `---` | (skip) |
| Actions | message components (buttons) | actions block | inline keyboard | `<button>` | numbered list "press 1-N" | "options: ..." |
| EmbedCard | native embed | Section + accessory | sendPhoto+caption | card div | text+box | "card: title — desc" |

**Degradation contract:** every channel MUST produce *something* for every block type. The fallback chain is:
`native rich → markdown-equivalent → plain-text-with-marker → silent-skip` (only for Divider).

---

## 4. Image-Source Decision Tree

When the LLM emits an "I want to show an image" signal (see §5), the gateway picks a source:

```
                  ┌───────────────────────────┐
                  │  Image intent detected    │
                  └────────────┬──────────────┘
                               ↓
              ┌──────────────────────────────┐
              │ Is it referring to a REAL    │
              │ thing? (person, place, prod) │
              └──────┬───────────────────┬───┘
                  YES                    NO (creative)
                     ↓                    ↓
            ┌────────────────┐   ┌──────────────────┐
            │ Web image      │   │ Generative model │
            │ search         │   │ pick:            │
            │ (new MCP tool) │   │                  │
            └────────┬───────┘   │ Z-Image (DGX)    │
                     ↓           │   ↑ fast, cheap  │
            top-1 result         │ fooocus          │
            with license filter  │   ↑ quality      │
                                 │ openai_image     │
                                 │   ↑ reliable     │
                                 └────────┬─────────┘
                                          ↓
                                 LLM-judged provider
                                 pick OR config-default
                                 (capability flag)
```

**Heuristic (deterministic, fast-path):**
- contains proper noun + ("photo of"/"picture of"/"what does X look like") → **web search**
- contains "draw" / "generate" / "imagine" / "illustration of" → **generative**
- ambiguous → **LLM-judged** (single classifier call, cached)

**License filter for web search:** restrict to CC-licensed / public-domain sources by default; configurable per workspace.

---

## 5. When-to-Attach-Image Trigger Model

The hardest question: *when* should Zeus proactively show an image vs stay text-only?

**Three trigger sources, ordered by precedence:**

1. **Explicit user ask** — "show me", "what does X look like", "draw", "make an image of". Deterministic regex over the user turn. High precision, miss recall.
2. **LLM self-signal** — the model emits a `<rich:image prompt="..." />` directive in its response stream. Tool-call-like, but inline. Cheap, accurate, requires model cooperation.
3. **Post-hoc classifier** — after text generation, a lightweight pass scores `would-this-benefit-from-an-image`. Defaults OFF for chat channels (noisy), ON for explicitly multimedia surfaces.

**Default policy:** (1) always, (2) when model emits it, (3) opt-in per channel.

**Anti-spam guardrails:**
- Max 1 image per response unless user asks for a comparison.
- Suppress images on follow-up turns within a 30s window unless explicitly requested.
- Channel-level disable flag in workspace config.

---

## 6. Migration Plan (Cut Sequence)

| Cut | Scope | SLOC est | Risk | Notes |
|---|---|---|---|---|
| #85-A | Add `RichResponse` + `ContentBlock` + `to_text()` fallback in `zeus-channels::rich` | ~250 | LOW | additive type only, no callsites changed |
| #85-B | Add `send_rich` + `capabilities()` default impls to `ChannelAdapter` | ~80 | LOW | default delegates to `send`, preserves all current behavior |
| #85-C | Implement `send_rich` for Discord (embed + attachment) | ~200 | MED | leverages existing `send_embed` (`discord.rs:710`) |
| #85-D | Implement `send_rich` for Slack (blocks + upload_file) | ~200 | MED | leverages existing `send_rich_message` (`slack.rs:827`) |
| #85-E | Implement `send_rich` for Telegram | ~250 | MED | needs sendMediaGroup wrapper — TBD verify substrate |
| #85-F | Gateway: route a single LLM response through `RichResponse` builder | ~400 | HIGH | first real cross-channel test; behind feature flag |
| #85-G | Image-source resolver: lazy → eager (`ImageSource::Generated`, `::Search` resolution) | ~300 | MED | depends on web-search MCP tool (NEW, §7) |
| #85-H | LLM directive parser `<rich:image>` (§5 trigger source 2) | ~150 | LOW | regex + escape handling |
| #85-I | WebUI renderer | ~200 | LOW | mostly HTML serialization |
| #85-J | Voice channel: image → describe-aloud | ~100 | LOW | TTS-side, uses caption/alt |

**Total estimate:** ~2130 SLOC across 10 cuts. Feature-flagged behind `rich_responses=true` until #85-F lands cleanly.

---

## 7. New Dependency: Web Image Search MCP Tool

**Gap finding:** Zeus has generative image providers (§1.5) but no way to fetch an existing image of a real subject.

**Proposed tool:**
```
name: image_search
args:
  query: string
  license_filter: "cc" | "public-domain" | "any" (default: cc)
  count: u8 (default: 1, max: 5)
returns:
  results: [{url, thumbnail, attribution, license, width, height}]
```

**Backend candidates (TBD pick):**
- DuckDuckGo image search (already used for `web_search`, lightweight, no API key) — **preferred**
- Wikimedia Commons API (CC-only, high quality, low coverage)
- Bing Image Search API (requires key, broad coverage)

Defer backend pick to implementation cut. Spec interface only here.

---

## 8. Open Questions / TBDs

- **Telegram substrate walk** — not completed this pass. Cut #85-E blocked until verified.
- **WebUI renderer architecture** — apps/ZeusWeb integration TBD.
- **Voice channel** — does Zeus have an outbound voice synth path today? TBD audit.
- **Image caching / dedup** — generated images should be content-addressed and reused. Out of scope for #85, file as follow-on.
- **Cost accounting** — generative calls cost real money. Need per-workspace budget gating. Out of scope, file as follow-on.
- **Z-Image MCP wrapper** — confirm whether `:7860` is already wrapped as an MCP tool or needs adapter.
- **Streaming rich responses** — current design assumes a complete `RichResponse` at dispatch. Streaming (progressive block emission) is a future cut.

---

## 9. Why Gateway-Centric Wins (Recap)

Per merakizzz's load-bearing directive, the alternative is per-channel features. The cost differential:

| Approach | New feature cost | Channels supported | Consistency |
|---|---|---|---|
| Per-channel | O(channels × features) | manual per channel | drifts |
| Gateway-centric | O(features + channels) once | automatic via degradation | enforced by type |

With ~6 channels (Discord/Slack/Telegram/WebUI/CLI/Voice) and a typical feature touching all of them, gateway-centric is **~5× cheaper** to ship the first feature and asymptotically free thereafter.

---

## 10. Acceptance Criteria for Sprint #85 Close

1. `RichResponse` type committed to `crates/zeus-channels/src/rich.rs`.
2. `send_rich` default impl + capability flags on `ChannelAdapter`.
3. Discord + Slack renderers landed and tested with a sample response containing: text, heading, code, image (url), table, divider.
4. Gateway feature-flag wired (`rich_responses=true`).
5. At least one end-to-end demo: LLM emits a response with one image and a table → renders correctly on Discord and Slack, degrades cleanly to plain text on CLI.

Telegram/WebUI/Voice can land in follow-on sprints.

---

*End first draft. Open to coord review and follow-on refinement cuts.*
