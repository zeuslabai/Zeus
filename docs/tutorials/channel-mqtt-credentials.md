# MQTT Channel — Getting Credentials

Zeus connects to any MQTT v5 broker. You can use a local broker or a cloud service.

## TUI Fields

```
broker_url:    mqtt://localhost             (see format below)
port:          1883                         (default — see below)
topic_prefix:  zeus/agents/                (prefix for all topics)
client_id:     zeus-agent-1                (any unique string)
username:      (optional)
password:      (optional)
```

## Broker URL Format

```
mqtt://hostname        # plain TCP (port 1883)
mqtts://hostname       # TLS (port 8883)
ws://hostname          # WebSocket
wss://hostname         # WebSocket + TLS
```

> **Note:** Zeus defaults to port **1883** if the port field is left blank.

## Local Broker (Mosquitto)

Quickest way to get started:

```bash
# macOS
brew install mosquitto
brew services start mosquitto

# Ubuntu/Debian
sudo apt install mosquitto mosquitto-clients
sudo systemctl start mosquitto
```

```
broker_url:   mqtt://localhost
port:         1883
username:     (blank)
password:     (blank)
```

## Cloud Brokers

| Provider | Broker URL | Notes |
|----------|-----------|-------|
| HiveMQ Cloud | `mqtts://your-id.s2.eu.hivemq.cloud` | Free tier available |
| EMQX Cloud | `mqtts://your-id.emqx.cloud` | Free tier available |
| AWS IoT Core | `mqtts://your-id.iot.region.amazonaws.com` | Cert-based auth |

Cloud brokers typically require username/password or certificates. Check your
provider's dashboard for connection details.

## Topic Prefix Convention

Zeus uses `{topic_prefix}{agent_name}` for message routing. Recommended format:

```
zeus/agents/          →  zeus/agents/zeus106, zeus/agents/zeus107, etc.
```
