# TUI Render-Fidelity Audit — onboarding + production

Audit base: `origin/main` `e49760f7c1b3f630be3d0844a547b8c7ed4a121f`.

Scope: recon only. No product code fixes in this branch.

References:
- Onboarding SoT: `docs/zeus-tui-onboarding.jsx`
- Production SoT: `docs/zeus-tui-production.jsx`
- Onboarding fidelity spec: `docs/tui-onboarding-fidelity-spec.md`
- Render gate used here: temporary local TestBackend dump harness, then removed. Render dumps live outside the repo under `/tmp/tui_audit/renders`.

Render coverage:
- Onboarding: steps 01–19 rendered through `zeus_tui::app::frame`.
- Production: 10 primary tabs, Advanced grid, and 13 Advanced detail views rendered through `frame`.
- Target size: ~`100x30`, per task. Notable exception: Welcome panics at `100x30`; captured at `120x40` after recording the panic.

Severity key:
- **P0**: cannot satisfy the requested render gate / broken at target size.
- **P1**: major prototype intent or layout missing.
- **P2**: visible fidelity drift that blocks 100% but not basic usability.
- **P3**: polish/copy/hint/style mismatch.

## Global findings

1. **P0 — Welcome cannot be audited at the requested 100×30 target.** Rendering onboarding step 01 at `100x30` panics with `index outside of buffer`; it only produced a dump at `120x40`. The audit target explicitly called for ~`100x30`, so this is the first fix before claiming 100% fidelity.
2. **P1 — App chrome uses 1-row terminal compression instead of the prototype's richer chrome heights.** Prototype production chrome has a 22px top bar, 26px tab bar, content, and 24px status bar; rendered output compresses TopBar/TabBar/StatusBar to one terminal row each. Same pattern exists in onboarding. Labels generally survive, but spacing, borders, badges, and visual weight are not 1:1.
3. **P1 — Repeated onboarding StepHeader copy appears on many screens.** Rendered steps often show the App StepHeader title/subtitle and then the screen repeats the same title/subtitle inside the body. Prototype screens own one header area plus content; this creates vertical drift and squeezes cards at 100×30.
4. **P2 — Color/badge/card intent is only partially represented in text dumps.** Many prototype pills/cards/badges render as plain bracket/glyph text (`[ANT]`, `★REC`, `▸ SELECTED`) without matching colored pill/card hierarchy. This is visible in rendered cell dumps even without style inspection.

## Pass 1 — Onboarding gaps

### 01. Welcome (`screens/welcome.rs` vs JSX 446–508)
- **P0:** Panics at `100x30` TestBackend; must fit/clamp before any 100% render gate can pass.
- **P1:** Missing the prototype **INITIATE** card: `▸ INITIATE`, version/LOC/tool-count meta, body blurb, 3 stats rows, footer `↵ Continue` / `N Exit` / build SHA.
- **P1:** Existing-config resume box is not visible in the fresh render path; confirm/restore the prototype conditional box for existing config.
- **P2:** ZeusFace is static; spec already calls animation a cross-cutting follow-up.

### 02. Mode (`screens/mode.rs` vs JSX 509–576)
- **P2:** Main cards are present, but rendered cards are narrower/flatter than the prototype's three-card layout with stronger selected state and metadata bands.
- **P2:** Footer/action affordances are generic App footer text rather than the prototype's mode-specific key-hint balance.
- **P3:** Selection details and subcopy wrap/truncate earlier than prototype at 100×30.

### 03. Provider (`screens/provider.rs` vs JSX 577–648)
- **P1:** Prototype is a 3-column screen: scrollable provider list, rich selected-provider detail, and right setup panel. Rendered output approximates this but squeezes detail/setup into truncated columns; several labels are partially clipped at 100×30.
- **P2:** Provider cards/badges (`FEATURED`, `● DETECTED`, flagship/pricing/key-format rows) do not visually match prototype pill/card hierarchy.
- **P2:** Right-side setup steps are less structured than prototype numbered steps and CTA panel.

