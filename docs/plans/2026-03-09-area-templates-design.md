# Area Templates Design — Rust & LPC

## Overview

Two area templates for the LPC adapter, replacing the current embedded template in
`send_templates()`. Both use Axum for the web layer via `.so` modules. Both default to
SPA mode.

- **"rust"** — Pure Rust areas. Every game object is a `.rs` file compiled to a `.so`.
- **"lpc"** — LPC game objects (`.c` files) with a Rust web layer (`.so` modules).

The LPC adapter hosts both languages — it already has the LPC VM and gains a `.so`
loader.

## Template Storage

Templates live on disk at `adapters/lpc/templates/area/`:

```
adapters/lpc/templates/area/
├── rust/
│   ├── mud.yaml
│   ├── agents.md
│   ├── rooms/entrance.rs
│   ├── items/.gitkeep
│   ├── npcs/.gitkeep
│   ├── daemons/.gitkeep
│   ├── web/routes.rs
│   ├── db/migrations/.gitkeep
│   └── web-spa/src/
│       ├── index.html
│       ├── main.js
│       ├── package.json
│       └── vite.config.js
└── lpc/
    ├── mud.yaml
    ├── agents.md
    ├── rooms/entrance.c
    ├── rooms/hall.c
    ├── items/.gitkeep
    ├── npcs/.gitkeep
    ├── daemons/area_daemon.c
    ├── web/routes.rs
    ├── db/migrations/.gitkeep
    └── web-spa/src/
        ├── index.html
        ├── main.js
        ├── package.json
        └── vite.config.js
```

The driver's `scan_disk_templates()` is extended to scan this directory. Each
subdirectory becomes a template named after the directory (`"rust"`, `"lpc"`). No
base+overlay merging needed — each template is self-contained.

## `.so` Module Contract

Every `.rs` file in `rooms/`, `items/`, `npcs/`, `daemons/`, and `web/` compiles to a
cdylib `.so` with a C ABI entry point:

```rust
use mud_adapter_sdk::prelude::*;

#[no_mangle]
pub extern "C" fn mud_module_init(registrar: &mut ModuleRegistrar) {
    registrar.set_path("rooms/entrance");
    registrar.set_type(ModuleType::Room);
    registrar.set_version(env!("CARGO_PKG_VERSION"));
    registrar.add_dependency("/std/room");

    registrar.register_kfun("title", title);
    registrar.register_kfun("description", description);
    registrar.register_kfun("exits", exits);
    registrar.register_kfun("on_enter", on_enter);
}

#[mud_kfun(cacheable)]
fn title(_ctx: &Context, _obj: ObjectId) -> String {
    "The Entrance".into()
}

#[mud_kfun(cacheable)]
fn description(_ctx: &Context, _obj: ObjectId) -> String {
    "Welcome to the area.".into()
}

#[mud_kfun(cacheable)]
fn exits(_ctx: &Context, _obj: ObjectId) -> Vec<Exit> {
    vec![Exit::new("north", "rooms/hall")]
}

#[mud_kfun(volatile)]
fn on_enter(ctx: &Context, obj: ObjectId, player: ObjectId) {
    ctx.send_message(player, "You arrive at the entrance.");
}
```

### Web Modules

Web `.so` modules export Axum router fragments:

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

## Compilation

Convention-based discovery — the adapter scans directories and compiles each `.rs` file
to a cdylib `.so` without requiring manual registration in a `Cargo.toml`.

```
rustc --crate-type=cdylib --edition=2021 \
    -L dependency=.mud/deps \
    --extern mud_adapter_sdk=... \
    --extern axum=... \
    -o .mud/build/rooms/entrance.so \
    rooms/entrance.rs
```

Compiled artifacts go to `.mud/build/` within the area's work path (gitignored).

Shared dependencies (mud-adapter-sdk, axum, serde, etc.) are pre-compiled by the
adapter and cached in a shared location so individual module compilation is fast.

## Hot-Reload Flow

### Initial Load

1. Driver sends `LoadArea` to LPC adapter.
2. Adapter scans the area directory:
   - `rooms/*.rs`, `items/*.rs`, `npcs/*.rs`, `daemons/*.rs` — compile each to `.so`
   - `web/*.rs` — compile each to `.so`
   - `rooms/*.c`, `items/*.c`, etc. — compile via LPC VM (lpc language areas only)
3. Load all `.so` modules via `dlopen`, call `mud_module_init`.
4. Merge web router fragments, start Axum server on a Unix socket.
5. Register web socket with driver via `register_area_web`.
6. Confirm with `AreaLoaded`.

### On Git Push (Diff-Based)

1. Driver sends `ReloadProgram { area_id, path, files }` with changed files.
2. For each changed `.rs` file:
   - Recompile to `.so` in `.mud/build/`
   - `dlclose` old `.so`
   - `dlopen` new `.so`
   - Call `mud_module_init` to re-register
   - If web module: rebuild merged router, swap in Axum server
3. For each changed `.c` file:
   - Recompile via LPC VM
   - Fire `upgraded()` on dependents
4. Respond with `ProgramReloaded` or `ProgramReloadError`.

## Configuration

### mud.yaml

**Rust area:**
```yaml
language: rust
web_mode: spa
```

**LPC area:**
```yaml
language: lpc
web_mode: spa
```

### Adapter Config

The adapter declares both languages so the driver routes correctly:

```yaml
adapters:
  lpc:
    enabled: true
    command: "adapters/lpc/target/release/mud-adapter-lpc"
    adapter_path: "adapters/lpc"
    languages: ["rust", "lpc"]
```

## Driver Integration

### scan_disk_templates() Extension

The driver's `scan_disk_templates()` in `server.rs` is extended to scan
`adapters/lpc/templates/area/`. Each subdirectory becomes a template:

- `adapters/lpc/templates/area/rust/` → template name `"rust"`
- `adapters/lpc/templates/area/lpc/` → template name `"lpc"`

Files are collected recursively using the existing `collect_template_files()` helper.
Templates are only registered if not already provided by a running adapter.

### Adapter Language Routing

The driver routes areas to adapters based on `mud.yaml` language:

- `language: rust` → LPC adapter (hosts `.so` loader)
- `language: lpc` → LPC adapter (hosts LPC VM + `.so` loader)
- `language: ruby` → Ruby adapter
- `language: kotlin` → JVM adapter

### Removal of Embedded Templates

The current `send_templates()` function in `adapters/lpc/src/main.rs` is removed. The
adapter no longer sends templates via MOP — the driver reads them from disk.
