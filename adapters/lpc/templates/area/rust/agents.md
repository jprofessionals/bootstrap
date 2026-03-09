# {{area_name}} -- Platform API Reference (Rust)

Area owned by **{{namespace}}**.

This document describes the MUD driver platform APIs available to code in this area repository. Use it as a reference when writing rooms, items, NPCs, daemons, web routes, and SPA frontends. All game objects in this template are written in Rust and compiled to `.so` modules.

---

## File Structure

```
.
├── mud.yaml              # Area metadata (language, web_mode)
├── agents.md             # This file (platform API reference)
├── rooms/                # Room modules (one .rs file per room)
├── items/                # Item modules
├── npcs/                 # NPC modules
├── daemons/              # Daemon modules (background services)
├── web/
│   └── routes.rs         # Web API module (Axum Router fragment)
├── web-spa/
│   └── src/              # SPA source (index.html, main.js, vite.config.js, package.json)
└── db/
    └── migrations/       # SQL migrations (run on load and reload)
```

Every `.rs` file in a mapped directory compiles to a `cdylib` `.so` module. The driver handles compilation automatically -- drop a `.rs` file in the right directory and push. Pushing to the git remote triggers a hot reload.

---

## Git Access

Clone and push via the portal's HTTP git endpoint using your account credentials (password or personal access token):

```
git clone http://<username>:<password>@<host>:<port>/git/{{namespace}}/{{area_name}}.git
```

Pushing to the remote triggers recompilation of changed `.rs` files and a hot reload of the area.

---

## The .so Module Contract

Every `.rs` file is compiled to a `cdylib` `.so` and must export a `mud_module_init` function with C ABI. The driver calls this function to discover what the module provides.

```rust
use mud_adapter_sdk::prelude::*;

#[no_mangle]
pub extern "C" fn mud_module_init(registrar: &mut ModuleRegistrar) {
    registrar.set_path("rooms/entrance");
    registrar.set_type(ModuleType::Room);
    registrar.set_version(1);
    registrar.add_dependency("/std/room");
    registrar.register_kfun("title", title);
    registrar.register_kfun("description", description);
    registrar.register_kfun("exits", exits);
}
```

The driver loads the `.so` via `dlopen`, calls `mud_module_init`, wires the registered kfuns into the runtime, and manages the module lifecycle.

---

## ModuleRegistrar API

These methods are called inside `mud_module_init` to declare the module's identity and capabilities.

| Method | Description |
|---|---|
| `set_path(path)` | Logical path of this module (e.g. `"rooms/entrance"`) |
| `set_type(ModuleType)` | Module type: `Room`, `Item`, `NPC`, `Daemon`, or `Web` |
| `set_version(u64)` | Version number (incremented on breaking changes) |
| `add_dependency(path)` | Declare a dependency on another module (e.g. `"/std/room"`) |
| `register_kfun(name, fn)` | Register a kernel function implemented by this module |
| `register_router(fn)` | Register an Axum Router fragment (Web modules only) |

---

## Module Types

### Room

Rooms are locations players can visit and move between. Register kfuns for `title`, `description`, and `exits`.

**Example** (`rooms/entrance.rs`):

```rust
use mud_adapter_sdk::prelude::*;

#[no_mangle]
pub extern "C" fn mud_module_init(registrar: &mut ModuleRegistrar) {
    registrar.set_path("rooms/entrance");
    registrar.set_type(ModuleType::Room);
    registrar.add_dependency("/std/room");
    registrar.register_kfun("title", title);
    registrar.register_kfun("description", description);
    registrar.register_kfun("exits", exits);
}

#[mud_kfun(cacheable)]
fn title(_ctx: &Context, _obj: ObjectId) -> String {
    "The Entrance".into()
}

#[mud_kfun(cacheable)]
fn description(_ctx: &Context, _obj: ObjectId) -> String {
    "Welcome to {{area_name}}. A passage leads north.".into()
}

#[mud_kfun(cacheable)]
fn exits(_ctx: &Context, _obj: ObjectId) -> Vec<Exit> {
    vec![Exit::new("north", "rooms/hall")]
}
```

### Item

Items are objects that exist in the world. Register kfuns for `title`, `description`, and optionally `portable` and `on_use`.

**Example** (`items/lantern.rs`):

```rust
#[no_mangle]
pub extern "C" fn mud_module_init(registrar: &mut ModuleRegistrar) {
    registrar.set_path("items/lantern");
    registrar.set_type(ModuleType::Item);
    registrar.register_kfun("title", title);
    registrar.register_kfun("description", description);
    registrar.register_kfun("portable", portable);
}

#[mud_kfun(cacheable)]
fn title(_ctx: &Context, _obj: ObjectId) -> String {
    "Brass Lantern".into()
}

#[mud_kfun(cacheable)]
fn portable(_ctx: &Context, _obj: ObjectId) -> bool {
    true
}
```

### NPC

NPCs are non-player characters placed in rooms. Register kfuns for `title`, `description`, `location`, and hooks like `on_talk`.

### Daemon

Daemons are background services. Register kfuns for `title`, `interval`, and `on_tick`.

**Example** (`daemons/weather.rs`):

```rust
#[no_mangle]
pub extern "C" fn mud_module_init(registrar: &mut ModuleRegistrar) {
    registrar.set_path("daemons/weather");
    registrar.set_type(ModuleType::Daemon);
    registrar.register_kfun("title", title);
    registrar.register_kfun("interval", interval);
    registrar.register_kfun("on_tick", on_tick);
}

#[mud_kfun(cacheable)]
fn interval(_ctx: &Context, _obj: ObjectId) -> u64 {
    300 // 5 minutes
}

#[mud_kfun(volatile)]
fn on_tick(ctx: &Context, _obj: ObjectId) {
    // Rotate weather, notify players, etc.
}
```

