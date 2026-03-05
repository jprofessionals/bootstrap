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
└── web/                 # Web assets (Tera templates or SPA source)
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

Each area can serve web content at `/project/{{namespace}}/{{area_name}}/`. The mode is set in `mud_web.rb`. Web content is built and served directly by the Rust driver.

### Tera Template Mode (default)

Serves Tera templates from `web/templates/`. Template variables are supplied via the `web_data` block in `mud_web.rb` (the driver requests this data from the adapter via MOP).

**URL:** `http://<host>:<port>/project/{{namespace}}/{{area_name}}/`

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

The returned Hash becomes template variables in your Tera files. Place templates in `web/templates/` (e.g., `web/templates/index.html`). Tera uses `{{ variable }}` syntax for interpolation, `{% if %}` / `{% for %}` for control flow.

### SPA Mode

Serves a single-page application built with Vite. The driver's `BuildManager` runs `npm install` and `vite build` automatically, triggered by git push (SSH or HTTP) and workspace commits.

**Page URL:** `http://<host>:<port>/project/{{namespace}}/{{area_name}}/`

**`mud_web.rb` example:**

```ruby
web_mode :spa
```

SPA source files go in `web/src/` (includes `index.html`, `main.js`, `vite.config.js`, `package.json`).

**Build details:**

The driver copies `web/src/` into a build directory, runs `npm install` and `vite build` with the correct base URL. This means:

- Asset paths are handled automatically — the platform sets `--base /project/{{namespace}}/{{area_name}}/` so all built assets resolve correctly
- In `index.html`, use relative paths: `src="./main.js"` (Vite rewrites these during build)
- The `MUD_BASE_URL` environment variable is available in `vite.config.js` for custom builds
- Any npm dependencies must be listed in `package.json`
- Builds are triggered by git push (SSH/HTTP) and workspace commits — not on first request

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
| `mud.baseUrl` | The area's mount path (e.g., `/project/ns/area/`) |
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

Deep links like `/project/{{namespace}}/{{area_name}}/settings/profile` work out of the box. The SPA fallback only applies after the `web_app` Rack handler (if defined) returns 404.

### Rack App Mode

Mount a Rack-compatible app for your backend. The recommended approach is to subclass `MUD::Stdlib::Web::RackApp` (a Roda subclass) directly in `mud_web.rb`. The platform auto-detects the subclass and wires it up — no `web_app` block needed.

**In SPA mode:** Only `/api/*` requests are forwarded to the Rack app. All other paths are served by the driver's SPA frontend (JS). This means the SPA owns all non-API routes — you don't need to handle the fall-through yourself.

**In Tera template mode:** All requests hit the Rack app first. If it returns a 404 status, the request falls through to Tera template rendering.

**`mud_web.rb` example (RackApp subclass — recommended):**

```ruby
web_mode :spa

class TodoApi < MUD::Stdlib::Web::RackApp
  route do |r|
    r.on "api/todos" do
      todos = area_db[:todos]

      r.get true do
        todos.order(:id).all
      end

      r.post true do
        body = JSON.parse(r.body.read) rescue {}
        text = body['text']&.strip
        r.halt(400, { error: 'Text required' }.to_json) unless text && !text.empty?
        id = todos.insert(text: text, completed: false)
        todos.where(id: id).first
      end
    end
  end
end
```

**RackApp features:**

| Method | Description |
|--------|-------------|
| `area_db` | Returns the area's Sequel database (auto-wired by the platform) |
| `route` block | Standard Roda routing — return a Hash to auto-serialize as JSON |
| Roda plugins | `json` and `all_verbs` are pre-loaded |

**Alternative: raw `web_app` block:**

You can also use a `web_app` block that returns any Rack-callable object (lambda, Sinatra app, etc.):

```ruby
web_mode :spa

web_app do |work_path|
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

- The app is cached and reused after first request; rebuilt on area hot-reload (git push).
- `PATH_INFO` starts with `/api/` — e.g. `/api/todos`, not `/project/ns/area/api/todos`.
- In SPA mode, only `/api/*` routes reach the Rack app; the SPA serves everything else.
- In Tera template mode, return 404 from unmatched routes to fall through to template rendering.
- Errors in the Rack app are caught, logged to the build log, and returned as 500 JSON responses. The cached app is invalidated on error so the next request re-evaluates.

### web_routes

The simpler `web_routes` block provides a concise way to define area API routes. It receives a Roda request object, the area, and the session. Return a Hash to auto-serialize as JSON. Routes are served by the Ruby adapter.

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
| `web_app` | 5xx error from Rack app / web_routes (logged by proxy) |

**Viewing logs:**

- **Builder UI:** Click the "Logs" tab in the editor to see recent reload history with color-coded entries and expandable backtraces
- **AI assistant:** The AI can call its `view_build_logs` tool to diagnose load failures
- **REST API:** `GET /api/builder/{{namespace}}/{{area_name}}/logs?limit=50&level=all`
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
