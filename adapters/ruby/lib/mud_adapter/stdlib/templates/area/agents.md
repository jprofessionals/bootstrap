# {{area_name}} -- Platform API Reference

Area owned by **{{namespace}}**.

This document describes the MUD driver platform APIs available to code in this area repository. Use it as a reference when writing rooms, items, NPCs, daemons, web pages, and network services.

---

## File Structure

```
.
├── .gitignore           # Excludes .mud/ working directory from git
├── .meta.yml            # Area metadata (status, owner)
├── mud_aliases.rb       # Class aliases (Room, Item, NPC, Daemon)
├── mud_loader.rb        # Directory-to-type mappings
├── mud_web.rb           # Web mode config, data, and routes
├── mud_network.rb       # TCP listener declarations
├── agents.md            # This file (platform API reference)
├── rooms/               # Room subclasses (one class per file)
├── items/               # Item subclasses
├── npcs/                # NPC subclasses
├── daemons/             # Daemon subclasses
├── db/
│   └── migrations/      # Sequel migrations (run on load and reload)
└── web/                 # Web assets (ERB templates or SPA source)
```

Files are auto-loaded by the driver based on `mud_loader.rb`. Each `.rb` file in a mapped directory must define exactly one class whose name matches the filename (snake_case to PascalCase). Pushing to the git remote triggers a hot reload.

---

## Git Access

Clone and push via the portal's HTTP git endpoint using your account credentials (password or personal access token):

```
git clone http://<username>:<password>@<host>:<port>/git/{{namespace}}/{{area_name}}.git
```

Pushing to the remote triggers a hot reload of the area.

---

## Base Class: GameObject

All DSL classes inherit from `GameObject`. It provides:

| Method | Context | Description |
|---|---|---|
| `title(value)` | class | Set the display title |
| `title` | class/instance | Get the display title |
| `description(value)` | class | Set the description text |
| `description` | class/instance | Get the description text |

---

## Room

Rooms are locations players can visit and move between.

**Class methods** (called in the class body):

| Method | Description |
|---|---|
| `title(value)` | Set the room title |
| `description(value)` | Set the room description |
| `exit(direction, to:)` | Add an exit -- `direction` is a symbol, `to:` is the target room key (string) |
| `exits` | Returns the exits Hash (`{ direction => room_key }`) |

**Instance methods:**

| Method | Description |
|---|---|
| `exits` | Returns the exits Hash |
| `exit_directions` | Returns an Array of exit direction symbols |
| `has_exit?(direction)` | Returns true if the given exit exists |

**Hook:** `on_enter(player)` -- called when a player enters the room.

**Example** (`rooms/town_square.rb`):

```ruby
class TownSquare < Room
  title "Town Square"
  description "A bustling square at the heart of town."

  exit :north, to: "market"
  exit :east,  to: "tavern"

  def on_enter(player)
    player.send_message("You hear the murmur of the crowd.")
  end
end
```

---

## Item

Items are objects that exist in the world. They can be portable (carryable) or fixed.

**Class methods:**

| Method | Description |
|---|---|
| `title(value)` | Set the item title |
| `description(value)` | Set the item description |
| `portable(bool)` | Mark the item as portable (default: false) |

**Instance methods:**

| Method | Description |
|---|---|
| `portable?` | Returns true if the item is portable |

**Hook:** `on_use(player, target)` -- called when a player uses the item.

**Example** (`items/healing_potion.rb`):

```ruby
class HealingPotion < Item
  title "Healing Potion"
  description "A small vial of glowing red liquid."
  portable true

  def on_use(player, target)
    player.send_message("You drink the potion and feel restored.")
  end
end
```

---

## NPC

NPCs are non-player characters that can be placed in rooms.

**Class methods:**

| Method | Description |
|---|---|
| `title(value)` | Set the NPC name |
| `description(value)` | Set the NPC description |
| `location(room_key)` | Set the room key where this NPC resides |

**Instance methods:**

