# JVM Adapter Design

## Overview

A Java/Kotlin adapter for the MUD driver that supports multiple JVM web frameworks
(Spring Boot, Ktor, Quarkus) with per-area framework choice, Gradle-based builds,
and full MOP protocol integration.

## Architecture

### Process Model

Unlike the Ruby adapter (single process, all areas), the JVM adapter uses a
**launcher + per-area child process** model:

```
Driver (Rust)
  |  MOP (one Unix socket)
  v
Launcher (thin JVM process)
  |  MOP (per-area Unix sockets)
  ├── Area A child JVM (e.g. Ktor)
  ├── Area B child JVM (e.g. Spring Boot)
  └── Area C child JVM (e.g. Quarkus)
```

**Launcher responsibilities:**
- Connects to the driver via MOP (single connection, same as Ruby adapter)
- Sends `set_area_template` with template files from stdlib on startup
- Routes MOP messages to the correct child process by area ID
- On `LoadArea`: triggers Gradle build, spawns child JVM process
- On `UnloadArea`: shuts down child JVM process
- Captures child stdout/stderr as fallback logging
- Health monitoring and restart of crashed children

**Child process responsibilities:**
- Connects to launcher via MOP on a per-area Unix socket
- Runs classpath scanning for annotated game objects
- Runs Flyway migrations from `db/migrations/`
- Handles `SessionStart`, `SessionInput`, `SessionEnd` for game logic
- Provides `@WebData` responses for Tera template rendering
- Optionally runs a framework web server for SPA mode API routes

### Why Per-Area Processes

Each area is its own Gradle project with potentially different frameworks and
dependency trees. Per-area isolation gives:

- No classloader complexity (no mini app server)
- Crash isolation between areas
- Areas can use different JVM versions or GraalVM native image
- Hot reload = restart one process
- Each area builds and runs as a normal Gradle project

The memory cost (~50-100MB per JVM) is less than classloader-isolated alternatives
in practice.

## MOP Protocol Integration

The JVM adapter uses the existing MOP protocol unchanged. The wire format is
length-prefixed MessagePack (4-byte big-endian u32 + payload, max 16MB).

### Message Flow

```
Driver ──MOP──> Launcher ──MOP──> Child (Area)
Driver <──MOP── Launcher <──MOP── Child (Area)
```

The launcher is a transparent message router. It adds no new message types.
All existing driver/adapter messages work as-is:

- `LoadArea / ReloadArea / UnloadArea` — launcher intercepts to manage child lifecycle
- `SessionStart / SessionInput / SessionEnd` — routed to correct child by area ID
- `GetWebData` — routed to child, response forwarded back
- `CheckBuilderAccess` — routed to child
- `DriverRequest / RequestResponse` — routed bidirectionally
- `Log` — forwarded from child to driver (area field already set)
- `Ping / Pong` — handled by launcher directly

### MOP Client Library (`mud-mop-jvm`)

Shared library used by both launcher and child processes:

- Unix socket connection management
- MessagePack serialization/deserialization (msgpack-java)
- Length-prefix framing
- Request/response correlation (blocking and async)
- Reconnection handling

## Game Object Model

### Annotations

Game objects are discovered via classpath scanning (no loader/aliases config files):

```kotlin
@MudRoom
class Entrance : Room() {
    override val name = "The Entrance"
    override val description = "A dimly lit entrance hall."

    override fun init() {
        exit("north", "rooms.hallway")
    }
}

@MudNPC
class Guard : NPC() {
    override val name = "A stern guard"
}

@MudItem
class Torch : Item() {
    override val name = "A flickering torch"
}

@MudDaemon
class WeatherDaemon : Daemon() {
    override fun tick() { /* periodic logic */ }
}
```

Available annotations:
- `@MudRoom` — room objects
- `@MudNPC` — non-player characters
- `@MudItem` — items/objects
- `@MudDaemon` — background services
- `@MudArea` — area entry point (one per area, on the main area class)
- `@WebData` — marks a method that returns template data for Tera rendering

### Area Entry Point

```kotlin
@MudArea(webMode = WebMode.TEMPLATE)
class MyArea : Area() {

    @WebData
    fun templateData(): Map<String, Any> {
        return mapOf(
            "room_count" to rooms.size,
            "area_name" to name
        )
    }
}
```

## Web Serving

Three modes, matching the existing Ruby adapter behavior:

