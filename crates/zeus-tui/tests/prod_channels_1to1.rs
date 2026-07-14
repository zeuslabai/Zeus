//! Render-fidelity guard for the Production TUI Channels tab.
//!
//! SoT: `docs/zeus-tui-production.jsx` (`ChannelsTab`, JSX 1107–1184).
//! The tab should present the prototype's messaging-adapter cards while
//! rendering only real `/v1/channels` rows. Fields not exposed by the API render
//! as `—`, not invented mock values.

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::widgets::Widget;

use zeus_tui::api::ChannelResponse;
use zeus_tui::prod::channels_tab::ChannelsTab;

fn render_channels(widget: ChannelsTab) -> (Buffer, String) {
    let backend = TestBackend::new(120, 34);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal
        .draw(|f| {
            widget.render(f.area(), f.buffer_mut());
        })
        .expect("draw must not panic");
    let buf = terminal.backend().buffer().clone();
    (buf.clone(), dump_buffer(&buf))
}

fn dump_buffer(buf: &Buffer) -> String {
    let mut lines = Vec::with_capacity(buf.area.height as usize);
    for y in 0..buf.area.height {
        let mut row = String::new();
        for x in 0..buf.area.width {
            row.push_str(buf[(x, y)].symbol());
        }
        lines.push(row.trim_end().to_string());
    }
    lines.join("\n")
}

fn sample_channels() -> Vec<ChannelResponse> {
    vec![
        ChannelResponse {
            id: "discord-prod".into(),
            channel_type: "discord".into(),
            name: "Fleet Discord".into(),
            status: "connected".into(),
            enabled: Some(true),
            connected_at: Some("2026-06-30T10:00:00Z".into()),
            last_message_at: Some("2026-06-30T10:58:00Z".into()),
        },
        ChannelResponse {
            id: "telegram-dm".into(),
            channel_type: "telegram".into(),
            name: "Telegram DM".into(),
            status: "connecting".into(),
            enabled: Some(true),
            connected_at: None,
            last_message_at: None,
        },
        ChannelResponse {
            id: "slack-disabled".into(),
            channel_type: "slack".into(),
            name: "Slack".into(),
            status: "disabled".into(),
            enabled: Some(false),
            connected_at: None,
            last_message_at: None,
        },
    ]
}

#[test]
fn channels_tab_matches_jsx_card_structure_with_real_rows() {
    let live = sample_channels();
    let (_buf, dump) = render_channels(ChannelsTab::with_live(Some(&live)));

    for expected in [
        "Messaging adapters",
        "3 channels — all running in single zeus-channels process",
        "● 1 CONNECTED",
        "● 1 RECONNECTING",
        "● 1 DISCONNECTED",
        "[DC]",
        "Fleet Discord",
        "id discord-prod",
        "binding —",
        "sdk —",
        "— MSGS / 24H",
        "[ TEST ] [ EDIT ] [ PAUSE ]",
    ] {
        assert!(dump.contains(expected), "missing {expected:?}:\n{dump}");
    }
}

#[test]
fn channels_tab_does_not_render_mock_catalog_without_api_rows() {
    let (_buf, dump) = render_channels(ChannelsTab::new());

    assert!(
        dump.contains("Waiting for /v1/channels"),
        "standalone empty state missing:\n{dump}"
    );
    assert!(
        !dump.contains("ZeusBot#0042"),
        "mock Discord binding leaked into production tab:\n{dump}"
    );
    assert!(
        !dump.contains("grammers MTProto"),
        "mock Telegram SDK leaked into production tab:\n{dump}"
    );
}

#[test]
fn channels_tab_renders_api_empty_response_as_empty_not_mocked() {
    let live: Vec<ChannelResponse> = Vec::new();
    let (_buf, dump) = render_channels(ChannelsTab::with_live(Some(&live)));

    assert!(
        dump.contains("0 channels — all running in single zeus-channels process"),
        "empty live count missing:\n{dump}"
    );
    assert!(
        dump.contains("No channel adapters returned by /v1/channels"),
        "empty live state missing:\n{dump}"
    );
}


#[test]
fn channels_tab_renders_instagram_and_tiktok_post_only_rows() {
    let live = vec![
        ChannelResponse {
            id: "ig-live".into(),
            channel_type: "instagram".into(),
            name: "".into(),
            status: "connected".into(),
            enabled: Some(true),
            connected_at: None,
            last_message_at: Some("2026-07-13T20:00:00Z".into()),
        },
        ChannelResponse {
            id: "tt-post".into(),
            channel_type: "tiktok".into(),
            name: "".into(),
            status: "connected".into(),
            enabled: Some(true),
            connected_at: None,
            last_message_at: None,
        },
    ];
    let (_buf, dump) = render_channels(ChannelsTab::with_live(Some(&live)));

    assert!(dump.contains("Instagram"), "Instagram live row missing:\n{dump}");
    assert!(dump.contains("TikTok"), "TikTok live row missing:\n{dump}");
    assert!(dump.contains("mode post-only"), "TikTok must be labeled post-only:\n{dump}");
    assert!(dump.contains("sdk post-only"), "TikTok SDK line must be post-only labeled:\n{dump}");
    assert!(
        !dump.contains("— MSGS / 24H"),
        "TikTok post-only row must not render fake inbound message stats:\n{dump}"
    );
}
