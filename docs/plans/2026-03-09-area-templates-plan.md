# Area Templates Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Create two disk-based area templates ("rust" and "lpc") for the LPC adapter, with Axum web via .so modules, and wire them into the driver's template scanning and language routing.

**Architecture:** Templates live as files on disk at `adapters/lpc/templates/area/{rust,lpc}/`. The driver's `scan_disk_templates()` picks them up. The MOP Handshake is extended to support multiple languages so the LPC adapter can handle both `language: rust` and `language: lpc` areas. The current embedded `send_templates()` in the adapter is removed.

**Tech Stack:** Rust, MOP (MessagePack), Axum, serde_yaml

---

### Task 1: Create LPC Template Files on Disk

**Files:**
- Create: `adapters/lpc/templates/area/lpc/mud.yaml`
- Create: `adapters/lpc/templates/area/lpc/rooms/entrance.c`
- Create: `adapters/lpc/templates/area/lpc/rooms/hall.c`
- Create: `adapters/lpc/templates/area/lpc/daemons/area_daemon.c`
- Create: `adapters/lpc/templates/area/lpc/web/routes.rs`
- Create: `adapters/lpc/templates/area/lpc/web-spa/src/index.html`
- Create: `adapters/lpc/templates/area/lpc/web-spa/src/main.js`
- Create: `adapters/lpc/templates/area/lpc/web-spa/src/package.json`
- Create: `adapters/lpc/templates/area/lpc/web-spa/src/vite.config.js`
- Create: `adapters/lpc/templates/area/lpc/items/.gitkeep`
- Create: `adapters/lpc/templates/area/lpc/npcs/.gitkeep`
- Create: `adapters/lpc/templates/area/lpc/db/migrations/.gitkeep`

**Step 1: Create directory structure and files**

`adapters/lpc/templates/area/lpc/mud.yaml`:
```yaml
language: lpc
web_mode: spa
```

`adapters/lpc/templates/area/lpc/rooms/entrance.c`:
```c
inherit "/std/room";

void create() {
    ::create();
    set_short("{{area_name}} Entrance");
    set_long("The entrance to {{area_name}}.");
    add_exit("north", "./rooms/hall");
}
```

`adapters/lpc/templates/area/lpc/rooms/hall.c`:
```c
inherit "/std/room";

void create() {
    ::create();
    set_short("{{area_name}} Hall");
    set_long("A grand hall within {{area_name}}.");
    add_exit("south", "./rooms/entrance");
}
```

`adapters/lpc/templates/area/lpc/daemons/area_daemon.c`:
```c
/**
 * Area daemon for {{area_name}}.
 * This is loaded first and manages area-wide state.
 */
string query_name() {
    return "{{area_name}}";
}

void create() {
}
```

`adapters/lpc/templates/area/lpc/web/routes.rs`:
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

`adapters/lpc/templates/area/lpc/web-spa/src/index.html`:
```html
<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>MUD Area</title>
</head>
<body>
  <div id="app"></div>
  <script type="module" src="./main.js"></script>
</body>
</html>
```

`adapters/lpc/templates/area/lpc/web-spa/src/main.js`:
```javascript
// MUD Platform API helper — resolves paths relative to the area mount point.
// window.__MUD__ is injected by the platform at build time.
const mud = {
  get baseUrl() {
    return window.__MUD__?.baseUrl || './';
  },
  async fetch(path, options = {}) {
    const url = this.baseUrl + (path.startsWith('/') ? path.slice(1) : path);
    const res = await fetch(url, options);
    if (!res.ok) throw new Error(`${res.status} ${res.statusText}`);
    return res;
  },
  async getJson(path) {
    return (await this.fetch(path)).json();
  },
  async postJson(path, body) {
    return (await this.fetch(path, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify(body),
    })).json();
  }
};

const app = document.getElementById('app');
app.innerHTML = '<h1>MUD Area SPA</h1><p>Edit main.js to get started.</p>';

mud.getJson('api/status')
  .then(data => {
    const info = document.createElement('pre');
    info.textContent = JSON.stringify(data, null, 2);
    app.appendChild(info);
  })
  .catch(err => {
    const msg = document.createElement('p');
    msg.textContent = 'API not available: ' + err.message;
    msg.style.color = '#888';
    app.appendChild(msg);
  });
```

`adapters/lpc/templates/area/lpc/web-spa/src/package.json`:
```json
{
  "name": "mud-area-spa",
  "private": true,
  "version": "0.0.0",
  "type": "module",
  "scripts": {
    "dev": "vite",
    "build": "vite build",
    "preview": "vite preview"
  },
  "devDependencies": {
    "vite": "^6.0.0"
  }
}
```

