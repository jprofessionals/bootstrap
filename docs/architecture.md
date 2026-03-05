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
   │               │    ├─ web/ (axum, AI providers)          │
   │               │    └─ build/ (BuildManager, SPA builds)  │
   │               └──────────────┬───────────────────────────┘
   │                              │ MOP (Unix socket)
   │  /project/* ─► driver       ▼
   │  /api/builder/* ─► driver  ┌──────────────────────────────────────────┐
   │  /api/editor/* ─► driver   │             Ruby Adapter                  │
   │                            │                                          │
   │                            │  client.rb (MOP client)                  │
   │                            │    ├─ area_loader.rb (load game world)   │
   │                            │    ├─ stdlib/ (game objects, commands)    │
   │                            │    └─ portal/ (Roda web apps)            │
   └──── proxy ────────────────►│         ├─ editor UI, git, builder UI    │
                                │         ├─ account, play                  │
                                │         └─ review (merge requests)        │
                                └──────────────────────────────────────────┘
```

**HTTP flow**: Axum handles `/project/*` (built SPA/template assets), `/api/builder/*` (build logs), `/api/editor/*` (file CRUD), `/api/ai/*`, and HTTP git directly. Portal pages and area API routes (`web_routes`/`web_app`) are reverse-proxied to the Ruby portal (Puma/Roda) over a Unix socket.

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

**MOP RPC from Rust to Ruby** (`MopRpcClient`):

The driver also initiates RPC calls to the adapter for web-related decisions:
- `CheckBuilderAccess` — ask the adapter whether a user has access to a given area (used by `/project/*` and `/api/builder/*` routes)
- `GetWebData` — request template data from the area's `web_data` block (used for Tera template rendering)

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
| `ai_api_keys` | Encrypted API keys (AES-GCM) for AI providers, with per-provider `enabled` toggle |
| `ai_custom_providers` | User-defined self-hosted LLM endpoints (name, base_url, api_mode, encrypted key, enabled) |

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

Operations: `checkout` (clone both directories), `pull` (fetch + hard reset), `commit` (stage all + push to origin), `log`, `diff`, `branches`, `create_branch`, `checkout_branch` (switch `@dev` to a different branch for viewing in the editor).

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
- `RepoManager` / `Workspace` — git operations (checkout, pull, commit, branch switching)
- `MergeRequestManager` — MR lifecycle
- `BuildManager` — SPA building (npm install + vite build), Tera template rendering, build log storage
- `MopRpcClient` — driver-initiated RPC to the adapter (access checks, template data)

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
- `/project/<ns>/<area>/*` — serve built SPA assets or Tera-rendered templates
- `/api/builder/<ns>/<area>/logs` — build log API (query params: `limit`, `level`)
- `/api/editor/files/*` — editor file CRUD operations (read, write, list, delete)
- `/api/ai/models` — list AI models (filtered by enabled providers)
- `/api/ai/stream` — SSE streaming chat (supports `provider: "custom:<id>"`)
- `/api/ai/apikey` — manage encrypted API keys (GET status, POST save, DELETE remove)
- `/api/ai/provider/toggle` — enable/disable built-in providers
- `/api/ai/custom-provider` — CRUD for self-hosted LLM endpoints
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

The editor's AI panel streams through the driver, which proxies to the selected provider. Built-in providers can be individually enabled/disabled. Users can also add custom self-hosted LLM endpoints (up to 5 per user) with configurable API modes (`ollama`, `openai`, `anthropic`). Custom providers reuse existing adapter code with a different `base_url`; Ollama uses the OpenAI-compatible adapter since its `/v1/chat/completions` endpoint follows the same format.

---

## Ruby Adapter Internals

### Client (`client.rb`)

MOP protocol client. Connects to the driver Unix socket, sends/receives length-prefixed MessagePack frames. Provides `send_driver_request()` for synchronous RPC (blocks on a response queue with timeout).

### Area Loading (`area_loader.rb`)

1. Receives `LoadArea` message with path and area_id
2. Creates `Stdlib::World::Area` instance
3. Area evaluates `mud_aliases.rb` (class aliases), `mud_loader.rb` (directory mappings), then loads all `.rb` files from mapped directories
4. Connects area database (if provisioned), runs Sequel migrations
5. Sends `AreaLoaded` or `AreaError` back to driver

After `AreaLoaded`, the Rust driver's `BuildManager` triggers SPA builds if the area declares `web_mode :spa` (npm install + vite build). Builds are also triggered by git push (SSH/HTTP) and workspace commits.

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
| `EditorApp` | `/editor` | Editor page UI (file ops handled by driver at `/api/editor/*`) |
| `GitApp` | `/git` | Git dashboard, branches, commits, MRs |
| `ReviewApp` | `/review` | Merge request review UI |
| `BuilderApp` | `/builder` | Builder UI tools; area API routes (`web_routes`/`web_app` blocks); caches `WebDataDSL` configs and Rack apps per area (invalidated by generation + file mtime) |

`BaseApp` provides shared helpers: `require_login!`, `current_account`, `mop_client`, `area_loader`, `render_view`.

### RackApp Base Class (`stdlib/web/rack_app.rb`)

Roda subclass that areas can extend for custom web APIs. Provides `area_db` access (wired by BuilderApp from the container) and JSON/all-verbs plugins. Subclasses are auto-detected during `WebDataDSL` evaluation — if a new RackApp subclass is defined in `mud_web.rb`, the DSL auto-sets the `app_block`.

### Area Web Content (Driver-Hosted)

Areas serve web content at `/project/<ns>/<area>/`, hosted directly by the Rust driver. Configured in `mud_web.rb`:

**Tera template mode** (default): The driver renders Tera templates from `web/templates/` with data provided by the adapter via the `GetWebData` MOP call (which invokes the area's `web_data` block).

**SPA mode**: Vite-built JS app from `web/src/`. The driver's `BuildManager` handles npm install + vite build (with correct `--base` URL) and `window.__MUD__` injection. Builds are triggered by git push (SSH/HTTP) and workspace commits.

**Rack app mode** (`web_app` block): Mount any Rack-compatible app (subclass `MUD::Stdlib::Web::RackApp`) for the API backend, served via the Ruby adapter. In SPA mode, only `/api/*` routes reach the Rack app; everything else is served by the driver's SPA frontend. API routes never fall through to static file serving — if Ruby returns 404 for `/api/*`, the driver returns 404 directly.

**Error logging**: 5xx responses from the Ruby portal proxy are automatically logged to the area's build log, making them visible in the editor's log panel. Ruby-side errors in `web_app` and `web_routes` blocks are caught, logged to stderr, and returned as 500 JSON responses (with cached app invalidation on error).

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
    ├── templates/       # Tera templates (template mode)
    │   └── index.html   # Main template
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

### Unit & Integration Tests

Fast tests in `crates/mud-driver/tests/` that don't need the full stack (all 192+ lib tests and 91+ integration tests run without `#[ignore]`):
- **testcontainers**: Spins up PostgreSQL in Docker per test (e.g., `account_auth_test.rs`, `http_git_test.rs`)
- **In-memory**: Config parsing, MOP codec, build log, adapter manager handshake
- **Tempdir**: Git operations, workspace (checkout, pull, commit, branch switching), editor file CRUD

### End-to-End Tests (`crates/mud-e2e/`)

A dedicated crate that runs the full application stack inside Docker containers. Each test file boots its own isolated pair of containers (PostgreSQL + mud-driver with Ruby adapter) via testcontainers, and interacts only via HTTP — no mocks.

**Infrastructure:**
- `Dockerfile.e2e` — multi-stage build: Rust binary compilation → Ruby 3.4 runtime with vendored gems
- `TestServer` harness — creates a Docker network, boots PG + mud-driver containers, generates config, polls for readiness, provides cookie-enabled HTTP client

**Test suites:**

| Test file | What it covers |
|-----------|---------------|
| `account_lifecycle.rs` | Register, login, logout, bad credentials, duplicate registration |
| `git_workflow.rs` | Repos, branches, editor, commits, code reload, cross-user access control |
| `editor_operations.rs` | File CRUD (create, read, update, delete) |
| `builder_webapp.rs` | Tera templates, @dev branch access control, hot reload, Rack /api endpoints |
| `stdlib_lifecycle.rs` | ERB template rendering for portal pages |
| `game_session.rs` | Play start, movement, commands (look, help, who), auth gating |
| `access_control.rs` | Unauthenticated redirects, cross-user isolation, @dev restrictions |
| `webapp_database.rs` | Sequel migrations, area DB provisioning, CRUD API |
| `ai_streaming.rs` | AI streaming via wiremock mock, provider toggle, custom provider CRUD |
| `spa_build.rs` | SPA build pipeline (npm install + vite build) |

Run with: `just test-e2e` or `cargo test -p mud-e2e`

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
  encryption_key: "32-byte-hex-key-for-aes-gcm"
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

**Rust**: tokio, axum, sqlx (PostgreSQL), git2, russh, rmp-serde, bcrypt, aes-gcm, reqwest, tera (templates), tracing

**Ruby**: roda 3, rack 3, puma 6, msgpack 1.7, sequel, bcrypt
