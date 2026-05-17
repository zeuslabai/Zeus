//! IRC tools via subprocess
//!
//! Uses a lightweight approach: spawns a TCP connection to send messages,
//! or uses an IRC client binary if available.

use crate::TalosTool;
use async_trait::async_trait;
use serde_json::Value;
use zeus_core::{Error, Result, ToolSchema};

fn get_server(args: &Value) -> Result<String> {
    args.get("server")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| std::env::var("IRC_SERVER").ok())
        .ok_or_else(|| Error::Tool("Missing 'server' / IRC_SERVER".to_string()))
}

fn get_nick(args: &Value) -> String {
    args.get("nick")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| std::env::var("IRC_NICK").ok())
        .unwrap_or_else(|| "zeus-bot".to_string())
}

fn get_port(args: &Value) -> u16 {
    args.get("port").and_then(|v| v.as_u64()).unwrap_or(6667) as u16
}

/// Send raw IRC commands via TCP
async fn irc_send_raw(server: &str, port: u16, nick: &str, commands: &[String]) -> Result<String> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::TcpStream;

    let stream = TcpStream::connect(format!("{}:{}", server, port))
        .await
        .map_err(|e| Error::Tool(format!("IRC connect failed: {}", e)))?;
    let (reader, mut writer) = stream.into_split();
    let mut buf_reader = BufReader::new(reader);

    // Register
    writer
        .write_all(format!("NICK {}\r\n", nick).as_bytes())
        .await
        .map_err(|e| Error::Tool(e.to_string()))?;
    writer
        .write_all(format!("USER {} 0 * :Zeus Bot\r\n", nick).as_bytes())
        .await
        .map_err(|e| Error::Tool(e.to_string()))?;

    // Wait for welcome (001) or timeout
    let mut registered = false;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
    while tokio::time::Instant::now() < deadline {
        let mut line = String::new();
        let read = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            buf_reader.read_line(&mut line),
        )
        .await;
        match read {
            Ok(Ok(0)) => break,
            Ok(Ok(_)) => {
                if line.starts_with("PING") {
                    let pong = line.replace("PING", "PONG");
                    writer.write_all(pong.as_bytes()).await.ok();
                }
                if line.contains(" 001 ") {
                    registered = true;
                    break;
                }
            }
            _ => break,
        }
    }

    if !registered {
        return Err(Error::Tool("IRC registration timeout".to_string()));
    }

    // Send commands
    for cmd in commands {
        writer
            .write_all(format!("{}\r\n", cmd).as_bytes())
            .await
            .map_err(|e| Error::Tool(e.to_string()))?;
    }

    // Brief pause then quit
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    writer.write_all(b"QUIT :Zeus\r\n").await.ok();
    Ok("Commands sent".to_string())
}

pub struct IrcSendMessageTool;
#[async_trait]
impl TalosTool for IrcSendMessageTool {
    fn name(&self) -> &'static str {
        "irc_send_message"
    }
    fn description(&self) -> &'static str {
        "Send a message to an IRC channel or user"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "target",
                "string",
                "Channel (#channel) or nick to message",
                true,
            )
            .with_param("message", "string", "Message text", true)
            .with_param(
                "server",
                "string",
                "IRC server hostname (or IRC_SERVER)",
                false,
            )
            .with_param("port", "integer", "Server port (default 6667)", false)
            .with_param("nick", "string", "Bot nickname (default zeus-bot)", false)
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let server = get_server(&args)?;
        let nick = get_nick(&args);
        let port = get_port(&args);
        let target = args
            .get("target")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'target'".to_string()))?;
        let message = args
            .get("message")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'message'".to_string()))?;

        let mut cmds = vec![];
        if target.starts_with('#') {
            cmds.push(format!("JOIN {}", target));
        }
        cmds.push(format!("PRIVMSG {} :{}", target, message));
        irc_send_raw(&server, port, &nick, &cmds).await?;
        Ok(format!("Message sent to {} on {}", target, server))
    }
}

pub struct IrcJoinChannelTool;
#[async_trait]
impl TalosTool for IrcJoinChannelTool {
    fn name(&self) -> &'static str {
        "irc_join_channel"
    }
    fn description(&self) -> &'static str {
        "Join an IRC channel"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("channel", "string", "Channel to join (e.g. #general)", true)
            .with_param("server", "string", "IRC server hostname", false)
            .with_param("port", "integer", "Server port", false)
            .with_param("nick", "string", "Bot nickname", false)
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let server = get_server(&args)?;
        let nick = get_nick(&args);
        let port = get_port(&args);
        let channel = args
            .get("channel")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'channel'".to_string()))?;
        irc_send_raw(&server, port, &nick, &[format!("JOIN {}", channel)]).await?;
        Ok(format!("Joined {}", channel))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_send() {
        assert_eq!(IrcSendMessageTool.name(), "irc_send_message");
    }
    #[test]
    fn test_join() {
        assert_eq!(IrcJoinChannelTool.name(), "irc_join_channel");
    }
}
