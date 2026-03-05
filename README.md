# MUD Driver

A multi-user dungeon platform with a Rust driver and Ruby game adapter. The driver handles infrastructure (databases, git, SSH, HTTP) while the Ruby adapter runs game logic and serves the web portal.

For a detailed breakdown of the system, see [docs/architecture.md](docs/architecture.md).

## Prerequisites

- **Rust** (stable toolchain)
- **Ruby** 3.x with Bundler
- **Docker** (for PostgreSQL)
- **Node.js** and npm (for SPA area builds)
- [**just**](https://github.com/casey/just) command runner

## Quick Start

```bash
# Start everything (PostgreSQL, build, run)
just up
```

This will:
1. Start a PostgreSQL container
2. Create directories and generate `config/server.yml`
3. Install Ruby gems
4. Build the Rust workspace
5. Run the driver

The server is available at:
- **HTTP portal**: http://localhost:8080
- **SSH**: port 2222

## Development Commands

```bash
just              # List all available commands
just up           # Start full environment
just down         # Stop everything
just dev          # Run driver with dev config
just build        # Build workspace
just clean        # Remove all build artifacts, databases, and world data
```

### Database

```bash
just db-start     # Start PostgreSQL in Docker
just db-stop      # Stop container
just db-destroy   # Remove container and data
just db-shell     # Open psql shell (driver DB)
just db-shell mud_stdlib  # Open psql shell (stdlib DB)
just db-status    # Show container status
```

### Testing

```bash
just test         # Run all tests (Rust + Ruby)
just test-rust    # Rust tests only
just test-ruby    # Ruby tests only
just test-portal  # Portal E2E test (requires Docker + Ruby)
just test-ignored # Run ignored tests (E2E tests that need Docker)
just test-verbose # Tests with stdout/stderr output
just test-one <name>  # Run a specific test by name
```

### Code Quality

```bash
just clippy       # Run clippy linter
just fmt          # Format code
just fmt-check    # Check formatting
just ci           # Full CI: format check + clippy + tests
```

## Configuration

The default config is generated at `config/server.yml` by `just setup`. Key sections:

| Section | What it controls |
|---------|-----------------|
| `ssh` | SSH server host and port (default 2222) |
| `http` | HTTP server host and port (default 8080) |
| `database` | PostgreSQL connection (host, port, credentials) |
| `world` | Paths for area working directories and git repos |
| `adapters.ruby` | Ruby adapter command and path |
| `ai` | AI provider settings |

## Project Layout

```
crates/
  mud-core/       # Shared types (AreaId, SessionId, ObjectId)
  mud-mop/        # MOP protocol (MessagePack over Unix socket)
  mud-driver/     # Main server (database, git, SSH, HTTP, orchestration)
adapters/
  ruby/           # Game logic adapter (areas, commands, web portal)
config/           # Server configuration (generated)
docs/             # Architecture documentation
justfile          # Development recipes
```