### Template Mode (default)

- Area provides `web/templates/*.html` (Tera templates)
- Area provides a `@WebData` method returning `Map<String, Any>`
- Driver renders templates server-side using Tera engine
- Flow: HTTP request -> Driver -> `GetWebData` via MOP -> child returns data ->
  Driver renders Tera template -> HTTP response

### SPA Mode

- Area provides `web/src/` with frontend source (Vite project)
- Driver's `BuildManager` builds the SPA (npm install, vite build)
- Driver serves built static files from `dist/`
- Area's framework web server handles `/project/<ns>/<area>/api/*` routes
- Framework choice declared in `mud.yaml`, routes defined in pure framework code
- WebSocket endpoints supported natively by the chosen framework

In SPA mode, the child process starts the framework's embedded web server.
The launcher tells the driver the child's web socket path so the driver can
proxy API requests to it.

### Static Mode

- Area provides `web/` with static files
- Driver serves files directly, no adapter involvement

### Framework Web Servers (SPA mode only)

Each framework runs its own embedded server in the child process. No custom
annotations needed — areas use pure framework idioms:

**Ktor:**
```kotlin
fun main() {
    embeddedServer(Netty, port = mudPort) {
        routing {
            get("/api/status") { call.respond(mapOf("online" to true)) }
            webSocket("/api/live") { /* ... */ }
        }
    }.start()
}
```

**Spring Boot:**
```kotlin
@RestController
class ApiController {
    @GetMapping("/api/status")
    fun status() = mapOf("online" to true)
}
```

**Quarkus:**
```kotlin
@Path("/api")
class ApiResource {
    @GET @Path("/status")
    fun status() = mapOf("online" to true)
}
```

The launcher passes connection details (port, base path) as environment variables.

## Database & Migrations

### Flyway Integration

The MOP client library includes built-in Flyway support, independent of any
web framework:

- Driver provisions a per-area database and sends the URL via MOP (`LoadArea` with `db_url`)
- Launcher passes `db_url` to child process as environment variable
- On startup, the MOP client library runs Flyway migrations from `db/migrations/`
- Migrations run before any area initialization code

This works for all modes — template-mode areas without a framework still get
database support.

Framework apps (Spring Boot, Quarkus) can also use their own Flyway integration
if preferred, since they receive the same `db_url`.

### Migration Files

Standard Flyway SQL migrations:

```
db/migrations/
  V1__create_scores.sql
  V2__add_leaderboard.sql
```

## Logging

Two complementary layers ensure complete log capture:

### SLF4J Appender (structured logs)

The MOP client library includes a custom SLF4J appender that forwards log
messages via MOP:

```
Area code -> SLF4J -> MopLogAppender -> MOP Log message -> Launcher -> Driver
```

- All JVM frameworks use SLF4J (or bridge to it)
- Area developers use standard `logger.info("...")` calls
- Logs arrive at the driver as `Log { level, message, area }` messages
- Integrated into the driver's existing log infrastructure

### stdout/stderr Capture (fallback)

The launcher captures child process stdout/stderr for:
- JVM startup output (before MOP connects)
- Gradle build output
- Crash stack traces
- Native library output

These are forwarded to the driver as `Log` messages with appropriate level.

## Project Structure

### Adapter Layout

```
adapters/jvm/
├── build.gradle.kts                 # Root multi-project build
├── settings.gradle.kts
├── launcher/
│   ├── build.gradle.kts
│   └── src/main/kotlin/
│       └── launcher/
│           ├── Main.kt              # Entry point
│           ├── MopRouter.kt         # Message routing by area ID
│           ├── AreaProcessManager.kt # Child lifecycle management
│           └── GradleBuilder.kt     # Triggers area builds
├── mud-mop-jvm/
│   ├── build.gradle.kts
│   └── src/main/kotlin/
│       └── mop/
│           ├── MopClient.kt         # Protocol client
│           ├── MopCodec.kt          # MessagePack codec
│           ├── annotations/         # @MudRoom, @MudArea, @WebData, etc.
│           ├── runtime/
│           │   ├── AreaRuntime.kt    # Classpath scanner, area bootstrap
│           │   └── SessionManager.kt
│           ├── logging/
│           │   └── MopLogAppender.kt # SLF4J -> MOP bridge
│           └── migrations/
│               └── FlywayRunner.kt  # DB migration on area load
└── stdlib/
    ├── build.gradle.kts             # Base classes (Room, Item, NPC, etc.)
    ├── src/main/kotlin/
    │   └── world/
    │       ├── Area.kt
    │       ├── Room.kt
    │       ├── Item.kt
    │       ├── NPC.kt
    │       ├── Daemon.kt
    │       └── GameObject.kt
    └── templates/area/              # Area template (sent to driver)
        ├── build.gradle.kts
        ├── settings.gradle.kts
        ├── mud.yaml
        ├── agents.md
        ├── src/main/kotlin/
        │   ├── MudArea.kt
        │   └── rooms/
        │       └── Entrance.kt
        ├── web/
        │   └── templates/
        │       └── index.html
        └── db/
            └── migrations/
```

