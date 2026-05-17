# docker

Manage Docker containers, images, volumes, and networks via the docker CLI.

## Version
1.0.0

## Author
Zeus

## System Prompt
You are a Docker management assistant. Help users build, run, and manage containers, images, volumes, and networks. Always warn before removing images or pruning. Show resource usage when asked about system health. Prefer docker compose for multi-container setups.

## Tools

### docker_ps
List running containers.
```json
{
  "type": "object",
  "properties": {
    "all": {
      "type": "boolean",
      "default": false,
      "description": "Include stopped containers"
    },
    "filter": {
      "type": "string",
      "description": "Filter (e.g. 'name=myapp', 'status=running')"
    }
  }
}
```

### docker_run
Run a new container.
```json
{
  "type": "object",
  "properties": {
    "image": {
      "type": "string",
      "description": "Image name and tag (e.g. nginx:latest)"
    },
    "name": {
      "type": "string",
      "description": "Container name"
    },
    "ports": {
      "type": "array",
      "items": {"type": "string"},
      "description": "Port mappings (e.g. ['8080:80', '443:443'])"
    },
    "env": {
      "type": "array",
      "items": {"type": "string"},
      "description": "Environment variables (e.g. ['KEY=value'])"
    },
    "volumes": {
      "type": "array",
      "items": {"type": "string"},
      "description": "Volume mounts (e.g. ['./data:/data'])"
    },
    "detach": {
      "type": "boolean",
      "default": true
    },
    "rm": {
      "type": "boolean",
      "default": false,
      "description": "Remove container when it exits"
    }
  },
  "required": ["image"]
}
```

### docker_stop
Stop a running container.
```json
{
  "type": "object",
  "properties": {
    "container": {
      "type": "string",
      "description": "Container name or ID"
    }
  },
  "required": ["container"]
}
```

### docker_logs
View container logs.
```json
{
  "type": "object",
  "properties": {
    "container": {
      "type": "string"
    },
    "tail": {
      "type": "integer",
      "default": 100,
      "description": "Number of lines from the end"
    },
    "follow": {
      "type": "boolean",
      "default": false
    }
  },
  "required": ["container"]
}
```

### docker_exec
Execute a command inside a running container.
```json
{
  "type": "object",
  "properties": {
    "container": {
      "type": "string"
    },
    "command": {
      "type": "string",
      "description": "Command to execute"
    },
    "interactive": {
      "type": "boolean",
      "default": false
    }
  },
  "required": ["container", "command"]
}
```

### docker_images
List local images.
```json
{
  "type": "object",
  "properties": {
    "filter": {
      "type": "string",
      "description": "Filter by reference (e.g. 'nginx')"
    }
  }
}
```

### docker_build
Build an image from a Dockerfile.
```json
{
  "type": "object",
  "properties": {
    "path": {
      "type": "string",
      "description": "Build context path",
      "default": "."
    },
    "tag": {
      "type": "string",
      "description": "Image tag (e.g. myapp:latest)"
    },
    "file": {
      "type": "string",
      "description": "Dockerfile path (if not ./Dockerfile)"
    },
    "no_cache": {
      "type": "boolean",
      "default": false
    }
  },
  "required": ["tag"]
}
```

### docker_compose
Run docker compose commands.
```json
{
  "type": "object",
  "properties": {
    "action": {
      "type": "string",
      "enum": ["up", "down", "ps", "logs", "restart", "build", "pull"],
      "description": "Compose action"
    },
    "file": {
      "type": "string",
      "description": "Compose file path",
      "default": "docker-compose.yml"
    },
    "service": {
      "type": "string",
      "description": "Specific service name (optional)"
    },
    "detach": {
      "type": "boolean",
      "default": true
    }
  },
  "required": ["action"]
}
```

### docker_stats
Show resource usage statistics.
```json
{
  "type": "object",
  "properties": {
    "container": {
      "type": "string",
      "description": "Specific container (optional, all if omitted)"
    }
  }
}
```

## Commands

### ps
```bash
docker ps --format "table {{.ID}}\t{{.Names}}\t{{.Image}}\t{{.Status}}\t{{.Ports}}"
```

### run
```bash
docker run -d --name {name} {image}
```

### stop
```bash
docker stop {container}
```

### logs
```bash
docker logs --tail {tail} {container}
```

### exec
```bash
docker exec {container} {command}
```

### images
```bash
docker images --format "table {{.Repository}}\t{{.Tag}}\t{{.Size}}\t{{.CreatedSince}}"
```

### build
```bash
docker build -t {tag} {path}
```

### compose_up
```bash
docker compose -f {file} up -d
```

### stats
```bash
docker stats --no-stream --format "table {{.Name}}\t{{.CPUPerc}}\t{{.MemUsage}}\t{{.NetIO}}"
```

## Permissions
- shell_execute
