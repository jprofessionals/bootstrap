# Rust/LPC Adapter Design

## Overview

A combined Rust and LPC adapter for the MUD driver. LPC (DGD-compatible) handles game
scripting — rooms, items, NPCs, daemons. Rust handles the web portal and
performance-critical game systems. The boundary between Rust and LPC is fluid and
configurable by the stdlib.

The adapter runs as a separate MOP process, consistent with Ruby and JVM adapters.

## Goals

- Full DGD LPC language parity (types, inheritance, object model, kfuns, preprocessor,
  `rlimits`, `atomic`, `parse_string`, hot-reload)
- 64-bit integers and 64-bit IEEE 754 floats across all languages
- Granular hot-reload of both LPC objects and Rust `.so` modules without game restart
- Seamless cross-language gameplay — players move between LPC, Ruby, and Kotlin areas
  without noticing language boundaries
- Full cross-area object interaction across all languages via the driver's object broker
- Diff-based reload on git push — only changed files are recompiled/reloaded
- Stdlib-defined hook system for cross-area event dispatch
- Cache policy system so adapters and the driver can avoid unnecessary round-trips

## Architecture

```
┌──────────────────────────────────────────────────────────────┐
│                       mud-driver-rs                          │
│                                                              │
│  ┌──────────────┐  ┌──────────────┐  ┌────────────────────┐ │
│  │ Object Broker │  │ State Store  │  │   Version Tree     │ │
│  │ (cross-lang   │  │ (properties, │  │ (dependency graph, │ │
│  │  method calls) │  │  ownership)  │  │  upgrades)         │ │
│  └──────┬────────┘  └──────┬───────┘  └────────┬───────────┘ │
│         │                  │                   │             │
│  ┌──────┴──────────────────┴───────────────────┴───────────┐ │
│  │                  AdapterRuntime trait                    │ │
│  └──┬──────────────────┬──────────────────┬────────────────┘ │
│     │                  │                  │                  │
│  ┌──┴──────┐    ┌──────┴──────┐    ┌──────┴──────┐          │
│  │   MOP   │    │    MOP      │    │    MOP      │          │
│  │ (socket)│    │  (socket)   │    │  (socket)   │          │
│  └──┬──────┘    └──────┬──────┘    └──────┴──────┘          │
└─────┼──────────────────┼──────────────────┼─────────────────┘
      │                  │                  │
┌─────┴──────┐    ┌──────┴──────┐    ┌──────┴──────┐
│  LPC/Rust  │    │    Ruby     │    │     JVM     │
│  Adapter   │    │   Adapter   │    │   Adapter   │
│            │    │             │    │             │
│ ┌────────┐ │    │             │    │             │
│ │ LPC VM │ │    │             │    │             │
│ └────────┘ │    │             │    │             │
│ ┌────────┐ │    │             │    │             │
│ │Rust .so│ │    │             │    │             │
│ │modules │ │    │             │    │             │
│ └────────┘ │    │             │    │             │
└────────────┘    └─────────────┘    └─────────────┘
```

All adapters communicate with the driver via MOP (MessagePack over Unix socket). The
driver manages the object broker, state store, and version tree. The LPC/Rust adapter
runs as a separate process — if it crashes, the driver stays up and can restart it.

## Unified Program Model

Every piece of reloadable code — regardless of language — is a **program** in the
driver's version tree.

| Type          | Source       | Compiled form       | Reload unit                  |
|---------------|--------------|---------------------|------------------------------|
| LPC object    | `.c` file    | VM bytecode         | Single file                  |
| Rust module   | `.rs` file(s)| `.so` dynamic lib   | One file or declared group   |
| Ruby object   | `.rb` file   | Interpreted by Ruby | Single file                  |
| Kotlin object | `.kt` file   | JVM class           | Single file / Gradle module  |

All programs share:

- **Path identity** — e.g., `/std/room`, `/areas/town/tavern`, `/portal/account`
- **Version number** — increments on each recompilation
- **Dependency list** — what this program inherits or uses
- **Upgrade callback** — `upgraded()` in LPC, equivalent hooks in other languages

## Driver State Store

The driver is the single source of truth for all object state. Adapters read and write
properties through the DriverServices trait (implemented via MOP).

```
ObjectId: 4827
├── program: "/areas/town/sword" (Ruby, version 3)
├── core_properties:             ← owned by the program's area
│   ├── title: "Iron Sword"
│   ├── damage: 12
│   └── weight: 3.5
├── attached_properties:         ← owned by other areas
│   ├── {source: "/areas/enchant", key: "fire_damage", value: 15}
│   ├── {source: "/areas/enchant", key: "on_hit_hook", value: HookRef(...)}
│   └── {source: "/areas/quest",   key: "quest_marker", value: "blade_of_fire"}
└── location: ObjectId(2001)     ← which room/container/player
```

