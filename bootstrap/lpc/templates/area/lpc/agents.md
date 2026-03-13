# {{area_name}} -- Platform API Reference (LPC)

Area owned by **{{namespace}}**.

This document describes the MUD driver platform APIs available to code in this area repository. Use it as a reference when writing rooms, items, NPCs, daemons, web routes, and SPA frontends. This template uses LPC for game objects and Rust `.so` modules for web routes and performance-critical daemons.

---

## File Structure

```
.
├── mud.yaml              # Area metadata (language, web_mode)
├── agents.md             # This file (platform API reference)
├── rooms/                # LPC room objects (.c files)
├── items/                # LPC item objects (.c files)
├── npcs/                 # LPC NPC objects (.c files)
├── daemons/              # LPC daemon objects (.c files)
├── web/
│   └── routes.rs         # Web API module (Rust .so, Axum Router fragment)
├── web-spa/
│   └── src/              # SPA source (index.html, main.js, vite.config.js, package.json)
└── db/
    └── migrations/       # SQL migrations (run on load and reload)
```

LPC `.c` files are loaded by the LPC VM. The `web/` directory uses Rust `.so` modules (same contract as the pure Rust template). Pushing to the git remote triggers a hot reload.

---

## Git Access

Clone and push via the portal's HTTP git endpoint using your account credentials (password or personal access token):

```
git clone http://<username>:<password>@<host>:<port>/git/{{namespace}}/{{area_name}}.git
```

Pushing to the remote triggers a hot reload of the area.

---

## Mixed Language Pattern

This template uses two languages for different purposes:

| Language | Used for | Files |
|---|---|---|
| **LPC** | Game objects -- rooms, items, NPCs, daemons | `.c` files in `rooms/`, `items/`, `npcs/`, `daemons/` |
| **Rust** | Web routes, performance-critical daemons | `.rs` files in `web/` (compiled to `.so`) |

LPC is the primary language for game logic. Rust is used where you need native performance or Axum web routing. Both are hot-reloaded on git push.

---

## LPC File Conventions

Every LPC source file uses the `.c` extension. Objects inherit from the `/std/` library and implement the `create()` lifecycle function, which the driver calls when the object is loaded.

```c
inherit "/std/room";

void create() {
    ::create();           // Call parent create
    set_short("Title");
    set_long("Description text.");
}
```

- `inherit` declares the parent object (similar to class inheritance).
- `::create()` calls the parent's `create()` function -- always call this first.
- The driver auto-loads `.c` files from mapped directories based on `mud.yaml`.

---

## The /std/ Library

The driver provides a standard library of base objects at `/std/`. Inherit from these in your area code.

### /std/room

Rooms are locations players can visit and move between.

| Function | Description |
|---|---|
| `set_short(string)` | Set the room's short title (shown in room headers and exits) |
| `set_long(string)` | Set the room's long description (shown on `look`) |
| `add_exit(string direction, string target)` | Add an exit in the given direction to the target room |

**Exit paths:** Use `"./rooms/target"` for rooms within the same area. The `./` prefix resolves relative to the area root.

**Example** (`rooms/entrance.c`):

```c
inherit "/std/room";

void create() {
    ::create();
    set_short("{{area_name}} Entrance");
    set_long("The entrance to {{area_name}}.");
    add_exit("north", "./rooms/hall");
}
```

**Example** (`rooms/hall.c`):

```c
inherit "/std/room";

void create() {
    ::create();
    set_short("{{area_name}} Hall");
    set_long("A grand hall within {{area_name}}.");
    add_exit("south", "./rooms/entrance");
}
```

### /std/item

Items are objects that exist in the world.

| Function | Description |
|---|---|
| `set_short(string)` | Set the item's short name |
| `set_long(string)` | Set the item's long description |
| `set_portable(int)` | Set whether the item can be picked up (1 = yes, 0 = no) |

**Hook:** `on_use(object player, object target)` -- called when a player uses the item.

**Example** (`items/lantern.c`):

```c
inherit "/std/item";

void create() {
    ::create();
    set_short("Brass Lantern");
    set_long("A battered brass lantern, still flickering with light.");
    set_portable(1);
}

void on_use(object player, object target) {
    send_message("The lantern flares brightly.\n");
}
```

### /std/npc

NPCs are non-player characters placed in rooms.

| Function | Description |
|---|---|
| `set_short(string)` | Set the NPC's name |
| `set_long(string)` | Set the NPC's description |
| `set_location(string)` | Set the room key where this NPC resides |

**Hook:** `on_talk(object player)` -- called when a player talks to the NPC.

