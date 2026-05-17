# Docker Deployment

Zeus ships with Docker support for containerized deployment.

## Quick Start

```bash
# Build the image
docker build -t zeus .

# Run with your existing config
docker run -d \
  --name zeus-gateway \
  -v ~/.zeus:/home/zeus/.zeus \
  --env-file ~/.zeus/.env \
  -p 8080:8080 \
  zeus
```

## Docker Compose

The included `docker-compose.yml` handles volume mounts and environment variables:

```bash
# Start Zeus gateway
docker compose up -d

# Rebuild and start
docker compose up -d --build

# Follow logs
docker compose logs -f zeus

# Stop
docker compose down
```

### Configuration

1. Create `~/.zeus/config.toml` with your model and provider settings
2. Create `~/.zeus/.env` with API keys:
   ```
   ANTHROPIC_API_KEY=sk-ant-...
   ZEUS_API_TOKEN=your-token
   ```
3. Run `docker compose up -d`

### Ollama Sidecar

Uncomment the `ollama` service in `docker-compose.yml` to run local LLM inference alongside Zeus:

```yaml
services:
  ollama:
    image: ollama/ollama:latest
    ports:
      - "11434:11434"
    volumes:
      - ollama_data:/root/.ollama
```

Then set `model = "ollama/llama3.2"` in config.toml and `OLLAMA_HOST=http://ollama:11434`.

## Fly.io

Deploy to [Fly.io](https://fly.io) with the included `fly.toml`:

```bash
# First time
fly launch

# Set secrets
fly secrets set ANTHROPIC_API_KEY=sk-ant-...
fly secrets set ZEUS_API_TOKEN=your-token

# Deploy
fly deploy

# Check status
fly status
fly logs
```

The Fly config uses a persistent volume mounted at `/home/zeus/.zeus` for config, workspace, sessions, and databases.

## Health Check

All deployment methods include a health check against `GET /health`:

```bash
curl http://localhost:8080/health
```

## Image Details

- **Base**: `debian:bookworm-slim`
- **Binary size**: ~100MB (stripped release build)
- **Runtime deps**: `ca-certificates`, `libssl3`, `libsqlite3-0`
- **User**: Runs as non-root `zeus` user
- **Init**: Uses `tini` for proper signal handling
- **Ports**: 8080 (API gateway), 3002 (MCP server, optional)