### 04. Auth (`screens/auth.rs` vs JSX 649–790)
- **P1:** Prototype includes richer auth-mode cards, API key field treatment, env-var/key-format explanation, OAuth/setup-token path, and testing states. Rendered view is present but visually flatter and more compressed.
- **P2:** Secret/key format hints are not visually aligned with prototype badge/field style.
- **P3:** App footer copy competes with screen-specific auth hints.

### 05. Model (`screens/model.rs` vs JSX 791–869)
- **P1:** Prototype expects model list + selected model detail + model search/filter affordance. Rendered list/detail exists but truncates right detail labels and card bodies heavily at 100×30.
- **P2:** Recommended/default badges and detail metrics render as plain text rather than prototype badge shapes.
- **P3:** Some list card borders touch dense adjacent columns, reducing the prototype's whitespace rhythm.

### 06. Fallback (`screens/fallback.rs` vs JSX 870–950)
- **P1:** Prototype fallback policy is a distinct policy card layout with fallback chain semantics. Rendered output shows selectable fallback models but the policy/chain intent is less obvious.
- **P2:** Selected markers and badges need prototype card-table-badge treatment.
- **P3:** Footer hints are generic rather than fallback-specific.

### 07. Channels (`screens/channels.rs` vs JSX 951–1006)
- **P2:** Allowed channel set is present, but card grid spacing is compressed and some descriptions are clipped.
- **P2:** Prototype cloud/local grouping and selected-channel detail hierarchy are only partially visible.
- **P3:** Supported-set difference is allowed by spec, but copy/badge styling still needs final pass.

### 08. ChannelConfig (`screens/chan_config.rs` vs JSX 1007–1190)
- **P1:** Prototype uses focused credential fields plus per-channel test action/states and SignalPair sub-step. Rendered view has fields, but the test-state/badge hierarchy is not 1:1.
- **P2:** Secret masking consistency and hint layout need render verification after final style pass.
- **P2:** SignalPair-specific follow-on render should be audited separately from the generic channel config state.

### 09. Gateway (`screens/gateway.rs` vs JSX 1191–1262)
- **P1:** Host/port fields are present, but the prototype BIND section, LAN hint, and port-in-use probe state need richer visual treatment.
- **P2:** FEATURE pill toggles render as text rows/cards rather than prototype 30×16 pill toggles.
- **P2:** INSTALL AS SERVICE card grid exists but is squeezed; `WILL INSTALL` path panel needs exact prototype spacing/badge treatment.

### 10. Agent (`screens/agent.rs` vs JSX 1263–1308)
- **P1:** Rendered body shows persona grid + identity panel, but the selected archetype detail/SOUL preview is heavily truncated at 100×30.
- **P2:** Card names appear as `THE The ...`; glyph/name composition needs cleanup to match prototype.
- **P2:** Right panel hierarchy (`Tone`, SOUL preview, writes-to path) is not yet 1:1.

### 11. Workspace (`screens/workspace.rs` vs JSX 1309–1352)
- **P1:** Duplicate title/subtitle consumes rows and causes path cards to truncate.
- **P2:** Existing-workspace counts/probes are called out in spec as follow-up and are not visibly realized in the dump.
- **P2:** Path fields and warning/info boxes need exact prototype sizing and focus treatment.

### 12. Security (`screens/security.rs` vs JSX 1353–1409)
- **P2:** Security level cards exist but lack prototype badge/card weight and right-side consequence/detail emphasis.
- **P2:** Warning/capability rows need closer visual alignment.
- **P3:** Generic footer hints dilute screen-specific controls.

### 13. Features (`screens/features.rs` vs JSX 1410–1466)
- **P1:** Prototype feature matrix has grouped categories and toggle cards; rendered output is visibly compressed and truncates labels/descriptions at 100×30.
- **P2:** Toggle state badges are not prototype-shaped.
- **P3:** Group headings need exact copy/spacing pass.

### 14. Voice (`screens/voice.rs` vs JSX 1467–1520)
- **P1:** Prototype voice setup cards include provider, STT/TTS, Twilio/recording details. Rendered view shows cards but loses several detail fields in 100×30 clipping.
- **P2:** Provider status badges and voice preview styling are not 1:1.
- **P3:** Footer hints should be voice-specific.

