# Shell Completions

Zeus can generate tab-completion scripts for bash, zsh, and fish. These provide auto-completion of subcommands, flags, and arguments as you type.

## Generating Completions

```bash
zeus completion bash    # Output bash completion script
zeus completion zsh     # Output zsh completion script
zeus completion fish    # Output fish completion script
```

## Installation

### Bash

Add the following to your `~/.bashrc` or `~/.bash_profile`:

```bash
eval "$(zeus completion bash)"
```

Or generate the script to a file:

```bash
zeus completion bash > ~/.local/share/bash-completion/completions/zeus
```

### Zsh

Add the following to your `~/.zshrc`:

```zsh
eval "$(zeus completion zsh)"
```

Or place the script in your completions directory:

```zsh
zeus completion zsh > ~/.zfunc/_zeus
```

Make sure `~/.zfunc` is in your `fpath` before `compinit`:

```zsh
fpath=(~/.zfunc $fpath)
autoload -Uz compinit && compinit
```

### Fish

```fish
zeus completion fish | source
```

Or save to the fish completions directory:

```fish
zeus completion fish > ~/.config/fish/completions/zeus.fish
```

## What Gets Completed

The completion scripts provide tab-completion for:

- Top-level subcommands (`tui`, `serve`, `gateway`, `chat`, `tool`, `config`, `memory`, `session`, `doctor`, `onboard`, `daemon`, `completion`)
- Subcommand-specific arguments (e.g., `memory show`, `memory remember`, `memory note`)
- Flags and options (e.g., `serve -p`, `chat -s`, `config --show-secrets`)