| Method | Description |
|---|---|
| `location` | Returns the room key string |

**Hook:** `on_talk(player)` -- called when a player talks to the NPC.

**Example** (`npcs/blacksmith.rb`):

```ruby
class Blacksmith < NPC
  title "Grumpy Blacksmith"
  description "A soot-covered dwarf hammering at an anvil."
  location "market"

  def on_talk(player)
    player.send_message("'What do ye want? I'm busy!'")
  end
end
```

---

## Daemon

Daemons are background services that run on a tick interval.

**Class methods:**

| Method | Description |
|---|---|
| `title(value)` | Set the daemon title |
| `description(value)` | Set the daemon description |
| `interval(seconds)` | Set the tick interval in seconds (default: 60) |

**Instance methods:**

| Method | Description |
|---|---|
| `interval` | Returns the interval in seconds |

**Hook:** `on_tick` -- called every interval.

**Example** (`daemons/weather.rb`):

```ruby
class Weather < Daemon
  title "Weather Daemon"
  description "Cycles the weather every 5 minutes."
  interval 300

  def on_tick
    # Rotate weather conditions, notify players, etc.
  end
end
```

---

## Web Modes

Each area can serve web content at `/builder/{{area_name}}/`. The mode is set in `mud_web.rb`.

### ERB Mode (default)

Serves ERB templates from `web/`. Template locals are supplied via `web_data`.

**URL:** `http://<host>:<port>/builder/{{area_name}}/`

**`mud_web.rb` example:**

```ruby
web_data do |area, helpers|
  {
    area_name: File.basename(area.path),
    room_count: area.rooms.size,
    server_name: helpers.server_name,
    players_online: helpers.total_players_online
  }
end
```

**`web_data` block parameters:**

| Parameter | Description |
|---|---|
| `area` | The Area object (`.rooms`, `.items`, `.npcs`, `.daemons`, `.path`) |
| `helpers` | Web helpers (`.server_name`, `.total_players_online`) |

The returned Hash becomes template locals in your ERB files.

### SPA Mode

Serves a single-page application built with Vite. The driver runs `npm install` and `vite build` automatically on area load.

**Page URL:** `http://<host>:<port>/builder/{{namespace}}/{{area_name}}/`

**`mud_web.rb` example:**

```ruby
web_mode :spa
```

SPA source files go in `web/src/` (includes `index.html`, `main.js`, `vite.config.js`, `package.json`).

**Build details:**

The driver copies `web/src/` into a build directory, runs `npm install` and `vite build` with the correct base URL. This means:

- Asset paths are handled automatically — the platform sets `--base /builder/{{namespace}}/{{area_name}}/` so all built assets resolve correctly
- In `index.html`, use relative paths: `src="./main.js"` (Vite rewrites these during build)
- The `MUD_BASE_URL` environment variable is available in `vite.config.js` for custom builds
- Any npm dependencies must be listed in `package.json`
- The build runs once on first request after area load; a git push triggers a rebuild

**API calls from JavaScript:**

The platform injects `window.__MUD__` into the built `index.html` with the area's base URL. Use the `mud` helper (included in the template) for API calls:

```javascript
// mud.getJson() resolves paths relative to the area mount point
const todos = await mud.getJson('api/todos');
const result = await mud.postJson('api/todos', { text: 'Buy milk' });
```

The `mud` helper methods:

| Method | Description |
|--------|-------------|
| `mud.baseUrl` | The area's mount path (e.g., `/builder/ns/area/`) |
| `mud.fetch(path, options)` | Like `fetch()` but resolves `path` against `baseUrl`. Throws on non-OK responses. |
| `mud.getJson(path)` | GET request, returns parsed JSON |
| `mud.postJson(path, body)` | POST with JSON body, returns parsed JSON |

**Client-side routing:**

All non-file paths serve `index.html` (catch-all for client-side routing). Pass `window.__MUD__.baseUrl` to your router:

```javascript
// Svelte Router
<Router basepath={window.__MUD__?.baseUrl || '/'}>

// React Router
<BrowserRouter basename={window.__MUD__?.baseUrl}>
```

Deep links like `/builder/{{namespace}}/{{area_name}}/settings/profile` work out of the box. The SPA fallback only applies after the `web_app` Rack handler (if defined) returns 404.

### Rack App Mode

Mount a full Rack-compatible app for your backend. The `web_app` block receives `work_path` (the area directory) and must return a Rack app — any object responding to `call(env)`.

**In SPA mode:** Only `/api/*` requests are forwarded to the Rack app. All other paths are served by the SPA frontend (JS). This means the SPA owns all non-API routes — you don't need to handle the fall-through yourself.

**In ERB mode:** All requests hit the Rack app first. If it returns a 404 status, the request falls through to ERB template serving.

**`mud_web.rb` example (API + SPA):**

```ruby
web_mode :spa

web_app do |work_path|
  require 'json'

  db = MUD::Container["database.{{namespace}}/{{area_name}}"]
  todos = db[:todos]

  ->(env) {
    req = Rack::Request.new(env)
    case [req.request_method, req.path_info]
    when ['GET', '/api/todos']
      [200, { 'content-type' => 'application/json' }, [todos.order(:id).all.to_json]]
    when ['POST', '/api/todos']
      body = JSON.parse(req.body.read) rescue {}
      text = body['text']&.strip
      if text && !text.empty?
        id = todos.insert(text: text, completed: false)
        [200, { 'content-type' => 'application/json' }, [todos.where(id: id).first.to_json]]
      else
        [400, { 'content-type' => 'application/json' }, [{ error: 'Text required' }.to_json]]
      end
    else
      [404, {}, []]
    end
  }
end
```

**Key points:**

- The block runs once on first request; the returned app is cached and reused.
- Use `work_path` to `require` area code (items, daemons, etc.).
- You can use any Rack framework (Roda, Sinatra, raw lambda, etc.).
- `PATH_INFO` is relative to the area mount point — `/api/todos`, not `/builder/ns/area/api/todos`.
- In SPA mode, only `/api/*` routes reach the Rack app; the SPA serves everything else.
- In ERB mode, return `[404, {}, []]` from unmatched routes to fall through to ERB.
- The app is rebuilt on area hot-reload (git push).

### web_routes (legacy)

The simpler `web_routes` block is still supported for backward compatibility. It receives a Roda request object, the area, and the session. Return a Hash to auto-serialize as JSON.

```ruby
web_routes do |r, area, session|
  r.get "status" do
    { status: "ok", area: File.basename(area.path) }
  end
end
```

For new areas, prefer `web_app` — it gives you full control over request/response handling without DSL constraints.

---

## Database Access

Each area gets its own PostgreSQL database, auto-provisioned by the driver on area load. You do not need to create the database yourself — the driver handles creation, connection, and teardown.

**Access the database:**

```ruby
db = MUD::Container["database.{{namespace}}/{{area_name}}"]
```

This returns a `Sequel::Database` object. Use it for queries, inserts, schema operations, etc.

**Migrations** in `db/migrations/` are automatically applied on load and re-applied on reload. Migration failures are logged but do not prevent the area from loading.

**Migration example** (`db/migrations/001_create_scores.rb`):

```ruby
Sequel.migration do
  change do
    create_table(:scores) do
      primary_key :id
      String :player_name, null: false
      Integer :score, default: 0
      DateTime :created_at, default: Sequel::CURRENT_TIMESTAMP
    end
  end
end
```

**Query example:**

```ruby
db = MUD::Container["database.{{namespace}}/{{area_name}}"]
db[:scores].insert(player_name: "Gandalf", score: 42)
db[:scores].where(player_name: "Gandalf").first
```

---

## Build Logs

The driver captures detailed logs during area load and reload. These are stored in:

- **Per-area log:** `.mud/reload.log` in the area directory (JSONL format, one JSON object per line)
- **Master driver log:** Written by the Rust driver (receives all area logs)

**Events logged:**

| Event | Description |
|-------|-------------|
| `reload_start` | Area load/reload begins |
| `file_load` | Error loading a Ruby file (includes backtrace) |
| `migration` | Database migration result (success count or error) |
| `spa_build` | npm/vite build output (stdout/stderr captured) |
| `reload_end` | Summary: success or error count |

**Viewing logs:**

- **Builder UI:** Click the "Logs" tab in the editor to see recent reload history with color-coded entries and expandable backtraces
- **AI assistant:** The AI can call its `view_build_logs` tool to diagnose load failures
- **REST API:** `GET /builder/{{namespace}}/{{area_name}}/api/logs?limit=50&level=all`
  - `limit` — max entries (default 50, max 200)
  - `level` — filter: `all`, `error`, `warn` (default: all)
- **File:** Read `.mud/reload.log` directly (one JSON object per line)

The `.mud/` directory is excluded from git (see `.gitignore`) and does not trigger reloads.

---

## Networking

### Inbound: TCP Listeners

Declare TCP listeners in `mud_network.rb`. The driver manages the sockets; your handler class processes the data.

```ruby
# mud_network.rb
listen_tcp port: 2121, handler: "FtpHandler"
```

The `handler:` value is the class name (string) of a Daemon or other class in your area that implements the handler interface.

The handler class must implement:
- `on_connect(socket)` -- called when a client connects
- `on_data(socket, data)` -- called when data is received
- `on_disconnect(socket)` -- called when the client disconnects

### Outbound: HTTP Requests

The driver's network manager provides fiber-aware blocking HTTP methods. These are available to area code via the container.

```ruby
network = MUD::Container["driver.network"]

# GET
response = network.http_get!("https://api.example.com/data", headers: { "Accept" => "application/json" }, area: self)

# POST
response = network.http_post!("https://api.example.com/data", body: '{"key":"value"}', headers: { "Content-Type" => "application/json" }, area: self)

# PUT
response = network.http_put!("https://api.example.com/data", body: '{"key":"updated"}', headers: {}, area: self)

# DELETE
response = network.http_delete!("https://api.example.com/data/1", headers: {}, area: self)
```

All outbound requests are subject to the driver's host allow/blocklist and SSRF protection (private IPs are blocked).

---

## Available Gems

These gems are loaded by the driver and available to area code:

| Gem | Use |
|---|---|
| `sequel` | Database queries and migrations |
| `dry-types` | Type coercion and constraints |
| `dry-struct` | Typed value objects |
| `dry-monads` | `Success`/`Failure` result types |
| `dry-validation` | Input validation contracts |
| `dry-events` | Event pub/sub |
| `dry-schema` | Schema validation |
| `async` | Fiber-based concurrency |
| `bcrypt` | Password hashing |
| `erb` | Template rendering |
| `json` | JSON parsing (stdlib) |
| `yaml` | YAML parsing (stdlib) |

---

## Conventions

- **One class per file.** The filename must match the class name in snake_case (e.g., `town_square.rb` defines `TownSquare`).
- **Auto-loading.** The driver loads all `.rb` files from directories declared in `mud_loader.rb`. No manual `require` needed for game objects.
- **Hot reload.** Pushing to the area's git remote triggers an automatic reload. All rooms, items, NPCs, and daemons are re-evaluated and migrations are re-run.
- **Class-based DSL.** All game objects are plain Ruby classes inheriting from the appropriate base class. Configuration is done via class-level method calls (`title`, `description`, `exit`, etc.), and behavior is defined by overriding hook methods (`on_enter`, `on_tick`, etc.).
- **Aliases.** `mud_aliases.rb` maps short names (`Room`, `Item`, `NPC`, `Daemon`) to the full module paths. You can add your own aliases there.
- **Metadata.** `.meta.yml` controls area ownership. The `owner` field identifies the area creator.
