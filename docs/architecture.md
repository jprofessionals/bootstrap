# MUD Driver Architecture

A multi-user dungeon platform built as a Rust driver + Ruby adapter, communicating over the MOP protocol (MessagePack over Unix sockets).

---

## System Overview

```
                   ┌──────────────────────────────────────────┐
                   │              Rust Driver                  │
                   │                                          │
  SSH ────────────►│  server.rs (orchestration)               │
                   │    ├─ persistence/ (PostgreSQL)           │
  HTTP ───────────►│    ├─ git/ (bare repos, MRs, workspace)  │
   │               │    ├─ ssh/ (russh)                       │
   │               │    └─ web/ (axum, AI providers)          │
   │               └──────────────┬───────────────────────────┘
   │                              │ MOP (Unix socket)
   │                              ▼
   │               ┌──────────────────────────────────────────┐
   │               │             Ruby Adapter                  │
   │               │                                          │
   │               │  client.rb (MOP client)                  │
   │               │    ├─ area_loader.rb (load game world)   │
   │               │    ├─ stdlib/ (game objects, commands)    │
   │               │    └─ portal/ (Roda web apps)            │
   └──── proxy ───►│         ├─ editor, git, builder          │
                   │         ├─ account, play                  │
                   │         └─ review (merge requests)        │
                   └──────────────────────────────────────────┘
```

**HTTP flow**: Axum handles `/api/ai/*` and HTTP git directly. Everything else is reverse-proxied to the Ruby portal (Puma/Roda) over a Unix socket.

---

## MOP Protocol

**Wire format**: 4-byte big-endian length prefix + MessagePack payload. Max 16 MB per frame.

Defined in `crates/mud-mop/`. Two message enums:

**DriverMessage** (Rust → Ruby):
- `LoadArea`, `ReloadArea`, `UnloadArea` — area lifecycle
- `SessionStart`, `SessionInput`, `SessionEnd` — player sessions
- `Configure` — send stdlib DB URL after boot
- `RequestResponse`, `RequestError` — replies to adapter requests
- `Ping`

**AdapterMessage** (Ruby → Rust):
- `AreaLoaded`, `AreaError` — load results
- `SessionOutput`, `SendMessage` — text to players
- `DriverRequest` — generic request (action string + params map)
- `Log` — structured area logging
- `Handshake`, `Pong`

The `DriverRequest`/`RequestResponse` pattern provides a synchronous RPC channel from Ruby to Rust (e.g., `mr_create`, `account_authenticate`, `provision_area_db`).

---

## Databases

Two PostgreSQL databases, both managed by the driver:

### Driver DB

Created and migrated by `persistence/database_manager.rs`. Tables:

| Table | Purpose |
|-------|---------|
| `area_databases` | Per-area DB credentials (namespace, area_name, db_user, password) |
| `area_registry` | Area metadata |
| `merge_requests` | MR state (open/approved/merged/rejected) |
| `merge_request_approvals` | Approval records |
| `ai_keys` | Encrypted API keys (AES-GCM) for AI providers |

### Stdlib DB

Created by the driver, migrated by the Ruby adapter (`stdlib_migrator.rb`). Tables:

| Table | Purpose |
|-------|---------|
| `players` | Accounts (username, bcrypt password_hash, role) |
| `characters` | Player characters |
| `sessions` | Active login sessions |
| `access_tokens` | Personal access tokens (for git HTTP auth) |

### Per-Area Databases

Each area gets its own PostgreSQL database, provisioned on first load. The driver creates the role and database, stores credentials in `area_databases`, and passes the connection URL to the adapter via MOP.

Area code accesses its database through the container:
```ruby
db = MUD::Container["database.namespace/area_name"]
```

Sequel migrations in `db/migrations/` run automatically on load/reload.

---

## Git Layer

### Repository Structure

```
git-server/
  └── <namespace>/
      ├── <area>.git          # bare repository
      └── <area>.git.acl.yml  # access control (owner + collaborators)
```