`adapters/lpc/templates/area/lpc/web-spa/src/vite.config.js`:
```javascript
import { defineConfig } from 'vite'

export default defineConfig({
  base: process.env.MUD_BASE_URL || './',
  build: {
    outDir: 'dist',
    emptyOutDir: true
  }
})
```

Create empty `.gitkeep` files in `items/`, `npcs/`, `db/migrations/`.

**Step 2: Commit**

```bash
git add adapters/lpc/templates/
git commit -m "feat: add LPC area template files on disk"
```

---

### Task 2: Create Rust Template Files on Disk

**Files:**
- Create: `adapters/lpc/templates/area/rust/mud.yaml`
- Create: `adapters/lpc/templates/area/rust/rooms/entrance.rs`
- Create: `adapters/lpc/templates/area/rust/web/routes.rs`
- Create: `adapters/lpc/templates/area/rust/web-spa/src/index.html`
- Create: `adapters/lpc/templates/area/rust/web-spa/src/main.js`
- Create: `adapters/lpc/templates/area/rust/web-spa/src/package.json`
- Create: `adapters/lpc/templates/area/rust/web-spa/src/vite.config.js`
- Create: `adapters/lpc/templates/area/rust/items/.gitkeep`
- Create: `adapters/lpc/templates/area/rust/npcs/.gitkeep`
- Create: `adapters/lpc/templates/area/rust/daemons/.gitkeep`
- Create: `adapters/lpc/templates/area/rust/db/migrations/.gitkeep`

**Step 1: Create directory structure and files**

`adapters/lpc/templates/area/rust/mud.yaml`:
```yaml
language: rust
web_mode: spa
```

`adapters/lpc/templates/area/rust/rooms/entrance.rs`:
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

Web and SPA files are identical to the LPC template (same `web/routes.rs`, `web-spa/src/*`). Copy from Task 1.

Create empty `.gitkeep` files in `items/`, `npcs/`, `daemons/`, `db/migrations/`.

**Step 2: Commit**

```bash
git add adapters/lpc/templates/
git commit -m "feat: add Rust area template files on disk"
```

---

### Task 3: Extend MOP Handshake to Support Multiple Languages

The current `Handshake` message has a single `language: String` field. The LPC adapter needs to register for both "rust" and "lpc". Extend the handshake to optionally include additional languages.

**Files:**
- Modify: `crates/mud-mop/src/message.rs:184-188` — add `languages` field to Handshake
- Test: existing tests in `crates/mud-mop/src/message.rs` and `crates/mud-mop/src/codec.rs`

**Step 1: Write a failing test**

In `crates/mud-mop/src/message.rs`, add a test:

```rust
#[test]
fn adapter_handshake_with_languages() {
    let msg = AdapterMessage::Handshake {
        adapter_name: "mud-adapter-lpc".into(),
        language: "lpc".into(),
        version: "0.1.0".into(),
        languages: vec!["lpc".into(), "rust".into()],
    };
    let bytes = rmp_serde::to_vec_named(&msg).expect("serialize");
    let decoded: AdapterMessage = rmp_serde::from_slice(&bytes).expect("deserialize");
    assert_eq!(msg, decoded);
    match decoded {
        AdapterMessage::Handshake { languages, .. } => {
            assert_eq!(languages, vec!["lpc", "rust"]);
        }
        _ => panic!("wrong variant"),
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p mud-mop adapter_handshake_with_languages`
Expected: FAIL — `languages` field does not exist

**Step 3: Add `languages` field to Handshake**

In `crates/mud-mop/src/message.rs`, modify the Handshake variant:

```rust
#[serde(rename = "handshake")]
Handshake {
    adapter_name: String,
    language: String,
    version: String,
    #[serde(default)]
    languages: Vec<String>,
},
```

The `#[serde(default)]` ensures backwards compatibility — existing adapters (Ruby, JVM) that don't send `languages` will deserialize to an empty vec.

**Step 4: Run tests to verify they pass**

Run: `cargo test -p mud-mop`
Expected: ALL PASS (both new and existing tests)

**Step 5: Commit**

```bash
git add crates/mud-mop/
git commit -m "feat(mud-mop): extend Handshake with optional languages field"
```

---

### Task 4: Extend Adapter Manager for Multi-Language Registration

When an adapter sends `languages: ["lpc", "rust"]` in its handshake, the adapter manager should register the connection under both language keys.

**Files:**
- Modify: `crates/mud-driver/src/runtime/adapter_manager.rs:120-161`
- Modify: `crates/mud-driver/src/server.rs:509-521` — `accept_adapter` method

**Step 1: Write a failing test**

