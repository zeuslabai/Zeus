use crossterm::event::KeyCode;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use zeus_tui::App;
use zeus_tui::app::frame;
use zeus_tui::model_fetch::{fetch_models, normalize_ollama_host};

const PROVIDER_STEP: usize = 3;
const AUTH_STEP: usize = 4;

fn render_text(app: &App) -> String {
    let backend = TestBackend::new(120, 40);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal.draw(|f| frame(f, app)).expect("draw");
    let buf = terminal.backend().buffer();
    let mut out = String::new();
    for y in 0..buf.area.height {
        for x in 0..buf.area.width {
            out.push_str(buf[(x, y)].symbol());
        }
        out.push('\n');
    }
    out
}

fn select_ollama(app: &mut App) {
    app.current_step = PROVIDER_STEP;
    app.on_step_enter();
    for _ in 0..3 {
        app.handle_key(KeyCode::Down);
    }
    app.advance_step();
    assert_eq!(app.current_step, AUTH_STEP);
}

#[test]
fn ollama_auth_collects_host_not_api_key() {
    let mut app = App::new();
    select_ollama(&mut app);

    let dump = render_text(&app);
    assert!(dump.contains("Authenticate with Ollama"), "{dump}");
    assert!(dump.contains("Ollama URL"), "{dump}");
    assert!(dump.contains("URL + port"), "{dump}");
    assert!(dump.contains("OLLAMA_HOST"), "{dump}");
    assert!(dump.contains("/api/tags"), "{dump}");
    assert!(dump.contains("http://localhost:11434"), "{dump}");
    assert!(!dump.contains("Paste a provider-issued API key"), "{dump}");
    assert!(!dump.contains("API Key"), "{dump}");
}

#[test]
fn ollama_auth_accepts_blank_default_host() {
    let mut app = App::new();
    select_ollama(&mut app);

    assert_eq!(app.auth_api_key(), "");
    assert!(
        app.auth_key_valid(),
        "blank Ollama host means default localhost"
    );
}

#[test]
fn normalize_ollama_host_accepts_bare_host_port() {
    assert_eq!(
        normalize_ollama_host("localhost:11434/"),
        "http://localhost:11434"
    );
    assert_eq!(
        normalize_ollama_host("https://ollama.lan:11434/"),
        "https://ollama.lan:11434"
    );
}

#[tokio::test]
async fn ollama_model_fetch_polls_api_tags_on_entered_host() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (tx, rx) = tokio::sync::oneshot::channel::<String>();
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut buf = [0u8; 1024];
        let n = socket.read(&mut buf).await.unwrap();
        let req = String::from_utf8_lossy(&buf[..n]).to_string();
        let body = r#"{"models":[{"name":"llama3.2:latest"},{"name":"qwen2.5:7b"}]}"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{}",
            body.len(),
            body
        );
        socket.write_all(response.as_bytes()).await.unwrap();
        let _ = tx.send(req);
    });

    let models = fetch_models("ollama", &format!("http://{addr}"))
        .await
        .unwrap();
    assert_eq!(models, vec!["llama3.2:latest", "qwen2.5:7b"]);
    let req = rx.await.unwrap();
    assert!(req.starts_with("GET /api/tags "), "{req}");
}
