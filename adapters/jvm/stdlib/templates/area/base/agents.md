# {{area_name}} -- Area Development Guide

## Project Structure

This is a MUD area built with the JVM adapter. It uses annotations for
game object discovery and Gradle for building.

## Annotations

- `@MudRoom` -- Marks a class as a room. Must extend `Room()`.
- `@MudNPC` -- Marks a class as an NPC. Must extend `NPC()`.
- `@MudItem` -- Marks a class as an item. Must extend `Item()`.
- `@MudDaemon` -- Marks a class as a background daemon. Must extend `Daemon()`.
- `@MudArea` -- Marks the area entry point (one per area). Must extend `Area()`.
- `@WebData` -- Marks a method that returns `Map<String, Any>` for Tera templates.

## Base Classes

- `Room` -- Has `name`, `description`, `exit(direction, to)`, `onEnter(player)`
- `Item` -- Has `name`, `description`, `portable`, `onUse(player, target)`
- `NPC` -- Has `name`, `description`, `location`, `onTalk(player)`
- `Daemon` -- Has `name`, `tick()` called periodically
- `Area` -- Has `rooms`, `items`, `npcs`, `daemons`, `name`, `namespace`

## Configuration (mud.yaml)

```yaml
framework: none | ktor | spring-boot | quarkus
web_mode: template | spa | static
entry_class: MudArea
```

## Web Modes

### Template Mode (default)
- Place HTML files in `web/templates/`
- Uses Tera template engine (Jinja2-like syntax)
- Data comes from `@WebData` method: `{{ variable_name }}`
- The driver renders templates server-side

### SPA Mode
- Place frontend source in `web/src/` with `package.json`
- Set `web_mode: spa` in `mud.yaml`
- Driver builds with Vite and serves from `dist/`
- Define API routes using your framework (Ktor/Spring Boot/Quarkus)

### Static Mode
- Place files in `web/`
- Served as-is by the driver

## Database

- Place Flyway migrations in `db/migrations/`
- Named `V1__description.sql`, `V2__next.sql`, etc.
- Migrations run automatically on area load
- Connection URL provided via `MUD_DB_URL` environment variable

## Building

Run `./gradlew build` or `./gradlew shadowJar` for a fat JAR.
The adapter builds automatically on area load and git push.