### 15. Images (`screens/images.rs` vs JSX 1521–1564)
- **P1:** Prototype image-provider grid + model side panel are present in spirit but rendered columns clip model details.
- **P2:** Provider badges/status and model field styling need prototype card/badge treatment.

### 16. Orchestration (`screens/orchestration.rs` vs JSX 1565–1608)
- **P2:** Three-mode cards are visible and close structurally, but card heights/spacing and selected-state treatment drift from prototype.
- **P3:** Copy wraps differently at 100×30; tune card width/line clamps.

### 17. Memory (`screens/memory.rs` vs JSX 1609–1653)
- **P1:** Prototype expects backend cards and visible detection/probe state (`Ollama`, embeddings, `★REC`, `● DETECTED`). Rendered output shows backend list but probes/status styling is incomplete.
- **P2:** Ollama detection is explicitly listed as cross-cutting follow-up in spec; render currently reads as static/default.

### 18. Skills (`screens/skills.rs` vs JSX 1654–1737)
- **P1:** Prototype has filter input, category tabs, skill cards with install/enable badges. Rendered output shows these concepts but clips list/card details at 100×30.
- **P2:** Category tabs and install-state badges are not 1:1 pill/card shapes.
- **P3:** Search/filter hint treatment differs from prototype.

### 19. Complete (`screens/complete.rs` vs JSX 1738–1810)
- **P1:** Rendered summary is much sparser than prototype summary list with per-section rows and `READY/SKIPPED/ERROR` badges.
- **P1:** `TEST ALL BACKENDS` must be real connectivity checks; rendered button exists, but audit cannot confirm full real backend behavior from render alone.
- **P1:** `AWAKEN ZEUS` must write config + launch gateway; rendered button exists, but production handoff behavior remains a functional gate, not just visual.
- **P2:** Right-side NEXT STEPS exists but needs exact copy/spacing and badge treatment.

## Pass 2 — Production gaps

### Production chrome (`top_bar.rs`, `tab_bar.rs`, `status_bar.rs` vs JSX TopBar/TabBar)
- **P1:** TopBar/TabBar/StatusBar are one terminal row each, while prototype has distinct 22/26/24px bands with separators, badges, unread counts, and progress bar styling.
- **P2:** Context meter renders as `[0%]`; prototype renders a 10-cell `▓/░` context bar plus percent.
- **P2:** Connection/model/version/hostname badges are simplified; active colors cannot reach prototype fidelity until styles are audited, not just text.
- **P3:** Tab labels/glyphs fit, but unread badges/pending approvals are not visible in the no-live dump.

### Chat (`prod/chat_tab.rs` vs JSX ChatTab 156–448)
- **P1:** Empty/no-live render is nearly blank: only top chrome, input line, and status bar. Prototype Chat has message stream cards, queue/cooking telemetry, tool badges, slash command palette states, and action controls.
- **P1:** Input area lacks the prototype bordered composer shape and command/action affordances.
- **P2:** Streaming/cancel/expanded message states need separate rendered states; default dump does not exercise them.

### Office (`prod/office_tab.rs` vs JSX OfficeTab 449–646)
- **P1:** Office map/sidebar exists, but the rendered map is visually noisy and clipped at 100×30; prototype has clear department lanes, agent cards, health/status pills, and focused-agent detail panel.
- **P2:** Focused-agent panel is sparse (`NO FOCUS`) in default render; prototype intent requires selected agent details and actions.
- **P2:** Dashed card borders differ from prototype card shapes.

### Pantheon (`prod/pantheon_tab.rs` vs JSX PantheonTab 647–815)
- **P1:** Default render needs mission list + war-room/detail + event feed structure; audit dump indicates a much thinner/no-live state than prototype intent.
- **P2:** Mission status badges/progress/participant chips need exact card-table-badge styling.
- **P2:** Empty state should preserve the prototype layout skeleton rather than collapsing detail density.