Property rules:

- **Core properties** belong to the program — overwritten on upgrade, `upgraded()` can
  migrate them.
- **Attached properties** belong to their source area — preserved when the base program
  is upgraded.
- If the **source area** is upgraded, its attached properties get an `upgraded()` call.
- If a source area is unloaded, its attached properties are cleaned up or frozen
  (stdlib decides).

## Cross-Language Object Interaction

When LPC code calls `sword->get_description()` and the sword lives in a Ruby area:

```
LPC adapter                 Driver                      Ruby adapter
   │                          │                              │
   │ call_other(4827,         │                              │
   │   "get_description")     │                              │
   │─────────────────────────>│                              │
   │                          │  lookup ObjectId 4827        │
   │                          │  → program: Ruby area        │
   │                          │                              │
   │                          │  Call{object_id: 4827,       │
   │                          │    method: "get_description"}│
   │                          │─────────────────────────────>│
   │                          │                              │
   │                          │  CallResult{result: "A       │
   │                          │    gleaming iron sword",     │
   │                          │    cache: Cacheable}         │
   │                          │<─────────────────────────────│
   │                          │                              │
   │  return "A gleaming      │                              │
   │    iron sword"           │                              │
   │<─────────────────────────│                              │
```

### Type Marshalling

All cross-language values go through MessagePack:

| LPC type     | MessagePack    | Ruby          | Kotlin     | Rust       |
|--------------|----------------|---------------|------------|------------|
| `int`        | integer (64b)  | Integer       | Long       | i64        |
| `float`      | float (64b)    | Float         | Double     | f64        |
| `string`     | string         | String        | String     | String     |
| `mapping`    | map            | Hash          | Map        | HashMap    |
| `int*`       | array          | Array         | List       | Vec        |
| `object`     | ObjectId (ext) | ObjectId wrap | ObjectId   | ObjectId   |
| `nil`        | nil            | nil           | null       | None       |

Object references serialize as ObjectIds. The receiving side makes further `call_other`
calls through the driver's object broker.

## Cache Policy

All adapters declare cacheability on their methods. The driver caches results and avoids
unnecessary round-trips for stable data, while always forwarding volatile calls.

| Policy          | Behavior                                  | Examples                                  |
|-----------------|-------------------------------------------|-------------------------------------------|
| `cacheable`     | Cached locally, invalidated on change     | `title`, `description`, `exits`, `weight` |
| `volatile`      | Never cached, always round-trips          | Hook results, tick-dependent values       |
| `ttl(duration)` | Cached with expiry                        | Player online status, aggregate counts    |

**Default is `volatile`** — safe by default, opt into caching.

Declared per-language:

**LPC:**
```c
cacheable string get_description() { return description; }
volatile mixed on_hit(object target) { return dispatch_hooks("on_hit", target); }
```

**Ruby:**
```ruby
cacheable def description
  "A gleaming iron sword"
end

volatile def on_hit(target)
  dispatch_hooks("on_hit", target)
end
```

**Kotlin:**
```kotlin
@Cacheable
override fun getDescription() = "A cozy tavern"

@Volatile
override fun onEnter(player: ObjectId) = dispatchHooks("on_enter", player)
```

**Rust `.so`:**
```rust
#[mud_kfun(cacheable)]
fn get_title(ctx: &Context, obj: ObjectId) -> String { ... }

#[mud_kfun(volatile)]
fn dispatch_hooks(ctx: &Context, event: &str, target: ObjectId) -> Value { ... }
```

The `CallResult` MOP message includes the cache hint so the driver can store and enforce
the policy.

## Hook System

The hook system is **stdlib-defined**, not a driver primitive. It enables cross-area
event dispatch — an enchantment area can hook into a sword from another area without
the sword knowing about enchantments.

```c
// Enchantment area attaches to a sword:
sword->register_hook("on_hit", this_object(), "apply_fire_damage");

// Sword's base Item class dispatches hooks:
volatile mixed on_hit(object target) {
    do_damage(target, base_damage);
    dispatch_hooks("on_hit", target);  // cross-language via object broker
}
```

- Hook registration and dispatch are implemented in stdlib base classes
  (`/std/item.c`, `/std/room.c`, etc.)
- Hooks use `call_other` under the hood — the driver's object broker routes
  cross-language if needed
- Hooks are stored as attached properties in the state store — they survive program
  upgrades
- Areas can define new hook points in their own objects
- The stdlib defines standard events: `on_enter`, `on_leave`, `on_hit`, `on_use`,
  `on_tick`, `heartbeat`, etc.