---

## Cache Policy Attributes

Control how kfun return values are cached by the driver. Apply these to kfun implementations.

| Attribute | Description |
|---|---|
| `#[mud_kfun(cacheable)]` | Return value is cached indefinitely (until module reload). Use for static data like titles and descriptions. |
| `#[mud_kfun(volatile)]` | Never cached -- called on every access. Use for dynamic state like `on_tick`. |
| `#[mud_kfun(ttl = "30s")]` | Cached for the specified duration. Use for data that changes infrequently (e.g. leaderboards). |

If no attribute is specified, the default is `volatile`.

---

## Web Module

Web modules export Axum Router fragments that the driver mounts at `/project/{{namespace}}/{{area_name}}/`. Use `register_router()` instead of `register_kfun()`.

**Example** (`web/routes.rs`):

```rust
use mud_adapter_sdk::prelude::*;
use axum::{Router, Json, routing::get};

#[no_mangle]
pub extern "C" fn mud_module_init(registrar: &mut ModuleRegistrar) {
    registrar.set_path("web/routes");
    registrar.set_type(ModuleType::Web);
    registrar.register_router(router);
}

fn router() -> Router<AppState> {
    Router::new()
        .route("/api/status", get(status))
}

async fn status() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "status": "ok" }))
}
```

In SPA mode (`web_mode: spa` in `mud.yaml`), only `/api/*` requests reach the Rust router. All other paths are served by the SPA frontend.

---

## SPA Frontend

SPA source files go in `web-spa/src/` (includes `index.html`, `main.js`, `vite.config.js`, `package.json`).

**Build details:**

The driver copies `web-spa/src/` into a build directory, runs `npm install` and `vite build` with the correct base URL. This means:

- Asset paths are handled automatically -- the platform sets `--base /project/{{namespace}}/{{area_name}}/`
- In `index.html`, use relative paths: `src="./main.js"`
- The `MUD_BASE_URL` environment variable is available in `vite.config.js`
- Builds are triggered by git push -- not on first request

**API calls from JavaScript:**

The platform injects `window.__MUD__` into the built `index.html`. Use the `mud` helper for API calls:

```javascript
const data = await mud.getJson('api/status');
const result = await mud.postJson('api/todos', { text: 'Buy milk' });
```

| Method | Description |
|---|---|
| `mud.baseUrl` | The area's mount path (e.g., `/project/ns/area/`) |
| `mud.fetch(path, options)` | Like `fetch()` but resolves `path` against `baseUrl` |
| `mud.getJson(path)` | GET request, returns parsed JSON |
| `mud.postJson(path, body)` | POST with JSON body, returns parsed JSON |

**Client-side routing:**

All non-file paths serve `index.html` (catch-all for client-side routing). Pass `window.__MUD__.baseUrl` to your router:

```javascript
// React Router
<BrowserRouter basename={window.__MUD__?.baseUrl}>

// Svelte Router
<Router basepath={window.__MUD__?.baseUrl || '/'}>
```

---

## Database Access

Each area gets its own PostgreSQL database, auto-provisioned by the driver on area load.

**Migrations** in `db/migrations/` are SQL files applied on load and re-applied on reload. Name them with a numeric prefix for ordering (e.g., `001_create_scores.sql`).

**Migration example** (`db/migrations/001_create_scores.sql`):

```sql
CREATE TABLE IF NOT EXISTS scores (
    id SERIAL PRIMARY KEY,
    player_name TEXT NOT NULL,
    score INTEGER DEFAULT 0,
    created_at TIMESTAMPTZ DEFAULT NOW()
);
```

Database access from Rust modules is available via the `Context` object passed to kfun calls, using `sqlx` queries.

---

## Hot Reload

On git push, the driver:

1. Detects changed `.rs` files
2. Recompiles them to `.so` modules (`cdylib` target)
3. Unloads the old module via `dlclose`
4. Loads the new module via `dlopen` and calls `mud_module_init`
5. Re-runs database migrations
6. Rebuilds the SPA if `web-spa/src/` changed

Only changed modules are recompiled. The driver keeps existing state for unchanged modules.

---

## Build Logs

The driver captures detailed logs during area load and reload:

- **Per-area log:** `.mud/reload.log` (JSONL format)
- **REST API:** `GET /api/builder/{{namespace}}/{{area_name}}/logs?limit=50&level=all`

**Events logged:**

| Event | Description |
|---|---|
| `reload_start` | Area load/reload begins |
| `compile` | Rust compilation result (stdout/stderr captured) |
| `module_load` | Module load/unload via dlopen/dlclose |
| `migration` | Database migration result |
| `spa_build` | npm/vite build output |
| `reload_end` | Summary: success or error count |

The `.mud/` directory is excluded from git and does not trigger reloads.

---

## Conventions

- **One module per file.** Each `.rs` file in a mapped directory compiles to a single `.so` module.
- **Convention-based compilation.** Drop a `.rs` file in `rooms/`, `items/`, `npcs/`, `daemons/`, or `web/` and it auto-compiles on push. No Cargo.toml needed -- the driver handles the build.
- **C ABI.** Every module must export `mud_module_init` with `#[no_mangle] pub extern "C"`. This is the only required export.
- **Hot reload.** Pushing to the area's git remote triggers recompilation and hot-swap of changed modules.
- **Template placeholders.** Use `{{area_name}}` and `{{namespace}}` in string literals -- these are substituted when the area is created from the template.
- **Metadata.** `mud.yaml` controls area language (`rust`) and web mode (`spa` or `template`).