### Tools (`prod/tools_tab.rs` vs JSX ToolsTab 816–951)
- **P1:** Prototype has searchable/filterable tool catalog, selected tool detail, schema/usage panels, and status chips. Rendered no-live state is simpler and needs richer skeleton/detail columns.
- **P2:** Filter input, category badges, and selected tool metadata need prototype spacing and badge styling.

### Memory (`prod/memory_tab.rs` vs JSX MemoryTab 952–1106)
- **P1:** Prototype has workspace/sessions/mnemosyne subtabs with tables/cards. Rendered route currently always calls `MemorySubTab::Workspace` from `frame_prod`; other subtabs are not reachable in the captured app route.
- **P1:** No-live workspace state should still match the prototype skeleton; rendered content is thinner than JSX intent.
- **P2:** Session/search result cards and embeddings/status badges need separate render-state coverage.

### Channels (`prod/channels_tab.rs` vs JSX ChannelsTab 1107–1188)
- **P1:** Prototype shows channel cards/status, unread/activity, and connection detail. Rendered no-live state lacks enough detail for 100% fidelity.
- **P2:** Channel status chips and per-channel controls need card/badge shape pass.

### Wallet (`prod/wallet_tab.rs` vs production intent / Wallet prototype references)
- **P1:** Wallet has direct render-fidelity tests and subview switcher, but production app capture only shows the selected default view; the six subviews need app-level render captures or direct tab dumps for the audit gate.
- **P2:** No-live honest-empty state is correct post-demock, but it must still preserve the prototype chrome: header glyph, 1–6 switcher, balance/activity table skeletons, and security cards.
- **P2:** Live balance/activity rows are covered in tests, but not in the default 100×30 production route dump.

### Approvals (`prod/approvals_tab.rs` vs JSX ApprovalsTab 1189–1252)
- **P1:** Prototype approval queue cards include risk/action badges and approve/deny controls. Rendered no-live state should keep that structural skeleton; current default appears sparse.
- **P2:** Pending count badge in TabBar not visible in capture.

### Settings (`prod/settings_tab.rs` vs JSX SettingsTab 1253–1381)
- **P1:** Prototype has settings groups, selected group detail, editable rows, and status badges. Rendered no-live state needs richer grouped layout and exact copy/spacing.
- **P2:** Config live overlay is good direction, but fallback placeholders must match prototype shapes rather than plain rows.

### Advanced grid (`prod/advanced.rs` vs JSX AdvancedTab 1382–1760)
- **P1:** Grid labels/glyphs match the 13 modules, but rendered cards are flattened into text rows; prototype uses card grid with colored glyph badges, selected border, and detail descriptions.
- **P2:** At 100×30, descriptions truncate (`S` after Skills, `semanti` after VectorStores), so grid width/columns need tuning.
- **P2:** Active/selected state is too subtle in text dump compared with prototype selected card.

### Advanced detail views (`prod/advanced_sub/*.rs`)
- **P1:** Detail route exists for all 13 modules, but many details are placeholder/skeleton-level relative to prototype intent: Agents, Skills, MCP, Projects, Canvas, Voice, NodeComms, VectorStores, Economy, Extensions, Knowledge Graph, Spawner, Deploy all need final per-view table/card/badge matching.
- **P2:** Header band `← Advanced / [GLYPH] Name · desc` is present, but per-view body density and card hierarchy vary significantly.
- **P2:** Empty/live states need separate captures; default no-live 100×30 does not prove fidelity for populated tables.

## Recommended fix order

1. Fix Welcome 100×30 panic and add an automated render smoke for every onboarding/prod route at 100×30.
2. Normalize chrome heights/spacing and remove duplicated onboarding body headers where the App StepHeader already provides them.
3. Restore/add missing high-value onboarding pieces: Welcome INITIATE card, Complete summary rows/backend-test/awaken states, Gateway service/probe panel, Agent SOUL preview, Skills filter/category/card density.
4. For production, add a small render-state matrix: default/no-live plus one representative populated state for Chat, Office, Pantheon, Tools, Memory, Channels, Wallet, Approvals, Settings, Advanced detail.
5. Only after shape/layout passes, do a style/color pass for card/table/badge fidelity.