**Example** (`npcs/blacksmith.c`):

```c
inherit "/std/npc";

void create() {
    ::create();
    set_short("Grumpy Blacksmith");
    set_long("A soot-covered dwarf hammering at an anvil.");
    set_location("./rooms/hall");
}

void on_talk(object player) {
    send_message("'What do ye want? I'm busy!'\n");
}
```

### /std/daemon

Daemons are background services. They do not need to inherit `/std/daemon` explicitly -- any `.c` file in `daemons/` is treated as a daemon.

**Example** (`daemons/area_daemon.c`):

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

---

## LPC Kfun Reference

The driver provides kernel functions (kfuns) organized by category. These are available to all LPC code.

### Type Operations

| Kfun | Description |
|---|---|
| `typeof(value)` | Return the type constant of a value |
| `instanceof(value, type)` | Check if a value is an instance of the given type |

Type constants: `T_NIL` (0), `T_INT` (1), `T_FLOAT` (2), `T_STRING` (3), `T_OBJECT` (4), `T_ARRAY` (5), `T_MAPPING` (6), `T_LWOBJECT` (7).

### String Operations

| Kfun | Description |
|---|---|
| `strlen(string)` | Return the length of a string |
| `explode(string, separator)` | Split a string into an array |
| `implode(array, separator)` | Join an array into a string |
| `lower_case(string)` | Convert to lowercase |
| `upper_case(string)` | Convert to uppercase |
| `sscanf(string, format, ...)` | Scan a string with a format pattern |

### Array Operations

| Kfun | Description |
|---|---|
| `allocate(size)` | Create an array of nil values |
| `allocate_int(size)` | Create an array of zeroes |
| `allocate_float(size)` | Create an array of 0.0 values |
| `sizeof(array)` | Return the number of elements |
| `sort_array(array, compare_fn)` | Sort an array using a comparison function |

### Mapping Operations

| Kfun | Description |
|---|---|
| `map_indices(mapping)` | Return an array of all keys |
| `map_values(mapping)` | Return an array of all values |
| `map_sizeof(mapping)` | Return the number of key-value pairs |
| `mkmapping(keys, values)` | Create a mapping from parallel arrays |

### Math Operations

| Kfun | Description |
|---|---|
| `fabs(x)`, `floor(x)`, `ceil(x)`, `sqrt(x)` | Basic math |
| `exp(x)`, `log(x)`, `log10(x)` | Exponential and logarithm |
| `sin(x)`, `cos(x)`, `tan(x)` | Trigonometry |
| `asin(x)`, `acos(x)`, `atan(x)` | Inverse trigonometry |
| `sinh(x)`, `cosh(x)`, `tanh(x)` | Hyperbolic |
| `pow(x, y)`, `fmod(x, y)`, `atan2(y, x)` | Two-argument math |
| `ldexp(x, exp)`, `frexp(x)`, `modf(x)` | Float decomposition |
| `random(max)` | Random integer in [0, max) |

### Object Management

| Kfun | Description |
|---|---|
| `this_object()` | Return the current object |
| `previous_object()` | Return the calling object |
| `clone_object(path)` | Create a clone of the named object |
| `new_object(path)` | Create a new instance of the named object |
| `destruct_object(obj)` | Destroy an object |
| `find_object(path)` | Find a loaded object by path |
| `object_name(obj)` | Return the name/path of an object |
| `function_object(func, obj)` | Find which object defines a function |
| `compile_object(path, ...)` | Compile an object from source |
| `this_user()` | Return the current user object |
| `call_other(obj, func, ...)` | Call a function in another object |
| `call_touch(obj)` | Mark an object as needing initialization |
| `previous_program()` | Return the program name of the caller |

### Timing and Scheduling

| Kfun | Description |
|---|---|
| `time()` | Current Unix timestamp (seconds) |
| `millitime()` | Current time with millisecond precision |
| `ctime(timestamp)` | Convert timestamp to human-readable string |
| `call_out(func, delay, ...)` | Schedule a delayed function call |
| `remove_call_out(handle)` | Cancel a scheduled call_out |

### File I/O

| Kfun | Description |
|---|---|
| `read_file(path, offset?, length?)` | Read file contents |
| `write_file(path, data, offset?)` | Write data to a file |
| `remove_file(path)` | Delete a file |
| `rename_file(from, to)` | Rename or move a file |
| `get_dir(pattern)` | List directory contents |
| `make_dir(path)` | Create a directory |
| `remove_dir(path)` | Remove a directory |

### Connection and Communication