`RepoManager` (`git/repo_manager.rs`) creates bare repos with template files and two branches: `main` (production) and `develop` (staging).

### Workspace

`Workspace` (`git/workspace.rs`) manages checked-out working directories under the world path:

```
world/
  └── <namespace>/
      ├── <area>/        # main branch checkout (loaded as area)
      └── <area>@dev/    # develop branch checkout (for editing)
```

Git pushes trigger area reloads via the post-receive hook → MOP.

### Merge Requests

`MergeRequestManager` (`git/merge_request_manager.rs`) handles the full lifecycle:
- Create MR (source → target branch)
- Approve (with configurable required approval count via `ReviewPolicy`)
- Execute merge (git2 merge on bare repo, fast-forward workspace)
- Reject / Close / Reopen

Branch protection (`branch_protection.rs`) can enforce MR-only merges to main.

### HTTP Git

`web/git_http.rs` implements the smart HTTP git protocol so areas can be cloned/pushed via:
```
git clone http://user:password@host:port/git/namespace/area.git
```

---

## Rust Driver Internals

### Server (`server.rs`)

The central coordinator. Key state:

- `SessionState` — maps session IDs to output channels
- `AdapterManager` — spawns and connects to Ruby adapter process
- `DatabaseManager` — PostgreSQL pools and migrations
- `PlayerStore` — account CRUD and authentication
- `RepoManager` / `Workspace` — git operations
- `MergeRequestManager` — MR lifecycle

**Boot sequence** (`boot()`):
1. Initialize databases (driver + stdlib), run migrations
2. Send stdlib DB URL to adapter via `Configure` message
3. Create `PlayerStore`, `RepoManager`, `Workspace`, `MergeRequestManager`
4. Start adapter process, read handshake
5. Start SSH server
6. Start HTTP server (axum + portal proxy)
7. Load area template files
8. Enter main message loop (dispatch MOP messages from adapter)

### Web Server (`web/server.rs`)

Axum routes:
- `/api/ai/models` — list AI models
- `/api/ai/chat` — SSE streaming chat
- `/api/ai/keys/*` — manage encrypted API keys
- `/git/<ns>/<area>.git/*` — HTTP git protocol
- `/*` — reverse proxy to Ruby portal Unix socket

### AI Providers (`web/ai_providers/`)

Pluggable provider trait:
```rust
pub trait AiProvider: Send + Sync {
    fn name(&self) -> &str;
    fn models(&self) -> Vec<ModelInfo>;
    fn build_request(&self, req: &StreamRequest, api_key: &str) -> (String, HeaderMap, String);
    fn translate_event(&self, event_type: &str, data: &str) -> Option<String>;
}
```

Implementations: `AnthropicProvider`, `OpenAiProvider`, `GeminiProvider`.

The editor's AI panel streams through the driver, which proxies to the selected provider.

---

## Ruby Adapter Internals

### Client (`client.rb`)

MOP protocol client. Connects to the driver Unix socket, sends/receives length-prefixed MessagePack frames. Provides `send_driver_request()` for synchronous RPC (blocks on a response queue with timeout).

### Area Loading (`area_loader.rb`)

1. Receives `LoadArea` message with path and area_id
2. Creates `Stdlib::World::Area` instance
3. Area evaluates `mud_aliases.rb` (class aliases), `mud_loader.rb` (directory mappings), then loads all `.rb` files from mapped directories
4. Connects area database (if provisioned), runs Sequel migrations
5. Builds SPA if `mud_web.rb` declares `web_mode :spa` (npm install + vite build)
6. Sends `AreaLoaded` or `AreaError` back to driver

### Game Object Hierarchy

```
GameObject (title, description)
  ├── Room (exits, on_enter)
  ├── Item (portable, on_use)
  ├── NPC (location, on_talk)
  └── Daemon (interval, on_tick)
```

All defined as Ruby classes using a class-level DSL. One class per file, filename matches class name (snake_case → PascalCase).

### Portal Web Apps

Roda-based apps mounted under `BaseApp`:

| App | Route | Purpose |
|-----|-------|---------|
| `AccountApp` | `/account` | Register, login, logout |
| `PlayApp` | `/play` | In-game text interface |
| `EditorApp` | `/editor` | Monaco code editor + AI assistant |
| `GitApp` | `/git` | Git dashboard, branches, commits, MRs |
| `ReviewApp` | `/review` | Merge request review UI |
| `BuilderApp` | `/builder` | Area web apps (ERB or SPA mode) |

`BaseApp` provides shared helpers: `require_login!`, `current_account`, `mop_client`, `area_loader`, `render_view`.

### Builder Web Modes

Areas can serve web content at `/builder/<ns>/<area>/`. Configured in `mud_web.rb`:

**ERB mode** (default): Renders `web/index.erb` with locals from `web_data` block.

**SPA mode**: Vite-built JS app from `web/src/`. The platform handles npm install, vite build (with correct `--base` URL), and `window.__MUD__` injection.

**Rack app mode** (`web_app` block): Mount any Rack-compatible app for the API backend. In SPA mode, only `/api/*` routes reach the Rack app; everything else is served by the SPA frontend.

---

## Area File Structure

```
<area>/
├── .meta.yml            # Owner, status
├── mud_aliases.rb       # Class aliases (Room, Item, NPC, Daemon)
├── mud_loader.rb        # Directory → type mappings
├── mud_web.rb           # Web mode, data, routes, Rack app
├── mud_network.rb       # TCP listener declarations
├── agents.md            # Platform API reference (sent to AI assistant)
├── rooms/               # Room subclasses
├── items/               # Item subclasses
├── npcs/                # NPC subclasses
├── daemons/             # Daemon subclasses
├── db/
│   └── migrations/      # Sequel migrations (auto-run on load)
└── web/                 # Web assets
    ├── index.erb        # ERB template (ERB mode)
    └── src/             # SPA source (SPA mode)
        ├── index.html
        ├── main.js
        ├── package.json
        └── vite.config.js
```

---

## Session Lifecycle

1. Player connects via SSH or HTTP (portal login)
2. `PlayerStore` authenticates (bcrypt)
3. Driver allocates session ID, sends `SessionStart` to adapter
4. Adapter creates session, places player in a room
5. Player input → `SessionInput` → adapter command parser → game logic
6. Game output → `SessionOutput` → driver → player's SSH/HTTP channel
7. Disconnect → `SessionEnd` → adapter cleanup

---

## Test Infrastructure

Integration tests in `crates/mud-driver/tests/` use:
- **testcontainers**: Spins up PostgreSQL in Docker per test
- **Real Ruby adapter**: Most e2e tests spawn the actual adapter process
- **HTTP client**: Tests exercise the full HTTP stack (portal, git, builder)

Key test files:
- `portal_webapp_e2e_test.rs` — web_app Rack mounting + area database provisioning
- `portal_git_ops_e2e_test.rs` — git push → area reload → verify content
- `full_stack_e2e_test.rs` — account → git → HTTP git → reload
- `portal_builder_e2e_test.rs` — builder web content serving

---

## Configuration

`config.yml` (loaded at startup):

```yaml
server_name: "My MUD"
ssh:
  host: "127.0.0.1"
  port: 2222
http:
  host: "127.0.0.1"
  port: 8080
  enabled: true
world:
  path: "world"
  git_path: "git-server"
  data_path: "data"
database:
  host: "localhost"
  port: 5432
  admin_user: "mud_admin"
  admin_password: "secret"
  driver_db: "mud_driver"
  stdlib_db: "mud_stdlib"
adapters:
  ruby:
    enabled: true
    command: "ruby"
    adapter_path: "adapters/ruby/bin/mud-adapter"
ai:
  enabled: true
```

---

## Key Dependencies

**Rust**: tokio, axum, sqlx (PostgreSQL), git2, russh, rmp-serde, bcrypt, aes-gcm, reqwest, tera, tracing

**Ruby**: roda 3, rack 3, puma 6, msgpack 1.7, sequel, erb, bcrypt