## LPC VM

The LPC VM is a Rust crate (`lpc-vm`) with full DGD feature parity.

### Compilation Pipeline

```
.c source → Preprocessor → Parser → AST → Compiler → Bytecode → VM execution
```

### Core Components

- **Preprocessor** — `#include`, `#define`, `#ifdef`, `#pragma`, macro expansion
- **Parser** — C-like grammar producing AST
- **Compiler** — AST to bytecode, type checking (configurable: none/typed/strict)
- **Bytecode VM** — stack-based with tick counting and stack depth limits
- **Object table** — master objects, clones, light-weight objects, dependency graph
- **Kfun registry** — built-in DGD kfuns + stdlib-registered custom kfuns

### Object Lifecycle

- `compile_object(path)` — compile `.c` file, create master object, call `create()`
- `clone_object(master)` — create clone with unique ID (`master#1234`)
- `new_object(master)` — create light-weight object (`master#-1`), auto-deallocated
- `destruct_object(obj)` — destroy, remove from dependency graph

### Hot-Reload

- `compile_object()` on an already-compiled path replaces the program
- Version tree walks inheritors, calls `upgraded()` on each
- Clones get the new program but keep their state (via driver state store)
- `atomic` functions roll back on error — a failed upgrade does not corrupt state

### Kfun Categories

| Category         | Examples                                    | Implementation          |
|------------------|---------------------------------------------|-------------------------|
| Pure computation | `sizeof`, `typeof`, `explode`, `implode`    | VM-internal             |
| Object mgmt      | `clone_object`, `find_object`, `destruct`  | VM object table         |
| Driver services  | `send_message`, `users`, file I/O           | MOP → DriverServices    |
| Stdlib-defined   | combat, hooks, crafting                     | Registered by `.so`     |
| Timing           | `call_out`, `remove_call_out`               | Driver event loop       |

### DGD Special Objects

- **Auto object** (`sys/auto.c`) — inherited by all objects, redefines kfuns,
  establishes base behavior
- **Driver object** (`sys/driver.c`) — interface between VM and driver:
  `initialize()`, `path_read()`, `compile_object()` hooks, connection handling

### Resource Control

- `rlimits(ticks; stack_depth) { ... }` — prevents runaway scripts
- `atomic` functions — transactional execution, full rollback on error including
  state store changes

## Rust Dynamic Module System

Rust stdlib code compiles to granular `.so` modules following the same reload semantics
as LPC objects.

### Module Contract

Every `.so` exports a C ABI entry point:

```rust
#[no_mangle]
pub extern "C" fn mud_module_init(registrar: &mut ModuleRegistrar) {
    registrar.set_path("/std/combat");
    registrar.set_version(VERSION);
    registrar.add_dependency("/std/base_object");
    registrar.register_kfun("calculate_damage", calculate_damage);
    registrar.register_hook_handler("on_hit", on_hit_handler);
}
```

### Compilation and Loading

- Each module (single file or declared group) compiles to a `.so` via `cargo build`
- Adapter loads via `dlopen`, calls `mud_module_init`
- On reload: `dlclose` old → `dlopen` new → call `mud_module_init` → fire `upgraded()`
  on dependents

### State Across Reloads

- Rust modules do not hold persistent state internally — state lives in the driver
  state store
- On reload, the new `.so` picks up where the old one left off
- If a module's interface changes (e.g., kfun signature), the version tree detects this
  and propagates upgrades

### Web Portal Modules

Portal route handlers are `.so` modules like any other:

| Module                | Path               | Purpose                          |
|-----------------------|--------------------|----------------------------------|
| `/portal/account.so`  | Account routes     | Login, register, session mgmt    |
| `/portal/editor.so`   | Editor API         | File ops, validation             |
| `/portal/play.so`     | Play interface     | WebSocket game terminal          |
| `/portal/git.so`      | Git dashboard      | Branches, merge requests         |
| `/portal/review.so`   | Review UI          | MR review interface              |
| `/portal/builder.so`  | Builder tools      | Area APIs, SPA config            |

Push to stdlib → recompile affected portal module → hot-swap → no restart.

## Diff-Based Reload

The driver handles diff-based reload as a **protocol-level feature** for all adapters.
Only changed files are recompiled and reloaded.

### Flow

```
Git push (any area or stdlib)
        │
        ▼
Driver diffs changed files against last known commit
        │
        ▼
Driver groups changes by adapter language
        │
        ├── .c files   → LPC adapter:  ReloadProgram { path, area_id, files }
        ├── .rb files  → Ruby adapter:  ReloadProgram { path, area_id, files }
        ├── .kt files  → JVM adapter:   ReloadProgram { path, area_id, files }
        ├── .rs files  → Cargo build  → ReloadModule  { path }
        └── templates  → Driver reloads Tera internally
```