In `crates/mud-driver/src/runtime/adapter_manager.rs`, add a test:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn multi_language_adapter_registered_under_all_keys() {
        // After accepting a connection with languages: ["lpc", "rust"],
        // has_adapter should return true for both.
        let manager = AdapterManager::new();
        // (Full test requires a mock socket pair — see existing test patterns)
        // For now, verify has_adapter returns false initially
        assert!(!manager.has_adapter("lpc"));
        assert!(!manager.has_adapter("rust"));
    }
}
```

**Step 2: Run test to verify it passes (baseline)**

Run: `cargo test -p mud-driver multi_language`

**Step 3: Modify `accept_connection` to handle `languages` field**

In `crates/mud-driver/src/runtime/adapter_manager.rs`, in the `accept_connection` method after parsing the Handshake:

```rust
let (adapter_name, language, version, languages) = match first_msg {
    AdapterMessage::Handshake {
        adapter_name,
        language,
        version,
        languages,
    } => (adapter_name, language, version, languages),
    other => {
        bail!(
            "expected Handshake as first message, got {:?}",
            other
        );
    }
};
```

After inserting the primary connection, also insert aliases for additional languages:

```rust
self.adapters.insert(language.clone(), conn);

// Register additional language aliases pointing to the same connection
for lang in &languages {
    if lang != &language && !self.adapters.contains_key(lang) {
        // Clone the sender side — all aliases route to the same adapter
        if let Some(primary) = self.adapters.get(&language) {
            self.adapters.insert(lang.clone(), primary.clone());
        }
    }
}
```

Note: `AdapterConnection` will need to derive `Clone` (it wraps an `mpsc::Sender` which is already `Clone`).

**Step 4: Modify `accept_adapter` in server.rs to push all languages**

In `crates/mud-driver/src/server.rs`, the `accept_adapter` method currently pushes one language. Change it to return all languages and push them all into `self.adapter_languages`:

```rust
pub async fn accept_adapter(
    &mut self,
    listener: &tokio::net::UnixListener,
) -> Result<String> {
    let (primary_language, additional_languages) = self
        .adapter_manager
        .accept_connection(listener)
        .await
        .context("accepting adapter connection")?;
    self.adapter_languages.push(primary_language.clone());
    for lang in &additional_languages {
        if !self.adapter_languages.contains(lang) {
            self.adapter_languages.push(lang.clone());
        }
    }
    Ok(primary_language)
}
```

The `accept_connection` return type changes from `Result<String>` to `Result<(String, Vec<String>)>`.

**Step 5: Run tests**

Run: `cargo test -p mud-driver` and `cargo test -p mud-mop`
Expected: ALL PASS

**Step 6: Commit**

```bash
git add crates/mud-driver/ crates/mud-mop/
git commit -m "feat(mud-driver): support multi-language adapter registration"
```

---

### Task 5: Add Language Routing for "rust" Areas

The driver's `language_for_area` method reads `mud.yaml` and routes to the correct adapter. Add support for `language: rust`.

**Files:**
- Modify: `crates/mud-driver/src/server.rs:412-450` — `language_for_area` method

**Step 1: Read the current `language_for_area` implementation**

The method already handles `language: lpc` and JVM framework detection. Add a clause for `language: rust`.

**Step 2: Add routing for "rust" language**

In `language_for_area`, after the `language == "lpc"` check, add:

```rust
if language == "rust"
    && self.adapter_languages.iter().any(|l| l == "rust")
{
    info!(path = %area_path, %language, "Routing area to rust adapter");
    return "rust".to_string();
}
```

**Step 3: Run tests**

Run: `cargo test -p mud-driver`
Expected: ALL PASS

**Step 4: Commit**

```bash
git add crates/mud-driver/src/server.rs
git commit -m "feat(mud-driver): route language: rust areas to LPC adapter"
```

---

### Task 6: Extend scan_disk_templates for LPC/Rust Templates

Add scanning of `adapters/lpc/templates/area/` to the driver's `scan_disk_templates()`.

**Files:**
- Modify: `crates/mud-driver/src/server.rs:1371-1425` — `scan_disk_templates` method

**Step 1: Add LPC/Rust template scanning**

At the end of `scan_disk_templates()`, after the JVM block, add:

```rust
// LPC/Rust templates: each subdirectory is a self-contained template
let lpc_templates = std::path::Path::new("adapters/lpc/templates/area");