| Kfun | Description |
|---|---|
| `send_message(message)` | Send a message to the current user |
| `users()` | Return an array of all connected user objects |
| `query_ip_number(obj)` | Return the IP address of a user |
| `query_ip_name(obj)` | Return the hostname of a user |
| `connect(address, port, ...)` | Initiate an outbound connection |
| `send_close()` | Close the current connection |
| `block_input(flag)` | Block or unblock input on the current connection |

### Serialization

| Kfun | Description |
|---|---|
| `save_object(path)` | Save object state to a file |
| `restore_object(path)` | Restore object state from a file |

### Crypto and Hashing

| Kfun | Description |
|---|---|
| `crypt(string, salt)` | Unix crypt hash |
| `hash_crc16(string, ...)` | CRC-16 checksum |
| `hash_crc32(string, ...)` | CRC-32 checksum |
| `hash_string(algo, string)` | Hash with named algorithm (e.g. "SHA256") |
| `encrypt(key, data, ...)` | Encrypt data |
| `decrypt(key, data, ...)` | Decrypt data |

### Miscellaneous

| Kfun | Description |
|---|---|
| `error(message)` | Throw a runtime error |
| `call_trace()` | Return the current call stack |
| `status(obj?)` | Return driver/object status information |

---

## Web Module (Rust .so)

The `web/` directory uses Rust `.so` modules -- the same contract as the pure Rust template. Web modules export Axum Router fragments mounted at `/project/{{namespace}}/{{area_name}}/`.

Every `.rs` file in `web/` must export `mud_module_init` with C ABI and use `register_router()`.

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

In SPA mode, only `/api/*` requests reach the Rust router. All other paths are served by the SPA frontend.

### Rust .so Module Contract (Summary)

For web modules and performance-critical Rust daemons in `daemons/`, the `.so` contract is:

| Method | Description |
|---|---|
| `set_path(path)` | Logical path of this module |
| `set_type(ModuleType)` | Module type: `Room`, `Item`, `NPC`, `Daemon`, or `Web` |
| `set_version(u64)` | Version number |
| `add_dependency(path)` | Declare a dependency on another module |
| `register_kfun(name, fn)` | Register a kernel function (non-web modules) |
| `register_router(fn)` | Register an Axum Router fragment (web modules) |

Cache policy attributes for Rust kfuns: `#[mud_kfun(cacheable)]`, `#[mud_kfun(volatile)]`, `#[mud_kfun(ttl = "30s")]`.

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

---

## Hot Reload

On git push, the driver:

1. Reloads changed LPC `.c` files (re-calls `create()` on affected objects)
2. Recompiles changed Rust `.rs` files to `.so` modules and hot-swaps them via `dlclose`/`dlopen`
3. Re-runs database migrations
4. Rebuilds the SPA if `web-spa/src/` changed

LPC objects are reloaded in-place. Rust modules are fully replaced.

---

## Build Logs

The driver captures detailed logs during area load and reload:

- **Per-area log:** `.mud/reload.log` (JSONL format)
- **REST API:** `GET /api/builder/{{namespace}}/{{area_name}}/logs?limit=50&level=all`

**Events logged:**

| Event | Description |
|---|---|
| `reload_start` | Area load/reload begins |
| `lpc_load` | LPC file load result (includes error backtraces) |
| `compile` | Rust compilation result (stdout/stderr captured) |
| `module_load` | Rust module load/unload via dlopen/dlclose |
| `migration` | Database migration result |
| `spa_build` | npm/vite build output |
| `reload_end` | Summary: success or error count |

The `.mud/` directory is excluded from git and does not trigger reloads.

---

## Conventions

- **LPC for game objects, Rust for web.** Rooms, items, NPCs, and simple daemons are written in LPC. Web routes and performance-critical code use Rust `.so` modules.
- **One object per file.** Each `.c` file defines one game object. Each `.rs` file compiles to one `.so` module.
- **Inherit from /std/.** All LPC game objects should inherit from the appropriate `/std/` base (`/std/room`, `/std/item`, `/std/npc`).
- **Local exit paths.** Use `add_exit("direction", "./rooms/target")` for exits within the area. The `./` prefix resolves relative to the area root.
- **Auto-loading.** Drop files in the right directory and push. No manifest or build configuration needed.
- **Hot reload.** Pushing to the area's git remote triggers an automatic reload of all changed files.
- **Template placeholders.** Use `{{area_name}}` and `{{namespace}}` in string literals -- these are substituted when the area is created from the template.
- **Metadata.** `mud.yaml` controls area language (`lpc`) and web mode (`spa` or `template`).