### New MOP Messages

| Message                                      | Direction          | Purpose                                       |
|----------------------------------------------|--------------------|-----------------------------------------------|
| `ReloadProgram { area_id, path, files }`     | Driver → Adapter   | Reload specific changed files within an area  |
| `ProgramReloaded { area_id, path, version }` | Adapter → Driver   | Confirm reload, report new version            |
| `ProgramReloadError { area_id, path, error }` | Adapter → Driver  | Reload failed, old version retained           |
| `InvalidateCache { object_ids }`             | Driver → All       | Cached values for these objects are stale     |

The existing `ReloadArea` is retained for full area reloads (initial load, recovery).
`ReloadProgram` is the surgical, diff-based reload for normal pushes.

## User and Character Model

The driver has a concept of users and characters:

- A **user** is an account (login credentials, access control)
- A **character** belongs to a user (game identity, inventory, stats)
- The driver handles authentication and connects a character to a game session
- The stdlib defines access rules, character creation, and the specifics of what a
  character contains
- Both driver and stdlib need awareness of this model

## Stdlib Repository Layout

```
stdlib/
├── mud.yaml                    # stdlib metadata
├── rust/                       # Rust .so modules
│   ├── portal/
│   │   ├── account.rs          # → /portal/account.so
│   │   ├── editor.rs           # → /portal/editor.so
│   │   ├── play.rs             # → /portal/play.so
│   │   ├── git.rs              # → /portal/git.so
│   │   ├── review.rs           # → /portal/review.so
│   │   └── builder.rs          # → /portal/builder.so
│   ├── std/
│   │   ├── hooks.rs            # → /std/hooks.so
│   │   ├── combat.rs           # → /std/combat.so
│   │   └── crafting.rs         # → /std/crafting.so
│   └── Cargo.toml              # workspace for .so modules
├── lpc/
│   ├── sys/
│   │   ├── auto.c              # Auto object — inherited by all LPC objects
│   │   └── driver.c            # Driver object — VM↔driver interface
│   ├── std/
│   │   ├── base_object.c       # Base game object — properties, hooks
│   │   ├── room.c              # Room — exits, enter/leave, look
│   │   ├── item.c              # Item — portable, use, weight
│   │   ├── npc.c               # NPC — dialogue, behavior, AI
│   │   └── daemon.c            # Daemon — heartbeat, scheduled tasks
│   ├── cmd/
│   │   ├── look.c              # Player commands
│   │   ├── take.c
│   │   ├── drop.c
│   │   ├── say.c
│   │   └── move.c
│   └── obj/
│       ├── player.c            # Player object — session, character state
│       └── user.c              # User object — account, character list
├── web/
│   └── templates/              # Tera templates for portal
│       ├── layout.html
│       ├── account/
│       ├── editor/
│       └── play/
└── db/
    └── migrations/             # Stdlib database migrations
```

## LPC Area Structure

An area written in LPC:

```
areas/town/
├── mud.yaml              # language: lpc
├── rooms/
│   ├── town_square.c
│   └── tavern.c
├── items/
│   └── sword.c
├── npcs/
│   └── bartender.c
├── daemons/
│   └── weather.c
├── web/
│   └── templates/        # area-specific web pages
└── db/
    └── migrations/       # area-specific database
```

No config files or loader DSL — the LPC code is the configuration. Each `.c` file
inherits from the stdlib (`inherit "/std/room";`) and defines its behavior in `create()`.

## Configuration

```yaml
# config/server.yml
adapters:
  lpc:
    mode: mop
    command: "mud-adapter-lpc"
    stdlib_path: "stdlib"
  ruby:
    mode: mop
    command: "ruby bin/mud-adapter"
  jvm:
    mode: mop
    command: "kotlin launcher"
```

Area language routing via `mud.yaml`:

```yaml
# LPC area
language: lpc

# Ruby area
language: ruby

# Kotlin area
language: kotlin
framework: ktor
```

## Crate Structure

| Crate               | Purpose                                              |
|----------------------|------------------------------------------------------|
| `lpc-vm`             | LPC interpreter — parser, compiler, bytecode VM     |
| `mud-adapter-lpc`    | MOP adapter binary — hosts `lpc-vm` and `.so` loader|
| `mud-mop`            | MOP protocol (existing, extended with new messages)  |
| `mud-driver`         | Driver core (existing, extended with state store,    |
|                      | object broker, version tree)                         |