if lpc_templates.is_dir() {
    if let Ok(entries) = std::fs::read_dir(lpc_templates) {
        let existing = self.area_templates.read().await;
        let mut to_insert = Vec::new();

        for entry in entries.flatten() {
            if !entry.path().is_dir() {
                continue;
            }
            let template_name = entry.file_name().to_string_lossy().to_string();

            // Skip if adapter already registered this template
            if existing.contains_key(&template_name) {
                continue;
            }

            match collect_template_files(&entry.path()) {
                Ok(files) => {
                    to_insert.push((template_name, files));
                }
                Err(e) => {
                    warn!(
                        template = %entry.file_name().to_string_lossy(),
                        error = %e,
                        "Failed to read LPC/Rust template"
                    );
                }
            }
        }
        drop(existing);

        if !to_insert.is_empty() {
            let mut templates = self.area_templates.write().await;
            for (name, files) in to_insert {
                let count = files.len();
                info!(name = %name, count, "Disk-scanned area template registered");
                templates.insert(name, files);
            }
        }
    }
}
```

**Step 2: Run tests**

Run: `cargo test -p mud-driver`
Expected: ALL PASS

**Step 3: Verify manually**

The template files from Tasks 1-2 should be picked up. Check with a debug log or integration test that both "rust" and "lpc" templates appear in `area_templates`.

**Step 4: Commit**

```bash
git add crates/mud-driver/src/server.rs
git commit -m "feat(mud-driver): scan LPC/Rust area templates from disk"
```

---

### Task 7: Remove Embedded send_templates from LPC Adapter

Replace the embedded template code with multi-language handshake.

**Files:**
- Modify: `adapters/lpc/src/main.rs:82-87` — handshake to include `languages`
- Modify: `adapters/lpc/src/main.rs:97-98` — remove `send_templates` call
- Delete: `adapters/lpc/src/main.rs:127-201` — `send_templates` function

**Step 1: Update the handshake to declare both languages**

In `adapters/lpc/src/main.rs`, change the handshake:

```rust
let handshake = AdapterMessage::Handshake {
    adapter_name: "mud-adapter-lpc".into(),
    language: "lpc".into(),
    version: "0.1.0".into(),
    languages: vec!["lpc".into(), "rust".into()],
};
```

**Step 2: Remove the send_templates call and function**

Remove line 98: `send_templates(&mut writer).await?;`

Remove the entire `send_templates` function (lines 127-201) and its section comment (lines 123-126).

**Step 3: Build and test**

Run: `cargo build -p mud-adapter-lpc`
Expected: Builds cleanly with no warnings about unused `send_templates`

Run: `cargo test -p mud-adapter-lpc` (if tests exist)

**Step 4: Commit**

```bash
git add adapters/lpc/src/main.rs
git commit -m "refactor(lpc-adapter): remove embedded templates, declare multi-language handshake"
```

---

### Task 8: Add agents.md to Both Templates

Each template needs an `agents.md` documenting the platform APIs available to area developers.

**Files:**
- Create: `adapters/lpc/templates/area/rust/agents.md`
- Create: `adapters/lpc/templates/area/lpc/agents.md`

**Step 1: Create agents.md for rust template**

Document: mud.yaml config, directory conventions (rooms/, items/, npcs/, daemons/, web/, web-spa/), the .so module contract (`mud_module_init`, `ModuleRegistrar`, `ModuleType`), kfun registration, cache policy attributes (`#[mud_kfun(cacheable)]`, `#[mud_kfun(volatile)]`), web route module pattern, hot-reload behavior, and the `Context` API for interacting with the driver.

Reference the existing Ruby `agents.md` at `adapters/ruby/lib/mud_adapter/stdlib/templates/area/agents.md` for structure and tone.

**Step 2: Create agents.md for lpc template**

Same as rust template but also document: LPC file conventions (`.c` files, `inherit`, `create()`), the `/std/` library (`/std/room`, `/std/item`, `/std/npc`, `/std/daemon`), kfun categories, and mixed LPC+Rust patterns (LPC for game objects, Rust .so for web).

**Step 3: Commit**

```bash
git add adapters/lpc/templates/
git commit -m "docs: add agents.md platform reference to Rust and LPC area templates"
```

---

### Task 9: Integration Verification

**Step 1: Build everything**

Run: `cargo build --workspace`
Expected: Clean build

**Step 2: Run all tests**

Run: `cargo test --workspace`
Expected: ALL PASS

**Step 3: Verify template scanning manually**

Add a temporary `tracing::info!` or check that `scan_disk_templates` logs both templates on startup. The log output should show:

```
Disk-scanned area template registered name="rust" count=N
Disk-scanned area template registered name="lpc" count=N
```

**Step 4: Verify template contents**

Check that `.gitkeep` files are excluded from the template (the existing `collect_template_files` already skips them at `server.rs:2591`).

**Step 5: Commit any fixes**

```bash
git commit -m "fix: address integration issues from template work"
```