### Area Template Contents

**`mud.yaml`** — Area configuration:
```yaml
framework: none           # none | ktor | spring-boot | quarkus
web_mode: template         # template | spa | static
entry_class: MudArea
```

**`build.gradle.kts`** — Gradle build with MOP client dependency:
```kotlin
plugins {
    kotlin("jvm") version "2.1.0"
}

dependencies {
    implementation("mud:mud-mop-jvm:1.0.0")
    implementation("mud:mud-stdlib:1.0.0")
}
```

**`MudArea.kt`** — Entry point with web data:
```kotlin
@MudArea(webMode = WebMode.TEMPLATE)
class MudArea : Area() {

    @WebData
    fun templateData(): Map<String, Any> = mapOf(
        "area_name" to name,
        "room_count" to rooms.size
    )
}
```

**`rooms/Entrance.kt`** — Example room:
```kotlin
@MudRoom
class Entrance : Room() {
    override val name = "The Entrance"
    override val description = """
        You stand at the entrance of {{area_name}}.
        Stone walls rise around you, cool and damp.
    """.trimIndent()

    override fun init() {
        // exit("north", "rooms.hallway")
    }
}
```

**`web/templates/index.html`** — Example Tera template:
```html
<!DOCTYPE html>
<html>
<head><title>{{ area_name }}</title></head>
<body>
  <h1>Welcome to {{ area_name }}</h1>
  <p>This area has {{ room_count }} rooms.</p>
</body>
</html>
```

**`agents.md`** — AI coding guide for area development:

Documents the annotation system, base class APIs, directory conventions,
`mud.yaml` options, web modes, and migration patterns. Provides context
for AI coding assistants working on area code.

## Driver-Side Changes

Minimal changes needed on the Rust driver side:

1. **Adapter config** — add JVM adapter entry in `config/server.yml`:
   ```yaml
   adapters:
     jvm:
       enabled: true
       command: "java"
       adapter_path: "adapters/jvm/launcher/build/libs/launcher.jar"
       java_home: null  # optional, defaults to system
   ```

2. **Build integration** — the driver already handles SPA builds (npm/vite).
   Gradle builds are handled by the launcher, not the driver.

3. **Multi-adapter web proxying** — the driver may need to proxy API requests
   to JVM area web servers in addition to the Ruby portal socket. This requires
   per-area proxy routing rather than a single fallback proxy.

## Key Dependencies

| Component | Library | Purpose |
|---|---|---|
| MOP client | msgpack-java | MessagePack serialization |
| MOP client | junixsocket | Unix domain socket support |
| MOP client | kotlinx-coroutines | Async message handling |
| Migrations | flyway-core | Database migrations |
| Logging | slf4j-api | Logging facade |
| Classpath scan | classgraph | Annotation discovery |
| Launcher | kotlinx-coroutines | Process management |
| Area (Ktor) | ktor-server-netty | Ktor web server |
| Area (Spring) | spring-boot-starter-webflux | Spring WebFlux |
| Area (Quarkus) | quarkus-resteasy-reactive | Quarkus REST |

## Open Questions

- **GraalVM native image**: Quarkus areas could compile to native binaries for
  faster startup and lower memory. Worth supporting in the build pipeline?
- **Hot reload**: Can we support class reloading within a child process (e.g. via
  Spring DevTools or Ktor's auto-reload), or always restart the child?
- **Dependency caching**: Gradle downloads can be slow. Should the launcher
  maintain a shared Gradle cache across areas?
- **Area-to-area communication**: Should areas be able to communicate with each
  other directly, or only through the driver via MOP?
