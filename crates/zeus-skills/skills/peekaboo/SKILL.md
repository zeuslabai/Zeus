# peekaboo

System monitoring and process inspection utility.

## Version
1.0.0

## Author
Zeus

## System Prompt
You are a system monitoring assistant. Help users inspect running processes, monitor resource usage, track system health, and diagnose performance issues.

## Tools

### peek_processes
List running processes with resource usage.
```json
{
  "type": "object",
  "properties": {
    "sort_by": {
      "type": "string",
      "enum": ["cpu", "memory", "pid", "name"],
      "default": "cpu"
    },
    "limit": {
      "type": "integer",
      "default": 20
    },
    "filter": {
      "type": "string",
      "description": "Filter by process name"
    }
  }
}
```

### peek_process_detail
Get detailed info about a process.
```json
{
  "type": "object",
  "properties": {
    "pid": {
      "type": "integer"
    }
  },
  "required": ["pid"]
}
```

### peek_ports
List open network ports.
```json
{
  "type": "object",
  "properties": {
    "state": {
      "type": "string",
      "enum": ["listening", "established", "all"],
      "default": "listening"
    }
  }
}
```

### peek_connections
Show network connections.
```json
{
  "type": "object",
  "properties": {
    "pid": {
      "type": "integer",
      "description": "Filter by process ID"
    }
  }
}
```

### peek_files
List open files for a process.
```json
{
  "type": "object",
  "properties": {
    "pid": {
      "type": "integer"
    }
  },
  "required": ["pid"]
}
```

### peek_memory
Show memory usage summary.
```json
{
  "type": "object",
  "properties": {}
}
```

### peek_disk
Show disk usage.
```json
{
  "type": "object",
  "properties": {
    "path": {
      "type": "string",
      "default": "/"
    }
  }
}
```

### peek_cpu
Show CPU usage.
```json
{
  "type": "object",
  "properties": {
    "interval": {
      "type": "integer",
      "default": 1,
      "description": "Sampling interval in seconds"
    }
  }
}
```

### peek_uptime
Show system uptime and load.
```json
{
  "type": "object",
  "properties": {}
}
```

### peek_kill
Kill a process.
```json
{
  "type": "object",
  "properties": {
    "pid": {
      "type": "integer"
    },
    "signal": {
      "type": "string",
      "enum": ["TERM", "KILL", "HUP", "INT"],
      "default": "TERM"
    }
  },
  "required": ["pid"]
}
```

## Commands

### processes
```bash
ps aux --sort=-%{sort_by} | head -{limit}
```

### processes_macos
```bash
ps aux -r | head -{limit}
```

### process_detail
```bash
ps -p {pid} -o pid,ppid,user,%cpu,%mem,vsz,rss,stat,start,time,command
```

### ports
```bash
lsof -i -P -n | grep LISTEN
```

### connections
```bash
netstat -an | grep ESTABLISHED
```

### open_files
```bash
lsof -p {pid}
```

### memory
```bash
vm_stat && echo "---" && top -l 1 -s 0 | head -10
```

### disk
```bash
df -h "{path}"
```

### cpu
```bash
top -l 2 -s {interval} | grep "CPU usage" | tail -1
```

### uptime
```bash
uptime
```

### kill
```bash
kill -{signal} {pid}
```

## Permissions
- shell
